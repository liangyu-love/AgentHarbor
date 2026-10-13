//! Tool argument transport: Gemini's restricted schemas use a lossless JSON
//! envelope; structured fields accidentally stringified by a model are decoded
//! only when the original tool schema explicitly excludes strings.
use super::{Bytes, ForwardResult, StatusCode, Value};
use crate::core::router::ModelProtocol;
use serde_json::json;
use std::collections::{HashMap, HashSet};

const ARGUMENTS: &str = "__agentharbor_arguments_json";
const MAX_BYTES: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;
const INVALID: &str = "invalid_tool_arguments";

#[derive(Default, Clone)]
pub(super) struct Adapter {
    names: HashSet<String>,
    schemas: HashMap<String, Value>,
    namespaces: HashMap<String, HashMap<String, Value>>,
}

// Unknown/reference/composed schemas are not guessed. The common nullable
// type and anyOf/oneOf forms are safe only when every branch specifies a type.
fn allows(schema: &Value, kind: &str, depth: usize) -> Option<bool> {
    if depth >= 64 || schema.get("$ref").is_some() || schema.get("allOf").is_some() {
        return None;
    }
    match &schema["type"] {
        Value::String(value) => Some(value == kind),
        Value::Array(values) => values.iter().try_fold(false, |found, value| {
            Some(found | (value.as_str()? == kind))
        }),
        _ => schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array)
            .filter(|branches| !branches.is_empty())?
            .iter()
            .try_fold(false, |found, branch| {
                Some(found | allows(branch, kind, depth + 1)?)
            }),
    }
}

fn has_structured_fields(schema: &Value) -> bool {
    schema["properties"].as_object().is_some_and(|fields| {
        fields.values().any(|field| {
            allows(field, "array", 0) == Some(true) || allows(field, "object", 0) == Some(true)
        })
    })
}

fn restore_values(value: &mut Value, schema: &Value, depth: usize) {
    if depth >= 64 || schema.get("$ref").is_some() || schema.get("allOf").is_some() {
        return;
    }
    if let Some(text) = value
        .as_str()
        .filter(|_| allows(schema, "string", 0) == Some(false))
    {
        if let Ok(decoded) = serde_json::from_str::<Value>(text) {
            if (decoded.is_array() && allows(schema, "array", 0) == Some(true))
                || (decoded.is_object() && allows(schema, "object", 0) == Some(true))
            {
                *value = decoded;
            }
        }
    }
    let schema = if let Some(branches) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let kind = if value.is_array() {
            "array"
        } else if value.is_object() {
            "object"
        } else {
            return;
        };
        let mut matches = branches
            .iter()
            .filter(|branch| allows(branch, kind, 0) == Some(true));
        let Some(first) = matches.next() else {
            return;
        };
        if matches.next().is_some() {
            return;
        }
        first
    } else {
        schema
    };
    match value {
        Value::Object(values) => {
            if let Some(fields) = schema["properties"].as_object() {
                for (key, value) in values {
                    if let Some(field) = fields.get(key) {
                        restore_values(value, field, depth + 1);
                    }
                }
            }
        }
        Value::Array(values) => {
            if schema["items"].is_object() {
                for value in values {
                    restore_values(value, &schema["items"], depth + 1);
                }
            }
        }
        _ => {}
    }
}

// Inspect schema positions, never default/examples/enum data. Empty/untyped
// items have the same restriction as an absent items declaration.
fn needs_envelope(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let array = object.get("type").is_some_and(|kind| {
        kind == "array"
            || kind
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "array"))
    });
    if array
        && object.get("items").is_none_or(|items| {
            items
                .as_object()
                .is_none_or(|items| !items.contains_key("type"))
        })
    {
        return true;
    }
    [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ]
    .iter()
    .any(|key| {
        object
            .get(*key)
            .and_then(Value::as_object)
            .is_some_and(|schemas| schemas.values().any(needs_envelope))
    }) || [
        "items",
        "additionalItems",
        "additionalProperties",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "unevaluatedItems",
        "unevaluatedProperties",
        "anyOf",
        "oneOf",
        "allOf",
        "prefixItems",
    ]
    .iter()
    .any(|key| {
        object.get(*key).is_some_and(|schema| {
            schema.as_array().map_or_else(
                || needs_envelope(schema),
                |schemas| schemas.iter().any(needs_envelope),
            )
        })
    })
}

pub(super) fn prepare(
    body: Option<Bytes>,
    model: &str,
    protocol: ModelProtocol,
) -> ForwardResult<(Option<Bytes>, Adapter)> {
    let mut adapter = Adapter::default();
    let gemini = crate::core::model_metadata::candidates(model)
        .iter()
        .any(|candidate| {
            candidate
                .value
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .starts_with("gemini-")
        });
    let Some(bytes) = &body else {
        return Ok((body, adapter));
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok((body, adapter));
    };
    if protocol == ModelProtocol::Responses {
        for namespace in value["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tool| tool["type"] == "namespace")
        {
            let Some(scope) = namespace["name"].as_str() else {
                continue;
            };
            for tool in namespace["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tool| tool["type"] == "function")
            {
                if let Some(name) = tool["name"]
                    .as_str()
                    .filter(|_| has_structured_fields(&tool["parameters"]))
                {
                    if scope == "functions" {
                        adapter
                            .schemas
                            .insert(name.into(), tool["parameters"].clone());
                    } else {
                        adapter
                            .namespaces
                            .entry(scope.into())
                            .or_default()
                            .insert(name.into(), tool["parameters"].clone());
                    }
                }
            }
        }
    }
    if let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            let definition = match protocol {
                ModelProtocol::AnthropicMessages => Some(tool),
                ModelProtocol::Responses if tool["type"] == "function" => Some(tool),
                ModelProtocol::ChatCompletions if tool["type"] == "function" => {
                    tool.get_mut("function")
                }
                _ => None,
            };
            let Some(definition) = definition else {
                continue;
            };
            let key = if protocol == ModelProtocol::AnthropicMessages {
                "input_schema"
            } else {
                "parameters"
            };
            let Some(schema) = definition.get(key) else {
                continue;
            };
            let Some(name) = definition["name"].as_str().filter(|name| !name.is_empty()) else {
                return Err((StatusCode::BAD_REQUEST, "tool is missing its name"));
            };
            if has_structured_fields(schema) {
                adapter.schemas.insert(name.to_owned(), schema.clone());
            }
            if !gemini || !needs_envelope(schema) {
                continue;
            }
            adapter.names.insert(name.to_owned());
            let description = format!("A JSON-encoded object containing the complete original tool arguments. Preserve all types and nested values. Original input JSON Schema: {schema}");
            let strict = definition["strict"] == true;
            definition[key] = json!({"type":"object","properties":{
                ARGUMENTS:{"type":"string","description":description}},"required":[ARGUMENTS]});
            if strict {
                definition[key]["additionalProperties"] = json!(false);
            }
        }
    }
    if adapter.names.is_empty() {
        return Ok((body, adapter));
    }
    // Conversation replay must use the same upstream representation as new
    // calls. Tool results and unrelated reasoning/IDs remain untouched.
    match protocol {
        ModelProtocol::AnthropicMessages => {
            if let Some(messages) = value["messages"].as_array_mut() {
                for message in messages.iter_mut().filter(|m| m["role"] == "assistant") {
                    if let Some(blocks) = message["content"].as_array_mut() {
                        for block in blocks.iter_mut().filter(|b| b["type"] == "tool_use") {
                            if adapter.contains(&block["name"]) {
                                block["input"] = json!({ARGUMENTS:block["input"].to_string()});
                            }
                        }
                    }
                }
            }
        }
        ModelProtocol::Responses => {
            if let Some(input) = value["input"].as_array_mut() {
                for call in input
                    .iter_mut()
                    .filter(|item| item["type"] == "function_call")
                {
                    adapter.wrap_string_arguments(call)?;
                }
            }
        }
        ModelProtocol::ChatCompletions => {
            if let Some(messages) = value["messages"].as_array_mut() {
                for message in messages {
                    if let Some(calls) = message["tool_calls"].as_array_mut() {
                        for call in calls {
                            adapter.wrap_string_arguments(&mut call["function"])?;
                        }
                    }
                }
            }
        }
    }
    let bytes = serde_json::to_vec(&value).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot encode Gemini tools",
        )
    })?;
    Ok((Some(Bytes::from(bytes)), adapter))
}

