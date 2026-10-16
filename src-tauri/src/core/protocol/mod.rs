//! Protocol translation for Responses, Messages and Chat callers.
//! Identical protocols pass through unchanged. Cross-protocol requests and
//! replies share explicit adapters, including incremental streaming and tools.
//! Only Responses callers use the continuation history store.
pub mod adapters;
mod anthropic;
mod anthropic_sse;
#[cfg(test)]
mod anthropic_tests;
mod cache_marks;
pub mod canonical;
mod chat;
mod chat_ingress;
pub mod chat_reasoning;
#[cfg(test)]
mod chat_ingress_tests;
mod chat_sse;
pub mod history;
pub mod message_history;
#[cfg(test)]
mod performance_tests;
#[cfg(test)]
mod search_compat_tests;
mod reasoning;
mod responses;
mod search_compat;
mod sse;
pub mod tool_namespaces;
mod types;
pub(crate) mod web_search;

use serde_json::Value;
pub use types::{ModelProtocol, ProtocolError, ProtocolErrorKind};

pub use responses::is_responses_body;
pub use search_compat::ChatSearch;

pub fn response_body(translation: Translation, value: &Value) -> Result<Value, String> {
    response_body_with_options(translation, value, &ResponseOptions::default())
}

#[derive(Clone, Debug, Default)]
pub struct ResponseOptions {
    pub search: Option<web_search::Search>,
    /// Optional declarations were omitted to keep ordinary messages usable.
    pub search_downgraded: bool,
    /// Keep a forced caller choice even when the native wire encodes it as auto.
    pub search_required: bool,
    /// A Codex cached/indexed search declaration runs as the target's live search.
    pub search_cached_as_live: bool,
    /// The target's search format has only an approximate call budget.
    pub search_budget_approximate: bool,
    /// Chat's usage-only terminal chunk is opt-in on its ingress contract.
    pub chat_include_usage: bool,
}

pub fn response_body_with_options(
    translation: Translation,
    value: &Value,
    options: &ResponseOptions,
) -> Result<Value, String> {
    let mut canonical = canonical_response(translation, value)?;
    if translation.to == ModelProtocol::ChatCompletions && options.search.is_some() && canonical["status"] == "completed" {
        if let Some(search) = search_compat::chat_result(canonical["output"].as_array().ok_or("protocol.errors.invalidResponse")?,
            &canonical["usage"], canonical["id"].as_str().unwrap_or("search")) {
            canonical["output"].as_array_mut().unwrap().insert(0, search);
        }
    }
    if translation.from == ModelProtocol::AnthropicMessages {
        anthropic::response_body_with_options(&canonical, options)
    } else if translation.from == ModelProtocol::ChatCompletions {
        chat_ingress::response(&canonical)
    } else {
        Ok(canonical)
    }
}

pub fn canonical_response(translation: Translation, value: &Value) -> Result<Value, String> {
    if translation.from == ModelProtocol::ChatCompletions {
        return chat_ingress::canonical_response(translation, value);
    }
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
    pub from: ModelProtocol,
    /// What the upstream provider expects.
    pub to: ModelProtocol,
}

impl Translation {
    /// `None` when both sides already agree and bytes can pass through untouched.
    pub fn between(
        from: ModelProtocol, to: ModelProtocol) -> Option<Self> {
        (from != to).then_some(Self { from, to })
    }

    /// True when a caller speaking `from` can reach an upstream speaking `to`,
    /// either directly or through a supported translation.
    pub fn can_serve(
        from: ModelProtocol, to: ModelProtocol) -> bool {
        Self::between(from, to).map_or(true, Self::is_supported)
    }

