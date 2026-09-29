//! Streaming connectivity probe; timings come from actual network milestones.
use super::{models::ApiProxyConfigPayload, newapi, router::ModelProtocol};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeEvent {
    pub stage: String,
    pub elapsed_ms: u64,
    pub status: Option<u16>,
    pub text: String,
}

pub fn run(
    config: &newapi::Connection,
    model: &str,
    protocol: ModelProtocol,
    proxy: &ApiProxyConfigPayload,
    emit: &dyn Fn(ProbeEvent),
) -> Result<(), String> {
    let started = Instant::now();
    let event = |stage: &str, status, text: String| {
        emit(ProbeEvent {
            stage: stage.into(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            status,
            text,
        })
    };
    let result = (|| {
        if model.trim().is_empty() || model.len() > 200 {
            return Err("newapi.errors.model".into());
        }
        let (path, body) = match protocol {
            ModelProtocol::Responses => (
                "v1/responses",
                json!({"model":model,"input":"Say hi.","max_output_tokens":64,"stream":true}),
            ),
            ModelProtocol::ChatCompletions => (
                "v1/chat/completions",
                json!({"model":model,"messages":[{"role":"user","content":"Say hi."}],"max_tokens":64,"stream":true}),
            ),
            ModelProtocol::AnthropicMessages => (
                "v1/messages",
                json!({"model":model,"messages":[{"role":"user","content":"Say hi."}],"max_tokens":64,"stream":true}),
            ),
        };
        let url = newapi::endpoint(config, path)?;
        let mut request = newapi::client_with_timeout(proxy, Duration::from_secs(60))?
            .post(url.clone())
            .header("Accept", "text/event-stream")
            .header("User-Agent", "AgentHarbor/1.0")
            .json(&body);
        if protocol == ModelProtocol::AnthropicMessages {
            request = request
                .header("x-api-key", newapi::api_key_header(&config.api_key)?)
                .header("anthropic-version", "2023-06-01");
        } else {
            request = request.header("Authorization", newapi::auth_header(&config.api_key)?);
        }
        if url.host_str() == Some("opencode.ai") {
            request = request.header("x-opencode-session", uuid::Uuid::new_v4().to_string());
        }
        event(
            "request",
            None,
            format!(
                "POST {}\nHost: {}\nAccept: text/event-stream\nModel: {}",
                url.path(),
                url.host_str().unwrap_or_default(),
                model
            ),
        );
        let mut response = request.send().map_err(newapi::request_error)?;
        let status = response.status();
        event("headers", Some(status.as_u16()), String::new());
        if !status.is_success() {
            let mut bytes = Vec::new();
            let _ = response.take(8192).read_to_end(&mut bytes);
            if !bytes.is_empty() {
                let text = String::from_utf8_lossy(&bytes).replace(&config.api_key, "[redacted]");
                event("content", None, text);
            }
            return Err(newapi::status_error(status));
        }
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.contains("text/event-stream"));
        let mut parser = ProbeStream::default();
        let mut pending = Vec::new();
        let mut total = 0usize;
        let mut first = true;
        let mut chunk = [0u8; 2048];
        loop {
            let count = response
                .read(&mut chunk)
                .map_err(|_| "newapi.errors.network")?;
            if count == 0 {
                break;
            }
            total += count;
            if total > 2 * 1024 * 1024 {
                return Err("newapi.errors.response".into());
            }
            if first {
                event("firstByte", None, String::new());
                first = false;
            }
            pending.extend_from_slice(&chunk[..count]);
            if sse {
                while let Some(index) = pending.iter().position(|&b| b == b'\n') {
                    let line = pending.drain(..=index).collect::<Vec<_>>();
                    let line = std::str::from_utf8(&line).map_err(|_| "newapi.errors.response")?;
                    if let Some(text) = parser.line(line, protocol)? {
                        event("content", None, text);
                    }
                }
                if parser.done {
                    break;
                }
            }
        }
        if sse {
            if !pending.is_empty() {
                if let Some(text) = parser.line(
                    std::str::from_utf8(&pending).map_err(|_| "newapi.errors.response")?,
                    protocol,
                )? {
                    event("content", None, text);
                }
            }
            if !parser.done || !parser.valid {
                return Err("newapi.errors.response".into());
            }
        } else {
            let value: Value =
                serde_json::from_slice(&pending).map_err(|_| "newapi.errors.response")?;
            let valid = match protocol {
                ModelProtocol::Responses => {
                    value["object"] == "response"
                        && value["status"] != "failed"
                        && value["error"].is_null()
                }
                ModelProtocol::ChatCompletions => {
                    value["choices"].as_array().is_some_and(|v| !v.is_empty())
                }
                ModelProtocol::AnthropicMessages => value["type"] == "message",
            };
            if !valid {
                return Err("newapi.errors.response".into());
            }
            // Some compatible servers ignore stream=true. Clearly show this response.
            event(
                "content",
                None,
                serde_json::to_string_pretty(&value)
                    .unwrap_or_default()
                    .chars()
                    .take(8192)
                    .collect(),
            );
        }
        Ok(())
    })();
    event(
        if result.is_ok() { "complete" } else { "failed" },
        None,
        result.as_ref().err().cloned().unwrap_or_default(),
    );
    result
}

