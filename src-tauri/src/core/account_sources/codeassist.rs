//! Google's Code Assist wire, which Gemini CLI and Antigravity accounts are
//! served on: a Gemini request wrapped in an envelope — {model, project,
//! request} — and a stream of Gemini chunks wrapped as {response}. A Chat
//! request goes in, Chat events come out (`wire`). Gemini 3 signs the calls
//! it makes and wants the signature back on them; signatures are kept here
//! by call id, and a call with none goes with the value Google documents for
//! history it did not sign. Ported from magpie (MIT, see
//! LICENSES/THIRD_PARTY.md).
use super::wire::Event;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Tells Google not to check a call's thought signature.
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";
/// Gemini's own namespace, which it now and then puts on a client tool's name.
const NAMESPACE: &str = "default_api";
const SIGNATURES_KEPT: usize = 4096;

/// Thought signatures by call id, newest last.
static SIGNATURES: Mutex<Option<(HashMap<String, String>, VecDeque<String>)>> = Mutex::new(None);

fn remember_signature(id: &str, signature: &str) {
    if let Ok(mut kept) = SIGNATURES.lock() {
        let (map, order) = kept.get_or_insert_with(Default::default);
        if map.insert(id.to_owned(), signature.to_owned()).is_none() {
            order.push_back(id.to_owned());
            while order.len() > SIGNATURES_KEPT {
                if let Some(old) = order.pop_front() {
                    map.remove(&old);
                }
            }
        }
    }
}

fn signature_of(id: &str) -> Option<String> {
    SIGNATURES.lock().ok()?.as_ref()?.0.get(id).cloned()
}

/// Which of Google's apps the account signed in to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    GeminiCli,
    Antigravity,
}

