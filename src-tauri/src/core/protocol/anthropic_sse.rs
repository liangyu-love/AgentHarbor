//! Incremental OpenAI -> Messages SSE. A missing terminal event is an error.
use super::{anthropic, sse, Translation};
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

const MAX_BYTES: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

struct Block {
    key: (u64, u64),
    kind: Value,
    text: String,
    emitted: usize,
    opened: bool,
    closed: bool,
}

pub(super) struct StreamTranslator {
    chat: Option<sse::StreamTranslator>,
    pending: Vec<u8>,
    blocks: Vec<Block>,
    started: bool,
    finished: bool,
    completed_response: Option<Value>,
    failure: Option<&'static str>,
}

fn event(kind: &str, mut value: Value) -> String {
    value["type"] = json!(kind);
    format!("event: {kind}\ndata: {value}\n\n")
}

impl StreamTranslator {
    pub fn new(upstream: ModelProtocol) -> Self {
        Self {
            chat: (upstream == ModelProtocol::ChatCompletions).then(|| {
                sse::StreamTranslator::new(Translation {
                    from: ModelProtocol::Responses,
                    to: upstream,
                })
                .with_tool_metadata()
            }),
            pending: Vec::new(),
            blocks: Vec::new(),
            started: false,
            finished: false,
            completed_response: None,
            failure: None,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        if let Some(chat) = self.chat.as_mut() {
            let frames = chat.push(chunk).concat();
            self.consume(frames.as_bytes())
        } else {
            self.consume(chunk)
        }
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.fail("upstream_stream_truncated")
    }

    pub fn completed_response(&self) -> Option<&Value> {
        self.completed_response.as_ref()
    }
    pub fn failure(&self) -> Option<&'static str> {
        self.failure
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn fail(&mut self, code: &'static str) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.failure = Some(code);
        self.pending.clear();
        // Never close a partial tool or send message_stop after an error.
        vec![event(
            "error",
            json!({"error":{"type":"api_error","message":code}}),
        )]
    }

    fn consume(&mut self, chunk: &[u8]) -> Vec<String> {
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

    fn start(&mut self, response: &Value, out: &mut Vec<String>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(event("message_start", json!({"message":{
            "id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant",
            "model":response["model"].as_str().unwrap_or_default(),"content":[],
            "stop_reason":null,"stop_sequence":null,"usage":anthropic::usage(&response["usage"])
        }})));
    }

    fn block(&mut self, key: (u64, u64), kind: Value, out: &mut Vec<String>) -> Result<usize> {
        self.start(&Value::Null, out);
        if let Some(index) = self.blocks.iter().position(|block| block.key == key) {
            let block = &mut self.blocks[index];
            if block.kind["type"] != kind["type"] {
                return Err("invalid_stream_frame");
            }
            if kind["type"] == "tool_use" {
                for field in ["id", "name"] {
                    if let Some(value) = kind[field].as_str().filter(|value| !value.is_empty()) {
                        if block.opened && block.kind[field] != value {
                            return Err("invalid_tool_arguments");
                        }
                        block.kind[field] = json!(value);
                    }
                }
            }
            self.emit(index, out)?;
            return Ok(index);
        }
        if self.blocks.len() >= 1024 {
            return Err("invalid_stream_frame");
        }
        let index = self.blocks.len();
        self.blocks.push(Block {
            key,
            kind,
            text: String::new(),
            emitted: 0,
            opened: false,
            closed: false,
        });
        self.emit(index, out)?;
        Ok(index)
    }

    fn emit(&mut self, index: usize, out: &mut Vec<String>) -> Result<()> {
        let block = &mut self.blocks[index];
        let tool = block.kind["type"] == "tool_use";
        if tool
            && (block.text.is_empty()
                || ["id", "name"]
                    .iter()
                    .any(|key| block.kind[key].as_str().is_none_or(|s| s.is_empty())))
        {
            return Ok(()); // Some relays send tool metadata late; retain its deltas.
        }
        if !block.opened {
            block.opened = true;
            out.push(event(
                "content_block_start",
                json!({"index":index,"content_block":block.kind}),
            ));
        }
        if block.emitted < block.text.len() {
            if block.closed {
                return Err("invalid_stream_frame");
            }
            let delta = &block.text[block.emitted..];
            out.push(event(
                "content_block_delta",
                json!({"index":index,"delta":if tool {
                json!({"type":"input_json_delta","partial_json":delta})
            } else { json!({"type":"text_delta","text":delta}) }}),
            ));
            block.emitted = block.text.len();
        }
        Ok(())
    }

    fn append(&mut self, index: usize, delta: &str, out: &mut Vec<String>) -> Result<()> {
        let block = &mut self.blocks[index];
        if block.closed || block.text.len() + delta.len() > MAX_BYTES {
            return Err("invalid_stream_frame");
        }
        block.text.push_str(delta);
        self.emit(index, out)
    }

    fn reconcile(&mut self, index: usize, full: &str, out: &mut Vec<String>) -> Result<()> {
        let block = &self.blocks[index];
        if let Some(rest) = full.strip_prefix(&block.text) {
            if !rest.is_empty() {
                return self.append(index, rest, out);
            }
            return self.emit(index, out);
        }
        if block.kind["type"] == "tool_use" {
            let a = serde_json::from_str::<Value>(&block.text);
            let b = serde_json::from_str::<Value>(full);
            if let (Ok(a), Ok(b)) = (a, b) {
                if a == b {
                    return Ok(());
                }
            }
        }
        Err("invalid_stream_frame")
    }

    fn close(&mut self, index: usize, out: &mut Vec<String>) -> Result<()> {
        let block = &mut self.blocks[index];
        if block.closed {
            return Ok(());
        }
        if !block.opened {
            return Err("invalid_tool_arguments");
        }
        if block.kind["type"] == "tool_use"
            && !serde_json::from_str::<Value>(&block.text).is_ok_and(|v| v.is_object())
        {
            return Err("invalid_tool_arguments");
        }
        block.closed = true;
        out.push(event("content_block_stop", json!({"index":index})));
        Ok(())
    }

    fn item(&mut self, index: u64, item: &Value, out: &mut Vec<String>) -> Result<()> {
        match item["type"].as_str() {
            Some("function_call") => {
                if item["status"] == "incomplete" || !super::responses::valid_function_call(item) {
                    return Err("invalid_tool_arguments");
                }
                let block = self.block(
                    (index, 0),
                    json!({"type":"tool_use","id":item["call_id"],"name":item["name"],"input":{}}),
                    out,
                )?;
                self.reconcile(
                    block,
                    item["arguments"].as_str().ok_or("invalid_tool_arguments")?,
                    out,
                )?;
                self.close(block, out)?;
            }
            Some("message") => {
                for (part_index, part) in item["content"]
                    .as_array()
                    .ok_or("invalid_stream_frame")?
                    .iter()
                    .enumerate()
                {
                    let text = match part["type"].as_str() {
                        Some("output_text") => part["text"].as_str(),
                        Some("refusal") => part["refusal"].as_str(),
                        _ => return Err("invalid_stream_frame"),
                    }
                    .ok_or("invalid_stream_frame")?;
                    let block = self.block(
                        (index, part_index as u64),
                        json!({"type":"text","text":""}),
                        out,
                    )?;
                    self.reconcile(block, text, out)?;
                    self.close(block, out)?;
                }
            }
            Some("reasoning") => {}
            _ => return Err("invalid_stream_frame"),
        }
        Ok(())
    }

    fn frame(&mut self, frame: &str, out: &mut Vec<String>) -> Result<()> {
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| "invalid_stream_frame")?;
        if !value["error"].is_null() {
            return Err("upstream_error");
        }
        let kind = value["type"].as_str().ok_or("invalid_stream_frame")?;
        let index = || value["output_index"].as_u64().ok_or("invalid_stream_frame");
        match kind {
            "response.created" | "response.in_progress" => self.start(&value["response"], out),
            "response.output_item.added" => match value["item"]["type"].as_str() {
                Some("function_call") => {
                    let item = &value["item"];
                    let block = self.block((index()?,0),json!({"type":"tool_use","id":item["call_id"],"name":item["name"],"input":{}}),out)?;
                    if let Some(args) = item["arguments"].as_str().filter(|s| !s.is_empty()) {
                        self.append(block, args, out)?;
                    }
                }
                Some("message" | "reasoning") => {}
                _ => return Err("invalid_stream_frame"),
            },
            "response.output_text.delta" | "response.refusal.delta" => {
                let key = (
                    index()?,
                    value["content_index"]
                        .as_u64()
                        .ok_or("invalid_stream_frame")?,
                );
                let block = self.block(key, json!({"type":"text","text":""}), out)?;
                self.append(
                    block,
                    value["delta"].as_str().ok_or("invalid_stream_frame")?,
                    out,
                )?;
            }
            "response.function_call_arguments.delta" => {
                let key = (index()?, 0);
                let block = self
                    .blocks
                    .iter()
                    .position(|block| block.key == key)
                    .ok_or("invalid_tool_arguments")?;
                if let (Some(id), Some(name)) = (value["call_id"].as_str(), value["name"].as_str())
                {
                    self.block(
                        key,
                        json!({"type":"tool_use","id":id,"name":name,"input":{}}),
                        out,
                    )?;
                }
                self.append(
                    block,
                    value["delta"].as_str().ok_or("invalid_tool_arguments")?,
                    out,
                )?;
            }
            "response.output_item.done" => self.item(index()?, &value["item"], out)?,
            "response.completed" | "response.incomplete" => {
                let response = &value["response"];
                if (kind == "response.completed" && response["status"] != "completed")
                    || (kind == "response.incomplete" && response["status"] != "incomplete")
                {
                    return Err("invalid_stream_frame");
                }
                let message =
                    anthropic::response_body(response).map_err(|_| "invalid_upstream_response")?;
                self.start(response, out);
                let output = response["output"]
                    .as_array()
                    .ok_or("invalid_stream_frame")?;
                let expected: Vec<(u64, u64)> = output
                    .iter()
                    .enumerate()
                    .flat_map(|(index, item)| {
                        let count = match item["type"].as_str() {
                            Some("function_call") => 1,
                            Some("message") => item["content"].as_array().map_or(0, Vec::len),
                            _ => 0,
                        };
                        (0..count).map(move |part| (index as u64, part as u64))
                    })
                    .collect();
                if self
                    .blocks
                    .iter()
                    .any(|block| !expected.contains(&block.key))
                {
                    return Err("invalid_stream_frame");
                }
                for (index, item) in output.iter().enumerate() {
                    self.item(index as u64, item, out)?;
                }
                for index in 0..self.blocks.len() {
                    self.close(index, out)?;
                }
                out.push(event("message_delta",json!({"delta":{"stop_reason":message["stop_reason"],"stop_sequence":null},"usage":message["usage"]})));
                out.push(event("message_stop", json!({})));
                if super::message_history::cacheable_response(response) {
                    self.completed_response = Some(response.clone());
                }
                self.finished = true;
            }
            "response.failed" | "error" => return Err("upstream_error"),
            // Part/done and reasoning events are redundant; final items validate them.
            _ => {}
        }
        Ok(())
    }
}
