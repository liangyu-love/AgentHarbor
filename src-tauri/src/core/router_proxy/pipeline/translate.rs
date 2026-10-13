//! The request body in the upstream's protocol: endpoint mapping, the wire
//! translation through the protocol layer, and the native preparation a
//! relay needs when no translation applies.
use super::super::*;

/// The protocol a destination's request body is being rewritten into.
pub(in crate::core::router_proxy) fn translation_target(destination: &Destination) -> Option<router::ModelProtocol> {
    match destination {
        Destination::Provider {
            translation: Some(translation),
            ..
        } => Some(translation.to),
        _ => None,
    }
}

/// Map a caller-facing endpoint onto the one the target protocol serves.
///
/// Only the protocol's core endpoints are rewritten; sub-paths such as
/// `responses/{id}` keep their tail so follow-up calls still address the right
/// resource.
pub(in crate::core::router_proxy) fn translate_endpoint(suffix: &str, target: router::ModelProtocol) -> String {
    let replacement = match target {
        router::ModelProtocol::Responses => "responses",
        router::ModelProtocol::ChatCompletions => "chat/completions",
        router::ModelProtocol::AnthropicMessages => "messages",
    };
    for source in ["responses", "chat/completions", "messages"] {
        if let Some(tail) = suffix.strip_prefix(source) {
            return format!("{replacement}{tail}");
        }
    }
    suffix.to_owned()
}

/// The path below the upstream's `/v1/`, rewritten for a translated protocol.
pub(in crate::core::router_proxy) fn upstream_suffix(
    segments: &[&str],
    destination: &Destination,
    required_protocol: Option<router::ModelProtocol>,
) -> ForwardResult<String> {
    let suffix = match segments {
        ["claude-desktop", "v1", rest @ ..] => rest.join("/"),
        ["p", _, "v1", rest @ ..] | ["codex", "v1", rest @ ..] | ["v1", rest @ ..] => {
            rest.join("/")
        }
        _ => String::new(),
    };
    // A translated request must reach the endpoint the provider's protocol
    // actually serves: `responses` exists only on a Responses provider, so the
    // path has to follow the protocol that the body was rewritten into.
    if required_protocol == Some(router::ModelProtocol::AnthropicMessages)
        && translation_target(destination).is_some()
        && suffix != "messages"
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "this endpoint requires a native Anthropic Messages provider",
        ));
    }
    // Only the core create endpoint is translated. Sub-resources such as
    // `responses/compact` or `responses/{id}` have no equivalent on another
    // protocol, and rewriting them would produce an endpoint that does not exist.
    if required_protocol == Some(router::ModelProtocol::Responses)
        && translation_target(destination).is_some()
        && suffix != "responses"
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "this endpoint requires a native Responses provider",
        ));
    }
    Ok(match translation_target(destination) {
        Some(protocol) => translate_endpoint(&suffix, protocol),
        None => suffix,
    })
}

/// The body to send upstream, and for a translated request the canonical
/// Responses input (excluding request-scoped instructions) to remember afterwards.
#[derive(Debug)]
pub(in crate::core::router_proxy) struct PreparedForward {
    pub(in crate::core::router_proxy) body: Option<Bytes>,
    pub(in crate::core::router_proxy) input: Vec<Value>,
    pub(in crate::core::router_proxy) namespaces: protocol::tool_namespaces::ToolNamespaces,
    pub(in crate::core::router_proxy) response_options: protocol::ResponseOptions,
}

impl PreparedForward {
    pub(in crate::core::router_proxy) fn native(body: Option<Bytes>) -> Self {
        let required = body.as_deref().and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .is_some_and(|value| protocol::web_search::required(&value));
        Self { body, input: Vec::new(), namespaces: Default::default(),
            response_options: protocol::ResponseOptions { search_required:required, ..Default::default() },
        }
    }
}

/// Responses tool types other protocols can carry: functions, and the
/// namespaced and freeform ones the translation itself handles.
const PORTABLE_TOOLS: [&str; 3] = ["function", "namespace", "custom"];
/// History items of hosted tools, which only a Responses provider can read.
const HOSTED_ITEMS: [&str; 3] = ["image_generation_call", "tool_search_call", "tool_search_output"];