/// The parts of a Chat message's content: text, images.
fn content_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) if !text.is_empty() => vec![json!({"text": text})],
        Value::Array(items) => items.iter().filter_map(|item| match item["type"].as_str() {
            Some("text") => item["text"].as_str().filter(|text| !text.is_empty()).map(|text| json!({"text": text})),
            Some("image_url") => {
                let url = item["image_url"]["url"].as_str().or(item["image_url"].as_str())?;
                match url.strip_prefix("data:").and_then(|rest| rest.split_once(";base64,")) {
                    Some((mime, data)) => Some(json!({"inlineData": {"mimeType": mime, "data": data}})),
                    None => Some(json!({"fileData": {"mimeType": "image/*", "fileUri": url}})),
                }
            }
            _ => None,
        }).collect(),
        _ => Vec::new(),
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().filter_map(|item| item["text"].as_str()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A JSON schema cut down to the OpenAPI subset Antigravity's function
/// declarations take: no references, unions or keywords outside it.
fn plain_schema(schema: &Value) -> Value {
    let mut defs = Map::new();
    for key in ["$defs", "definitions"] {
        if let Some(found) = schema[key].as_object() {
            defs.extend(found.clone());
        }
    }
    fn walk(value: &Value, defs: &Map<String, Value>, depth: usize) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(|item| walk(item, defs, depth)).collect()),
            Value::Object(object) => {
                if depth > 32 {
                    return json!({"type": "object"});
                }
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    let name = reference.rsplit('/').next().unwrap_or_default();
                    return defs.get(name).map(|found| walk(found, defs, depth + 1)).unwrap_or_else(|| json!({"type": "object"}));
                }
                if let Some(all) = object.get("allOf").and_then(Value::as_array) {
                    let mut merged: Map<String, Value> = object.iter().filter(|(key, _)| *key != "allOf")
                        .map(|(key, value)| (key.clone(), value.clone())).collect();
                    for member in all {
                        if let Value::Object(member) = walk(member, defs, depth + 1) {
                            for (key, value) in member {
                                if key == "properties" {
                                    let properties = merged.entry("properties").or_insert_with(|| json!({}));
                                    if let (Some(into), Some(from)) = (properties.as_object_mut(), value.as_object()) {
                                        into.extend(from.clone());
                                    }
                                } else {
                                    merged.entry(key).or_insert(value);
                                }
                            }
                        }
                    }
                    return walk(&Value::Object(merged), defs, depth + 1);
                }
                for key in ["anyOf", "oneOf"] {
                    if let Some(choices) = object.get(key).and_then(Value::as_array) {
                        let nullable = choices.iter().any(|choice| choice["type"] == "null");
                        let pick = choices.iter().find(|choice| choice.is_object() && choice["type"] != "null");
                        let mut out = pick.map(|pick| walk(pick, defs, depth + 1)).and_then(|value| value.as_object().cloned()).unwrap_or_default();
                        if let Some(description) = object.get("description") {
                            out.entry("description").or_insert(description.clone());
                        }
                        if nullable {
                            out.insert("nullable".into(), json!(true));
                        }
                        out.entry("type").or_insert(json!("string"));
                        return Value::Object(out);
                    }
                }
                let mut out = Map::new();
                for (key, value) in object {
                    match key.as_str() {
                        "description" | "nullable" | "required" | "pattern" | "minimum" | "maximum" | "minLength" | "maxLength"
                        | "minItems" | "maxItems" | "minProperties" | "maxProperties" | "propertyOrdering" => {
                            out.insert(key.clone(), value.clone());
                        }
                        "exclusiveMinimum" | "exclusiveMaximum" => {
                            let bound = if key == "exclusiveMinimum" { "minimum" } else { "maximum" };
                            if value.is_number() && !object.contains_key(bound) {
                                out.insert(bound.into(), value.clone());
                            }
                        }
                        "type" => match value {
                            Value::Array(types) => {
                                for kind in types {
                                    if kind == "null" {
                                        out.insert("nullable".into(), json!(true));
                                    } else {
                                        out.entry("type").or_insert(kind.clone());
                                    }
                                }
                            }
                            other => {
                                out.insert("type".into(), other.clone());
                            }
                        },
                        "properties" => {
                            let properties: Map<String, Value> = value.as_object().map(|properties| properties.iter()
                                .map(|(name, schema)| (name.clone(), walk(schema, defs, depth + 1))).collect()).unwrap_or_default();
                            out.insert(key.clone(), Value::Object(properties));
                        }
                        "items" => {
                            out.insert(key.clone(), walk(value, defs, depth + 1));
                        }
                        "enum" => {
                            // Google takes string enums only.
                            if let Some(values) = value.as_array().filter(|values| values.iter().all(Value::is_string)) {
                                out.insert(key.clone(), Value::Array(values.clone()));
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(constant) = object.get("const").and_then(Value::as_str) {
                    out.insert("enum".into(), json!([constant]));
                }
                if !out.contains_key("type") {
                    if out.contains_key("properties") {
                        out.insert("type".into(), json!("object"));
                    } else if out.contains_key("items") {
                        out.insert("type".into(), json!("array"));
                    }
                }
                if let Some(required) = out.get("required").and_then(Value::as_array).cloned() {
                    let properties = out.get("properties").and_then(Value::as_object);
                    let keep: Vec<Value> = required.into_iter()
                        .filter(|name| name.as_str().is_some_and(|name| properties.is_some_and(|properties| properties.contains_key(name))))
                        .collect();
                    if keep.is_empty() {
                        out.remove("required");
                    } else {
                        out.insert("required".into(), Value::Array(keep));
                    }
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
    walk(schema, &defs, 0)
}

/// How hard the model thinks: a level for Gemini 3, a budget for the rest;
/// none for a model that does not think or a request that did not ask.
fn thinking_config(model: &str, effort: Option<&str>, claude: bool) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    if model.starts_with("gpt-oss") || (claude && !model.contains("thinking")) {
        return None;
    }
    let effort = match effort {
        Some("none") => return None,
        Some(effort) => effort,
        None if claude => "medium",
        None => return None,
    };
    if model.starts_with("gemini-3") || model.starts_with("gemini-pro-agent") {
        let level = match effort {
            "minimal" if model.contains("flash") => "minimal",
            "minimal" | "low" => "low",
            "medium" if model.contains("flash") => "medium",
            _ => "high",
        };
        return Some(json!({"includeThoughts": true, "thinkingLevel": level}));
    }
    let budget = match effort {
        "minimal" => 512,
        "low" => 2048,
        "medium" => 8192,
        "high" => 24576,
        _ => 32768,
    };
    Some(json!({"includeThoughts": true, "thinkingBudget": budget}))
}

/// The Gemini request for a Chat one.
pub(crate) fn gemini_request(chat: &Value, model: &str, app: App) -> Value {
    let antigravity = app == App::Antigravity;
    let claude = model.to_ascii_lowercase().contains("claude");
    // Antigravity hands Claude the calls back with their ids, which Anthropic
    // takes only of letters, digits, `_` and `-`.
    let tool_id = |id: &str| if antigravity {
        id.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' }).collect()
    } else {
        id.to_owned()
    };
    let mut system = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut contents: Vec<Value> = Vec::new();
    let push = |contents: &mut Vec<Value>, role: &str, parts: Vec<Value>| {
        if parts.is_empty() {
            return;
        }
        if let Some(last) = contents.last_mut().filter(|last| last["role"] == role) {
            last["parts"].as_array_mut().expect("parts").extend(parts);
        } else {
            contents.push(json!({"role": role, "parts": parts}));
        }
    };
    for message in chat["messages"].as_array().into_iter().flatten() {
        match message["role"].as_str().unwrap_or("user") {
            "system" | "developer" => {
                let text = text_of(&message["content"]);
                if !text.is_empty() {
                    system.push(text);
                }
            }
            "assistant" => {
                let mut parts = content_parts(&message["content"]);
                let calls = message["tool_calls"].as_array().cloned().unwrap_or_default();
                let signed = calls.iter().any(|call| call["id"].as_str().and_then(signature_of).is_some());
                for call in calls {
                    let id = call["id"].as_str().unwrap_or_default();
                    let name = call["function"]["name"].as_str().unwrap_or_default();
                    names.insert(id.to_owned(), name.to_owned());
                    let args: Value = call["function"]["arguments"].as_str().and_then(|text| serde_json::from_str(text).ok())
                        .filter(Value::is_object).unwrap_or_else(|| json!({}));
                    let mut function_call = json!({"name": name, "args": args});
                    if !id.is_empty() {
                        function_call["id"] = json!(tool_id(id));
                    }
                    let mut part = json!({"functionCall": function_call});
                    match signature_of(id) {
                        _ if claude => part["thoughtSignature"] = json!(SKIP_SIGNATURE),
                        Some(signature) => part["thoughtSignature"] = json!(signature),
                        None if !signed => part["thoughtSignature"] = json!(SKIP_SIGNATURE),
                        None => {}
                    }
                    parts.push(part);
                }
                push(&mut contents, "model", parts);
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().unwrap_or_default();
                let name = names.get(id).cloned().unwrap_or_else(|| "tool".into());
                let key = if antigravity { "result" } else { "output" };
                let mut response = json!({"name": name, "response": {key: text_of(&message["content"])}});
                if !id.is_empty() {
                    response["id"] = json!(tool_id(id));
                }
                push(&mut contents, "user", vec![json!({"functionResponse": response})]);
            }
            _ => {
                let parts = content_parts(&message["content"]);
                push(&mut contents, "user", parts);
            }
        }
    }
    if contents.first().is_some_and(|first| first["role"] == "model") {
        // Gemini refuses a history that opens with the model's turn, which
        // an agent's compaction can leave; Gemini CLI puts this before it.
        contents.insert(0, json!({"role": "user", "parts": [{"text": "[Continuing from previous AI thoughts...]"}]}));
    }
    let mut request = json!({"contents": contents});
    if !system.is_empty() {
        request["systemInstruction"] = json!({"role": "user", "parts": [{"text": system.join("\n\n")}]});
    }
    let choice = &chat["tool_choice"];
    if let Some(tools) = chat["tools"].as_array().filter(|tools| !tools.is_empty() && choice != "none") {
        let declarations: Vec<Value> = tools.iter().filter(|tool| tool["type"] == "function" || tool.get("function").is_some()).map(|tool| {
            let function = &tool["function"];
            let schema = if function["parameters"].is_object() { function["parameters"].clone() } else { json!({"type": "object", "properties": {}}) };
            let mut declaration = json!({"name": function["name"]});
            if let Some(description) = function["description"].as_str().filter(|text| !text.is_empty()) {
                declaration["description"] = json!(description);
            }
            if antigravity {
                declaration["parameters"] = plain_schema(&schema);
            } else {
                declaration["parametersJsonSchema"] = schema;
            }
            declaration
        }).collect();
        request["tools"] = json!([{"functionDeclarations": declarations}]);
        let mut config = json!({"mode": "AUTO"});
        if choice == "required" {
            config["mode"] = json!("ANY");
        } else if let Some(name) = choice["function"]["name"].as_str() {
            config = json!({"mode": "ANY", "allowedFunctionNames": [name]});
        } else if antigravity && claude {
            // Antigravity's Claude wants its calls checked against the schema.
            config["mode"] = json!("VALIDATED");
        }
        request["toolConfig"] = json!({"functionCallingConfig": config});
    }
    let mut generation = Map::new();
    let max_tokens = chat["max_completion_tokens"].as_u64().or(chat["max_tokens"].as_u64());
    if let Some(max) = max_tokens.filter(|_| !antigravity || claude) {
        generation.insert("maxOutputTokens".into(), json!(max));
    }
    for (from, to) in [("temperature", "temperature"), ("top_p", "topP")] {
        if chat[from].is_number() {
            generation.insert(to.into(), chat[from].clone());
        }
    }
    match &chat["stop"] {
        Value::String(stop) => {
            generation.insert("stopSequences".into(), json!([stop]));
        }
        Value::Array(stops) if !stops.is_empty() => {
            generation.insert("stopSequences".into(), json!(stops));
        }
        _ => {}
    }
    if let Some(thinking) = thinking_config(model, chat["reasoning_effort"].as_str(), claude) {
        if let (Some(budget), true, false) = (thinking["thinkingBudget"].as_u64(), claude, generation.contains_key("maxOutputTokens")) {
            // Claude's answer needs room past its thinking.
            generation.insert("maxOutputTokens".into(), json!(budget + 32000));
        }
        generation.insert("thinkingConfig".into(), thinking);
    }
    if chat["response_format"]["type"] == "json_schema" || chat["response_format"]["type"] == "json_object" {
        generation.insert("responseMimeType".into(), json!("application/json"));
        if let Some(schema) = chat["response_format"]["json_schema"]["schema"].as_object() {
            generation.insert("responseJsonSchema".into(), Value::Object(schema.clone()));
        }
    }
    if !generation.is_empty() {
        request["generationConfig"] = Value::Object(generation);
    }
    request
}

/// Antigravity names a conversation after its first message, so every turn
/// of one conversation carries the same session.
fn antigravity_session(request: &Value) -> String {
    use sha2::{Digest, Sha256};
    let first = request["contents"].as_array().into_iter().flatten()
        .find(|content| content["role"] == "user")
        .and_then(|content| content["parts"].as_array()?.iter().find_map(|part| part["text"].as_str().map(str::to_owned)))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let sum = Sha256::digest(first.as_bytes());
    let number = u64::from_be_bytes(sum[..8].try_into().expect("eight bytes")) & 0x7fff_ffff_ffff_ffff;
    format!("-{number}")
}

/// The envelope a request on the account's app goes out in.
pub(crate) fn envelope(request: Value, model: &str, project: &str, app: App) -> Value {
    let id = uuid::Uuid::new_v4().simple().to_string();
    match app {
        App::GeminiCli => json!({"model": model, "project": project, "user_prompt_id": id, "request": request}),
        App::Antigravity => {
            let mut request = request;
            request["sessionId"] = json!(antigravity_session(&request));
            json!({"model": model, "project": project, "userAgent": "antigravity", "requestType": "agent",
                "requestId": format!("agent-{id}"), "request": request})
        }
    }
}

/// A client's tool name without Gemini's namespace on it.
fn tool_name(name: &str) -> &str {
    for separator in [':', '.'] {
        if let Some(rest) = name.strip_prefix(NAMESPACE).and_then(|rest| rest.strip_prefix(separator)).filter(|rest| !rest.is_empty()) {
            return rest;
        }
    }
    name
}

fn finish_of(reason: &str) -> &'static str {
    match reason {
        "MAX_TOKENS" => "length",
        "STOP" | "FINISH_REASON_UNSPECIFIED" | "OTHER" | "MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL" => "stop",
        _ => "content_filter",
    }
}

/// Reads Code Assist's SSE: Gemini chunks, each wrapped in {response}.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    stopped: bool,
    usage: Option<Event>,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            self.line(&line, out);
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Event>) {
        let rest = std::mem::take(&mut self.buffer);
        self.line(&rest, out);
        if let Some(usage) = self.usage.take() {
            out.push(usage);
        }
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<Event>) {
        let text = String::from_utf8_lossy(line);
        let text = text.trim();
        let Some(data) = text.strip_prefix("data:").map(str::trim).or_else(|| text.starts_with('{').then_some(text)) else { return };
        let Ok(value) = serde_json::from_str::<Value>(data) else { return };
        self.chunk(&value, out);
    }

    /// One Gemini chunk, or a whole reply that did not stream.
    pub(crate) fn chunk(&mut self, value: &Value, out: &mut Vec<Event>) {
        if let Some(message) = value["error"]["message"].as_str() {
            out.push(Event::Error(message.to_owned()));
            return;
        }
        let chunk = if value["response"].is_object() { &value["response"] } else { value };
        if let Some(usage) = chunk["usageMetadata"].as_object() {
            let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or_default();
            let (prompt, cached, thoughts) = (number("promptTokenCount"), number("cachedContentTokenCount"), number("thoughtsTokenCount"));
            self.usage = Some(Event::Usage { input: prompt, output: number("candidatesTokenCount") + thoughts, cached, reasoning: thoughts });
        }
        for candidate in chunk["candidates"].as_array().into_iter().flatten() {
            for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
                let signature = part["thoughtSignature"].as_str().filter(|signature| !signature.is_empty());
                if let Some(call) = part["functionCall"].as_object() {
                    let id = call.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned)
                        .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
                    if let Some(signature) = signature {
                        remember_signature(&id, signature);
                    }
                    let args = call.get("args").filter(|args| args.is_object()).cloned().unwrap_or_else(|| json!({}));
                    out.push(Event::ToolStart { id: id.clone(), name: tool_name(call.get("name").and_then(Value::as_str).unwrap_or_default()).to_owned() });
                    out.push(Event::ToolArgs { id, text: args.to_string() });
                } else if part["thought"] == true {
                    if let Some(text) = part["text"].as_str() {
                        out.push(Event::Reasoning(text.to_owned()));
                    }
                } else if let Some(text) = part["text"].as_str() {
                    out.push(Event::Text(text.to_owned()));
                }
            }
            if let Some(reason) = candidate["finishReason"].as_str().filter(|reason| !reason.is_empty()) {
                if !self.stopped {
                    self.stopped = true;
                    out.push(Event::Finish(finish_of(reason)));
                }
            }
        }
        if self.stopped {
            if let Some(usage) = self.usage.take() {
                out.push(usage);
            }
        }
    }
}

