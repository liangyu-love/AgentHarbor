//! Chat reasoning that a Responses caller cannot hand back.
//!
//! A Chat upstream's `reasoning_content` reaches Codex as a readable reasoning
//! item. Codex replays it, but a relay never receives replayed reasoning (it
//! may be another service's; see `codex_compat`), while DeepSeek refuses a
//! tool-enabled history whose assistant turns lack their `reasoning_content`.
//! The router keeps that text itself, for the route that produced it, and
//! puts it back on the assistant turn it came with. Only a reply that carried
//! `reasoning_content` is kept: hosts that answer in another field may refuse
//! this one. In memory and bounded, as Messages continuations are; after a
//! restart the earlier turns go without it, as they did before.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_TURNS: usize = 2048;
/// Long enough for a paused agent session; every restore refreshes it.
const TTL: Duration = Duration::from_secs(6 * 3600);

struct Turn {
    key: String,
    text: String,
    used: Instant,
}

#[derive(Default)]
struct Store {
    turns: VecDeque<Turn>,
    bytes: usize,
}

impl Store {
    fn prune(&mut self) {
        while self
            .turns
            .front()
            .is_some_and(|turn| turn.used.elapsed() >= TTL)
        {
            self.pop();
        }
    }
    fn pop(&mut self) {
        if let Some(turn) = self.turns.pop_front() {
            self.bytes -= turn.key.len() + turn.text.len();
        }
    }
    fn get(&mut self, key: &str) -> Option<String> {
        self.prune();
        let index = self.turns.iter().position(|turn| turn.key == key)?;
        let mut turn = self.turns.remove(index)?;
        turn.used = Instant::now();
        let text = turn.text.clone();
        self.turns.push_back(turn);
        Some(text)
    }
    fn insert(&mut self, key: String, text: String) {
        self.prune();
        if let Some(index) = self.turns.iter().position(|turn| turn.key == key) {
            let old = self.turns.remove(index).expect("found turn");
            self.bytes -= old.key.len() + old.text.len();
        }
        let size = key.len() + text.len();
        if size > MAX_BYTES {
            return;
        }
        while self.turns.len() >= MAX_TURNS || self.bytes + size > MAX_BYTES {
            self.pop();
        }
        self.bytes += size;
        self.turns.push_back(Turn {
            key,
            text,
            used: Instant::now(),
        });
    }
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

/// What a turn shows, which the next request repeats as it was: its text and
/// the ids of its calls. Names are left out, as namespaced tools are renamed
/// on the way to the caller.
fn signature(text: &str, calls: Vec<&str>) -> Value {
    json!([text, calls])
}

fn message_signature(message: &Value) -> Value {
    let calls = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|call| call["id"].as_str())
        .collect();
    signature(&text(&message["content"]), calls)
}

/// The conversation before each position. Instructions are left out, so a
/// changed system prompt does not part a session from its own turns.
fn prefixes(messages: &[Value]) -> Vec<String> {
    let mut running = Sha256::new();
    let mut prefixes = vec![format!("{:x}", running.clone().finalize())];
    for message in messages {
        let turn = match message["role"].as_str() {
            Some(role @ ("user" | "tool")) => Some(json!([
                role,
                text(&message["content"]),
                message["tool_call_id"]
            ])),
            Some("assistant") => Some(json!(["assistant", message_signature(message)])),
            _ => None,
        };
        if let Some(turn) = turn {
            running.update(digest(&turn).as_bytes());
        }
        prefixes.push(format!("{:x}", running.clone().finalize()));
    }
    prefixes
}

/// A whole Chat reply sent its reasoning as `reasoning_content`.
pub fn sent_reasoning_content(reply: &Value) -> bool {
    reply["choices"][0]["message"]["reasoning_content"]
        .as_str()
        .is_some_and(|text| !text.is_empty())
}

pub struct Context {
    scope: String,
    prefixes: Vec<String>,
}

impl Context {
    /// `body` is the Chat request as translated, before anything is restored.
    pub fn new(scope: &[&str], body: &Value) -> Option<Self> {
        let messages = body["messages"].as_array()?;
        Some(Self {
            scope: digest(&json!(scope)),
            prefixes: prefixes(messages),
        })
    }

    fn key(&self, position: usize, signature: &Value) -> String {
        digest(&json!([self.scope, self.prefixes[position], signature]))
    }

