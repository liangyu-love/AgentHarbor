//! Portable Chat ingress. Unknown private wire features are never replayed as
//! ordinary messages; the existing Responses egress encoders own target shapes.
use super::{chat, responses, ModelProtocol, ResponseOptions, TranslatedRequest, Translation};
use serde_json::{json, Map, Value};

const INVALID: &str = "protocol.errors.invalidRequest";
const UNSUPPORTED: &str = "protocol.errors.unsupportedContent";

pub(super) fn request(target: ModelProtocol, value: &Value) -> Result<TranslatedRequest, String> {
    let object = value.as_object().ok_or(INVALID)?;
    let model = string(value, "model")?;
    let messages = value["messages"]
        .as_array()
        .filter(|m| !m.is_empty())
        .ok_or(INVALID)?;
    validate_options(object, target)?;
    chat::validate_tool_history(value)?;
    let mut input = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for message in messages {
        if !message.is_object() {
            return Err(INVALID.into());
        }
        reject_private(message)?;
        let role = string(message, "role")?;
        if message.get("function_call").is_some_and(|v| !v.is_null()) {
            return Err(UNSUPPORTED.into());
        }
        if role == "tool" {
            if message.get("tool_calls").is_some_and(|v| !v.is_null()) {
                return Err(INVALID.into());
            }
            input.push(
                json!({"type":"function_call_output","call_id":string(message,"tool_call_id")?,
                "output":tool_output(&message["content"])?}),
            );
            continue;
        }
        if !matches!(role, "system" | "developer" | "user" | "assistant") {
            return Err("protocol.errors.unsupportedMessageRole".into());
        }
        if message.get("name").is_some_and(|name| !name.is_null()) {
            // Responses/Messages have no named-message attribution equivalent.
            return Err(UNSUPPORTED.into());
        }
        let calls = match message.get("tool_calls").filter(|v| !v.is_null()) {
            Some(calls) if role == "assistant" => Some(calls.as_array().ok_or(INVALID)?),
            Some(_) => return Err(INVALID.into()),
            None => None,
        };
        let content = content(
            role,
            message.get("content"),
            calls.is_some_and(|c| !c.is_empty()),
        )?;
        input.push(json!({"type":"message","role":role,"content":content}));
        if let Some(calls) = calls {
            for call in calls {
                reject_private(call)?;
                if call["type"] != "function" {
                    return Err("protocol.errors.unsupportedCustomTool".into());
                }
                let id = string(call, "id")?;
                if !ids.insert(id) {
                    return Err("protocol.errors.unansweredToolCall".into());
                }
                let item = json!({"type":"function_call","call_id":id,
                    "name":string(&call["function"],"name")?,"arguments":string(&call["function"],"arguments")?});
                if !responses::valid_function_call(&item) {
                    return Err("protocol.errors.invalidToolArguments".into());
                }
                input.push(item);
            }
        }
    }
    let mut canonical = json!({"model":model,"input":input,"stream":value["stream"].as_bool().unwrap_or(false),"store":false});
    responses::validate_tool_history(&canonical)?;
    if let Some(max) = object
        .get("max_completion_tokens")
        .filter(|v| !v.is_null())
        .or_else(|| object.get("max_tokens").filter(|v| !v.is_null()))
    {
        if max.as_u64().is_none_or(|n| n == 0) {
            return Err(INVALID.into());
        }
        canonical["max_output_tokens"] = max.clone();
    }
    if let Some(tools) = object.get("tools").filter(|v| !v.is_null()) {
        let mut names = std::collections::HashSet::new();
        let tools = tools
            .as_array()
            .ok_or(INVALID)?
            .iter()
            .map(|tool| {
                reject_private(tool)?;
                if tool["type"] != "function" {
                    return Err("protocol.errors.unsupportedHostedTool".into());
                }
                let function = &tool["function"];
                let name = string(function, "name")?;
                if !names.insert(name) {
                    return Err("protocol.errors.unsupportedToolDefinition".into());
                }
                let parameters = function
                    .get("parameters")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}}));
                if !parameters.is_object()
                    || function
                        .get("description")
                        .is_some_and(|v| !v.is_null() && !v.is_string())
                    || function
                        .get("strict")
                        .is_some_and(|v| !v.is_null() && !v.is_boolean())
                {
                    return Err("protocol.errors.unsupportedToolDefinition".into());
                }
                let mut mapped = json!({"type":"function","name":name,"parameters":parameters});
                for key in ["description", "strict"] {
                    if let Some(v) = function.get(key).filter(|v| !v.is_null()) {
                        mapped[key] = v.clone();
                    }
                }
                Ok(mapped)
            })
            .collect::<Result<Vec<Value>, String>>()?;
        canonical["tools"] = json!(tools);
    }
    if let Some(choice) = object.get("tool_choice").filter(|v| !v.is_null()) {
        canonical["tool_choice"] = match choice {
            Value::String(kind) if matches!(kind.as_str(), "auto" | "none" | "required") => {
                choice.clone()
            }
            Value::Object(_) if choice["type"] == "function" => {
                let name = string(&choice["function"], "name")?;
                if !canonical["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
                {
                    return Err("protocol.errors.unsupportedToolDefinition".into());
                }
                json!({"type":"function","name":name})
            }
            _ => return Err("protocol.errors.unsupportedToolDefinition".into()),
        };
        if choice == "required"
            && !canonical["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty())
        {
            return Err("protocol.errors.unsupportedToolDefinition".into());
        }
    }
    for key in ["temperature", "top_p", "parallel_tool_calls"] {
        if let Some(setting) = object.get(key).filter(|v| !v.is_null()) {
            canonical[key] = setting.clone();
        }
    }
    if let Some(effort) = object.get("reasoning_effort").filter(|v| !v.is_null()) {
        if !effort.is_string() {
            return Err(INVALID.into());
        }
        canonical["reasoning"] = json!({"effort":effort});
    }
    if let Some(format) = object.get("response_format").filter(|v| !v.is_null()) {
        canonical["text"] = json!({"format":text_format(format)?});
    }
    let mut body = if target == ModelProtocol::Responses {
        canonical
    } else {
        chat::request_from_responses(target, &canonical)?
    };
    if target == ModelProtocol::AnthropicMessages {
        if let Some(stops) = stops(object)? {
            body["stop_sequences"] = json!(stops);
        }
    }
    Ok(TranslatedRequest {
        body,
        input,
        namespaces: Default::default(),
        response_options: ResponseOptions {
            chat_include_usage: value["stream_options"]["include_usage"]
                .as_bool()
                .unwrap_or(false),
            ..Default::default()
        },
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| INVALID.into())
}

fn reject_private(value: &Value) -> Result<(), String> {
    if value.as_object().is_some_and(|o| {
        [
            "signature",
            "encrypted_content",
            "redacted_thinking",
            "thinking",
            "reasoning_details",
            "audio",
            "reasoning_content",
            "reasoning",
        ]
        .iter()
        .any(|key| o.get(*key).is_some_and(|v| !v.is_null()))
    }) {
        return Err(UNSUPPORTED.into());
    }
    Ok(())
}

fn text_format(value: &Value) -> Result<Value, String> {
    match value["type"].as_str() {
        Some("text" | "json_object") => Ok(json!({"type":value["type"]})),
        Some("json_schema") => {
            let schema = value["json_schema"].as_object().ok_or(INVALID)?;
            let name = schema
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 64
                        && name
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                })
                .ok_or(INVALID)?;
            let definition = schema
                .get("schema")
                .filter(|schema| schema.is_object())
                .ok_or(INVALID)?;
            if schema
                .get("strict")
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
                || schema
                    .get("description")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                return Err(INVALID.into());
            }
            let mut format = json!({"type":"json_schema","name":name,"schema":definition});
            for key in ["strict", "description"] {
                if let Some(value) = schema.get(key).filter(|v| !v.is_null()) {
                    format[key] = value.clone();
                }
            }
            Ok(format)
        }
        _ => Err(UNSUPPORTED.into()),
    }
}