/// Antigravity answers the system prompt of Claude Code and the Claude Agent
/// SDK with a 429 "Resource has been exhausted" whatever quota is left.
pub const TURNED_AWAY_HINT: &str = "Antigravity answers the system prompt of Claude Code and the Claude Agent SDK with this 429 \
    whatever quota is left, so waiting will not help: use Antigravity from OpenCode or Pi, or put another member after it in an aggregate";

pub fn explain(app: App, status: u16, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    match status {
        429 if app == App::Antigravity && text.contains("resource has been exhausted") => Some(TURNED_AWAY_HINT.into()),
        429 => Some("Google's allowance for this model is used up for now; it resets on its own (see the account's quota)".into()),
        401 => Some("Google refused the account's token; sign in again on the Accounts page".into()),
        403 if app == App::GeminiCli => Some("Google serves Gemini CLI's sign-in only to Gemini Code Assist Standard and Enterprise with a Google Cloud project; set the account's project, or use Antigravity for a personal Google account".into()),
        403 => Some("Google refused this account for Antigravity (its region, age or plan)".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_turn_becomes_gemini_contents_with_signatures_back_on_calls() {
        remember_signature("call-signed", "sig-1");
        let chat = json!({"messages":[
            {"role":"system","content":"be brief"},
            {"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call-signed","type":"function","function":{"name":"bash","arguments":"{\"cmd\":\"ls\"}"}},
                {"id":"call-two","type":"function","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call-signed","content":"a.txt"},
            {"role":"tool","tool_call_id":"call-two","content":"…"}
        ],"tools":[{"type":"function","function":{"name":"bash","parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}}],
          "max_tokens":512,"reasoning_effort":"low","stop":"END"});
        let request = gemini_request(&chat, "gemini-3-flash", App::GeminiCli);
        assert_eq!(request["systemInstruction"]["parts"][0]["text"], "be brief");
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3, "the two tool results share one user turn");
        assert_eq!(contents[0]["parts"][1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(contents[1]["parts"][0]["functionCall"]["args"]["cmd"], "ls");
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "sig-1");
        assert!(contents[1]["parts"][1].get("thoughtSignature").is_none(), "Gemini signs only the first call of a step");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "bash");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["response"]["output"], "a.txt");
        assert!(request["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"].is_object());
        let generation = &request["generationConfig"];
        assert_eq!((generation["maxOutputTokens"].as_u64(), generation["stopSequences"][0].as_str()), (Some(512), Some("END")));
        assert_eq!(generation["thinkingConfig"]["thinkingLevel"], "low");
    }

    #[test]
    fn antigravity_takes_plain_schemas_and_names_its_session() {
        let chat = json!({"messages":[{"role":"assistant","content":"earlier"},{"role":"user","content":"go"}],
            "tools":[{"type":"function","function":{"name":"edit","parameters":{"$defs":{"P":{"type":"string","format":"uri"}},
                "type":"object","properties":{"path":{"$ref":"#/$defs/P"},"mode":{"anyOf":[{"type":"string","enum":["a","b"]},{"type":"null"}]}},
                "required":["path","gone"],"additionalProperties":false}}}],"max_tokens":100});
        let request = gemini_request(&chat, "claude-sonnet-4-6", App::Antigravity);
        assert_eq!(request["contents"][0]["parts"][0]["text"], "[Continuing from previous AI thoughts...]");
        let parameters = &request["tools"][0]["functionDeclarations"][0]["parameters"];
        assert_eq!(parameters["properties"]["path"], json!({"type":"string"}), "references resolved, unknown keywords dropped");
        assert_eq!(parameters["properties"]["mode"]["nullable"], true);
        assert_eq!(parameters["required"], json!(["path"]));
        assert!(parameters.get("additionalProperties").is_none());
        assert_eq!(request["toolConfig"]["functionCallingConfig"]["mode"], "VALIDATED");
        assert_eq!(request["generationConfig"]["maxOutputTokens"], 100);
        let one = envelope(request.clone(), "claude-sonnet-4-6", "proj-1", App::Antigravity);
        let two = envelope(request, "claude-sonnet-4-6", "proj-1", App::Antigravity);
        assert_eq!(one["request"]["sessionId"], two["request"]["sessionId"], "one conversation, one session");
        assert_eq!((one["project"].as_str(), one["requestType"].as_str()), (Some("proj-1"), Some("agent")));
    }

    #[test]
    fn the_stream_becomes_events_and_keeps_signatures() {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        let sse = concat!(
            "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"plan\",\"thought\":true}]}}]}}\n\n",
            "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]}}]}}\n\n",
            "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo\"},",
            "{\"functionCall\":{\"id\":\"fc-1\",\"name\":\"default_api:bash\",\"args\":{\"cmd\":\"ls\"}},\"thoughtSignature\":\"sig-9\"}]},",
            "\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":4,\"thoughtsTokenCount\":3,\"cachedContentTokenCount\":2}}}\n\n",
        );
        for piece in sse.as_bytes().chunks(17) {
            decoder.feed(piece, &mut out);
        }
        decoder.finish(&mut out);
        assert_eq!(out, vec![
            Event::Reasoning("plan".into()), Event::Text("Hel".into()), Event::Text("lo".into()),
            Event::ToolStart { id: "fc-1".into(), name: "bash".into() }, Event::ToolArgs { id: "fc-1".into(), text: "{\"cmd\":\"ls\"}".into() },
            Event::Finish("stop"), Event::Usage { input: 10, output: 7, cached: 2, reasoning: 3 },
        ]);
        assert_eq!(signature_of("fc-1").as_deref(), Some("sig-9"));
        let mut failed = Vec::new();
        Decoder::default().feed(b"data: {\"error\":{\"message\":\"quota\"}}\n", &mut failed);
        assert_eq!(failed, vec![Event::Error("quota".into())]);
    }

    #[test]
    fn antigravity_refusing_claude_codes_prompt_is_named() {
        assert_eq!(explain(App::Antigravity, 429, b"Resource has been exhausted (e.g. check quota)").as_deref(), Some(TURNED_AWAY_HINT));
        assert!(explain(App::GeminiCli, 403, b"{}").unwrap().contains("Google Cloud project"));
        assert!(explain(App::GeminiCli, 500, b"{}").is_none());
    }
}