/// The request without hosted tools, their tool choice and their history
/// items; `None` when it has none of them.
fn without_hosted_tools(value: &Value, keep_search: bool) -> Option<Value> {
    let mut stripped = value.clone();
    let mut changed = false;
    if let Some(tools) = stripped.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        tools.retain(|tool| tool["type"].as_str().is_some_and(|kind| PORTABLE_TOOLS.contains(&kind))
            || (keep_search && protocol::web_search::is_search(tool)));
        changed |= tools.len() != before;
    }
    if let Some(object) = stripped.as_object_mut() {
        let hosted_choice = object.get("tool_choice").and_then(Value::as_object)
            .and_then(|choice| choice.get("type")).and_then(Value::as_str)
            .is_some_and(|kind| !PORTABLE_TOOLS.contains(&kind) && kind != "allowed_tools"
                && !(keep_search && kind.starts_with("web_search")));
        if hosted_choice {
            object.remove("tool_choice");
            changed = true;
        }
        if object.get("tools").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            object.remove("tools");
            if object.get("tool_choice").is_some() {
                object.remove("tool_choice");
            }
        }
    }
    if let Some(items) = stripped.get_mut("input").and_then(Value::as_array_mut) {
        let before = items.len();
        items.retain(|item| item["type"].as_str().is_none_or(|kind| !HOSTED_ITEMS.contains(&kind)));
        changed |= items.len() != before;
    }
    changed.then_some(stripped)
}

/// A native endpoint can advertise the protocol without implementing search.
/// Retry only a specifically rejected optional declaration, never a forced call.
pub(in crate::core::router_proxy) fn optional_native_search(body: &[u8]) -> Option<Bytes> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let tools = value["tools"].as_array().cloned().unwrap_or_default();
    let searches: Vec<_> = tools.iter().filter(|tool| protocol::web_search::is_search(tool)).collect();
    let forced = protocol::web_search::required(&value);
    if forced || (searches.is_empty() && value.get("web_search_options").is_none()) { return None; }
    let object = value.as_object_mut()?;
    object.remove("web_search_options");
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| !protocol::web_search::is_search(tool));
        if tools.is_empty() {
            object.remove("tools");
            object.remove("tool_choice");
        }
    }
    serde_json::to_vec(&value).ok().map(Bytes::from)
}

/// Gate retries on a single structured error, including gateways that wrap an
/// HTTP 400 in an SSE data frame. Multiple payloads (possibly executed tools)
/// cannot parse as one JSON object and must never trigger a search retry.
fn search_error(body: &[u8]) -> Option<Value> {
    serde_json::from_slice(body).ok().or_else(|| {
        let data = std::str::from_utf8(body).ok()?.trim_start_matches('\u{feff}').lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>().join("\n");
        serde_json::from_str(&data).ok()
    })
}

pub(in crate::core::router_proxy) fn rejected_native_search(status: StatusCode, body: &[u8]) -> bool {
    if !matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY) { return false; }
    let Some(value) = search_error(body) else { return false; };
    let error = &value["error"];
    // MiMo puts the reason in `param`: "web search tool found in the request
    // body, but webSearchEnabled is false" under message "Param Incorrect".
    let text = ["code", "type", "message", "param"].iter()
        .filter_map(|key| error[*key].as_str()).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    if ["permission", "unauthorized", "api key", "api_key", "authentication", "quota", "rate_limit", "rate limit", "billing", "too_many_requests"].iter()
        .any(|word| text.contains(word)) { return false; }
    ["web_search", "web search", "websearch"].iter().any(|word| text.contains(word))
        && ["unsupported", "not supported", "unknown tool", "unknown parameter", "unknown field", "unrecognized",
            "not enabled", "enabled is false", "is disabled", "not available"].iter().any(|word| text.contains(word))
}

