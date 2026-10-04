//! Protocol translation for Codex Responses and Claude Messages callers.
//! Identical protocols pass through unchanged. Cross-protocol requests and
//! replies share explicit adapters, including incremental streaming and tools.
//! Only Responses callers use the continuation history store.
mod anthropic;
mod anthropic_sse;
#[cfg(test)]
mod anthropic_tests;
mod chat;
pub mod history;
pub mod message_history;
mod reasoning;
mod responses;
mod sse;

use crate::core::router::ModelProtocol;
use serde_json::Value;

pub use responses::is_responses_body;

pub fn response_body(translation: Translation, value: &Value) -> Result<Value, String> {
    let canonical = canonical_response(translation, value)?;
    if translation.from == ModelProtocol::AnthropicMessages {
        anthropic::response_body(&canonical)
    } else {
        Ok(canonical)
    }
}

pub fn canonical_response(translation: Translation, value: &Value) -> Result<Value, String> {
    if is_responses_body(value) {
        Ok(value.clone())
    } else {
        responses::response_body(translation, value)
    }
}

/// Pick the protocol a relay most likely serves, from the capabilities it
/// advertises for its own models.
///
/// Relays that expose only `/v1/chat/completions` — the common OpenAI-compatible
/// case — report chat endpoints, and Anthropic-compatible relays report message
/// endpoints. Only a relay that explicitly lists a Responses endpoint is treated
/// as Responses, because guessing it wrongly makes Codex calls fail outright,
/// whereas the OpenAI-compatible assumption is what such relays almost always
/// accept.
pub fn declared_protocol(endpoint_types: &[String]) -> Option<ModelProtocol> {
    let has = |needle: &str| {
        endpoint_types
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(needle))
    };
    if has("response") {
        return Some(ModelProtocol::Responses);
    }
    if has("anthropic") || has("message") {
        return Some(ModelProtocol::AnthropicMessages);
    }
    if has("chat") || has("completion") {
        return Some(ModelProtocol::ChatCompletions);
    }
    None
}

/// Infer a provider default from its selected model. Other models can use
/// their own protocol overrides without changing this default.
pub fn infer_protocol_from_models(
    models: &[crate::core::newapi::Model],
    selected_model: &str,
) -> ModelProtocol {
    if let Some(model) = models.iter().find(|model| model.id == selected_model) {
        return model
            .protocol
            .or_else(|| declared_protocol(&model.endpoint_types))
            .unwrap_or_else(|| {
                if selected_model.to_ascii_lowercase().contains("claude") {
                    ModelProtocol::AnthropicMessages
                } else {
                    ModelProtocol::ChatCompletions
                }
            });
    }
    models
        .iter()
        .find_map(|model| {
            model
                .protocol
                .or_else(|| declared_protocol(&model.endpoint_types))
        })
        .unwrap_or_else(|| {
            if selected_model.to_ascii_lowercase().contains("claude") {
                ModelProtocol::AnthropicMessages
            } else {
                ModelProtocol::ChatCompletions
            }
        })
}

/// The protocol pair a request needs translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translation {
    /// What the caller sent: Responses for Codex or Messages for Claude.
    pub from: crate::core::router::ModelProtocol,
    /// What the upstream provider expects.
    pub to: crate::core::router::ModelProtocol,
}

impl Translation {
    /// `None` when both sides already agree and bytes can pass through untouched.
    pub fn between(
        from: crate::core::router::ModelProtocol,
        to: crate::core::router::ModelProtocol,
    ) -> Option<Self> {
        (from != to).then_some(Self { from, to })
    }

    /// True when a caller speaking `from` can reach an upstream speaking `to`,
    /// either directly or through a supported translation.
    pub fn can_serve(
        from: crate::core::router::ModelProtocol,
        to: crate::core::router::ModelProtocol,
    ) -> bool {
        Self::between(from, to).map_or(true, Self::is_supported)
    }

