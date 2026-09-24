//! Streaming translation from an upstream protocol back into Responses events.
//!
//! Codex parses `text/event-stream` frames of the Responses API, so a translated
//! upstream stream must emit that event sequence — not just text. The sequence is
//! ordered and stateful: `response.created`, then item lifecycle events, then
//! deltas, then a terminal `response.completed` carrying usage. Events reference
//! ids and an output index chosen here, so the translator keeps per-stream state
//! and is fed one chunk at a time because HTTP chunks and SSE frames do not align.

use crate::core::protocol::Translation;
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

/// Buffers an upstream byte stream and emits Responses SSE frames.
pub struct StreamTranslator {
    translation: Translation,
    /// Bytes received but not yet forming a complete SSE frame.
    pending: Vec<u8>,
    state: StreamState,
}

struct StreamState {
    response_id: String,
    created: bool,
    text_started: bool,
    text: String,
    /// Reasoning is emitted as its own item ahead of the message.
    reasoning_started: bool,
    reasoning: String,
    /// Tool calls accumulate until the upstream finishes sending them.
    tool_calls: Vec<ToolCall>,
    tools_closed: bool,
    usage: Option<Value>,
    finished: bool,
    model: String,
}

impl StreamState {
    /// Reasoning occupies the first output slot when it was emitted at all.
    fn reasoning_index(&self) -> u64 {
        0
    }

    /// The assistant message follows the reasoning item when there is one.
    fn message_index(&self) -> u64 {
        u64::from(self.reasoning_started)
    }

    /// Tool calls follow both, in the order the upstream announced them.
    fn function_index(&self, offset: usize) -> u64 {
        self.message_index() + u64::from(self.text_started) + offset as u64
    }
}

#[derive(Default, Clone)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
    /// Whether a Responses `function_call` item has been opened for it.
    opened: bool,
}

impl StreamTranslator {
    pub fn new(translation: Translation) -> Self {
        Self {
            translation,
            pending: Vec::new(),
            state: StreamState {
                response_id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
                created: false,
                text_started: false,
                text: String::new(),
                reasoning_started: false,
                reasoning: String::new(),
                tool_calls: Vec::new(),
                tools_closed: false,
                usage: None,
                finished: false,
                model: String::new(),
            },
        }
    }