/// Some Gemini relays cannot combine hosted and client tools. Their standard
/// protocol ignores the native Google ToolConfig flag; separate execution on
/// the same provider preserves both capabilities without assuming a backend.
pub(in crate::core::router_proxy) fn rejected_mixed_search(status: StatusCode, body: &[u8]) -> bool {
    if !matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY) {return false;}
    let Some(value) = search_error(body) else {return false;};
    let message = value["error"]["message"].as_str().unwrap_or_default().to_ascii_lowercase();
    message.trim().trim_end_matches('.') == "please enable tool_config.include_server_side_tool_invocations to use built-in tools with function calling"
}

pub(in crate::core::router_proxy) fn upstream_body(
    destination: &Destination,
    body: Option<Bytes>) -> ForwardResult<PreparedForward> {
    let (
        Destination::Provider {
            provider,
            model_override,
            upstream_protocol,
            translation,
            ..
        },
        Some(bytes),
    ) = (destination, &body)
    else {
        return Ok(PreparedForward::native(body));
    };
    match translation {
        // A translated request is rebuilt for the provider's protocol,
        // with the slug prefix replaced by the provider's real model.
        Some(translation) => {
            let mut value: Value = serde_json::from_slice(bytes)
                .map_err(|_| (StatusCode::BAD_REQUEST, "request body is not JSON"))?;
            if let Some(model) = model_override.as_deref() {
                value["model"] = json!(model);
            }
            let adapter = WireAdapter::new(*upstream_protocol);
            let chat_search = provider.chat_search.resolve(&provider.base_url,
                value["model"].as_str().unwrap_or(&provider.model));
            let had_search = value["tools"].as_array().into_iter().flatten().any(protocol::web_search::is_search);
            let forced_search = protocol::web_search::required(&value);
            let mut search_downgraded = false;
            let adapted = match adapter.encode_request_with_chat_search(translation.from, &value, chat_search) {
                // Codex offers its built-in tools (web search, image generation,
                // tool search) with every request. A relay on another protocol
                // cannot run them, so it gets the request without them rather
                // than a refusal of the whole turn.
                Err(error) if translation.from == router::ModelProtocol::Responses
                    && matches!(error.code.as_str(), "protocol.errors.unsupportedHostedTool" | "protocol.errors.unsupportedContent" | "protocol.errors.unsupportedSearchOptions")
                    =>
                {
                    let keep_search = error.code != "protocol.errors.unsupportedSearchOptions"
                        && (*upstream_protocol == router::ModelProtocol::AnthropicMessages || chat_search.is_some());
                    search_downgraded = had_search && !keep_search;
                    match without_hosted_tools(&value, keep_search) {
                        _ if (forced_search && !keep_search)
                            || (value["tool_choice"] == "none" && chat_search == Some(protocol::ChatSearch::OpenAi)) => Err(error),
                        Some(stripped) => adapter.encode_request_with_chat_search(translation.from, &stripped, chat_search),
                        None => Err(error),
                    }
                }
                Err(error) if translation.from == router::ModelProtocol::AnthropicMessages && had_search && !forced_search
                    && !(value["tool_choice"]["type"] == "none" && chat_search == Some(protocol::ChatSearch::OpenAi))
                    && matches!(error.code.as_str(), "protocol.errors.unsupportedHostedTool" | "protocol.errors.unsupportedSearchOptions") => {
                    let mut stripped = value.clone();
                    stripped["tools"].as_array_mut().unwrap().retain(|tool| !protocol::web_search::is_search(tool));
                    if stripped["tools"].as_array().is_some_and(Vec::is_empty) {
                        stripped.as_object_mut().unwrap().remove("tools");
                        stripped.as_object_mut().unwrap().remove("tool_choice");
                    }
                    search_downgraded = true;
                    adapter.encode_request_with_chat_search(translation.from, &stripped, chat_search)
                }
                other => other,
            };
            let adapted = adapted.map_err(|error| {
                (
                    StatusCode::BAD_REQUEST,
                    match error.code.as_str() {
                        "protocol.errors.unknownPreviousResponse" =>
                            "previous_response_id is unknown or expired; resend the full conversation",
                        "protocol.errors.unsupportedMessageRole" =>
                            "cannot translate request: unsupported conversation role",
                        "protocol.errors.unsupportedToolDefinition" =>
                            "cannot translate request: unsupported hosted/custom tool, tool choice or function schema; select a provider using the caller's native protocol",
                        "protocol.errors.unsupportedHostedTool" =>
                            "this target has no compatible hosted search contract; configure a supported native search format or an explicitly selected search backend",
                        "protocol.errors.unsupportedSearchOptions" =>
                            "cannot translate WebSearch options without loss: unsupported tool version, programmatic caller, domain exclusion or result policy",
                        "protocol.errors.unansweredToolCall" =>
                            "cannot translate conversation: tool calls need matching tool results before the next message",
                        "protocol.errors.unsupportedNamespacedTool" =>
                            "cannot translate namespaced tools; select a Responses provider or disable namespaced agent/MCP tools in the caller",
                        "protocol.errors.unsupportedCustomTool" =>
                            "cannot translate custom/freeform tools; select a Responses provider or use function tools in the caller",
                        "protocol.errors.invalidToolArguments" =>
                            "cannot translate request: tool arguments must be a JSON object",
                        "protocol.errors.unsupportedContent" =>
                            "cannot translate request without loss: unsupported content block, history item or output format; select a provider using the caller's protocol",
                        "protocol.errors.invalidRequest" =>
                            "cannot translate request: invalid request fields or message structure",
                        _ => "cannot translate request",
                    },
                )
            })?;
            let AdapterRequest::Translated(mut rewritten) = adapted else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "invalid provider protocol translation",
                ));
            };
            rewritten.response_options.search_downgraded = search_downgraded;
            rewritten.response_options.search_required = forced_search;
            if translation.from == router::ModelProtocol::Responses {
                // Check the expanded history too: a cached continuation may carry
                // items that the target protocol cannot represent without loss.
                session_compat::validate_translated_input(&rewritten.input)?;
            }
            let encoded = serde_json::to_vec(&rewritten.body).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cannot re-encode request",
                )
            })?;
            Ok(PreparedForward { body: Some(Bytes::from(encoded)), input: rewritten.input, namespaces: rewritten.namespaces, response_options: rewritten.response_options,
            })
        }
        None if model_override.is_some()
            || *upstream_protocol == router::ModelProtocol::ChatCompletions =>
        {
            let prepared =
                prepare_relay_body(bytes, model_override.as_deref(), *upstream_protocol)?;
            Ok(PreparedForward::native(Some(prepared)))
        }
        None => Ok(PreparedForward::native(body)),
    }
}

