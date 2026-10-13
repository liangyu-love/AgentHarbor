//! Complete upstream assistant output for Messages conversation continuations.
//! Private reasoning is never persisted or exposed as fabricated Anthropic
//! thinking/signatures. Keys include route scope, conversation prefix, visible
//! text and the complete tool-call group, not just a provider ID.
use super::canonical::{IncompleteReason, TerminalState};
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
        Self {
            reasoning: Arc::from(bytes),
            fingerprint,
        }
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
        let CachedOutput {
            reasoning,
            fingerprint,
        } = output;
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
    let hosted: Vec<_> = message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| {
            matches!(
                b["type"].as_str(),
                Some("server_tool_use" | "web_search_tool_result")
            )
        })
        .cloned()
        .collect();
    if hosted.is_empty() {
        json!([text, calls])
    } else {
        json!([text, calls, hosted])
    }
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

// Translate a single visible reply into the span already present in the full
// Chat body. Its final client calls receive temporary placeholders only for
// validating that isolated translation; placeholders never enter the request.
fn local_search_span(source: &Value, name: &str, visible_name: &str) -> Option<Vec<Value>> {
    let mut normalized =
        super::web_search::local_search_history(std::slice::from_ref(source), name, visible_name)
            .ok()?;
    let client_results: Vec<_> = source["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| json!({"type":"tool_result","tool_use_id":block["id"],"content":""}))
        .collect();
    let placeholders = client_results.len();
    if placeholders > 0 {
        normalized.push(json!({"role":"user","content":client_results}));
    }
    let single = json!({"model":"history","max_tokens":1,"messages":normalized});
    let translated =
        super::anthropic::request_body(ModelProtocol::ChatCompletions, &single).ok()?;
    let mut expected = translated["messages"].as_array()?.clone();
    expected.truncate(expected.len().checked_sub(placeholders)?);
    (!expected.is_empty()).then_some(expected)
}

fn local_search_name(source: &Value, messages: &[Value]) -> Option<(String, String)> {
    let call = source["content"]
        .as_array()?
        .iter()
        .find(|block| block["type"] == "server_tool_use")?;
    let id = call["id"].as_str()?;
    let upstream = messages
        .iter()
        .flat_map(|message| message["tool_calls"].as_array().into_iter().flatten())
        .find(|candidate| candidate["id"] == id)?;
    Some((
        upstream["function"]["name"].as_str()?.to_owned(),
        call["name"].as_str()?.to_owned(),
    ))
}

fn native_search_span(source: &Value, name: &str, visible_name: &str, protocol: ModelProtocol) -> Option<Vec<Value>> {
    let mut messages = super::web_search::local_search_history(std::slice::from_ref(source),name,visible_name).ok()?;
    if protocol == ModelProtocol::AnthropicMessages {return (!messages.is_empty()).then_some(messages)}
    let placeholders = client_placeholders(source);
    let count = placeholders.len();
    if count > 0 {messages.push(json!({"role":"user","content":placeholders}));}
    let value = super::anthropic::request_body(protocol,&json!({"model":"history","max_tokens":1,"messages":messages})).ok()?;
    let mut input = value["input"].as_array()?.clone();
    input.truncate(input.len().checked_sub(count)?);
    (!input.is_empty()).then_some(input)
}

fn client_placeholders(source: &Value) -> Vec<Value> {
    source["content"].as_array().into_iter().flatten().filter(|block| block["type"] == "tool_use")
        .map(|block| json!({"type":"tool_result","tool_use_id":block["id"],"content":""})).collect()
}

fn safe_native_search_replay(source: &Value, name: &str, messages: &[Value], protocol: ModelProtocol) -> bool {
    let mut messages = if protocol == ModelProtocol::AnthropicMessages {messages.to_vec()} else {
        let mut turns = Vec::new();
        for item in messages {
            let turn = match item["type"].as_str() {
                Some("reasoning") => continue,
                Some("message") if item["role"] == "assistant" => json!({"role":"assistant","content":item["content"].as_array().into_iter().flatten()
                    .map(|part| json!({"type":"text","text":part["text"]})).collect::<Vec<_>>()}),
                Some("function_call") => {
                    let Some(arguments) = item["arguments"].as_str().and_then(|args| serde_json::from_str::<Value>(args).ok()) else {return false};
                    json!({"role":"assistant","content":[{"type":"tool_use","id":item["call_id"],"name":item["name"],"input":arguments}]})
                }
                Some("function_call_output") => json!({"role":"user","content":[{"type":"tool_result","tool_use_id":item["call_id"],"content":item["output"]}]}),
                _ => return false,
            };
            // Responses splits a single assistant call group into output items.
            if turn["role"] == "assistant" && turns.last().is_some_and(|turn: &Value| turn["role"] == "assistant") {
                turns.last_mut().unwrap()["content"].as_array_mut().unwrap().extend(turn["content"].as_array().unwrap().iter().cloned());
            } else {turns.push(turn)}
        }
        turns
    };
    let placeholders = client_placeholders(source);
    let count = placeholders.len();
    if count > 0 {messages.push(json!({"role":"user","content":placeholders}));}
    let value = super::anthropic::request_body(ModelProtocol::ChatCompletions,&json!({"model":"history","max_tokens":1,"messages":messages}));
    value.ok().and_then(|value| value["messages"].as_array().cloned()).is_some_and(|mut messages| {
        let Some(len) = messages.len().checked_sub(count) else {return false};
        messages.truncate(len);
        safe_local_replay(source,name,&messages)
    })
}

/// Exact replay may end with unanswered caller tools, but never an unanswered
/// local search or an extra tool/role absent from the visible reply.
fn safe_local_replay(source: &Value, name: &str, messages: &[Value]) -> bool {
    use std::collections::{HashMap, HashSet};
    let Some(blocks) = source["content"].as_array() else {
        return messages.len() == 1
            && messages[0]["role"] == "assistant"
            && messages[0]
                .get("tool_calls")
                .is_none_or(|calls| calls.is_null() || calls == &json!([]));
    };
    let expected: HashMap<_, _> = blocks
        .iter()
        .filter(|block| matches!(block["type"].as_str(), Some("tool_use" | "server_tool_use")))
        .filter_map(|block| block["id"].as_str().map(|id| (id, block)))
        .collect();
    let client_ids: HashSet<_> = blocks
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .filter_map(|block| block["id"].as_str())
        .collect();
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();
    if messages
        .first()
        .is_none_or(|message| message["role"] != "assistant")
    {
        return false;
    }
    for message in messages {
        match message["role"].as_str() {
            Some("assistant") => {
                if !pending.is_empty() {
                    return false;
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) else {
                        return false;
                    };
                    let Some(block) = expected.get(id) else {
                        return false;
                    };
                    let search = block["type"] == "server_tool_use";
                    let correct_name = if search {
                        call["function"]["name"] == name
                    } else {
                        call["function"]["name"] == block["name"]
                    };
                    let arguments = call["function"]["arguments"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok());
                    if !seen.insert(id)
                        || !correct_name
                        || arguments.as_ref() != Some(&block["input"])
                    {
                        return false;
                    }
                    pending.insert(id, search);
                }
            }
            Some("tool") => {
                let Some(id) = message["tool_call_id"].as_str() else {
                    return false;
                };
                if pending.remove(id) != Some(true) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    seen.len() == expected.len()
        && pending.len() == client_ids.len()
        && pending
            .iter()
            .all(|(id, search)| !*search && client_ids.contains(id))
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
    response_options: super::ResponseOptions,
}
/// Extra client identity is independent of provider credentials and visible
/// conversation content. Aliases and revisions sharing a target cannot reuse
/// another client's retained private output.
#[derive(Clone, serde::Serialize)]
pub struct ClientScope<'a> {
    pub client_source: &'a str,
    pub config_identity: &'a str,
    pub model_alias: &'a str,
    pub route_revision: u64,
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
        Self::new_with_client_scope(scope, protocol, inbound, session, None)
    }
    pub fn new_with_client_scope(
        scope: &[&str],
        protocol: ModelProtocol,
        inbound: &[u8],
        session: Option<&str>,
        client_scope: Option<&ClientScope<'_>>,
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
            scope: digest(&if let Some(client_scope) = client_scope {
                json!([scope, protocol, user_id, session, fallback, client_scope])
            } else {
                json!([scope, protocol, user_id, session, fallback])
            }),
            protocol,
            messages,
            prefixes,
            response_options: super::web_search::options_from_tools(protocol, &value).unwrap_or_default(),
        })
    }
    fn key(&self, prefix_len: usize, calls: &[Value]) -> String {
        digest(&json!([self.scope, self.prefixes[prefix_len], calls]))
    }
    /// Keep the exact private Chat turns behind one visible hosted-search reply.
    /// Uses the same bounded, credential/client/session-scoped cache as reasoning.
    pub(crate) fn remember_local_search(
        &self,
        visible: &Value,
        name: &str,
        visible_name: &str,
        messages: &[Value],
    ) {
        let safe = if self.protocol == ModelProtocol::ChatCompletions {
            local_search_span(visible,name,visible_name).is_some() && safe_local_replay(visible,name,messages)
        } else {native_search_span(visible,name,visible_name,self.protocol).is_some() && safe_native_search_replay(visible,name,messages,self.protocol)};
        if visible["role"] != "assistant" || !safe {
            return;
        }
        let signature = visible_signature(visible);
        let key = self.key(self.messages.len(), signature.as_array().unwrap());
        let output = json!([{"type":if self.protocol == ModelProtocol::ChatCompletions {"local_search_replay"} else {"native_search_replay"},"name":name,"visible_name":visible_name,"messages":messages}]);
        if let Ok(bytes) = serde_json::to_vec(&output) {
            if let Ok(mut cache) = cache().lock() {
                cache.record(key, CachedOutput::from(bytes));
            }
        }
    }
    pub fn remember(&self, response: &Value) {
        if !cacheable_response(response) {
            return;
        }
        let Some(output) = response["output"].as_array() else {
            return;
        };
        let Ok(visible) = super::anthropic::response_body_with_options(response, &self.response_options) else {
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
            if self.protocol != ModelProtocol::ChatCompletions {
                if let Some(replay) = output.as_ref().and_then(|items| items.first()).filter(|item| item["type"] == "native_search_replay") {
                    let restored = (|| {
                        let name = replay["name"].as_str()?;
                        let visible_name = replay["visible_name"].as_str()?;
                        let replayed = replay["messages"].as_array()?;
                        if !safe_native_search_replay(source,name,replayed,self.protocol) {return None}
                        let expected = native_search_span(source,name,visible_name,self.protocol)?;
                        let items = body[if self.protocol == ModelProtocol::AnthropicMessages {"messages"} else {"input"}].as_array_mut()?;
                        let found = items[cursor..].windows(expected.len()).position(|items| items == expected.as_slice()).map(|index| (cursor+index,expected.len()))
                            .or_else(|| (self.protocol == ModelProtocol::AnthropicMessages).then(|| items[cursor..].iter().position(|item| item["role"] == "assistant" && visible_signature(item) == signature)
                                .map(|index| (cursor+index,1))).flatten())
                            .or_else(|| {
                                if self.protocol != ModelProtocol::Responses {return None}
                                let original = super::anthropic::request_body(ModelProtocol::Responses,&json!({"model":"history","max_tokens":1,"messages":[source]})).ok()?;
                                let original = original["input"].as_array().filter(|items| !items.is_empty())?;
                                items[cursor..].windows(original.len()).position(|items| items == original.as_slice()).map(|index| (cursor+index,original.len()))
                            });
                        let (index,len) = found?;
                        items.splice(index..index+len,replayed.iter().cloned());
                        cursor = index+replayed.len();
                        Some(())
                    })();
                    let _ = restored;
                    continue;
                }
            }
            if self.protocol == ModelProtocol::ChatCompletions {
                if let Some(messages) = body["messages"].as_array_mut() {
                    let replay = output
                        .as_ref()
                        .and_then(|items| items.first())
                        .filter(|item| item["type"] == "local_search_replay");
                    let has_hosted = source["content"].as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| {
                            matches!(
                                block["type"].as_str(),
                                Some("server_tool_use" | "web_search_tool_result")
                            )
                        })
                    });
                    if has_hosted || replay.is_some() {
                        let names = local_search_name(source, &messages[cursor..]).or_else(|| {
                            let replay = replay?;
                            Some((
                                replay["name"].as_str()?.to_owned(),
                                replay["visible_name"].as_str()?.to_owned(),
                            ))
                        });
                        if let Some((name, visible_name)) = names {
                            if let Some(expected) = local_search_span(source, &name, &visible_name)
                            {
                                if let Some(relative) = messages[cursor..]
                                    .windows(expected.len())
                                    .position(|part| part == expected.as_slice())
                                {
                                    let index = cursor + relative;
                                    cursor = index + expected.len();
                                    let replayed = replay
                                        .filter(|replay| {
                                            replay["name"] == name
                                                && replay["visible_name"] == visible_name
                                        })
                                        .and_then(|replay| replay["messages"].as_array())
                                        .filter(|replayed| {
                                            safe_local_replay(source, &name, replayed)
                                        });
                                    if let Some(replayed) = replayed {
                                        messages.splice(
                                            index..index + expected.len(),
                                            replayed.iter().cloned(),
                                        );
                                        cursor = index + replayed.len();
                                    }
                                }
                            }
                        }
                        continue;
                    }
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
    let state = TerminalState::from_canonical_response(response);
    let safe_status = state.is_success()
        || (matches!(
            state,
            TerminalState::Incomplete {
                reason: IncompleteReason::MaxOutputTokens
            }
        ) && response["output"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["type"] != "function_call")));
    safe_status
        && response["output"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
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

    fn hosted_visible(mixed: bool) -> Value {
        let mut blocks = vec![json!({"type":"text","text":"Searching"})];
        blocks.extend(
            super::super::web_search::blocks(
                &json!({"type":"web_search_call",
            "id":"local1","status":"completed","action":{"type":"search","query":"latest",
                "sources":[{"url":"https://example.org/latest","title":"Latest source"}]}}),
                None,
                0,
            )
            .unwrap(),
        );
        if mixed {
            blocks.push(
                json!({"type":"tool_use","id":"client1","name":"read_file","input":{"path":"a"}}),
            );
        } else {
            blocks.push(json!({"type":"text","text":"Final answer"}));
        }
        json!({"role":"assistant","content":blocks})
    }
    fn hosted_replay(mixed: bool) -> Vec<Value> {
        let mut calls = vec![json!({"id":"local1","type":"function","function":{
            "name":"internal_search","arguments":"{\"query\":\"latest\"}"}})];
        if mixed {
            calls.push(json!({"id":"client1","type":"function","function":{
                "name":"read_file","arguments":"{\"path\":\"a\"}"}}));
        }
        let mut replay = vec![
            json!({"role":"assistant","content":"Searching","reasoning_content":"private search reasoning","tool_calls":calls}),
            json!({"role":"tool","tool_call_id":"local1","content":"{\"results\":[{\"url\":\"https://example.org/latest\",\"title\":\"Latest source\",\"snippet\":\"Exact search snippet\"}]}"}),
        ];
        if !mixed {
            replay.push(json!({"role":"assistant","content":"Final answer","reasoning_content":"private final reasoning"}));
        }
        replay
    }
    fn hosted_next(first: &Value, mixed: bool) -> Value {
        let mut next = first.clone();
        next["messages"]
            .as_array_mut()
            .unwrap()
            .push(hosted_visible(mixed));
        next["messages"].as_array_mut().unwrap().push(if mixed {
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"client1","content":"Read succeeded"}]})
        } else { json!({"role":"user","content":"Continue"}) });
        next
    }
    fn hosted_chat(next: &Value, name: &str) -> Value {
        let mut normalized = next.clone();
        normalized["messages"] = json!(super::super::web_search::local_search_history(
            next["messages"].as_array().unwrap(),
            name,
            "web_search"
        )
        .unwrap());
        super::super::anthropic::request_body(ModelProtocol::ChatCompletions, &normalized).unwrap()
    }

    #[test]
    fn local_search_replays_exact_private_chat_rounds_and_actual_snippets() {
        for mixed in [false, true] {
            let scope = uuid::Uuid::new_v4().to_string();
            let first = source();
            let replay = hosted_replay(mixed);
            Context::new(
                &[&scope],
                ModelProtocol::ChatCompletions,
                first.to_string().as_bytes(),
            )
            .unwrap()
            .remember_local_search(
                &hosted_visible(mixed),
                "internal_search",
                "web_search",
                &replay,
            );
            let next = hosted_next(&first, mixed);
            let mut body = hosted_chat(&next, "internal_search");
            Context::new(
                &[&scope],
                ModelProtocol::ChatCompletions,
                next.to_string().as_bytes(),
            )
            .unwrap()
            .restore(&mut body);
            let messages = body["messages"].as_array().unwrap();
            assert_eq!(&messages[1..1 + replay.len()], replay.as_slice());
            assert!(messages[2]["content"]
                .as_str()
                .unwrap()
                .contains("Exact search snippet"));
            assert!(!next.to_string().contains("private search reasoning"));
            if mixed {
                assert_eq!(messages[3]["tool_call_id"], "client1");
            } else {
                assert_eq!(messages[3]["reasoning_content"], "private final reasoning");
            }
            super::super::chat::validate_tool_history(&body).unwrap();
        }
    }

    #[test]
    fn local_search_replay_misses_preserve_public_context_without_private_leaks() {
        let scope = uuid::Uuid::new_v4().to_string();
        let first = source();
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            first.to_string().as_bytes(),
        )
        .unwrap()
        .remember_local_search(
            &hosted_visible(false),
            "internal_search",
            "web_search",
            &hosted_replay(false),
        );
        for case in ["scope", "prefix", "arguments", "internal_name"] {
            let mut next = hosted_next(&first, false);
            if case == "prefix" {
                next["messages"][0]["content"] = json!("Another conversation");
            }
            if case == "arguments" {
                next["messages"][1]["content"][1]["input"]["query"] = json!("different query");
            }
            let name = if case == "internal_name" {
                "new_internal_search"
            } else {
                "internal_search"
            };
            let mut body = hosted_chat(&next, name);
            let before = body.clone();
            Context::new(
                &[if case == "scope" {
                    "another credential"
                } else {
                    &scope
                }],
                ModelProtocol::ChatCompletions,
                next.to_string().as_bytes(),
            )
            .unwrap()
            .restore(&mut body);
            assert_eq!(body, before, "{case}");
            assert!(body.to_string().contains("Latest source"));
            assert!(!body.to_string().contains("private search reasoning"));
            assert!(!body.to_string().contains("Exact search snippet"));
            super::super::chat::validate_tool_history(&body).unwrap();
        }
    }

    #[test]
    fn local_search_replay_validates_pending_client_tools_and_extra_roles() {
        let visible = hosted_visible(true);
        let replay = hosted_replay(true);
        assert!(safe_local_replay(&visible, "internal_search", &replay));
        for case in 0..5 {
            let mut invalid = replay.clone();
            match case {
                0 => invalid[0]["tool_calls"][0]["id"] = json!("other"),
                1 => invalid[0]["tool_calls"][0]["function"]["arguments"] = json!("{\"query\":\"changed\"}"),
                2 => invalid.push(json!({"role":"user","content":"foreign conversation"})),
                3 => invalid.push(json!({"role":"tool","tool_call_id":"client1","content":"invented client result"})),
                _ => { invalid.remove(1); },
            }
            assert!(
                !safe_local_replay(&visible, "internal_search", &invalid),
                "{case}"
            );
        }
    }

    #[test]
    fn consecutive_hosted_turns_restore_their_own_private_rounds() {
        let scope = uuid::Uuid::new_v4().to_string();
        let first = source();
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            first.to_string().as_bytes(),
        )
        .unwrap()
        .remember_local_search(
            &hosted_visible(false),
            "internal_search",
            "web_search",
            &hosted_replay(false),
        );
        let second = hosted_next(&first, false);
        let mut visible = hosted_visible(false);
        visible["content"][1]["id"] = json!("local2");
        visible["content"][2]["tool_use_id"] = json!("local2");
        let mut replay = hosted_replay(false);
        replay[0]["tool_calls"][0]["id"] = json!("local2");
        replay[0]["reasoning_content"] = json!("second private reasoning");
        replay[1]["tool_call_id"] = json!("local2");
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            second.to_string().as_bytes(),
        )
        .unwrap()
        .remember_local_search(&visible, "internal_search", "web_search", &replay);
        let mut third = second;
        third["messages"]
            .as_array_mut()
            .unwrap()
            .extend([visible, json!({"role":"user","content":"Third turn"})]);
        let mut body = hosted_chat(&third, "internal_search");
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            third.to_string().as_bytes(),
        )
        .unwrap()
        .restore(&mut body);
        assert_eq!(
            body["messages"][1]["reasoning_content"],
            "private search reasoning"
        );
        assert_eq!(
            body["messages"][5]["reasoning_content"],
            "second private reasoning"
        );
        super::super::chat::validate_tool_history(&body).unwrap();
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
        cache.record(
            "same-turn".into(),
            b"another session output".to_vec().into(),
        );
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
            assert!(!cacheable_response(
                &json!({"status":status,"incomplete_details":{"reason":"max_output_tokens"},"output":[]})
            ));
        }
        let scope = uuid::Uuid::new_v4().to_string();
        let original = source();
        let mut response = canonical();
        response["output"][0]["summary"][0]["text"] = json!("  ");
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            original.to_string().as_bytes(),
        )
        .unwrap()
        .remember(&response);
        let next = next(&original);
        let mut body =
            super::super::anthropic::request_body(ModelProtocol::ChatCompletions, &next).unwrap();
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            next.to_string().as_bytes(),
        )
        .unwrap()
        .restore(&mut body);
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
        before["messages"]
            .as_array_mut()
            .unwrap()
            .push(request["messages"][1].clone());
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            before.to_string().as_bytes(),
        )
        .unwrap()
        .remember(&canonical());
        let mut body =
            super::super::anthropic::request_body(ModelProtocol::ChatCompletions, &request)
                .unwrap();
        Context::new(
            &[&scope],
            ModelProtocol::ChatCompletions,
            request.to_string().as_bytes(),
        )
        .unwrap()
        .restore(&mut body);
        assert_eq!(body["messages"].as_array().unwrap().len(), 4);
        assert!(body["messages"][1].get("tool_calls").is_none());
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "c1");
        assert_eq!(body["messages"][1]["reasoning_content"], "first reasoning");
        assert_eq!(body["messages"][2]["reasoning_content"], "exact reasoning");
    }
}
