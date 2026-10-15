//! Lossless edits to DeepSeek Harness's YAML files: a profile's
//! `cordis.patch.yml` (a list of rows addressed by `id`) and the credential
//! store (`version`, `refs`, `records`). yaml-edit keeps comments, `!!js`
//! expressions and layout; each edit is then read back by an independent
//! parser and is refused unless it reads exactly as intended, so a layout this
//! code cannot edit is reported instead of written broken.
use serde_json::{json, Value};
use std::str::FromStr;
use yaml_edit::{Document, Mapping, YamlFile};

const SHAPE: &str = "clients.errors.harnessPatchShape";
const CREDENTIALS_SHAPE: &str = "clients.errors.harnessCredentialsShape";
const LAYOUT: &str = "clients.errors.harnessYamlLayout";

/// YAML text as plain data. Absent, empty and comment-only text is `Null`.
pub(super) fn read(text: Option<&str>) -> Result<Value, String> {
    let Some(text) = text else {
        return Ok(Value::Null);
    };
    let value: serde_yaml::Value = serde_yaml::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| "clients.errors.invalidConfig")?;
    Ok(plain(value))
}

fn plain(value: serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(value) => json!(value),
        serde_yaml::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                json!(value)
            } else if let Some(value) = number.as_u64() {
                json!(value)
            } else {
                number
                    .as_f64()
                    .map(|value| json!(value))
                    .unwrap_or(Value::Null)
            }
        }
        serde_yaml::Value::String(value) => json!(value),
        serde_yaml::Value::Sequence(items) => Value::Array(items.into_iter().map(plain).collect()),
        serde_yaml::Value::Mapping(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| {
                    let key = match key {
                        serde_yaml::Value::String(key) => key,
                        other => serde_yaml::to_string(&other)
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                    };
                    (key, plain(value))
                })
                .collect(),
        ),
        // `!!js` expressions and other tags only need to compare equal to
        // themselves; they are never interpreted here.
        serde_yaml::Value::Tagged(tagged) => {
            json!({"!tag": tagged.tag.to_string(), "value": plain(tagged.value)})
        }
    }
}

// ---- the profile patch -------------------------------------------------------

fn rows(document: &Value) -> Result<&[Value], String> {
    match document {
        Value::Null => Ok(&[]),
        Value::Array(rows) => Ok(rows),
        _ => Err(SHAPE.into()),
    }
}

/// The position of the row with this `id`. Two rows with one id would leave
/// it unclear which one the edit belongs to, so they are refused.
pub(super) fn row_index(document: &Value, id: &str) -> Result<Option<usize>, String> {
    let mut found = None;
    for (index, row) in rows(document)?.iter().enumerate() {
        if row.get("id").and_then(Value::as_str) == Some(id) {
            if found.is_some() {
                return Err("clients.errors.harnessDuplicateRow".into());
            }
            found = Some(index);
        }
    }
    Ok(found)
}

pub(super) fn row<'a>(document: &'a Value, id: &str) -> Option<&'a Value> {
    let index = row_index(document, id).ok().flatten()?;
    rows(document).ok()?.get(index)
}

/// A value inside a row's `config`.
pub(super) fn patch_value<'a>(document: &'a Value, id: &str, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(row(document, id)?.get("config")?, |value, key| {
            value.get(key)
        })
}

fn nest(path: &[String], value: Value) -> Value {
    path.iter()
        .rev()
        .fold(value, |inner, key| json!({ key: inner }))
}

/// `path` set to `value` (or removed) in the plain document, creating the row
/// and the containers on the way.
fn intended_patch(
    document: &Value,
    id: &str,
    path: &[String],
    value: Option<&Value>,
) -> Result<Value, String> {
    let mut expected = if document.is_null() {
        json!([])
    } else {
        document.clone()
    };
    let index = row_index(&expected, id)?;
    let rows = expected.as_array_mut().ok_or(SHAPE)?;
    let full: Vec<String> = std::iter::once("config".to_owned())
        .chain(path.iter().cloned())
        .collect();
    match (index, value) {
        (None, None) => {}
        (None, Some(value)) => rows.push(json!({"id": id, "config": nest(path, value.clone())})),
        (Some(index), value) => set_in(&mut rows[index], &full, value)?,
    }
    Ok(expected)
}

fn set_in(target: &mut Value, path: &[String], value: Option<&Value>) -> Result<(), String> {
    let object = target.as_object_mut().ok_or(SHAPE)?;
    let (key, rest) = path.split_first().ok_or(SHAPE)?;
    if rest.is_empty() {
        match value {
            Some(value) => {
                object.insert(key.clone(), value.clone());
            }
            None => {
                object.remove(key);
            }
        }
        return Ok(());
    }
    match object.get_mut(key) {
        Some(child) => set_in(child, rest, value),
        None => {
            if let Some(value) = value {
                object.insert(key.clone(), nest(rest, value.clone()));
            }
            Ok(())
        }
    }
}