    pub fn is_supported(self) -> bool {
        use crate::core::router::ModelProtocol::*;
        matches!(
            (self.from, self.to),
            (Responses, ChatCompletions)
                | (Responses, AnthropicMessages)
                | (AnthropicMessages, ChatCompletions)
                | (AnthropicMessages, Responses)
        )
    }
}

/// Rewrite an inbound request body for the provider's protocol.
///
/// Responses callers expand continuation history; Messages callers carry their full conversation.
pub struct TranslatedRequest {
    pub body: Value,
    /// Canonical input without request-scoped instructions, ready for history.
    pub input: Vec<Value>,
}

pub fn request_body(translation: Translation, value: &Value) -> Result<TranslatedRequest, String> {
    use crate::core::router::ModelProtocol::*;
    match (translation.from, translation.to) {
        (Responses, ChatCompletions) | (Responses, AnthropicMessages) => {
            let input = history::input_for_request(value)?;
            let mut expanded = value.clone();
            let object = expanded
                .as_object_mut()
                .ok_or("protocol.errors.invalidRequest")?;
            object.remove("previous_response_id");
            object.insert("input".into(), Value::Array(input.clone()));
            let body = chat::request_from_responses(translation.to, &expanded)?;
            Ok(TranslatedRequest { body, input })
        }
        (AnthropicMessages, ChatCompletions) | (AnthropicMessages, Responses) => {
            Ok(TranslatedRequest {
                body: anthropic::request_body(translation.to, value)?,
                input: Vec::new(),
            })
        }
        _ => Err("protocol.errors.unsupportedTranslation".into()),
    }
}

/// Convert upstream SSE chunks into events in the caller's protocol.
///
/// The translator stays free of side effects: the proxy records the completed
/// turn itself, so no lock is taken while the body stream is being pumped.
pub struct StreamTranslator {
    responses: Option<sse::StreamTranslator>,
    anthropic: Option<anthropic_sse::StreamTranslator>,
}

impl StreamTranslator {
    pub fn is_finished(&self) -> bool {
        self.responses
            .as_ref()
            .is_some_and(sse::StreamTranslator::is_finished)
            || self
                .anthropic
                .as_ref()
                .is_some_and(anthropic_sse::StreamTranslator::is_finished)
    }
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        if let Some(translator) = self.responses.as_mut() {
            translator.push(chunk)
        } else {
            self.anthropic
                .as_mut()
                .expect("registered translator")
                .push(chunk)
        }
    }
    pub fn finish(&mut self) -> Vec<String> {
        if let Some(translator) = self.responses.as_mut() {
            translator.finish()
        } else {
            self.anthropic
                .as_mut()
                .expect("registered translator")
                .finish()
        }
    }
    pub fn fail(&mut self, code: &'static str) -> Vec<String> {
        if let Some(translator) = self.responses.as_mut() {
            translator.fail(code)
        } else {
            self.anthropic
                .as_mut()
                .expect("registered translator")
                .fail(code)
        }
    }
    pub fn completed_response(&self) -> Option<&Value> {
        self.responses
            .as_ref()
            .and_then(sse::StreamTranslator::completed_response)
            .or_else(|| {
                self.anthropic
                    .as_ref()
                    .and_then(anthropic_sse::StreamTranslator::completed_response)
            })
    }
    pub fn failure(&self) -> Option<&'static str> {
        self.responses
            .as_ref()
            .and_then(sse::StreamTranslator::failure)
            .or_else(|| {
                self.anthropic
                    .as_ref()
                    .and_then(anthropic_sse::StreamTranslator::failure)
            })
    }
}

