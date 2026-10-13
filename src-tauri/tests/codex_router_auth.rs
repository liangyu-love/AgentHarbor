//! Opt-in compatibility checks against an installed Codex CLI.
//!
//! Set AGENTHARBOR_TEST_CODEX_BIN to the absolute executable path, then run:
//! cargo test --test codex_router_auth real_codex -- --ignored --nocapture
//! All credentials are synthetic, all writable paths are temporary, and any
//! non-loopback HTTP traffic is sent to a local rejecting proxy. The separate
//! `live_codex_switches_configured_relays` test requires explicit live opt-in;
//! its Router (not the child CLI) can reach the two selected providers.

use agentharbor_lib::core::router::{
    self, CodexAccessMode, ModelProtocol, Provider, RouterSettings, Store,
};
use agentharbor_lib::core::{
    models::{ApiProxyConfigPayload, ApiProxyMode},
    router_proxy,
};
use agentharbor_lib::platform::paths::CodexPaths;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

#[path = "support/codex.rs"]
mod codex_support;
use codex_support::*;

struct StopRouter;

impl Drop for StopRouter {
    fn drop(&mut self) {
        router_proxy::stop();
    }
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
    // The built-in provider cannot add headers; the local token is in the URL.
    assert!(captured.header("x-agentharbor-token").is_none());
    assert!(
        captured.request_line.contains(&format!("/openai/{}/v1/responses", store.router.bearer_token)),
        "{}",
        captured.request_line
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
    let mut request = client.post(format!(
        "http://127.0.0.1:{router_port}/openai/{}/v1/responses",
        store.router.bearer_token
    ));
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
    eprintln!("real CLI sends the local token in its URL and OpenAI auth in headers; the actual router decodes the body, strips both and substitutes the provider key");
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TranslationFixture {
    Text,
    Structured,
    InvalidStream,
}

fn assert_real_codex_translated_text(
    protocol: ModelProtocol,
    context_override: Option<u64>,
    fixture: TranslationFixture,
) {
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
    let mock = if fixture == TranslationFixture::InvalidStream {
        MockResponses::with_stream(protocol, "data: {invalid JSON}\n\n".into())
    } else {
        MockResponses::for_protocol(protocol)
    };
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nweb_search = 'disabled'\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n", proxy.url
    )).unwrap();
    let auth = synthetic_auth();
    fs::write(&paths.auth_path, &auth).unwrap();
    let model = if protocol == ModelProtocol::AnthropicMessages {
        "claude-haiku-4-5"
    } else {
        "fixture-model"
    };
    let input = serde_json::from_value(json!({
        "name":"Synthetic streaming endpoint", "baseUrl":format!("http://127.0.0.1:{}", mock.port),
        "accessToken":"", "apiKey":"synthetic-upstream-key", "model":model,
        "models":[{"id":model,"contextWindow":context_override,"metadataSource":if context_override.is_some() { "manual" } else { "local" }}],
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
    let mut params = json!({
        "threadId":thread_id,
        "input":[{"type":"text","text":"Return the fixed fixture text without calling tools.","text_elements":[]}],
    });
    let schema = json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false});
    if fixture == TranslationFixture::Structured {
        params["outputSchema"] = schema.clone();
    }
    server.send(json!({"id":3,"method":"turn/start","params":params}));
    let deadline = Instant::now() + RPC_TIMEOUT;
    let mut turn_id = None;
    let mut completed_id = None;
    let mut deltas = String::new();
    let mut final_text = String::new();
    let mut effective_context = None;
    while turn_id.is_none()
        || completed_id.is_none()
        || (fixture != TranslationFixture::InvalidStream && effective_context.is_none())
    {
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
            Some("thread/tokenUsage/updated") => {
                effective_context = params["tokenUsage"]["modelContextWindow"].as_u64();
            }
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
                    params["turn"]["status"],
                    if fixture == TranslationFixture::InvalidStream {
                        "failed"
                    } else {
                        "completed"
                    },
                    "{protocol:?}: {message}"
                );
                if fixture == TranslationFixture::InvalidStream {
                    assert!(
                        params["turn"]["error"]["message"]
                            .as_str()
                            .unwrap()
                            .contains("invalid stream frame"),
                        "{message}"
                    );
                } else {
                    assert!(params["turn"]["error"].is_null(), "{protocol:?}: {message}");
                }
                completed_id = Some(
                    params["turn"]["id"]
                        .as_str()
                        .expect("completed turn id")
                        .to_owned(),
                );
            }
            Some("error") if fixture == TranslationFixture::InvalidStream => {
                assert_eq!(
                    params["willRetry"], false,
                    "deterministic failures must not retry: {message}"
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
    let expected_text = if fixture == TranslationFixture::InvalidStream {
        ""
    } else {
        STREAM_TEXT
    };
    assert_eq!(deltas, expected_text, "{protocol:?} streaming text");
    assert_eq!(final_text, expected_text, "{protocol:?} completed text");
    let upstream = mock.requests.recv_timeout(RPC_TIMEOUT).unwrap();
    let body: Value = serde_json::from_slice(&upstream.body).unwrap();
    assert_eq!(body["model"], model);
    assert_eq!(body["stream"], true);
    if fixture == TranslationFixture::Structured {
        assert_eq!(
            if protocol == ModelProtocol::ChatCompletions {
                &body["response_format"]["json_schema"]["schema"]
            } else {
                &body["output_config"]["format"]["schema"]
            },
            &schema
        );
    }
    assert!(!body["messages"]
        .as_array()
        .expect("translated messages")
        .is_empty());
    assert!(upstream.header("x-agentharbor-token").is_none());
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    if fixture == TranslationFixture::InvalidStream {
        assert!(
            mock.requests.try_recv().is_err(),
            "Codex retried a deterministic translation failure"
        );
        return;
    }
    let context = context_override.unwrap_or(if protocol == ModelProtocol::AnthropicMessages {
        200_000
    } else {
        128_000
    });
    // The installed Codex applies its 95% usable-context margin to catalog limits.
    assert_eq!(effective_context, Some(context * 95 / 100));
    eprintln!("real CLI completed {protocol:?} through actual router: model={model}, context={context}, effective_context={effective_context:?}, text={STREAM_TEXT:?}");
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_completes_chat_stream_through_actual_router() {
    assert_real_codex_translated_text(
        ModelProtocol::ChatCompletions,
        None,
        TranslationFixture::Text,
    );
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_completes_anthropic_stream_through_actual_router() {
    assert_real_codex_translated_text(
        ModelProtocol::AnthropicMessages,
        None,
        TranslationFixture::Text,
    );
    assert_real_codex_translated_text(
        ModelProtocol::AnthropicMessages,
        Some(200_000),
        TranslationFixture::Text,
    );
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_output_schema_reaches_both_translated_upstreams() {
    assert_real_codex_translated_text(
        ModelProtocol::ChatCompletions,
        None,
        TranslationFixture::Structured,
    );
    assert_real_codex_translated_text(
        ModelProtocol::AnthropicMessages,
        None,
        TranslationFixture::Structured,
    );
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_does_not_retry_deterministic_translated_stream_errors() {
    assert_real_codex_translated_text(
        ModelProtocol::ChatCompletions,
        None,
        TranslationFixture::InvalidStream,
    );
    assert_real_codex_translated_text(
        ModelProtocol::AnthropicMessages,
        None,
        TranslationFixture::InvalidStream,
    );
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_reasoning_menu_uses_model_identity_across_channels() {
    let _serial = ROUTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    fs::write(&paths.config_path, "check_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\n").unwrap();
    let mut store = Store {
        providers: vec![serde_json::from_value(json!({
            "id": "synthetic-reasoning", "name": "Reasoning fixture",
            "baseUrl": "http://127.0.0.1:1", "apiKey": "synthetic-key",
            "protocol": "responses", "model": "gpt-6.1-sol", "enabled": true,
            "models": [
                {"id":"gpt-6.1-sol"},
                {"id":"gpt-6-sol", "overrides":{"reasoning":[{"effort":"low","description":"Manual low"}],"defaultEffort":"low"}},
                {"id":"gpt-6-luna", "overrides":{"reasoning":[]}}
            ]
        })).unwrap()],
        active_provider_id: Some("synthetic-reasoning".into()),
        router: RouterSettings { enabled: true, accountless: true, bearer_token: LOCAL_TOKEN.into(), ..Default::default() },
        ..Default::default()
    };
    for mode in [CodexAccessMode::Router, CodexAccessMode::Native] {
        store.codex_access_mode = mode;
        router::apply_routing_config(&paths, &store).unwrap();
        let config: toml::Value = fs::read_to_string(&paths.config_path)
            .unwrap()
            .parse()
            .unwrap();
        let catalog: Value = serde_json::from_slice(
            &fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let menu = server.request(2, "model/list", json!({}));
        for (id, expected) in [
            ("gpt-6.1-sol", vec![]),
            ("gpt-6-sol", vec!["low"]),
            ("gpt-6-luna", vec![]),
        ] {
            let id = if mode == CodexAccessMode::Router {
                format!("synthetic-reasoning::{id}")
            } else {
                id.to_owned()
            };
            let model = menu["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["id"] == id)
                .unwrap_or_else(|| panic!("Missing {id} in {menu}"));
            let efforts: Vec<_> = model["supportedReasoningEfforts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["reasoningEffort"].as_str().unwrap())
                .collect();
            let expected = if id.ends_with("gpt-6.1-sol") {
                catalog["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["slug"] == id)
                    .unwrap()["supported_reasoning_levels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|level| level["effort"].as_str().unwrap())
                    .collect()
            } else {
                expected
            };
            assert_eq!(efforts, expected, "{id}: {model}");
        }
    }
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_switches_an_existing_official_thread_to_relays_without_adoption() {
    fn turn(server: &mut AppServer, thread_id: &str, model: &str, text: &str) {
        server
            .turn(thread_id, model, "high", text, RPC_TIMEOUT)
            .unwrap();
    }

    let _serial = ROUTER_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    let direct = MockResponses::official_account();
    let relay_a = MockResponses::start();
    let relay_b = MockResponses::start();
    fs::write(&paths.config_path, format!(
        "model = 'gpt-6-astra'\nmodel_provider = 'openai'\nopenai_base_url = 'http://127.0.0.1:{}/v1'\ncli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\nresponses_websockets = false\nresponses_websockets_v2 = false\n", direct.port
    )).unwrap();
    let auth = synthetic_auth_for_plan("free");
    fs::write(&paths.auth_path, &auth).unwrap();
    let thread_id = {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let started = server.request(
            2,
            "thread/start",
            json!({
                "cwd":cwd.to_string_lossy(),"approvalPolicy":"never","sandbox":"read-only"
            }),
        );
        let id = started["thread"]["id"].as_str().unwrap().to_owned();
        turn(
            &mut server,
            &id,
            "gpt-6-astra",
            "SYNTHETIC_ORIGINAL_CONTEXT",
        );
        direct.requests.recv_timeout(RPC_TIMEOUT).unwrap();
        let outcome = server.turn(
            &id, "synthetic-relay::gpt-6.1-sol", "high",
            "Switch to a relay without adopting the provider.", RPC_TIMEOUT,
        );
        let rejected = direct.requests.recv_timeout(RPC_TIMEOUT).unwrap();
        let error = outcome.expect_err("an official account endpoint must reject relay IDs");
        assert!(error.to_string().contains("not supported when using Codex with a ChatGPT account"), "{error}");
        let metadata: Value = serde_json::from_str(rejected.header("x-codex-turn-metadata").unwrap()).unwrap();
        assert_eq!(metadata["model"], "synthetic-relay::gpt-6.1-sol");
        assert!(rejected.header("x-agentharbor-token").is_none());
        id
    };
    let mut aliases = Vec::new();
    for (index, mock) in [&relay_a, &relay_b].into_iter().enumerate() {
        let saved = router::save_provider(&paths, serde_json::from_value(json!({
            "name":format!("Relay {index}"),"baseUrl":format!("http://127.0.0.1:{}", mock.port),
            "apiKey":format!("synthetic-relay-{index}"),"accessToken":"","model":"gpt-6-astra","networkMode":"direct",
            "contextWindow":(index == 0).then_some(400_000),"autoCompactLimit":(index == 0).then_some(350_000)
        })).unwrap()).unwrap();
        let provider = saved
            .state
            .providers
            .iter()
            .find(|p| p.name == format!("Relay {index}"))
            .unwrap();
        router::set_enabled(&paths, &provider.id, true).unwrap();
        aliases.push(format!("{}::gpt-6-astra", provider.id));
    }
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

    // A complete synthetic official cache entry verifies menu coexistence
    // without contacting a model service or reading the user's real cache.
    let config: toml::Value = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap();
    let catalog: Value =
        serde_json::from_slice(&fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap())
            .unwrap();
    let mut official = catalog["models"][0].clone();
    official["slug"] = json!("gpt-6-astra");
    official["display_name"] = json!("Synthetic official Astra");
    official["supported_reasoning_levels"] = json!([
        {"effort":"low","description":"Low"},{"effort":"medium","description":"Medium"},
        {"effort":"high","description":"High"},{"effort":"xhigh","description":"Extra high"},
        {"effort":"max","description":"Max"},{"effort":"ultra","description":"Ultra"}
    ]);
    official["multi_agent_reasoning_effort"] = json!("xhigh");
    fs::write(
        paths.codex_home.join("models_cache.json"),
        json!({"models":[official]}).to_string(),
    )
    .unwrap();
    router::apply_routing_config(&paths, &store).unwrap();
    let catalog: Value = serde_json::from_slice(
        &fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap(),
    ).unwrap();

    // The thread was created on the official endpoint and keeps its provider.
    // Routing only moved the built-in provider's endpoint, so no adoption runs.
    let connection = rusqlite::Connection::open(&paths.codex_state_db_path).unwrap();
    let provider: String = connection
        .query_row(
            "SELECT model_provider FROM threads WHERE id=?1",
            [&thread_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "openai");
    drop(connection);

    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let resumed = server.request(2, "thread/resume", json!({"threadId":thread_id}));
        assert_eq!(resumed["thread"]["id"], thread_id);
        assert_eq!(resumed["modelProvider"], "openai");
        let menu = server.request(3, "model/list", json!({}));
        assert!(
            menu["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|model| model["id"] == "gpt-6-astra"),
            "{menu}"
        );
        for alias in &aliases {
            let model = menu["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["id"].as_str() == Some(alias.as_str()))
                .unwrap_or_else(|| panic!("missing relay model in Codex menu: {menu}"));
            let efforts: Vec<_> = model["supportedReasoningEfforts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["reasoningEffort"].as_str().unwrap())
                .collect();
            // Compare the client's menu with the resolver's declared catalog;
            // catalog updates may resolve identities that used to be ambiguous.
            let declared = catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["slug"].as_str() == Some(alias))
                .unwrap();
            let expected: Vec<_> = declared["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect();
            assert_eq!(efforts, expected, "{model}");
            assert!(!efforts.contains(&"none"));
        }
        for (index, effort, wire_effort) in [
            (0, "high", "high"), (1, "xhigh", "xhigh"),
            (0, "max", "max"), (1, "high", "high"), (0, "xhigh", "xhigh"),
        ] {
            server.turn(&thread_id, &aliases[index], effort,
                "Continue the same synthetic history.", RPC_TIMEOUT).unwrap();
            let mock = if index == 0 { &relay_a } else { &relay_b };
            let captured = mock.requests.recv_timeout(RPC_TIMEOUT).unwrap();
            let body: Value = serde_json::from_slice(&captured.body).unwrap();
            assert_eq!(body["model"], "gpt-6-astra");
            assert_eq!(body["reasoning"]["effort"], wire_effort);
            assert!(body["input"]
                .to_string()
                .contains("SYNTHETIC_ORIGINAL_CONTEXT"));
            assert!(body.get("previous_response_id").is_none());
            assert_eq!(
                captured.header("authorization"),
                Some(format!("Bearer synthetic-relay-{index}").as_str())
            );
            assert!(captured.header("x-agentharbor-token").is_none());
            assert!(captured.header("chatgpt-account-id").is_none());
        }
    }
    // A later cold resume still uses Router; changing models never rewrites the provider.
    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let resumed = server.request(2, "thread/resume", json!({"threadId":thread_id}));
    assert_eq!(resumed["thread"]["id"], thread_id);
    assert_eq!(resumed["modelProvider"], "openai");
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    assert!(
        direct.requests.try_recv().is_err(),
        "routed turns must not reach the old endpoint"
    );
}

/// This test sends three small, billable synthetic turns through the production
/// Router. Real credentials exist only in its temporary encrypted provider store;
/// the source store/config/history are never written. No official auth is copied.
#[test]
#[ignore = "requires explicit live provider/model pairs and AGENTHARBOR_LIVE_ROUTING=1"]
fn live_codex_switches_configured_relays() {
    assert_eq!(
        std::env::var("AGENTHARBOR_LIVE_ROUTING").as_deref(),
        Ok("1")
    );
    let _serial = ROUTER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let source_path =
        PathBuf::from(std::env::var_os("AGENTHARBOR_PROBE_STORE").expect("explicit source store"));
    let before = fs::read(&source_path).unwrap();
    let source = router::read_store(&source_path).unwrap();
    let selections: Vec<(String, String)> = serde_json::from_str(
        &std::env::var("AGENTHARBOR_LIVE_MODELS").expect("two [provider ID, model] pairs"),
    )
    .unwrap();
    assert_eq!(selections.len(), 2);
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.config_path, "cli_auth_credentials_store='file'\ncheck_for_update_on_startup=false\n[features]\napps=false\nplugins=false\n[analytics]\nenabled=false\n[feedback]\nenabled=false\n").unwrap();
    fs::write(&paths.auth_path, synthetic_auth()).unwrap();
    let mut aliases = Vec::new();
    for (index, (id, model)) in selections.iter().enumerate() {
        let provider = source
            .providers
            .iter()
            .find(|p| p.id == *id && p.enabled)
            .expect("enabled configured provider");
        assert!(provider.models.iter().any(|m| m.id == *model && m.included));
        assert!(provider.base_url.starts_with("https://"));
        let saved = router::save_provider(&paths, serde_json::from_value(json!({
            "name":format!("Live fixture {index}"),"baseUrl":provider.base_url,
            "apiKey":provider.api_key,"accessToken":"","model":model,"networkMode":provider.network_mode,
            "protocol":provider.protocol_for_model(model)
        })).unwrap()).unwrap();
        let id = &saved
            .state
            .providers
            .iter()
            .find(|p| p.name == format!("Live fixture {index}"))
            .unwrap()
            .id;
        router::set_enabled(&paths, id, true).unwrap();
        aliases.push(format!("{id}::{model}"));
    }
    router::set_smart_routing(&paths, true).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let outbound = std::env::var("AGENTHARBOR_PROBE_PROXY").ok();
    let port = runtime
        .block_on(router_proxy::ensure_running(
            0,
            paths.clone(),
            ApiProxyConfigPayload {
                mode: if outbound.is_some() {
                    ApiProxyMode::Manual
                } else {
                    ApiProxyMode::System
                },
                url: outbound,
            },
        ))
        .unwrap();
    let _guard = StopRouter;
    let mut store = router::load_store(&paths).unwrap();
    store.router.port = port;
    router::apply_routing_config(&paths, &store).unwrap();
    let proxy = RejectingProxy::start();
    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let started = server.request(2, "thread/start", json!({
        "cwd":cwd,"model":aliases[0],"approvalPolicy":"never","sandbox":"read-only",
        "baseInstructions":"This is a synthetic connectivity test. Answer only in plain text. Never use tools or read files."
    }));
    let id = started["thread"]["id"].as_str().unwrap();
    assert_eq!(started["modelProvider"], router::CATALOG_PROVIDER_KEY);
    let marker = format!("PROBE_{}", uuid::Uuid::new_v4().simple());
    let mut result = Vec::new();
    for (turn, index) in [0, 1, 0].into_iter().enumerate() {
        let prompt = if turn == 0 {
            format!("Remember the synthetic marker {marker}. Reply with only OK. Do not use tools.")
        } else {
            "What was the synthetic marker in my first message? Reply with only that marker. Do not use tools.".into()
        };
        let outcome = server.turn(id, &aliases[index], "low", &prompt, Duration::from_secs(60));
        let mut row = json!({"turn":turn,"model":selections[index].1,"completed":outcome.is_ok(),"marker_retained":outcome.as_ref().is_ok_and(|text| text.contains(&marker))});
        if let Err(error) = &outcome {
            let mut error = error.to_string();
            for provider in &source.providers {
                for secret in [&provider.api_key, &provider.access_token] {
                    if !secret.is_empty() {
                        error = error.replace(secret, "[redacted]");
                    }
                }
            }
            row["error"] = json!(error.chars().take(700).collect::<String>());
        }
        eprintln!("LIVE_ROUTING {row}");
        let success = outcome.is_ok()
            && (turn == 0 || outcome.as_ref().is_ok_and(|text| text.contains(&marker)));
        result.push(row);
        if !success {
            break;
        }
    }
    assert_eq!(
        fs::read(source_path).unwrap(),
        before,
        "source provider store changed"
    );
    if let Some(path) = std::env::var_os("AGENTHARBOR_PROBE_REPORT") {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("choose a new report path; never overwrite an existing file");
        serde_json::to_writer_pretty(
            file,
            &json!({"same_thread_id":id,"provider":started["modelProvider"],"turns":result}),
        )
        .unwrap();
    }
    assert_eq!(
        result.len(),
        3,
        "a live turn failed; inspect the sanitized report"
    );
    assert!(result.iter().all(|row| row["completed"] == true));
    assert!(result[1..].iter().all(|row| row["marker_retained"] == true));
}

/// One Codex turn: its completed items and the final turn.
fn search_turn(server: &mut AppServer, id: u64, thread_id: &str, text: &str) -> (Vec<Value>, Value) {
    server.send(json!({"id":id,"method":"turn/start","params":{"threadId":thread_id,
        "input":[{"type":"text","text":text,"text_elements":[]}]}}));
    let deadline = Instant::now() + RPC_TIMEOUT;
    let mut items = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = server.messages.recv_timeout(remaining).unwrap_or_else(|error| {
            panic!("Codex search turn did not complete: {error}; stderr: {}", server.stderr.lock().unwrap())
        }).expect("Codex stdout must contain JSON-RPC messages");
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            assert!(message.get("error").is_none(), "turn/start failed: {message}");
            continue;
        }
        let params = &message["params"];
        if params.get("threadId").and_then(Value::as_str) != Some(thread_id) { continue; }
        match message.get("method").and_then(Value::as_str) {
            Some("item/completed") => items.push(params["item"].clone()),
            Some("turn/completed") => return (items, params["turn"].clone()),
            Some("error") => panic!("Codex reported an error: {message}"),
            _ => {}
        }
    }
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_default_hosted_search_runs_on_a_messages_relay_and_continues() {
    // Default Codex config: hosted web_search in cached mode (no Search
    // Service). A Messages relay runs it as live native search; Codex must
    // accept the translated search, the merged cited answer and continue.
    let _serial = ROUTER_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let binary = PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN")
        .expect("set AGENTHARBOR_TEST_CODEX_BIN to an installed codex executable"));
    assert!(binary.is_absolute() && binary.is_file());
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let proxy = RejectingProxy::start();
    let cite = |url: &str, title: &str| json!({"type":"web_search_result_location","url":url,"title":title,"cited_text":"x","encrypted_index":"e"});
    let mock = MockResponses::with_stream(ModelProtocol::AnthropicMessages, [
        json!({"type":"message_start","message":{"id":"msg_search","type":"message","role":"assistant","model":"claude-haiku-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_01fixture","name":"web_search","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"Rust release\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"web_search_tool_result","tool_use_id":"srvtoolu_01fixture","content":[
            {"type":"web_search_result","url":"https://www.rust-lang.org/","title":"Rust","encrypted_content":"opaque","page_age":null}]}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Rust 1.99 shipped"}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":cite("https://www.rust-lang.org/","Rust")}}),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"content_block_start","index":3,"content_block":{"type":"text","text":" with fixes."}}),
        json!({"type":"content_block_stop","index":3}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5,"server_tool_use":{"web_search_requests":1}}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(|value| format!("event: {}\ndata: {value}\n\n", value["type"].as_str().unwrap())).collect());
    fs::write(&paths.config_path, format!(
        "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\nchatgpt_base_url = '{}'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n", proxy.url
    )).unwrap();
    fs::write(&paths.auth_path, synthetic_auth()).unwrap();
    let input = serde_json::from_value(json!({
        "name":"Synthetic search relay", "baseUrl":format!("http://127.0.0.1:{}", mock.port),
        "accessToken":"", "apiKey":"synthetic-upstream-key", "model":"claude-haiku-4-5",
        "models":[{"id":"claude-haiku-4-5","metadataSource":"local"}],
        "networkMode":"direct", "protocol":ModelProtocol::AnthropicMessages,
    })).unwrap();
    let saved = router::save_provider(&paths, input).unwrap();
    router::set_enabled(&paths, &saved.state.providers[0].id, true).unwrap();
    router::set_smart_routing(&paths, true).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let port = runtime.block_on(router_proxy::ensure_running(0, paths.clone(),
        ApiProxyConfigPayload { mode: ApiProxyMode::Manual, url: Some(proxy.url.clone()) })).unwrap();
    let _router_guard = StopRouter;
    let mut store = router::load_store(&paths).unwrap();
    store.router.port = port;
    router::apply_routing_config(&paths, &store).unwrap();

    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let started = server.request(2, "thread/start", json!({"cwd":cwd.to_string_lossy(), "ephemeral":true,
        "approvalPolicy":"never", "sandbox":"read-only"}));
    let thread_id = started["thread"]["id"].as_str().expect("thread id").to_owned();
    let (items, turn) = search_turn(&mut server, 3, &thread_id, "Search the web for the latest Rust release.");
    assert_eq!(turn["status"], "completed", "{turn}");
    let first: Value = serde_json::from_slice(&mock.requests.recv_timeout(RPC_TIMEOUT).unwrap().body).unwrap();
    let search_tool = first["tools"].as_array().unwrap().iter().find(|tool| tool["type"] == "web_search_20250305")
        .unwrap_or_else(|| panic!("hosted search was not mapped: {}", first["tools"]));
    assert_eq!(search_tool["name"], "web_search");
    let searches: Vec<_> = items.iter().filter(|item| item["type"] == "webSearch").collect();
    assert_eq!(searches.len(), 1, "{items:?}");
    assert!(searches[0].to_string().contains("Rust release"), "{}", searches[0]);
    let answer: String = items.iter().filter(|item| item["type"] == "agentMessage")
        .filter_map(|item| item["text"].as_str()).collect();
    assert_eq!(answer, "Rust 1.99 shipped with fixes.\n\nSources:\n- [Rust](https://www.rust-lang.org/)", "{items:?}");

    // The next turn replays the stored search item; it must not fail.
    let (_, turn) = search_turn(&mut server, 4, &thread_id, "Summarize it in one line.");
    assert_eq!(turn["status"], "completed", "{turn}");
    let second: Value = serde_json::from_slice(&mock.requests.recv_timeout(RPC_TIMEOUT).unwrap().body).unwrap();
    let history = second["messages"].to_string();
    assert!(history.contains("[Previous web search]") && history.contains("Rust release"), "{history}");
    assert!(history.contains("Sources:"), "{history}");
    eprintln!("real Codex hosted search through Messages relay: tools={}, items={}", first["tools"].as_array().unwrap().len(), items.len());
}
