//! Lossless transport for tool schemas that Gemini's restricted Schema cannot
//! express. Only affected tools use a JSON string envelope; client tools still
//! receive their original object, including heterogeneous/nested array values.
use super::{Bytes, ForwardResult, StatusCode, Value};
use crate::core::router::ModelProtocol;
use serde_json::json;
use std::collections::{HashMap, HashSet};

const ARGUMENTS: &str = "__agentharbor_arguments_json";
const MAX_BYTES: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;
const INVALID: &str = "invalid_gemini_tool_arguments";

#[derive(Default, Clone)]
pub(super) struct Adapter {
    names: HashSet<String>,
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
    if !gemini {
        return Ok((body, adapter));
    }
    let Some(bytes) = &body else {
        return Ok((body, adapter));
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok((body, adapter));
    };
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
            let Some(schema) = definition.get(key).filter(|schema| needs_envelope(schema)) else {
                continue;
            };
            let Some(name) = definition["name"].as_str().filter(|name| !name.is_empty()) else {
                return Err((StatusCode::BAD_REQUEST, "Gemini tool is missing its name"));
            };
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
    if adapter.is_empty() {
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
        self.names.is_empty()
    }
    fn contains(&self, name: &Value) -> bool {
        name.as_str().is_some_and(|name| self.names.contains(name))
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
    fn decode_string(&self, text: &str) -> Result<String> {
        let value = serde_json::from_str(text).map_err(|_| INVALID)?;
        Ok(self.decode(&value)?.to_string())
    }
    fn decode_call(&self, call: &mut Value) -> Result<()> {
        if self.contains(&call["name"]) {
            call["arguments"] =
                json!(self.decode_string(call["arguments"].as_str().ok_or(INVALID)?)?);
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
                        if self.contains(&block["name"]) {
                            block["input"] = self.decode(&block["input"])?;
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
    pub fn push(&mut self, bytes: &[u8]) -> Result<Bytes> {
        if self.adapter.is_empty() {
            return Ok(Bytes::copy_from_slice(bytes));
        }
        if self.finished {
            return Ok(Bytes::new());
        }
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        while let Some((end, next)) = separator(&self.pending) {
            if end > MAX_BYTES {
                return Err(INVALID);
            }
            let bytes: Vec<_> = self.pending.drain(..next).collect();
            let raw = crate::core::protocol::canonical::strip_stream_bom(&bytes[..end], &mut self.at_start);
            let raw = std::str::from_utf8(raw).map_err(|_| INVALID)?;
            let data = raw
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data == "[DONE]" && self.calls.values().any(|call| !call.emitted) {
                return Err(INVALID);
            }
            if data.is_empty() || data == "[DONE]" {
                output.push_str(std::str::from_utf8(&bytes).map_err(|_| INVALID)?);
                if data == "[DONE]" {
                    self.finished = true;
                    self.pending.clear();
                    break;
                }
                continue;
            }
            let mut value: Value = serde_json::from_str(&data).map_err(|_| INVALID)?;
            output.push_str(&self.event(&mut value)?);
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
    pub fn finish(&self) -> Result<()> {
        if !self.adapter.is_empty() && !self.finished {
            return Err(INVALID);
        }
        if !self.pending.is_empty() || self.calls.values().any(|call| !call.emitted) {
            return Err(INVALID);
        }
        Ok(())
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
        let decoded = self.adapter.decode_string(&call.arguments)?;
        self.argument_bytes -= call.arguments.len();
        call.arguments.clear();
        call.emitted = true;
        Ok(Some(decoded))
    }
    fn event(&mut self, value: &mut Value) -> Result<String> {
        let mut before = String::new();
        match self.protocol {
            ModelProtocol::AnthropicMessages => {
                let key = (0, value["index"].as_u64().unwrap_or(0));
                match value["type"].as_str().unwrap_or_default() {
                    "content_block_start"
                        if value["content_block"]["type"] == "tool_use"
                            && self.adapter.contains(&value["content_block"]["name"]) =>
                    {
                        let initial = &value["content_block"]["input"];
                        let emitted = initial.as_object().is_some_and(|object| !object.is_empty());
                        if emitted {
                            value["content_block"]["input"] = self.adapter.decode(initial)?;
                        }
                        self.calls.insert(
                            key,
                            Call {
                                arguments: String::new(),
                                emitted,
                            },
                        );
                    }
                    "content_block_delta"
                        if value["delta"]["type"] == "input_json_delta"
                            && self.calls.contains_key(&key) =>
                    {
                        if self.calls[&key].emitted {
                            return Err(INVALID);
                        }
                        self.append(key, value["delta"]["partial_json"].as_str().ok_or(INVALID)?)?;
                        return Ok(String::new());
                    }
                    "content_block_stop" if self.calls.contains_key(&key) => {
                        if let Some(arguments) = self.decoded(key)? {
                            before = frame(
                                &json!({"type":"content_block_delta","index":key.1,"delta":{"type":"input_json_delta","partial_json":arguments}}),
                            );
                        }
                    }
                    "message_stop" if self.calls.values().any(|call| !call.emitted) => {
                        return Err(INVALID)
                    }
                    "message_stop" => self.finished = true,
                    _ => {}
                }
            }
            ModelProtocol::Responses => {
                let key = (0, value["output_index"].as_u64().unwrap_or(0));
                match value["type"].as_str().unwrap_or_default() {
                    "response.output_item.added"
                        if value["item"]["type"] == "function_call"
                            && self.adapter.contains(&value["item"]["name"]) =>
                    {
                        let arguments = value["item"]["arguments"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        self.calls.insert(
                            key,
                            Call {
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
                            let decoded = self.adapter.decode_string(text)?;
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
                    "response.completed" | "response.incomplete" => {
                        self.adapter
                            .response(&mut value["response"], ModelProtocol::Responses)?;
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
                        if self.calls.values().any(|call| !call.emitted) {
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
                                let key = (choice_index, call["index"].as_u64().ok_or(INVALID)?);
                                if self.adapter.contains(&call["function"]["name"]) {
                                    self.calls.entry(key).or_insert(Call {
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
                        if !choice["finish_reason"].is_null() {
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
                assert!(adapter.is_empty());
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
                json!({"id":"chat","model":"m","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":arguments}}]},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            ],
        }
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
            for (prefix, valid) in [("", true), ("\u{feff}", true), ("", false), ("\u{feff}", false)] {
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
        assert_eq!(
            frames[0]["choices"][0]["delta"]["tool_calls"][1]["function"]["arguments"],
            "{\"values\":[1,2]}"
        );
        assert_eq!(frames[1]["id"], "chat-id");
        assert_eq!(frames[1]["model"], "gemini");
        assert_eq!(
            serde_json::from_str::<Value>(
                frames[1]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
            args()
        );
        assert!(stream.push(b"late bytes").unwrap().is_empty());
    }
    #[test]
    fn reference_only_array_items_need_lossless_envelope() {
        assert!(needs_envelope(&json!({"type":"array","items":{"$ref":"#/$defs/item"}})));
        assert!(!needs_envelope(&json!({"type":"array","items":{"type":"string"}})));
    }

}
