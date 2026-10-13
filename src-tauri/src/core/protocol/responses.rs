//! Non-streaming translation back into the OpenAI Responses format.
//!
//! Codex validates the shape it receives (`object: "response"` with a typed
//! `output` array), so an upstream Chat Completions or Anthropic body cannot be
//! forwarded as-is. Streaming replies go through [`super::sse`] instead; this
//! module handles the single-JSON-body case.

use crate::core::protocol::Translation;
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

/// Every client function call in a replayed Messages conversation needs an
/// actual result. Responses allows interleaved reasoning/messages, unlike Chat;
/// enforce pairing and uniqueness without requiring Chat's contiguous ordering.
pub(super) fn validate_tool_history(body: &Value) -> Result<(), String> {
    let error = "protocol.errors.unansweredToolCall";
    let mut seen = std::collections::HashSet::new();
    let mut pending = std::collections::HashSet::new();
    for item in body["input"].as_array().ok_or("protocol.errors.invalidRequest")? {
        match item["type"].as_str() {
            Some("function_call") => {
                let id = item["call_id"].as_str().filter(|s| !s.is_empty()).ok_or(error)?;
                if !seen.insert(id) { return Err(error.into()); }
                pending.insert(id);
            }
            Some("function_call_output") => {
                let id = item["call_id"].as_str().ok_or(error)?;
                if !pending.remove(id) { return Err(error.into()); }
            }
            _ => {}
        }
    }
    if pending.is_empty() { Ok(()) } else { Err(error.into()) }
}

/// Shared completion semantics for streaming and single-body translations.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    Completed,
    Incomplete(&'static str),
    Failed(&'static str),
}

impl Completion {
    pub(super) fn from_reason(protocol: ModelProtocol, reason: Option<&str>) -> Self {
        match (protocol, reason) {
            (ModelProtocol::ChatCompletions, Some("stop" | "tool_calls"))
            | (ModelProtocol::AnthropicMessages, Some("end_turn" | "stop_sequence" | "tool_use")) => {
                Self::Completed
            }
            (_, Some("length" | "max_tokens" | "model_context_window_exceeded")) => {
                Self::Incomplete("max_output_tokens")
            }
            (_, Some("content_filter" | "refusal")) => Self::Incomplete("content_filter"),
            (_, None) => Self::Failed("missing_finish_reason"),
            _ => Self::Failed("unsupported_finish_reason"),
        }
    }

    pub(super) fn status(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Incomplete(_) => "incomplete",
            Self::Failed(_) => "failed",
        }
    }

    pub(super) fn apply(self, response: &mut Value) {
        response["status"] = json!(self.status());
        match self {
            Self::Completed => {}
            Self::Incomplete(reason) => response["incomplete_details"] = json!({"reason":reason}),
            Self::Failed(code) => {
                // Codex treats unknown response.failed codes as retryable.
                // Use its invalid_prompt class for deterministic adapter errors;
                // preserve our exact reason separately for diagnostics. Transport
                // failures remain retryable and real upstream errors retain theirs.
                let deterministic = matches!(
                    code,
                    "invalid_tool_arguments"
                        | "invalid_stream_frame"
                        | "missing_finish_reason"
                        | "unsupported_finish_reason"
                );
                response["error"] = json!({
                    "code":if deterministic { "invalid_prompt" } else { code },
                    "reason":code,
                    "message": match code {
                        "invalid_tool_arguments" => "Provider returned an invalid or incomplete function call",
                        "upstream_stream_truncated" => "Provider stream ended before its terminal event",
                        "invalid_stream_frame" => "Provider returned an invalid stream frame",
                        "upstream_stream_error" => "Provider stream could not be read",
                        _ => "Provider did not report a supported completion reason",
                    },
                })
            }
        }
    }
}

pub(super) fn valid_function_call(item: &Value) -> bool {
    ["call_id", "name"].iter().all(|key| {
        item.get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    }) && item
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value.is_object())
}

/// Convert a complete upstream response body into a Responses payload.
pub fn response_body(translation: Translation, value: &Value) -> Result<Value, String> {
    match translation.to {
        ModelProtocol::ChatCompletions => chat_to_responses(value),
        ModelProtocol::AnthropicMessages => anthropic_to_responses(value),
        _ => Err("protocol.errors.unsupportedTranslation".into()),
    }
}

