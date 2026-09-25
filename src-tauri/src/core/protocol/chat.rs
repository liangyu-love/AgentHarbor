//! Request translation out of the OpenAI Responses format.
//!
//! Codex sends Responses bodies; relays commonly speak Chat Completions or
//! Anthropic Messages. Field names, message shapes and tool schemas all differ,
//! so each target gets its own explicit mapping rather than a generic rename.

use crate::core::router::ModelProtocol;
use serde_json::{json, Map, Value};

/// Translate a Responses request body into the provider's protocol.
pub(super) fn request_from_responses(
    target: ModelProtocol,
    value: &Value,
) -> Result<Value, String> {
    let object = value.as_object().ok_or("protocol.errors.invalidRequest")?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or("protocol.errors.invalidRequest")?
        .to_owned();
    let (messages, leading_system) = messages_from_responses(target, object)?;
    let tools = tools_from_responses(target, object)?;
    let stream = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_output = object.get("max_output_tokens").and_then(Value::as_u64);

    let mut out = Map::new();
    out.insert("model".into(), json!(model));
    out.insert("messages".into(), Value::Array(messages));
    out.insert("stream".into(), json!(stream));

    match target {
        ModelProtocol::ChatCompletions => {
            // Chat Completions reports usage on the final chunk only when asked.
            if stream {
                out.insert("stream_options".into(), json!({ "include_usage": true }));
            }
            if let Some(max) = max_output {
                // Reasoning-series models reject `max_tokens` in favour of the
                // newer name; everything else (and most relays) expects the old
                // one, so the model decides rather than sending both.
                let key = if prefers_completion_tokens(&model) {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                out.insert(key.into(), json!(max));
            }
        }
        ModelProtocol::AnthropicMessages => {
            // Anthropic carries the system prompt out of band and requires
            // `max_tokens`, so fall back to a conservative cap.
            if let Some(system) = leading_system {
                out.insert("system".into(), json!(system));
            }
            out.insert("max_tokens".into(), json!(max_output.unwrap_or(4096)));
        }
        _ => return Err("protocol.errors.unsupportedTranslation".into()),
    }
    if let Some(choice) = object.get("tool_choice") {
        out.insert("tool_choice".into(), tool_choice(target, choice));
    }
    if target == ModelProtocol::ChatCompletions {
        if let Some(parallel) = object.get("parallel_tool_calls") {
            out.insert("parallel_tool_calls".into(), parallel.clone());
        }
    } else if object.get("parallel_tool_calls") == Some(&Value::Bool(false)) {
        let choice = out
            .entry("tool_choice")
            .or_insert_with(|| json!({"type":"auto"}));
        if let Some(choice) = choice.as_object_mut() {
            choice.insert("disable_parallel_tool_use".into(), json!(true));
        }
    }

    if !tools.is_empty() {
        out.insert("tools".into(), Value::Array(tools));
    }
    for key in ["temperature", "top_p"] {
        if let Some(setting) = object.get(key) {
            out.insert(key.into(), setting.clone());
        }
    }
    if let Some(effort) = object.get("reasoning").and_then(|r| r.get("effort")) {
        // Both targets accept a reasoning effort hint under different names.
        match target {
            ModelProtocol::ChatCompletions => {
                out.insert("reasoning_effort".into(), effort.clone());
            }
            ModelProtocol::AnthropicMessages if max_output.unwrap_or(4096) > 1024 => {
                out.insert("thinking".into(), json!({ "type": "enabled", "budget_tokens": 2048.min(max_output.unwrap_or(4096) - 1) }));
            }
            _ => {}
        }
    }
    Ok(Value::Object(out))
}

/// Flatten Responses `input` items into provider messages.
///
/// Returns the messages plus, for Anthropic, the system text hoisted out of them.
/// When the request continues an earlier turn by id, the messages that turn used
/// are replayed first, because the upstream keeps no conversation state.
fn messages_from_responses(
    target: ModelProtocol,
    object: &Map<String, Value>,
) -> Result<(Vec<Value>, Option<String>), String> {
    let mut messages: Vec<Value> = Vec::new();
    let mut system: Option<String> = None;
    if let Some(instructions) = object.get("instructions").and_then(Value::as_str) {
        if !instructions.trim().is_empty() {
            system = Some(instructions.to_owned());
            if target == ModelProtocol::ChatCompletions {
                messages.push(json!({ "role": "system", "content": instructions }));
            }
        }
    }
    match object.get("input") {
        // Codex always sends an array, but a bare string is valid Responses too.
        Some(Value::String(text)) => messages.push(json!({ "role": "user", "content": text })),
        Some(Value::Array(items)) => {
            let mut pending_calls: Vec<Value> = Vec::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                        pending_calls.push(json!({
                            "id": item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                            "type": "function",
                            "function": {
                                "name": name,
                                // Responses reports arguments as a JSON string already.
                                "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
                            },
                        }));
                    }
                    Some("function_call_output") => {
                        flush_calls(target, &mut messages, &mut pending_calls)?;
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let output = text_of(item.get("output"));
                        if target == ModelProtocol::AnthropicMessages {
                            messages.push(json!({ "role": "user", "content": [{
                                "type": "tool_result", "tool_use_id": call_id, "content": output
                            }] }));
                        } else {
                            messages.push(json!({
                                "role": "tool", "tool_call_id": call_id, "content": output
                            }));
                        }
                    }
                    Some("reasoning") | Some("item_reference") => {}
                    // A `message` item, or anything carrying a role.
                    _ => {
                        let Some(role) = item.get("role").and_then(Value::as_str) else {
                            continue;
                        };
                        flush_calls(target, &mut messages, &mut pending_calls)?;
                        let content = content_of(target, item.get("content"));
                        if role == "system" || role == "developer" {
                            let text = system_text(item.get("content"));
                            if target == ModelProtocol::AnthropicMessages {
                                system = Some(match system {
                                    Some(previous) => format!("{previous}\n{text}"),
                                    None => text,
                                });
                            } else {
                                messages.push(json!({ "role": "system", "content": content }));
                            }
                            continue;
                        }
                        messages.push(json!({ "role": role, "content": content }));
                    }
                }
            }
            flush_calls(target, &mut messages, &mut pending_calls)?;
        }
        None => {}
        _ => return Err("protocol.errors.invalidRequest".into()),
    }
    Ok((messages, system))
}