fn validate_options(object: &Map<String, Value>, target: ModelProtocol) -> Result<(), String> {
    for key in ["stream", "parallel_tool_calls"] {
        if object
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_boolean())
        {
            return Err(INVALID.into());
        }
    }
    for key in ["max_tokens", "max_completion_tokens"] {
        if object
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_u64().is_none_or(|n| n == 0))
        {
            return Err(INVALID.into());
        }
    }
    if let (Some(a), Some(b)) = (
        object.get("max_tokens").filter(|v| !v.is_null()),
        object.get("max_completion_tokens").filter(|v| !v.is_null()),
    ) {
        if a != b {
            return Err(INVALID.into());
        }
    }
    for key in ["temperature", "top_p"] {
        if object
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_number())
        {
            return Err(INVALID.into());
        }
    }
    if object
        .get("n")
        .is_some_and(|v| !v.is_null() && v.as_u64() != Some(1))
    {
        return Err(UNSUPPORTED.into());
    }
    if object
        .get("modalities")
        .is_some_and(|v| !v.is_null() && v != &json!(["text"]))
    {
        return Err(UNSUPPORTED.into());
    }
    for key in [
        "audio",
        "prediction",
        "functions",
        "function_call",
        "reasoning_details",
        "encrypted_content",
        "signature",
    ] {
        if object.get(key).is_some_and(|v| !v.is_null()) {
            return Err(UNSUPPORTED.into());
        }
    }
    if object
        .get("logprobs")
        .is_some_and(|v| !v.is_null() && v != &json!(false))
        || object
            .get("top_logprobs")
            .is_some_and(|v| !v.is_null() && v != &json!(0))
        || object
            .get("logit_bias")
            .is_some_and(|v| !v.is_null() && v != &json!({}))
    {
        return Err(UNSUPPORTED.into());
    }
    for key in ["frequency_penalty", "presence_penalty"] {
        if object
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0))
        {
            return Err(UNSUPPORTED.into());
        }
    }
    if let Some(options) = object.get("stream_options").filter(|v| !v.is_null()) {
        let options = options.as_object().ok_or(INVALID)?;
        if options.keys().any(|key| key != "include_usage")
            || options
                .get("include_usage")
                .is_some_and(|v| !v.is_boolean())
        {
            return Err(UNSUPPORTED.into());
        }
    }
    if target == ModelProtocol::Responses && stops(object)?.is_some() {
        return Err(UNSUPPORTED.into());
    }
    Ok(())
}

