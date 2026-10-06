//! Responses or Messages streams encoded as ordinary Chat chunks. Tool headers
//! and argument deltas keep stable indices, and EOF never invents completion.
use super::{canonical::MAX_RETAINED_BYTES, chat_ingress, sse, ModelProtocol, Translation};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Default)]
struct Tool {
    index: usize,
    id: String,
    name: String,
    arguments: String,
    emitted: usize,
    opened: bool,
}

pub(super) struct StreamTranslator {
    messages: Option<sse::StreamTranslator>,
    pending: Vec<u8>,
    id: String,
    model: String,
    created: u64,
    started: bool,
    finished: bool,
    failure: Option<&'static str>,
    include_usage: bool,
    text: BTreeMap<(u64, u64, u8), String>,
    tools: BTreeMap<u64, Tool>,
    retained_bytes: usize,
    completed_response: Option<Value>,
}

impl StreamTranslator {
    pub fn new(upstream: ModelProtocol, include_usage: bool) -> Self {
        let id = format!("chatcmpl_{}", uuid::Uuid::new_v4().simple());
        let retained_bytes = id.len();
        Self {
            messages: (upstream == ModelProtocol::AnthropicMessages).then(|| {
                sse::StreamTranslator::new(Translation {
                    from: ModelProtocol::ChatCompletions,
                    to: upstream,
                })
                .with_tool_metadata()
            }),
            pending: Vec::new(),
            id,
            model: String::new(),
            created: 0,
            started: false,
            finished: false,
            failure: None,
            include_usage,
            text: BTreeMap::new(),
            tools: BTreeMap::new(),
            retained_bytes,
            completed_response: None,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        if let Some(messages) = self.messages.as_mut() {
            let events = messages.push_events(chunk);
            let mut out = Vec::new();
            for event in events {
                let result = self.value(&event.payload, &mut out);
                if let Err(code) = result {
                    out.extend(self.fail(code));
                }
                if self.finished {
                    break;
                }
            }
            return out;
        }
        self.pending.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, next)) = sse::find_separator(&self.pending) {
            if end > MAX_BYTES {
                out.extend(self.fail("invalid_stream_frame"));
                break;
            }
            let bytes = self.pending.drain(..next).take(end).collect::<Vec<_>>();
            let result = std::str::from_utf8(&bytes)
                .map_err(|_| "invalid_stream_frame")
                .and_then(|frame| self.frame(frame, &mut out));
            if let Err(code) = result {
                out.extend(self.fail(code));
            }
            if self.finished {
                self.pending.clear();
                break;
            }
        }
        if self.pending.len() > MAX_BYTES {
            out.extend(self.fail("invalid_stream_frame"));
        }
        out
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.fail("upstream_stream_truncated")
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    pub fn failure(&self) -> Option<&'static str> {
        self.failure
    }
    pub fn completed_response(&self) -> Option<&Value> {
        self.completed_response.as_ref()
    }

    pub fn fail(&mut self, code: &'static str) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.failure = Some(code);
        self.pending.clear();
        vec![format!(
            "data: {}\n\n",
            json!({"error":{"type":"api_error","code":code,"message":code}})
        )]
    }

    fn frame(&mut self, frame: &str, out: &mut Vec<String>) -> Result<()> {
        let mut data = Vec::new();
        let mut kind = None;
        for line in frame.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                kind = Some(value.trim());
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.trim_start());
            }
        }
        if data.is_empty() {
            return Ok(());
        }
        let data = data.join("\n");
        if data.trim() == "[DONE]" {
            return Err("invalid_stream_frame");
        }
        let mut value: Value = serde_json::from_str(&data).map_err(|_| "invalid_stream_frame")?;
        if value.get("type").is_none() {
            if let Some(kind) = kind {
                value["type"] = json!(kind);
            }
        }
        self.value(&value, out)
    }

    fn value(&mut self, value: &Value, out: &mut Vec<String>) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        if !matches!(
            value["type"].as_str(),
            Some("response.completed" | "response.incomplete")
        ) && chat_ingress::has_private_output(value)
        {
            return Err("unsupported_content");
        }
        if value.get("error").is_some_and(|error| !error.is_null())
            && value["type"] != "response.failed"
        {
            return Err("upstream_error");
        }
        match value["type"].as_str() {
            Some("response.created" | "response.in_progress") => {
                self.metadata(&value["response"])?;
                self.start(out);
            }
            Some("response.output_item.added" | "response.output_item.done") => {
                self.item(index(value, "output_index")?, &value["item"], out)?;
            }
            Some("response.content_part.added" | "response.content_part.done") => {
                let part = &value["part"];
                match part["type"].as_str() {
                    Some("output_text") => self.reconcile_text(
                        key(value, 0)?,
                        part["text"].as_str().ok_or("invalid_stream_frame")?,
                        out,
                    )?,
                    Some("refusal") => self.reconcile_text(
                        key(value, 2)?,
                        part["refusal"].as_str().ok_or("invalid_stream_frame")?,
                        out,
                    )?,
                    _ => return Err("unsupported_content"),
                }
            }
            Some("response.output_text.delta") => {
                self.append_text(key(value, 0)?, delta(value)?, out)?
            }
            Some("response.refusal.delta") => {
                self.append_text(key(value, 2)?, delta(value)?, out)?
            }
            Some("response.reasoning_summary_text.delta") => {
                self.append_text(key(value, 1)?, delta(value)?, out)?
            }
            Some("response.output_text.done") => self.reconcile_text(
                key(value, 0)?,
                value["text"].as_str().ok_or("invalid_stream_frame")?,
                out,
            )?,
            Some("response.refusal.done") => self.reconcile_text(
                key(value, 2)?,
                value["refusal"].as_str().ok_or("invalid_stream_frame")?,
                out,
            )?,
            Some("response.reasoning_summary_text.done") => self.reconcile_text(
                key(value, 1)?,
                value["text"].as_str().ok_or("invalid_stream_frame")?,
                out,
            )?,
            Some("response.function_call_arguments.delta") => {
                let output_index = index(value, "output_index")?;
                self.tool(output_index)?;
                self.tool_header(output_index, value)?;
                self.append_arguments(output_index, delta(value)?, out)?;
            }
            Some("response.function_call_arguments.done") => {
                let output_index = index(value, "output_index")?;
                self.tool(output_index)?;
                self.tool_header(output_index, value)?;
                self.reconcile_arguments(
                    output_index,
                    value["arguments"]
                        .as_str()
                        .ok_or("invalid_tool_arguments")?,
                    out,
                )?;
            }
            Some("response.completed" | "response.incomplete") => self.terminal(value, out)?,
            Some("response.failed" | "response.cancelled" | "error") => {
                if let Some(code) = value
                    .pointer("/response/error/reason")
                    .or_else(|| value.pointer("/response/error/code"))
                    .and_then(Value::as_str)
                {
                    return Err(match code {
                        "invalid_tool_arguments" => "invalid_tool_arguments",
                        "invalid_stream_frame" => "invalid_stream_frame",
                        "unsupported_content" => "unsupported_content",
                        "upstream_stream_truncated" => "upstream_stream_truncated",
                        "upstream_stream_error" => "upstream_stream_error",
                        _ => "upstream_error",
                    });
                }
                return Err("upstream_error");
            }
            Some(kind)
                if kind.contains("audio")
                    || kind.contains("image")
                    || kind.contains("web_search")
                    || kind.contains("code_interpreter") =>
            {
                return Err("unsupported_content")
            }
            // Metadata and annotations have no required Chat payload. Unknown
            // same-protocol events remain on the byte-preserving path.
            _ => {}
        }
        Ok(())
    }

    fn metadata(&mut self, response: &Value) -> Result<()> {
        if self.started {
            return Ok(());
        }
        let id = response["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or(&self.id);
        let model = response["model"].as_str().unwrap_or(&self.model);
        let bytes = self
            .retained_bytes
            .saturating_sub(self.id.len())
            .saturating_sub(self.model.len())
            .checked_add(id.len())
            .and_then(|bytes| bytes.checked_add(model.len()))
            .filter(|bytes| *bytes <= MAX_RETAINED_BYTES)
            .ok_or("invalid_stream_frame")?;
        let id = id.to_owned();
        let model = model.to_owned();
        self.retained_bytes = bytes;
        self.id = id;
        self.model = model;
        if let Some(created) = response["created_at"].as_u64() {
            self.created = created;
        }
        Ok(())
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,
            "choices":[{"index":0,"delta":delta,"logprobs":null,"finish_reason":finish}]})
        )
    }

    fn start(&mut self, out: &mut Vec<String>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(self.chunk(json!({"role":"assistant","content":""}), None));
    }

    fn retain(&mut self, bytes: usize) -> Result<()> {
        self.retained_bytes = self
            .retained_bytes
            .checked_add(bytes)
            .filter(|size| *size <= MAX_RETAINED_BYTES)
            .ok_or("stream_output_limit")?;
        Ok(())
    }

    fn append_text(
        &mut self,
        key: (u64, u64, u8),
        text: &str,
        out: &mut Vec<String>,
    ) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if !self.text.contains_key(&key) && self.text.len() >= 1024 {
            return Err("invalid_stream_frame");
        }
        self.retain(text.len())?;
        self.text.entry(key).or_default().push_str(text);
        self.start(out);
        let field = match key.2 {
            0 => "content",
            1 => "reasoning_content",
            _ => "refusal",
        };
        let mut delta = json!({});
        delta[field] = json!(text);
        out.push(self.chunk(delta, None));
        Ok(())
    }

    fn reconcile_text(
        &mut self,
        key: (u64, u64, u8),
        full: &str,
        out: &mut Vec<String>,
    ) -> Result<()> {
        let previous = self.text.get(&key).map(String::as_str).unwrap_or_default();
        let rest = full.strip_prefix(previous).ok_or("invalid_stream_frame")?;
        self.append_text(key, rest, out)
    }

    fn tool(&mut self, output_index: u64) -> Result<()> {
        if self.tools.contains_key(&output_index) {
            return Ok(());
        }
        if self.tools.len() >= 128 {
            return Err("invalid_tool_arguments");
        }
        let index = self.tools.len();
        self.tools.insert(
            output_index,
            Tool {
                index,
                ..Default::default()
            },
        );
        Ok(())
    }

    fn tool_header(&mut self, output_index: u64, item: &Value) -> Result<()> {
        for (field, key) in [("call_id", "id"), ("name", "name")] {
            let Some(value) = item[field].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            let previous = if key == "id" {
                &self.tools[&output_index].id
            } else {
                &self.tools[&output_index].name
            };
            if !previous.is_empty() && previous != value {
                return Err("invalid_tool_arguments");
            }
            if previous.is_empty() {
                self.retain(value.len())?;
                let tool = self.tools.get_mut(&output_index).unwrap();
                if key == "id" {
                    tool.id = value.to_owned();
                } else {
                    tool.name = value.to_owned();
                }
            }
        }
        Ok(())
    }

    fn append_arguments(
        &mut self,
        output_index: u64,
        arguments: &str,
        out: &mut Vec<String>,
    ) -> Result<()> {
        self.retain(arguments.len())?;
        self.tools
            .get_mut(&output_index)
            .unwrap()
            .arguments
            .push_str(arguments);
        self.emit_tool(output_index, out)
    }

    fn reconcile_arguments(
        &mut self,
        output_index: u64,
        full: &str,
        out: &mut Vec<String>,
    ) -> Result<()> {
        let previous = &self.tools[&output_index].arguments;
        if let Some(rest) = full.strip_prefix(previous) {
            return self.append_arguments(output_index, rest, out);
        }
        // Some Messages providers supply complete {} initially and replace it
        // with canonical formatting after streamed arguments. Semantic equality
        // permits that formatting difference without sending duplicate deltas.
        if let (Ok(a), Ok(b)) = (
            serde_json::from_str::<Value>(previous),
            serde_json::from_str::<Value>(full),
        ) {
            if a == b {
                return self.emit_tool(output_index, out);
            }
        }
        Err("invalid_tool_arguments")
    }

    fn emit_tool(&mut self, output_index: u64, out: &mut Vec<String>) -> Result<()> {
        let tool = &self.tools[&output_index];
        if tool.id.is_empty() || tool.name.is_empty() {
            return Ok(());
        }
        let opening = !tool.opened;
        if !opening && tool.emitted == tool.arguments.len() {
            return Ok(());
        }
        let mut delta =
            json!({"index":tool.index,"function":{"arguments":&tool.arguments[tool.emitted..]}});
        if opening {
            delta["id"] = json!(tool.id);
            delta["type"] = json!("function");
            delta["function"]["name"] = json!(tool.name);
        }
        self.start(out);
        out.push(self.chunk(json!({"tool_calls":[delta]}), None));
        let tool = self.tools.get_mut(&output_index).unwrap();
        tool.opened = true;
        tool.emitted = tool.arguments.len();
        Ok(())
    }

    fn item(&mut self, output_index: u64, item: &Value, out: &mut Vec<String>) -> Result<()> {
        if chat_ingress::has_private_output(item) {
            return Err("unsupported_content");
        }
        match item["type"].as_str() {
            Some("function_call") => {
                self.tool(output_index)?;
                self.tool_header(output_index, item)?;
                if let Some(arguments) = item["arguments"].as_str() {
                    self.reconcile_arguments(output_index, arguments, out)?;
                }
                self.emit_tool(output_index, out)?;
            }
            Some("message") if item["role"] == "assistant" => {
                for (content_index, part) in item["content"]
                    .as_array()
                    .ok_or("invalid_stream_frame")?
                    .iter()
                    .enumerate()
                {
                    let kind = match part["type"].as_str() {
                        Some("output_text") => 0,
                        Some("refusal") => 2,
                        _ => return Err("unsupported_content"),
                    };
                    let field = if kind == 0 { "text" } else { "refusal" };
                    self.reconcile_text(
                        (output_index, content_index as u64, kind),
                        part[field].as_str().ok_or("invalid_stream_frame")?,
                        out,
                    )?;
                }
            }
            Some("reasoning") => {
                for (summary_index, part) in
                    item["summary"].as_array().into_iter().flatten().enumerate()
                {
                    if part["type"] != "summary_text" {
                        return Err("unsupported_content");
                    }
                    self.reconcile_text(
                        (output_index, summary_index as u64, 1),
                        part["text"].as_str().ok_or("invalid_stream_frame")?,
                        out,
                    )?;
                }
            }
            _ => return Err("unsupported_content"),
        }
        Ok(())
    }

    fn terminal(&mut self, value: &Value, out: &mut Vec<String>) -> Result<()> {
        let response = &value["response"];
        let expected = if value["type"] == "response.completed" {
            "completed"
        } else {
            "incomplete"
        };
        if response["status"] != expected {
            return Err("invalid_stream_frame");
        }
        let chat = chat_ingress::response(response).map_err(|code| {
            if code == "protocol.errors.unsupportedContent" {
                "unsupported_content"
            } else {
                "invalid_stream_frame"
            }
        })?;
        self.metadata(response)?;
        let output = response["output"]
            .as_array()
            .ok_or("invalid_stream_frame")?;
        if output.len() > 1024 {
            return Err("invalid_stream_frame");
        }
        let mut final_tools = BTreeSet::new();
        let mut final_text = BTreeSet::new();
        for (index, item) in output.iter().enumerate() {
            match item["type"].as_str() {
                Some("function_call") => {
                    final_tools.insert(index as u64);
                }
                Some("message") => {
                    for (content_index, part) in
                        item["content"].as_array().into_iter().flatten().enumerate()
                    {
                        let kind = if part["type"] == "refusal" { 2 } else { 0 };
                        final_text.insert((index as u64, content_index as u64, kind));
                    }
                }
                Some("reasoning") => {
                    for (summary_index, _) in
                        item["summary"].as_array().into_iter().flatten().enumerate()
                    {
                        final_text.insert((index as u64, summary_index as u64, 1));
                    }
                }
                _ => {}
            }
            self.item(index as u64, item, out)?;
        }
        if self.tools.keys().any(|index| !final_tools.contains(index))
            || self.text.keys().any(|key| !final_text.contains(key))
        {
            return Err("invalid_stream_frame");
        }
        if self.tools.values().any(|tool| !tool.opened) {
            return Err("invalid_tool_arguments");
        }
        self.start(out);
        let finish = chat["choices"][0]["finish_reason"]
            .as_str()
            .ok_or("invalid_stream_frame")?;
        out.push(self.chunk(json!({}), Some(finish)));
        if self.include_usage {
            out.push(format!("data: {}\n\n",json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,
                "choices":[],"usage":chat["usage"]})));
        }
        out.push("data: [DONE]\n\n".into());
        self.finished = true;
        if expected == "completed" {
            self.completed_response = Some(response.clone());
        }
        Ok(())
    }
}

fn index(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .filter(|index| *index < 1024)
        .ok_or("invalid_stream_frame")
}
fn key(value: &Value, kind: u8) -> Result<(u64, u64, u8)> {
    Ok((
        index(value, "output_index")?,
        index(
            value,
            if kind == 1 {
                "summary_index"
            } else {
                "content_index"
            },
        )?,
        kind,
    ))
}
fn delta(value: &Value) -> Result<&str> {
    value["delta"].as_str().ok_or("invalid_stream_frame")
}