    /// Puts the remembered reasoning back on each assistant turn it came with;
    /// says whether any was.
    pub fn restore(&self, body: &mut Value) -> bool {
        let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
            return false;
        };
        let pending: Vec<_> = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message["role"] == "assistant" && message["reasoning_content"].is_null()
            })
            .map(|(index, message)| (index, self.key(index, &message_signature(message))))
            .collect();
        if pending.is_empty() {
            return false;
        }
        let restored: Vec<_> = {
            let Ok(mut store) = store().lock() else {
                return false;
            };
            pending
                .into_iter()
                .filter_map(|(index, key)| Some((index, store.get(&key)?)))
                .collect()
        };
        for (index, text) in &restored {
            messages[*index]["reasoning_content"] = json!(text);
        }
        !restored.is_empty()
    }

    /// Keeps the reasoning of a reply, read from its Responses form; the
    /// caller checks that the upstream sent it as `reasoning_content`.
    pub fn remember(&self, response: &Value) {
        if matches!(response["status"].as_str(), Some("failed" | "cancelled")) {
            return;
        }
        let Some(output) = response["output"].as_array() else {
            return;
        };
        let reasoning: String = output
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .flat_map(|item| item["summary"].as_array().into_iter().flatten())
            .filter_map(|part| part["text"].as_str())
            .collect();
        if reasoning.trim().is_empty() {
            return;
        }
        let shown: String = output
            .iter()
            .filter(|item| item["type"] == "message")
            .flat_map(|item| item["content"].as_array().into_iter().flatten())
            .filter_map(|part| part["text"].as_str())
            .collect();
        let calls = output
            .iter()
            .filter(|item| item["type"] == "function_call")
            .filter_map(|item| item["call_id"].as_str())
            .collect();
        let key = self.key(self.prefixes.len() - 1, &signature(&shown, calls));
        if let Ok(mut store) = store().lock() {
            store.insert(key, reasoning);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(messages: Value) -> Value {
        json!({"model":"m","messages":messages,"tools":[{"type":"function","function":{"name":"read"}}]})
    }

    fn reply(reasoning: &str, text: &str, call: Option<&str>) -> Value {
        let mut output =
            vec![json!({"type":"reasoning","summary":[{"type":"summary_text","text":reasoning}]})];
        if !text.is_empty() {
            output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}));
        }
        if let Some(call) = call {
            output.push(
                json!({"type":"function_call","call_id":call,"name":"ns__read","arguments":"{}"}),
            );
        }
        json!({"status":"completed","output":output})
    }

    #[test]
    fn each_assistant_turn_gets_its_own_reasoning_back() {
        let scope = [
            "store",
            "deepseek",
            "https://api.deepseek.com/chat/completions",
            "deepseek-v4",
        ];
        let user = format!("plan the fix {}", uuid::Uuid::new_v4());
        let first =
            request(json!([{"role":"system","content":"rules"},{"role":"user","content":user}]));
        Context::new(&scope, &first).unwrap().remember(&reply(
            "look at the parser",
            "",
            Some("call_1"),
        ));

        let second = request(
            json!([{"role":"system","content":"rules"},{"role":"user","content":user},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"file body"}]),
        );
        let context = Context::new(&scope, &second).unwrap();
        context.remember(&reply("the parser is fine", "Done.", None));
        let mut sent = second.clone();
        context.restore(&mut sent);
        assert_eq!(
            sent["messages"][2]["reasoning_content"],
            "look at the parser"
        );

        // A changed system prompt keeps the session's turns; the final answer too.
        let mut third = request(
            json!([{"role":"system","content":"new rules"},{"role":"user","content":user},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"file body"},
            {"role":"assistant","content":"Done."},{"role":"user","content":"thanks"}]),
        );
        Context::new(&scope, &third).unwrap().restore(&mut third);
        assert_eq!(
            third["messages"][2]["reasoning_content"],
            "look at the parser"
        );
        assert_eq!(
            third["messages"][4]["reasoning_content"],
            "the parser is fine"
        );
    }

    #[test]
    fn another_route_or_conversation_gets_nothing() {
        let scope = ["store", "deepseek", "url", "deepseek-v4"];
        let user = format!("question {}", uuid::Uuid::new_v4());
        let first = request(json!([{"role":"user","content":user}]));
        Context::new(&scope, &first)
            .unwrap()
            .remember(&reply("private steps", "Answer", None));
        let later = |scope: &[&str], user: &str| {
            let mut body = request(
                json!([{"role":"user","content":user},{"role":"assistant","content":"Answer"},{"role":"user","content":"more"}]),
            );
            Context::new(scope, &body).unwrap().restore(&mut body);
            body["messages"][1].get("reasoning_content").cloned()
        };
        assert_eq!(later(&scope, &user), Some(json!("private steps")));
        assert_eq!(
            later(&["store", "groq", "url", "deepseek-v4"], &user),
            None,
            "another provider"
        );
        assert_eq!(later(&scope, "another conversation"), None);
    }

    #[test]
    fn failed_replies_and_empty_reasoning_are_not_kept() {
        let scope = ["store", "p", "url", "m"];
        let user = format!("q {}", uuid::Uuid::new_v4());
        let first = request(json!([{"role":"user","content":user}]));
        let mut failed = reply("half a thought", "Partial", None);
        failed["status"] = json!("failed");
        Context::new(&scope, &first).unwrap().remember(&failed);
        Context::new(&scope, &first)
            .unwrap()
            .remember(&reply("  ", "Other", None));
        for text in ["Partial", "Other"] {
            let mut body = request(
                json!([{"role":"user","content":user},{"role":"assistant","content":text}]),
            );
            Context::new(&scope, &body).unwrap().restore(&mut body);
            assert!(
                body["messages"][1].get("reasoning_content").is_none(),
                "{text}"
            );
        }
    }
}
