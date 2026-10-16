//! Chat Completions bodies fitted to what a vendor's chat endpoint takes:
//! tools whose schemas and names every model behind it accepts, the model's
//! calls named back as the client named its tools, and a `tool_choice` in
//! the string form. Ported from magpie's WorkBuddy plugin (MIT, see
//! LICENSES/THIRD_PARTY.md), where WorkBuddy AI answered 400 "Invalid request
//! parameters" (11133) to tool shapes some of its models (DeepSeek, Kimi)
//! turn away. Every change keeps what a call may hold, except an empty enum
//! (which nothing fits) is left out.
use futures_util::StreamExt;
use hyper::body::Bytes;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};

const TYPES: [&str; 7] = ["string", "number", "integer", "boolean", "object", "array", "null"];
/// Keywords whose value is a schema, a list of schemas, a map of schemas.
const SUB_ONE: [&str; 11] = ["items", "additionalItems", "unevaluatedItems", "contains", "not", "if", "then", "else", "propertyNames",
    "additionalProperties", "unevaluatedProperties"];
const SUB_LIST: [&str; 4] = ["anyOf", "oneOf", "allOf", "prefixItems"];
const SUB_MAP: [&str; 5] = ["properties", "patternProperties", "$defs", "definitions", "dependentSchemas"];
/// Where `true`/`false` stays as it is.
const BOOL_OK: [&str; 2] = ["additionalProperties", "unevaluatedProperties"];

fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// `name` with each character a vendor refuses as `_`, kept apart from `taken`.
fn safe_name(name: &str, taken: &mut BTreeSet<String>) -> String {
    let base: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' }).collect();
    let base = if base.is_empty() { "tool".to_owned() } else { base };
    let mut candidate = base.clone();
    let mut index = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}_{index}");
        index += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// A regex's `\0` (NUL) as `\u0000`, stepping over each `\\`; `\01` (octal) is left.
fn nul_escaped(pattern: &str) -> String {
    let bytes = pattern.as_bytes();
    let mut out = String::with_capacity(pattern.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 1 < bytes.len() {
            let next = bytes[index + 1];
            if next == b'\\' {
                out.push_str("\\\\");
                index += 2;
                continue;
            }
            if next == b'0' && !bytes.get(index + 2).is_some_and(u8::is_ascii_digit) {
                out.push_str("\\u0000");
                index += 2;
                continue;
            }
        }
        let ch = pattern[index..].chars().next().expect("a char at a char boundary");
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// A pattern's `\0` and a tuple's `items` (a list), at every depth.
fn fit_schema(schema: &mut Value, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let mut changed = false;
    match schema {
        Value::Array(values) => {
            for value in values {
                changed |= fit_schema(value, depth + 1);
            }
        }
        Value::Object(object) => {
            if let Some(Value::String(pattern)) = object.get("pattern") {
                if pattern.contains("\\0") {
                    let fitted = nul_escaped(pattern);
                    if &fitted != pattern {
                        object.insert("pattern".into(), Value::String(fitted));
                        changed = true;
                    }
                }
            }
            if let Some(Value::Array(items)) = object.get("items") {
                let list: Vec<Value> = items.iter().filter(|item| item.is_object()).cloned().collect();
                let one = match list.len() {
                    0 => json!({}),
                    1 => list.into_iter().next().expect("one"),
                    _ => json!({"anyOf": list}),
                };
                object.insert("items".into(), one);
                object.remove("additionalItems");
                changed = true;
            }
            for value in object.values_mut() {
                changed |= fit_schema(value, depth + 1);
            }
        }
        _ => {}
    }
    changed
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Array(_) => "array",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Bool(_) => "boolean",
        Value::Object(_) => "object",
    }
}

/// An enum given the type its values have (or that and `"null"`); values
/// of several types become an `anyOf` of one typed enum each.
fn fit_enum(object: &mut Map<String, Value>) -> bool {
    let Some(values) = object.get("enum") else { return false };
    let Some(values) = values.as_array().filter(|values| !values.is_empty()).cloned() else {
        object.remove("enum");
        return true;
    };
    let listed: Vec<&str> = match object.get("type") {
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(kind)) => vec![kind.as_str()],
        _ => Vec::new(),
    };
    if listed.iter().filter(|kind| **kind != "null").count() == 1 {
        return false;
    }
    let mut by_type: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for value in values {
        let kind = value_type(&value);
        match by_type.iter_mut().find(|(found, _)| *found == kind) {
            Some((_, list)) => list.push(value),
            None => by_type.push((kind, vec![value])),
        }
    }
    if let (Some(integer), Some(_)) = (by_type.iter().position(|(kind, _)| *kind == "integer"), by_type.iter().position(|(kind, _)| *kind == "number")) {
        let (_, integers) = by_type.remove(integer);
        by_type.iter_mut().find(|(kind, _)| *kind == "number").expect("number").1.extend(integers);
    }
    let has_null = match by_type.iter().position(|(kind, _)| *kind == "null") {
        Some(index) => {
            by_type.remove(index);
            true
        }
        None => false,
    };
    if by_type.len() <= 1 {
        let want = match by_type.first() {
            None => json!("null"),
            Some((kind, _)) if has_null => json!([kind, "null"]),
            Some((kind, _)) => json!(kind),
        };
        if object.get("type") == Some(&want) {
            return false;
        }
        object.insert("type".into(), want);
        return true;
    }
    let mut split: Vec<Value> = by_type.into_iter().map(|(kind, values)| json!({"type": kind, "enum": values})).collect();
    if has_null {
        split.push(json!({"type": "null"}));
    }
    object.remove("enum");
    object.remove("type");
    if object.get("anyOf").is_some_and(Value::is_array) {
        let mut all = object.get("allOf").and_then(Value::as_array).cloned().unwrap_or_default();
        all.push(json!({"anyOf": split}));
        object.insert("allOf".into(), Value::Array(all));
    } else {
        object.insert("anyOf".into(), Value::Array(split));
    }
    true
}

