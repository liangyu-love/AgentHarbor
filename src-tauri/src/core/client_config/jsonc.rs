//! Writes a changed client file without losing its comments or layout: only
//! the values that differ are edited in place. A file the client's reader
//! would not accept as JSON-with-comments, a change the editor cannot express,
//! or a result that does not read back as the intended value returns `None`,
//! and the caller writes normalized JSON as before.

use jsonc_parser::{
    cst::{CstInputValue, CstObject, CstRootNode},
    ParseOptions,
};
use serde_json::{Map, Value};

/// JSON with comments, as the client reads it. Trailing commas are taken only
/// where the client's reader takes them: OpenCode does, Pi's `JSON.parse` does
/// not, so Pi's files stay strict.
pub(super) fn options(trailing_commas: bool) -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: trailing_commas,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

/// `base` (the text the file held) with its object changed from `from` to
/// `next`, keeping everything the edit does not touch. `base` must already
/// parse under `options`, so the output only adds strict JSON values.
pub(super) fn patch(
    base: &str,
    options: &ParseOptions,
    from: &Value,
    next: &Value,
) -> Option<String> {
    let root = CstRootNode::parse(base, options).ok()?;
    patch_object(
        &root.object_value_or_set(),
        from.as_object()?,
        next.as_object()?,
    );
    let text = root.to_string();
    (json5::from_str::<Value>(&text).ok()? == *next).then_some(text)
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
        let options = options(true);
        let from = json5::from_str::<Value>(base).unwrap();
        let next = json!({"theme": "light", "keep": {"a": 1, "b": true}, "added": [1, "x"]});
        let text = patch(base, &options, &from, &next).unwrap();
        assert!(text.contains("// the user's note"));
        assert!(text.contains("/* kept */"));
        assert_eq!(json5::from_str::<Value>(&text).unwrap(), next);
    }

    #[test]
    fn removed_keys_go_and_their_siblings_stay() {
        let base = "{\n  // header\n  \"provider\": {\"x\": 1},\n  \"theme\": \"dark\",\n}\n";
        let options = options(true);
        let from = json5::from_str::<Value>(base).unwrap();
        let next = json!({"theme": "dark"});
        let text = patch(base, &options, &from, &next).unwrap();
        assert!(text.contains("// header"));
        assert_eq!(json5::from_str::<Value>(&text).unwrap(), next);
    }

    #[test]
    fn syntax_the_reader_does_not_take_is_refused() {
        // Pi's reader takes comments and nothing else beyond strict JSON.
        assert!(
            CstRootNode::parse("{ // note\n \"theme\": \"dark\", }\n", &options(false)).is_err()
        );
        assert!(CstRootNode::parse("{ theme: 'dark' }\n", &options(false)).is_err());
        assert!(CstRootNode::parse("{ // note\n \"theme\": \"dark\", }\n", &options(true)).is_ok());
    }
}
