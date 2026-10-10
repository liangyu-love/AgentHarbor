//! First-token failover. A streamed reply that shows no content (text,
//! reasoning or a tool call) within the aggregate's limit is given up before
//! any of it reaches the client, so another member can answer instead. What
//! was read while waiting is replayed in front of the rest, unchanged.
use crate::core::relay_usage::event_boundary;
use futures_util::{stream, StreamExt};
use hyper::header::CONTENT_TYPE;
use serde_json::Value;
use std::time::Duration;

/// How much of a reply is scanned for content. A reply this long without any
/// is not merely slow and is let through as it is.
const SCAN_LIMIT: usize = 256 * 1024;

/// Watches a reply's bytes for the first event that settles the wait: content,
/// or the reply ending (an error or completion is passed on as it is).
pub struct Scanner {
    event_stream: bool,
    buffer: Vec<u8>,
    scanned: usize,
}

impl Scanner {
    pub fn new(event_stream: bool) -> Self {
        Self { event_stream, buffer: Vec::new(), scanned: 0 }
    }

    /// True once the reply has shown content or ended. A body that is not an
    /// event stream is a whole reply: its first bytes settle it.
    pub fn feed(&mut self, chunk: &[u8]) -> bool {
        if !self.event_stream {
            return !chunk.is_empty();
        }
        self.scanned += chunk.len();
        self.buffer.extend_from_slice(chunk);
        while let Some((end, width)) = event_boundary(&self.buffer) {
            let event: Vec<u8> = self.buffer.drain(..end + width).collect();
            if settles(&event[..end]) {
                return true;
            }
        }
        self.scanned >= SCAN_LIMIT
    }
}

fn settles(event: &[u8]) -> bool {
    let text = String::from_utf8_lossy(event);
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return text.lines().any(|line| line.strip_prefix("event:").is_some_and(|name| name.trim() == "error"));
    }
    if data.trim() == "[DONE]" {
        return true;
    }
    serde_json::from_str::<Value>(&data).is_ok_and(|value| content(&value) || ended(&value))
}

fn content(value: &Value) -> bool {
    let kind = value["type"].as_str().unwrap_or_default();
    // Responses streams text, reasoning, refusals and tool arguments as deltas.
    if kind.starts_with("response.") && kind.ends_with(".delta") {
        return true;
    }
    // Messages: text, thinking and tool input deltas, and a tool call opening.
    if kind == "content_block_delta" {
        return true;
    }
    if kind == "content_block_start" {
        return value
            .pointer("/content_block/type")
            .and_then(Value::as_str)
            .is_some_and(|block| block.ends_with("tool_use"));
    }
    // Chat: any delta field besides the role that carries something.
    value
        .pointer("/choices/0/delta")
        .and_then(Value::as_object)
        .is_some_and(|delta| {
            delta.iter().any(|(key, field)| {
                key != "role"
                    && match field {
                        Value::Null => false,
                        Value::String(text) => !text.is_empty(),
                        Value::Array(items) => !items.is_empty(),
                        _ => true,
                    }
            })
        })
}

fn ended(value: &Value) -> bool {
    matches!(
        value["type"].as_str().unwrap_or_default(),
        "response.completed" | "response.failed" | "response.incomplete" | "error" | "message_delta" | "message_stop"
    ) || value.get("error").is_some_and(|error| !error.is_null())
        || value.pointer("/choices/0/finish_reason").is_some_and(|reason| !reason.is_null())
}

pub enum Outcome {
    /// The reply showed content (or ended) in time; relay it as usual.
    Ready(reqwest::Response),
    /// Nothing within the limit. The reply is dropped unread past this point.
    Slow,
}