/// `true` is `{}` and `false` `{"not": {}}` where a schema goes.
fn sub_schema(value: &mut Value, key: &str) -> bool {
    match value {
        Value::Bool(flag) if !BOOL_OK.contains(&key) => {
            *value = if *flag { json!({}) } else { json!({"not": {}}) };
            true
        }
        _ => false,
    }
}

/// Types a model knows, a `required` that is a list of names, typed enums
/// and no bare booleans, at every depth; a name under `properties` is never
/// read as a keyword.
fn fit_shape(schema: &mut Value, depth: usize) -> bool {
    let Some(object) = schema.as_object_mut().filter(|_| depth <= 64) else { return false };
    let mut changed = false;
    if let Some(kind) = object.get("type") {
        let list: Vec<Value> = match kind {
            Value::Array(kinds) => kinds.clone(),
            other => vec![other.clone()],
        };
        let mut unique: Vec<String> = Vec::new();
        for kind in list.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase) {
            if TYPES.contains(&kind.as_str()) && !unique.contains(&kind) {
                unique.push(kind);
            }
        }
        let want = match unique.len() {
            0 => None,
            1 => Some(json!(unique[0])),
            _ => Some(json!(unique)),
        };
        if want.as_ref() != Some(kind) {
            match want {
                Some(want) => object.insert("type".into(), want),
                None => object.remove("type"),
            };
            changed = true;
        }
    }
    match object.get_mut("required") {
        Some(Value::Array(required)) if required.iter().any(|key| !key.is_string()) => {
            required.retain(Value::is_string);
            changed = true;
        }
        Some(Value::Array(_)) | None => {}
        Some(_) => {
            object.remove("required");
            changed = true;
        }
    }
    changed |= fit_enum(object);
    for key in SUB_ONE {
        if let Some(value) = object.get_mut(key) {
            changed |= sub_schema(value, key);
            changed |= fit_shape(value, depth + 1);
        }
    }
    for key in SUB_LIST {
        if let Some(Value::Array(values)) = object.get_mut(key) {
            for value in values {
                changed |= sub_schema(value, key);
                changed |= fit_shape(value, depth + 1);
            }
        }
    }
    for key in SUB_MAP {
        if let Some(Value::Object(map)) = object.get_mut(key) {
            for value in map.values_mut() {
                changed |= sub_schema(value, key);
                changed |= fit_shape(value, depth + 1);
            }
        }
    }
    changed
}