fn flush_calls(
    target: ModelProtocol,
    messages: &mut Vec<Value>,
    calls: &mut Vec<Value>,
) -> Result<(), String> {
    if calls.is_empty() {
        return Ok(());
    }
    let calls = std::mem::take(calls);
    if calls.iter().any(|call| {
        !super::responses::valid_function_call(&json!({
            "call_id":call["id"], "name":call["function"]["name"],
            "arguments":call["function"]["arguments"],
        }))
    }) {
        return Err("protocol.errors.invalidToolArguments".into());
    }
    if target == ModelProtocol::AnthropicMessages {
        let blocks: Vec<Value> = calls
            .iter()
            .map(|call| {
                let function = &call["function"];
                let input = function["arguments"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .filter(Value::is_object)
                    .ok_or("protocol.errors.invalidToolArguments")?;
                Ok(json!({
                    "type": "tool_use",
                    "id": call["id"],
                    "name": function["name"],
                    "input": input,
                }))
            })
            .collect::<Result<Vec<_>, &str>>()?;
        if let Some(last) = messages
            .last_mut()
            .filter(|item| item["role"] == "assistant")
        {
            let mut content = match &last["content"] {
                Value::Array(parts) => parts.clone(),
                Value::String(text) => vec![json!({"type":"text","text":text})],
                _ => Vec::new(),
            };
            content.extend(blocks);
            last["content"] = json!(content);
        } else {
            messages.push(json!({ "role": "assistant", "content": blocks }));
        }
    } else {
        if let Some(last) = messages
            .last_mut()
            .filter(|item| item["role"] == "assistant")
        {
            let mut combined = last
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            combined.extend(calls);
            last["tool_calls"] = json!(combined);
        } else {
            messages
                .push(json!({ "role": "assistant", "content": Value::Null, "tool_calls": calls }));
        }
    }
    Ok(())
}

fn system_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text_of(value),
    }
}

/// Responses content parts become plain text or a multimodal part array.
fn content_of(target: ModelProtocol, content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(text)) => json!(text),
        Some(Value::Array(parts)) => {
            let converted: Vec<Value> = parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => {
                        Some(json!({ "type": "text", "text": part.get("text").cloned().unwrap_or(json!("")) }))
                    }
                    Some("input_image") => part.get("image_url").map(|url| {
                        if target == ModelProtocol::AnthropicMessages {
                            json!({ "type": "image", "source": { "type": "url", "url": url } })
                        } else {
                            json!({ "type": "image_url", "image_url": { "url": url } })
                        }
                    }),
                    _ => None,
                })
                .collect();
            // A single text part is cheaper for upstreams to accept as a string.
            match converted.as_slice() {
                [only] if only.get("type").and_then(Value::as_str) == Some("text") => {
                    only.get("text").cloned().unwrap_or(json!(""))
                }
                _ => Value::Array(converted),
            }
        }
        _ => json!(""),
    }
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Whether Chat Completions upstreams for this model want the newer cap name.
///
/// Only OpenAI's reasoning families moved; treating every model as newer would
/// break the OpenAI-compatible relays that make up most of a relay catalog.
fn prefers_completion_tokens(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    let compact: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    compact.starts_with("o1")
        || compact.starts_with("o3")
        || compact.starts_with("o4")
        || compact.starts_with("gpt5")
        || compact.starts_with("gpt6")
}

