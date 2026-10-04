//! Complete upstream assistant output for Messages conversation continuations.
//! Private reasoning is never persisted or exposed as fabricated Anthropic
//! thinking/signatures. Keys include route scope, conversation prefix, visible
//! text and the complete tool-call group, not just a provider ID.
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_TURNS: usize = 512;
const TTL: Duration = Duration::from_secs(3600);

// Fingerprint the complete serialized output before entering the shared lock.
// Private output stays in memory; only constant-size hashes are compared there.
struct CachedOutput {
    reasoning: Arc<[u8]>,
    fingerprint: [u8; 32],
}
impl From<Vec<u8>> for CachedOutput {
    fn from(bytes: Vec<u8>) -> Self {
        let fingerprint = Sha256::digest(&bytes).into();
        Self { reasoning: Arc::from(bytes), fingerprint }
    }
}

struct Turn {
    key: String,
    reasoning: Arc<[u8]>,
    fingerprint: [u8; 32],
    created: Instant,
    ambiguous: bool,
}
#[derive(Default)]
struct Cache {
    turns: VecDeque<Turn>,
    bytes: usize,
}
impl Cache {
    fn prune(&mut self) {
        while self
            .turns
            .front()
            .is_some_and(|t| t.created.elapsed() >= TTL)
        {
            self.pop();
        }
    }
    fn pop(&mut self) {
        if let Some(t) = self.turns.pop_front() {
            self.bytes -= t.reasoning.len() + t.key.len();
        }
    }
    fn get(&mut self, key: &str) -> Option<Arc<[u8]>> {
        self.prune();
        let index = self.turns.iter().position(|t| t.key == key)?;
        if self.turns[index].ambiguous {
            return None;
        }
        let mut turn = self.turns.remove(index)?;
        turn.created = Instant::now();
        let reasoning = turn.reasoning.clone();
        self.turns.push_back(turn);
        Some(reasoning)
    }
    fn insert(&mut self, key: String, output: CachedOutput) {
        let CachedOutput { reasoning, fingerprint } = output;
        self.prune();
        if let Some(index) = self.turns.iter().position(|t| t.key == key) {
            let old = self.turns.remove(index).unwrap();
            self.bytes -= old.reasoning.len() + old.key.len();
        }
        let size = reasoning.len() + key.len();
        if size > MAX_BYTES {
            return;
        }
        while self.turns.len() >= MAX_TURNS || self.bytes + size > MAX_BYTES {
            self.pop();
        }
        self.bytes += size;
        self.turns.push_back(Turn {
            key,
            reasoning,
            fingerprint,
            created: Instant::now(),
            ambiguous: false,
        });
    }
    // Two replies can have identical visible text/history but different private
    // output. No key can distinguish them: retain a bounded tombstone instead
    // of replacing one session's data with another's.
    fn record(&mut self, key: String, output: CachedOutput) {
        self.prune();
        if let Some(index) = self.turns.iter().position(|turn| turn.key == key) {
            let mut turn = self.turns.remove(index).unwrap();
            if turn.ambiguous || turn.fingerprint != output.fingerprint {
                self.bytes -= turn.reasoning.len();
                turn.reasoning = Arc::from([]);
                turn.ambiguous = true;
            }
            turn.created = Instant::now();
            self.turns.push_back(turn);
        } else {
            self.insert(key, output);
        }
    }
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn digest(value: &Value) -> String {
    let mut sorted = value.clone();
    fn sort(value: &mut Value) {
        match value {
            Value::Object(obj) => {
                obj.sort_keys();
                for v in obj.values_mut() {
                    sort(v);
                }
            }
            Value::Array(items) => {
                for v in items {
                    sort(v);
                }
            }
            _ => {}
        }
    }
    sort(&mut sorted);
    format!("{:x}", Sha256::digest(sorted.to_string().as_bytes()))
}
fn visible_signature(message: &Value) -> Value {
    let text = if let Some(text) = message["content"].as_str() {
        text.to_owned()
    } else {
        message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    };
    let calls: Vec<_> = message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "tool_use")
        .map(|b| json!([b["id"], b["name"], b["input"]]))
        .collect();
    json!([text, calls])
}
fn chat_signature(message: &Value) -> Value {
    let text = if let Some(text) = message["content"].as_str() {
        text.to_owned()
    } else {
        message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    };
    let calls: Vec<_> = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            json!([
                c["id"],
                c["function"]["name"],
                c["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
            ])
        })
        .collect();
    json!([text, calls])
}