/// The schema a local `$ref` in `root` names, or `schema` itself.
fn local_ref<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str).and_then(|reference| reference.strip_prefix("#/")) else {
        return schema;
    };
    let mut at = root;
    for key in reference.split('/') {
        match at.get(key.replace("~1", "/").replace("~0", "~")) {
            Some(next) => at = next,
            None => return schema,
        }
    }
    if at.is_object() { at } else { schema }
}

fn has_union(value: &Value) -> bool {
    ["allOf", "anyOf", "oneOf"].iter().any(|key| value.get(*key).is_some_and(Value::is_array))
}

/// A tool's parameters as an object at the root: `allOf`'s branches merged
/// whole; of `anyOf`'s and `oneOf`'s object branches the properties merged
/// and a field each requires kept required.
fn object_root(params: &mut Value, root: &Value, depth: usize) -> bool {
    let unions: Vec<&str> = ["allOf", "anyOf", "oneOf"].into_iter().filter(|key| params.get(*key).is_some_and(Value::is_array)).collect();
    if params["type"] == "object" && unions.is_empty() {
        return false;
    }
    let mut props = params.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut required: Vec<Value> = params.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
    let expand = |branch: &Value| -> Value {
        let mut branch = local_ref(root, branch).clone();
        if depth < 8 && has_union(&branch) {
            object_root(&mut branch, root, depth + 1);
        }
        branch
    };
    for branch in params.get("allOf").and_then(Value::as_array).cloned().unwrap_or_default() {
        let branch = expand(&branch);
        if !branch.is_object() {
            continue;
        }
        for (key, value) in branch.get("properties").and_then(Value::as_object).cloned().unwrap_or_default() {
            props.entry(key).or_insert(value);
        }
        required.extend(branch.get("required").and_then(Value::as_array).cloned().unwrap_or_default());
    }
    let mut branches = Vec::new();
    for key in ["anyOf", "oneOf"] {
        for branch in params.get(key).and_then(Value::as_array).cloned().unwrap_or_default() {
            let branch = expand(&branch);
            if branch.is_object() && (branch["type"] == "object" || branch.get("properties").is_some()) {
                branches.push(branch);
            }
        }
    }
    let mut from: Vec<(String, Vec<Value>)> = Vec::new();
    for branch in &branches {
        for (key, value) in branch.get("properties").and_then(Value::as_object).into_iter().flatten() {
            if props.contains_key(key) {
                continue;
            }
            match from.iter_mut().find(|(found, _)| found == key) {
                Some((_, seen)) => if !seen.contains(value) { seen.push(value.clone()) },
                None => from.push((key.clone(), vec![value.clone()])),
            }
        }
    }
    for (key, list) in from {
        props.insert(key, if list.len() == 1 { list.into_iter().next().expect("one") } else { json!({"anyOf": list}) });
    }
    if let Some(first) = branches.first() {
        for key in first.get("properties").and_then(Value::as_object).into_iter().flatten().map(|(key, _)| key) {
            if branches.iter().all(|branch| branch["required"].as_array().is_some_and(|list| list.iter().any(|item| item == key))) {
                required.push(json!(key));
            }
        }
    }
    let Some(object) = params.as_object_mut() else {
        *params = json!({"type": "object", "properties": props});
        return true;
    };
    for key in unions {
        object.remove(key);
    }
    object.insert("type".into(), json!("object"));
    let mut kept: Vec<Value> = Vec::new();
    for key in required {
        if key.as_str().is_some_and(|name| props.contains_key(name)) && !kept.contains(&key) {
            kept.push(key);
        }
    }
    object.insert("properties".into(), Value::Object(props));
    if kept.is_empty() {
        object.remove("required");
    } else {
        object.insert("required".into(), Value::Array(kept));
    }
    true
}

