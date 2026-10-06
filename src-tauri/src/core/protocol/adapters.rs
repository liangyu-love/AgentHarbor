//! Behavior-preserving adapter boundaries over the existing translation engine.
//!
//! The current JSON representation stays inside the engine. These interfaces
//! let execution select ingress and egress independently without embedding
//! routing, history persistence, diagnostics, or accounting in each converter.
use super::{
    request_body, response_body_with_options, stream_translator_with_options, ModelProtocol,
    ProtocolError, ResponseOptions, StreamTranslator, TranslatedRequest, Translation,
};
use serde_json::Value;

pub trait InboundAdapter {
    fn protocol(&self) -> ModelProtocol;
    fn encode_response(
        &self,
        upstream: ModelProtocol,
        value: &Value,
        options: &ResponseOptions,
    ) -> Result<Value, ProtocolError>;
    /// `None` keeps same-protocol SSE on the existing byte-preserving path.
    fn stream_decoder(
        &self,
        upstream: ModelProtocol,
        options: ResponseOptions,
    ) -> Result<Option<StreamTranslator>, ProtocolError>;
}

pub trait OutboundAdapter {
    fn protocol(&self) -> ModelProtocol;
    fn encode_request(
        &self,
        caller: ModelProtocol,
        value: &Value,
    ) -> Result<AdapterRequest, ProtocolError>;
}

pub enum AdapterRequest {
    /// Same-protocol callers do not expand history or reinterpret fields.
    Passthrough(Value),
    Translated(TranslatedRequest),
}

