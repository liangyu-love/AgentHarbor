//! Claude's Messages protocol translated through the existing Responses format.
//! Native Messages requests never enter this adapter.
use super::{chat, responses::valid_function_call, web_search, ResponseOptions};
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

const INVALID: &str = "protocol.errors.invalidRequest";
const UNSUPPORTED: &str = "protocol.errors.unsupportedContent";

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| INVALID.into())
}

fn image(block: &Value) -> Result<Value, String> {
    let source = &block["source"];
    let url = match string(source, "type")? {
        "url" => string(source, "url")?.to_owned(),
        "base64" => format!(
            "data:{};base64,{}",
            string(source, "media_type")?,
            string(source, "data")?
        ),
        _ => return Err(UNSUPPORTED.into()),
    };
    Ok(json!({"type":"input_image","image_url":url}))
}

fn document(block: &Value) -> Result<Value, String> {
    let source = &block["source"];
    match string(source, "type")? {
        "url" => Ok(json!({"type":"input_file","file_url":string(source,"url")?})),
        "base64" => Ok(json!({
            "type":"input_file",
            "filename":block.get("title").and_then(Value::as_str).unwrap_or("document.pdf"),
            "file_data":format!("data:{};base64,{}", string(source,"media_type")?, string(source,"data")?)
        })),
        _ => Err(UNSUPPORTED.into()),
    }
}

fn flush(input: &mut Vec<Value>, parts: &mut Vec<Value>, role: &str) {
    if !parts.is_empty() {
        input.push(json!({"role":role,"content":std::mem::take(parts)}));
    }
}