/// The names given the tools, each to the client's own.
pub type Names = HashMap<String, String>;

/// Mend a chat's function tools in place: parameters every model takes,
/// names of letters, digits, `_` and `-`. Calls in the chat's history, and
/// a named `tool_choice`, follow their tool's new name. Says whether it
/// changed anything, and the names it gave.
pub fn fit_tools(body: &mut Value) -> (bool, Names) {
    let mut names = Names::new();
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut).filter(|tools| !tools.is_empty()) else {
        return (false, names);
    };
    let mut changed = false;
    let mut taken: BTreeSet<String> = tools.iter().filter_map(|tool| tool["function"]["name"].as_str()).filter(|name| name_ok(name)).map(str::to_owned).collect();
    let mut theirs: HashMap<String, String> = HashMap::new();
    for tool in tools.iter_mut() {
        let is_function = tool["type"] == "function" || tool.get("function").is_some();
        let Some(function) = tool.get_mut("function").filter(|_| is_function).and_then(Value::as_object_mut) else { continue };
        if let Some(name) = function.get("name").and_then(Value::as_str).map(str::to_owned) {
            if !name_ok(&name) {
                let safe = safe_name(&name, &mut taken);
                names.insert(safe.clone(), name.clone());
                theirs.insert(name, safe.clone());
                function.insert("name".into(), Value::String(safe));
                changed = true;
            }
        }
        let Some(params) = function.get_mut("parameters") else { continue };
        if !params.is_object() {
            *params = json!({"type": "object", "properties": {}});
            changed = true;
            continue;
        }
        changed |= fit_schema(params, 0);
        changed |= fit_shape(params, 0);
        let root = params.clone();
        changed |= object_root(params, &root, 0);
    }
    if !theirs.is_empty() {
        for message in body.get_mut("messages").and_then(Value::as_array_mut).into_iter().flatten() {
            for call in message.get_mut("tool_calls").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(safe) = call["function"]["name"].as_str().and_then(|name| theirs.get(name)) {
                    call["function"]["name"] = json!(safe);
                }
            }
            if message["role"] == "tool" {
                if let Some(safe) = message["name"].as_str().and_then(|name| theirs.get(name)) {
                    message["name"] = json!(safe);
                }
            }
        }
        if let Some(safe) = body["tool_choice"]["function"]["name"].as_str().and_then(|name| theirs.get(name)) {
            body["tool_choice"]["function"]["name"] = json!(safe);
        }
    }
    (changed, names)
}

/// A `tool_choice` in the string form some vendors take: `none` drops the
/// tools, `auto`/`required` stay, a named function is its name.
pub fn string_tool_choice(body: &mut Value) -> bool {
    let Some(object) = body.as_object_mut() else { return false };
    let Some(choice) = object.get("tool_choice").cloned() else { return false };
    let drop_tools = |object: &mut Map<String, Value>| {
        object.remove("tool_choice");
        object.remove("tools");
        object.remove("functions");
    };
    match &choice {
        Value::String(text) => {
            if text.trim().eq_ignore_ascii_case("none") {
                drop_tools(object);
                return true;
            }
            false
        }
        Value::Object(choice) => {
            match choice.get("type").and_then(Value::as_str).map(|kind| kind.trim().to_ascii_lowercase()).as_deref() {
                Some("none") => drop_tools(object),
                Some(kind @ ("auto" | "required")) => {
                    object.insert("tool_choice".into(), json!(kind));
                }
                Some("function") => {
                    let name = choice.get("function").and_then(|function| function["name"].as_str()).map(str::trim).filter(|name| !name.is_empty());
                    object.insert("tool_choice".into(), json!(name.unwrap_or("auto")));
                }
                _ => {
                    object.remove("tool_choice");
                }
            }
            true
        }
        _ => {
            object.remove("tool_choice");
            true
        }
    }
}

