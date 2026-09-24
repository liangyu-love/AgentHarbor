//! Recent Responses turns, kept so a Chat Completions upstream can continue a
//! conversation it has no state for.
//!
//! The Responses API addresses prior turns with `previous_response_id`, but Chat
//! Completions is stateless: the whole history must ride in `messages` every
//! time. When Codex follows up a tool call it sends only the previous id plus the
//! tool results, so the translator looks the earlier turns up here and rebuilds
//! the message list. Without this the upstream receives tool results that answer
//! a call it never saw, and the conversation cannot continue.

use crate::core::router::ModelProtocol;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Responses from the recent past are enough for any realistic agent loop; the
/// bound keeps an idle app from growing without limit.
const MAX_RESPONSES: usize = 512;

/// One completed turn: the messages that produced it and the tool calls it made.
#[derive(Clone)]
struct Turn {
    response_id: String,
    /// The conversation leading up to and including this turn's request.
    messages: Vec<Value>,
    /// Tool calls this turn's assistant message made, in Responses item form.
    tool_calls: Vec<Value>,
}

#[derive(Default)]
struct History {
    turns: VecDeque<Turn>,
}

static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();

fn history() -> &'static Mutex<History> {
    HISTORY.get_or_init(|| Mutex::new(History::default()))
}

/// Record the messages used for a turn and the tool calls it returned.
///
/// Called after a translated turn completes so the next request can be
/// reconstructed from it.
pub fn remember(response_id: &str, messages: Vec<Value>, tool_calls: Vec<Value>) {
    let Ok(mut history) = history().lock() else {
        return;
    };
    // A repeated id supersedes the earlier entry rather than duplicating it.
    history.turns.retain(|turn| turn.response_id != response_id);
    history.turns.push_back(Turn {
        response_id: response_id.to_owned(),
        messages,
        tool_calls,
    });
    while history.turns.len() > MAX_RESPONSES {
        history.turns.pop_front();
    }
}

/// Rebuild the message list a follow-up request implies.
///
/// Returns `None` when the referenced turn is unknown — after a restart, or once
/// it has aged out — which leaves the caller to forward what it was given rather
/// than inventing history.
pub fn replay(previous_response_id: &str) -> Option<Vec<Value>> {
    replay_for(previous_response_id, ModelProtocol::ChatCompletions)
}

pub fn replay_for(previous_response_id: &str, target: ModelProtocol) -> Option<Vec<Value>> {
    let history = history().lock().ok()?;
    let turn = history
        .turns
        .iter()
        .find(|turn| turn.response_id == previous_response_id)?;
    let mut messages = turn.messages.clone();
    if !turn.tool_calls.is_empty() {
        // The assistant turn must restate the calls before their results, which
        // the follow-up request carries separately.
        if target == ModelProtocol::AnthropicMessages {
            let blocks: Vec<Value> = turn
                .tool_calls
                .iter()
                .map(|call| {
                    let arguments = call["function"]["arguments"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        .unwrap_or_else(|| serde_json::json!({}));
                    serde_json::json!({
                        "type": "tool_use",
                        "id": call["id"],
                        "name": call["function"]["name"],
                        "input": arguments,
                    })
                })
                .collect();
            messages.push(serde_json::json!({ "role": "assistant", "content": blocks }));
        } else {
            messages.push(serde_json::json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": turn.tool_calls,
            }));
        }
    }
    Some(messages)
}

/// Drop all remembered turns. Used by tests and by an explicit user reset.
pub fn clear() {
    if let Ok(mut history) = history().lock() {
        history.turns.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_recorded_turn_is_replayed_with_its_tool_calls() {
        clear();
        remember(
            "resp_1",
            vec![json!({ "role": "user", "content": "hi" })],
            vec![
                json!({ "id": "call_1", "type": "function", "function": { "name": "f", "arguments": "{}" } }),
            ],
        );

        let replayed = replay("resp_1").expect("the turn must be found");
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0]["role"], "user");
        assert_eq!(replayed[1]["role"], "assistant");
        assert_eq!(replayed[1]["tool_calls"][0]["id"], "call_1");
    }

    #[test]
    fn an_unknown_response_id_is_reported_rather_than_faked() {
        clear();
        assert!(replay("resp_missing").is_none());
    }

    #[test]
    fn recording_the_same_id_twice_replaces_the_earlier_turn() {
        clear();
        remember(
            "resp_dup",
            vec![json!({ "role": "user", "content": "one" })],
            Vec::new(),
        );
        remember(
            "resp_dup",
            vec![json!({ "role": "user", "content": "two" })],
            Vec::new(),
        );

        let replayed = replay("resp_dup").unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0]["content"], "two");
    }

    #[test]
    fn the_cache_is_bounded() {
        clear();
        for index in 0..(MAX_RESPONSES + 10) {
            remember(
                &format!("resp_{index}"),
                vec![json!({ "role": "user", "content": index })],
                Vec::new(),
            );
        }
        // The oldest entries are evicted, the newest are kept.
        assert!(replay("resp_0").is_none());
        assert!(replay(&format!("resp_{}", MAX_RESPONSES + 9)).is_some());
    }
}