impl AdapterRequest {
    pub fn body(&self) -> &Value {
        match self {
            Self::Passthrough(body) => body,
            Self::Translated(request) => &request.body,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WireAdapter {
    protocol: ModelProtocol,
}

impl WireAdapter {
    pub fn new(protocol: ModelProtocol) -> Self {
        Self { protocol }
    }
}

fn translation(
    from: ModelProtocol,
    to: ModelProtocol,
) -> Result<Option<Translation>, ProtocolError> {
    let translation = Translation::between(from, to);
    if translation.is_some_and(|pair| !pair.is_supported()) {
        return Err(ProtocolError::from_legacy(
            "protocol.errors.unsupportedTranslation".into(),
            from,
            to,
        ));
    }
    Ok(translation)
}

impl OutboundAdapter for WireAdapter {
    fn protocol(&self) -> ModelProtocol {
        self.protocol
    }

    fn encode_request(
        &self,
        caller: ModelProtocol,
        value: &Value,
    ) -> Result<AdapterRequest, ProtocolError> {
        match translation(caller, self.protocol)? {
            None => Ok(AdapterRequest::Passthrough(value.clone())),
            Some(pair) => request_body(pair, value)
                .map(AdapterRequest::Translated)
                .map_err(|code| ProtocolError::from_legacy(code, caller, self.protocol)),
        }
    }
}

impl InboundAdapter for WireAdapter {
    fn protocol(&self) -> ModelProtocol {
        self.protocol
    }

    fn encode_response(
        &self,
        upstream: ModelProtocol,
        value: &Value,
        options: &ResponseOptions,
    ) -> Result<Value, ProtocolError> {
        match translation(self.protocol, upstream)? {
            None => Ok(value.clone()),
            Some(pair) => response_body_with_options(pair, value, options)
                .map_err(|code| ProtocolError::from_legacy(code, self.protocol, upstream)),
        }
    }

    fn stream_decoder(
        &self,
        upstream: ModelProtocol,
        options: ResponseOptions,
    ) -> Result<Option<StreamTranslator>, ProtocolError> {
        Ok(translation(self.protocol, upstream)?
            .map(|pair| stream_translator_with_options(pair, options)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::ProtocolErrorKind;
    use serde_json::json;

    #[test]
    fn adapter_boundary_preserves_the_existing_support_matrix() {
        for caller in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            for upstream in [
                ModelProtocol::Responses,
                ModelProtocol::ChatCompletions,
                ModelProtocol::AnthropicMessages,
            ] {
                let decoder = WireAdapter::new(caller).stream_decoder(upstream, Default::default());
                assert_eq!(decoder.is_ok(), Translation::can_serve(caller, upstream));
                if caller == upstream {
                    assert!(decoder.unwrap().is_none());
                }
            }
        }
    }

    #[test]
    fn native_request_and_response_keep_unknown_fields() {
        let value = json!({"model":"custom", "unknown":{"signed":"opaque"}, "previous_response_id":"upstream-resource"});
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let adapter = WireAdapter::new(protocol);
            let request = adapter.encode_request(protocol, &value).unwrap();
            assert!(matches!(request, AdapterRequest::Passthrough(_)));
            assert_eq!(request.body(), &value);
            assert_eq!(
                adapter
                    .encode_response(protocol, &value, &Default::default())
                    .unwrap(),
                value
            );
        }
    }

    #[test]
    fn adapter_request_matches_the_legacy_converter_in_all_cross_pairs() {
        for caller in [
            ModelProtocol::Responses,
            ModelProtocol::AnthropicMessages,
            ModelProtocol::ChatCompletions,
        ] {
            let value = if caller == ModelProtocol::Responses {
                json!({"model":"fixture", "input":"hello"})
            } else if caller == ModelProtocol::AnthropicMessages {
                json!({"model":"fixture", "max_tokens":20, "messages":[{"role":"user", "content":"hello"}]})
            } else {
                json!({"model":"fixture", "max_completion_tokens":20, "messages":[{"role":"user", "content":"hello"}]})
            };
            for upstream in [
                ModelProtocol::Responses,
                ModelProtocol::ChatCompletions,
                ModelProtocol::AnthropicMessages,
            ] {
                let Some(pair) = Translation::between(caller, upstream) else {
                    continue;
                };
                let expected = request_body(pair, &value).unwrap();
                let actual = WireAdapter::new(upstream)
                    .encode_request(caller, &value)
                    .unwrap();
                assert_eq!(actual.body(), &expected.body);
            }
        }
    }

    #[test]
    fn invalid_chat_ingress_returns_structured_error_after_support_is_enabled() {
        let error = match WireAdapter::new(ModelProtocol::Responses)
            .encode_request(ModelProtocol::ChatCompletions, &json!({"messages":[]}))
        {
            Ok(_) => panic!("invalid Chat body must be rejected before forwarding"),
            Err(error) => error,
        };
        assert_eq!(error.kind, ProtocolErrorKind::InvalidRequest);
        assert_eq!(error.code, "protocol.errors.invalidRequest");
    }

    #[test]
    fn inbound_adapter_preserves_response_and_fragmented_stream_semantics() {
        fn stable(mut value: Value) -> Value {
            match &mut value {
                Value::Object(fields) => {
                    for (key, field) in fields {
                        if matches!(key.as_str(), "id" | "response_id") {
                            *field = json!("generated-id");
                        } else {
                            *field = stable(std::mem::take(field));
                        }
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        *item = stable(std::mem::take(item));
                    }
                }
                _ => {}
            }
            value
        }
        fn frames(values: Vec<String>) -> Vec<Value> {
            values
                .into_iter()
                .map(|frame| {
                    let event = frame.lines().find_map(|line| line.strip_prefix("event: "));
                    let data = frame
                        .lines()
                        .find_map(|line| line.strip_prefix("data: "))
                        .unwrap();
                    json!({"event":event,"data":stable(serde_json::from_str(data).unwrap())})
                })
                .collect()
        }
        for caller in [ModelProtocol::Responses, ModelProtocol::AnthropicMessages] {
            for upstream in [
                ModelProtocol::Responses,
                ModelProtocol::ChatCompletions,
                ModelProtocol::AnthropicMessages,
            ] {
                let Some(pair) = Translation::between(caller, upstream) else {
                    continue;
                };
                let value = match upstream {
                    ModelProtocol::Responses => json!({"id":"response_adapter","object":"response",
                        "model":"fixture","status":"completed","output":[{"type":"message","role":"assistant",
                        "content":[{"type":"output_text","text":"adapter reply"}]}],
                        "usage":{"input_tokens":3,"output_tokens":2}}),
                    ModelProtocol::ChatCompletions => json!({"id":"chat_adapter","model":"fixture",
                        "choices":[{"message":{"role":"assistant","content":"adapter reply"},"finish_reason":"stop"}],
                        "usage":{"prompt_tokens":3,"completion_tokens":2}}),
                    ModelProtocol::AnthropicMessages => {
                        json!({"id":"message_adapter","model":"fixture",
                        "type":"message","role":"assistant","content":[{"type":"text","text":"adapter reply"}],
                        "stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}})
                    }
                };
                let adapter = WireAdapter::new(caller);
                assert_eq!(
                    stable(
                        adapter
                            .encode_response(upstream, &value, &Default::default())
                            .unwrap()
                    ),
                    stable(response_body_with_options(pair, &value, &Default::default()).unwrap()),
                );
                let source = match upstream {
                    ModelProtocol::Responses => format!(
                        "data: {}\n\n",
                        json!({"type":"response.completed","response":value})
                    ),
                    ModelProtocol::ChatCompletions => format!("data: {value}\n\ndata: [DONE]\n\n"),
                    ModelProtocol::AnthropicMessages => format!(
                        "event: message_start\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
                        json!({"type":"message_start","message":value}),
                        json!({"type":"message_stop"})
                    ),
                };
                let mut actual = adapter
                    .stream_decoder(upstream, Default::default())
                    .unwrap()
                    .unwrap();
                let mut expected = stream_translator_with_options(pair, Default::default());
                for chunk in source.as_bytes().chunks(7) {
                    assert_eq!(frames(actual.push(chunk)), frames(expected.push(chunk)));
                }
                assert_eq!(frames(actual.finish()), frames(expected.finish()));
                assert_eq!(
                    actual.completed_response().cloned().map(stable),
                    expected.completed_response().cloned().map(stable)
                );
                assert_eq!(actual.failure(), expected.failure());
            }
        }
    }

    #[test]
    fn outbound_adapter_retains_hosted_search_response_options() {
        let value = json!({"model":"fixture","max_tokens":32,
            "messages":[{"role":"user","content":"search"}],
            "tools":[{"type":"web_search_20250305","name":"visible_search","max_uses":2}],
            "tool_choice":{"type":"tool","name":"visible_search"}});
        let request = WireAdapter::new(ModelProtocol::Responses)
            .encode_request(ModelProtocol::AnthropicMessages, &value)
            .unwrap();
        let AdapterRequest::Translated(request) = request else {
            panic!("Messages must be translated for Responses");
        };
        assert_eq!(
            request.response_options.search.unwrap().name,
            "visible_search"
        );
        assert_eq!(request.body["tool_choice"], "required");
        assert_eq!(request.body["tools"][0]["type"], "web_search");
    }
}
