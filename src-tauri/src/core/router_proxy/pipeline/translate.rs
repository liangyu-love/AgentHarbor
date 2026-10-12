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
        Self { body, input: Vec::new(), namespaces: Default::default(), response_options: Default::default(),
        }
    }
}

/// Responses tool types other protocols can carry: functions, and the
/// namespaced and freeform ones the translation itself handles.
const PORTABLE_TOOLS: [&str; 3] = ["function", "namespace", "custom"];
/// History items of hosted tools, which only a Responses provider can read.
const HOSTED_ITEMS: [&str; 4] = ["web_search_call", "image_generation_call", "tool_search_call", "tool_search_output"];

/// The request without hosted tools, their tool choice and their history
/// items; `None` when it has none of them.
fn without_hosted_tools(value: &Value) -> Option<Value> {
    let mut stripped = value.clone();
    let mut changed = false;
    if let Some(tools) = stripped.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        tools.retain(|tool| tool["type"].as_str().is_some_and(|kind| PORTABLE_TOOLS.contains(&kind)));
        changed |= tools.len() != before;
    }
    if let Some(object) = stripped.as_object_mut() {
        let hosted_choice = object.get("tool_choice").and_then(Value::as_object)
            .and_then(|choice| choice.get("type")).and_then(Value::as_str)
            .is_some_and(|kind| !PORTABLE_TOOLS.contains(&kind) && kind != "allowed_tools");
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

pub(in crate::core::router_proxy) fn upstream_body(
    destination: &Destination,
    body: Option<Bytes>) -> ForwardResult<PreparedForward> {
    let (
        Destination::Provider {
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
            let adapted = match adapter.encode_request(translation.from, &value) {
                // Codex offers its built-in tools (web search, image generation,
                // tool search) with every request. A relay on another protocol
                // cannot run them, so it gets the request without them rather
                // than a refusal of the whole turn.
                Err(error) if translation.from == router::ModelProtocol::Responses
                    && matches!(error.code.as_str(), "protocol.errors.unsupportedHostedTool" | "protocol.errors.unsupportedContent") =>
                {
                    match without_hosted_tools(&value) {
                        Some(stripped) => adapter.encode_request(translation.from, &stripped),
                        None => Err(error),
                    }
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
                            "this target protocol has no equivalent hosted tool; WebSearch requires a Responses or native Messages provider with search support",
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
            let AdapterRequest::Translated(rewritten) = adapted else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "invalid provider protocol translation",
                ));
            };
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

    fn destination(target: router::ModelProtocol) -> Destination {
        Destination::Provider {
            provider: Box::new(router::Provider::default()),
            upstream_protocol: target,
            model_override: Some("relay-model".into()),
            translation: protocol::Translation::between(router::ModelProtocol::Responses, target),
        }
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
            assert_eq!(body["tools"].as_array().unwrap().len(), 1, "{target:?} keeps the function only");
            let text = body.to_string();
            assert!(!text.contains("web_search") && !text.contains("image_generation"), "{target:?}: {text}");
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
}
