//! Writes a changed client file without losing its comments or layout: only
//! the values that differ are edited in place. A file the client's reader
//! would not accept, a change the editor cannot express, or a result that
//! does not read back as the intended value returns `None`, and the caller
//! writes normalized JSON as before.

use jsonc_parser::{
    ast::Comment,
    cst::{CstInputValue, CstObject, CstRootNode},
    parse_to_ast, CollectOptions, CommentCollectionStrategy, ParseOptions,
};
use serde_json::{Map, Value};

/// The JSON-with-comments a client's reader takes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Syntax {
    /// OpenCode reads with jsonc-parser: comments and trailing commas.
    OpenCode,
    /// Pi strips `//` comments and trailing commas, then calls `JSON.parse`,
    /// so a block comment breaks it.
    Pi,
}

const OPTIONS: ParseOptions = ParseOptions {
    allow_comments: true,
    allow_loose_object_property_names: false,
    allow_trailing_commas: true,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
    allow_bare_decimal_point_numbers: false,
    allow_non_finite_numbers: false,
    allow_extended_string_escapes: false,
};

/// `text` as the client reads it, or `None` when the client would not.
pub(super) fn read(text: &str, syntax: Syntax) -> Option<Value> {
    let collect = CollectOptions {
        comments: CommentCollectionStrategy::Separate,
        tokens: false,
    };
    let parsed = parse_to_ast(text, &collect, &OPTIONS).ok()?;
    let block_comment = parsed
        .comments
        .iter()
        .flat_map(|comments| comments.values())
        .flat_map(|comments| comments.iter())
        .any(|comment| matches!(comment, Comment::Block(_)));
    if syntax == Syntax::Pi && block_comment {
        return None;
    }
    parsed.value.map(Value::from)
}

/// `base` (the text the file held) with its object changed to `next`,
/// keeping everything the edit does not touch. The edit only adds strict JSON
/// values, and the result must read back as `next` under `syntax`.
pub(super) fn patch(base: &str, syntax: Syntax, next: &Value) -> Option<String> {
    let base = base.trim_start_matches('\u{feff}');
    let from = read(base, syntax)?;
    let root = CstRootNode::parse(base, &OPTIONS).ok()?;
    patch_object(
        &root.object_value_or_set(),
        from.as_object()?,
        next.as_object()?,
    );
    let text = root.to_string();
    (read(&text, syntax)? == *next).then_some(text)
}

fn patch_object(object: &CstObject, from: &Map<String, Value>, next: &Map<String, Value>) {
    for key in from.keys().filter(|key| !next.contains_key(key.as_str())) {
        if let Some(property) = object.get(key) {
            property.remove();
        }
    }
    for (key, value) in next {
        match (from.get(key), value) {
            (Some(before), _) if before == value => {}
            (Some(Value::Object(before)), Value::Object(after)) => {
                match child_object(object, key) {
                    Some(child) => patch_object(&child, before, after),
                    None => set(object, key, value),
                }
            }
            _ => set(object, key, value),
        }
    }
}

fn child_object(object: &CstObject, key: &str) -> Option<CstObject> {
    object.get(key)?.value()?.as_object()
}

fn set(object: &CstObject, key: &str, value: &Value) {
    match object.get(key) {
        Some(property) => property.set_value(input(value)),
        None => {
            object.append(key, input(value));
        }
    }
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), input(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edits_only_the_changed_values_and_keeps_comments() {
        let base = "{\n  // the user's note\n  \"theme\": \"dark\",\n  \"keep\": { /* kept */ \"a\": 1 },\n}\n";
        let next = json!({"theme": "light", "keep": {"a": 1, "b": true}, "added": [1, "x"]});
        let text = patch(base, Syntax::OpenCode, &next).unwrap();
        assert!(text.contains("// the user's note"));
        assert!(text.contains("/* kept */"));
        assert_eq!(read(&text, Syntax::OpenCode).unwrap(), next);
    }

    #[test]
    fn removed_keys_go_and_their_siblings_stay() {
        let base = "{\n  // header\n  \"provider\": {\"x\": 1},\n  \"theme\": \"dark\",\n}\n";
        let next = json!({"theme": "dark"});
        let text = patch(base, Syntax::Pi, &next).unwrap();
        assert!(text.contains("// header"));
        assert_eq!(read(&text, Syntax::Pi).unwrap(), next);
    }

    #[test]
    fn a_byte_order_mark_is_dropped_and_the_comments_stay() {
        let base = "\u{feff}{\n  // note\n  \"theme\": \"dark\"\n}\n";
        let text = patch(base, Syntax::Pi, &json!({"theme": "light"})).unwrap();
        assert!(text.starts_with('{'));
        assert!(text.contains("// note"));
    }

    #[test]
    fn syntax_the_reader_does_not_take_is_refused() {
        // Pi strips line comments and trailing commas, nothing else.
        assert!(read("{ // note\n \"theme\": \"dark\", }\n", Syntax::Pi).is_some());
        assert!(read("{ /* note */ \"theme\": \"dark\" }\n", Syntax::Pi).is_none());
        assert!(read("{ /* note */ \"theme\": \"dark\" }\n", Syntax::OpenCode).is_some());
        assert!(read("{ theme: 'dark' }\n", Syntax::OpenCode).is_none());
        assert!(patch("{ /* note */ \"a\": 1 }", Syntax::Pi, &json!({"a": 2})).is_none());
    }
}