fn prefix_hashes(messages: &[Value]) -> Vec<String> {
    fn content(value: &Value) -> Value {
        if let Some(text) = value.as_str() {
            return json!([{"type":"text","text":text}]);
        }
        let mut value = value.clone();
        if let Some(blocks) = value.as_array_mut() {
            for block in blocks {
                if let Some(object) = block.as_object_mut() {
                    object.remove("cache_control");
                }
                if block["type"] == "tool_result" {
                    block["content"] = content(&block["content"]);
                }
            }
        }
        value
    }
    let mut running = Sha256::new();
    let mut hashes = vec![format!("{:x}", running.clone().finalize())];
    for message in messages {
        if matches!(message["role"].as_str(), Some("user" | "assistant")) {
            let item = digest(&if message["role"] == "assistant" {
                json!(["assistant", visible_signature(message)])
            } else {
                json!({"role":message["role"],"content":content(&message["content"])})
            });
            running.update(item.as_bytes());
        }
        hashes.push(format!("{:x}", running.clone().finalize()));
    }
    hashes
}

pub struct Context {
    scope: String,
    protocol: ModelProtocol,
    messages: Vec<Value>,
    prefixes: Vec<String>,
}
impl Context {
    #[cfg(test)]
    pub fn new(scope: &[&str], protocol: ModelProtocol, inbound: &[u8]) -> Option<Self> {
        Self::new_with_session(scope, protocol, inbound, None)
    }
    pub fn new_with_session(
        scope: &[&str],
        protocol: ModelProtocol,
        inbound: &[u8],
        session: Option<&str>,
    ) -> Option<Self> {
        let value: Value = serde_json::from_slice(inbound).ok()?;
        let messages = value["messages"].as_array()?.clone();
        let user_id = value
            .pointer("/metadata/user_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty());
        let session = session.filter(|id| !id.trim().is_empty());
        // Dynamic system context is excluded only when a reliable session ID
        // separates conversations. Without one, changing context safely misses.
        let fallback = if user_id.is_none() && session.is_none() {
            let mut context = json!([
                value.get("system"),
                messages
                    .iter()
                    .filter(|message| matches!(
                        message["role"].as_str(),
                        Some("system" | "developer")
                    ))
                    .collect::<Vec<_>>()
            ]);
            fn strip_cache(value: &mut Value) {
                match value {
                    Value::Object(object) => {
                        object.remove("cache_control");
                        for value in object.values_mut() {
                            strip_cache(value);
                        }
                    }
                    Value::Array(values) => {
                        for value in values {
                            strip_cache(value);
                        }
                    }
                    _ => {}
                }
            }
            strip_cache(&mut context);
            Some(context)
        } else {
            None
        };
        let prefixes = prefix_hashes(&messages);
        Some(Self {
            scope: digest(&json!([scope, protocol, user_id, session, fallback])),
            protocol,
            messages,
            prefixes,
        })
    }
    fn key(&self, prefix_len: usize, calls: &[Value]) -> String {
        digest(&json!([self.scope, self.prefixes[prefix_len], calls]))
    }
    pub fn remember(&self, response: &Value) {
        if !cacheable_response(response) {
            return;
        }
        let Some(output) = response["output"].as_array() else {
            return;
        };
        let Ok(visible) = super::anthropic::response_body(response) else {
            return;
        };
        // Every assistant reply matters, including greetings and final answers:
        // DeepSeek requires its reasoning_text throughout tool-enabled histories.
        let signature = visible_signature(&visible);
        let key = self.key(self.messages.len(), signature.as_array().unwrap());
        if let Ok(bytes) = serde_json::to_vec(output) {
            let output = CachedOutput::from(bytes);
            if let Ok(mut cache) = cache().lock() {
                cache.record(key, output);
            }
        }
    }
    pub fn restore(&self, body: &mut Value) {
        // Computing signatures can traverse large tool arguments. Do this
        // outside the shared lock as well as decoding the cached output.
        let pending: Vec<_> = self.messages.iter().enumerate()
            .filter(|(_, message)| message["role"] == "assistant")
            .map(|(index, message)| {
                let signature = visible_signature(message);
                let key = self.key(index, signature.as_array().unwrap());
                (message, signature, key)
            }).collect();
        let mut restored = Vec::new();
        if let Ok(mut cache) = cache().lock() {
            for (message, signature, key) in pending {
                restored.push((message, signature, cache.get(&key)));
            }
        }
        let mut cursor = 0;
        for (source, signature, output) in restored {
            let output = output.and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
                .filter(|items| !items.is_empty());
            if self.protocol == ModelProtocol::ChatCompletions {
                if let Some(messages) = body["messages"].as_array_mut() {
                    if let Some(relative) = messages[cursor..]
                        .iter()
                        .position(|m| m["role"] == "assistant" && chat_signature(m) == signature)
                    {
                        let index = cursor + relative;
                        cursor = index + 1;
                        let Some(output) = output else { continue };
                        let reasoning: Vec<_> =
                            output.iter().filter(|v| v["type"] == "reasoning").collect();
                        if reasoning.is_empty() {
                            continue;
                        }
                        let text = reasoning
                            .iter()
                            .flat_map(|r| {
                                let content = r["content"].as_array().filter(|parts| {
                                    parts.iter().any(|p| p["type"] == "reasoning_text")
                                });
                                content
                                    .or_else(|| r["summary"].as_array())
                                    .into_iter()
                                    .flatten()
                            })
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.trim().is_empty() {
                            messages[index]["reasoning_content"] = json!(text);
                        }
                    }
                }
            } else if let Some(input) = body["input"].as_array_mut() {
                // Locate exactly the items generated for this original assistant
                // turn, then replay the original upstream items with all IDs and
                // private reasoning fields. Never reduce reasoning to a summary.
                let single = json!({"model":"history","max_tokens":1,"messages":[source]});
                let Ok(canonical) =
                    super::anthropic::request_body(ModelProtocol::Responses, &single)
                else {
                    continue;
                };
                let Some(items) = canonical["input"].as_array().filter(|v| !v.is_empty()) else {
                    continue;
                };
                if let Some(relative) = input[cursor..]
                    .windows(items.len())
                    .position(|part| part == items.as_slice())
                {
                    let index = cursor + relative;
                    if let Some(output) = output {
                        let length = output.len();
                        input.splice(index..index + items.len(), output);
                        cursor = index + length;
                    } else {
                        cursor = index + items.len();
                    }
                }
            }
        }
    }
}

/// Token-limited text is a delivered assistant turn too. Never replay a failed
/// response or a tool call whose arguments were cut short.
pub(super) fn cacheable_response(response: &Value) -> bool {
    let safe_status = response["status"] == "completed"
        || (response["status"] == "incomplete"
            && response["incomplete_details"]["reason"] == "max_output_tokens"
            && response["output"]
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item["type"] != "function_call")));
    safe_status
        && response["output"].as_array().is_some_and(|items| !items.is_empty())
        && super::anthropic::response_body(response).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::{self, Translation};
    fn source() -> Value {
        json!({"model":"m","max_tokens":128,"messages":[{"role":"user","content":"fixture"}]})
    }
    fn canonical() -> Value {
        json!({"object":"response","status":"completed","model":"m","output":[
        {"type":"reasoning","id":"rs-original","encrypted_content":"opaque-original","summary":[{"type":"summary_text","text":"exact reasoning"}]},
        {"type":"function_call","id":"fc-original","call_id":"c1","name":"lookup","arguments":"{}","status":"completed"}]})
    }
    fn next(first: &Value) -> Value {
        let mut value = first.clone();
        let messages = value["messages"].as_array_mut().unwrap();
        messages.push(json!({"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"lookup","input":{}}]}));
        messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"ok"}]}));
        value
    }
    #[test]
    fn reasoning_replay_preserves_chat_text_and_responses_opaque_items() {
        for target in [ModelProtocol::ChatCompletions, ModelProtocol::Responses] {
            let scope = uuid::Uuid::new_v4().to_string();
            let original = source();
            Context::new(&[&scope], target, original.to_string().as_bytes())
                .unwrap()
                .remember(&canonical());
            let next = next(&original);
            let mut body = protocol::request_body(
                Translation {
                    from: ModelProtocol::AnthropicMessages,
                    to: target,
                },
                &next,
            )
            .unwrap()
            .body;
            Context::new(&[&scope], target, next.to_string().as_bytes())
                .unwrap()
                .restore(&mut body);
            if target == ModelProtocol::ChatCompletions {
                assert_eq!(body["messages"][1]["reasoning_content"], "exact reasoning");
            } else {
                assert_eq!(body["input"][1], canonical()["output"][0]);
            }
        }
    }
    #[test]
    fn replay_isolates_scope_history_and_tool_arguments() {
        let scope = uuid::Uuid::new_v4().to_string();
        let target = ModelProtocol::ChatCompletions;
        let original = source();
        Context::new(&[&scope], target, original.to_string().as_bytes())
            .unwrap()
            .remember(&canonical());
        for case in 0..3 {
            let mut next = next(&original);
            if case == 1 {
                next["messages"][0]["content"] = json!("different conversation");
            }
            if case == 2 {
                next["messages"][1]["content"][0]["input"] = json!({"changed":true});
            }
            let mut body = protocol::request_body(
                Translation {
                    from: ModelProtocol::AnthropicMessages,
                    to: target,
                },
                &next,
            )
            .unwrap()
            .body;
            Context::new(
                &[if case == 0 {
                    "another provider or credential"
                } else {
                    &scope
                }],
                target,
                next.to_string().as_bytes(),
            )
            .unwrap()
            .restore(&mut body);
            assert!(body["messages"][1].get("reasoning_content").is_none());
        }
    }
    #[test]
    fn cache_is_bounded_expires_and_replaces_without_leaking_size() {
        let mut cache = Cache::default();
        cache.insert("a".into(), vec![0; 10].into());
        cache.insert("a".into(), vec![0; 20].into());
        assert_eq!(cache.bytes, 21);
        cache.turns[0].created = Instant::now() - TTL;
        cache.prune();
        assert_eq!(cache.bytes, 0);
        for i in 0..MAX_TURNS + 1 {
            cache.insert(i.to_string(), vec![0].into());
        }
        assert_eq!(cache.turns.len(), MAX_TURNS);
        cache.insert("oversize".into(), vec![0; MAX_BYTES].into());
        assert!(cache.bytes <= MAX_BYTES);
        assert_eq!(cache.turns.len(), MAX_TURNS);
    }
    #[test]
    fn dynamic_system_cache_markers_and_json_key_order_do_not_break_identity() {
        let scope = uuid::Uuid::new_v4().to_string();
        let target = ModelProtocol::ChatCompletions;
        let mut first = source();
        first["metadata"] = json!({"user_id":"session-one"});
        first["messages"][0]["content"] =
            json!([{"type":"text","text":"fixture","cache_control":{"type":"ephemeral"}}]);
        first["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"system","content":"changing token budget 1"}));
        Context::new(&[&scope], target, first.to_string().as_bytes())
            .unwrap()
            .remember(&canonical());
        let mut next = next(&first);
        next["messages"][0]["content"] = json!("fixture");
        next["messages"][1]["content"] = json!("changing token budget 2");
        let translation = Translation {
            from: ModelProtocol::AnthropicMessages,
            to: target,
        };
        let mut body = protocol::request_body(translation, &next).unwrap().body;
        Context::new(&[&scope], target, next.to_string().as_bytes())
            .unwrap()
            .restore(&mut body);
        assert_eq!(body["messages"][2]["reasoning_content"], "exact reasoning");
        assert_eq!(body["messages"][1]["content"], "changing token budget 2");
        next["metadata"]["user_id"] = json!("session-two");
        let mut other = protocol::request_body(translation, &next).unwrap().body;
        Context::new(&[&scope], target, next.to_string().as_bytes())
            .unwrap()
            .restore(&mut other);
        assert!(other["messages"][2].get("reasoning_content").is_none());
    }
    #[test]
    fn multiple_tool_rounds_restore_each_assistants_own_reasoning() {
        let scope = uuid::Uuid::new_v4().to_string();
        let target = ModelProtocol::ChatCompletions;
        let first = source();
        Context::new(&[&scope], target, first.to_string().as_bytes())
            .unwrap()
            .remember(&canonical());
        let second = next(&first);
        let mut output = canonical();
        output["output"][0]["summary"][0]["text"] = json!("second reasoning");
        output["output"][1]["call_id"] = json!("c2");
        Context::new(&[&scope], target, second.to_string().as_bytes())
            .unwrap()
            .remember(&output);
        let mut third = next(&second);
        third["messages"][3]["content"][0]["id"] = json!("c2");
        third["messages"][4]["content"][0]["tool_use_id"] = json!("c2");
        let mut body = protocol::request_body(
            Translation {
                from: ModelProtocol::AnthropicMessages,
                to: target,
            },
            &third,
        )
        .unwrap()
        .body;
        Context::new(&[&scope], target, third.to_string().as_bytes())
            .unwrap()
            .restore(&mut body);
        assert_eq!(body["messages"][1]["reasoning_content"], "exact reasoning");
        assert_eq!(body["messages"][3]["reasoning_content"], "second reasoning");
    }
    #[test]
    fn absent_reasoning_and_failed_responses_never_invent_chat_fields() {
        let first = source();
        let next = next(&first);
        for failed in [false, true] {
            let scope = uuid::Uuid::new_v4().to_string();
            let mut response = canonical();
            if failed {
                response["status"] = json!("failed");
            } else {
                response["output"].as_array_mut().unwrap().remove(0);
            }
            Context::new(
                &[&scope],
                ModelProtocol::ChatCompletions,
                first.to_string().as_bytes(),
            )
            .unwrap()
            .remember(&response);
            let mut body = protocol::request_body(
                Translation {
                    from: ModelProtocol::AnthropicMessages,
                    to: ModelProtocol::ChatCompletions,
                },
                &next,
            )
            .unwrap()
            .body;
            Context::new(
                &[&scope],
                ModelProtocol::ChatCompletions,
                next.to_string().as_bytes(),
            )
            .unwrap()
            .restore(&mut body);
            assert!(body["messages"][1].get("reasoning_content").is_none());
        }
    }
    #[test]
    fn deepseek_responses_replays_greeting_and_tool_reasoning_text_with_original_items() {
        let scope = uuid::Uuid::new_v4().to_string();
        let target = ModelProtocol::Responses;
        let first = source();
        let greeting = json!({"object":"response","status":"completed","model":"m","output":[
            {"type":"reasoning","id":"rs-greeting","status":"completed","content":[{"type":"reasoning_text","text":"greeting reasoning"}],"summary":[]},
            {"type":"message","id":"msg-greeting","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello","annotations":[]}]}]});
        Context::new(&[&scope], target, first.to_string().as_bytes())
            .unwrap()
            .remember(&greeting);
        let mut second = first.clone();
        second["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":[{"type":"text","text":"hello"}]}),
            json!({"role":"user","content":"use a tool"}),
        ]);
        let mut tool_reply = canonical();
        tool_reply["output"][0] = json!({"type":"reasoning","id":"rs-tool","status":"completed","content":[{"type":"reasoning_text","text":"tool reasoning"}],"summary":[]});
        Context::new(&[&scope], target, second.to_string().as_bytes())
            .unwrap()
            .remember(&tool_reply);
        let next = next(&second);
        let mut body = protocol::request_body(
            Translation {
                from: ModelProtocol::AnthropicMessages,
                to: target,
            },
            &next,
        )
        .unwrap()
        .body;
        Context::new(&[&scope], target, next.to_string().as_bytes())
            .unwrap()
            .restore(&mut body);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items[1], greeting["output"][0]);
        assert_eq!(items[2], greeting["output"][1]);
        assert_eq!(items[4], tool_reply["output"][0]);
        assert_eq!(items[5], tool_reply["output"][1]);
        assert_eq!(items[6]["type"], "function_call_output");
    }

    #[test]
    fn token_limited_text_replays_reasoning_but_incomplete_tools_and_failures_do_not() {
        for target in [ModelProtocol::ChatCompletions, ModelProtocol::Responses] {
            for mode in ["text", "tool", "failed", "filter"] {
                let scope = uuid::Uuid::new_v4().to_string();
                let first = source();
                let mut response = json!({"object":"response","status":"incomplete","model":"m",
                    "incomplete_details":{"reason":"max_output_tokens"},"output":[
                        {"type":"reasoning","id":"rs-limited","content":[{"type":"reasoning_text","text":"limited reasoning"}],"summary":[]},
                        {"type":"message","id":"msg-limited","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"partial"}]}]});
                if mode == "tool" {
                    response["output"]
                        .as_array_mut()
                        .unwrap()
                        .push(canonical()["output"][1].clone());
                }
                if mode == "failed" {
                    response["status"] = json!("failed");
                }
                if mode == "filter" {
                    response["incomplete_details"]["reason"] = json!("content_filter");
                }
                Context::new(&[&scope], target, first.to_string().as_bytes())
                    .unwrap()
                    .remember(&response);
                let mut next = first.clone();
                next["messages"].as_array_mut().unwrap().extend([
                    json!({"role":"assistant","content":"partial"}),
                    json!({"role":"user","content":"continue with a tool"}),
                ]);
                let mut body = protocol::request_body(
                    Translation {
                        from: ModelProtocol::AnthropicMessages,
                        to: target,
                    },
                    &next,
                )
                .unwrap()
                .body;
                Context::new(&[&scope], target, next.to_string().as_bytes())
                    .unwrap()
                    .restore(&mut body);
                if target == ModelProtocol::ChatCompletions {
                    assert_eq!(
                        body["messages"][1]["reasoning_content"].as_str(),
                        (mode == "text").then_some("limited reasoning")
                    );
                } else {
                    assert_eq!(
                        body["input"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v["id"] == "rs-limited"),
                        mode == "text"
                    );
                }
            }
        }
    }

    #[test]
    fn missing_session_ids_isolate_system_and_developer_context() {
        for target in [ModelProtocol::ChatCompletions, ModelProtocol::Responses] {
            for inline in [false, true] {
                let scope = uuid::Uuid::new_v4().to_string();
                let mut first = source();
                first["metadata"] = json!({"user_id":""});
                if inline {
                    first["messages"]
                        .as_array_mut()
                        .unwrap()
                        .insert(0, json!({"role":"developer","content":"task A"}));
                } else {
                    first["system"] = json!("task A");
                }
                let mut second = first.clone();
                if inline {
                    second["messages"][0]["content"] = json!("task B");
                } else {
                    second["system"] = json!("task B");
                }
                let text = |id: &str| {
                    json!({"object":"response","status":"completed","model":"m","output":[
                    {"type":"reasoning","id":id,"content":[{"type":"reasoning_text","text":id}],"summary":[]},
                    {"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]})
                };
                for (request, id) in [(&first, "reasoning-A"), (&second, "reasoning-B")] {
                    Context::new(&[&scope], target, request.to_string().as_bytes())
                        .unwrap()
                        .remember(&text(id));
                }
                for (request, id) in [(first, "reasoning-A"), (second, "reasoning-B")] {
                    let mut next = request;
                    next["messages"].as_array_mut().unwrap().extend([
                        json!({"role":"assistant","content":"hello"}),
                        json!({"role":"user","content":"next"}),
                    ]);
                    let mut body = protocol::request_body(
                        Translation {
                            from: ModelProtocol::AnthropicMessages,
                            to: target,
                        },
                        &next,
                    )
                    .unwrap()
                    .body;
                    Context::new(&[&scope], target, next.to_string().as_bytes())
                        .unwrap()
                        .restore(&mut body);
                    assert!(body.to_string().contains(id));
                    assert!(!body.to_string().contains(if id == "reasoning-A" {
                        "reasoning-B"
                    } else {
                        "reasoning-A"
                    }));
                }
            }
        }
    }

    #[test]
    fn explicit_session_header_keeps_dynamic_system_compatible() {
        let scope = uuid::Uuid::new_v4().to_string();
        let mut first = source();
        first["system"] = json!("dynamic 1");
        Context::new_with_session(
            &[&scope],
            ModelProtocol::ChatCompletions,
            first.to_string().as_bytes(),
            Some("header-session"),
        )
        .unwrap()
        .remember(&canonical());
        let mut next = next(&first);
        next["system"] = json!("dynamic 2");
        let pair = Translation {
            from: ModelProtocol::AnthropicMessages,
            to: ModelProtocol::ChatCompletions,
        };
        let mut body = protocol::request_body(pair, &next).unwrap().body;
        Context::new_with_session(
            &[&scope],
            pair.to,
            next.to_string().as_bytes(),
            Some("header-session"),
        )
        .unwrap()
        .restore(&mut body);
        assert_eq!(body["messages"][2]["reasoning_content"], "exact reasoning");
    }

    #[test]
    fn indistinguishable_replies_never_overwrite_private_output() {
        let mut cache = Cache::default();
        cache.record("same-turn".into(), b"first output".to_vec().into());
        cache.record("same-turn".into(), b"first output".to_vec().into());
        assert_eq!(&*cache.get("same-turn").unwrap(), b"first output");
        cache.record("same-turn".into(), b"another session output".to_vec().into());
        assert!(cache.get("same-turn").is_none());
        assert_eq!(cache.bytes, "same-turn".len());
        cache.record("same-turn".into(), b"first output".to_vec().into());
        assert!(cache.get("same-turn").is_none());
        cache.turns[0].created = Instant::now() - TTL;
        cache.prune();
        assert_eq!(cache.bytes, 0);
        cache.record("same-turn".into(), b"fresh output".to_vec().into());
        assert_eq!(&*cache.get("same-turn").unwrap(), b"fresh output");
    }
    #[test]
    fn empty_output_and_blank_reasoning_are_not_replayed() {
        for status in ["completed", "incomplete"] {
            assert!(!cacheable_response(&json!({"status":status,"incomplete_details":{"reason":"max_output_tokens"},"output":[]})));
        }
        let scope = uuid::Uuid::new_v4().to_string();
        let original = source();
        let mut response = canonical();
        response["output"][0]["summary"][0]["text"] = json!("  ");
        Context::new(&[&scope], ModelProtocol::ChatCompletions, original.to_string().as_bytes()).unwrap().remember(&response);
        let next = next(&original);
        let mut body = super::super::anthropic::request_body(ModelProtocol::ChatCompletions, &next).unwrap();
        Context::new(&[&scope], ModelProtocol::ChatCompletions, next.to_string().as_bytes()).unwrap().restore(&mut body);
        assert!(body["messages"][1].get("reasoning_content").is_none());
    }

    #[test]
    fn consecutive_assistants_keep_their_tool_call_boundaries() {
        let request = json!({"model":"m","max_tokens":100,"messages":[
            {"role":"user","content":"hello"},
            {"role":"assistant","content":"Earlier reply"},
            {"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"lookup","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"ok"}]}]});
        let scope = uuid::Uuid::new_v4().to_string();
        let mut before = request.clone();
        before["messages"].as_array_mut().unwrap().truncate(1);
        Context::new(&[&scope], ModelProtocol::ChatCompletions, before.to_string().as_bytes()).unwrap()
            .remember(&json!({"status":"completed","output":[
                {"type":"reasoning","summary":[{"type":"summary_text","text":"first reasoning"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Earlier reply"}]}]}));
        before["messages"].as_array_mut().unwrap().push(request["messages"][1].clone());
        Context::new(&[&scope], ModelProtocol::ChatCompletions, before.to_string().as_bytes()).unwrap().remember(&canonical());
        let mut body = super::super::anthropic::request_body(ModelProtocol::ChatCompletions, &request).unwrap();
        Context::new(&[&scope], ModelProtocol::ChatCompletions, request.to_string().as_bytes()).unwrap().restore(&mut body);
        assert_eq!(body["messages"].as_array().unwrap().len(), 4);
        assert!(body["messages"][1].get("tool_calls").is_none());
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "c1");
        assert_eq!(body["messages"][1]["reasoning_content"], "first reasoning");
        assert_eq!(body["messages"][2]["reasoning_content"], "exact reasoning");
    }

}
