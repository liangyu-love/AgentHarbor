//! Bounded, protocol-independent history for translated Responses requests.
//! Request instructions are deliberately excluded: Responses does not carry
//! them into a continuation. Explicit system/developer input items do persist.

use super::canonical::{OutputItem, TerminalState};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_RESPONSES: usize = 512;
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;

struct Turn {
    response_id: String,
    // Store serialized input/output items so the byte budget bounds storage.
    items: Vec<u8>,
}

#[derive(Default)]
struct History {
    turns: VecDeque<Turn>,
    bytes: usize,
}

impl History {
    fn remember(&mut self, response: &Value, mut input: Vec<Value>) -> bool {
        if !TerminalState::from_canonical_response(response).is_success() {
            return false;
        }
        let Some(id) = response
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return false;
        };
        let Some(output) = response.get("output").and_then(Value::as_array) else {
            return false;
        };
        if output
            .iter()
            .any(|item| !OutputItem::from_legacy(item).replayable())
        {
            return false;
        }
        input.extend(output.iter().cloned());
        let Ok(items) = serde_json::to_vec(&input) else {
            return false;
        };
        let size = items.len() + id.len();
        // Remove an older value even if its replacement cannot fit.
        if let Some(index) = self.turns.iter().position(|turn| turn.response_id == id) {
            let previous = self.turns.remove(index).unwrap();
            self.bytes -= previous.items.len() + previous.response_id.len();
        }
        if size > MAX_HISTORY_BYTES {
            return false;
        }
        while self.turns.len() >= MAX_RESPONSES || self.bytes + size > MAX_HISTORY_BYTES {
            let Some(oldest) = self.turns.pop_front() else {
                break;
            };
            self.bytes -= oldest.items.len() + oldest.response_id.len();
        }
        self.bytes += size;
        self.turns.push_back(Turn {
            response_id: id.to_owned(),
            items,
        });
        true
    }

    fn replay(&self, id: &str) -> Option<Vec<Value>> {
        let turn = self.turns.iter().find(|turn| turn.response_id == id)?;
        serde_json::from_slice(&turn.items).ok()
    }
}

static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();

fn history() -> &'static Mutex<History> {
    HISTORY.get_or_init(|| Mutex::new(History::default()))
}

/// Cache only a complete successful response, including ordinary assistant text.
pub fn remember(response: &Value, input: Vec<Value>) {
    if let Ok(mut history) = history().lock() {
        history.remember(response, input);
    }
}

/// Expand a continuation before converting its items to an upstream protocol.
/// An expired/missing response is an error, never a context-free continuation.
pub(super) fn input_for_request(value: &Value) -> Result<Vec<Value>, String> {
    let mut input = match value.get("previous_response_id") {
        Some(Value::String(id)) if !id.is_empty() => history()
            .lock()
            .map_err(|_| "protocol.errors.historyUnavailable")?
            .replay(id)
            .ok_or("protocol.errors.unknownPreviousResponse")?,
        None | Some(Value::Null) => Vec::new(),
        _ => return Err("protocol.errors.invalidRequest".into()),
    };
    match value.get("input") {
        Some(Value::String(text)) => input.push(json!({"role":"user", "content":text})),
        Some(Value::Array(items)) => input.extend(items.iter().cloned()),
        None => {}
        _ => return Err("protocol.errors.invalidRequest".into()),
    }
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(id: &str, output: Vec<Value>) -> Value {
        json!({"id":id, "status":"completed", "output":output})
    }

    #[test]
    fn ordinary_text_and_tools_are_both_replayed() {
        let mut cache = History::default();
        let output = vec![
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Checking"}]}),
            json!({"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{}"}),
        ];
        assert!(cache.remember(
            &response("r1", output.clone()),
            vec![json!({"role":"user","content":"hi"})]
        ));
        let items = cache.replay("r1").unwrap();
        assert_eq!(&items[1..], output.as_slice());
    }

    #[test]
    fn failed_incomplete_and_invalid_tools_are_not_cached() {
        let mut cache = History::default();
        for status in ["failed", "incomplete", "in_progress"] {
            assert!(!cache.remember(&json!({"id":status,"status":status,"output":[]}), vec![]));
        }
        assert!(!cache.remember(
            &response(
                "bad",
                vec![json!({"type":"function_call","call_id":"c","name":"f","arguments":"{"})]
            ),
            vec![]
        ));
        assert!(cache.turns.is_empty());
    }

    #[test]
    fn duplicate_ids_replace_prior_history_and_account_for_bytes() {
        let mut cache = History::default();
        cache.remember(&response("r", vec![]), vec![json!("one")]);
        cache.remember(&response("r", vec![]), vec![json!("two")]);
        assert_eq!(cache.replay("r").unwrap(), vec![json!("two")]);
        assert_eq!(cache.turns.len(), 1);
        assert_eq!(cache.bytes, cache.turns[0].items.len() + 1);
    }

    #[test]
    fn cache_is_bounded_by_count_and_serialized_bytes() {
        let mut cache = History::default();
        for index in 0..MAX_RESPONSES + 1 {
            cache.remember(&response(&index.to_string(), vec![]), vec![]);
        }
        assert!(cache.replay("0").is_none());
        assert_eq!(cache.turns.len(), MAX_RESPONSES);
        let large = json!("x".repeat(MAX_HISTORY_BYTES / 2));
        cache.remember(&response("large1", vec![]), vec![large.clone()]);
        cache.remember(&response("large2", vec![]), vec![large]);
        assert!(cache.replay("large1").is_none());
        assert!(cache.replay("large2").is_some());
        assert!(cache.bytes <= MAX_HISTORY_BYTES);
        assert!(!cache.remember(
            &response("oversized", vec![]),
            vec![json!("x".repeat(MAX_HISTORY_BYTES))]
        ));
    }
}
