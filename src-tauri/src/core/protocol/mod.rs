//! Protocol translation between the OpenAI Responses API and the protocols a
//! relay actually speaks.
//!
//! Codex only ever calls `/v1/responses` with `wire_api = "responses"`, but many
//! relays expose Chat Completions or Anthropic Messages. Instead of forcing the
//! provider to speak Responses, the local router accepts Codex's Responses
//! traffic and translates it outward, then translates the upstream reply back
//! into Responses events. Both directions share this module so the two halves of
//! a stream cannot drift apart.
//!
//! Translation is only engaged when the provider's protocol differs from the
//! protocol the caller spoke; identical protocols keep the fast byte-for-byte
//! pass-through that the router has always had.

mod chat;
pub mod history;
mod responses;
mod sse;

use crate::core::router::ModelProtocol;
use serde_json::Value;

pub use responses::{is_responses_body, response_body};

/// Pick the protocol a relay most likely serves, from the capabilities it
/// advertises for its own models.
///
/// Relays that expose only `/v1/chat/completions` — the common OpenAI-compatible
/// case — report chat endpoints, and Anthropic-compatible relays report message
/// endpoints. Only a relay that explicitly lists a Responses endpoint is treated
/// as Responses, because guessing it wrongly makes Codex calls fail outright,
/// whereas the OpenAI-compatible assumption is what such relays almost always
/// accept.
pub fn infer_protocol(endpoint_types: &[String]) -> ModelProtocol {
    let has = |needle: &str| {
        endpoint_types
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(needle))
    };
    if has("response") {
        return ModelProtocol::Responses;
    }
    if has("anthropic") || has("message") {
        return ModelProtocol::AnthropicMessages;
    }
    if has("chat") || has("completion") {
        return ModelProtocol::ChatCompletions;
    }
    // Nothing advertised: OpenAI-compatible is the broadest common denominator.
    ModelProtocol::ChatCompletions
}

/// Infer a protocol from every model a relay reported.
///
/// Later models do not override an earlier answer, so a relay that advertises
/// Responses on some models keeps it.
pub fn infer_protocol_from_models(models: &[crate::core::newapi::Model]) -> ModelProtocol {
    for model in models {
        if !model.endpoint_types.is_empty() {
            return infer_protocol(&model.endpoint_types);
        }
    }
    ModelProtocol::ChatCompletions
}

/// The protocol pair a request needs translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translation {
    /// What the caller sent — always Responses for Codex requests.
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

    pub fn is_supported(self) -> bool {
        use crate::core::router::ModelProtocol::*;
        matches!(
            (self.from, self.to),
            (Responses, ChatCompletions) | (Responses, AnthropicMessages)
        )
    }
}

/// Rewrite an inbound request body for the provider's protocol.
///
/// Only the Responses protocol arrives here today: Codex is the sole caller that
/// needs translating, and its `/v1/responses` path is the one the router injects.
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
        _ => Err("protocol.errors.unsupportedTranslation".into()),
    }
}

/// Convert one upstream SSE chunk into zero or more Responses SSE events.
///
/// The translator stays free of side effects: the proxy records the completed
/// turn itself, so no lock is taken while the body stream is being pumped.
pub fn stream_translator(translation: Translation) -> sse::StreamTranslator {
    sse::StreamTranslator::new(translation)
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
