//! A client's TOML file as the reversible adapters see it: read as the JSON
//! object the transaction compares and journals, and written by editing only
//! the values that differ, so comments, layout and every table the adapter
//! does not manage survive. A result that does not read back as the intended
//! value is refused rather than written.
use serde_json::{Map, Value};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike};

/// `text` as a JSON object, or the error the client page shows for a file
/// the client itself would not read.
pub(super) fn read(text: &str) -> Result<Value, String> {
    let table: toml::Table = text.parse().map_err(|_| "clients.errors.invalidConfig")?;
    serde_json::to_value(table).map_err(|_| "clients.errors.invalidConfig".into())
}

/// `base` (the text the file held when it was read) changed to `next`.
pub(super) fn patch(base: Option<&str>, next: &Value) -> Result<String, String> {
    let base = base.unwrap_or("").trim_start_matches('\u{feff}');
    let mut document: DocumentMut = base.parse().map_err(|_| "clients.errors.invalidConfig")?;
    let before = read(base)?;
    let (Some(before), Some(next_map)) = (before.as_object(), next.as_object()) else {
        return Err("clients.errors.invalidConfig".into());
    };
    apply(document.as_table_mut(), before, next_map, 0, false)?;
    let text = document.to_string();
    if read(&text)? != *next {
        return Err("clients.errors.invalidConfig".into());
    }
    Ok(text)
}

fn apply(
    table: &mut dyn TableLike,
    before: &Map<String, Value>,
    next: &Map<String, Value>,
    depth: usize,
    inline: bool,
) -> Result<(), String> {
    for key in before.keys().filter(|key| !next.contains_key(*key)) {
        table.remove(key);
    }
    for (key, value) in next {
        let old = before.get(key);
        if old == Some(value) {
            continue;
        }
        if let (Some(Value::Object(old)), Value::Object(value)) = (old, value) {
            match table.get_mut(key) {
                Some(Item::Table(child)) => {
                    apply(child, old, value, depth + 1, false)?;
                    continue;
                }
                Some(Item::Value(toml_edit::Value::InlineTable(child))) => {
                    apply(child, old, value, depth + 1, true)?;
                    continue;
                }
                _ => {}
            }
        }
        let mut new = item(value, depth, inline)?;
        // A replaced value keeps the comment written after it.
        if let (Some(Item::Value(old)), Item::Value(new)) = (table.get(key), &mut new) {
            *new.decor_mut() = old.decor().clone();
        }
        table.insert(key, new);
    }
    Ok(())
}

/// A new value: an object at the top two levels of a file's own tables is a
/// `[header]` table (implicit when it only holds tables, like `[model.x]`'s
/// parent); anything deeper or inside an inline table stays inline.
fn item(value: &Value, depth: usize, inline: bool) -> Result<Item, String> {
    match value {
        Value::Object(map) if !inline && depth < 2 => {
            let mut table = Table::new();
            table.set_implicit(!map.is_empty() && map.values().all(Value::is_object));
            for (key, value) in map {
                table.insert(key, item(value, depth + 1, false)?);
            }
            Ok(Item::Table(table))
        }
        value => Ok(Item::Value(scalar(value)?)),
    }
}

fn scalar(value: &Value) -> Result<toml_edit::Value, String> {
    Ok(match value {
        Value::String(text) => text.as_str().into(),
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => integer.into(),
            (None, Some(float)) if number.is_f64() => float.into(),
            _ => return Err("clients.errors.invalidConfig".into()),
        },
        Value::Array(items) => {
            let mut array = Array::new();
            for value in items {
                array.push(scalar(value)?);
            }
            array.into()
        }
        Value::Object(map) => {
            let mut table = InlineTable::new();
            for (key, value) in map {
                table.insert(key, scalar(value)?);
            }
            table.into()
        }
        // TOML has no null; the transaction removes a key instead.
        Value::Null => return Err("clients.errors.invalidConfig".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edits_keep_comments_and_unmanaged_tables() {
        let base = "# my settings\n[ui]\ntheme = \"dark\" # kept\n\n[mcp_servers.x]\ncommand = \"npx\"\n";
        let mut next = read(base).unwrap();
        next["model"] = json!({"relay/gpt-5.5": {
            "model": "gpt-5.5",
            "context_window": 400000,
            "extra_headers": {"x-api-key": "k"},
            "reasoning_efforts": [{"id": "high", "value": "high", "default": true}],
        }});
        next["models"] = json!({"default": "relay/gpt-5.5"});
        let text = patch(Some(base), &next).unwrap();
        assert!(text.starts_with("# my settings\n[ui]\ntheme = \"dark\" # kept\n"), "{text}");
        assert!(text.contains("[model.\"relay/gpt-5.5\"]"), "{text}");
        assert!(!text.contains("[model]\n"), "the parent table stays implicit: {text}");
        assert!(text.contains("extra_headers = { x-api-key = \"k\" }"), "{text}");
        assert_eq!(read(&text).unwrap(), next);

        // Taking the edits back out returns the original text.
        assert_eq!(patch(Some(&text), &read(base).unwrap()).unwrap(), base);
    }

    #[test]
    fn values_inside_existing_tables_are_edited_in_place() {
        let base = "[models]\ndefault = \"grok-build\" # mine\nweb_search = \"grok-4.6\"\n";
        let mut next = read(base).unwrap();
        next["models"]["default"] = json!("relay/x");
        let text = patch(Some(base), &next).unwrap();
        assert_eq!(text, "[models]\ndefault = \"relay/x\" # mine\nweb_search = \"grok-4.6\"\n");
    }

    #[test]
    fn a_file_the_client_cannot_read_is_refused() {
        assert!(read("[models\n").is_err());
        assert!(patch(Some("[models\n"), &json!({})).is_err());
        assert!(patch(None, &json!({"a": null})).is_err());
    }
}