/// The patch text with `path` inside row `id`'s config set to `value`, or
/// removed when `value` is `None`. Unchanged text is returned as it is.
pub(super) fn set_patch(
    text: Option<&str>,
    id: &str,
    path: &[String],
    value: Option<&Value>,
) -> Result<Option<String>, String> {
    let document = read(text)?;
    let expected = intended_patch(&document, id, path, value)?;
    if expected == document || (document.is_null() && expected == json!([])) {
        return Ok(text.map(str::to_owned));
    }
    let index = row_index(&document, id)?;
    let source = text.unwrap_or("");
    let mut candidates = Vec::new();
    match (index, value) {
        (None, Some(value)) => {
            let entry = json!({"id": id, "config": nest(path, value.clone())});
            candidates.push(append_rows(source, &entry));
            candidates.extend(structural(source, |document| {
                document
                    .as_sequence()?
                    .push(flow_snippet(&entry)?.as_mapping()?);
                Some(())
            }));
        }
        (Some(index), value) => {
            let full: Vec<String> = std::iter::once("config".to_owned())
                .chain(path.iter().cloned())
                .collect();
            for flow in [false, true] {
                candidates.extend(structural(source, |document| {
                    let node = document.as_sequence()?.get(index)?;
                    edit_mapping(node.as_mapping()?, &full, value, flow)
                }));
            }
        }
        // Nothing to remove: answered above.
        (None, None) => {}
    }
    first_reading(candidates, &expected)
}

/// The patch text without row `id`. A patch must never end up empty — Harness
/// refuses to start on one — so an emptied list is written as `[]`.
pub(super) fn remove_row(text: Option<&str>, id: &str) -> Result<Option<String>, String> {
    let document = read(text)?;
    let Some(index) = row_index(&document, id)? else {
        return Ok(text.map(str::to_owned));
    };
    let mut expected = document.clone();
    expected.as_array_mut().ok_or(SHAPE)?.remove(index);
    let candidates = structural(text.unwrap_or(""), |document| {
        document.as_sequence()?.remove(index)?;
        Some(())
    })
    .into_iter()
    .map(|text| {
        if read(Some(&text)).is_ok_and(|value| value.is_null()) {
            with_line(&text, "[]")
        } else {
            text
        }
    })
    .collect();
    first_reading(candidates, &expected)
}

/// New rows as block YAML after the existing ones. A list that holds nothing
/// but `[]` gives that token up for the rows.
fn append_rows(source: &str, entry: &Value) -> String {
    let rows = block(&json!([entry]), 0);
    let content: Vec<&str> = source
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .collect();
    if content.len() == 1 && content[0].trim() == "[]" {
        let kept: String = source
            .lines()
            .filter(|line| line.trim() != "[]")
            .map(|line| format!("{line}\n"))
            .collect();
        return format!("{kept}{rows}");
    }
    with_line(source, rows.trim_end())
}

fn with_line(source: &str, line: &str) -> String {
    let mut text = source.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    text
}

/// Walks `path` from `mapping`, creating what is missing at the first absent
/// key, and sets or removes the last key.
fn edit_mapping(
    mapping: &Mapping,
    path: &[String],
    value: Option<&Value>,
    flow: bool,
) -> Option<()> {
    let (key, rest) = path.split_first()?;
    if rest.is_empty() {
        match value {
            Some(value) => set_value(mapping, key, value, flow),
            None => {
                mapping.remove(key.as_str());
                Some(())
            }
        }
    } else if let Some(child) = mapping.get_mapping(key.as_str()) {
        edit_mapping(&child, rest, value, flow)
    } else if mapping.contains_key(key.as_str()) {
        None
    } else {
        match value {
            Some(value) => set_value(mapping, key, &nest(rest, value.clone()), flow),
            None => Some(()),
        }
    }
}

fn set_value(mapping: &Mapping, key: &str, value: &Value, flow: bool) -> Option<()> {
    match value {
        Value::String(text) => mapping.set(key, text.as_str()),
        Value::Bool(value) => mapping.set(key, *value),
        Value::Number(number) if number.is_i64() => mapping.set(key, number.as_i64()?),
        Value::Number(number) if number.is_u64() => mapping.set(key, number.as_u64()?),
        Value::Number(number) => mapping.set(key, number.as_f64()?),
        Value::Object(_) | Value::Array(_) => {
            let snippet = if flow || mapping.is_flow_style() {
                flow_snippet(value)?
            } else {
                block_snippet(value)?
            };
            if let Some(child) = snippet.as_mapping() {
                mapping.set(key, child);
            } else {
                mapping.set(key, snippet.as_sequence()?);
            }
        }
        Value::Null => return None,
    }
    Some(())
}

fn flow_snippet(value: &Value) -> Option<Document> {
    YamlFile::from_str(&serde_json::to_string(value).ok()?)
        .ok()?
        .document()
}

fn block_snippet(value: &Value) -> Option<Document> {
    YamlFile::from_str(&block(value, 0)).ok()?.document()
}

/// The edit applied to a fresh parse of `source`, as text.
fn structural(source: &str, edit: impl FnOnce(&Document) -> Option<()>) -> Option<String> {
    let file = YamlFile::from_str(source).ok()?;
    let document = file.document()?;
    edit(&document)?;
    Some(file.to_string())
}