#[derive(Default)]
struct ProbeStream {
    valid: bool,
    done: bool,
}
impl ProbeStream {
    fn line(&mut self, line: &str, protocol: ModelProtocol) -> Result<Option<String>, String> {
        let Some(data) = line.trim().strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(None);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| "newapi.errors.response")?;
        let kind = value["type"].as_str().unwrap_or_default();
        if !value["error"].is_null() || matches!(kind, "error" | "response.failed") {
            return Err("newapi.errors.server".into());
        }
        let text = match protocol {
            ModelProtocol::ChatCompletions => {
                if value["choices"].as_array().is_some_and(|v| !v.is_empty()) {
                    self.valid = true;
                }
                value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
            }
            ModelProtocol::AnthropicMessages => {
                if kind == "message_start" {
                    self.valid = true;
                }
                if kind == "message_stop" {
                    self.done = true;
                }
                value.pointer("/delta/text").and_then(Value::as_str)
            }
            ModelProtocol::Responses => {
                if kind == "response.created"
                    || kind == "response.completed"
                    || kind == "response.incomplete"
                {
                    self.valid = true;
                }
                if kind == "response.completed" || kind == "response.incomplete" {
                    self.done = true;
                }
                if kind == "response.output_text.delta" {
                    value["delta"].as_str()
                } else {
                    None
                }
            }
        };
        Ok(text.map(|s| s.chars().take(4096).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_error_events_and_does_not_accept_done_without_response() {
        let mut parser = ProbeStream::default();
        parser
            .line("data: [DONE]", ModelProtocol::ChatCompletions)
            .unwrap();
        assert!(!parser.valid);
        assert!(parser
            .line(
                "data: {\"type\":\"error\",\"error\":{}}",
                ModelProtocol::AnthropicMessages
            )
            .is_err());
        let mut parser = ProbeStream::default();
        assert_eq!(
            parser
                .line(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}",
                    ModelProtocol::ChatCompletions
                )
                .unwrap()
                .as_deref(),
            Some("hello")
        );
        assert!(parser.valid && !parser.done);
    }

    #[test]
    fn streaming_probe_reports_real_stages_and_rejects_truncated_streams() {
        use std::{io::Write, net::TcpListener, sync::Mutex};
        for (protocol, body, success) in [
            (ModelProtocol::ChatCompletions, "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n", true),
            (ModelProtocol::Responses, "data: {\"type\":\"response.created\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\ndata: {\"type\":\"response.completed\"}\n\n", true),
            (ModelProtocol::AnthropicMessages, "data: {\"type\":\"message_start\"}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n", true),
            (ModelProtocol::ChatCompletions, "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut buffer = [0u8;8192]; let _ = socket.read(&mut buffer).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                socket.flush().unwrap();
                std::thread::sleep(Duration::from_millis(30));
                socket.write_all(body.as_bytes()).unwrap();
            });
            let events = Mutex::new(Vec::new());
            let config = newapi::Connection { base_url: format!("http://{address}"), api_key: "synthetic-test".into(), ..Default::default() };
            let proxy = newapi::effective_proxy("direct", &ApiProxyConfigPayload::default());
            let result = run(&config, "test-model", protocol, &proxy, &|e| events.lock().unwrap().push(e));
            server.join().unwrap();
            assert_eq!(result.is_ok(), success);
            let events = events.into_inner().unwrap();
            assert_eq!(events[0].stage, "request");
            assert_eq!(events[1].stage, "headers");
            assert_eq!(events[2].stage, "firstByte");
            assert!(events[2].elapsed_ms >= events[1].elapsed_ms);
            assert!(events.iter().any(|e| e.text == "hi"));
            assert_eq!(events.last().unwrap().stage, if success {"complete"} else {"failed"});
        }
    }
}