    /// Feed bytes and return whatever complete SSE frames they produced.
    ///
    /// Incomplete trailing bytes are retained, so callers can pass arbitrary
    /// chunk boundaries.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        // Frames are separated by a blank line; CRLF is normalised first.
        while let Some(split) = find_separator(&self.pending) {
            let frame: Vec<u8> = self.pending.drain(..split.0).collect();
            self.pending.drain(..split.1 - split.0);
            let text = String::from_utf8_lossy(&frame).into_owned();
            events.extend(self.handle_frame(&text));
        }
        self.prepend_created(events)
    }

    /// The response id Codex will quote back as `previous_response_id`.
    pub fn response_id(&self) -> &str {
        &self.state.response_id
    }

    /// The tool calls this turn produced, in the shape a follow-up replays.
    ///
    /// Returns them in Responses `function_call` item form so the next request
    /// can restate the assistant turn before its tool results.
    pub fn completed_tool_calls(&self) -> Vec<Value> {
        self.state
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": { "name": call.name, "arguments": call.arguments },
                })
            })
            .collect()
    }

    /// Emit any closing events when the upstream stream ends without one.
    pub fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if !self.state.finished {
            events.extend(self.close_tool_calls());
            events.extend(self.completed_event(None));
        }
        self.prepend_created(events)
    }

    fn prepend_created(&mut self, mut events: Vec<String>) -> Vec<String> {
        if !self.state.created && !events.is_empty() {
            self.state.created = true;
            events.insert(
                0,
                sse_frame(
                    "response.created",
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": self.state.response_id,
                            "object": "response",
                            "status": "in_progress",
                            "model": self.state.model,
                            "output": [],
                        },
                    }),
                ),
            );
        }
        events
    }

    fn handle_frame(&mut self, frame: &str) -> Vec<String> {
        let mut data: Option<String> = None;
        let mut event_name: Option<&str> = None;
        for line in frame.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("event:") {
                event_name = Some(rest.trim());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.trim_start();
                data = Some(match data {
                    Some(mut existing) => {
                        existing.push('\n');
                        existing.push_str(rest);
                        existing
                    }
                    None => rest.to_owned(),
                });
            }
        }
        let Some(data) = data else {
            return Vec::new();
        };
        if data.trim() == "[DONE]" {
            let mut events = self.close_tool_calls();
            events.extend(self.completed_event(None));
            return events;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return Vec::new();
        };
        // Anthropic names the event in a field; OpenAI puts `type` in the JSON.
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| event_name.map(str::to_owned));
        match self.translation.to {
            ModelProtocol::ChatCompletions => self.chat_chunk(kind.as_deref(), &value),
            ModelProtocol::AnthropicMessages => self.anthropic_chunk(kind.as_deref(), &value),
            _ => Vec::new(),
        }
    }

    /// Translate one Chat Completions chunk (`choices[].delta`) into events.
    fn chat_chunk(&mut self, kind: Option<&str>, value: &Value) -> Vec<String> {
        let mut events = Vec::new();
        if let Some(model) = value.get("model").and_then(Value::as_str) {
            self.state.model = model.to_owned();
        }
        if let Some(usage) = value.get("usage").filter(|u| !u.is_null()) {
            self.state.usage = Some(openai_usage_to_responses(usage));
        }
        // Some relays still send an error object inside the stream.
        if let Some(error) = value.get("error") {
            events.push(open_error_event(&self.state.response_id, error));
            self.state.finished = true;
            return events;
        }
        if kind == Some("chat.completion.chunk") || value.get("choices").is_some() {
            let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
                return events;
            };
            let delta = choice.get("delta").cloned().unwrap_or(json!({}));
            // `reasoning_content` is a widely used relay extension; thinking
            // always precedes the visible answer.
            if let Some(reasoning) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                events.extend(self.reasoning_delta(reasoning));
            }
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    events.extend(self.text_delta(text));
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                events.extend(self.chat_tool_deltas(calls));
            }
            let finish = choice.get("finish_reason").and_then(Value::as_str);
            if finish.is_some() {
                events.extend(self.close_tool_calls());
            }
        }
        events
    }

    /// Translate one Anthropic stream event (`content_block_delta`, …).
    fn anthropic_chunk(&mut self, kind: Option<&str>, value: &Value) -> Vec<String> {
        let mut events = Vec::new();
        if let Some(model) = value
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(Value::as_str)
        {
            self.state.model = model.to_owned();
        }
        match kind {
            Some("message_start") => {
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    self.state.usage = Some(json!({
                        "input_tokens": input,
                        "output_tokens": output,
                        "total_tokens": input + output,
                    }));
                }
            }
            Some("content_block_delta") => {
                let delta = value.get("delta").cloned().unwrap_or(json!({}));
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            events.extend(self.text_delta(text));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            events.extend(self.reasoning_delta(text));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                            if let Some(call) = self.state.tool_calls.last_mut() {
                                call.arguments.push_str(partial);
                                let index = self.state.tool_calls.len() - 1;
                                events.push(self.argument_delta(index, partial));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Anthropic announces a tool before streaming its arguments.
            Some("content_block_start") => {
                if let Some(block) = value.get("content_block") {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        self.state.tool_calls.push(ToolCall {
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            name: block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            arguments: String::new(),
                            opened: false,
                        });
                    }
                }
            }
            Some("message_delta") => {
                if let Some(usage) = value.get("usage") {
                    let mut merged = self.state.usage.clone().unwrap_or_else(|| json!({}));
                    if let (Some(target), Some(output)) = (
                        merged.as_object_mut(),
                        usage.get("output_tokens").and_then(Value::as_u64),
                    ) {
                        target.insert("output_tokens".into(), json!(output));
                        let input = target
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        target.insert("total_tokens".into(), json!(input + output));
                    }
                    self.state.usage = Some(merged);
                }
            }
            Some("message_stop") | Some("message_end") => {
                events.extend(self.close_tool_calls());
                events.extend(self.completed_event(None));
            }
            Some("error") => {
                events.push(open_error_event(&self.state.response_id, value));
                self.state.finished = true;
            }
            _ => {}
        }
        events
    }

    /// Emit the message-item opening plus a text delta.
    fn text_delta(&mut self, text: &str) -> Vec<String> {
        self.state.text.push_str(text);
        let mut events = Vec::new();
        if !self.state.text_started {
            events.extend(self.open_message_item());
        }
        events.push(sse_frame(
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": self.item_id(),
                "output_index": self.state.message_index(),
                "content_index": 0,
                "delta": text,
            }),
        ));
        events
    }

    /// Emit a reasoning-summary delta for the model's thinking.
    ///
    /// Codex renders these separately from the answer text, and both upstreams
    /// stream reasoning before it, so the item is opened on first use.
    fn reasoning_delta(&mut self, text: &str) -> Vec<String> {
        self.state.reasoning.push_str(text);
        let mut events = Vec::new();
        if !self.state.reasoning_started {
            self.state.reasoning_started = true;
            events.push(sse_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": self.state.reasoning_index(),
                    "item": {
                        "id": self.reasoning_item_id(),
                        "type": "reasoning",
                        "summary": [],
                    },
                }),
            ));
            events.push(sse_frame(
                "response.reasoning_summary_part.added",
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "item_id": self.reasoning_item_id(),
                    "output_index": self.state.reasoning_index(),
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": "" },
                }),
            ));
        }
        events.push(sse_frame(
            "response.reasoning_summary_text.delta",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": self.reasoning_item_id(),
                "output_index": self.state.reasoning_index(),
                "summary_index": 0,
                "delta": text,
            }),
        ));
        events
    }

    /// Chat completions streams tool calls as indexed argument fragments.
    fn chat_tool_deltas(&mut self, calls: &[Value]) -> Vec<String> {
        let mut events = Vec::new();
        for call in calls {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if index >= 128 {
                continue;
            }
            while self.state.tool_calls.len() <= index {
                self.state.tool_calls.push(ToolCall::default());
            }
            let entry = &mut self.state.tool_calls[index];
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.id = id.to_owned();
            }
            let mut argument_delta = None;
            if let Some(function) = call.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    entry.name.push_str(name);
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    entry.arguments.push_str(arguments);
                    argument_delta = Some(arguments);
                }
            }
            if !entry.opened && !entry.name.is_empty() {
                entry.opened = true;
                events.push(self.open_function_item(index));
            }
            if let Some(arguments) = argument_delta {
                if !arguments.is_empty() && self.state.tool_calls[index].opened {
                    events.push(self.argument_delta(index, arguments));
                }
            }
        }
        events
    }

    fn close_tool_calls(&mut self) -> Vec<String> {
        if self.state.tools_closed {
            return Vec::new();
        }
        self.state.tools_closed = true;
        let mut events = Vec::new();
        for index in 0..self.state.tool_calls.len() {
            let call = self.state.tool_calls[index].clone();
            if !call.opened {
                events.push(self.open_function_item(index));
            }
            events.push(sse_frame(
                "response.function_call_arguments.done",
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": self.tool_item_id(index),
                    "output_index": self.state.function_index(index),
                    "arguments": call.arguments,
                }),
            ));
            events.push(sse_frame(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": self.state.function_index(index),
                    "item": self.function_item(index),
                }),
            ));
        }
        events
    }

    fn argument_delta(&self, index: usize, delta: &str) -> String {
        sse_frame(
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": self.tool_item_id(index),
                "output_index": self.state.function_index(index),
                "delta": delta,
            }),
        )
    }

    fn function_item(&self, index: usize) -> Value {
        let call = &self.state.tool_calls[index];
        json!({
            "id": self.tool_item_id(index),
            "type": "function_call",
            "call_id": call.id,
            "name": call.name,
            "arguments": call.arguments,
            "status": "completed",
        })
    }

    fn open_message_item(&mut self) -> Vec<String> {
        self.state.text_started = true;
        let item = json!({
            "id": self.item_id(),
            "type": "message",
            "role": "assistant",
            "status": "in_progress",
            "content": [],
        });
        vec![
            sse_frame(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": self.state.message_index(),
                    "item": item,
                }),
            ),
            sse_frame(
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": self.item_id(),
                    "output_index": self.state.message_index(),
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "", "annotations": [] },
                }),
            ),
        ]
    }

    fn open_function_item(&self, index: usize) -> String {
        let call = &self.state.tool_calls[index];
        sse_frame(
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": self.state.function_index(index),
                "item": {
                    "id": self.tool_item_id(index),
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.name,
                    "arguments": "",
                    "status": "in_progress",
                },
            }),
        )
    }

    fn completed_event(&mut self, _unused: Option<()>) -> Vec<String> {
        if self.state.finished {
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut output = Vec::new();
        if self.state.reasoning_started {
            let item = json!({
                "id": self.reasoning_item_id(),
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": self.state.reasoning }],
            });
            events.push(sse_frame(
                "response.reasoning_summary_text.done",
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "item_id": self.reasoning_item_id(),
                    "output_index": self.state.reasoning_index(),
                    "summary_index": 0,
                    "text": self.state.reasoning,
                }),
            ));
            events.push(sse_frame(
                "response.output_item.done",
                json!({ "type": "response.output_item.done", "output_index": self.state.reasoning_index(), "item": item }),
            ));
            output.push(item);
        }
        if self.state.text_started {
            events.push(sse_frame(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "item_id": self.item_id(),
                    "output_index": self.state.message_index(),
                    "content_index": 0,
                    "text": self.state.text,
                }),
            ));
            events.push(sse_frame(
                "response.content_part.done",
                json!({
                    "type": "response.content_part.done",
                    "item_id": self.item_id(),
                    "output_index": self.state.message_index(),
                    "content_index": 0,
                    "part": { "type": "output_text", "text": self.state.text, "annotations": [] },
                }),
            ));
            let item = json!({
                "id": self.item_id(),
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{ "type": "output_text", "text": self.state.text, "annotations": [] }],
            });
            events.push(sse_frame(
                "response.output_item.done",
                json!({ "type": "response.output_item.done", "output_index": self.state.message_index(), "item": item }),
            ));
            output.push(item);
        }
        output.extend((0..self.state.tool_calls.len()).map(|index| self.function_item(index)));
        events.push(sse_frame(
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {
                    "id": self.state.response_id,
                    "object": "response",
                    "status": "completed",
                    "model": self.state.model,
                    "output": output,
                    "usage": self.state.usage.clone().unwrap_or_else(|| json!({
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "total_tokens": 0,
                    })),
                },
            }),
        ));
        self.state.finished = true;
        events
    }

    fn item_id(&self) -> String {
        format!("msg_{}", self.state.response_id.trim_start_matches("resp_"))
    }

    fn reasoning_item_id(&self) -> String {
        format!("rs_{}", self.state.response_id.trim_start_matches("resp_"))
    }

    fn tool_item_id(&self, index: usize) -> String {
        format!(
            "fc_{index}_{}",
            self.state.response_id.trim_start_matches("resp_")
        )
    }
}