pub(in crate::core::router_proxy) fn prepare_relay_body(
    body: &Bytes,
    model_id: Option<&str>,
    protocol: router::ModelProtocol,
) -> Result<Bytes, (StatusCode, &'static str)> {
    let mut value: Value = serde_json::from_slice(body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "request body is not JSON"))?;
    let object = value
        .as_object_mut()
        .ok_or((StatusCode::BAD_REQUEST, "request body is not an object"))?;
    let mut changed = false;
    if let Some(model_id) = model_id {
        if object.get("model").and_then(Value::as_str) != Some(model_id) {
        object.insert("model".to_owned(), json!(model_id));
            changed = true;
        }
    }
    if protocol == router::ModelProtocol::ChatCompletions
        && object.get("stream") == Some(&Value::Bool(true))
    {
        let options = object.entry("stream_options").or_insert_with(|| json!({}));
        if options.is_null() {
            *options = json!({});
        }
        let options = options
            .as_object_mut()
            .ok_or((StatusCode::BAD_REQUEST, "stream_options is not an object"))?;
        if options.get("include_usage") != Some(&Value::Bool(true)) {
            options.insert("include_usage".into(), json!(true));
            changed = true;
        }
    }
    if !changed {
        return Ok(body.clone());
    }
    let bytes = serde_json::to_vec(&value).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot re-encode request",
        )
    })?;
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_search_recovery_requires_the_specific_configuration_error() {
        let message = "Please enable tool_config.include_server_side_tool_invocations to use Built-in tools with Function calling.";
        for (status,text,expected) in [(400,message,true),(422,message,true),(401,message,false),(429,message,false),(503,message,false),
            (400,"invalid function schema",false),(400,"tool_config.include_server_side_tool_invocations is not supported",false)] {
            assert_eq!(rejected_mixed_search(StatusCode::from_u16(status).unwrap(),
                json!({"error":{"message":text}}).to_string().as_bytes()),expected);
        }
        assert!(!rejected_mixed_search(StatusCode::BAD_REQUEST,message.as_bytes()));
    }

    #[test]
    fn optional_search_retry_is_specific_and_preserves_client_tools() {
        let source = json!({"tools":[{"type":"web_search"},{"type":"function","function":{"name":"web_search","parameters":{}}}],
            "tool_choice":"auto","messages":[{"role":"user","content":"hi"}]});
        let stripped: Value = serde_json::from_slice(&optional_native_search(source.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(stripped["tools"], json!([source["tools"][1]]));
        assert_eq!(stripped["messages"], source["messages"]);
        for choice in [json!({"type":"web_search"}),json!({"type":"allowed_tools","mode":"required","tools":[{"type":"web_search"}]})] {
            let mut forced = source.clone(); forced["tool_choice"] = choice;
            assert!(optional_native_search(forced.to_string().as_bytes()).is_none());
        }
        for (status, message, expected) in [
            (400,"web_search is not supported",true), (422,"unknown tool web_search",true),
            (400,"web_search quota exceeded: unsupported account",false),
            (400,"web_search permission denied; unsupported plan",false),
            (401,"web_search is not supported",false), (429,"web_search is not supported",false),
            (503,"web_search is not supported",false), (400,"invalid function schema",false),
        ] {
            assert_eq!(rejected_native_search(StatusCode::from_u16(status).unwrap(),
                json!({"error":{"message":message}}).to_string().as_bytes()),expected,"{status}: {message}");
        }
        let error = json!({"error":{"code":"responses_feature_not_supported","message":"tool type 'web_search' is not supported by this gateway phase.","type":"unsupported_feature"}});
        for wire in [format!("data:{error}\n\n"),format!("\u{feff}event: error\r\ndata: {error}\r\n\r\n")] {
            assert!(rejected_native_search(StatusCode::BAD_REQUEST,wire.as_bytes()));
            assert!(!rejected_native_search(StatusCode::UNAUTHORIZED,wire.as_bytes()));
            assert!(!rejected_native_search(StatusCode::TOO_MANY_REQUESTS,wire.as_bytes()));
        }
        let partial = format!("data: {{\"type\":\"web_search_call\",\"status\":\"completed\"}}\n\ndata: {error}\n\n");
        assert!(!rejected_native_search(StatusCode::BAD_REQUEST,partial.as_bytes()));
        // Captured from MiMo Chat with the account's search plugin switched off.
        let mimo = br#"{"error":{"code":"400","message":"Param Incorrect","param":"web search tool found in the request body, but webSearchEnabled is false","type":""}}"#;
        assert!(rejected_native_search(StatusCode::BAD_REQUEST,mimo));
        assert!(!rejected_native_search(StatusCode::BAD_REQUEST,br#"{"error":{"message":"Param Incorrect","param":"messages[0].content"}}"#));
    }

    fn destination(target: router::ModelProtocol) -> Destination {
        Destination::Provider {
            provider: Box::new(router::Provider::default()),
            upstream_protocol: target,
            model_override: Some("relay-model".into()),
            translation: protocol::Translation::between(router::ModelProtocol::Responses, target),
        }
    }

    #[test]
    fn review_claude_disabled_search_is_not_downgraded_to_always_search_chat() {
        let mut target = destination(router::ModelProtocol::ChatCompletions);
        if let Destination::Provider {provider,translation,..} = &mut target {
            provider.chat_search = protocol::ChatSearch::OpenAi;
            *translation = protocol::Translation::between(router::ModelProtocol::AnthropicMessages,router::ModelProtocol::ChatCompletions);
        }
        let request = json!({"model":"m","max_tokens":100,"messages":[{"role":"user","content":"hi"}],
            "tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"none"}});
        assert!(upstream_body(&target,Some(Bytes::from(request.to_string()))).is_err());
    }

    #[test]
    fn codex_hosted_tools_are_left_out_for_a_relay_on_another_protocol() {
        let codex = json!({"model": "relay::model", "stream": true,
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "find it"}]},
                {"type": "web_search_call", "id": "ws_1", "status": "completed", "action": {"type": "search", "query": "q"}},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "found"}]},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go on"}]}],
            "tools": [
                {"type": "function", "name": "exec", "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}},
                {"type": "web_search", "external_web_access": true},
                {"type": "image_generation"}],
            "tool_choice": "auto"});
        for target in [router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
            let prepared = upstream_body(&destination(target), Some(Bytes::from(codex.to_string())))
                .unwrap_or_else(|error| panic!("{target:?}: {error:?}"));
            let body: Value = serde_json::from_slice(&prepared.body.unwrap()).unwrap();
            let native_search = target == router::ModelProtocol::AnthropicMessages;
            assert_eq!(body["tools"].as_array().unwrap().len(), if native_search {2} else {1}, "{target:?}");
            let text = body.to_string();
            assert_eq!(text.contains("web_search"), native_search, "{target:?}: {text}");
            assert!(!text.contains("image_generation"), "{target:?}: {text}");
            assert_eq!(prepared.response_options.search_downgraded, !native_search);
            assert!(text.contains("go on"), "the conversation itself goes on");
        }
        // The protocol layer itself still refuses; only the router leaves them out.
        let refused = WireAdapter::new(router::ModelProtocol::ChatCompletions)
            .encode_request(router::ModelProtocol::Responses, &codex);
        assert!(refused.is_err());
        // Nothing hosted to leave out: a refusal stays one.
        let lossy = json!({"model": "m", "input": [{"type": "item_reference", "id": "x"}]});
        assert!(upstream_body(&destination(router::ModelProtocol::ChatCompletions), Some(Bytes::from(lossy.to_string()))).is_err());
    }

    #[test]
    fn unknown_account_sources_keep_messages_but_cannot_claim_forced_search() {
        let mut target = destination(router::ModelProtocol::ChatCompletions);
        if let Destination::Provider { provider, .. } = &mut target {
            provider.account_source = Some(account_sources::SourceKind::GeminiCli);
        }
        let optional = json!({"model":"m","input":"hi","tools":[{"type":"web_search"}],"tool_choice":"auto"});
        let prepared = upstream_body(&target, Some(Bytes::from(optional.to_string()))).unwrap();
        assert!(prepared.response_options.search_downgraded);
        let outgoing: Value = serde_json::from_slice(&prepared.body.unwrap()).unwrap();
        assert_eq!(outgoing["messages"][0]["content"], "hi");
        assert!(outgoing.get("tools").is_none());
        let mut forced = optional;
        forced["tool_choice"] = json!({"type":"web_search"});
        assert_eq!(upstream_body(&target, Some(Bytes::from(forced.to_string()))).unwrap_err().0, StatusCode::BAD_REQUEST);
        if let Destination::Provider { translation, .. } = &mut target {
            *translation = protocol::Translation::between(router::ModelProtocol::AnthropicMessages, router::ModelProtocol::ChatCompletions);
        }
        let claude = json!({"model":"m","max_tokens":100,"messages":[{"role":"user","content":"hi"}],
            "tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"auto"}});
        assert!(upstream_body(&target, Some(Bytes::from(claude.to_string()))).unwrap().response_options.search_downgraded);
        let mut forced = claude;
        forced["tool_choice"] = json!({"type":"tool","name":"web_search"});
        assert!(upstream_body(&target, Some(Bytes::from(forced.to_string()))).is_err());
    }
}
