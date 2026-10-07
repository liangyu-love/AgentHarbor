//! Shared loopback fixtures for real Codex tests. Credentials and writable
//! homes are synthetic; child processes can reach only the loopback services.
#![allow(dead_code)]
use super::{CodexPaths, ModelProtocol};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const FIXTURE_EMAIL: &str = "router-regression@example.invalid";
pub(super) const FIXTURE_ACCOUNT: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const LOCAL_TOKEN: &str = "agentharbor_synthetic_local_token";
pub(super) const STREAM_TEXT: &str = "Local translated stream completed.";
// router_proxy is process-global; isolate the tests that start its listener.
pub(super) static ROUTER_TEST_LOCK: Mutex<()> = Mutex::new(());

/// The listener never opens an outbound connection, including for CONNECT.
pub(super) struct RejectingProxy {
    pub(super) url: String,
    pub(super) requests: Receiver<String>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RejectingProxy {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
                        let mut first_line = String::new();
                        if BufReader::new(&mut socket)
                            .read_line(&mut first_line)
                            .is_ok()
                        {
                            let _ = sender.send(first_line);
                        }
                        let _ = socket.write_all(
                            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for RejectingProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Owns only the newly spawned test process; never discovers or kills others.
pub(super) struct AppServer {
    child: Child,
    stdin: Option<ChildStdin>,
    pub(super) messages: Receiver<Result<Value, String>>,
    pub(super) stderr: Arc<Mutex<String>>,
    readers: Vec<JoinHandle<()>>,
}

pub(super) struct CapturedRequest {
    pub(super) request_line: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl CapturedRequest {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A loopback-only model endpoint. It records requests and returns a fixed
/// completion with no tools, instructions, or network calls.
pub(super) struct MockResponses {
    pub(super) port: u16,
    pub(super) requests: Receiver<CapturedRequest>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MockResponses {
    pub(super) fn start() -> Self {
        Self::for_protocol(ModelProtocol::Responses)
    }

    pub(super) fn for_protocol(protocol: ModelProtocol) -> Self {
        Self::with_stream(protocol, mock_stream(protocol))
    }

    pub(super) fn with_stream(protocol: ModelProtocol, event: String) -> Self {
        Self::with_account_policy(protocol, event, false)
    }

    pub(super) fn official_account() -> Self {
        Self::with_account_policy(
            ModelProtocol::Responses,
            mock_stream(ModelProtocol::Responses),
            true,
        )
    }

    pub(super) fn with_account_policy(
        protocol: ModelProtocol,
        event: String,
        reject_relay_ids: bool,
    ) -> Self {
        Self::with_handler(protocol, move |_| event.clone(), reject_relay_ids)
    }

    /// Script only local model replies; no returned tool may access a network.
    pub(super) fn scripted(handler: impl Fn(&CapturedRequest) -> String + Send + 'static) -> Self {
        Self::with_handler(ModelProtocol::Responses, handler, false)
    }

    fn with_handler(
        protocol: ModelProtocol,
        handler: impl Fn(&CapturedRequest) -> String + Send + 'static,
        reject_relay_ids: bool,
    ) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                let (socket, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(socket);
                let mut first_line = String::new();
                if reader.read_line(&mut first_line).is_err() {
                    continue;
                }
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
                    }
                }
                let length = headers
                    .iter()
                    .find(|(name, _)| name == "content-length")
                    .and_then(|(_, value)| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if length > 1024 * 1024 {
                    continue;
                }
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let endpoint = match protocol {
                    ModelProtocol::Responses => "/responses ",
                    ModelProtocol::ChatCompletions => "/chat/completions ",
                    ModelProtocol::AnthropicMessages => "/messages ",
                };
                if first_line.starts_with("POST ") && first_line.contains(endpoint) {
                    let model = serde_json::from_slice::<Value>(&body)
                        .ok()
                        .and_then(|value| value["model"].as_str().map(str::to_owned))
                        // Official-provider requests can use zstd. The real
                        // client's routing metadata identifies the same model;
                        // this rejection fixture need not decode the history.
                        .or_else(|| {
                            headers
                                .iter()
                                .find(|(name, _)| name == "x-codex-turn-metadata")
                                .and_then(|(_, metadata)| {
                                    serde_json::from_str::<Value>(metadata).ok()
                                })
                                .and_then(|value| value["model"].as_str().map(str::to_owned))
                        });
                    let captured = CapturedRequest {
                        request_line: first_line,
                        headers,
                        body,
                    };
                    let event = handler(&captured);
                    let _ = sender.send(captured);
                    let response = if reject_relay_ids
                        && model.as_deref().is_some_and(|model| model.contains("::"))
                    {
                        let body = json!({"detail":format!(
                            "The '{}' model is not supported when using Codex with a ChatGPT account.",
                            model.unwrap()
                        )})
                        .to_string();
                        format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", event.len(), event)
                    };
                    let _ = reader.get_mut().write_all(response.as_bytes());
                } else {
                    let _ = reader.get_mut().write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        });
        Self {
            port,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for MockResponses {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl AppServer {
    pub(super) fn start(
        binary: &Path,
        paths: &CodexPaths,
        cwd: &Path,
        proxy: &RejectingProxy,
    ) -> Self {
        let mut command = Command::new(binary);
        command.env_clear();
        // Preserve only OS runtime requirements, never auth, proxy or Codex settings.
        for name in [
            "SystemRoot",
            "WINDIR",
            "SystemDrive",
            "COMSPEC",
            "PATH",
            "PATHEXT",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "TEMP",
            "TMP",
        ] {
            command.env(name, cwd);
        }
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(name, &proxy.url);
        }
        for name in ["NO_PROXY", "no_proxy"] {
            command.env(name, "127.0.0.1,localhost,::1");
        }
        command
            .env("CODEX_HOME", &paths.codex_home)
            .env("DO_NOT_TRACK", "1")
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn isolated Codex app-server");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();
        let (tx, messages) = mpsc::channel();
        let stdout_reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let parsed = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if tx.send(parsed).is_err() {
                    break;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let captured_stderr = stderr.clone();
        let stderr_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let mut captured = captured_stderr.lock().unwrap();
                if captured.len() < 16_384 {
                    captured.push_str(&String::from_utf8_lossy(&buffer[..count]));
                }
            }
        });
        let mut server = Self {
            child,
            stdin: Some(stdin),
            messages,
            stderr,
            readers: vec![stdout_reader, stderr_reader],
        };
        server.request(1, "initialize", json!({
            "clientInfo": {"name": "agentharbor_auth_regression", "title": "Isolated regression test", "version": "1.0.0"}
        }));
        server.send(json!({"method": "initialized", "params": {}}));
        server
    }

    pub(super) fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    pub(super) fn turn(
        &mut self,
        thread_id: &str,
        model: &str,
        effort: &str,
        text: &str,
        timeout: Duration,
    ) -> Result<String, Value> {
        self.send(json!({"id":30,"method":"turn/start","params":{
            "threadId":thread_id,"model":model,"effort":effort,
            "input":[{"type":"text","text":text,"text_elements":[]}]
        }}));
        let deadline = Instant::now() + timeout;
        let mut text = String::new();
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| json!({"error":"turn timeout"}))?
                .map_err(|_| json!({"error":"invalid app-server output"}))?;
            if let Some(error) = message.get("error") {
                return Err(error.clone());
            }
            if message["params"]["threadId"] != thread_id {
                continue;
            }
            if message["method"] == "item/started"
                && matches!(
                    message["params"]["item"]["type"].as_str(),
                    Some("commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall")
                )
            {
                return Err(json!({"error":"unexpected tool in text-only test"}));
            }
            if message["method"] == "item/completed"
                && message["params"]["item"]["type"] == "agentMessage"
            {
                text.push_str(
                    message["params"]["item"]["text"]
                        .as_str()
                        .unwrap_or_default(),
                );
            }
            if message["method"] == "turn/completed" {
                return if message["params"]["turn"]["status"] == "completed" {
                    Ok(text)
                } else {
                    Err(message["params"]["turn"]["error"].clone())
                };
            }
        }
    }

    pub(super) fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.request_outcome(id, method, params)
            .unwrap_or_else(|error| panic!("Codex {method} failed: {error}"))
    }

    pub(super) fn request_outcome(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, Value> {
        self.send(json!({"id": id, "method": method, "params": params}));
        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self
                .messages
                .recv_timeout(remaining)
                .unwrap_or_else(|error| {
                    panic!(
                        "Codex {method} did not respond: {error}; stderr: {}",
                        self.stderr.lock().unwrap()
                    )
                })
                .expect("Codex stdout must contain JSON-RPC messages");
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(error.clone());
            }
            return Ok(message.get("result").expect("RPC result").clone());
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        for reader in self.readers.drain(..) {
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}

pub(super) fn synthetic_auth() -> Vec<u8> {
    synthetic_auth_for_plan("plus")
}

pub(super) fn synthetic_auth_for_plan(plan: &str) -> Vec<u8> {
    let now = chrono::Utc::now();
    let claims = json!({
        "email": FIXTURE_EMAIL, "email_verified": true,
        "sub": "synthetic-router-user", "iat": now.timestamp(), "exp": now.timestamp() + 3600,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": FIXTURE_ACCOUNT,
            "chatgpt_user_id": "synthetic-router-user", "user_id": "synthetic-router-user",
            "chatgpt_plan_type": plan
        }
    });
    // Saved claims are decoded locally. This is not a usable authentication token.
    let token = format!(
        "{}.{}.fixture",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    serde_json::to_vec_pretty(&json!({
        "auth_mode": "chatgpt", "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": token, "access_token": token,
            // Leave workspace selection unset: current Codex otherwise discovers
            // workspace routing over accounts/check before account/read. The
            // synthetic JWT still identifies the account, email and plan.
            "refresh_token": "synthetic-never-refresh", "account_id": null
        },
        "last_refresh": now.to_rfc3339()
    }))
    .unwrap()
}

pub(super) fn mock_stream(protocol: ModelProtocol) -> String {
    match protocol {
        ModelProtocol::Responses => {
            let part = json!({"type": "output_text", "text": STREAM_TEXT, "annotations": []});
            let item = json!({"id": "msg_fixture", "type": "message", "role": "assistant", "status": "completed", "content": [part.clone()]});
            [
                json!({"type": "response.created", "response": {"id": "resp_fixture", "object": "response", "created_at": 0, "model": "fixture-model", "status": "in_progress", "output": []}}),
                json!({"type": "response.output_item.added", "output_index": 0, "item": {"id": "msg_fixture", "type": "message", "role": "assistant", "status": "in_progress", "content": []}}),
                json!({"type": "response.content_part.added", "item_id": "msg_fixture", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}),
                json!({"type": "response.output_text.delta", "item_id": "msg_fixture", "output_index": 0, "content_index": 0, "delta": STREAM_TEXT}),
                json!({"type": "response.output_text.done", "item_id": "msg_fixture", "output_index": 0, "content_index": 0, "text": STREAM_TEXT}),
                json!({"type": "response.content_part.done", "item_id": "msg_fixture", "output_index": 0, "content_index": 0, "part": part}),
                json!({"type": "response.output_item.done", "output_index": 0, "item": item.clone()}),
                json!({"type": "response.completed", "response": {"id": "resp_fixture", "object": "response", "created_at": 0, "model": "fixture-model", "status": "completed", "output": [item], "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}}}),
            ].into_iter().enumerate().map(|(sequence, mut event)| {
                event["sequence_number"] = json!(sequence);
                format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())
            }).collect()
        },
        ModelProtocol::ChatCompletions => {
            let mut stream = [
                json!({"id":"chat_fixture","object":"chat.completion.chunk","model":"fixture-model","choices":[{"index":0,"delta":{"role":"assistant","content":"Local translated "},"finish_reason":null}]}),
                json!({"id":"chat_fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"stream completed."},"finish_reason":null}]}),
                json!({"id":"chat_fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                json!({"id":"chat_fixture","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}),
            ].into_iter().map(|value| format!("data: {value}\n\n")).collect::<String>();
            stream.push_str("data: [DONE]\n\n");
            stream
        },
        ModelProtocol::AnthropicMessages => [
            json!({"type":"message_start","message":{"id":"msg_fixture","type":"message","role":"assistant","model":"fixture-model","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Local translated "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"stream completed."}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}),
            json!({"type":"message_stop"}),
        ].into_iter().map(|value| format!("event: {}\ndata: {value}\n\n", value["type"].as_str().unwrap())).collect(),
    }
}