fn open_error_event(response_id: &str, error: &Value) -> String {
    sse_frame(
        "response.failed",
        json!({
            "type": "response.failed",
            "response": {
                "id": response_id,
                "object": "response",
                "status": "failed",
                "error": {
                    "code": error.get("code").and_then(Value::as_str).unwrap_or("upstream_error"),
                    "message": error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("upstream returned an error"),
                },
            },
        }),
    )
}

/// Chat Completions reports prompt/completion; Responses names them input/output.
fn openai_usage_to_responses(usage: &Value) -> Value {
    let input = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input + output);
    json!({
        "input_tokens": input,
        "output_tokens": output,
        "total_tokens": total,
    })
}

/// Wrap a payload as one SSE frame, terminated by the blank line Codex expects.
fn sse_frame(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Find the end of the first complete SSE frame in `buffer`.
///
/// Returns `(frame_end, next_start)` so CRLF and LF separators both work.
fn find_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0;
    while index < buffer.len() {
        if buffer[index] == b'\n' {
            // A blank line ends the frame; tolerate a preceding CR.
            let next = index + 1;
            if buffer.get(next) == Some(&b'\n') {
                return Some((index, next + 1));
            }
            if buffer.get(next) == Some(&b'\r') && buffer.get(next + 1) == Some(&b'\n') {
                return Some((index, next + 2));
            }
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::Translation;

    fn translator(to: ModelProtocol) -> StreamTranslator {
        StreamTranslator::new(Translation {
            from: ModelProtocol::Responses,
            to,
        })
    }

    fn joined(events: &[String]) -> String {
        events.join("")
    }

    #[test]
    fn chat_stream_becomes_text_delta_and_completion() {
        let mut translator = translator(ModelProtocol::ChatCompletions);
        let mut events = translator.push(
            b"data: {\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"He\"}}]}\n\n",
        );
        events.extend(translator.push(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\ndata: [DONE]\n\n",
        ));
        let text = joined(&events);

        assert!(text.contains("response.output_item.added"));
        assert!(text.contains("response.content_part.added"));
        assert!(text.contains("\"delta\": \"He\"") || text.contains("\"delta\":\"He\""));
        assert!(text.contains("response.completed"));
        assert!(text.contains("\"output_tokens\": 2") || text.contains("\"output_tokens\":2"));
    }

    #[test]
    fn split_chunks_are_buffered_until_a_frame_is_complete() {
        let mut translator = translator(ModelProtocol::ChatCompletions);
        // A frame delivered in three arbitrary pieces must still parse once.
        assert!(translator
            .push(b"data: {\"choices\":[{\"delta\":{\"con")
            .is_empty());
        assert!(translator.push(b"tent\":\"hi\"}}]}").is_empty());
        let events = translator.push(b"\n\n");
        assert!(joined(&events).contains("hi"));
    }

    #[test]
    fn anthropic_stream_maps_text_and_message_stop() {
        let mut translator = translator(ModelProtocol::AnthropicMessages);
        let mut events = translator.push(
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-x\",\"usage\":{\"input_tokens\":11,\"output_tokens\":0}}}\n\n",
        );
        events.extend(translator.push(
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
        ));
        events.extend(translator.push(
            b"event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
        ));
        events
            .extend(translator.push(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
        let text = joined(&events);
        assert!(text.contains("\"text\":\"Hi\"") || text.contains("\"text\": \"Hi\""));
        assert!(text.contains("response.completed"));
        assert!(text.contains("7"), "usage output tokens must survive");
        assert!(text.contains("\"input_tokens\":11") || text.contains("\"input_tokens\": 11"));
    }

    #[test]
    fn crlf_frames_are_recognised() {
        let mut translator = translator(ModelProtocol::ChatCompletions);
        let events =
            translator.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\r\n\r\n");
        assert!(joined(&events).contains("x"));
    }

    #[test]
    fn finish_emits_a_terminal_event_when_upstream_stops_silently() {
        let mut translator = translator(ModelProtocol::ChatCompletions);
        let mut events =
            translator.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n");
        events.extend(translator.finish());
        assert!(joined(&events).contains("response.completed"));
    }

    #[test]
    fn malformed_json_frames_are_skipped_without_panicking() {
        let mut translator = translator(ModelProtocol::ChatCompletions);
        assert!(translator.push(b"data: {not json\n\n").is_empty());
        // The stream must still be usable afterwards.
        let events = translator.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n");
        assert!(joined(&events).contains("ok"));
    }

    #[test]
    fn anthropic_tool_stream_completes_and_keeps_calls_for_follow_up() {
        let mut translator = translator(ModelProtocol::AnthropicMessages);
        let mut events = translator.push(b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"lookup\"}}\n\n");
        events.extend(translator.push(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\\\"x\\\"}\"}}\n\n"));
        events
            .extend(translator.push(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
        let text = joined(&events);
        assert!(text.contains("response.created"));
        assert!(text.contains("response.function_call_arguments.done"));
        assert!(text.contains("response.output_item.done"));
        assert!(text.contains("response.completed"));
        assert_eq!(translator.completed_tool_calls().len(), 1);
        assert_eq!(
            translator.completed_tool_calls()[0]["function"]["name"],
            "lookup"
        );
    }
}