/// Whether an upstream body already looks like a Responses payload.
///
/// Relays that advertise one protocol sometimes answer in another, and a body
/// that is already correct must not be re-wrapped.
pub fn is_responses_body(value: &Value) -> bool {
    value.get("object").and_then(Value::as_str) == Some("response")
}

fn chat_to_responses(value: &Value) -> Result<Value, String> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or("protocol.errors.invalidResponse")?;
    let message = choice
        .get("message")
        .filter(|message| message.is_object())
        .ok_or("protocol.errors.invalidResponse")?;
    let mut output = Vec::new();

    // `reasoning_content` is a common relay extension for reasoning models.
    if let Some(reasoning) = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        output.push(json!({
            "id": item_id("rs"),
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": reasoning }],
        }));
    }
    let mut text = message.get("content").and_then(Value::as_str).unwrap_or("").to_owned();
    let annotations = super::search_compat::citations(&mut text, &message["annotations"]);
    if !text.is_empty() {
        output.push(json!({
            "id": item_id("msg"),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text, "annotations": annotations }],
        }));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let function = call.get("function").cloned().unwrap_or(json!({}));
            output.push(json!({
                "id": item_id("fc"),
                "type": "function_call",
                "call_id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
                "arguments": function.get("arguments").and_then(Value::as_str).unwrap_or(""),
                "status": "completed",
            }));
        }
    }
    let completion = if message["error_message"].as_str().is_some_and(|message| !message.is_empty()) {
        Completion::Failed("upstream_search_error")
    } else if let Some(refusal) = message
        .get("refusal")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        output.push(json!({
            "id":item_id("msg"),"type":"message","role":"assistant","status":"incomplete",
            "content":[{"type":"refusal","refusal":refusal}],
        }));
        Completion::Incomplete("content_filter")
    } else {
        Completion::from_reason(
            ModelProtocol::ChatCompletions,
            choice.get("finish_reason").and_then(Value::as_str),
        )
    };
    let mut response = finish(
        value,
        output,
        value.get("usage").map(openai_usage),
        completion,
    );
    if let Some(usage) = value["usage"].get("web_search_usage") {
        response["usage"]["web_search_usage"] = usage.clone();
    }
    Ok(response)
}

fn anthropic_to_responses(value: &Value) -> Result<Value, String> {
    let mut output = Vec::new();
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or("protocol.errors.invalidResponse")?;
    // Claude splits a cited answer into many text blocks. Consecutive blocks
    // form one message whose sources are listed once at its end.
    fn flush_text(output: &mut Vec<Value>, run: &mut Option<(String, Vec<Value>)>) {
        if let Some((mut text, citations)) = run.take() {
            let annotations = super::search_compat::citations(&mut text, &Value::Array(citations));
            if !text.is_empty() {
                output.push(json!({
                    "id": item_id("msg"),
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{ "type": "output_text", "text": text, "annotations": annotations }],
                }));
            }
        }
    }
    let mut text_run: Option<(String, Vec<Value>)> = None;
    for block in blocks {
        if block["type"] != "text" {
            flush_text(&mut output, &mut text_run);
        }
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let run = text_run.get_or_insert_with(Default::default);
                run.0.push_str(block.get("text").and_then(Value::as_str).unwrap_or(""));
                run.1.extend(block["citations"].as_array().into_iter().flatten().cloned());
            }
            Some("thinking") => {
                let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() {
                    output.push(json!({
                        "id": item_id("rs"),
                        "type": "reasoning",
                        "summary": [{ "type": "summary_text", "text": text }],
                    }));
                }
            }
            Some("tool_use") => {
                output.push(json!({
                    "id": item_id("fc"),
                    "type": "function_call",
                    "call_id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                    "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                    // Responses reports arguments as a JSON string.
                    "arguments": block.get("input").map(Value::to_string).unwrap_or_default(),
                    "status": "completed",
                }));
            }
            Some("server_tool_use") if block["name"] == "web_search" => {
                let result = blocks.iter().find(|result| result["type"] == "web_search_tool_result"
                    && result["tool_use_id"] == block["id"])
                    .ok_or("protocol.errors.invalidResponse")?;
                let search = super::web_search::history_call(block, result)
                    .map_err(|_| "protocol.errors.invalidResponse")?;
                output.push(search);
            }
            Some("web_search_tool_result") => {
                if !blocks.iter().any(|call| call["type"] == "server_tool_use" && call["id"] == block["tool_use_id"]) {
                    return Err("protocol.errors.invalidResponse".into());
                }
            }
            _ => {}
        }
    }
    flush_text(&mut output, &mut text_run);
    Ok(finish(
        value,
        output,
        value.get("usage").map(anthropic_usage),
        Completion::from_reason(
            ModelProtocol::AnthropicMessages,
            value.get("stop_reason").and_then(Value::as_str),
        ),
    ))
}

