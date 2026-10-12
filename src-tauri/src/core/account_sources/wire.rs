//! Vendor wires: account sources whose own API is neither Chat, Responses
//! nor Messages (Google's Code Assist, Kiro). The router talks Chat
//! Completions to them like to any relay; this layer turns that Chat request
//! into the vendor's, and the vendor's reply back into Chat — an SSE stream,
//! or one JSON body for a request that did not stream — before the router's
//! own translation reads it. Errors pass through untouched, so refusals are
//! explained on the vendor's own words.
use super::{codeassist, kiro};
use futures_util::StreamExt;
use hyper::body::Bytes;
use serde_json::{json, Value};
use std::collections::HashMap;

/// What a vendor's reply says, in the order it says it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    Reasoning(String),
    /// A tool call opens with its id and name; its arguments follow.
    ToolStart { id: String, name: String },
    ToolArgs { id: String, text: String },
    /// `stop`, `length`, `tool_calls` or `content_filter`.
    Finish(&'static str),
    Usage { input: u64, output: u64, cached: u64, reasoning: u64 },
    Error(String),
}

/// The vendor half of a decoder: raw reply bytes in, events out.
pub enum Codec {
    CodeAssist(codeassist::Decoder),
    Kiro(kiro::Decoder),
}

impl Codec {
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        match self {
            Codec::CodeAssist(decoder) => decoder.feed(bytes, out),
            Codec::Kiro(decoder) => decoder.feed(bytes, out),
        }
    }

    fn finish(&mut self, out: &mut Vec<Event>) {
        match self {
            Codec::CodeAssist(decoder) => decoder.finish(out),
            Codec::Kiro(decoder) => decoder.finish(out),
        }
    }
}

/// A Chat request in the vendor's shape, and what reads its reply.
pub struct Encoded {
    pub body: Vec<u8>,
    /// The path below the source's base the request is posted to.
    pub path: String,
    pub headers: Vec<(&'static str, String)>,
    pub decoder: Decoder,
}

/// Turns one vendor reply into Chat.
pub struct Decoder {
    codec: Codec,
    writer: Writer,
}

impl Decoder {
    pub fn new(codec: Codec, model: &str, stream: bool) -> Self {
        Decoder { codec, writer: Writer::new(model, stream) }
    }