    pub fn is_supported(self) -> bool {
        use ModelProtocol::*;
        matches!(
            (self.from, self.to),
            (Responses, ChatCompletions)
                | (Responses, AnthropicMessages)
                | (AnthropicMessages, ChatCompletions)
                | (AnthropicMessages, Responses)
                | (ChatCompletions, Responses)
                | (ChatCompletions, AnthropicMessages)
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
    pub namespaces: tool_namespaces::ToolNamespaces,
    pub response_options: ResponseOptions,
}

pub fn request_body(translation: Translation, value: &Value) -> Result<TranslatedRequest, String> {
    request_body_with_chat_search(translation, value, None)
}

pub(crate) fn request_body_with_chat_search(
    translation: Translation, value: &Value, chat_search: Option<ChatSearch>,
) -> Result<TranslatedRequest, String> {
    let mut translated = translate_request(translation, value, chat_search)?;
    // Messages caches nothing unmarked; the other targets cache by prefix.
    if translation.to == ModelProtocol::AnthropicMessages {
        cache_marks::mark(&mut translated.body);
    }
    Ok(translated)
}

fn translate_request(
    translation: Translation, value: &Value, chat_search: Option<ChatSearch>,
) -> Result<TranslatedRequest, String> {
    use ModelProtocol::*;
    match (translation.from, translation.to) {
        (Responses, ChatCompletions) | (Responses, AnthropicMessages) => {
            let input = history::input_for_request(value)?;
            let mut expanded = value.clone();
            let object = expanded
                .as_object_mut()
                .ok_or("protocol.errors.invalidRequest")?;
            object.remove("previous_response_id");
            object.insert("input".into(), Value::Array(input.clone()));
            let namespaces = tool_namespaces::ToolNamespaces::prepare(&mut expanded)?;
            let body = chat::request_from_responses_with_search(translation.to, &expanded, chat_search)?;
            let declared = expanded["tools"].as_array().into_iter().flatten().find(|tool| web_search::is_search(tool));
            let search = (expanded["tool_choice"] != "none" && declared.is_some())
                .then(|| web_search::Search { name:"web_search".into(), max_uses:expanded["max_tool_calls"].as_u64() });
            let search_cached_as_live = search.is_some() && declared.is_some_and(|tool|
                tool["external_web_access"] == false || tool["indexed_web_access"] == true);
            let search_budget_approximate = search.is_some() && translation.to == ChatCompletions
                && expanded.get("max_tool_calls").is_some();
            Ok(TranslatedRequest {
                body,
                input,
                namespaces,
                response_options: ResponseOptions { search, search_cached_as_live, search_budget_approximate, ..Default::default() },
            })
        }
        (AnthropicMessages, ChatCompletions) | (AnthropicMessages, Responses) => {
            let (body, response_options) = anthropic::request_with_search(translation.to, value, chat_search)?;
            if translation.to == ChatCompletions {
                chat::validate_tool_history(&body)?;
            }
            if translation.to == Responses {
                responses::validate_tool_history(&body)?;
            }
            Ok(TranslatedRequest {
                body,
                input: Vec::new(),
                namespaces: Default::default(),
                response_options,
            })
        }
        (ChatCompletions, Responses) | (ChatCompletions, AnthropicMessages) => {
            chat_ingress::request(translation.to, value)
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
    chat: Option<chat_sse::StreamTranslator>,
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
            || self
                .chat
                .as_ref()
                .is_some_and(chat_sse::StreamTranslator::is_finished)
    }
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        if let Some(translator) = self.responses.as_mut() {
            translator.push(chunk)
        } else if let Some(translator) = self.chat.as_mut() {
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
        } else if let Some(translator) = self.chat.as_mut() {
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
        } else if let Some(translator) = self.chat.as_mut() {
            translator.fail(code)
        } else {
            self.anthropic
                .as_mut()
                .expect("registered translator")
                .fail(code)
        }
    }
    /// A Chat upstream streamed its reasoning as `reasoning_content`.
    pub fn sent_reasoning_content(&self) -> bool {
        self.responses
            .as_ref()
            .is_some_and(sse::StreamTranslator::sent_reasoning_content)
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
            .or_else(|| {
                self.chat
                    .as_ref()
                    .and_then(chat_sse::StreamTranslator::completed_response)
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
            .or_else(|| {
                self.chat
                    .as_ref()
                    .and_then(chat_sse::StreamTranslator::failure)
            })
    }
}

pub fn stream_translator(translation: Translation) -> StreamTranslator {
    stream_translator_with_options(translation, ResponseOptions::default())
}

pub fn stream_translator_with_options(
    translation: Translation,
    options: ResponseOptions,
) -> StreamTranslator {
    StreamTranslator {
        responses: (translation.from == ModelProtocol::Responses)
            .then(|| sse::StreamTranslator::new(translation).with_search(options.search.is_some())),
        anthropic: (translation.from == ModelProtocol::AnthropicMessages).then(|| {
            anthropic_sse::StreamTranslator::new(translation.to).with_options(options.clone())
        }),
        chat: (translation.from == ModelProtocol::ChatCompletions)
            .then(|| chat_sse::StreamTranslator::new(translation.to, options.chat_include_usage)),
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
            // Messages carries its cache mark on the last turn's block.
            let follow_up = if target == ModelProtocol::AnthropicMessages {
                &next.body["messages"][2]["content"][0]["text"]
            } else {
                &next.body["messages"][2]["content"]
            };
            assert_eq!(follow_up, "follow-up");
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
                assert_eq!(next["system"][0]["text"], "new instruction\npersistent system");
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
