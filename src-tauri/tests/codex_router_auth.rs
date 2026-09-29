//! Opt-in compatibility checks against an installed Codex CLI.
//!
//! Set AGENTHARBOR_TEST_CODEX_BIN to the absolute executable path, then run:
//! cargo test --test codex_router_auth -- --ignored --nocapture
//! All credentials are synthetic, all writable paths are temporary, and any
//! non-loopback HTTP traffic is sent to a local rejecting proxy.

use agentharbor_lib::core::router::{
    self, CodexAccessMode, ModelProtocol, Provider, RouterSettings, Store,
};
use agentharbor_lib::core::{
    models::{ApiProxyConfigPayload, ApiProxyMode},
    router_proxy,
};
use agentharbor_lib::platform::paths::CodexPaths;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const FIXTURE_EMAIL: &str = "router-regression@example.invalid";
const FIXTURE_ACCOUNT: &str = "00000000-0000-4000-8000-000000000001";
const LOCAL_TOKEN: &str = "agentharbor_synthetic_local_token";
const STREAM_TEXT: &str = "Local translated stream completed.";
// router_proxy is process-global; isolate the tests that start its listener.
static ROUTER_TEST_LOCK: Mutex<()> = Mutex::new(());

struct StopRouter;

impl Drop for StopRouter {
    fn drop(&mut self) {
        router_proxy::stop();
    }
}