fn tool_result(block: &Value, target: ModelProtocol) -> Result<(Value, Vec<Value>), String> {
    let mut parts = Vec::new();
    match block.get("content") {
        Some(Value::String(text)) => parts.push(json!({"type":"input_text","text":text})),
        Some(Value::Array(items)) => {
            for item in items {
                parts.push(match string(item, "type")? {
                    "text" => json!({"type":"input_text","text":string(item, "text")?}),
                    "image" => image(item)?,
                    "document" => document(item)?,
                    _ => return Err(UNSUPPORTED.into()),
                });
            }
        }
        None => {}
        _ => return Err(INVALID.into()),
    }
    if block["is_error"] == true {
        parts.insert(0, json!({"type":"input_text","text":"Tool error:"}));
    }
    let mut attachments = Vec::new();
    let output = if target == ModelProtocol::ChatCompletions {
        attachments = parts
            .iter()
            .filter(|part| matches!(part["type"].as_str(), Some("input_image" | "input_file")))
            .cloned()
            .collect();
        json!(parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"))
    } else if parts.iter().all(|part| part["type"] == "input_text") {
        json!(parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"))
    } else {
        json!(parts)
    };
    Ok((
        json!({"type":"function_call_output","call_id":string(block,"tool_use_id")?,"output":output}),
        attachments,
    ))
}

pub(super) fn request_body(target: ModelProtocol, value: &Value) -> Result<Value, String> {
    request_with_options(target, value).map(|(body, _)| body)
}

pub(super) fn request_with_options(target: ModelProtocol, value: &Value) -> Result<(Value, ResponseOptions), String> {
    request_with_search(target, value, None)
}

pub(super) fn request_with_search(target: ModelProtocol, value: &Value, chat_search: Option<super::ChatSearch>) -> Result<(Value, ResponseOptions), String> {
    let model = string(value, "model")?;
    let messages = value["messages"].as_array().ok_or(INVALID)?;
    let max = value["max_tokens"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or(INVALID)?;
    let mut input = Vec::new();
    let mut options = ResponseOptions::default();
    for message in messages {
        let role = string(message, "role")?;
        // Current Claude Code adds dynamic system messages to the conversation.
        // Preserve their role and position instead of rejecting them or moving
        // them into the leading system prompt (which changes their scope).
        if !matches!(role, "user" | "assistant" | "system" | "developer") {
            return Err("protocol.errors.unsupportedMessageRole".into());
        }
        let text_type = if role == "assistant" {
            "output_text"
        } else {
            "input_text"
        };
        let mut parts = Vec::new();
        let mut chat_calls = Vec::new();
        let turn_start = input.len();
        match &message["content"] {
            Value::String(text) => parts.push(json!({"type":text_type,"text":text})),
            Value::Array(blocks) => {
                let chat_tool_results = role == "user"
                    && target == ModelProtocol::ChatCompletions
                    && blocks.iter().any(|block| block["type"] == "tool_result");
                if chat_tool_results {
                    // Chat Completions expects every tool reply to immediately follow
                    // the assistant tool_calls message. Defer ordinary user content
                    // and media attachments until all results from this Anthropic message are sent.
                    let mut attachments = Vec::new();
                    for block in blocks {
                        match string(block, "type")? {
                            "text" => parts
                                .push(json!({"type":"input_text","text":string(block,"text")?})),
                            "image" => parts.push(image(block)?),
                            "document" => parts.push(document(block)?),
                            "tool_result" => {
                                let (result, tool_attachments) = tool_result(block, target)?;
                                input.push(result);
                                attachments.extend(tool_attachments);
                            }
                            _ => return Err(UNSUPPORTED.into()),
                        }
                    }
                    parts.extend(attachments);
                } else {
                    for block in blocks {
                        match string(block, "type")? {
                            "text" => {
                                parts.push(json!({"type":text_type,"text":string(block,"text")?}))
                            }
                            "image" if role == "user" => parts.push(image(block)?),
                            "document" if role == "user" => parts.push(document(block)?),
                            "tool_use" if role == "assistant" => {
                                if target != ModelProtocol::ChatCompletions {
                                    flush(&mut input, &mut parts, role);
                                }
                                let call = json!({"type":"function_call","call_id":string(block,"id")?,
                                "name":string(block,"name")?,"arguments":block["input"].to_string()});
                                if !valid_function_call(&call) {
                                    return Err("protocol.errors.invalidToolArguments".into());
                                }
                                if target == ModelProtocol::ChatCompletions {
                                    chat_calls.push(call);
                                } else {
                                    input.push(call);
                                }
                            }
                            "tool_result" if role == "user" => {
                                flush(&mut input, &mut parts, role);
                                let (result, attachments) = tool_result(block, target)?;
                                input.push(result);
                                if !attachments.is_empty() {
                                    input.push(json!({"role":"user","content":attachments}));
                                }
                            }
                            "server_tool_use" if role == "assistant" && (target == ModelProtocol::Responses || chat_search.is_some()) => {
                                let result = blocks.iter().find(|r| r["type"] == "web_search_tool_result" && r["tool_use_id"] == block["id"])
                                    .ok_or(UNSUPPORTED)?;
                                flush(&mut input, &mut parts, role);
                                input.push(if target == ModelProtocol::Responses {
                                    web_search::history_input(block, result)?
                                } else {web_search::history_call(block, result)?});
                            }
                            "web_search_tool_result" if role == "assistant" && (target == ModelProtocol::Responses || chat_search.is_some()) => {
                                if !blocks.iter().any(|b| b["type"] == "server_tool_use" && b["id"] == block["tool_use_id"]) {
                                    return Err(INVALID.into());
                                }
                            }
                            // Signed Anthropic thinking cannot be replayed to another provider.
                            "thinking" | "redacted_thinking" if role == "assistant" => {}
                            _ => return Err(UNSUPPORTED.into()),
                        }
                    }
                }
            }
            _ => return Err(INVALID.into()),
        }
        flush(&mut input, &mut parts, role);
        // Chat has one assistant message per tool-call group, regardless of
        // how text and tool blocks were interleaved within the Messages turn.
        // A tool-only assistant turn still needs its own boundary. Otherwise
        // Chat conversion attaches its calls to the previous assistant turn.
        if !chat_calls.is_empty() && input.len() == turn_start {
            input.push(json!({"role":"assistant","content":[]}));
        }
        input.extend(chat_calls);
    }
    let mut response = json!({"model":model,"input":input,"max_output_tokens":max,
        "stream":value.get("stream").and_then(Value::as_bool).unwrap_or(false),"store":false});
    if let Some(system) = value.get("system") {
        let text = match system {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .map(|part| {
                    if part["type"] != "text" {
                        return Err(UNSUPPORTED.to_owned());
                    }
                    string(part, "text").map(str::to_owned)
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("\n"),
            _ => return Err(INVALID.into()),
        };
        response["instructions"] = json!(text);
    }
    if let Some(tools) = value.get("tools") {
        let mut names = std::collections::HashSet::new();
        let programmatic = tools.as_array().is_some_and(|tools| web_search::programmatic_callers(tools));
        response["tools"] = json!(tools
            .as_array()
            .ok_or(INVALID)?
            .iter()
            .map(|tool| {
                if !names.insert(string(tool, "name")?) { return Err(INVALID.into()); }
                if web_search::is_search(tool) {
                    if options.search.is_some() { return Err("protocol.errors.unsupportedSearchOptions".into()); }
                    let (search, mapped) = web_search::prepare(if chat_search.is_some() { ModelProtocol::Responses } else { target }, tool, programmatic)?;
                    options.search = Some(search);
                    return Ok(mapped);
                }
                if tool.get("type").is_some_and(|kind| kind != "custom")
                    || !tool["input_schema"].is_object()
                {
                    return Err("protocol.errors.unsupportedToolDefinition".to_owned());
                }
                Ok(json!({"type":"function","name":string(tool,"name")?,
                "description":tool.get("description").and_then(Value::as_str).unwrap_or(""),
                "parameters":tool["input_schema"],"strict":false}))
            })
            .collect::<Result<Vec<_>, String>>()?);
    }
    if let Some(search) = &options.search {
        response["include"] = json!(["web_search_call.action.sources"]);
        if let Some(max) = search.max_uses { response["max_tool_calls"] = json!(max); }
        options.search_budget_approximate = target == ModelProtocol::ChatCompletions && search.max_uses.is_some();
    }
    if let Some(choice) = value.get("tool_choice") {
        response["tool_choice"] = match string(choice, "type")? {
            "auto" => json!("auto"),
            "any" => json!("required"),
            "none" => json!("none"),
            "tool" if options.search.as_ref().is_some_and(|s| Some(s.name.as_str()) == choice["name"].as_str()) => {
                // Responses gateways commonly accept the standard `required` choice,
                // but reject a search-specific object. Keep only the selected tool
                // so `required` still means this particular hosted search.
                response["tools"].as_array_mut().ok_or(INVALID)?.retain(|tool| tool["type"] == "web_search");
                json!("required")
            },
            "tool" => json!({"type":"function","name":string(choice,"name")?}),
            _ => return Err(UNSUPPORTED.into()),
        };
        if choice["disable_parallel_tool_use"] == true {
            response["parallel_tool_calls"] = json!(false);
        }
        if choice["type"] == "none" { options.search = None; }
    }
    for key in ["temperature", "top_p"] {
        if let Some(setting) = value.get(key) {
            response[key] = setting.clone();
        }
    }
    if let Some(effort) = super::reasoning::messages_effort(value) {
        response["reasoning"] = json!({"effort":effort});
    }
    if let Some(format) = value.get("output_config").and_then(|c| c.get("format")) {
        if format["type"] != "json_schema" || !format["schema"].is_object() {
            return Err(UNSUPPORTED.into());
        }
        response["text"] = json!({"format":{"type":"json_schema","name":"response","schema":format["schema"],"strict":true}});
    }
    let stops = value
        .get("stop_sequences")
        .and_then(Value::as_array)
        .filter(|stops| !stops.is_empty());
    match target {
        ModelProtocol::Responses => {
            // Responses has no equivalent stop-sequence parameter.
            if stops.is_some() {
                return Err(UNSUPPORTED.into());
            }
            Ok((response, options))
        }
        ModelProtocol::ChatCompletions => {
            let mut chat = chat::request_from_responses_with_search(target, &response, chat_search)?;
            if let Some(stops) = stops {
                chat["stop"] = json!(stops);
            }
            if let Some(format) = response.get("text").and_then(|t| t.get("format")) {
                chat["response_format"] = json!({"type":"json_schema","json_schema":{
                    "name":format["name"],"schema":format["schema"],"strict":true}});
            }
            Ok((chat, options))
        }
        _ => Err("protocol.errors.unsupportedTranslation".into()),
    }
}

pub(super) fn usage(value: &Value) -> Value {
    let input = value["input_tokens"].as_u64().unwrap_or(0);
    let cached = value["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap_or(0)
        .min(input);
    let mut usage = json!({"input_tokens":input - cached,"output_tokens":value["output_tokens"].as_u64().unwrap_or(0),
        "cache_read_input_tokens":cached,"cache_creation_input_tokens":0});
    if let Some(search) = value.get("server_tool_use") { usage["server_tool_use"] = search.clone(); }
    usage
}

pub(super) fn response_body(value: &Value) -> Result<Value, String> {
    response_body_with_options(value, &ResponseOptions::default())
}

pub(super) fn response_body_with_options(value: &Value, options: &ResponseOptions) -> Result<Value, String> {
    let output = web_search::enrich_output(value["output"].as_array().ok_or("protocol.errors.invalidResponse")?, options.search.as_ref().and_then(|s| s.max_uses));
    response_body_from_output(value, options, &output)
}

pub(super) fn response_body_from_output(value: &Value, options: &ResponseOptions, output: &[Value]) -> Result<Value, String> {
    let invalid = "protocol.errors.invalidResponse";
    let status = value["status"].as_str().ok_or(invalid)?;
    if !matches!(status, "completed" | "incomplete") || !value["error"].is_null() {
        return Err(invalid.into());
    }
    let mut content = Vec::new();
    let mut has_tools = false;
    let mut refusal = false;
    let mut searches = 0_u64;
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                for part in item["content"].as_array().ok_or(invalid)? {
                    let text = match part["type"].as_str() {
                        Some("output_text") => part["text"].as_str().ok_or(invalid)?,
                        Some("refusal") => {
                            refusal = true;
                            part["refusal"].as_str().ok_or(invalid)?
                        }
                        _ => return Err(invalid.into()),
                    };
                    content.push(json!({"type":"text","text":format!("{text}{}", web_search::citation_suffix(part))}));
                }
            }
            Some("function_call") => {
                if status != "completed" || !valid_function_call(item) {
                    return Err(invalid.into());
                }
                has_tools = true;
                let input: Value = serde_json::from_str(item["arguments"].as_str().ok_or(invalid)?)
                    .map_err(|_| invalid)?;
                content.push(json!({"type":"tool_use","id":item["call_id"],"name":item["name"],"input":input}));
            }
            // Provider reasoning cannot be represented as signed Anthropic thinking.
            Some("reasoning") => {}
            Some("web_search_call") => {
                content.extend(web_search::blocks(item, options.search.as_ref(), searches)?);
                searches += 1;
            }
            _ => return Err(invalid.into()),
        }
    }
    let stop = if status == "incomplete" {
        match value["incomplete_details"]["reason"].as_str() {
            Some("max_output_tokens") => "max_tokens",
            Some("content_filter") => "refusal",
            _ => return Err(invalid.into()),
        }
    } else if has_tools {
        "tool_use"
    } else if refusal {
        "refusal"
    } else {
        "end_turn"
    };
    if status == "completed" && content.is_empty() {
        return Err(invalid.into());
    }
    let mut usage = usage(&value["usage"]);
    if searches > 0 { usage["server_tool_use"] = json!({"web_search_requests":searches}); }
    Ok(
        json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant",
        "model":value["model"].as_str().unwrap_or_default(),"content":content,
        "stop_reason":stop,"stop_sequence":null,"usage":usage}),
    )
}
