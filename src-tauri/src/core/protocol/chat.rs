//! Request translation out of the OpenAI Responses format.
//!
//! Codex sends Responses bodies; relays commonly speak Chat Completions or
//! Anthropic Messages. Field names, message shapes and tool schemas all differ,
//! so each target gets its own explicit mapping rather than a generic rename.

use crate::core::router::ModelProtocol;
use serde_json::{json, Map, Value};

/// Used when a Responses request carries no `max_output_tokens`.
const ANTHROPIC_DEFAULT_MAX_TOKENS: u64 = 32_000;

/// Chat requires a contiguous response to every call before any later message.
/// Refuse malformed history locally; never invent successful tool results.
pub(super) fn validate_tool_history(body: &Value) -> Result<(), String> {
    let mut pending = std::collections::HashSet::new();
    for message in body["messages"].as_array().ok_or("protocol.errors.invalidRequest")? {
        if message["role"] == "tool" {
            let id = message["tool_call_id"].as_str().ok_or("protocol.errors.unansweredToolCall")?;
            if !pending.remove(id) { return Err("protocol.errors.unansweredToolCall".into()); }
        } else {
            if !pending.is_empty() { return Err("protocol.errors.unansweredToolCall".into()); }
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                let id = call["id"].as_str().filter(|s| !s.is_empty()).ok_or("protocol.errors.unansweredToolCall")?;
                if !pending.insert(id) { return Err("protocol.errors.unansweredToolCall".into()); }
            }
        }
    }
    if pending.is_empty() { Ok(()) } else { Err("protocol.errors.unansweredToolCall".into()) }
}

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
            // `max_tokens`. Codex usually omits a cap, so default to one large
            // enough for agentic turns instead of truncating them.
            if let Some(system) = leading_system {
                out.insert("system".into(), json!(system));
            }
            out.insert(
                "max_tokens".into(),
                json!(max_output.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS)),
            );
        }
        _ => return Err("protocol.errors.unsupportedTranslation".into()),
    }
    if let Some(choice) = object.get("tool_choice") {
        out.insert("tool_choice".into(), tool_choice(target, choice)?);
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
    let mut thinking = false;
    if let Some(effort) = object.get("reasoning").and_then(|r| r.get("effort")) {
        // Both targets accept a reasoning effort hint under different names.
        match target {
            ModelProtocol::ChatCompletions => {
                out.insert("reasoning_effort".into(), effort.clone());
            }
            ModelProtocol::AnthropicMessages => {
                if let Some(effort) = effort.as_str() {
                    if effort == "none" || !continues_tool_turn(&out) {
                        thinking = super::reasoning::anthropic(&mut out, &model, effort);
                    }
                }
            }
            _ => {}
        }
    }
    // Anthropic rejects custom sampling while extended thinking is enabled.
    if !thinking {
        for key in ["temperature", "top_p"] {
            if let Some(setting) = object.get(key) {
                out.insert(key.into(), setting.clone());
            }
        }
    }
    apply_text_format(target, object, &mut out)?;
    Ok(Value::Object(out))
}

/// Preserve structured output constraints instead of silently requesting prose.
fn apply_text_format(
    target: ModelProtocol,
    object: &Map<String, Value>,
    out: &mut Map<String, Value>,
) -> Result<(), String> {
    let Some(text) = object.get("text").filter(|text| !text.is_null()) else {
        return Ok(());
    };
    let text = text.as_object().ok_or("protocol.errors.invalidRequest")?;
    let Some(format) = text.get("format").filter(|format| !format.is_null()) else {
        return Ok(());
    };
    let format = format.as_object().ok_or("protocol.errors.invalidRequest")?;
    match format.get("type").and_then(Value::as_str) {
        Some("text") => {}
        Some("json_object") if target == ModelProtocol::ChatCompletions => {
            out.insert("response_format".into(), json!({"type":"json_object"}));
        }
        Some("json_schema") => {
            let name = format
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 64
                        && name
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                })
                .ok_or("protocol.errors.invalidRequest")?;
            let schema = format
                .get("schema")
                .filter(|schema| schema.is_object())
                .ok_or("protocol.errors.invalidRequest")?;
            if format
                .get("strict")
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
                || format.get("description").is_some_and(|v| !v.is_string())
            {
                return Err("protocol.errors.invalidRequest".into());
            }
            if target == ModelProtocol::ChatCompletions {
                let mut schema_config = json!({"name":name,"schema":schema});
                for key in ["strict", "description"] {
                    if let Some(value) = format.get(key) {
                        schema_config[key] = value.clone();
                    }
                }
                out.insert(
                    "response_format".into(),
                    json!({"type":"json_schema","json_schema":schema_config}),
                );
            } else {
                // Merge after reasoning conversion, which also uses output_config.
                out.entry("output_config").or_insert_with(|| json!({}))["format"] =
                    json!({"type":"json_schema","schema":schema});
            }
        }
        _ => return Err("protocol.errors.unsupportedContent".into()),
    }
    Ok(())
}