/// The listener never opens an outbound connection, including for CONNECT.
struct RejectingProxy {
    url: String,
    requests: Receiver<String>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RejectingProxy {
    fn start() -> Self {
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
struct AppServer {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Result<Value, String>>,
    stderr: Arc<Mutex<String>>,
    readers: Vec<JoinHandle<()>>,
}

struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A loopback-only model endpoint. It records requests and returns a fixed
/// completion with no tools, instructions, or network calls.
struct MockResponses {
    port: u16,
    requests: Receiver<CapturedRequest>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MockResponses {
    fn start() -> Self {
        Self::for_protocol(ModelProtocol::Responses)
    }

    fn for_protocol(protocol: ModelProtocol) -> Self {
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
                    let _ = sender.send(CapturedRequest {
                        request_line: first_line,
                        headers,
                        body,
                    });
                    let event = mock_stream(protocol);
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", event.len(), event);
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
    fn start(binary: &Path, paths: &CodexPaths, cwd: &Path, proxy: &RejectingProxy) -> Self {
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

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.request_outcome(id, method, params)
            .unwrap_or_else(|error| panic!("Codex {method} failed: {error}"))
    }

    fn request_outcome(&mut self, id: u64, method: &str, params: Value) -> Result<Value, Value> {
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

fn synthetic_auth() -> Vec<u8> {
    let now = chrono::Utc::now();
    let claims = json!({
        "email": FIXTURE_EMAIL, "email_verified": true,
        "sub": "synthetic-router-user", "iat": now.timestamp(), "exp": now.timestamp() + 3600,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": FIXTURE_ACCOUNT,
            "chatgpt_user_id": "synthetic-router-user", "user_id": "synthetic-router-user",
            "chatgpt_plan_type": "plus"
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

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_preserves_chatgpt_account_across_router_modes() {
    let binary = PathBuf::from(
        std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN")
            .expect("set AGENTHARBOR_TEST_CODEX_BIN to an installed codex executable"),
    );
    assert!(
        binary.is_absolute() && binary.is_file(),
        "use an absolute Codex executable path"
    );
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n",
        proxy.url
    )).unwrap();
    let auth = synthetic_auth();
    fs::write(&paths.auth_path, &auth).unwrap();
    let mut provider = Provider::default();
    provider.id = "synthetic-provider".into();
    provider.name = "Synthetic local provider".into();
    provider.base_url = proxy.url.clone();
    provider.api_key = "synthetic-provider-key".into();
    provider.model = "fixture-model".into();
    provider.enabled = true;
    let mut store = Store {
        active_provider_id: Some(provider.id.clone()),
        providers: vec![provider],
        router: RouterSettings {
            enabled: true,
            bearer_token: LOCAL_TOKEN.into(),
            port: proxy.url.rsplit(':').next().unwrap().parse().unwrap(),
            ..Default::default()
        },
        ..Default::default()
    };

    router::apply_routing_config(&paths, &store).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert_eq!(result["account"]["type"], "chatgpt", "{result}");
        assert_eq!(result["account"]["email"], FIXTURE_EMAIL, "{result}");
        assert_eq!(result["account"]["planType"], "plus", "{result}");
        assert_eq!(result["requiresOpenaiAuth"], true, "{result}");
        eprintln!("signed-in routing: synthetic ChatGPT account remains visible");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);

    // Reproduce the original regression with the same synthetic login file.
    let mut legacy = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    let injected = &mut legacy["model_providers"][router::CATALOG_PROVIDER_KEY];
    injected["requires_openai_auth"] = toml_edit::value(false);
    injected["experimental_bearer_token"] = toml_edit::value(LOCAL_TOKEN);
    injected.as_table_mut().unwrap().remove("http_headers");
    fs::write(&paths.config_path, legacy.to_string()).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert!(
            result["account"].is_null(),
            "legacy regression not reproduced: {result}"
        );
        assert_eq!(result["requiresOpenaiAuth"], false, "{result}");
        eprintln!("legacy routing: account=null reproduced without removing credentials");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);

    store.router.accountless = true;
    router::apply_routing_config(&paths, &store).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert!(result["account"].is_null(), "accountless routing: {result}");
        assert_eq!(result["requiresOpenaiAuth"], false, "{result}");
        eprintln!("accountless routing: intentionally reports account=null");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);

    store.router.accountless = false;
    router::apply_routing_config(&paths, &store).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert_eq!(result["account"]["type"], "chatgpt", "{result}");
        assert_eq!(result["account"]["email"], FIXTURE_EMAIL, "{result}");
        assert_eq!(result["requiresOpenaiAuth"], true, "{result}");
        eprintln!("return to signed-in routing: synthetic ChatGPT account restored");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_sends_separate_router_auth_and_router_strips_it_upstream() {
    let _serial = ROUTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let binary = PathBuf::from(
        std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN")
            .expect("set AGENTHARBOR_TEST_CODEX_BIN to an installed codex executable"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    let mock = MockResponses::start();
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n", proxy.url
    )).unwrap();
    let auth = synthetic_auth();
    let auth_value: Value = serde_json::from_slice(&auth).unwrap();
    fs::write(&paths.auth_path, &auth).unwrap();
    let input = serde_json::from_value(json!({
        "name": "Synthetic response endpoint", "baseUrl": format!("http://127.0.0.1:{}", mock.port),
        "accessToken": "", "apiKey": "synthetic-upstream-key", "model": "gpt-6-astra",
        "networkMode": "direct"
    }))
    .unwrap();
    let saved = router::save_provider(&paths, input).unwrap();
    let provider_id = &saved.state.providers[0].id;
    router::set_enabled(&paths, provider_id, true).unwrap();
    router::set_smart_routing(&paths, true).unwrap();
    let mut store = router::load_store(&paths).unwrap();
    store.router.port = mock.port;
    router::apply_routing_config(&paths, &store).unwrap();

    let captured = {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let started = server.request(
            2,
            "thread/start",
            json!({
                "cwd": cwd.to_string_lossy(), "ephemeral": true,
                "approvalPolicy": "never", "sandbox": "read-only"
            }),
        );
        let thread_id = started["thread"]["id"].as_str().expect("thread id");
        server.request(3, "turn/start", json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": "Return the fixed local fixture response.", "text_elements": []}]
        }));
        mock.requests
            .recv_timeout(RPC_TIMEOUT)
            .unwrap_or_else(|error| {
                panic!(
                    "Codex did not call the local Responses fixture: {error}; stderr: {}",
                    server.stderr.lock().unwrap()
                )
            })
    };
    assert_eq!(
        captured.header("x-agentharbor-token"),
        Some(store.router.bearer_token.as_str())
    );
    let bearer = format!(
        "Bearer {}",
        auth_value["tokens"]["access_token"].as_str().unwrap()
    );
    assert_eq!(captured.header("authorization"), Some(bearer.as_str()));
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);

    // Replay the bytes emitted by the real CLI through the actual router into
    // the same local fixture. Neither leg can reach a real model service.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let router_port = runtime
        .block_on(router_proxy::ensure_running(
            0,
            paths.clone(),
            ApiProxyConfigPayload {
                mode: ApiProxyMode::Manual,
                url: Some(proxy.url.clone()),
            },
        ))
        .unwrap();
    let _router_guard = StopRouter;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(RPC_TIMEOUT)
        .build()
        .unwrap();
    let mut request = client.post(format!("http://127.0.0.1:{router_port}/codex/v1/responses"));
    for (name, value) in &captured.headers {
        if !matches!(
            name.as_str(),
            "host" | "content-length" | "connection" | "transfer-encoding"
        ) {
            request = request.header(name, value);
        }
    }
    let response = request.body(captured.body).send().unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let upstream = mock.requests.recv_timeout(RPC_TIMEOUT).unwrap();
    assert_eq!(
        upstream.header("authorization"),
        Some("Bearer synthetic-upstream-key")
    );
    assert!(upstream.header("x-agentharbor-token").is_none());
    assert!(upstream.header("chatgpt-account-id").is_none());
    assert!(!upstream.headers.iter().any(|(_, value)| value == &bearer));
    let body: Value = serde_json::from_slice(&upstream.body).unwrap();
    assert_eq!(body["model"], "gpt-6-astra");
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    eprintln!("real CLI sends separate local/OpenAI auth; actual router strips both and substitutes the provider key");
}

fn complete_fixture_turn(server: &mut AppServer, cwd: &Path) -> Result<(), Value> {
    let started = server.request_outcome(
        10,
        "thread/start",
        json!({
            "cwd": cwd.to_string_lossy(), "ephemeral": true,
            "approvalPolicy": "never", "sandbox": "read-only",
        }),
    )?;
    let thread_id = started["thread"]["id"].as_str().expect("thread id");
    server.send(json!({"id": 11, "method": "turn/start", "params": {
        "threadId": thread_id,
        "input": [{"type": "text", "text": "Return the fixed local fixture response without tools.", "text_elements": []}],
    }}));
    let deadline = Instant::now() + RPC_TIMEOUT;
    let mut turn_id = None;
    let mut completed = None;
    while turn_id.is_none() || completed.is_none() {
        let message = server
            .messages
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "Codex fixture turn did not finish: {error}; stderr: {}",
                    server.stderr.lock().unwrap()
                )
            })
            .expect("Codex stdout must contain JSON-RPC messages");
        if message.get("id").and_then(Value::as_u64) == Some(11) {
            if let Some(error) = message.get("error") {
                return Err(error.clone());
            }
            turn_id = Some(
                message["result"]["turn"]["id"]
                    .as_str()
                    .expect("turn id")
                    .to_owned(),
            );
            continue;
        }
        let params = &message["params"];
        if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
            continue;
        }
        match message.get("method").and_then(Value::as_str) {
            Some("turn/completed") => completed = Some(params["turn"].clone()),
            Some("error") => return Err(params.clone()),
            Some("item/started") => assert!(
                !matches!(
                    params["item"]["type"].as_str(),
                    Some("commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall")
                ),
                "fixture unexpectedly requested a tool: {message}"
            ),
            _ => {}
        }
    }
    let completed = completed.unwrap();
    assert_eq!(completed["id"].as_str(), turn_id.as_deref());
    if completed["status"] != "completed" || !completed["error"].is_null() {
        return Err(completed);
    }
    Ok(())
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_native_uses_dotenv_without_official_auth_and_restores_router_account() {
    let _serial = ROUTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let binary = PathBuf::from(
        std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN")
            .expect("set AGENTHARBOR_TEST_CODEX_BIN to an installed codex executable"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    let mock = MockResponses::start();
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n", proxy.url
    )).unwrap();
    let original_env = b"UNRELATED_NATIVE_FIXTURE=kept\n";
    let env_path = paths.codex_home.join(".env");
    fs::write(&env_path, original_env).unwrap();
    let auth = synthetic_auth();
    let auth_value: Value = serde_json::from_slice(&auth).unwrap();
    fs::write(&paths.auth_path, &auth).unwrap();
    let mut provider = Provider::default();
    provider.id = "synthetic-native-provider".into();
    provider.name = "Synthetic direct Responses endpoint".into();
    provider.base_url = format!("http://127.0.0.1:{}/", mock.port);
    provider.api_key = "synthetic-native-upstream-key".into();
    provider.protocol = ModelProtocol::Responses;
    provider.model = "fixture-model".into();
    provider.enabled = true;
    provider.network_mode = "direct".into();
    let mut store = Store {
        active_provider_id: Some(provider.id.clone()),
        providers: vec![provider],
        router: RouterSettings {
            enabled: true,
            bearer_token: LOCAL_TOKEN.into(),
            // A rejecting listener occupies the router endpoint. The successful
            // Native turn below must therefore connect directly to the mock.
            port: proxy.url.rsplit(':').next().unwrap().parse().unwrap(),
            ..Default::default()
        },
        ..Default::default()
    };
    router::apply_routing_config(&paths, &store).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert_eq!(result["account"]["type"], "chatgpt", "{result}");
        assert_eq!(result["account"]["email"], FIXTURE_EMAIL, "{result}");
        assert_eq!(result["requiresOpenaiAuth"], true, "{result}");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    store.codex_access_mode = CodexAccessMode::Native;
    router::apply_routing_config(&paths, &store).unwrap();
    let config_text = fs::read_to_string(&paths.config_path).unwrap();
    let config: toml::Value = config_text.parse().unwrap();
    let native = &config["model_providers"][router::NATIVE_PROVIDER_KEY];
    let env_key = native["env_key"].as_str().expect("native env_key");
    assert_eq!(
        config["model_provider"].as_str(),
        Some(router::NATIVE_PROVIDER_KEY)
    );
    assert_eq!(config["model"].as_str(), Some("fixture-model"));
    assert_eq!(
        native["base_url"].as_str(),
        Some(format!("http://127.0.0.1:{}/v1", mock.port).as_str())
    );
    assert_eq!(native["wire_api"].as_str(), Some("responses"));
    assert_eq!(native["requires_openai_auth"].as_bool(), Some(false));
    assert!(native.get("experimental_bearer_token").is_none());
    assert!(native.get("http_headers").is_none());
    assert!(!config_text.contains("synthetic-native-upstream-key"));
    assert!(!config_text.contains(LOCAL_TOKEN));
    let installed_env = fs::read(&env_path).unwrap();
    let variables: Vec<(String, String)> =
        dotenvy::from_read_iter(std::io::Cursor::new(&installed_env))
            .collect::<Result<_, _>>()
            .unwrap();
    assert_eq!(
        variables
            .iter()
            .find(|(name, _)| name == env_key)
            .map(|(_, value)| value.as_str()),
        Some("synthetic-native-upstream-key")
    );
    {
        // AppServer clears inherited environment variables, so this credential
        // can only be loaded from this isolated Codex home's generated .env.
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert!(
            result["account"].is_null(),
            "Native external-provider state: {result}"
        );
        assert_eq!(result["requiresOpenaiAuth"], false, "{result}");
        complete_fixture_turn(&mut server, &cwd)
            .unwrap_or_else(|error| panic!("Native fixture turn failed: {error}"));
    }
    let captured = mock.requests.recv_timeout(RPC_TIMEOUT).unwrap();
    assert_eq!(captured.request_line.trim(), "POST /v1/responses HTTP/1.1");
    assert_eq!(
        captured.header("authorization"),
        Some("Bearer synthetic-native-upstream-key")
    );
    for name in [
        "x-agentharbor-token",
        "chatgpt-account-id",
        "x-api-key",
        "cookie",
    ] {
        assert!(
            captured.header(name).is_none(),
            "unexpected Native header {name}"
        );
    }
    let official_token = auth_value["tokens"]["access_token"].as_str().unwrap();
    assert!(!captured
        .headers
        .iter()
        .any(|(_, value)| value.contains(official_token) || value.contains(LOCAL_TOKEN)));
    let body: Value = serde_json::from_slice(&captured.body).unwrap();
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], true);
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    assert!(
        matches!(mock.requests.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "Native fixture unexpectedly retried"
    );

    // Keeping auth.json while removing the generated credential proves that
    // the CLI does not fall back to the official login for this provider.
    fs::remove_file(&env_path).unwrap();
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let error =
            complete_fixture_turn(&mut server, &cwd).expect_err("missing native env_key must fail");
        assert!(
            error.to_string().contains(env_key),
            "missing credential error must identify env_key: {error}"
        );
    }
    assert!(
        matches!(mock.requests.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "missing env_key must not reach the upstream endpoint"
    );
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    assert!(
        !env_path.exists(),
        "Codex must not recreate the provider credential"
    );
    for request in proxy.requests.try_iter() {
        let request = request.to_ascii_lowercase();
        assert!(
            !request.starts_with("connect "),
            "Native must not attempt official network fallback: {request}"
        );
        assert!(
            !request.contains("/responses") && !request.contains("/chat/completions"),
            "Native inference must bypass the router and official endpoints: {request}"
        );
    }

    fs::write(&env_path, installed_env).unwrap();
    store.codex_access_mode = CodexAccessMode::Router;
    router::apply_routing_config(&paths, &store).unwrap();
    assert_eq!(fs::read(&env_path).unwrap(), original_env);
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let result = server.request(2, "account/read", json!({"refreshToken": false}));
        assert_eq!(result["account"]["type"], "chatgpt", "{result}");
        assert_eq!(result["account"]["email"], FIXTURE_EMAIL, "{result}");
        assert_eq!(result["account"]["planType"], "plus", "{result}");
        assert_eq!(result["requiresOpenaiAuth"], true, "{result}");
    }
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    eprintln!("real CLI Native uses its isolated .env provider key, rejects missing credentials without fallback, and restores the ChatGPT account when returning to Router");
}

