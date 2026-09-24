//! Non-streaming translation back into the OpenAI Responses format.
//!
//! Codex validates the shape it receives (`object: "response"` with a typed
//! `output` array), so an upstream Chat Completions or Anthropic body cannot be
//! forwarded as-is. Streaming replies go through [`super::sse`] instead; this
//! module handles the single-JSON-body case.

use crate::core::protocol::Translation;
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

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
    let text = message.get("content").and_then(Value::as_str).unwrap_or("");
    if !text.is_empty() {
        output.push(json!({
            "id": item_id("msg"),
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }],
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
                "arguments": function.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
                "status": "completed",
            }));
        }
    }
    Ok(finish(value, output, value.get("usage").map(openai_usage)))
}

fn anthropic_to_responses(value: &Value) -> Result<Value, String> {
    let mut output = Vec::new();
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or("protocol.errors.invalidResponse")?;
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() {
                    output.push(json!({
                        "id": item_id("msg"),
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [{ "type": "output_text", "text": text, "annotations": [] }],
                    }));
                }
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
                        "arguments": block.get("input").map(Value::to_string).unwrap_or_else(|| "{}".into()),
                        "status": "completed",
                    }));
            }
            _ => {}
        }
    }
    Ok(finish(
        value,
        output,
        value.get("usage").map(anthropic_usage),
    ))
}

/// Assemble the envelope both upstreams share.
fn finish(upstream: &Value, output: Vec<Value>, usage: Option<Value>) -> Value {
    json!({
        "id": format!("resp_{}", upstream.get("id").and_then(Value::as_str).unwrap_or("translated")),
        "object": "response",
        "status": "completed",
        "model": upstream.get("model").and_then(Value::as_str).unwrap_or_default(),
        "output": output,
        "usage": usage.unwrap_or_else(|| json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
        })),
    })
}

fn openai_usage(usage: &Value) -> Value {
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
    json!({
        "input_tokens": input,
        "output_tokens": output,
        "total_tokens": input + output,
    })
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
                "choices": [{ "message": { "role": "assistant", "content": "Hello" } }],
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
                "choices": [{ "message": {
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