fn stops(object: &Map<String, Value>) -> Result<Option<Vec<String>>, String> {
    let Some(value) = object.get("stop").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let stops = match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(values) => values
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or(INVALID))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(INVALID.into()),
    };
    if stops.len() > 4 || stops.iter().any(String::is_empty) {
        return Err(INVALID.into());
    }
    Ok((!stops.is_empty()).then_some(stops))
}

fn content(role: &str, value: Option<&Value>, has_calls: bool) -> Result<Value, String> {
    match value {
        Some(Value::String(s)) => Ok(json!(s)),
        None | Some(Value::Null) if role == "assistant" && has_calls => Ok(json!([])),
        Some(Value::Array(parts)) => Ok(json!(parts.iter().map(|part| {
            reject_private(part)?;
            match part["type"].as_str() {
                Some("text") => Ok(json!({"type":if role == "assistant" {"output_text"} else {"input_text"},"text":part["text"].as_str().ok_or(INVALID)?})),
                Some("image_url") if role == "user" => {
                    let url = string(&part["image_url"],"url")?;
                    if !url.starts_with("https://") && !url.starts_with("http://") && !url.starts_with("data:image/") { return Err(INVALID.into()); }
                    let mut mapped = json!({"type":"input_image","image_url":url});
                    if let Some(detail) = part["image_url"].get("detail") {
                        if !matches!(detail.as_str(), Some("auto" | "low" | "high")) { return Err(INVALID.into()); }
                        mapped["detail"] = detail.clone();
                    }
                    Ok(mapped)
                }
                _ => Err(UNSUPPORTED.into()),
            }
        }).collect::<Result<Vec<Value>, String>>()?)),
        _ => Err(INVALID.into()),
    }
}

fn tool_output(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                reject_private(part)?;
                if part["type"] != "text" {
                    return Err(UNSUPPORTED.to_owned());
                }
                part["text"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| INVALID.to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| parts.join("\n")),
        _ => Err(INVALID.into()),
    }
}

