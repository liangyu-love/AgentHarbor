//! Prompt-cache marks for a request rebuilt as Anthropic Messages.
//!
//! Responses and Chat upstreams cache a repeated prefix on their own, but
//! Messages reads nothing from cache unless the request carries
//! `cache_control`, so a translated turn would pay full input price every
//! time. Two block marks, the shape Claude Code itself sends (block marks are
//! what every Messages relay accepts; the top-level form is newer and refused
//! by some): the end of the system prompt, which caches the tools and system
//! every turn shares, and the last block of the conversation, which the next
//! turn reads back within the 20-block lookback. A mark changes no rendered
//! text, so moving it forward each turn keeps the earlier entry readable.
use serde_json::{json, Value};

fn blocks(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn marked(block: &Value) -> bool {
    block
        .get("cache_control")
        .is_some_and(|mark| !mark.is_null())
}

/// Marks the caller placed itself; at most four are allowed, so they stay
/// as they are. Tool schemas are not looked into: a property there may be
/// named `cache_control`.
fn has_mark(body: &Value) -> bool {
    blocks(&body["system"]).any(marked)
        || blocks(&body["tools"]).any(marked)
        || blocks(&body["messages"])
            .flat_map(|message| blocks(&message["content"]))
            .any(|block| marked(block) || blocks(&block["content"]).any(marked))
}

/// Blocks Messages accepts a mark on: thinking and empty text take none.
fn markable(block: &Value) -> bool {
    match block["type"].as_str() {
        Some("text") => block["text"].as_str().is_some_and(|text| !text.is_empty()),
        Some("image" | "document" | "tool_use" | "tool_result") => true,
        _ => false,
    }
}

fn mark_last(blocks: Option<&mut Vec<Value>>, eligible: fn(&Value) -> bool) -> bool {
    let Some(block) = blocks
        .and_then(|blocks| blocks.iter_mut().rev().find(|block| eligible(block)))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    block.insert("cache_control".into(), json!({"type": "ephemeral"}));
    true
}

/// A string becomes the one text block it stands for, which renders the same.
fn as_blocks(content: &mut Value) {
    if let Some(text) = content
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
    {
        *content = json!([{"type": "text", "text": text}]);
    }
}

fn mark_system(body: &mut Value) -> bool {
    let Some(system) = body.get_mut("system") else {
        return false;
    };
    as_blocks(system);
    mark_last(system.as_array_mut(), markable)
}

fn mark_conversation(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return false;
    };
    for message in messages.iter_mut().rev() {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        as_blocks(content);
        if mark_last(content.as_array_mut(), markable) {
            return true;
        }
    }
    false
}

pub(super) fn mark(body: &mut Value) {
    if has_mark(body) {
        return;
    }
    // Without a system prompt the last tool closes the shared prefix.
    if !mark_system(body) {
        mark_last(
            body.get_mut("tools").and_then(Value::as_array_mut),
            Value::is_object,
        );
    }
    mark_conversation(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(body: &Value) -> usize {
        body.to_string().matches("\"cache_control\"").count()
    }

    #[test]
    fn system_and_last_block_are_marked() {
        let mut body = json!({"model":"m","max_tokens":10,"system":"stable rules",
            "tools":[{"name":"read","input_schema":{"type":"object"}}],
            "messages":[{"role":"user","content":"hi"},
                {"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"read","input":{}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"body"}]}]});
        mark(&mut body);
        assert_eq!(
            body["system"],
            json!([{"type":"text","text":"stable rules","cache_control":{"type":"ephemeral"}}])
        );
        assert_eq!(
            body["messages"][2]["content"][0]["cache_control"],
            json!({"type":"ephemeral"})
        );
        assert_eq!(
            body["messages"][0]["content"], "hi",
            "earlier turns keep their form"
        );
        assert!(
            body["tools"][0].get("cache_control").is_none(),
            "the system mark already covers the tools"
        );
        assert_eq!(marks(&body), 2);
    }

    #[test]
    fn string_turns_become_one_text_block_and_unmarkable_blocks_are_skipped() {
        let mut body = json!({"model":"m","max_tokens":10,
            "tools":[{"name":"a","input_schema":{}},{"name":"b","input_schema":{}}],
            "messages":[{"role":"user","content":"question"},
                {"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"s"},{"type":"text","text":""}]}]});
        mark(&mut body);
        assert_eq!(
            body["tools"][1]["cache_control"],
            json!({"type":"ephemeral"}),
            "no system: the last tool closes the prefix"
        );
        assert!(body["messages"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|block| block.get("cache_control").is_none()));
        assert_eq!(
            body["messages"][0]["content"],
            json!([{"type":"text","text":"question","cache_control":{"type":"ephemeral"}}])
        );
    }

    #[test]
    fn caller_marks_are_kept_and_schema_keys_are_not_mistaken_for_marks() {
        let mut own = json!({"model":"m","max_tokens":10,"system":"s",
            "messages":[{"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral","ttl":"1h"}}]},
                {"role":"user","content":"b"}]});
        let before = own.clone();
        mark(&mut own);
        assert_eq!(own, before);

        let mut schema = json!({"model":"m","max_tokens":10,"system":"s",
            "tools":[{"name":"t","input_schema":{"type":"object","properties":{"cache_control":{"type":"string"}}}}],
            "messages":[{"role":"user","content":"b"}]});
        mark(&mut schema);
        assert_eq!(
            schema["messages"][0]["content"][0]["cache_control"],
            json!({"type":"ephemeral"})
        );
    }

    #[test]
    fn an_empty_request_is_left_alone() {
        let mut body = json!({"model":"m","max_tokens":10,"system":"","messages":[{"role":"user","content":""}]});
        let before = body.clone();
        mark(&mut body);
        assert_eq!(body, before);
    }
}
