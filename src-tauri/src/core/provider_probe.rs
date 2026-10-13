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
        let model = newapi::validated_test_model(model)?;
        let mut budget = 64;
        let mut retried = false;
        let mut response = loop {
        let (path, body) = match protocol {
            ModelProtocol::Responses => (
                "v1/responses",
                json!({"model":model,"input":"Say hi.","max_output_tokens":budget,"stream":true}),
            ),
            ModelProtocol::ChatCompletions => (
                "v1/chat/completions",
                json!({"model":model,"messages":[{"role":"user","content":"Say hi."}],"max_tokens":budget,"stream":true}),
            ),
            ModelProtocol::AnthropicMessages => (
                "v1/messages",
                json!({"model":model,"messages":[{"role":"user","content":"Say hi."}],"max_tokens":budget,"stream":true}),
            ),
        };
        let url = newapi::endpoint(config, path)?;
        let mut request = newapi::client_with_timeout(proxy, Duration::from_secs(60))?
            .post(url.clone())
            .header("Accept", "text/event-stream")
            .header("User-Agent", "AgentHarbor/1.0")
            .json(&body);
        let (name, value) = newapi::model_auth_header(&config.base_url, protocol, &config.api_key)?;
        request = request.header(name, value);
        if protocol == ModelProtocol::AnthropicMessages {
            request = request.header("anthropic-version", "2023-06-01");
        }
        if url.host_str() == Some("opencode.ai") {
            request = request.header("x-opencode-session", uuid::Uuid::new_v4().to_string());
        }
        event(
            "request",
            None,
            format!(
                "POST {}\nHost: {}\nAccept: text/event-stream\nModel: {}\nOutput budget: {}",
                url.path(),
                url.host_str().unwrap_or_default(),
                model, budget
            ),
        );
        let response = request.send().map_err(newapi::request_error)?;
        let status = response.status();
        event("headers", Some(status.as_u16()), String::new());
        if !status.is_success() {
            let mut bytes = Vec::new();
            let _ = response.take(8192).read_to_end(&mut bytes);
            let message = error_message(&bytes);
            let detail = format!("HTTP {}: {}", status.as_u16(), message.as_deref().unwrap_or(status.canonical_reason().unwrap_or("request rejected")));
            event("content", None, redact(&detail, config));
            if !retried && matches!(status.as_u16(), 400 | 422) {
                if let Some(minimum) = message.as_deref().and_then(|m| minimum_budget(m, budget)) {
                    budget = minimum;
                    retried = true;
                    event("retry", Some(status.as_u16()), format!("Retry once with output budget {budget}"));
                    continue;
                }
            }
            return Err(detail);
        }
        break response;
        };
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
            if !value["error"].is_null() || value["status"] == "failed" || value["type"] == "error" {
                return Err(error_message(&pending).unwrap_or_else(|| "newapi.errors.server".into()));
            }
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
    })().map_err(|error: String| redact(&error, config));
    event(
        if result.is_ok() { "complete" } else { "failed" },
        None,
        result.as_ref().err().cloned().unwrap_or_default(),
    );
    result
}

pub(crate) fn redact(text: &str, config: &newapi::Connection) -> String {
    let mut text = text.to_owned();
    for secret in [&config.api_key, &config.access_token] {
        for token in [
            secret.as_str(),
            secret
                .trim()
                .strip_prefix("Bearer ")
                .unwrap_or(secret.trim()),
        ] {
            if !token.is_empty() {
                text = text.replace(token, "[redacted]");
            }
        }
    }
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(8192)
        .collect()
}

pub(crate) fn error_message(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    let message = value
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/response/error/message"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
        })
        .and_then(Value::as_str);
    let raw = String::from_utf8_lossy(bytes);
    let message = message.unwrap_or(raw.trim());
    (!message.is_empty()).then(|| message.chars().take(2048).collect())
}

/// An explicit lower bound, not quota exhaustion, a context error or a guess.
fn minimum_budget(message: &str, current: u64) -> Option<u64> {
    let lower = message.to_ascii_lowercase();
    let param = ["max_output_tokens", "max_completion_tokens", "max_tokens"]
        .iter()
        .find_map(|p| lower.find(p))?;
    let tail = &lower[param..];
    let minimum = [
        "must be at least ",
        "minimum value is ",
        "greater than or equal to ",
        "minimum is ",
    ]
    .iter()
    .find_map(|phrase| tail.find(phrase).map(|i| &tail[i + phrase.len()..]))?;
    let digits: String = minimum
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse::<u64>()
        .ok()
        .filter(|n| *n > current && *n <= 4096)
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
            return Err(
                error_message(data.as_bytes()).unwrap_or_else(|| "newapi.errors.server".into())
            );
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
    fn budget_retry_requires_a_specific_bounded_minimum() {
        assert_eq!(
            minimum_budget("max_tokens must be at least 128", 64),
            Some(128)
        );
        assert_eq!(
            minimum_budget("max_output_tokens must be greater than or equal to 256", 64),
            Some(256)
        );
        for message in [
            "insufficient quota",
            "max_tokens must be at most 32",
            "max_tokens must be at least 1000000",
            "max_tokens must be at least 32",
            "context length minimum is 128",
            "max_tokens exceeded output limit",
        ] {
            assert_eq!(minimum_budget(message, 64), None, "{message}");
        }
        let config = newapi::Connection {
            api_key: "Bearer sk-secret".into(),
            access_token: "access-secret".into(),
            ..Default::default()
        };
        assert_eq!(
            redact("sk-secret access-secret", &config),
            "[redacted] [redacted]"
        );
    }

    #[test]
    fn both_probe_paths_share_one_bounded_retry_and_preserve_final_errors() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            sync::Mutex,
        };
        for progress in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let mut budgets = Vec::new();
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(socket.try_clone().unwrap());
                    let mut len = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse::<usize>().unwrap();
                        }
                    }
                    let mut body = vec![0; len];
                    reader.read_exact(&mut body).unwrap();
                    let v: Value = serde_json::from_slice(&body).unwrap();
                    budgets.push(v["max_output_tokens"].as_u64().unwrap());
                    let error = r#"{"error":{"message":"max_output_tokens must be at least 128; sk-secret"}}"#;
                    write!(socket,"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{error}",error.len()).unwrap();
                }
                listener.set_nonblocking(true).unwrap();
                assert!(
                    listener.accept().is_err(),
                    "must not attempt a third request"
                );
                budgets
            });
            let config = newapi::Connection {
                base_url: format!("http://{address}"),
                api_key: "sk-secret".into(),
                ..Default::default()
            };
            let proxy = newapi::effective_proxy("direct", &ApiProxyConfigPayload::default());
            let events = Mutex::new(Vec::new());
            let result = if progress {
                run(&config, "test", ModelProtocol::Responses, &proxy, &|e| {
                    events.lock().unwrap().push(e)
                })
            } else {
                newapi::test_responses(&config, "test", &proxy)
            };
            let error = result.unwrap_err();
            assert!(error.contains("at least 128"));
            assert!(!error.contains("sk-secret"));
            assert_eq!(server.join().unwrap(), vec![64, 128]);
            if progress {
                let events = events.lock().unwrap();
                assert_eq!(events.iter().filter(|e| e.stage == "retry").count(), 1);
                assert_eq!(events.last().unwrap().stage, "failed");
                assert!(!serde_json::to_string(&*events)
                    .unwrap()
                    .contains("sk-secret"));
            }
        }
    }
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