fn mock_stream(protocol: ModelProtocol) -> String {
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

fn assert_real_codex_translated_text(protocol: ModelProtocol) {
    let _serial = ROUTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let binary = PathBuf::from(
        std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN")
            .expect("set AGENTHARBOR_TEST_CODEX_BIN to an installed codex executable"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    let mock = MockResponses::for_protocol(protocol);
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n", proxy.url
    )).unwrap();
    let auth = synthetic_auth();
    fs::write(&paths.auth_path, &auth).unwrap();
    let input = serde_json::from_value(json!({
        "name":"Synthetic streaming endpoint", "baseUrl":format!("http://127.0.0.1:{}", mock.port),
        "accessToken":"", "apiKey":"synthetic-upstream-key", "model":"fixture-model",
        "networkMode":"direct", "protocol":protocol,
    }))
    .unwrap();
    let saved = router::save_provider(&paths, input).unwrap();
    router::set_enabled(&paths, &saved.state.providers[0].id, true).unwrap();
    router::set_smart_routing(&paths, true).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let port = runtime
        .block_on(router_proxy::ensure_running(
            0,
            paths.clone(),
            ApiProxyConfigPayload {
                mode: ApiProxyMode::Manual,
                url: Some(proxy.url.clone()),
            },
        ))
        .unwrap();
    let _router_guard = StopRouter;
    let mut store = router::load_store(&paths).unwrap();
    store.router.port = port;
    router::apply_routing_config(&paths, &store).unwrap();

    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let started = server.request(
        2,
        "thread/start",
        json!({
            "cwd":cwd.to_string_lossy(), "ephemeral":true,
            "approvalPolicy":"never", "sandbox":"read-only",
        }),
    );
    let thread_id = started["thread"]["id"].as_str().expect("thread id");
    // Drain the RPC reply and notifications together so fast local replies do
    // not lose text events while waiting for the turn/start result.
    server.send(json!({"id":3,"method":"turn/start","params":{
        "threadId":thread_id,
        "input":[{"type":"text","text":"Return the fixed fixture text without calling tools.","text_elements":[]}],
    }}));
    let deadline = Instant::now() + RPC_TIMEOUT;
    let mut turn_id = None;
    let mut completed_id = None;
    let mut deltas = String::new();
    let mut final_text = String::new();
    while turn_id.is_none() || completed_id.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = server.messages.recv_timeout(remaining).unwrap_or_else(|error| {
            panic!("Codex {protocol:?} stream did not complete: {error}; deltas={deltas:?}; stderr: {}", server.stderr.lock().unwrap())
        }).expect("Codex stdout must contain JSON-RPC messages");
        if message.get("id").and_then(Value::as_u64) == Some(3) {
            assert!(
                message.get("error").is_none(),
                "turn/start failed: {message}"
            );
            turn_id = Some(
                message["result"]["turn"]["id"]
                    .as_str()
                    .expect("turn id")
                    .to_owned(),
            );
            continue;
        }
        let params = &message["params"];
        if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
            continue;
        }
        match message.get("method").and_then(Value::as_str) {
            Some("item/agentMessage/delta") => {
                deltas.push_str(params["delta"].as_str().expect("text delta"))
            }
            Some("item/completed") if params["item"]["type"] == "agentMessage" => {
                final_text.push_str(
                    params["item"]["text"]
                        .as_str()
                        .expect("completed assistant text"),
                );
            }
            Some("turn/completed") => {
                assert_eq!(
                    params["turn"]["status"], "completed",
                    "{protocol:?}: {message}"
                );
                assert!(params["turn"]["error"].is_null(), "{protocol:?}: {message}");
                completed_id = Some(
                    params["turn"]["id"]
                        .as_str()
                        .expect("completed turn id")
                        .to_owned(),
                );
            }
            Some("error") => panic!("Codex {protocol:?} reported an error: {message}"),
            Some("item/started") => {
                assert!(
                    !matches!(
                        params["item"]["type"].as_str(),
                        Some("commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall")
                    ),
                    "fixture unexpectedly requested a tool: {message}"
                );
            }
            _ => {}
        }
    }
    assert_eq!(turn_id, completed_id);
    assert_eq!(deltas, STREAM_TEXT, "{protocol:?} streaming text");
    assert_eq!(final_text, STREAM_TEXT, "{protocol:?} completed text");
    let upstream = mock.requests.recv_timeout(RPC_TIMEOUT).unwrap();
    let body: Value = serde_json::from_slice(&upstream.body).unwrap();
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], true);
    assert!(!body["messages"]
        .as_array()
        .expect("translated messages")
        .is_empty());
    assert!(upstream.header("x-agentharbor-token").is_none());
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    eprintln!("real CLI completed {protocol:?} through actual router and received {STREAM_TEXT:?}");
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_completes_chat_stream_through_actual_router() {
    assert_real_codex_translated_text(ModelProtocol::ChatCompletions);
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_completes_anthropic_stream_through_actual_router() {
    assert_real_codex_translated_text(ModelProtocol::AnthropicMessages);
}