/// The model's calls in a chat reply named as the client named its tools.
fn named_back(value: &mut Value, names: &Names) -> bool {
    let mut changed = false;
    for choice in value.get_mut("choices").and_then(Value::as_array_mut).into_iter().flatten() {
        for at in ["message", "delta"] {
            for call in choice.get_mut(at).and_then(|at| at.get_mut("tool_calls")).and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(ours) = call["function"]["name"].as_str().and_then(|name| names.get(name)) {
                    call["function"]["name"] = json!(ours);
                    changed = true;
                }
            }
        }
    }
    changed
}

fn renamed_line(line: &str, names: &Names) -> String {
    let Some(data) = line.strip_prefix("data:") else { return line.to_owned() };
    let data = data.trim_start();
    if data == "[DONE]" {
        return line.to_owned();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(data) else { return line.to_owned() };
    if named_back(&mut value, names) { format!("data: {value}") } else { line.to_owned() }
}

/// A Chat reply (one JSON body or an SSE stream) with its calls named back.
pub async fn rename_reply(response: reqwest::Response, names: Names) -> reqwest::Response {
    if names.is_empty() || !response.status().is_success() {
        return response;
    }
    let status = response.status();
    let streamed = response.headers().get(hyper::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("event-stream"));
    if !streamed {
        let bytes = response.bytes().await.unwrap_or_default();
        let body = match serde_json::from_slice::<Value>(&bytes) {
            Ok(mut value) if !value.is_null() => {
                if named_back(&mut value, &names) { value.to_string().into_bytes() } else { bytes.to_vec() }
            }
            _ => bytes.to_vec(),
        };
        return reqwest::Response::from(hyper::Response::builder().status(status)
            .header(hyper::header::CONTENT_TYPE, "application/json").body(body).expect("static response parts"));
    }
    let state = (response.bytes_stream(), names, String::new());
    let stream = futures_util::stream::unfold(Some(state), |state| async move {
        let (mut upstream, names, mut pending) = state?;
        loop {
            match upstream.next().await {
                Some(Ok(chunk)) => {
                    pending.push_str(&String::from_utf8_lossy(&chunk));
                    let Some(end) = pending.rfind('\n') else { continue };
                    let lines: String = pending.drain(..=end).collect();
                    let out: String = lines.split_inclusive('\n')
                        .map(|line| {
                            let bare = line.trim_end_matches(['\r', '\n']);
                            format!("{}{}", renamed_line(bare, &names), &line[bare.len()..])
                        })
                        .collect();
                    return Some((Ok::<_, std::io::Error>(Bytes::from(out)), Some((upstream, names, pending))));
                }
                Some(Err(error)) => return Some((Err(std::io::Error::other(error.to_string())), None)),
                None => {
                    return (!pending.is_empty()).then(|| (Ok(Bytes::from(renamed_line(&pending, &names))), None));
                }
            }
        }
    });
    reqwest::Response::from(hyper::Response::builder().status(status)
        .header(hyper::header::CONTENT_TYPE, "text/event-stream")
        .body(reqwest::Body::wrap_stream(stream)).expect("static response parts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_made_safe_and_calls_follow_them() {
        let mut body = json!({
            "tools": [
                {"type": "function", "function": {"name": "mcp.read file", "parameters": {"type": "object", "properties": {}}}},
                {"type": "function", "function": {"name": "mcp_read_file", "parameters": {"type": "object"}}},
            ],
            "messages": [
                {"role": "assistant", "tool_calls": [{"id": "1", "function": {"name": "mcp.read file", "arguments": "{}"}}]},
                {"role": "tool", "name": "mcp.read file", "tool_call_id": "1", "content": "x"},
            ],
            "tool_choice": {"type": "function", "function": {"name": "mcp.read file"}},
        });
        let (changed, names) = fit_tools(&mut body);
        assert!(changed);
        assert_eq!(body["tools"][0]["function"]["name"], "mcp_read_file_2", "kept apart from a tool named so already");
        assert_eq!(names["mcp_read_file_2"], "mcp.read file");
        assert_eq!(body["messages"][0]["tool_calls"][0]["function"]["name"], "mcp_read_file_2");
        assert_eq!(body["messages"][1]["name"], "mcp_read_file_2");
        assert_eq!(body["tool_choice"]["function"]["name"], "mcp_read_file_2");
        let mut reply = json!({"choices": [{"delta": {"tool_calls": [{"function": {"name": "mcp_read_file_2"}}]}}]});
        assert!(named_back(&mut reply, &names));
        assert_eq!(reply["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "mcp.read file");
    }

    #[test]
    fn schemas_take_shapes_every_model_accepts() {
        let mut body = json!({"tools": [{"type": "function", "function": {"name": "t", "parameters": {
            "anyOf": [
                {"type": "object", "properties": {"a": {"type": "string"}, "b": {"enum": ["x", 1, null]}}, "required": ["a"]},
                {"type": "object", "properties": {"a": {"type": "string"}, "c": true}, "required": ["a", "c"]},
            ],
        }}}]});
        assert!(fit_tools(&mut body).0);
        let params = &body["tools"][0]["function"]["parameters"];
        assert_eq!(params["type"], "object");
        assert!(params.get("anyOf").is_none());
        assert_eq!(params["required"], json!(["a"]), "only what every branch requires stays required");
        assert_eq!(params["properties"]["c"], json!({}), "true where a schema goes is {{}}");
        let b = &params["properties"]["b"];
        assert!(b.get("enum").is_none() && b["anyOf"].as_array().unwrap().len() == 3, "{b}");

        let mut tuple = json!({"tools": [{"function": {"name": "t", "parameters": {"type": "object", "properties": {
            "p": {"type": "string", "pattern": "^[^\\0]+$"},
            "q": {"type": "array", "items": [{"type": "string"}, {"type": "number"}]},
            "r": {"enum": ["a", "b"]},
            "s": {"enum": []},
            "t": {"type": "String"},
        }, "required": "p"}}}]});
        assert!(fit_tools(&mut tuple).0);
        let props = &tuple["tools"][0]["function"]["parameters"]["properties"];
        assert_eq!(props["p"]["pattern"], "^[^\\u0000]+$");
        assert_eq!(props["q"]["items"], json!({"anyOf": [{"type": "string"}, {"type": "number"}]}));
        assert_eq!(props["r"]["type"], "string");
        assert!(props["s"].get("enum").is_none());
        assert_eq!(props["t"]["type"], "string");
        assert!(tuple["tools"][0]["function"]["parameters"].get("required").is_none());

        let mut fine = json!({"tools": [{"function": {"name": "ok", "parameters": {"type": "object", "properties": {"x": {"type": "string", "enum": ["a"]}}}}}]});
        assert!(!fit_tools(&mut fine).0, "a schema every model takes is left alone");
    }

    #[test]
    fn tool_choice_takes_the_string_form() {
        let mut named = json!({"tools": [1], "tool_choice": {"type": "function", "function": {"name": "bash"}}});
        assert!(string_tool_choice(&mut named));
        assert_eq!(named["tool_choice"], "bash");
        let mut none = json!({"tools": [1], "tool_choice": "none"});
        assert!(string_tool_choice(&mut none));
        assert!(none.get("tools").is_none() && none.get("tool_choice").is_none());
        let mut auto = json!({"tool_choice": "auto"});
        assert!(!string_tool_choice(&mut auto));
        let mut required = json!({"tool_choice": {"type": "required"}});
        string_tool_choice(&mut required);
        assert_eq!(required["tool_choice"], "required");
    }

    #[test]
    fn a_streamed_reply_is_renamed_line_by_line() {
        let names: Names = [("a_b".to_owned(), "a.b".to_owned())].into();
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"a_b"}}]}}]}"#;
        assert!(renamed_line(line, &names).contains("\"a.b\""));
        assert_eq!(renamed_line("data: [DONE]", &names), "data: [DONE]");
        assert_eq!(renamed_line(": keep-alive", &names), ": keep-alive");
    }
}