/// Translate a Responses tool choice into the Chat Completions spelling.
///
/// Both name options, but Responses may also force a specific tool by name
/// instead of an object, so the string form is normalised to the object form
/// whenever a tool list is present.
fn tool_choice(target: ModelProtocol, choice: &Value) -> Value {
    if target == ModelProtocol::AnthropicMessages {
        return match choice {
            Value::String(kind) if kind == "required" => json!({ "type": "any" }),
            Value::String(kind) if ["auto", "none"].contains(&kind.as_str()) => {
                json!({ "type": kind })
            }
            Value::String(name) => json!({ "type": "tool", "name": name }),
            Value::Object(object) => {
                let name = object
                    .get("name")
                    .or_else(|| object.get("function").and_then(|f| f.get("name")));
                name.map(|name| json!({ "type": "tool", "name": name }))
                    .unwrap_or_else(|| json!({ "type": "auto" }))
            }
            _ => json!({ "type": "auto" }),
        };
    }
    match choice {
        Value::String(kind) => match kind.as_str() {
            "auto" | "none" | "required" => json!(kind),
            // A bare string naming a function is the forced-function form.
            name => json!({ "type": "function", "function": { "name": name } }),
        },
        _ => choice.clone(),
    }
}

/// Responses tools are flat (`{type, name, parameters}`); both targets nest them.
fn tools_from_responses(
    target: ModelProtocol,
    object: &Map<String, Value>,
) -> Result<Vec<Value>, String> {
    let Some(tools) = object.get("tools") else {
        return Ok(Vec::new());
    };
    let tools = tools.as_array().ok_or("protocol.errors.invalidRequest")?;
    let mut converted = Vec::new();
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let description = tool.get("description").cloned().unwrap_or(json!(""));
        let parameters = tool
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        converted.push(if target == ModelProtocol::AnthropicMessages {
            json!({ "name": name, "description": description, "input_schema": parameters })
        } else {
            json!({ "type": "function", "function": {
                "name": name, "description": description, "parameters": parameters
            } })
        });
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_tools_use_function_schema() {
        let converted = request_from_responses(
            ModelProtocol::ChatCompletions,
            &json!({
                "model": "chat-model", "input": "hi",
                "tools": [{ "type": "function", "name": "lookup", "description": "Find", "parameters": { "type": "object" } }],
                "tool_choice": "lookup"
            }),
        ).unwrap();
        assert_eq!(converted["tools"][0]["type"], "function");
        assert_eq!(converted["tools"][0]["function"]["name"], "lookup");
        assert_eq!(
            converted["tools"][0]["function"]["parameters"]["type"],
            "object"
        );
        assert!(converted["tools"][0].get("input_schema").is_none());
        assert_eq!(converted["tool_choice"]["function"]["name"], "lookup");
    }

    #[test]
    fn anthropic_request_hoists_system_and_uses_native_tool_messages() {
        let converted = request_from_responses(
            ModelProtocol::AnthropicMessages,
            &json!({
                "model": "claude-model",
                "instructions": "Be concise",
                "input": [
                    { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":\"x\"}" },
                    { "type": "function_call_output", "call_id": "call_1", "output": "found" }
                ],
                "tools": [{ "type": "function", "name": "lookup", "parameters": { "type": "object" } }],
                "tool_choice": "lookup",
                "parallel_tool_calls": false
            }),
        ).unwrap();
        assert_eq!(converted["system"], "Be concise");
        assert_eq!(converted["messages"][0]["content"][0]["type"], "tool_use");
        assert_eq!(converted["messages"][0]["content"][0]["input"]["q"], "x");
        assert_eq!(
            converted["messages"][1]["content"][0]["type"],
            "tool_result"
        );
        assert_eq!(
            converted["messages"][1]["content"][0]["tool_use_id"],
            "call_1"
        );
        assert_eq!(converted["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(converted["tool_choice"]["type"], "tool");
        assert_eq!(converted["tool_choice"]["disable_parallel_tool_use"], true);
        assert!(converted.get("parallel_tool_calls").is_none());
    }
}