/// Assemble the envelope both upstreams share.
fn finish(
    upstream: &Value,
    mut output: Vec<Value>,
    usage: Option<Value>,
    mut completion: Completion,
) -> Value {
    if completion == Completion::Completed
        && output.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && !valid_function_call(item)
        })
    {
        completion = Completion::Failed("invalid_tool_arguments");
    }
    if completion != Completion::Completed {
        for item in &mut output {
            if item.get("status").is_some() {
                item["status"] = json!("incomplete");
            }
        }
    }
    let mut response = json!({
        "id": item_id("resp"),
        "object": "response",
        "status": "completed",
        "model": upstream.get("model").and_then(Value::as_str).unwrap_or_default(),
        "output": output,
        "usage": usage.unwrap_or_else(|| json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
        })),
    });
    completion.apply(&mut response);
    response
}

pub(super) fn openai_usage(usage: &Value) -> Value {
    let input = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "input_tokens": input,
        "output_tokens": output,
        "total_tokens": usage.get("total_tokens").and_then(Value::as_u64).unwrap_or(input + output),
        "input_tokens_details": {"cached_tokens":usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0)},
    })
}

fn anthropic_usage(usage: &Value) -> Value {
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut mapped = json!({
        "input_tokens": input,
        "output_tokens": output,
        "total_tokens": input + output,
    });
    if let Some(search) = usage.get("server_tool_use") { mapped["server_tool_use"] = search.clone(); }
    mapped
}

