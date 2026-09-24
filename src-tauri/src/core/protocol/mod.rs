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
pub fn request_body(translation: Translation, value: &Value) -> Result<Value, String> {
    use crate::core::router::ModelProtocol::*;
    match (translation.from, translation.to) {
        (Responses, ChatCompletions) | (Responses, AnthropicMessages) => {
            chat::request_from_responses(translation.to, value)
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

/// The provider-format messages a translated request was built from.
///
/// Kept separate from [`request_body`] so the proxy can hand them to the
/// translator without re-deriving them.
pub fn request_messages(translated: &Value) -> Vec<Value> {
    translated
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