pub fn stream_translator(translation: Translation) -> StreamTranslator {
    StreamTranslator {
        responses: (translation.from == ModelProtocol::Responses)
            .then(|| sse::StreamTranslator::new(translation)),
        anthropic: (translation.from == ModelProtocol::AnthropicMessages)
            .then(|| anthropic_sse::StreamTranslator::new(translation.to)),
    }
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    use serde_json::json;

    fn pair(to: ModelProtocol) -> Translation {
        Translation {
            from: ModelProtocol::Responses,
            to,
        }
    }
    fn assistant(text: &str) -> Value {
        json!({"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]})
    }
    fn record(input: Vec<Value>, output: Vec<Value>) -> String {
        let id = format!("resp_test_{}", uuid::Uuid::new_v4().simple());
        history::remember(
            &json!({"id":id,"status":"completed","output":output}),
            input,
        );
        id
    }

    #[test]
    fn ordinary_assistant_text_survives_a_continuation_in_both_protocols() {
        let first = request_body(
            pair(ModelProtocol::ChatCompletions),
            &json!({"model":"m","input":"question"}),
        )
        .unwrap();
        let id = record(first.input, vec![assistant("answer")]);
        for target in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let next = request_body(
                pair(target),
                &json!({"model":"m","previous_response_id":id,"input":"follow-up"}),
            )
            .unwrap();
            assert_eq!(next.body["messages"][0]["content"], "question");
            assert_eq!(next.body["messages"][1]["role"], "assistant");
            assert_eq!(next.body["messages"][1]["content"], "answer");
            assert_eq!(next.body["messages"][2]["content"], "follow-up");
        }
    }

    #[test]
    fn mixed_text_and_tool_calls_replay_in_the_target_protocol() {
        let id = record(
            vec![json!({"role":"user","content":"question"})],
            vec![
                assistant("Checking"),
                json!({"type":"function_call","call_id":"c1","name":"lookup","arguments":"{\"q\":1}","status":"completed"}),
            ],
        );
        let request = json!({"model":"m","previous_response_id":id,"input":[{"type":"function_call_output","call_id":"c1","output":"result"}]});
        let chat = request_body(pair(ModelProtocol::ChatCompletions), &request)
            .unwrap()
            .body;
        assert_eq!(chat["messages"][1]["content"], "Checking");
        assert_eq!(chat["messages"][1]["tool_calls"][0]["id"], "c1");
        assert_eq!(chat["messages"][2]["role"], "tool");
        let anthropic = request_body(pair(ModelProtocol::AnthropicMessages), &request)
            .unwrap()
            .body;
        assert_eq!(anthropic["messages"][1]["content"][0]["text"], "Checking");
        assert_eq!(anthropic["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(anthropic["messages"][1]["content"][1]["input"]["q"], 1);
        assert_eq!(
            anthropic["messages"][2]["content"][0]["type"],
            "tool_result"
        );
    }

    #[test]
    fn request_instructions_are_replaced_without_removing_explicit_system_input() {
        let first = request_body(pair(ModelProtocol::ChatCompletions), &json!({
            "model":"m","instructions":"old instruction",
            "input":[{"role":"system","content":[{"type":"input_text","text":"persistent system"}]},{"role":"user","content":"hi"}],
        })).unwrap();
        let id = record(first.input, vec![assistant("hello")]);
        for target in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let next = request_body(pair(target), &json!({"model":"m","instructions":"new instruction","previous_response_id":id,"input":"again"})).unwrap().body;
            assert!(!next.to_string().contains("old instruction"));
            if target == ModelProtocol::ChatCompletions {
                assert_eq!(next["messages"][0]["content"], "new instruction");
                assert_eq!(next["messages"][1]["content"], "persistent system");
                assert_eq!(next["messages"].as_array().unwrap().len(), 5);
            } else {
                assert_eq!(next["system"], "new instruction\npersistent system");
                assert_eq!(next["messages"].as_array().unwrap().len(), 3);
            }
            let without = request_body(
                pair(target),
                &json!({"model":"m","previous_response_id":id,"input":"again"}),
            )
            .unwrap()
            .body;
            assert!(!without.to_string().contains("old instruction"));
            assert!(without.to_string().contains("persistent system"));
        }
    }

    #[test]
    fn unknown_history_is_an_explicit_error() {
        let error = request_body(
            pair(ModelProtocol::ChatCompletions),
            &json!({"model":"m","previous_response_id":"unrecorded-test-id","input":"again"}),
        )
        .err()
        .unwrap();
        assert_eq!(error, "protocol.errors.unknownPreviousResponse");
    }
}