/// Waits up to `limit` for the reply's first content.
pub async fn await_content(upstream: reqwest::Response, limit: Duration) -> Outcome {
    let status = upstream.status();
    let version = upstream.version();
    let headers = upstream.headers().clone();
    let event_stream = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("event-stream"));
    let mut scanner = Scanner::new(event_stream);
    let mut body = Box::pin(upstream.bytes_stream());
    let mut read = Vec::new();
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        match tokio::time::timeout_at(deadline, body.next()).await {
            Err(_) => return Outcome::Slow,
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                read.push(Err(error));
                break;
            }
            Ok(Some(Ok(chunk))) => {
                let settled = scanner.feed(&chunk);
                read.push(Ok(chunk));
                if settled {
                    break;
                }
            }
        }
    }
    let mut response = hyper::Response::new(reqwest::Body::wrap_stream(stream::iter(read).chain(body)));
    *response.status_mut() = status;
    *response.version_mut() = version;
    *response.headers_mut() = headers;
    Outcome::Ready(response.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::body::Bytes;
    use std::convert::Infallible;

    fn settled_by(events: &[&str]) -> bool {
        let mut scanner = Scanner::new(true);
        events.iter().any(|event| scanner.feed(event.as_bytes()))
    }

    #[test]
    fn content_and_endings_settle_but_preambles_and_pings_do_not() {
        assert!(!settled_by(&[": ping\n\n", "event: response.created\ndata: {\"type\":\"response.created\"}\n\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\"}}\n\n"]));
        assert!(settled_by(&["data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Plan\"}\n\n"]));
        assert!(settled_by(&["data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\"}\n\n"]));
        assert!(!settled_by(&["event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"thinking\"}}\n\n"]));
        assert!(settled_by(&["data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\"}}\n\n"]));
        assert!(settled_by(&["data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"x\"}}\n\n"]));
        assert!(!settled_by(&["data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\",\"reasoning_content\":null}}]}\n\n"]));
        assert!(settled_by(&["data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"so\"}}]}\n\n"]));
        assert!(settled_by(&["data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0}]}}]}\n\n"]));
        assert!(settled_by(&["data: [DONE]\n\n"]));
        assert!(settled_by(&["event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}\n\n"]));
    }

    #[test]
    fn events_split_across_chunks_are_reassembled() {
        let mut scanner = Scanner::new(true);
        assert!(!scanner.feed(b"data: {\"type\":\"response.output_te"));
        assert!(!scanner.feed(b"xt.delta\",\"delta\":\"Hi\"}\r\n"));
        assert!(scanner.feed(b"\r\n"));
        assert!(Scanner::new(false).feed(b"{\"id\":\"whole json\"}"));
    }

    fn reply(chunks: Vec<&'static str>, then_wait: bool) -> reqwest::Response {
        let chunks = stream::iter(chunks.into_iter().map(|chunk| Ok::<_, Infallible>(Bytes::from(chunk))));
        let body = if then_wait {
            reqwest::Body::wrap_stream(chunks.chain(stream::pending()))
        } else {
            reqwest::Body::wrap_stream(chunks)
        };
        let mut response = hyper::Response::new(body);
        response.headers_mut().insert(CONTENT_TYPE, "text/event-stream".parse().unwrap());
        response.into()
    }

    #[tokio::test]
    async fn silence_past_the_limit_is_slow() {
        let upstream = reply(vec![": ping\n\n", "data: {\"type\":\"response.created\"}\n\n"], true);
        assert!(matches!(await_content(upstream, Duration::from_millis(60)).await, Outcome::Slow));
    }

    #[tokio::test]
    async fn the_reply_read_while_waiting_is_replayed_whole() {
        let chunks = vec![": ping\n\n", "data: {\"type\":\"response.created\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n", "data: [DONE]\n\n"];
        let Outcome::Ready(response) = await_content(reply(chunks.clone(), false), Duration::from_secs(5)).await else {
            panic!("content arrived in time");
        };
        assert_eq!(response.headers()[CONTENT_TYPE], "text/event-stream");
        assert_eq!(response.text().await.unwrap(), chunks.concat());
    }
}
