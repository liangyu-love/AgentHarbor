//! Stateful translation of upstream SSE frames into Responses events.
//! Only [DONE]/message_stop completes a stream; EOF cannot complete tool calls.

use super::canonical::{CanonicalEvent, MAX_RETAINED_BYTES};
use super::responses::{openai_usage, valid_function_call, Completion};
use super::Translation;
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;

pub struct StreamTranslator {
    translation: Translation,
    pending: Vec<u8>,
    response_id: String,
    model: String,
    created: bool,
    finished: bool,
    finish_reason: Option<String>,
    // Allocate indices once, including when tools precede text or reasoning.
    output: Vec<Value>,
    text_index: Option<usize>,
    reasoning_index: Option<usize>,
    tool_indices: HashMap<u64, usize>,
    tool_initial_inputs: HashSet<usize>,
    search_blocks: HashMap<u64, Value>,
    search_arguments: HashMap<u64, String>,
    search_indices: HashMap<String, usize>,
    citations: Vec<Value>,
    native_search: bool,
    /// A Chat upstream sent its reasoning as `reasoning_content`, the field
    /// it takes back (`chat_reasoning`).
    reasoning_content: bool,
    usage: Option<Value>,
    anthropic_input_tokens: u64,
    anthropic_cache_read_tokens: u64,
    anthropic_cache_write_tokens: u64,
    completed_response: Option<Value>,
    include_tool_metadata: bool,
    failure: Option<&'static str>,
    retained_bytes: usize,
}

impl StreamTranslator {
    pub fn new(translation: Translation) -> Self {
        Self {
            translation,
            pending: Vec::new(),
            response_id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            model: String::new(),
            created: false,
            finished: false,
            finish_reason: None,
            output: Vec::new(),
            text_index: None,
            reasoning_index: None,
            tool_indices: HashMap::new(),
            tool_initial_inputs: HashSet::new(),
            search_blocks: HashMap::new(),
            search_arguments: HashMap::new(),
            search_indices: HashMap::new(),
            citations: Vec::new(),
            native_search: false,
            reasoning_content: false,
            usage: None,
            anthropic_input_tokens: 0,
            anthropic_cache_read_tokens: 0,
            anthropic_cache_write_tokens: 0,
            completed_response: None,
            include_tool_metadata: false,
            failure: None,
            retained_bytes: 0,
        }
    }

    /// Include tool metadata only for the Messages stream adapter.
    pub(super) fn with_tool_metadata(mut self) -> Self {
        self.include_tool_metadata = true;
        self
    }
    pub(super) fn with_search(mut self, enabled: bool) -> Self {
        self.native_search = enabled;
        self
    }

    /// Only explicitly completed, validated responses may enter history.
    pub fn completed_response(&self) -> Option<&Value> {
        self.completed_response.as_ref()
    }
    pub fn sent_reasoning_content(&self) -> bool {
        self.reasoning_content
    }
    pub fn failure(&self) -> Option<&'static str> {
        self.failure
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The wire encoder runs only at the final client boundary. Messages and
    /// Chat encoders consume these owned events without intermediate SSE text.
    pub(super) fn push_events(&mut self, chunk: &[u8]) -> Vec<CanonicalEvent> {
        self.decode(chunk)
            .into_iter()
            .map(|payload| {
                CanonicalEvent::from_decoded(
                    self.translation.to,
                    ModelProtocol::Responses,
                    payload,
                    None,
                )
            })
            .collect()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.push_events(chunk)
            .into_iter()
            .map(encode_event)
            .collect()
    }