fn first_reading(candidates: Vec<String>, expected: &Value) -> Result<Option<String>, String> {
    candidates
        .into_iter()
        .find(|text| read(Some(text)).is_ok_and(|value| &value == expected))
        .map(Some)
        .ok_or_else(|| LAYOUT.into())
}

// ---- the credential store ------------------------------------------------------

/// A stored key, by its reference name.
pub(super) fn credential<'a>(document: &'a Value, name: &str) -> Option<&'a Value> {
    document.get("refs")?.get(name)
}

/// The credential file with `refs.<name>` set to `value`, or removed.
pub(super) fn set_credential(
    text: Option<&str>,
    name: &str,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let document = read(text)?;
    if !document.is_null() && !document.is_object() {
        return Err(CREDENTIALS_SHAPE.into());
    }
    let mut expected = if document.is_null() {
        json!({"version": 1})
    } else {
        document.clone()
    };
    let object = expected.as_object_mut().ok_or(CREDENTIALS_SHAPE)?;
    let refs_present = object.contains_key("refs");
    match (object.get_mut("refs"), value) {
        (Some(Value::Object(refs)), Some(value)) => {
            refs.insert(name.to_owned(), json!(value));
        }
        (Some(Value::Object(refs)), None) => {
            refs.remove(name);
        }
        (Some(_), _) => return Err(CREDENTIALS_SHAPE.into()),
        (None, Some(value)) => {
            object.insert("refs".into(), json!({ name: value }));
        }
        (None, None) => return Ok(text.map(str::to_owned)),
    }
    if expected == document {
        return Ok(text.map(str::to_owned));
    }
    let source = text.unwrap_or("");
    let mut candidates = Vec::new();
    match (document.is_null(), refs_present, value) {
        (true, _, Some(value)) => {
            candidates.push(with_line(
                source,
                block(&json!({"version": 1, "refs": { name: value }}), 0).trim_end(),
            ));
        }
        (false, true, value) => candidates.extend(structural(source, |document| {
            let refs = document.as_mapping()?.get_mapping("refs")?;
            match value {
                Some(value) => refs.set(name, value),
                None => {
                    refs.remove(name);
                }
            }
            Some(())
        })),
        (false, false, Some(value)) => {
            let refs = json!({ name: value });
            candidates.push(with_line(
                source,
                block(&json!({"refs": refs}), 0).trim_end(),
            ));
            candidates.extend(structural(source, |document| {
                document
                    .as_mapping()?
                    .set("refs", flow_snippet(&refs)?.as_mapping()?);
                Some(())
            }));
        }
        _ => {}
    }
    first_reading(candidates, &expected)
}

/// The credential file without its top-level `key`.
pub(super) fn remove_key(text: Option<&str>, key: &str) -> Result<Option<String>, String> {
    let document = read(text)?;
    if document.get(key).is_none() {
        return Ok(text.map(str::to_owned));
    }
    let mut expected = document.clone();
    expected
        .as_object_mut()
        .ok_or(CREDENTIALS_SHAPE)?
        .remove(key);
    let candidates = structural(text.unwrap_or(""), |document| {
        document.as_mapping()?.remove(key)?;
        Some(())
    });
    first_reading(candidates.into_iter().collect(), &expected)
}

// ---- block YAML ----------------------------------------------------------------

/// `value` (an object or array) as block YAML at `indent`.
pub(super) fn block(value: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    match value {
        Value::Object(entries) => {
            for (key, value) in entries {
                out.push_str(&pad);
                out.push_str(&scalar_text(key));
                out.push(':');
                if is_block(value) {
                    out.push('\n');
                    out.push_str(&block(value, indent + 2));
                } else {
                    out.push(' ');
                    out.push_str(&inline(value));
                    out.push('\n');
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if is_block(item) && item.is_object() {
                    // The item's first line moves up beside its dash.
                    let nested = block(item, indent + 2);
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(&nested[indent + 2..]);
                } else if is_block(item) {
                    out.push_str(&pad);
                    out.push_str("-\n");
                    out.push_str(&block(item, indent + 2));
                } else {
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(&inline(item));
                    out.push('\n');
                }
            }
        }
        other => {
            out.push_str(&pad);
            out.push_str(&inline(other));
            out.push('\n');
        }
    }
    out
}

/// Containers that hold other containers are written as blocks; empty ones
/// and lists of plain values stay on one line.
fn is_block(value: &Value) -> bool {
    match value {
        Value::Object(entries) => !entries.is_empty(),
        Value::Array(items) => items.iter().any(|item| item.is_object() || item.is_array()),
        _ => false,
    }
}

fn inline(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => scalar_text(text),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(inline).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(key, value)| format!("{}: {}", scalar_text(key), inline(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A string plain when YAML reads it back as the same string in any context,
/// otherwise double-quoted (JSON's escapes are a subset of YAML's).
fn scalar_text(text: &str) -> String {
    let plain = text
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n"
        );
    if plain {
        text.to_owned()
    } else {
        serde_json::to_string(text).unwrap_or_default()
    }
}