/// With thinking enabled, Anthropic requires the latest assistant turn of an
/// in-progress tool loop to start with its signed thinking block. Responses
/// reasoning items cannot be turned back into signed blocks, so such a request
/// has to go without thinking instead of failing with a 400.
fn continues_tool_turn(out: &Map<String, Value>) -> bool {
    out.get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        })
        .and_then(|message| message.get("content").and_then(Value::as_array))
        .is_some_and(|content| {
            content
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        })
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
                    Some("reasoning") => {
                        flush_calls(target, &mut messages, &mut pending_calls)?;
                        // Plain summaries can cross protocols as assistant text;
                        // do not invent Anthropic signatures or silently lose them.
                        // Opaque reasoning is rejected by the Router before send.
                        let summary = item
                            .get("summary")
                            .and_then(Value::as_array)
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        if !summary.is_empty() {
                            let text = format!("[Previous reasoning summary]\n{summary}");
                            let previous = messages.last_mut().filter(|m| m["role"] == "assistant" &&
                                (m["tool_calls"].is_array() || m["content"].as_array().is_some_and(|parts| parts.iter().any(|p| p["type"] == "tool_use"))));
                            if let Some(previous) = previous {
                                // Reasoning reported after a call belongs to that
                                // same turn. Do not insert a message between a
                                // Chat call group and its contiguous tool replies.
                                if target == ModelProtocol::ChatCompletions {
                                    match &mut previous["content"] {
                                        Value::Array(parts) => parts.push(json!({"type":"text","text":text})),
                                        Value::String(content) => { *content = if content.is_empty() { text } else { format!("{content}\n{text}") }; }
                                        Value::Null => previous["content"] = json!(text),
                                        _ => return Err("protocol.errors.unsupportedContent".into()),
                                    }
                                } else {
                                    previous["content"].as_array_mut().unwrap().push(json!({"type":"text","text":text}));
                                }
                            } else {
                                messages.push(json!({"role":"assistant", "content":text}));
                            }
                        }
                    }
                    Some("message") | None => {
                        let Some(role) = item.get("role").and_then(Value::as_str) else {
                            return Err("protocol.errors.unsupportedContent".into());
                        };
                        if !matches!(role, "user" | "assistant" | "system" | "developer") {
                            return Err("protocol.errors.unsupportedContent".into());
                        }
                        flush_calls(target, &mut messages, &mut pending_calls)?;
                        let content = content_of(target, item.get("content"))?;
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
                    // References, hosted/custom calls and opaque history cannot
                    // be reconstructed as ordinary function messages.
                    _ => return Err("protocol.errors.unsupportedContent".into()),
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
fn content_of(target: ModelProtocol, content: Option<&Value>) -> Result<Value, String> {
    match content {
        Some(Value::String(text)) => Ok(json!(text)),
        Some(Value::Array(parts)) => {
            let converted: Vec<Value> = parts
                .iter()
                .map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => {
                        let text = part
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or("protocol.errors.invalidRequest")?;
                        Ok(json!({ "type": "text", "text": text }))
                    }
                    Some("input_image") => {
                        if part.get("file_id").and_then(Value::as_str).is_some()
                            && part.get("image_url").is_none()
                        {
                            // File IDs belong to the originating API account.
                            return Err("protocol.errors.unsupportedContent".into());
                        }
                        let url = part
                            .get("image_url")
                            .and_then(Value::as_str)
                            .ok_or("protocol.errors.invalidRequest")?;
                        Ok(if target == ModelProtocol::AnthropicMessages {
                            if let Some(data_uri) = url.strip_prefix("data:") {
                                let (media_type, data) = data_uri
                                    .split_once(";base64,")
                                    .ok_or("protocol.errors.invalidRequest")?;
                                if !matches!(media_type, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
                                    || data.is_empty()
                                {
                                    return Err("protocol.errors.invalidRequest".into());
                                }
                                json!({ "type": "image", "source": { "type": "base64", "media_type": media_type, "data": data } })
                            } else if url.starts_with("https://") || url.starts_with("http://") {
                                json!({ "type": "image", "source": { "type": "url", "url": url } })
                            } else {
                                return Err("protocol.errors.invalidRequest".into());
                            }
                        } else {
                            json!({ "type": "image_url", "image_url": { "url": url } })
                        })
                    }
                    Some("input_file") if target == ModelProtocol::AnthropicMessages => {
                        if let Some(url) = part.get("file_url").and_then(Value::as_str) {
                            Ok(json!({"type":"document","source":{"type":"url","url":url}}))
                        } else if let Some(data_uri) = part.get("file_data").and_then(Value::as_str) {
                            let (media_type, data) = data_uri
                                .strip_prefix("data:")
                                .and_then(|uri| uri.split_once(";base64,"))
                                .ok_or("protocol.errors.invalidRequest")?;
                            let mut document = json!({"type":"document","source":{"type":"base64","media_type":media_type,"data":data}});
                            if let Some(title) = part.get("filename").and_then(Value::as_str) {
                                document["title"] = json!(title);
                            }
                            Ok(document)
                        } else {
                            Err("protocol.errors.unsupportedContent".into())
                        }
                    }
                    Some("input_file") if target == ModelProtocol::ChatCompletions => {
                        if part.get("file_url").is_some() {
                            return Err("protocol.errors.unsupportedContent".into());
                        }
                        let file_data = part
                            .get("file_data")
                            .and_then(Value::as_str)
                            .ok_or("protocol.errors.invalidRequest")?;
                        let mut file = json!({"type":"file","file":{"file_data":file_data}});
                        if let Some(filename) = part.get("filename").and_then(Value::as_str) {
                            file["file"]["filename"] = json!(filename);
                        }
                        Ok(file)
                    }
                    // Chat Completions uses the same input_audio wire shape as
                    // Responses. Unknown Responses blocks cannot safely be sent
                    // to a different protocol without an explicit mapping.
                    Some("input_audio") if target == ModelProtocol::ChatCompletions => {
                        Ok(part.clone())
                    }
                    Some("input_audio") => Err("protocol.errors.unsupportedContent".into()),
                    Some(_) if target == ModelProtocol::ChatCompletions => Ok(part.clone()),
                    Some(_) => Err("protocol.errors.unsupportedContent".into()),
                    None => Err("protocol.errors.invalidRequest".into()),
                })
                .collect::<Result<_, String>>()?;
            // A single text part is cheaper for upstreams to accept as a string.
            match converted.as_slice() {
                [only] if only.get("type").and_then(Value::as_str) == Some("text") => {
                    Ok(only.get("text").cloned().unwrap_or(json!("")))
                }
                _ => Ok(Value::Array(converted)),
            }
        }
        None | Some(Value::Null) => Ok(json!("")),
        Some(_) => Err("protocol.errors.invalidRequest".into()),
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
fn tool_choice(target: ModelProtocol, choice: &Value) -> Result<Value, String> {
    let name = match choice {
        Value::String(kind) if matches!(kind.as_str(), "auto" | "none" | "required") => {
            return Ok(if target == ModelProtocol::AnthropicMessages {
                json!({"type":if kind == "required" { "any" } else { kind }})
            } else {
                choice.clone()
            });
        }
        Value::String(name) => Some(name.as_str()),
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            object.get("name").and_then(Value::as_str)
        }
        _ => None,
    }
    .filter(|name| !name.is_empty())
    .ok_or("protocol.errors.unsupportedToolDefinition")?;
    if target == ModelProtocol::AnthropicMessages {
        Ok(json!({"type":"tool","name":name}))
    } else {
        Ok(json!({"type":"function","function":{"name":name}}))
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
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            return Err(match tool.get("type").and_then(Value::as_str) {
                Some("namespace") => "protocol.errors.unsupportedNamespacedTool",
                Some("custom") => "protocol.errors.unsupportedCustomTool",
                _ => "protocol.errors.unsupportedHostedTool",
            }
            .into());
        }
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or("protocol.errors.unsupportedToolDefinition")?;
        let description = tool
            .get("description")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(json!(""));
        let parameters = tool
            .get("parameters")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        if !parameters.is_object()
            || !description.is_string()
            || tool
                .get("strict")
                .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            return Err("protocol.errors.unsupportedToolDefinition".into());
        }
        let mut mapped = if target == ModelProtocol::AnthropicMessages {
            json!({ "name": name, "description": description, "input_schema": parameters })
        } else {
            json!({ "type": "function", "function": {
                "name": name, "description": description, "parameters": parameters
            } })
        };
        if let Some(strict) = tool.get("strict").filter(|v| !v.is_null()) {
            if target == ModelProtocol::AnthropicMessages {
                mapped["strict"] = strict.clone();
            } else {
                mapped["function"]["strict"] = strict.clone();
            }
        }
        converted.push(mapped);
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_after_a_call_keeps_the_call_and_reply_in_their_original_turn() {
        let request = json!({"model":"m","input":[
            {"role":"assistant","content":"Checking"},
            {"type":"function_call","call_id":"c1","name":"lookup","arguments":"{}"},
            {"type":"reasoning","summary":[{"type":"summary_text","text":"Need lookup"}]},
            {"type":"function_call_output","call_id":"c1","output":"ok"}]});
        let chat = request_from_responses(ModelProtocol::ChatCompletions, &request).unwrap();
        validate_tool_history(&chat).unwrap();
        assert_eq!(chat["messages"].as_array().unwrap().len(), 2);
        assert_eq!(chat["messages"][0]["tool_calls"][0]["id"], "c1");
        assert!(chat["messages"][0]["content"].as_str().unwrap().contains("Checking\n[Previous reasoning summary]\nNeed lookup"));
        assert_eq!(chat["messages"][1]["tool_call_id"], "c1");
        let messages = request_from_responses(ModelProtocol::AnthropicMessages, &request).unwrap();
        assert_eq!(messages["messages"].as_array().unwrap().len(), 2);
        assert_eq!(messages["messages"][0]["content"][1]["type"], "tool_use");
        assert_eq!(messages["messages"][0]["content"][2]["type"], "text");
    }

    #[test]
    fn translated_tools_and_history_never_silently_drop_unsupported_items() {
        for target in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            for tool in [
                json!({"type":"web_search"}),
                json!({"type":"custom","name":"apply_patch"}),
                json!({"type":"function"}),
                json!({"type":"function","name":"lookup","parameters":[]}),
            ] {
                let result = request_from_responses(
                    target,
                    &json!({"model":"m","input":"hi","tools":[tool]}),
                );
                let expected = match tool["type"].as_str() {
                    Some("web_search") => "protocol.errors.unsupportedHostedTool",
                    Some("custom") => "protocol.errors.unsupportedCustomTool",
                    _ => "protocol.errors.unsupportedToolDefinition",
                };
                assert_eq!(result.unwrap_err(), expected);
            }
            for item in [
                json!({"type":"custom_tool_call","name":"apply_patch","input":"patch"}),
                json!({"type":"custom_tool_call_output","call_id":"c","output":"done"}),
                json!({"type":"web_search_call","status":"completed"}),
                json!({"type":"item_reference","id":"x"}),
                json!({"type":"future_item","role":"assistant","content":"important"}),
            ] {
                let result = request_from_responses(target, &json!({"model":"m","input":[item]}));
                assert_eq!(result.unwrap_err(), "protocol.errors.unsupportedContent");
            }
            let result = request_from_responses(
                target,
                &json!({"model":"m","input":"hi","tool_choice":{"type":"web_search"}}),
            );
            assert_eq!(
                result.unwrap_err(),
                "protocol.errors.unsupportedToolDefinition"
            );
        }
    }

    #[test]
    fn structured_output_and_strict_function_choice_reach_both_targets() {
        let schema = json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false});
        let request = json!({"model":"claude-opus-4-6","input":"hi","reasoning":{"effort":"high"},
            "text":{"format":{"type":"json_schema","name":"answer","schema":schema,"strict":true,"description":"Answer"}},
            "tools":[{"type":"function","name":"lookup","strict":true,"parameters":schema}],
            "tool_choice":{"type":"function","name":"lookup"}});
        let chat = request_from_responses(ModelProtocol::ChatCompletions, &request).unwrap();
        assert_eq!(
            chat["response_format"],
            json!({"type":"json_schema","json_schema":{"name":"answer","schema":schema,"strict":true,"description":"Answer"}})
        );
        assert_eq!(chat["tools"][0]["function"]["strict"], true);
        assert_eq!(
            chat["tool_choice"],
            json!({"type":"function","function":{"name":"lookup"}})
        );
        let messages = request_from_responses(ModelProtocol::AnthropicMessages, &request).unwrap();
        assert_eq!(
            messages["output_config"]["format"],
            json!({"type":"json_schema","schema":schema})
        );
        assert_eq!(messages["output_config"]["effort"], "high");
        assert_eq!(messages["tools"][0]["strict"], true);
        assert_eq!(
            messages["tool_choice"],
            json!({"type":"tool","name":"lookup"})
        );
        let json_mode = json!({"model":"m","input":"hi","text":{"format":{"type":"json_object"}}});
        assert_eq!(
            request_from_responses(ModelProtocol::ChatCompletions, &json_mode).unwrap()
                ["response_format"],
            json!({"type":"json_object"})
        );
        assert_eq!(
            request_from_responses(ModelProtocol::AnthropicMessages, &json_mode).unwrap_err(),
            "protocol.errors.unsupportedContent"
        );
        for format in [
            json!({"type":"json_schema","name":"answer"}),
            json!({"type":"json_schema","name":"answer","schema":[],"strict":true}),
        ] {
            assert_eq!(
                request_from_responses(
                    ModelProtocol::ChatCompletions,
                    &json!({"model":"m","input":"hi","text":{"format":format}})
                )
                .unwrap_err(),
                "protocol.errors.invalidRequest"
            );
        }
    }

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
    fn chat_translation_preserves_images_and_model_specific_modalities() {
        let image = request_from_responses(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","input":[{"role":"user","content":[
                {"type":"input_text","text":"describe"},
                {"type":"input_image","image_url":"data:image/png;base64,YWJj"}
            ]}]}),
        )
        .unwrap();
        assert_eq!(
            image["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,YWJj"
        );
        let anthropic_image = request_from_responses(
            ModelProtocol::AnthropicMessages,
            &json!({"model":"m","input":[{"role":"user","content":[
                {"type":"input_text","text":"describe"},
                {"type":"input_image","image_url":"data:image/png;base64,YWJj"},
                {"type":"input_image","image_url":"https://example.test/image.png"}
            ]}]}),
        )
        .unwrap();
        assert_eq!(
            anthropic_image["messages"][0]["content"][1]["source"]["type"],
            "base64"
        );
        assert_eq!(
            anthropic_image["messages"][0]["content"][1]["source"]["data"],
            "YWJj"
        );
        assert_eq!(
            anthropic_image["messages"][0]["content"][2]["source"]["type"],
            "url"
        );

        let audio = request_from_responses(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","input":[{"role":"user","content":[
                {"type":"input_audio","input_audio":{"data":"YWJj","format":"wav"}}
            ]}]}),
        );
        let audio = audio.unwrap();
        assert_eq!(audio["messages"][0]["content"][0]["type"], "input_audio");
        assert_eq!(
            audio["messages"][0]["content"][0]["input_audio"]["format"],
            "wav"
        );
        assert!(matches!(
            request_from_responses(
                ModelProtocol::AnthropicMessages,
                &json!({"model":"m","input":[{"role":"user","content":[
                    {"type":"input_audio","input_audio":{"data":"YWJj","format":"wav"}}
                ]}]})
            ),
            Err(error) if error == "protocol.errors.unsupportedContent"
        ));

        let file = content_of(
            ModelProtocol::ChatCompletions,
            Some(&json!([{"type":"input_file","filename":"report.pdf","file_data":"data:application/pdf;base64,JVBERi0="}])),
        ).unwrap();
        assert_eq!(file[0]["type"], "file");
        assert_eq!(file[0]["file"]["filename"], "report.pdf");
        let file = content_of(
            ModelProtocol::AnthropicMessages,
            Some(&json!([{"type":"input_file","filename":"report.pdf","file_data":"data:application/pdf;base64,JVBERi0="}])),
        ).unwrap();
        assert_eq!(file[0]["type"], "document");
        assert_eq!(file[0]["source"]["media_type"], "application/pdf");

        let missing_image = request_from_responses(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","input":[{"role":"user","content":[{"type":"input_image"}]}]}),
        );
        assert!(matches!(missing_image, Err(error) if error == "protocol.errors.invalidRequest"));
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

    #[test]
    fn anthropic_none_explicitly_disables_thinking() {
        let body = request_from_responses(
            ModelProtocol::AnthropicMessages,
            &json!({
                "model":"claude-opus-4-6", "reasoning":{"effort":"none"},
                "temperature":0.2, "input":"hi"
            }),
        )
        .unwrap();
        assert_eq!(body["thinking"], json!({"type":"disabled"}));
        assert!(body.get("output_config").is_none());
        assert_eq!(body["temperature"], 0.2);
    }

    #[test]
    fn anthropic_effort_reaches_the_wire_instead_of_one_fixed_budget() {
        for (effort, budget) in [("low", 4096), ("medium", 10000), ("high", 24000)] {
            let body = request_from_responses(
                ModelProtocol::AnthropicMessages,
                &json!({
                    "model":"claude-haiku-4-5-20251001", "reasoning":{"effort":effort}, "input":"hi"
                }),
            )
            .unwrap();
            assert_eq!(body["thinking"]["budget_tokens"], budget);
            let body = request_from_responses(
                ModelProtocol::AnthropicMessages,
                &json!({
                    "model":"claude-opus-4-6", "reasoning":{"effort":effort}, "input":"hi"
                }),
            )
            .unwrap();
            assert_eq!(body["thinking"], json!({"type":"adaptive"}));
            assert_eq!(body["output_config"]["effort"], effort);
        }
    }

    #[test]
    fn anthropic_thinking_is_skipped_inside_a_tool_loop_and_drops_sampling() {
        let first_turn = request_from_responses(
            ModelProtocol::AnthropicMessages,
            &json!({
                "model": "claude-model",
                "reasoning": { "effort": "high" },
                "temperature": 0.2,
                "top_p": 0.5,
                "input": [{ "role": "user", "content": "hi" }]
            }),
        )
        .unwrap();
        assert_eq!(first_turn["thinking"]["type"], "enabled");
        assert!(first_turn.get("temperature").is_none());
        assert!(first_turn.get("top_p").is_none());

        let tool_loop = request_from_responses(
            ModelProtocol::AnthropicMessages,
            &json!({
                "model": "claude-model",
                "reasoning": { "effort": "high" },
                "temperature": 0.2,
                "input": [
                    { "role": "user", "content": "hi" },
                    { "type": "reasoning", "summary": [] },
                    { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}" },
                    { "type": "function_call_output", "call_id": "call_1", "output": "found" }
                ]
            }),
        )
        .unwrap();
        assert!(tool_loop.get("thinking").is_none());
        assert_eq!(tool_loop["temperature"], 0.2);
    }
}