    fn decode(&mut self, chunk: &[u8]) -> Vec<Value> {
        if self.finished {
            return Vec::new();
        }
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, next)) = find_separator(&self.pending) {
            if end > MAX_PENDING_BYTES {
                events.extend(self.terminal(Completion::Failed("invalid_stream_frame")));
                break;
            }
            let frame = self.pending.drain(..next).take(end).collect::<Vec<_>>();
            match String::from_utf8(frame) {
                Ok(frame) => events.extend(self.handle_frame(&frame)),
                Err(_) => events.extend(self.terminal(Completion::Failed("invalid_stream_frame"))),
            }
            if self.finished {
                self.pending.clear();
                break;
            }
        }
        if !self.finished && self.pending.len() > MAX_PENDING_BYTES {
            self.pending.clear();
            events.extend(self.terminal(Completion::Failed("invalid_stream_frame")));
        }
        self.prepend_created(events)
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.fail("upstream_stream_truncated")
    }

    pub(super) fn fail_events(&mut self, code: &'static str) -> Vec<CanonicalEvent> {
        let events = self.terminal(Completion::Failed(code));
        self.prepend_created(events)
            .into_iter()
            .map(|payload| {
                CanonicalEvent::from_decoded(
                    self.translation.to,
                    ModelProtocol::Responses,
                    payload,
                    None,
                )
            })
            .collect()
    }

    pub fn fail(&mut self, code: &'static str) -> Vec<String> {
        self.fail_events(code)
            .into_iter()
            .map(encode_event)
            .collect()
    }

    fn prepend_created(&mut self, mut events: Vec<Value>) -> Vec<Value> {
        if !self.created && !events.is_empty() {
            self.created = true;
            events.insert(0, sse_frame("response.created", json!({
                "type":"response.created",
                "response":{"id":self.response_id,"object":"response","status":"in_progress","model":self.model,"output":[]},
            })));
        }
        events
    }

    fn handle_frame(&mut self, frame: &str) -> Vec<Value> {
        let mut data = Vec::new();
        let mut event_name = None;
        for line in frame.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event_name = Some(value.trim());
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.trim_start());
            }
        }
        if data.is_empty() {
            return Vec::new();
        }
        let data = data.join("\n");
        if data.trim() == "[DONE]" {
            return if self.translation.to == ModelProtocol::ChatCompletions {
                self.upstream_terminal()
            } else {
                self.terminal(Completion::Failed("invalid_stream_frame"))
            };
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return self.terminal(Completion::Failed("invalid_stream_frame"));
        };
        let kind = value.get("type").and_then(Value::as_str).or(event_name);
        if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
            let mut events = self.terminal(Completion::Failed("upstream_error"));
            if let Some(last) = events.last_mut() {
                let mut response = self.response(Completion::Failed("upstream_error"));
                response["error"] = json!({
                    "code":error.get("code").or_else(|| error.get("type")).and_then(Value::as_str).unwrap_or("upstream_error"),
                    "message":error.get("message").and_then(Value::as_str).unwrap_or("Provider returned an error"),
                });
                *last = sse_frame(
                    "response.failed",
                    json!({"type":"response.failed","response":response}),
                );
            }
            return events;
        }
        match self.translation.to {
            ModelProtocol::ChatCompletions => self.chat_chunk(&value),
            ModelProtocol::AnthropicMessages => self.anthropic_chunk(kind, &value),
            _ => self.terminal(Completion::Failed("invalid_stream_frame")),
        }
    }

    fn chat_chunk(&mut self, value: &Value) -> Vec<Value> {
        if let Some(model) = value.get("model").and_then(Value::as_str) {
            if !self.retain_replacing(self.model.len(), model.len()) {
                return self.terminal(Completion::Failed("invalid_stream_frame"));
            }
            self.model = model.to_owned();
        }
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(openai_usage(usage));
            if let Some(search) = usage.get("web_search_usage") {
                self.usage.as_mut().unwrap()["web_search_usage"] = search.clone();
            }
        }
        let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) else {
            return Vec::new(); // A final usage chunk has no choices.
        };
        let mut events = Vec::new();
        if let Some(delta) = choice.get("delta") {
            if delta["error_message"].as_str().is_some_and(|message| !message.is_empty()) {
                return self.terminal(Completion::Failed("upstream_search_error"));
            }
            if let Some(annotations) = delta["annotations"].as_array() {
                if !self.retain_replacing(0, delta["annotations"].to_string().len()) {
                    return self.terminal(Completion::Failed("invalid_stream_frame"));
                }
                self.citations.extend(annotations.iter().cloned());
            }
            if let Some(text) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
            {
                self.reasoning_content |= !text.is_empty() && delta["reasoning_content"].is_string();
                events.extend(self.text_delta(text, true));
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                events.extend(self.text_delta(text, false));
            }
            if self.finished {
                return events;
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let Some(index) = call
                        .get("index")
                        .and_then(Value::as_u64)
                        .filter(|index| *index < 128)
                    else {
                        events.extend(self.terminal(Completion::Failed("invalid_tool_arguments")));
                        return events;
                    };
                    let function = &call["function"];
                    let is_new = !self.tool_indices.contains_key(&index);
                    let output_index = match self.tool_indices.get(&index).copied() {
                        Some(index) => index,
                        None => {
                            let Some((output_index, event)) = self.open_tool(
                                index,
                                call.get("id").and_then(Value::as_str).unwrap_or_default(),
                                function
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                "",
                            ) else {
                                events.extend(
                                    self.terminal(Completion::Failed("invalid_stream_frame")),
                                );
                                return events;
                            };
                            events.push(event);
                            output_index
                        }
                    };
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        if !self.retain_replacing(
                            self.output[output_index]["call_id"]
                                .as_str()
                                .map(str::len)
                                .unwrap_or(0),
                            id.len(),
                        ) {
                            events
                                .extend(self.terminal(Completion::Failed("invalid_stream_frame")));
                            return events;
                        }
                        self.output[output_index]["call_id"] = json!(id);
                    }
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        // Some relays repeat the whole call header (same id and full
                        // name) on every chunk; that is not a new name fragment.
                        let repeated_header =
                            call.get("id").and_then(Value::as_str).is_some_and(|id| {
                                self.output[output_index]["call_id"] == id
                                    && self.output[output_index]["name"] == name
                            });
                        if !is_new && !repeated_header {
                            if !self.retain_replacing(0, name.len()) {
                                events.extend(
                                    self.terminal(Completion::Failed("invalid_stream_frame")),
                                );
                                return events;
                            }
                            append_string(&mut self.output[output_index]["name"], name);
                        }
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        let Some(event) = self.argument_delta(output_index, arguments) else {
                            events
                                .extend(self.terminal(Completion::Failed("invalid_stream_frame")));
                            return events;
                        };
                        events.push(event);
                    }
                }
            }
            // WorkBuddy sends an empty legacy field alongside valid tool_calls
            // in its terminal chunk. Reject actual unsupported legacy calls.
            if delta
                .get("function_call")
                .is_some_and(|call| !call.is_null() && !call.as_object().is_some_and(|object|
                    object.iter().all(|(key,value)| matches!(key.as_str(), "name" | "arguments") && value == "")))
            {
                events.extend(self.terminal(Completion::Failed("invalid_tool_arguments")));
                return events;
            }
            if delta
                .get("refusal")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
            {
                events.extend(self.terminal(Completion::Incomplete("content_filter")));
                return events;
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if !self.retain_replacing(
                self.finish_reason.as_deref().map(str::len).unwrap_or(0),
                reason.len(),
            ) {
                events.extend(self.terminal(Completion::Failed("invalid_stream_frame")));
                return events;
            }
            self.finish_reason = Some(reason.to_owned());
        }
        // Do not close tools yet: usage and the transport terminator follow.
        events
    }

    fn anthropic_chunk(&mut self, kind: Option<&str>, value: &Value) -> Vec<Value> {
        if self.translation.from == ModelProtocol::ChatCompletions
            && unsupported_chat_messages_event(kind, value)
        {
            return self.terminal(Completion::Failed("unsupported_content"));
        }
        let mut events = Vec::new();
        match kind {
            Some("message_start") => {
                if let Some(model) = value["message"].get("model").and_then(Value::as_str) {
                    if !self.retain_replacing(self.model.len(), model.len()) {
                        return self.terminal(Completion::Failed("invalid_stream_frame"));
                    }
                    self.model = model.to_owned();
                }
                self.merge_anthropic_usage(&value["message"]["usage"]);
            }
            Some("content_block_start") => {
                let block = &value["content_block"];
                // Claude splits one cited answer into many text blocks; only a
                // tool or search between them starts the next message item.
                if matches!(block["type"].as_str(), Some("server_tool_use" | "web_search_tool_result" | "tool_use")) {
                    events.extend(self.flush_citations());
                    if self.finished { return events; }
                    self.text_index = None;
                }
                match block.get("type").and_then(Value::as_str) {
                    Some("server_tool_use") if block["name"] == "web_search" => {
                        let index = value["index"].as_u64().unwrap_or(0);
                        let Some(id) = block["id"].as_str().filter(|id| !id.is_empty()) else {
                            return self.terminal(Completion::Failed("invalid_search_result"));
                        };
                        if self.search_blocks.contains_key(&index) || self.search_indices.contains_key(id)
                            || self.search_blocks.len() >= 128 || !self.retain_replacing(0, block.to_string().len()) {
                            return self.terminal(Completion::Failed("invalid_search_result"));
                        }
                        self.search_blocks.insert(index, block.clone());
                        self.search_indices.insert(id.to_owned(), self.output.len());
                        let item = json!({"type":"web_search_call","id":id,"status":"in_progress",
                            "action":{"type":"search","query":block["input"]["query"].as_str().unwrap_or("")}});
                        events.push(sse_frame("response.output_item.added", json!({"output_index":self.output.len(),"item":item})));
                        self.output.push(item);
                    }
                    Some("web_search_tool_result") => {
                        let Some((_, call)) = self.search_blocks.iter().find(|(_, call)| call["id"] == block["tool_use_id"]) else {
                            return self.terminal(Completion::Failed("invalid_search_result"));
                        };
                        let Ok(item) = super::web_search::history_call(call, block) else {
                            return self.terminal(Completion::Failed("invalid_search_result"));
                        };
                        let index = self.search_indices[block["tool_use_id"].as_str().unwrap()];
                        if self.output[index]["status"] != "in_progress" || !self.retain_replacing(0, item.to_string().len()) {
                            return self.terminal(Completion::Failed("invalid_search_result"));
                        }
                        self.output[index] = item;
                        if self.output[index]["status"] == "completed" {
                            events.push(sse_frame("response.web_search_call.completed", json!({"output_index":index,"item_id":self.output[index]["id"]})));
                        }
                    }
                    Some("tool_use") => {
                        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                        if self.tool_indices.contains_key(&index) || self.tool_indices.len() >= 128
                        {
                            return self.terminal(Completion::Failed("invalid_tool_arguments"));
                        }
                        let arguments = block
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| json!({}))
                            .to_string();
                        let Some((index, event)) = self.open_tool(
                            index,
                            block.get("id").and_then(Value::as_str).unwrap_or_default(),
                            block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            &arguments,
                        ) else {
                            return self.terminal(Completion::Failed("invalid_stream_frame"));
                        };
                        self.tool_initial_inputs.insert(index);
                        events.push(event);
                    }
                    Some("text") => {
                        if let Some(citations) = block["citations"].as_array() {
                            if !self.retain_replacing(0, block["citations"].to_string().len()) {
                                return self.terminal(Completion::Failed("invalid_stream_frame"));
                            }
                            self.citations.extend(citations.iter().cloned());
                        }
                        events.extend(self.text_delta(
                            block
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            false,
                        ));
                    }
                    Some("thinking") => events.extend(
                        self.text_delta(
                            block
                                .get("thinking")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            true,
                        ),
                    ),
                    _ => {}
                }
            }
            Some("content_block_delta") => {
                let delta = &value["delta"];
                match delta.get("type").and_then(Value::as_str) {
                    Some("citations_delta") => {
                        if !self.retain_replacing(0, delta["citation"].to_string().len()) {
                            return self.terminal(Completion::Failed("invalid_stream_frame"));
                        }
                        self.citations.push(delta["citation"].clone());
                    }
                    Some("text_delta") => events.extend(
                        self.text_delta(
                            delta
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            false,
                        ),
                    ),
                    Some("thinking_delta") => events.extend(
                        self.text_delta(
                            delta
                                .get("thinking")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            true,
                        ),
                    ),
                    Some("input_json_delta") => {
                        let upstream_index =
                            value.get("index").and_then(Value::as_u64).unwrap_or(0);
                        if self.search_blocks.contains_key(&upstream_index) {
                            let partial = delta["partial_json"].as_str().unwrap_or("");
                            if !self.retain_replacing(0, partial.len()) {
                                return self.terminal(Completion::Failed("invalid_stream_frame"));
                            }
                            self.search_arguments.entry(upstream_index).or_default().push_str(partial);
                            return events;
                        }
                        let Some(index) = self.tool_indices.get(&upstream_index).copied() else {
                            return self.terminal(Completion::Failed("invalid_tool_arguments"));
                        };
                        if self.tool_initial_inputs.remove(&index) {
                            self.retained_bytes = self.retained_bytes.saturating_sub(
                                self.output[index]["arguments"]
                                    .as_str()
                                    .map(str::len)
                                    .unwrap_or(0),
                            );
                            self.output[index]["arguments"] = json!("");
                        }
                        let Some(event) = self.argument_delta(
                            index,
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ) else {
                            return self.terminal(Completion::Failed("invalid_stream_frame"));
                        };
                        events.push(event);
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                let index = value["index"].as_u64().unwrap_or(0);
                if let Some(arguments) = self.search_arguments.remove(&index) {
                    let Ok(input) = serde_json::from_str::<Value>(&arguments) else {
                        return self.terminal(Completion::Failed("invalid_search_result"));
                    };
                    if !input.is_object() { return self.terminal(Completion::Failed("invalid_search_result")); }
                    self.search_blocks.get_mut(&index).unwrap()["input"] = input;
                }
            }
            Some("message_delta") => {
                self.merge_anthropic_usage(&value["usage"]);
                if let Some(reason) = value["delta"].get("stop_reason").and_then(Value::as_str) {
                    if !self.retain_replacing(
                        self.finish_reason.as_deref().map(str::len).unwrap_or(0),
                        reason.len(),
                    ) {
                        return self.terminal(Completion::Failed("invalid_stream_frame"));
                    }
                    self.finish_reason = Some(reason.to_owned());
                }
            }
            Some("message_stop") | Some("message_end") => events.extend(self.upstream_terminal()),
            Some("error") => events.extend(self.terminal(Completion::Failed("upstream_error"))),
            _ => {}
        }
        events
    }

    /// Messages counts are cumulative, so a later frame never lowers one; a
    /// relay repeating zeros in `message_delta` keeps the start's counts.
    fn merge_anthropic_usage(&mut self, usage: &Value) {
        let count = |key: &str| usage[key].as_u64().unwrap_or(0);
        self.anthropic_input_tokens = self.anthropic_input_tokens.max(count("input_tokens"));
        self.anthropic_cache_read_tokens = self.anthropic_cache_read_tokens.max(count("cache_read_input_tokens"));
        self.anthropic_cache_write_tokens = self.anthropic_cache_write_tokens.max(count("cache_creation_input_tokens"));
        let merged = self
            .usage
            .get_or_insert_with(|| json!({"input_tokens":0,"output_tokens":0,"total_tokens":0}));
        if let Some(search) = usage.get("server_tool_use") {
            merged["server_tool_use"] = search.clone();
        }
        let output = merged["output_tokens"].as_u64().unwrap_or(0).max(count("output_tokens"));
        merged["output_tokens"] = json!(output);
        // Responses and Chat count cache reads and writes as part of the input
        // (Messages leaves them out); a context meter reads this total.
        merged["input_tokens"] = json!(self
            .anthropic_input_tokens
            .saturating_add(self.anthropic_cache_read_tokens)
            .saturating_add(self.anthropic_cache_write_tokens));
        merged["input_tokens_details"] = json!({"cached_tokens":self.anthropic_cache_read_tokens});
        merged["total_tokens"] = json!(
            merged["input_tokens"].as_u64().unwrap_or(0)
                + merged["output_tokens"].as_u64().unwrap_or(0)
        );
    }

    fn text_delta(&mut self, text: &str, reasoning: bool) -> Vec<Value> {
        if self.finished {
            return Vec::new();
        }
        if text.is_empty() {
            return Vec::new();
        }
        if !self.retain_replacing(0, text.len()) {
            return self.terminal(Completion::Failed("invalid_stream_frame"));
        }
        let existing = if reasoning {
            self.reasoning_index
        } else {
            self.text_index
        };
        let mut events = Vec::new();
        let index = existing.unwrap_or_else(|| {
            let index = self.output.len();
            let id = format!("{}_{index}_{}", if reasoning { "rs" } else { "msg" }, self.response_id);
            let item = if reasoning {
                json!({"id":id,"type":"reasoning","summary":[]})
            } else {
                json!({"id":id,"type":"message","role":"assistant","status":"in_progress","content":[]})
            };
            events.push(sse_frame("response.output_item.added", json!({"type":"response.output_item.added","output_index":index,"item":item})));
            self.output.push(item);
            let (event, part) = if reasoning {
                self.reasoning_index = Some(index);
                ("response.reasoning_summary_part.added", json!({"type":"summary_text","text":""}))
            } else {
                self.text_index = Some(index);
                ("response.content_part.added", json!({"type":"output_text","text":"","annotations":[]}))
            };
            let mut payload = json!({"type":event,"item_id":id,"output_index":index,"part":part});
            payload[if reasoning { "summary_index" } else { "content_index" }] = json!(0);
            events.push(sse_frame(event, payload));
            self.output[index][if reasoning { "summary" } else { "content" }] = json!([part]);
            index
        });
        append_string(
            &mut self.output[index][if reasoning { "summary" } else { "content" }][0]["text"],
            text,
        );
        let event = if reasoning {
            "response.reasoning_summary_text.delta"
        } else {
            "response.output_text.delta"
        };
        let mut payload = json!({"type":event,"item_id":self.output[index]["id"],"output_index":index,"delta":text});
        payload[if reasoning {
            "summary_index"
        } else {
            "content_index"
        }] = json!(0);
        events.push(sse_frame(event, payload));
        events
    }

    fn open_tool(
        &mut self,
        upstream_index: u64,
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> Option<(usize, Value)> {
        if !self.retain_replacing(
            0,
            call_id
                .len()
                .saturating_add(name.len())
                .saturating_add(arguments.len()),
        ) {
            return None;
        }
        let index = self.output.len();
        let item = json!({"id":format!("fc_{index}_{}",self.response_id),"type":"function_call","call_id":call_id,"name":name,"arguments":"","status":"in_progress"});
        let event = sse_frame(
            "response.output_item.added",
            json!({"type":"response.output_item.added","output_index":index,"item":item}),
        );
        self.output.push(item);
        self.output[index]["arguments"] = json!(arguments);
        self.tool_indices.insert(upstream_index, index);
        Some((index, event))
    }

    fn argument_delta(&mut self, index: usize, delta: &str) -> Option<Value> {
        if !self.retain_replacing(0, delta.len()) {
            return None;
        }
        append_string(&mut self.output[index]["arguments"], delta);
        if self.include_tool_metadata {
            return Some(sse_frame(
                "response.function_call_arguments.delta",
                json!({
                    "type":"response.function_call_arguments.delta","item_id":self.output[index]["id"],
                    "output_index":index,"delta":delta,"name":self.output[index]["name"],
                    "call_id":self.output[index]["call_id"],
                }),
            ));
        }
        Some(sse_frame(
            "response.function_call_arguments.delta",
            json!({
                "type":"response.function_call_arguments.delta","item_id":self.output[index]["id"],"output_index":index,"delta":delta,
            }),
        ))
    }

    /// Bound accumulated text and arguments as well as individual SSE frames.
    /// Repeated complete metadata replaces its previous byte count; actual
    /// name/argument/text fragments grow the request budget exactly once.
    fn retain_replacing(&mut self, removed: usize, added: usize) -> bool {
        let Some(bytes) = self
            .retained_bytes
            .saturating_sub(removed)
            .checked_add(added)
            .filter(|bytes| *bytes <= MAX_RETAINED_BYTES)
        else {
            return false;
        };
        self.retained_bytes = bytes;
        true
    }

    fn upstream_terminal(&mut self) -> Vec<Value> {
        let mut completion =
            Completion::from_reason(self.translation.to, self.finish_reason.as_deref());
        if completion == Completion::Completed
            && self
                .output
                .iter()
                .any(|item| item["type"] == "function_call" && !valid_function_call(item))
        {
            completion = Completion::Failed("invalid_tool_arguments");
        }
        self.terminal(completion)
    }

    fn response(&self, completion: Completion) -> Value {
        let mut response = json!({
            "id":self.response_id,"object":"response","model":self.model,
            "output":self.output,
            "usage":self.usage.clone().unwrap_or_else(|| json!({"input_tokens":0,"output_tokens":0,"total_tokens":0})),
        });
        completion.apply(&mut response);
        response
    }

    fn flush_citations(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        if !self.citations.is_empty() {
            let previous = self.text_index.and_then(|index| self.output[index]["content"][0]["text"].as_str())
                .unwrap_or("").to_owned();
            let mut text = previous.clone();
            let annotations = super::search_compat::citations(&mut text, &json!(std::mem::take(&mut self.citations)));
            events.extend(self.text_delta(&text[previous.len()..], false));
            if self.finished { return events; }
            if let Some(index) = self.text_index {
                self.output[index]["content"][0]["annotations"] = json!(annotations);
                for (ordinal, annotation) in annotations.iter().enumerate() {
                    events.push(sse_frame("response.output_text.annotation.added", json!({"item_id":self.output[index]["id"],
                        "output_index":index,"content_index":0,"annotation_index":ordinal,"annotation":annotation})));
                }
            }
        }
        events
    }

    fn terminal(&mut self, mut completion: Completion) -> Vec<Value> {
        if self.finished { return Vec::new(); }
        let mut events = self.flush_citations();
        if self.finished { return events; }
        if completion == Completion::Completed && self.output.iter().any(|item| item["type"] == "web_search_call" && item["status"] == "in_progress") {
            completion = Completion::Failed("invalid_search_result");
        }
        if self.native_search && self.translation.to == ModelProtocol::ChatCompletions && completion == Completion::Completed {
            if let Some(search) = super::search_compat::chat_result(&self.output, self.usage.as_ref().unwrap_or(&Value::Null), &self.response_id) {
                if !self.retain_replacing(0, search.to_string().len()) {
                    completion = Completion::Failed("invalid_stream_frame");
                } else {
                    events.push(sse_frame("response.output_item.added", json!({"output_index":self.output.len(),"item":search})));
                    self.output.push(search);
                }
            }
        }
        self.finished = true;
        if let Completion::Failed(code) = completion {
            self.failure = Some(code);
        }
        let success = completion == Completion::Completed;
        for (index, item) in self.output.iter_mut().enumerate() {
            if item["type"] == "web_search_call" && item["status"] == "in_progress" && !success {
                item["status"] = json!("incomplete");
            }
            if item.get("status").is_some() && item["type"] != "web_search_call" {
                item["status"] = json!(if success { "completed" } else { "incomplete" });
            }
            // Incomplete/failed tools must not become executable.
            if !success {
                continue;
            }
            let id = &item["id"];
            match item["type"].as_str() {
                Some("function_call") => events.push(sse_frame("response.function_call_arguments.done", json!({"type":"response.function_call_arguments.done","item_id":id,"output_index":index,"arguments":item["arguments"]}))),
                Some("message") => {
                    let part = &item["content"][0];
                    events.push(sse_frame("response.output_text.done", json!({"type":"response.output_text.done","item_id":id,"output_index":index,"content_index":0,"text":part["text"]})));
                    events.push(sse_frame("response.content_part.done", json!({"type":"response.content_part.done","item_id":id,"output_index":index,"content_index":0,"part":part})));
                },
                Some("reasoning") => {
                    let part = &item["summary"][0];
                    events.push(sse_frame("response.reasoning_summary_text.done", json!({"type":"response.reasoning_summary_text.done","item_id":id,"output_index":index,"summary_index":0,"text":part["text"]})));
                    events.push(sse_frame("response.reasoning_summary_part.done", json!({"type":"response.reasoning_summary_part.done","item_id":id,"output_index":index,"summary_index":0,"part":part})));
                },
                _ => {},
            }
            events.push(sse_frame(
                "response.output_item.done",
                json!({"type":"response.output_item.done","output_index":index,"item":item}),
            ));
        }
        let response = self.response(completion);
        if success {
            self.completed_response = Some(response.clone());
        }
        let event = format!("response.{}", completion.status());
        events.push(sse_frame(&event, json!({"type":event,"response":response})));
        events
    }
}

fn append_string(target: &mut Value, text: &str) {
    if let Value::String(value) = target {
        value.push_str(text);
    }
}

/// The new Chat encoder has no representation for hosted blocks or private
/// signatures. Gate only that path: existing Responses/Message translations
/// and byte-preserving same-protocol forwarding keep their current behavior.
fn unsupported_chat_messages_event(kind: Option<&str>, value: &Value) -> bool {
    let private = |value: &Value| {
        value.as_object().is_some_and(|object| {
            ["signature", "encrypted_content", "redacted_thinking"]
                .iter()
                .any(|key| object.get(*key).is_some_and(|value| !value.is_null()))
        })
    };
    if private(value)
        || private(&value["message"])
        || private(&value["content_block"])
        || private(&value["delta"])
    {
        return true;
    }
    match kind {
        Some("content_block_start") => !matches!(
            value["content_block"]["type"].as_str(),
            Some("text" | "thinking" | "tool_use")
        ),
        Some("content_block_delta") => !matches!(
            value["delta"]["type"].as_str(),
            Some("text_delta" | "thinking_delta" | "input_json_delta")
        ),
        // A standard message_start carries no body content. The old decoder
        // cannot safely reinterpret a provider's complete-message extension.
        Some("message_start") => value["message"]["content"]
            .as_array()
            .is_some_and(|content| !content.is_empty()),
        _ => false,
    }
}

fn sse_frame(event: &str, mut data: Value) -> Value {
    data["type"] = json!(event);
    data
}

fn encode_event(event: CanonicalEvent) -> String {
    let name = event.payload["type"].as_str().unwrap_or("error");
    format!("event: {name}\ndata: {}\n\n", event.payload)
}

pub(super) fn find_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    for (index, byte) in buffer.iter().enumerate() {
        if *byte == b'\n' {
            if buffer.get(index + 1) == Some(&b'\n') {
                return Some((index, index + 2));
            }
            if buffer.get(index + 1) == Some(&b'\r') && buffer.get(index + 2) == Some(&b'\n') {
                return Some((index, index + 3));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translator(to: ModelProtocol) -> StreamTranslator {
        StreamTranslator::new(Translation {
            from: ModelProtocol::Responses,
            to,
        })
    }
    fn feed(t: &mut StreamTranslator, value: Value) -> Vec<String> {
        t.push(format!("data: {value}\n\n").as_bytes())
    }
    fn values(events: &[String]) -> Vec<Value> {
        events
            .iter()
            .map(|frame| {
                serde_json::from_str(
                    frame
                        .lines()
                        .find_map(|line| line.strip_prefix("data: "))
                        .unwrap(),
                )
                .unwrap()
            })
            .collect()
    }
    fn terminal_response(events: &[String]) -> Value {
        values(events)
            .into_iter()
            .rev()
            .find_map(|event| {
                event
                    .get("response")
                    .filter(|r| r["status"] != "in_progress")
                    .cloned()
            })
            .unwrap()
    }
    fn end(t: &mut StreamTranslator, reason: &str) -> Vec<String> {
        if t.translation.to == ModelProtocol::ChatCompletions {
            let mut events = feed(t, json!({"choices":[{"delta":{},"finish_reason":reason}]}));
            events.extend(t.push(b"data: [DONE]\n\n"));
            events
        } else {
            let mut events = feed(
                t,
                json!({"type":"message_delta","delta":{"stop_reason":reason},"usage":{"output_tokens":7}}),
            );
            events.extend(feed(t, json!({"type":"message_stop"})));
            events
        }
    }
    fn text(t: &mut StreamTranslator) -> Vec<String> {
        if t.translation.to == ModelProtocol::ChatCompletions {
            feed(
                t,
                json!({"model":"m","choices":[{"delta":{"content":"Hello"}}]}),
            )
        } else {
            feed(
                t,
                json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":11,"output_tokens":0}}}),
            );
            feed(
                t,
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}),
            )
        }
    }
    fn tool(t: &mut StreamTranslator, arguments: &str) -> Vec<String> {
        if t.translation.to == ModelProtocol::ChatCompletions {
            feed(
                t,
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"lookup","arguments":arguments}}]}}]}),
            )
        } else {
            let mut events = feed(
                t,
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_1","name":"lookup","input":{}}}),
            );
            events.extend(feed(t, json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":arguments}})));
            events
        }
    }

    #[test]
    fn chat_waits_for_done_and_keeps_late_usage_and_mixed_output() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        let mut events = text(&mut t);
        events.extend(tool(&mut t, r#"{"q":"x"}"#));
        let before_done = feed(
            &mut t,
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        );
        assert!(!before_done.join("").contains(".done"));
        assert!(t.completed_response().is_none());
        events.extend(before_done);
        events.extend(feed(&mut t, json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}})));
        events.extend(t.push(b"data: [DONE]\n\n"));
        let response = terminal_response(&events);
        assert_eq!(response["status"], "completed");
        assert_eq!(response["usage"]["total_tokens"], 8);
        assert_eq!(response["output"][0]["content"][0]["text"], "Hello");
        assert_eq!(response["output"][1]["name"], "lookup");
        assert_eq!(t.completed_response(), Some(&response));
        assert!(t.finish().is_empty());
    }

    #[test]
    fn anthropic_normal_stop_preserves_usage_and_full_output() {
        let mut t = translator(ModelProtocol::AnthropicMessages);
        let mut events = text(&mut t);
        events.extend(tool(&mut t, "{}"));
        events.extend(end(&mut t, "tool_use"));
        let response = terminal_response(&events);
        assert_eq!(response["status"], "completed");
        assert_eq!(response["usage"]["input_tokens"], 11);
        assert_eq!(response["usage"]["output_tokens"], 7);
        assert_eq!(response["output"].as_array().unwrap().len(), 2);
        let event_types: Vec<Value> = values(&events)
            .into_iter()
            .map(|event| event["type"].clone())
            .collect();
        assert!(event_types.contains(&json!("response.function_call_arguments.done")));
    }

    #[test]
    fn normal_and_limited_streams_in_both_protocols_have_correct_terminal_state() {
        for (protocol, reason, status, detail) in [
            (ModelProtocol::ChatCompletions, "stop", "completed", None),
            (
                ModelProtocol::AnthropicMessages,
                "end_turn",
                "completed",
                None,
            ),
            (
                ModelProtocol::ChatCompletions,
                "length",
                "incomplete",
                Some("max_output_tokens"),
            ),
            (
                ModelProtocol::ChatCompletions,
                "content_filter",
                "incomplete",
                Some("content_filter"),
            ),
            (
                ModelProtocol::AnthropicMessages,
                "max_tokens",
                "incomplete",
                Some("max_output_tokens"),
            ),
            (
                ModelProtocol::AnthropicMessages,
                "refusal",
                "incomplete",
                Some("content_filter"),
            ),
        ] {
            let mut t = translator(protocol);
            let mut events = text(&mut t);
            events.extend(end(&mut t, reason));
            let response = terminal_response(&events);
            assert_eq!(response["status"], status);
            assert_eq!(response["incomplete_details"]["reason"].as_str(), detail);
            assert_eq!(t.completed_response().is_some(), status == "completed");
        }
    }

    #[test]
    fn truncated_streams_fail_without_completing_tools_in_either_protocol() {
        for protocol in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let mut t = translator(protocol);
            let mut events = tool(&mut t, "{");
            events.extend(t.finish());
            assert_eq!(terminal_response(&events)["status"], "failed");
            assert!(!events
                .join("")
                .contains("response.function_call_arguments.done"));
            assert!(t.completed_response().is_none());
            assert!(t.finish().is_empty());
        }
    }

    #[test]
    fn empty_legacy_call_placeholder_does_not_reject_valid_chat_tools() {
        let arguments = r#"{"system2_search_query":[{"q":"Rust","recency":7,"domains":null}]}"#;
        let mut t = translator(ModelProtocol::ChatCompletions);
        let mut events = tool(&mut t, "");
        for part in ["{", &arguments[1..arguments.len()-1], "}"] {
            events.extend(feed(&mut t, json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"name":"","arguments":part}}]},"finish_reason":""}]})));
        }
        events.extend(feed(&mut t, json!({"choices":[{"delta":{"tool_calls":[],
            "function_call":{"name":"","arguments":""}},"finish_reason":"tool_calls"}]})));
        events.extend(t.push(b"data: [DONE]\n\n"));
        assert_eq!(terminal_response(&events)["status"], "completed");
        assert_eq!(t.completed_response().unwrap()["output"][0]["arguments"], arguments);

        // A real legacy call or malformed modern arguments must still fail.
        for placeholder in [json!({"name":"legacy","arguments":"{}"}),json!({"name":"","arguments":"{}"}),json!({"unexpected":true})] {
            let mut t = translator(ModelProtocol::ChatCompletions);
            tool(&mut t, "{}");
            let events = feed(&mut t, json!({"choices":[{"delta":{"function_call":placeholder},"finish_reason":"tool_calls"}]}));
            assert_eq!(terminal_response(&events)["error"]["reason"], "invalid_tool_arguments");
        }
        let mut t = translator(ModelProtocol::ChatCompletions);
        tool(&mut t, "{");
        feed(&mut t, json!({"choices":[{"delta":{"function_call":{"name":"","arguments":""}},"finish_reason":"tool_calls"}]}));
        assert_eq!(terminal_response(&t.push(b"data: [DONE]\n\n"))["error"]["reason"], "invalid_tool_arguments");
    }

    #[test]
    fn invalid_tool_json_fails_but_length_limited_json_remains_incomplete() {
        for (protocol, success_reason, limit_reason) in [
            (ModelProtocol::ChatCompletions, "tool_calls", "length"),
            (ModelProtocol::AnthropicMessages, "tool_use", "max_tokens"),
        ] {
            for (reason, status) in [(success_reason, "failed"), (limit_reason, "incomplete")] {
                let mut t = translator(protocol);
                let mut events = tool(&mut t, "{");
                events.extend(end(&mut t, reason));
                assert_eq!(terminal_response(&events)["status"], status);
                assert!(!events
                    .join("")
                    .contains("response.function_call_arguments.done"));
                assert!(t.completed_response().is_none());
            }
        }
    }

    #[test]
    fn eof_after_finish_reason_still_requires_transport_terminator() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        tool(&mut t, "{}");
        feed(
            &mut t,
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        );
        assert_eq!(terminal_response(&t.finish())["status"], "failed");
    }

    #[test]
    fn stream_markers_without_finish_reason_are_not_success() {
        for protocol in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let mut t = translator(protocol);
            text(&mut t);
            let events = if protocol == ModelProtocol::ChatCompletions {
                t.push(b"data: [DONE]\n\n")
            } else {
                feed(&mut t, json!({"type":"message_stop"}))
            };
            assert_eq!(
                terminal_response(&events)["error"]["reason"],
                "missing_finish_reason"
            );
        }
    }

    #[test]
    fn split_utf8_and_crlf_frames_are_buffered() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        let frame = format!(
            "data: {}\r\n\r\n",
            json!({"choices":[{"delta":{"content":"中文"}}]})
        );
        let mut events = Vec::new();
        for byte in frame.as_bytes() {
            events.extend(t.push(&[*byte]));
        }
        events.extend(end(&mut t, "stop"));
        assert_eq!(
            terminal_response(&events)["output"][0]["content"][0]["text"],
            "中文"
        );
    }

    #[test]
    fn invalid_frame_fails_once_and_created_precedes_earlier_deltas() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        let frame = format!(
            "data: {}\n\ndata: {{bad json\n\n",
            json!({"choices":[{"delta":{"content":"x"}}]})
        );
        let events = t.push(frame.as_bytes());
        assert_eq!(values(&events)[0]["type"], "response.created");
        assert_eq!(terminal_response(&events)["status"], "failed");
        assert!(t.push(b"data: [DONE]\n\n").is_empty());
    }

    #[test]
    fn output_indices_stay_stable_when_text_follows_tool() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        let mut events = tool(&mut t, "{}");
        events.extend(text(&mut t));
        events.extend(end(&mut t, "tool_calls"));
        for event in values(&events) {
            if event["type"]
                .as_str()
                .unwrap()
                .starts_with("response.function_call_arguments.")
            {
                assert_eq!(event["output_index"], 0);
            }
        }
        assert_eq!(
            t.completed_response().unwrap()["output"][0]["type"],
            "function_call"
        );
        assert_eq!(
            t.completed_response().unwrap()["output"][1]["type"],
            "message"
        );
    }

    #[test]
    fn anthropic_tool_deltas_match_block_indices() {
        let mut t = translator(ModelProtocol::AnthropicMessages);
        for index in [0, 2] {
            feed(
                &mut t,
                json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":format!("call_{index}"),"name":"lookup","input":{}}}),
            );
        }
        for index in [2, 0] {
            feed(
                &mut t,
                json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":format!("{{\"n\":{index}}}")}}),
            );
        }
        end(&mut t, "tool_use");
        let response = t.completed_response().unwrap();
        assert_eq!(response["output"][0]["arguments"], "{\"n\":0}");
        assert_eq!(response["output"][1]["arguments"], "{\"n\":2}");
    }

    #[test]
    fn identical_tool_name_fragments_are_not_dropped() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        feed(
            &mut t,
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"test","arguments":""}}]}}]}),
        );
        feed(
            &mut t,
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"test","arguments":"{}"}}]}}]}),
        );
        end(&mut t, "tool_calls");
        assert_eq!(
            t.completed_response().unwrap()["output"][0]["name"],
            "testtest"
        );
    }

    #[test]
    fn repeated_tool_call_headers_do_not_duplicate_the_name() {
        let mut t = translator(ModelProtocol::ChatCompletions);
        for arguments in ["{\"q\":", "1}"] {
            feed(
                &mut t,
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"lookup","arguments":arguments}}]}}]}),
            );
        }
        end(&mut t, "tool_calls");
        let response = t.completed_response().unwrap();
        assert_eq!(response["output"][0]["name"], "lookup");
        assert_eq!(response["output"][0]["arguments"], "{\"q\":1}");
    }
}
