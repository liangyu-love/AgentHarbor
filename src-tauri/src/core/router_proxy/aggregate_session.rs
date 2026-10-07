//! Explicit session IDs and exact delivered Messages transcript ownership.
//! No prompt text is retained in the registry. A new headerless first turn gets
//! a fresh session; only a completed reply can identify its later continuation.
use super::*;
use crate::core::aggregation::affinity::{self, TranscriptReservation};
use sha2::{Digest, Sha256};

fn valid_id(value: &str) -> Option<&str> {
    (!value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
        .then_some(value)
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().and_then(valid_id)
}
pub(super) fn explicit(headers: &HeaderMap, body: &Value) -> Option<String> {
    for name in [
        "session_id",
        "session-id",
        "x-session-id",
        "x-codex-session-id",
        "conversation_id",
        "x-opencode-session",
        "x-opencode-session-id",
    ] {
        if let Some(id) = headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(valid_id)
        {
            return Some(id.into());
        }
    }
    if let Some(raw) = headers
        .get("x-codex-turn-metadata")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 4096)
    {
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            if let Some(id) = field(&value, "session_id") {
                return Some(id.into());
            }
        }
    }
    for value in [body, &body["metadata"]] {
        if let Some(id) = field(value, "session_id").or_else(|| field(value, "conversation_id")) {
            return Some(id.into());
        }
    }
    let user_id = body
        .pointer("/metadata/user_id")?
        .as_str()
        .filter(|v| v.len() <= 4096)?;
    if let Ok(value) = serde_json::from_str::<Value>(user_id) {
        return field(&value, "session_id").map(str::to_owned);
    }
    // Claude Code legacy metadata: account/device identity is not a session.
    let (account, session) = user_id.rsplit_once("_session_")?;
    if !account.starts_with("user_") || !account.contains("_account_") {
        return None;
    }
    uuid::Uuid::parse_str(session).ok().map(|id| id.to_string())
}