fn item_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::Translation;

    fn translation(to: ModelProtocol) -> Translation {
        Translation {
            from: ModelProtocol::Responses,
            to,
        }
    }

    #[test]
    fn chat_response_becomes_a_responses_payload() {
        let converted = response_body(
            translation(ModelProtocol::ChatCompletions),
            &json!({
                "id": "chatcmpl-1",
                "model": "gpt-x",
                "choices": [{ "finish_reason": "stop", "message": { "role": "assistant", "content": "Hello" } }],
                "usage": { "prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6 }
            }),
        )
        .unwrap();

        assert_eq!(converted["object"], "response");
        assert_eq!(converted["status"], "completed");
        assert_eq!(converted["output"][0]["type"], "message");
        assert_eq!(converted["output"][0]["content"][0]["text"], "Hello");
        assert_eq!(converted["usage"]["output_tokens"], 2);
    }

    #[test]
    fn chat_tool_calls_become_function_call_items() {
        let converted = response_body(
            translation(ModelProtocol::ChatCompletions),
            &json!({
                "choices": [{ "finish_reason": "tool_calls", "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{\"path\":\"a\"}" }
                    }]
                }}]
            }),
        )
        .unwrap();

        assert_eq!(converted["output"][0]["type"], "function_call");
        assert_eq!(converted["output"][0]["call_id"], "call_1");
        assert_eq!(converted["output"][0]["name"], "read_file");
    }

    #[test]
    fn anthropic_response_includes_thinking_and_tools() {
        let converted = response_body(
            translation(ModelProtocol::AnthropicMessages),
            &json!({
                "id": "msg_1",
                "model": "claude-x",
                "stop_reason": "tool_use",
                "content": [
                    { "type": "thinking", "thinking": "considering" },
                    { "type": "text", "text": "Answer" },
                    { "type": "tool_use", "id": "toolu_1", "name": "search", "input": { "q": "x" } }
                ],
                "usage": { "input_tokens": 3, "output_tokens": 5 }
            }),
        )
        .unwrap();

        let types: Vec<&str> = converted["output"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.get("type").and_then(Value::as_str))
            .collect();
        assert_eq!(types, vec!["reasoning", "message", "function_call"]);
        assert_eq!(converted["usage"]["total_tokens"], 8);
    }

    #[test]
    fn an_already_responses_shaped_body_is_recognised() {
        assert!(is_responses_body(&json!({ "object": "response" })));
        assert!(!is_responses_body(&json!({ "object": "chat.completion" })));
    }

    #[test]
    fn unexpected_success_bodies_are_not_reported_as_empty_answers() {
        assert!(response_body(translation(ModelProtocol::ChatCompletions), &json!({})).is_err());
        assert!(response_body(
            translation(ModelProtocol::AnthropicMessages),
            &json!({"error":"bad key"})
        )
        .is_err());
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;

    #[test]
    fn nonstream_finish_reasons_match_streaming_states() {
        for (target, reason, status, detail) in [
            (ModelProtocol::ChatCompletions, "stop", "completed", None),
            (
                ModelProtocol::ChatCompletions,
                "length",
                "incomplete",
                Some("max_output_tokens"),
            ),
            (
                ModelProtocol::ChatCompletions,
                "content_filter",
                "incomplete",
                Some("content_filter"),
            ),
            (
                ModelProtocol::AnthropicMessages,
                "end_turn",
                "completed",
                None,
            ),
            (
                ModelProtocol::AnthropicMessages,
                "max_tokens",
                "incomplete",
                Some("max_output_tokens"),
            ),
            (
                ModelProtocol::AnthropicMessages,
                "refusal",
                "incomplete",
                Some("content_filter"),
            ),
        ] {
            let value = if target == ModelProtocol::ChatCompletions {
                json!({"choices":[{"message":{"content":"answer"},"finish_reason":reason}]})
            } else {
                json!({"content":[{"type":"text","text":"answer"}],"stop_reason":reason})
            };
            let result = response_body(
                Translation {
                    from: ModelProtocol::Responses,
                    to: target,
                },
                &value,
            )
            .unwrap();
            assert_eq!(result["status"], status);
            assert_eq!(result["incomplete_details"]["reason"].as_str(), detail);
            assert_eq!(
                result["output"][0]["status"],
                if status == "completed" {
                    "completed"
                } else {
                    "incomplete"
                }
            );
        }
    }

    #[test]
    fn missing_terminal_reasons_and_malformed_tools_cannot_succeed() {
        for target in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let body = if target == ModelProtocol::ChatCompletions {
                json!({"choices":[{"message":{"content":"partial"}}]})
            } else {
                json!({"content":[{"type":"text","text":"partial"}]})
            };
            let result = response_body(
                Translation {
                    from: ModelProtocol::Responses,
                    to: target,
                },
                &body,
            )
            .unwrap();
            assert_eq!(result["status"], "failed");
            let bad_tool = if target == ModelProtocol::ChatCompletions {
                json!({"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"f","arguments":"{"}}]},"finish_reason":"tool_calls"}]})
            } else {
                json!({"content":[{"type":"tool_use","id":"c","name":"f","input":"not an object"}],"stop_reason":"tool_use"})
            };
            let result = response_body(
                Translation {
                    from: ModelProtocol::Responses,
                    to: target,
                },
                &bad_tool,
            )
            .unwrap();
            assert_eq!(result["status"], "failed");
            assert_eq!(result["error"]["code"], "invalid_prompt");
            assert_eq!(result["error"]["reason"], "invalid_tool_arguments");
        }
    }

    #[test]
    fn a_chat_refusal_is_preserved_and_never_an_empty_completed_answer() {
        let result = response_body(Translation {from:ModelProtocol::Responses,to:ModelProtocol::ChatCompletions}, &json!({"choices":[{"message":{"content":null,"refusal":"Cannot comply"},"finish_reason":"stop"}]})).unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["incomplete_details"]["reason"], "content_filter");
        assert_eq!(
            result["output"][0]["content"][0]["refusal"],
            "Cannot comply"
        );
    }
}