impl Adapter {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.schemas.is_empty() && self.namespaces.is_empty()
    }
    fn contains(&self, name: &Value) -> bool {
        name.as_str().is_some_and(|name| self.names.contains(name))
    }
    fn handles(&self, name: &Value) -> bool {
        self.contains(name)
            || name
                .as_str()
                .is_some_and(|name| self.schemas.contains_key(name))
    }
    fn scoped_schema(&self, namespace: &Value, name: &Value) -> Option<&Value> {
        match namespace.as_str().filter(|scope| *scope != "functions") {
            Some(scope) => self.namespaces.get(scope)?.get(name.as_str()?),
            None => self.schemas.get(name.as_str()?),
        }
    }
    fn handles_call(&self, call: &Value) -> bool {
        if call["namespace"].is_null() || call["namespace"] == "functions" {
            self.handles(&call["name"])
        } else {
            self.scoped_schema(&call["namespace"], &call["name"])
                .is_some()
        }
    }
    fn wrap_string_arguments(&self, call: &mut Value) -> ForwardResult<()> {
        if self.contains(&call["name"]) {
            let arguments = call["arguments"]
                .as_str()
                .ok_or((StatusCode::BAD_REQUEST, "invalid tool history arguments"))?;
            let parsed: Value = serde_json::from_str(arguments)
                .map_err(|_| (StatusCode::BAD_REQUEST, "invalid tool history arguments"))?;
            if !parsed.is_object() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "tool history arguments must be an object",
                ));
            }
            call["arguments"] = json!(json!({ARGUMENTS:arguments}).to_string());
        }
        Ok(())
    }
    fn decode(&self, value: &Value) -> Result<Value> {
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or(INVALID)?;
        let text = object
            .get(ARGUMENTS)
            .and_then(Value::as_str)
            .ok_or(INVALID)?;
        let decoded: Value = serde_json::from_str(text).map_err(|_| INVALID)?;
        if !decoded.is_object() {
            return Err(INVALID);
        }
        Ok(decoded)
    }
    fn decode_input(&self, name: &Value, value: &Value) -> Result<Value> {
        let mut decoded = if self.contains(name) {
            self.decode(value)?
        } else {
            value.clone()
        };
        if !decoded.is_object() {
            return Err(INVALID);
        }
        if let Some(schema) = name.as_str().and_then(|name| self.schemas.get(name)) {
            restore_values(&mut decoded, schema, 0);
        }
        Ok(decoded)
    }
    fn decode_scoped_input(&self, call: &Value, value: &Value) -> Result<Value> {
        if call["namespace"].is_null() || call["namespace"] == "functions" {
            return self.decode_input(&call["name"], value);
        }
        let mut decoded = value.clone();
        if !decoded.is_object() {
            return Err(INVALID);
        }
        if let Some(schema) = self.scoped_schema(&call["namespace"], &call["name"]) {
            restore_values(&mut decoded, schema, 0);
        }
        Ok(decoded)
    }
    /// Gemini envelopes must decode. Type repair is best effort: malformed or
    /// truncated arguments reach the client unchanged, as without this adapter.
    fn envelope(&self, name: &str, namespace: &Value) -> bool {
        (namespace.is_null() || namespace == "functions") && self.names.contains(name)
    }
    fn decode_string(&self, name: &str, namespace: &Value, text: &str) -> Result<String> {
        let decoded = serde_json::from_str::<Value>(text)
            .map_err(|_| INVALID)
            .and_then(|value| {
                let decoded = self
                    .decode_scoped_input(&json!({"name":name,"namespace":namespace}), &value)?;
                Ok((decoded != value).then(|| decoded.to_string()))
            });
        match decoded {
            Ok(Some(repaired)) => Ok(repaired),
            Ok(None) => Ok(text.to_owned()),
            Err(error) if self.envelope(name, namespace) => Err(error),
            Err(_) => Ok(text.to_owned()),
        }
    }
    fn decode_call(&self, call: &mut Value) -> Result<()> {
        if self.handles_call(call) {
            call["arguments"] = json!(self.decode_string(
                call["name"].as_str().ok_or(INVALID)?,
                &call["namespace"],
                call["arguments"].as_str().ok_or(INVALID)?
            )?);
        }
        Ok(())
    }
    pub fn response(&self, value: &mut Value, protocol: ModelProtocol) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let protocol = if super::protocol::is_responses_body(value) {
            ModelProtocol::Responses
        } else {
            protocol
        };
        match protocol {
            ModelProtocol::Responses => {
                if let Some(items) = value["output"].as_array_mut() {
                    for item in items
                        .iter_mut()
                        .filter(|item| item["type"] == "function_call")
                    {
                        self.decode_call(item)?;
                    }
                }
            }
            ModelProtocol::AnthropicMessages => {
                if let Some(blocks) = value["content"].as_array_mut() {
                    for block in blocks
                        .iter_mut()
                        .filter(|block| block["type"] == "tool_use")
                    {
                        if self.handles(&block["name"]) {
                            match self.decode_input(&block["name"], &block["input"]) {
                                Ok(decoded) => block["input"] = decoded,
                                Err(error) if self.contains(&block["name"]) => return Err(error),
                                Err(_) => {}
                            }
                        }
                    }
                }
            }
            ModelProtocol::ChatCompletions => {
                if let Some(choices) = value["choices"].as_array_mut() {
                    for choice in choices {
                        if let Some(calls) = choice["message"]["tool_calls"].as_array_mut() {
                            for call in calls {
                                self.decode_call(&mut call["function"])?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub fn stream(self, protocol: ModelProtocol) -> Stream {
        Stream {
            adapter: self,
            protocol,
            at_start: true,
            pending: Vec::new(),
            calls: HashMap::new(),
            argument_bytes: 0,
            finished: false,
        }
    }
}

struct Call {
    name: String,
    namespace: Value,
    item_id: Value,
    arguments: String,
    emitted: bool,
}
pub(super) struct Stream {
    adapter: Adapter,
    protocol: ModelProtocol,
    at_start: bool,
    pending: Vec<u8>,
    calls: HashMap<(u64, u64), Call>,
    argument_bytes: usize,
    finished: bool,
}
fn frame(value: &Value) -> String {
    match value["type"].as_str() {
        Some(kind) => format!("event: {kind}\ndata: {value}\n\n"),
        None => format!("data: {value}\n\n"),
    }
}
impl Stream {
    /// A Gemini envelope must decode, so its stream is validated. Without one
    /// the adapter only repairs types and forwards anything it cannot parse.
    fn strict(&self) -> bool {
        !self.adapter.names.is_empty()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Bytes> {
        if self.adapter.is_empty() {
            return Ok(Bytes::copy_from_slice(bytes));
        }
        if self.finished {
            return Ok(Bytes::new());
        }
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some((end, next)) = separator(&self.pending) {
            if end > MAX_BYTES {
                return Err(INVALID);
            }
            let bytes: Vec<_> = self.pending.drain(..next).collect();
            let raw = crate::core::protocol::canonical::strip_stream_bom(
                &bytes[..end],
                &mut self.at_start,
            );
            let data = std::str::from_utf8(raw).ok().map(|raw| {
                raw.lines()
                    .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            let Some(data) = data else {
                if self.strict() {
                    return Err(INVALID);
                }
                output.extend_from_slice(&bytes);
                continue;
            };
            if data == "[DONE]" {
                output.extend_from_slice(self.flush_raw().as_bytes());
                if self.calls.values().any(|call| !call.emitted) {
                    return Err(INVALID);
                }
            }
            if data.is_empty() || data == "[DONE]" {
                output.extend_from_slice(&bytes);
                if data == "[DONE]" {
                    self.finished = true;
                    self.pending.clear();
                    break;
                }
                continue;
            }
            let Ok(mut value) = serde_json::from_str::<Value>(&data) else {
                if self.strict() {
                    return Err(INVALID);
                }
                output.extend_from_slice(&bytes);
                continue;
            };
            output.extend_from_slice(self.event(&mut value)?.as_bytes());
            if self.calls.len() > 512 {
                return Err(INVALID);
            }
            if self.finished {
                self.pending.clear();
                break;
            }
        }
        if self.pending.len() > MAX_BYTES {
            return Err(INVALID);
        }
        Ok(Bytes::from(output))
    }
    /// End of the upstream body. A repair-only stream delivers what remains,
    /// including a last frame without its blank line, instead of failing.
    pub fn finish(&mut self) -> Result<Bytes> {
        if self.adapter.is_empty() {
            return Ok(Bytes::new());
        }
        if self.strict() {
            if !self.finished
                || !self.pending.is_empty()
                || self.calls.values().any(|call| !call.emitted)
            {
                return Err(INVALID);
            }
            return Ok(Bytes::new());
        }
        let mut output = Vec::new();
        if !self.finished && !self.pending.is_empty() {
            let mut rest = std::mem::take(&mut self.pending);
            rest.extend_from_slice(b"\n\n");
            output.extend_from_slice(&self.push(&rest)?);
        }
        output.extend_from_slice(self.flush_raw().as_bytes());
        Ok(Bytes::from(output))
    }
    /// Repair-only calls the provider never closed reach the client exactly as
    /// received. Envelope calls stay pending so strict checks still apply.
    fn flush_raw(&mut self) -> String {
        let mut keys: Vec<_> = self
            .calls
            .iter()
            .filter(|(_, call)| {
                !call.emitted && !self.adapter.envelope(&call.name, &call.namespace)
            })
            .map(|(key, _)| *key)
            .collect();
        keys.sort_unstable();
        let mut output = String::new();
        for key in keys {
            let Some(call) = self.calls.get_mut(&key) else {
                continue;
            };
            let arguments = std::mem::take(&mut call.arguments);
            self.argument_bytes -= arguments.len();
            call.emitted = true;
            if arguments.is_empty() {
                continue;
            }
            output.push_str(&frame(&match self.protocol {
                ModelProtocol::AnthropicMessages => json!({"type":"content_block_delta","index":key.1,
                    "delta":{"type":"input_json_delta","partial_json":arguments}}),
                ModelProtocol::Responses => json!({"type":"response.function_call_arguments.delta",
                    "output_index":key.1,"item_id":call.item_id,"delta":arguments}),
                ModelProtocol::ChatCompletions => json!({"choices":[{"index":key.0,
                    "delta":{"tool_calls":[{"index":key.1,"function":{"arguments":arguments}}]},"finish_reason":null}]}),
            }));
        }
        output
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    fn append(&mut self, key: (u64, u64), text: &str) -> Result<()> {
        self.argument_bytes += text.len();
        if self.argument_bytes > MAX_BYTES {
            return Err(INVALID);
        }
        self.calls
            .get_mut(&key)
            .ok_or(INVALID)?
            .arguments
            .push_str(text);
        Ok(())
    }
    fn decoded(&mut self, key: (u64, u64)) -> Result<Option<String>> {
        let call = self.calls.get_mut(&key).ok_or(INVALID)?;
        if call.emitted {
            return Ok(None);
        }
        let decoded = self
            .adapter
            .decode_string(&call.name, &call.namespace, &call.arguments)?;
        self.argument_bytes -= call.arguments.len();
        call.arguments.clear();
        call.emitted = true;
        Ok(Some(decoded).filter(|arguments| !arguments.is_empty()))
    }
    fn event(&mut self, value: &mut Value) -> Result<String> {
        let mut before = String::new();
        match self.protocol {
            ModelProtocol::AnthropicMessages => {
                let key = (0, value["index"].as_u64().unwrap_or(0));
                match value["type"].as_str().unwrap_or_default() {
                    "content_block_start"
                        if value["content_block"]["type"] == "tool_use"
                            && self.adapter.handles(&value["content_block"]["name"]) =>
                    {
                        let initial = &value["content_block"]["input"];
                        let emitted = initial.as_object().is_some_and(|object| !object.is_empty());
                        if emitted {
                            value["content_block"]["input"] = self
                                .adapter
                                .decode_input(&value["content_block"]["name"], initial)?;
                        }
                        self.calls.insert(
                            key,
                            Call {
                                name: value["content_block"]["name"]
                                    .as_str()
                                    .ok_or(INVALID)?
                                    .into(),
                                namespace: Value::Null,
                                item_id: Value::Null,
                                arguments: String::new(),
                                emitted,
                            },
                        );
                    }
                    "content_block_delta"
                        if value["delta"]["type"] == "input_json_delta"
                            && self.calls.contains_key(&key) =>
                    {
                        let call = &self.calls[&key];
                        if !call.emitted {
                            self.append(key, value["delta"]["partial_json"].as_str().ok_or(INVALID)?)?;
                            return Ok(String::new());
                        }
                        // More input after a complete initial input.
                        if self.adapter.envelope(&call.name, &call.namespace) {
                            return Err(INVALID);
                        }
                    }
                    "content_block_stop" if self.calls.contains_key(&key) => {
                        if let Some(arguments) = self.decoded(key)? {
                            before = frame(
                                &json!({"type":"content_block_delta","index":key.1,"delta":{"type":"input_json_delta","partial_json":arguments}}),
                            );
                        }
                    }
                    "message_delta" | "message_stop" | "error" => {
                        before = self.flush_raw();
                        if value["type"] == "message_stop"
                            && self.calls.values().any(|call| !call.emitted)
                        {
                            return Err(INVALID);
                        }
                        if value["type"] != "message_delta" {
                            self.finished = true;
                        }
                    }
                    _ => {}
                }
            }
            ModelProtocol::Responses => {
                let key = (0, value["output_index"].as_u64().unwrap_or(0));
                match value["type"].as_str().unwrap_or_default() {
                    "response.output_item.added"
                        if value["item"]["type"] == "function_call"
                            && self.adapter.handles_call(&value["item"]) =>
                    {
                        let arguments = value["item"]["arguments"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        self.calls.insert(
                            key,
                            Call {
                                name: value["item"]["name"].as_str().ok_or(INVALID)?.into(),
                                namespace: value["item"]["namespace"].clone(),
                                item_id: value["item"]["id"].clone(),
                                arguments: String::new(),
                                emitted: false,
                            },
                        );
                        self.append(key, &arguments)?;
                        value["item"]["arguments"] = json!("");
                    }
                    "response.function_call_arguments.delta" if self.calls.contains_key(&key) => {
                        self.append(key, value["delta"].as_str().ok_or(INVALID)?)?;
                        return Ok(String::new());
                    }
                    "response.function_call_arguments.done" | "response.output_item.done"
                        if self.calls.contains_key(&key) =>
                    {
                        // Final payloads are authoritative and also support streams
                        // whose provider omits the incremental arguments events.
                        let text = if value["type"] == "response.output_item.done" {
                            value["item"]["arguments"].as_str()
                        } else {
                            value["arguments"].as_str()
                        };
                        if let Some(text) = text {
                            let decoded = self.adapter.decode_string(
                                &self.calls[&key].name,
                                &self.calls[&key].namespace,
                                text,
                            )?;
                            if !self.calls[&key].emitted {
                                before = frame(
                                    &json!({"type":"response.function_call_arguments.delta","output_index":key.1,"item_id":value.get("item_id").cloned().unwrap_or_else(|| value["item"]["id"].clone()),"delta":decoded}),
                                );
                                let call = self.calls.get_mut(&key).unwrap();
                                self.argument_bytes -= call.arguments.len();
                                call.arguments.clear();
                                call.emitted = true;
                            }
                            if value["type"] == "response.output_item.done" {
                                value["item"]["arguments"] = json!(decoded);
                            } else {
                                value["arguments"] = json!(decoded);
                            }
                        } else {
                            return Err(INVALID);
                        }
                    }
                    "response.completed" | "response.incomplete" | "response.failed" | "error" => {
                        if value["response"].is_object() {
                            self.adapter
                                .response(&mut value["response"], ModelProtocol::Responses)?;
                        }
                        // A terminal full response can close tools without done frames.
                        if let Some(items) = value["response"]["output"].as_array() {
                            for index in 0..items.len() {
                                if let Some(call) = self.calls.get_mut(&(0, index as u64)) {
                                    call.emitted = true;
                                    self.argument_bytes -= call.arguments.len();
                                    call.arguments.clear();
                                }
                            }
                        }
                        before = self.flush_raw();
                        if matches!(
                            value["type"].as_str(),
                            Some("response.completed" | "response.incomplete")
                        ) && self.calls.values().any(|call| !call.emitted)
                        {
                            return Err(INVALID);
                        }
                        self.finished = true;
                    }
                    _ => {}
                }
            }
            ModelProtocol::ChatCompletions => {
                let header = value.clone();
                if let Some(choices) = value["choices"].as_array_mut() {
                    for choice in choices {
                        let choice_index = choice["index"].as_u64().unwrap_or(0);
                        if let Some(calls) = choice["delta"]["tool_calls"].as_array_mut() {
                            for call in calls {
                                // Without an index a fragment cannot be attributed.
                                let Some(call_index) = call["index"].as_u64() else {
                                    if self.strict() {
                                        return Err(INVALID);
                                    }
                                    continue;
                                };
                                let key = (choice_index, call_index);
                                if self.adapter.handles(&call["function"]["name"]) {
                                    self.calls.entry(key).or_insert(Call {
                                        name: call["function"]["name"]
                                            .as_str()
                                            .ok_or(INVALID)?
                                            .into(),
                                        namespace: Value::Null,
                                        item_id: Value::Null,
                                        arguments: String::new(),
                                        emitted: false,
                                    });
                                }
                                if self.calls.contains_key(&key) {
                                    if let Some(text) = call["function"]["arguments"].as_str() {
                                        self.append(key, text)?;
                                    }
                                    if let Some(function) = call["function"].as_object_mut() {
                                        function.remove("arguments");
                                    }
                                }
                            }
                        }
                        if choice["finish_reason"]
                            .as_str()
                            .is_some_and(|reason| !reason.is_empty())
                        {
                            let keys: Vec<_> = self
                                .calls
                                .keys()
                                .filter(|key| key.0 == choice_index)
                                .copied()
                                .collect();
                            let mut calls = Vec::new();
                            for key in keys {
                                if let Some(arguments) = self.decoded(key)? {
                                    calls.push(
                                        json!({"index":key.1,"function":{"arguments":arguments}}),
                                    );
                                }
                            }
                            if !calls.is_empty() {
                                let mut chunk = header.clone();
                                if let Some(object) = chunk.as_object_mut() {
                                    object.remove("usage");
                                }
                                chunk["choices"] = json!([{"index":choice_index,"delta":{"tool_calls":calls},"finish_reason":null}]);
                                before.push_str(&frame(&chunk));
                            }
                        }
                    }
                }
            }
        }
        before.push_str(&frame(value));
        Ok(before)
    }
}
fn separator(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|i| (i, i + 2))
        .into_iter()
        .chain(
            bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| (i, i + 4)),
        )
        .min_by_key(|p| p.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(protocol: ModelProtocol) -> Value {
        let schema = json!({"type":"object","properties":{"query":{"type":"object","properties":{"where":{"type":"array"}}},"typed":{"type":"array","items":{"type":"integer"}}},"required":["query"],"examples":[{"type":"array"}]});
        let typed = json!({"type":"object","properties":{"values":{"type":"array","items":{"type":"integer"}}}});
        let tool = |name: &str, schema: Value| match protocol {
            ModelProtocol::Responses => json!({"type":"function","name":name,"parameters":schema}),
            ModelProtocol::ChatCompletions => {
                json!({"type":"function","function":{"name":name,"parameters":schema}})
            }
            ModelProtocol::AnthropicMessages => json!({"name":name,"input_schema":schema}),
        };
        json!({"tools":[tool("lookup",schema),tool("typed",typed)],"messages":[],"input":[]})
    }
    fn schema<'a>(value: &'a Value, protocol: ModelProtocol, index: usize) -> &'a Value {
        match protocol {
            ModelProtocol::Responses => &value["tools"][index]["parameters"],
            ModelProtocol::ChatCompletions => &value["tools"][index]["function"]["parameters"],
            ModelProtocol::AnthropicMessages => &value["tools"][index]["input_schema"],
        }
    }
    // Model the restricted Google Schema contract for the actual transmitted
    // shape. Unlike the former mock, an empty items object cannot pass.
    pub(super) fn valid_google_schema(value: &Value) -> bool {
        let Some(kind) = value["type"].as_str() else {
            return false;
        };
        match kind {
            "array" => valid_google_schema(&value["items"]),
            "object" => value["properties"].as_object().is_some_and(|properties| {
                !properties.is_empty() && properties.values().all(valid_google_schema)
            }),
            "string" | "number" | "integer" | "boolean" => true,
            _ => false,
        }
    }
    fn args() -> Value {
        json!({"query":{"where":[1,true,null,{"x":["text",[3]]}]},"typed":[1,2]})
    }
    fn wrapped() -> String {
        json!({ARGUMENTS:args().to_string()}).to_string()
    }
    #[test]
    fn affected_tools_use_valid_google_schema_and_roundtrip_all_json_types() {
        assert!(!valid_google_schema(&json!({"type":"array","items":{}})));
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let original = source(protocol);
            let bytes = Bytes::from(original.to_string());
            for model in [
                "gpt-6-luna",
                "deepseek-flash",
                "glm-5.3-flash",
                "not-gemini",
            ] {
                let (sent, adapter) = prepare(Some(bytes.clone()), model, protocol).unwrap();
                assert_eq!(sent.unwrap(), bytes);
                assert!(
                    adapter.names.is_empty(),
                    "only Gemini uses an upstream envelope"
                );
            }
            let (sent, adapter) = prepare(
                Some(bytes),
                "channel/google/gemini-3.8-flash-high",
                protocol,
            )
            .unwrap();
            let sent: Value = serde_json::from_slice(&sent.unwrap()).unwrap();
            assert!(valid_google_schema(schema(&sent, protocol, 0)));
            assert_eq!(schema(&sent, protocol, 1), schema(&original, protocol, 1));
            let description = schema(&sent, protocol, 0)["properties"][ARGUMENTS]["description"]
                .as_str()
                .unwrap();
            assert!(description.contains(&schema(&original, protocol, 0).to_string()));
            let mut response = match protocol {
                ModelProtocol::Responses => {
                    json!({"id":"resp","output":[{"type":"function_call","id":"fc","call_id":"c","name":"lookup","arguments":wrapped()}]})
                }
                ModelProtocol::ChatCompletions => {
                    json!({"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"lookup","arguments":wrapped()}}]}}]})
                }
                ModelProtocol::AnthropicMessages => {
                    json!({"content":[{"type":"tool_use","id":"c","name":"lookup","input":{ARGUMENTS:args().to_string()}}]})
                }
            };
            adapter.response(&mut response, protocol).unwrap();
            let actual: Value = match protocol {
                ModelProtocol::Responses => {
                    serde_json::from_str(response["output"][0]["arguments"].as_str().unwrap())
                        .unwrap()
                }
                ModelProtocol::ChatCompletions => serde_json::from_str(
                    response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
                ModelProtocol::AnthropicMessages => response["content"][0]["input"].clone(),
            };
            assert_eq!(actual, args());
            let mut next = original.clone();
            match protocol {
                ModelProtocol::Responses => next["input"] = response["output"].clone(),
                ModelProtocol::ChatCompletions => {
                    next["messages"] = json!([response["choices"][0]["message"]])
                }
                ModelProtocol::AnthropicMessages => {
                    next["messages"] = json!([{"role":"assistant","content":response["content"]}])
                }
            }
            let (sent, _) = prepare(
                Some(Bytes::from(next.to_string())),
                "gemini-3.8-flash",
                protocol,
            )
            .unwrap();
            let sent: Value = serde_json::from_slice(&sent.unwrap()).unwrap();
            let wire = match protocol {
                ModelProtocol::Responses => {
                    serde_json::from_str::<Value>(sent["input"][0]["arguments"].as_str().unwrap())
                        .unwrap()
                }
                ModelProtocol::ChatCompletions => serde_json::from_str::<Value>(
                    sent["messages"][0]["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
                ModelProtocol::AnthropicMessages => {
                    sent["messages"][0]["content"][0]["input"].clone()
                }
            };
            assert_eq!(wire, json!({ARGUMENTS:args().to_string()}));
            assert!(adapter.decode(&json!({ARGUMENTS:"[1,2]"})).is_err());
            assert!(adapter.decode(&json!({ARGUMENTS:"{"})).is_err());
            assert!(adapter.decode(&json!({ARGUMENTS:"{}","extra":1})).is_err());
        }
    }
    fn events(protocol: ModelProtocol, arguments: &str) -> Vec<Value> {
        match protocol {
            ModelProtocol::Responses => vec![
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc","call_id":"c","name":"lookup","arguments":""}}),
                json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":arguments}),
                json!({"type":"response.function_call_arguments.done","output_index":0,"arguments":arguments}),
                json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc","call_id":"c","name":"lookup","arguments":arguments,"status":"completed"}}),
                json!({"type":"response.completed","response":{"object":"response","model":"m","status":"completed","output":[{"type":"function_call","id":"fc","call_id":"c","name":"lookup","arguments":arguments,"status":"completed"}]}}),
            ],
            ModelProtocol::AnthropicMessages => vec![
                json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"lookup","input":{}}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":arguments}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"message_stop"}),
            ],
            ModelProtocol::ChatCompletions => vec![
                json!({"id":"chat","model":"m","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":""}]}),
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":arguments}}]},"finish_reason":""}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            ],
        }
    }
    #[test]
    fn schema_declared_arrays_restore_stringified_arguments_in_json_and_sse() {
        let input_schema = json!({"type":"object","properties":{
            "system2_search_query":{"type":["array","null"],"items":{"type":"object","properties":{
                "q":{"type":"string"},"recency":{"type":["integer","null"]},
                "domains":{"type":["array","null"],"items":{"type":"string"}}}}},
            "nullable":{"anyOf":[{"type":"array","items":{"type":"string"}},{"type":"null"}]},
            "literal":{"type":"string"},"union":{"type":["array","string"]},"unknown":{}}});
        let original = json!({"system2_search_query":"[{\"q\":\"Rust\",\"recency\":7,\"domains\":\"[\\\"rust-lang.org\\\"]\"}]",
            "nullable":"[\"keep\"]","literal":"[1,2]","union":"[1,2]","unknown":"[1,2]"});
        let expected = json!({"system2_search_query":[{"q":"Rust","recency":7,"domains":["rust-lang.org"]}],
            "nullable":["keep"],"literal":"[1,2]","union":"[1,2]","unknown":"[1,2]"});
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let tool = match protocol {
                ModelProtocol::Responses => {
                    json!({"type":"function","name":"lookup","parameters":input_schema})
                }
                ModelProtocol::ChatCompletions => {
                    json!({"type":"function","function":{"name":"lookup","parameters":input_schema}})
                }
                ModelProtocol::AnthropicMessages => {
                    json!({"name":"lookup","input_schema":input_schema})
                }
            };
            let request = Bytes::from(json!({"tools":[tool]}).to_string());
            let (sent, adapter) =
                prepare(Some(request.clone()), "mimo-v2.6-flash", protocol).unwrap();
            assert_eq!(sent.unwrap(), request, "request schema must not change");
            let mut response = match protocol {
                ModelProtocol::Responses => {
                    json!({"object":"response","output":[{"type":"function_call","name":"lookup","arguments":original.to_string()}]})
                }
                ModelProtocol::ChatCompletions => {
                    json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"lookup","arguments":original.to_string()}}]}}]})
                }
                ModelProtocol::AnthropicMessages => {
                    json!({"content":[{"type":"tool_use","name":"lookup","input":original}]})
                }
            };
            adapter.response(&mut response, protocol).unwrap();
            let actual = match protocol {
                ModelProtocol::Responses => serde_json::from_str::<Value>(
                    response["output"][0]["arguments"].as_str().unwrap(),
                )
                .unwrap(),
                ModelProtocol::ChatCompletions => serde_json::from_str::<Value>(
                    response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
                ModelProtocol::AnthropicMessages => response["content"][0]["input"].clone(),
            };
            assert_eq!(actual, expected);
            let mut wire = events(protocol, &original.to_string())
                .iter()
                .map(frame)
                .collect::<String>();
            if protocol == ModelProtocol::ChatCompletions {
                wire.push_str("data: [DONE]\n\n");
            }
            let mut stream = adapter.stream(protocol);
            let mut output = Vec::new();
            for byte in wire.as_bytes() {
                output.extend_from_slice(&stream.push(&[*byte]).unwrap());
            }
            stream.finish().unwrap();
            let values: Vec<Value> = std::str::from_utf8(&output)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .filter_map(|data| serde_json::from_str(data.trim()).ok())
                .collect();
            let actual = match protocol {
                ModelProtocol::Responses => values
                    .iter()
                    .find(|v| v["type"] == "response.function_call_arguments.done")
                    .unwrap()["arguments"]
                    .as_str()
                    .unwrap(),
                ModelProtocol::ChatCompletions => values
                    .iter()
                    .find_map(|v| {
                        v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str()
                    })
                    .unwrap(),
                ModelProtocol::AnthropicMessages => values
                    .iter()
                    .find(|v| v["type"] == "content_block_delta")
                    .unwrap()["delta"]["partial_json"]
                    .as_str()
                    .unwrap(),
            };
            assert_eq!(serde_json::from_str::<Value>(actual).unwrap(), expected);
        }
    }
    #[test]
    fn namespaced_schemas_do_not_coerce_other_namespaces_with_the_same_function_name() {
        let array = json!({"type":"object","properties":{"values":{"type":"array","items":{"type":"integer"}}}});
        let text = json!({"type":"object","properties":{"values":{"type":"string"}}});
        let source = json!({"tools":[{"type":"namespace","name":"search_service","tools":[{"type":"function","name":"lookup","parameters":array}]},
            {"type":"namespace","name":"another_service","tools":[{"type":"function","name":"lookup","parameters":text}]},
            {"type":"namespace","name":"functions","tools":[{"type":"function","name":"lookup","parameters":array}]}]});
        let (_, adapter) = prepare(
            Some(Bytes::from(source.to_string())),
            "mimo-v2.6-pro",
            ModelProtocol::Responses,
        )
        .unwrap();
        let arguments = "{\"values\":\"[1,2]\"}";
        let mut response = json!({"object":"response","output":[
            {"type":"function_call","name":"lookup","namespace":"search_service","arguments":arguments},
            {"type":"function_call","name":"lookup","namespace":"another_service","arguments":arguments},
            {"type":"function_call","name":"lookup","namespace":"unknown_service","arguments":arguments},
            {"type":"function_call","name":"lookup","namespace":"functions","arguments":arguments}]});
        adapter
            .response(&mut response, ModelProtocol::Responses)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(response["output"][0]["arguments"].as_str().unwrap())
                .unwrap(),
            json!({"values":[1,2]})
        );
        for item in &response["output"].as_array().unwrap()[1..3] {
            assert_eq!(item["arguments"], arguments);
        }
        assert_eq!(
            serde_json::from_str::<Value>(response["output"][3]["arguments"].as_str().unwrap())
                .unwrap(),
            json!({"values":[1,2]})
        );
        let mut events = events(ModelProtocol::Responses, arguments);
        for event in &mut events {
            if event["item"].is_object() {
                event["item"]["namespace"] = json!("search_service");
            }
            if event["response"].is_object() {
                event["response"]["output"][0]["namespace"] = json!("search_service");
            }
        }
        let mut stream = adapter.stream(ModelProtocol::Responses);
        let bytes = stream
            .push(events.iter().map(frame).collect::<String>().as_bytes())
            .unwrap();
        stream.finish().unwrap();
        let done = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["type"] == "response.function_call_arguments.done")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(done["arguments"].as_str().unwrap()).unwrap(),
            json!({"values":[1,2]})
        );
    }

    #[test]
    fn stream_restores_arguments_across_every_byte_boundary_and_rejects_bad_json() {
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let (_, adapter) = prepare(
                Some(Bytes::from(source(protocol).to_string())),
                "gemini-3.8-flash",
                protocol,
            )
            .unwrap();
            for (prefix, valid) in [
                ("", true),
                ("\u{feff}", true),
                ("", false),
                ("\u{feff}", false),
            ] {
                let arguments = if valid { wrapped() } else { "{broken".into() };
                let mut wire = events(protocol, &arguments)
                    .iter()
                    .map(frame)
                    .collect::<String>();
                if protocol == ModelProtocol::ChatCompletions {
                    wire.push_str("data: [DONE]\n\n");
                }
                wire.insert_str(0, prefix);
                let mut stream = adapter.clone().stream(protocol);
                let mut output = Vec::new();
                let mut failed = false;
                for byte in wire.as_bytes() {
                    match stream.push(&[*byte]) {
                        Ok(bytes) => output.extend_from_slice(&bytes),
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                assert_eq!(failed, !valid);
                if valid {
                    stream.finish().unwrap();
                    let output = String::from_utf8(output).unwrap();
                    assert!(!output.contains(ARGUMENTS));
                    let values: Vec<Value> = output
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter(|text| *text != "[DONE]")
                        .map(|text| serde_json::from_str(text).unwrap())
                        .collect();
                    let text = match protocol {
                        ModelProtocol::Responses => values
                            .iter()
                            .find(|v| v["type"] == "response.function_call_arguments.delta")
                            .unwrap()["delta"]
                            .as_str()
                            .unwrap(),
                        ModelProtocol::AnthropicMessages => values
                            .iter()
                            .find(|v| v["type"] == "content_block_delta")
                            .unwrap()["delta"]["partial_json"]
                            .as_str()
                            .unwrap(),
                        ModelProtocol::ChatCompletions => values
                            .iter()
                            .find_map(|v| {
                                v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
                                    .as_str()
                            })
                            .unwrap(),
                    };
                    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), args());
                }
            }
        }
    }
    #[test]
    fn truncated_envelope_never_emits_a_completed_tool() {
        let (_, adapter) = prepare(
            Some(Bytes::from(
                source(ModelProtocol::AnthropicMessages).to_string(),
            )),
            "gemini-3.8-flash",
            ModelProtocol::AnthropicMessages,
        )
        .unwrap();
        let mut stream = adapter.stream(ModelProtocol::AnthropicMessages);
        stream
            .push(frame(&events(ModelProtocol::AnthropicMessages, "{")[0]).as_bytes())
            .unwrap();
        assert!(stream.finish().is_err());
        assert!(stream
            .push(frame(&json!({"type":"message_stop"})).as_bytes())
            .is_err());
    }

    #[test]
    fn empty_items_are_wrapped_but_annotation_data_never_triggers_transport_changes() {
        for items in [
            Value::Null,
            json!({}),
            json!(true),
            json!({"description":"anything"}),
        ] {
            let mut body = source(ModelProtocol::Responses);
            body["tools"][0]["parameters"]["properties"]["query"]["properties"]["where"]["items"] =
                items;
            let (_, adapter) = prepare(
                Some(Bytes::from(body.to_string())),
                "gemini-3.8-flash",
                ModelProtocol::Responses,
            )
            .unwrap();
            assert!(adapter.names.contains("lookup"));
        }
        let body = json!({"tools":[{"type":"function","name":"typed","parameters":{"type":"object","properties":{"type":{"type":"string"}},"default":{"type":"array"},"examples":[{"type":"array"}],"enum":[{"type":"array"}]}}]});
        let bytes = Bytes::from(body.to_string());
        let (sent, adapter) = prepare(
            Some(bytes.clone()),
            "gemini-3.8-flash",
            ModelProtocol::Responses,
        )
        .unwrap();
        assert_eq!(sent.unwrap(), bytes);
        assert!(adapter.is_empty());
    }

    #[test]
    fn interleaved_typed_tools_keep_arguments_and_chat_chunk_metadata() {
        let protocol = ModelProtocol::ChatCompletions;
        let (_, adapter) = prepare(
            Some(Bytes::from(source(protocol).to_string())),
            "gemini-3.8-flash",
            protocol,
        )
        .unwrap();
        let mut stream = adapter.stream(protocol);
        let first = json!({"id":"chat-id","object":"chat.completion.chunk","created":1,"model":"gemini","choices":[{"index":0,"delta":{"tool_calls":[
            {"index":0,"id":"c","type":"function","function":{"name":"lookup","arguments":wrapped()}},
            {"index":1,"id":"typed-call","type":"function","function":{"name":"typed","arguments":"{\"values\":[1,2]}"}}]},"finish_reason":null}]});
        let last = json!({"id":"chat-id","object":"chat.completion.chunk","created":1,"model":"gemini","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
        let wire = (frame(&first) + &frame(&last) + "data: [DONE]\n\n").replace("\n", "\r\n");
        let bytes = stream.push(wire.as_bytes()).unwrap();
        stream.finish().unwrap();
        assert!(stream.is_finished());
        let text = std::str::from_utf8(&bytes).unwrap();
        let frames: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|line| *line != "[DONE]")
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            frames[0]["choices"][0]["delta"]["tool_calls"][1]["function"]
                .get("arguments")
                .is_none()
        );
        assert_eq!(frames[1]["id"], "chat-id");
        assert_eq!(frames[1]["model"], "gemini");
        let calls = frames[1]["choices"][0]["delta"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls.iter().find(|call| call["index"] == 1).unwrap()["function"]["arguments"],
            "{\"values\":[1,2]}"
        );
        let wrapped = calls.iter().find(|call| call["index"] == 0).unwrap()["function"]
            ["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(wrapped).unwrap(), args());
        assert!(stream.push(b"late bytes").unwrap().is_empty());
    }
    #[test]
    fn reference_only_array_items_need_lossless_envelope() {
        assert!(needs_envelope(
            &json!({"type":"array","items":{"$ref":"#/$defs/item"}})
        ));
        assert!(!needs_envelope(
            &json!({"type":"array","items":{"type":"string"}})
        ));
    }
    #[test]
    fn type_repair_never_rejects_malformed_or_truncated_arguments() {
        // Repair-only adapters (no Gemini envelope) used to validate every
        // structured tool; a truncated call replaced the whole stream by an error.
        let schema = json!({"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"}}}});
        let truncated = "{\"paths\":[\"a";
        for protocol in [ModelProtocol::Responses, ModelProtocol::AnthropicMessages, ModelProtocol::ChatCompletions] {
            let tool = match protocol {
                ModelProtocol::Responses => json!({"type":"function","name":"read_file","parameters":schema}),
                ModelProtocol::ChatCompletions => json!({"type":"function","function":{"name":"read_file","parameters":schema}}),
                ModelProtocol::AnthropicMessages => json!({"name":"read_file","input_schema":schema}),
            };
            let body = Bytes::from(json!({"tools":[tool]}).to_string());
            let (sent, adapter) = prepare(Some(body.clone()), "gpt-6-luna", protocol).unwrap();
            assert_eq!(sent.unwrap(), body);
            assert!(!adapter.is_empty() && adapter.names.is_empty());
            let events: Vec<Value> = match protocol {
                ModelProtocol::Responses => vec![
                    json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc","type":"function_call","call_id":"c1","name":"read_file","arguments":""}}),
                    json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc","delta":truncated}),
                    json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},
                        "output":[{"id":"fc","type":"function_call","call_id":"c1","name":"read_file","arguments":truncated,"status":"incomplete"}]}}),
                ],
                ModelProtocol::AnthropicMessages => vec![
                    json!({"type":"message_start","message":{"id":"m","usage":{"input_tokens":1}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c1","name":"read_file","input":{}}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":truncated}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
                    json!({"type":"message_stop"}),
                ],
                ModelProtocol::ChatCompletions => vec![
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"read_file","arguments":truncated}}]},"finish_reason":null}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
                ],
            };
            let mut frames = events.iter().map(frame).collect::<String>();
            if protocol == ModelProtocol::ChatCompletions { frames.push_str("data: [DONE]\n\n"); }
            for width in [1, 9, frames.len()] {
                let mut stream = adapter.clone().stream(protocol);
                let mut output = Vec::new();
                for chunk in frames.as_bytes().chunks(width) {
                    output.extend_from_slice(&stream.push(chunk).unwrap());
                }
                output.extend_from_slice(&stream.finish().unwrap());
                let text = String::from_utf8(output).unwrap();
                assert!(text.contains(r#"{\"paths\":[\"a"#), "{protocol:?}/{width}: {text}");
                assert!(!text.contains("invalid_tool_arguments"), "{protocol:?}: {text}");
            }
            // A non-stream reply is forwarded unchanged as well.
            let mut reply = match protocol {
                ModelProtocol::Responses => json!({"object":"response","status":"incomplete","output":[{"type":"function_call","name":"read_file","arguments":truncated}]}),
                ModelProtocol::ChatCompletions => json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"read_file","arguments":truncated}}]}}]}),
                ModelProtocol::AnthropicMessages => json!({"content":[{"type":"tool_use","name":"read_file","input":"not an object"}]}),
            };
            let before = reply.clone();
            adapter.response(&mut reply, protocol).unwrap();
            assert_eq!(reply, before);
        }
        // Repair still restores a stringified array when the JSON is complete.
        let (_, adapter) = prepare(Some(Bytes::from(json!({"tools":[{"type":"function","name":"read_file","parameters":schema}]}).to_string())),
            "gpt-6-luna", ModelProtocol::Responses).unwrap();
        assert_eq!(adapter.decode_string("read_file", &Value::Null, "{\"paths\":\"[\\\"a\\\"]\"}").unwrap(), "{\"paths\":[\"a\"]}");
    }
    #[test]
    fn repair_only_streams_forward_unparsable_frames_and_unclosed_calls() {
        let schema = json!({"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"}}}});
        let (_, adapter) = prepare(Some(Bytes::from(json!({"tools":[{"name":"read_file","input_schema":schema}]}).to_string())),
            "claude-x", ModelProtocol::AnthropicMessages).unwrap();
        let mut stream = adapter.stream(ModelProtocol::AnthropicMessages);
        let mut output = stream.push(b": keepalive\n\ndata: not json\n\n").unwrap().to_vec();
        output.extend_from_slice(&stream.push(frame(&json!({"type":"content_block_start","index":1,
            "content_block":{"type":"tool_use","id":"c1","name":"read_file","input":{}}})).as_bytes()).unwrap());
        output.extend_from_slice(&stream.push(frame(&json!({"type":"content_block_delta","index":1,
            "delta":{"type":"input_json_delta","partial_json":"{\"paths\":"}})).as_bytes()).unwrap());
        // The provider never closes the block; the unterminated last frame is
        // delivered at the end of the body together with the buffered input.
        output.extend_from_slice(&stream.push(br#"event: error
data: {"type":"error","error":{"type":"overloaded_error"}}"#).unwrap());
        output.extend_from_slice(&stream.finish().unwrap());
        let text = String::from_utf8(output).unwrap();
        assert!(text.starts_with(": keepalive\n\ndata: not json\n\n"), "{text}");
        assert!(text.contains(r#"{\"paths\":"#) && text.contains("overloaded_error"), "{text}");
        assert!(text.find("paths").unwrap() < text.find("overloaded_error").unwrap(), "{text}");
    }
}