pub(super) fn response(value: &Value) -> Result<Value, String> {
    let invalid = "protocol.errors.invalidResponse";
    let status = value["status"].as_str().ok_or(invalid)?;
    if !matches!(status, "completed" | "incomplete") || !value["error"].is_null() {
        return Err(invalid.into());
    }
    let output = value["output"].as_array().ok_or(invalid)?;
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut refusal = String::new();
    let mut calls = Vec::new();
    let mut call_ids = std::collections::HashSet::new();
    for item in output {
        if has_private_output(item) {
            return Err(UNSUPPORTED.into());
        }
        match item["type"].as_str() {
            Some("message") if item["role"] == "assistant" => {
                for part in item["content"].as_array().ok_or(invalid)? {
                    match part["type"].as_str() {
                        Some("output_text") => text.push_str(part["text"].as_str().ok_or(invalid)?),
                        Some("refusal") => {
                            refusal.push_str(part["refusal"].as_str().ok_or(invalid)?)
                        }
                        _ => return Err(invalid.into()),
                    }
                }
            }
            Some("function_call") => {
                if status != "completed"
                    || !responses::valid_function_call(item)
                    || !call_ids.insert(item["call_id"].as_str().ok_or(invalid)?)
                {
                    return Err(invalid.into());
                }
                calls.push(json!({"id":item["call_id"],"type":"function","function":{"name":item["name"],"arguments":item["arguments"]}}));
            }
            Some("reasoning") => {
                for part in item["summary"].as_array().into_iter().flatten() {
                    if part["type"] != "summary_text" {
                        return Err(invalid.into());
                    }
                    reasoning.push_str(part["text"].as_str().ok_or(invalid)?);
                }
            }
            _ => return Err(invalid.into()),
        }
    }
    let finish = if status == "incomplete" {
        match value["incomplete_details"]["reason"].as_str() {
            Some("max_output_tokens" | "max_tokens") => "length",
            Some("content_filter") => "content_filter",
            _ => return Err(invalid.into()),
        }
    } else if !calls.is_empty() {
        "tool_calls"
    } else if !refusal.is_empty() {
        "content_filter"
    } else {
        "stop"
    };
    let mut message = json!({"role":"assistant","content":if text.is_empty() {Value::Null} else {json!(text)},"refusal":if refusal.is_empty() {Value::Null} else {json!(refusal)}});
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    Ok(
        json!({"id":value["id"].as_str().unwrap_or("chatcmpl-adapter"),"object":"chat.completion",
        "created":value["created_at"].as_u64().unwrap_or(0),"model":value["model"].as_str().unwrap_or_default(),
        "choices":[{"index":0,"message":message,"logprobs":null,"finish_reason":finish}],"usage":usage(&value["usage"])}),
    )
}

/// Legacy Messages -> Responses projection is permissive about new block
/// types. The new Chat direction cannot silently turn hosted/private output
/// into an empty successful message.
pub(super) fn validate_upstream_response(
    upstream: ModelProtocol,
    value: &Value,
) -> Result<(), String> {
    if upstream != ModelProtocol::AnthropicMessages || super::is_responses_body(value) {
        return Ok(());
    }
    let blocks = value["content"]
        .as_array()
        .ok_or("protocol.errors.invalidResponse")?;
    if blocks.iter().any(|block| {
        !matches!(
            block["type"].as_str(),
            Some("text" | "thinking" | "tool_use")
        ) || has_private_output(block)
    }) {
        return Err("protocol.errors.unsupportedContent".into());
    }
    Ok(())
}

pub(super) fn canonical_response(translation: Translation, value: &Value) -> Result<Value, String> {
    validate_upstream_response(translation.to, value)?;
    if super::is_responses_body(value) {
        return Ok(value.clone());
    }
    let mut canonical = responses::response_body(translation, value)?;
    if translation.to == ModelProtocol::AnthropicMessages {
        // Messages counts newly consumed, cached-read and cached-write input
        // separately. Chat's prompt_tokens is their total, with read-cache
        // details reported independently rather than subtracted a second time.
        let usage = &value["usage"];
        let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        let input = usage["input_tokens"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(read)
            .saturating_add(write);
        let output = usage["output_tokens"].as_u64().unwrap_or(0);
        canonical["usage"] = json!({"input_tokens":input,"output_tokens":output,"total_tokens":input.saturating_add(output),
            "input_tokens_details":{"cached_tokens":read}});
    }
    Ok(canonical)
}

/// Source-bound output cannot be interpreted as portable plain reasoning.
pub(super) fn has_private_output(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "signature" | "encrypted_content" | "redacted_thinking" | "reasoning_details"
            ) && !value.is_null())
                || (matches!(
                    key.as_str(),
                    "output"
                        | "content"
                        | "summary"
                        | "item"
                        | "part"
                        | "delta"
                        | "response"
                        | "content_block"
                        | "message"
                ) && has_private_output(value))
        }),
        Value::Array(values) => values.iter().any(has_private_output),
        _ => false,
    }
}

pub(super) fn usage(value: &Value) -> Value {
    let input = value["input_tokens"].as_u64().unwrap_or(0);
    let output = value["output_tokens"].as_u64().unwrap_or(0);
    let mut usage = json!({"prompt_tokens":input,"completion_tokens":output,"total_tokens":value["total_tokens"].as_u64().unwrap_or(input.saturating_add(output))});
    for (from, to) in [
        ("input_tokens_details", "prompt_tokens_details"),
        ("output_tokens_details", "completion_tokens_details"),
    ] {
        if value[from].is_object() {
            usage[to] = value[from].clone();
        }
    }
    usage
}