#[derive(Clone, Default)]
pub(super) struct Fingerprint(Sha256);
impl Fingerprint {
    pub fn request(value: &Value) -> Option<(Self, Option<String>)> {
        let messages = value["messages"].as_array()?;
        let mut prefix = Self::default();
        let mut previous = None;
        for message in messages {
            if matches!(message["role"].as_str(), Some("user" | "assistant")) {
                prefix.append(message)?;
                if message["role"] == "assistant" {
                    previous = Some(prefix.hash());
                }
            }
        }
        Some((prefix, previous))
    }
    fn append(&mut self, message: &Value) -> Option<()> {
        fn sort(value: &mut Value) {
            match value {
                Value::Object(object) => {
                    object.sort_keys();
                    for value in object.values_mut() {
                        sort(value);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        sort(value);
                    }
                }
                _ => {}
            }
        }
        fn normalize_block(block: &mut Value) {
            if let Some(object) = block.as_object_mut() {
                object.remove("cache_control");
                if object.get("type").is_some_and(|v| v == "text")
                    && object
                        .get("citations")
                        .is_some_and(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
                {
                    object.remove("citations");
                }
                if object.get("type").is_some_and(|v| v == "tool_result") {
                    match object.get_mut("content") {
                        Some(Value::String(text)) => {
                            let text = text.clone();
                            object.insert("content".into(), json!([{"type":"text","text":text}]));
                        }
                        Some(Value::Array(blocks)) => {
                            for block in blocks {
                                normalize_block(block);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Arbitrary tool input keys (even "cache_control") are semantic.
            // Sorting them is safe; deleting them would merge distinct calls.
            sort(block);
        }
        let content = match &message["content"] {
            Value::String(text) => vec![json!({"type":"text","text":text})],
            Value::Array(blocks) => blocks.clone(),
            _ => return None,
        };
        let mut blocks: Vec<Value> = Vec::new();
        for mut block in content {
            normalize_block(&mut block);
            if block["type"] == "text" {
                let text = block["text"].as_str()?;
                if text.is_empty() {
                    continue;
                }
                let standard = |value: &Value| {
                    value.as_object().is_some_and(|object| {
                        object
                            .keys()
                            .all(|key| matches!(key.as_str(), "type" | "text" | "citations"))
                    })
                };
                if let Some(last) = blocks.last_mut().filter(|last| {
                    last["type"] == "text"
                        && last.get("citations") == block.get("citations")
                        && standard(last)
                        && standard(&block)
                }) {
                    if let Some(Value::String(previous)) = last.get_mut("text") {
                        previous.push_str(text);
                        continue;
                    }
                }
            }
            blocks.push(block);
        }
        let message = json!([message["role"], blocks]).to_string();
        self.0.update((message.len() as u64).to_le_bytes());
        self.0.update(message.as_bytes());
        Some(())
    }
    fn hash(&self) -> String {
        format!("{:x}", self.0.clone().finalize())
    }
}

/// Bounded reconstruction of the current delivered reply only. Unknown delta
/// shapes disable the fallback instead of creating an inaccurate fingerprint.
#[derive(Default)]
pub(super) struct Reply {
    blocks: Vec<Value>,
    arguments: std::collections::HashMap<usize, String>,
    bytes: usize,
    invalid: bool,
}
impl Reply {
    pub fn event(&mut self, value: &Value) {
        if self.invalid {
            return;
        }
        match value["type"].as_str() {
            Some("message_start") => {
                self.blocks = value["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                self.bytes = self.blocks.iter().map(|v| v.to_string().len()).sum();
            }
            Some("content_block_start") => {
                let Some(index) = value["index"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .filter(|index| *index == self.blocks.len() && *index < 4096)
                else {
                    self.invalid = true;
                    return;
                };
                let block = value["content_block"].clone();
                self.bytes = self.bytes.saturating_add(block.to_string().len());
                self.blocks.push(block);
                debug_assert_eq!(index + 1, self.blocks.len());
            }
            Some("content_block_delta") => {
                let Some(index) = value["index"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .filter(|index| *index < self.blocks.len())
                else {
                    self.invalid = true;
                    return;
                };
                let delta = &value["delta"];
                if delta["type"] == "citations_delta" {
                    let citation = delta["citation"].clone();
                    if !citation.is_object() {
                        self.invalid = true;
                        return;
                    }
                    self.bytes = self.bytes.saturating_add(citation.to_string().len());
                    if self.bytes > 16 * 1024 * 1024 {
                        self.invalid = true;
                        self.blocks.clear();
                        self.arguments.clear();
                        return;
                    }
                    let block = &mut self.blocks[index];
                    if block.get("citations").is_none_or(|v| v.is_null()) {
                        block["citations"] = json!([]);
                    }
                    if let Some(citations) = block["citations"].as_array_mut() {
                        citations.push(citation);
                    } else {
                        self.invalid = true;
                    }
                    return;
                }
                let key = match delta["type"].as_str() {
                    Some("text_delta") => "text",
                    Some("thinking_delta") => "thinking",
                    Some("signature_delta") => "signature",
                    Some("input_json_delta") => "partial_json",
                    _ => {
                        self.invalid = true;
                        return;
                    }
                };
                let Some(text) = delta[key].as_str() else {
                    self.invalid = true;
                    return;
                };
                self.bytes = self.bytes.saturating_add(text.len());
                if self.bytes > 16 * 1024 * 1024 {
                    self.invalid = true;
                    self.blocks.clear();
                    self.arguments.clear();
                    return;
                }
                if key == "partial_json" {
                    self.arguments.entry(index).or_default().push_str(text);
                } else if let Some(Value::String(current)) = self.blocks[index].get_mut(key) {
                    current.push_str(text);
                } else {
                    self.blocks[index][key] = json!(text);
                }
            }
            Some("content_block_stop") => {
                if let Some(index) = value["index"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                {
                    if let Some(arguments) = self.arguments.remove(&index) {
                        if let Ok(input) = serde_json::from_str::<Value>(&arguments) {
                            self.blocks[index]["input"] = input;
                        } else {
                            self.invalid = true;
                        }
                    }
                }
            }
            _ => {}
        }
        if self.bytes > 16 * 1024 * 1024 {
            self.invalid = true;
            self.blocks.clear();
            self.arguments.clear();
        }
    }
    pub fn assistant(&self) -> Option<Value> {
        (!self.invalid && self.arguments.is_empty() && !self.blocks.is_empty())
            .then(|| json!({"role":"assistant","content":self.blocks}))
    }
}

pub(super) struct Tracker {
    namespace: String,
    prefix: Fingerprint,
    session_key: String,
    reservation: Option<TranscriptReservation>,
    unpublished_first: bool,
    reply: Reply,
}
impl Tracker {
    pub fn new(
        namespace: String,
        prefix: Fingerprint,
        session_key: String,
        reservation: TranscriptReservation,
        first: bool,
    ) -> Self {
        Self {
            namespace,
            prefix,
            session_key,
            reservation: Some(reservation),
            unpublished_first: first,
            reply: Reply::default(),
        }
    }
    pub fn body(&mut self, value: &Value, delivered: bool) {
        if delivered && value["type"] == "message" {
            self.record(value);
        }
    }
    pub fn event(&mut self, value: &Value, delivered: bool) {
        if self.reservation.is_none() {
            return;
        }
        self.reply.event(value);
        if value["type"] == "message_stop" && delivered {
            if let Some(assistant) = self.reply.assistant() {
                self.record(&assistant);
            }
            self.reply = Reply::default();
        }
    }
    fn record(&mut self, assistant: &Value) {
        if self.prefix.append(assistant).is_some() {
            if let Some(reservation) = self.reservation.take() {
                reservation.commit(
                    affinity::scope(&[&self.namespace, &self.prefix.hash()]),
                    &self.session_key,
                );
                self.unpublished_first = false;
            }
        }
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        if self.unpublished_first {
            affinity::shared().abandon_unpublished_binding(&self.session_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_session_parses_client_formats_without_using_account_identity() {
        let mut headers = HeaderMap::new();
        let legacy = format!(
            "user_{}_account__session_f25958b8-e75c-455d-8b40-f006d87cc2a4",
            "a".repeat(64)
        );
        assert_eq!(
            explicit(&headers, &json!({"metadata":{"user_id":legacy}})).as_deref(),
            Some("f25958b8-e75c-455d-8b40-f006d87cc2a4")
        );
        assert_eq!(explicit(&headers, &json!({"metadata":{"user_id":
            json!({"device_id":"same device","account_uuid":"same account","session_id":"body session"}).to_string()}})).as_deref(),
            Some("body session"));
        for body in [
            json!({"metadata":{"user_id":"account-only"}}),
            json!({"metadata":{"user_id":"{\"account_uuid\":\"account\"}"}}),
            json!({"session_id":"\u{0000}"}),
            json!({"session_id":" ".repeat(40)}),
            json!({"session_id":"a".repeat(257)}),
        ] {
            assert_eq!(explicit(&headers, &body), None);
        }
        headers.insert(
            "x-codex-turn-metadata",
            HeaderValue::from_static("{\"session_id\":\"turn session\"}"),
        );
        assert_eq!(
            explicit(&headers, &json!({})).as_deref(),
            Some("turn session")
        );
        headers.insert("session-id", HeaderValue::from_static("header session"));
        assert_eq!(
            explicit(&headers, &json!({"session_id":"body session"})).as_deref(),
            Some("header session")
        );
    }

    #[test]
    fn exact_history_normalizes_cache_hints_but_keeps_tool_input_and_signatures() {
        let value = json!({"messages":[{"role":"user","content":"hi"},
            {"role":"assistant","content":[{"type":"text","text":"ok"},
                {"type":"tool_use","id":"c1","name":"lookup","input":{"cache_control":"semantic","z":1}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"result"}]}]});
        let original = Fingerprint::request(&value).unwrap();
        let mut normalized = value.clone();
        normalized["messages"][0]["content"] =
            json!([{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]);
        normalized["messages"][1]["content"][0]["citations"] = json!([]);
        normalized["messages"][2]["content"][0]["content"] =
            json!([{"type":"text","text":"result","cache_control":{"type":"ephemeral"}}]);
        let other = Fingerprint::request(&normalized).unwrap();
        assert_eq!(original.0.hash(), other.0.hash());
        assert_eq!(original.1, other.1);
        normalized["messages"][1]["content"][1]["input"]["cache_control"] = json!("changed");
        assert_ne!(original.1, Fingerprint::request(&normalized).unwrap().1);
        normalized = value.clone();
        normalized["messages"][1]["content"]
            .as_array_mut()
            .unwrap()
            .insert(
                0,
                json!({"type":"thinking","thinking":"plan","signature":"signed"}),
            );
        assert_ne!(original.1, Fingerprint::request(&normalized).unwrap().1);
        assert_eq!(
            Fingerprint::request(&json!({"messages":[{"role":"user","content":"hi"}]}))
                .unwrap()
                .1,
            None
        );
    }

    #[test]
    fn reply_reassembles_signed_thinking_parallel_tools_citations_and_split_text() {
        let mut reply = Reply::default();
        for event in [
            json!({"type":"message_start","message":{"content":[]}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"O"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"K"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"citations_delta","citation":{"type":"char_location","start_char_index":0,"end_char_index":2,"document_index":0}}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"a","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}),
            json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"b","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"q\":2}"}}),
            json!({"type":"content_block_stop","index":3}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"1}"}}),
            json!({"type":"content_block_stop","index":2}),
        ] {
            reply.event(&event);
        }
        let assistant = reply.assistant().unwrap();
        assert_eq!(assistant["content"][0]["signature"], "signed");
        assert_eq!(assistant["content"][0]["thinking"], "plan");
        assert_eq!(assistant["content"][1]["text"], "OK");
        assert_eq!(assistant["content"][1]["citations"][0]["document_index"], 0);
        assert_eq!(assistant["content"][2]["input"]["q"], 1);
        assert_eq!(assistant["content"][3]["input"]["q"], 2);
        reply.event(&json!({"type":"content_block_delta","index":1,"delta":{"type":"future_delta","text":"unknown"}}));
        assert!(
            reply.assistant().is_none(),
            "unknown semantic deltas must not invent an ownership key"
        );
    }

    #[test]
    fn incomplete_tool_arguments_and_oversized_replies_are_not_registered() {
        let mut reply = Reply::default();
        reply.event(&json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"lookup","input":{}}}));
        reply.event(&json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}));
        assert!(reply.assistant().is_none());
        reply.event(&json!({"type":"content_block_stop","index":0}));
        assert!(reply.assistant().is_none());
        let mut large = Reply::default();
        large.event(&json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}));
        large.event(&json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a".repeat(16 * 1024 * 1024 + 1)}}));
        assert!(large.assistant().is_none());
        assert!(large.blocks.is_empty());
    }

    #[test]
    fn unpublished_first_reply_releases_binding_but_failed_continuation_keeps_it() {
        let _guard = global_router_test_lock();
        let registry = affinity::shared();
        let namespace = affinity::scope(&["tracker lifecycle", &uuid::Uuid::new_v4().to_string()]);
        let key = affinity::scope(&[&namespace, "session"]);
        let owner = affinity::Owner {
            aggregate_id: "pool".into(),
            member_id: "a".into(),
            provider_id: "a".into(),
            actual_model_id: "wire".into(),
            connection_key: "physical".into(),
            configured_protocol: router::ModelProtocol::AnthropicMessages,
            effective_protocol: router::ModelProtocol::AnthropicMessages,
            native_resource: false,
        };
        let prefix = Fingerprint::request(&json!({"messages":[{"role":"user","content":"hello"}]}))
            .unwrap()
            .0;
        registry
            .reserve_binding(&key)
            .unwrap()
            .commit(owner.clone());
        {
            let mut tracker = Tracker::new(
                namespace.clone(),
                prefix.clone(),
                key.clone(),
                registry.reserve_transcript().unwrap(),
                true,
            );
            tracker.event(&json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"partial"}}), false);
        }
        assert!(
            registry.binding(&key).is_none(),
            "a dropped first stream has no published transcript"
        );
        registry.reserve_binding(&key).unwrap().commit(owner);
        {
            let mut tracker = Tracker::new(
                namespace.clone(),
                prefix.clone(),
                key.clone(),
                registry.reserve_transcript().unwrap(),
                true,
            );
            tracker.body(&json!({"type":"message","role":"assistant","content":[{"type":"text","text":"complete"}]}), true);
        }
        assert!(registry.binding(&key).is_some());
        drop(Tracker::new(
            namespace,
            prefix,
            key.clone(),
            registry.reserve_transcript().unwrap(),
            false,
        ));
        assert!(
            registry.binding(&key).is_some(),
            "a failed later request must keep existing ownership"
        );
        registry.abandon_unpublished_binding(&key);
    }
}