    /// The reply as Chat: a successful one rewritten, an error left as the
    /// vendor sent it.
    pub async fn decode(self, response: reqwest::Response) -> reqwest::Response {
        if !response.status().is_success() {
            return response;
        }
        let status = response.status();
        let Decoder { mut codec, mut writer } = self;
        if !writer.stream {
            let bytes = response.bytes().await.unwrap_or_default();
            let mut events = Vec::new();
            codec.feed(&bytes, &mut events);
            codec.finish(&mut events);
            let body = writer.collect(events);
            return reqwest::Response::from(hyper::Response::builder().status(status)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(body.to_string()).expect("static response parts"));
        }
        let upstream = response.bytes_stream();
        let stream = futures_util::stream::unfold(Some((upstream, codec, writer)), |state| async move {
            let (mut upstream, mut codec, mut writer) = state?;
            loop {
                let mut events = Vec::new();
                match upstream.next().await {
                    Some(Ok(chunk)) => codec.feed(&chunk, &mut events),
                    Some(Err(error)) => {
                        return Some((Err(std::io::Error::other(error.to_string())), None));
                    }
                    None => {
                        codec.finish(&mut events);
                        let mut out = writer.write(events);
                        out.push_str(&writer.done());
                        return Some((Ok(Bytes::from(out)), None));
                    }
                }
                let out = writer.write(events);
                if !out.is_empty() {
                    return Some((Ok::<_, std::io::Error>(Bytes::from(out)), Some((upstream, codec, writer))));
                }
            }
        });
        reqwest::Response::from(hyper::Response::builder().status(status)
            .header(hyper::header::CONTENT_TYPE, "text/event-stream")
            .body(reqwest::Body::wrap_stream(stream)).expect("static response parts"))
    }
}

/// Builds Chat from events: SSE chunks for a stream, one completion else.
pub(super) struct Writer {
    stream: bool,
    id: String,
    model: String,
    created: i64,
    started: bool,
    finished: bool,
    tools: HashMap<String, usize>,
    content: String,
    reasoning: String,
    calls: Vec<(String, String, String)>,
    finish: Option<&'static str>,
    usage: Option<Value>,
    error: Option<String>,
}

impl Writer {
    pub(super) fn new(model: &str, stream: bool) -> Self {
        Writer {
            stream,
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
            model: model.to_owned(),
            created: chrono::Utc::now().timestamp(),
            started: false,
            finished: false,
            tools: HashMap::new(),
            content: String::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            finish: None,
            usage: None,
            error: None,
        }
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> String {
        let value = json!({"id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
        format!("data: {value}\n\n")
    }

    fn usage_of(input: u64, output: u64, cached: u64, reasoning: u64) -> Value {
        json!({"prompt_tokens": input, "completion_tokens": output, "total_tokens": input + output,
            "prompt_tokens_details": {"cached_tokens": cached}, "completion_tokens_details": {"reasoning_tokens": reasoning}})
    }

    /// SSE for these events.
    pub(super) fn write(&mut self, events: Vec<Event>) -> String {
        let mut out = String::new();
        for event in events {
            if self.finished && !matches!(event, Event::Usage { .. }) {
                continue;
            }
            if !self.started && !matches!(event, Event::Error(_)) {
                self.started = true;
                out.push_str(&self.chunk(json!({"role": "assistant", "content": ""}), None));
            }
            match event {
                Event::Text(text) if !text.is_empty() => out.push_str(&self.chunk(json!({"content": text}), None)),
                Event::Reasoning(text) if !text.is_empty() => out.push_str(&self.chunk(json!({"reasoning_content": text}), None)),
                Event::ToolStart { id, name } => {
                    let index = self.tools.len();
                    self.tools.insert(id.clone(), index);
                    out.push_str(&self.chunk(json!({"tool_calls": [{"index": index, "id": id, "type": "function",
                        "function": {"name": name, "arguments": ""}}]}), None));
                }
                Event::ToolArgs { id, text } if !text.is_empty() => {
                    let index = self.tools.get(&id).copied().unwrap_or(self.tools.len().saturating_sub(1));
                    out.push_str(&self.chunk(json!({"tool_calls": [{"index": index, "function": {"arguments": text}}]}), None));
                }
                Event::Finish(reason) => {
                    self.finished = true;
                    let reason = if reason == "stop" && !self.tools.is_empty() { "tool_calls" } else { reason };
                    out.push_str(&self.chunk(json!({}), Some(reason)));
                }
                Event::Usage { input, output, cached, reasoning } => {
                    let value = json!({"id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
                        "choices": [], "usage": Self::usage_of(input, output, cached, reasoning)});
                    out.push_str(&format!("data: {value}\n\n"));
                }
                Event::Error(message) => {
                    self.finished = true;
                    out.push_str(&format!("data: {}\n\n", json!({"error": {"message": message, "type": "upstream_error"}})));
                }
                _ => {}
            }
        }
        out
    }

    /// The end of a stream: a finish if the vendor gave none, then `[DONE]`.
    pub(super) fn done(&mut self) -> String {
        let mut out = String::new();
        if !self.finished {
            if !self.started {
                out.push_str(&self.chunk(json!({"role": "assistant", "content": ""}), None));
            }
            let reason = if self.tools.is_empty() { "stop" } else { "tool_calls" };
            out.push_str(&self.chunk(json!({}), Some(reason)));
        }
        out.push_str("data: [DONE]\n\n");
        out
    }

    /// One completion from every event of a reply that did not stream.
    pub(super) fn collect(&mut self, events: Vec<Event>) -> Value {
        for event in events {
            match event {
                Event::Text(text) => self.content.push_str(&text),
                Event::Reasoning(text) => self.reasoning.push_str(&text),
                Event::ToolStart { id, name } => {
                    self.tools.insert(id.clone(), self.calls.len());
                    self.calls.push((id, name, String::new()));
                }
                Event::ToolArgs { id, text } => {
                    if let Some(index) = self.tools.get(&id).copied().or(self.calls.len().checked_sub(1)) {
                        self.calls[index].2.push_str(&text);
                    }
                }
                Event::Finish(reason) => self.finish = self.finish.or(Some(reason)),
                Event::Usage { input, output, cached, reasoning } => self.usage = Some(Self::usage_of(input, output, cached, reasoning)),
                Event::Error(message) => self.error = Some(message),
            }
        }
        if let Some(message) = self.error.take() {
            return json!({"error": {"message": message, "type": "upstream_error"}});
        }
        let mut message = json!({"role": "assistant", "content": self.content});
        if !self.reasoning.is_empty() {
            message["reasoning_content"] = json!(self.reasoning);
        }
        if !self.calls.is_empty() {
            message["tool_calls"] = Value::Array(self.calls.iter().map(|(id, name, args)| json!({
                "id": id, "type": "function", "function": {"name": name, "arguments": if args.is_empty() { "{}" } else { args }},
            })).collect());
        }
        let finish = match self.finish.unwrap_or("stop") {
            "stop" if !self.calls.is_empty() => "tool_calls",
            other => other,
        };
        let mut body = json!({"id": self.id, "object": "chat.completion", "created": self.created, "model": self.model,
            "choices": [{"index": 0, "message": message, "finish_reason": finish}]});
        if let Some(usage) = self.usage.take() {
            body["usage"] = usage;
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(sse: &str) -> Vec<Value> {
        sse.split("\n\n").filter_map(|frame| frame.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]").map(|data| serde_json::from_str(data).unwrap()).collect()
    }

    #[test]
    fn events_become_chat_chunks_with_a_finish_and_done() {
        let mut writer = Writer::new("m", true);
        let mut out = writer.write(vec![
            Event::Reasoning("think".into()), Event::Text("hi".into()),
            Event::ToolStart { id: "c1".into(), name: "bash".into() }, Event::ToolArgs { id: "c1".into(), text: "{\"a\":1}".into() },
            Event::Finish("stop"), Event::Usage { input: 5, output: 3, cached: 1, reasoning: 2 },
        ]);
        out.push_str(&writer.done());
        assert!(out.ends_with("data: [DONE]\n\n"));
        let chunks = frames(&out);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(chunks[1]["choices"][0]["delta"]["reasoning_content"], "think");
        assert_eq!(chunks[2]["choices"][0]["delta"]["content"], "hi");
        assert_eq!(chunks[3]["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "bash");
        assert_eq!(chunks[4]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert_eq!(chunks[5]["choices"][0]["finish_reason"], "tool_calls", "a reply that called a tool stops for it");
        assert_eq!(chunks[6]["usage"]["completion_tokens_details"]["reasoning_tokens"], 2);
    }

    #[test]
    fn a_reply_that_did_not_stream_is_one_completion() {
        let mut writer = Writer::new("m", false);
        let body = writer.collect(vec![Event::Text("a".into()), Event::Text("b".into()), Event::Finish("length"),
            Event::Usage { input: 1, output: 2, cached: 0, reasoning: 0 }]);
        assert_eq!(body["choices"][0]["message"]["content"], "ab");
        assert_eq!(body["choices"][0]["finish_reason"], "length");
        assert_eq!(body["usage"]["total_tokens"], 3);
        let failed = Writer::new("m", false).collect(vec![Event::Error("quota".into())]);
        assert_eq!(failed["error"]["message"], "quota");
    }

    #[test]
    fn a_stream_without_a_finish_still_ends_cleanly() {
        let mut writer = Writer::new("m", true);
        let out = writer.done();
        let chunks = frames(&out);
        assert_eq!(chunks.last().unwrap()["choices"][0]["finish_reason"], "stop");
    }
}
