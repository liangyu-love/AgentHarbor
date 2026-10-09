//! Real Codex through the built-in provider: remote compaction (V2) on relay
//! and official turns, then switching channels on the compacted thread.
use super::*;
use crate::core::router::{ModelProtocol, Provider, RouterSettings, Store};
use std::{fs, time::Duration};

#[path = "../../../tests/support/codex.rs"]
mod fixtures;
use fixtures::*;

const OFFICIAL_COMPACTION: &str = "synthetic-official-encrypted-compaction";

fn official_stream(request: &CapturedRequest) -> String {
    let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
    let compacting = body["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "compaction_trigger")
    });
    if !compacting {
        return mock_stream(ModelProtocol::Responses);
    }
    let item = json!({"type":"compaction","encrypted_content":OFFICIAL_COMPACTION});
    [
        json!({"type":"response.created","response":{"id":"resp_official_compact","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"resp_official_compact","status":"completed",
            "usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}}),
    ]
    .into_iter()
    .map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap()))
    .collect()
}

fn compact(server: &mut AppServer, thread_id: &str) -> Result<(), Value> {
    server.send(json!({"id":40,"method":"thread/compact/start","params":{"threadId":thread_id}}));
    let deadline = std::time::Instant::now() + RPC_TIMEOUT;
    loop {
        let message = server
            .messages
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| json!({"error":"compaction timeout"}))?
            .map_err(|_| json!({"error":"invalid app-server output"}))?;
        if let Some(error) = message.get("error") {
            return Err(error.clone());
        }
        if message["method"] == "turn/completed" && message["params"]["threadId"] == thread_id {
            return if message["params"]["turn"]["status"] == "completed" {
                Ok(())
            } else {
                Err(message["params"]["turn"]["error"].clone())
            };
        }
    }
}

fn last_body(endpoint: &MockResponses) -> Value {
    let captured: Vec<_> = endpoint.requests.try_iter().collect();
    let request = captured.last().expect("a captured request");
    serde_json::from_slice(&request.body).unwrap()
}

fn texts(body: &Value) -> String {
    body["input"].to_string()
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_compacts_on_relays_and_official_and_keeps_switching() {
    let _router = ROUTER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.auth_path, synthetic_auth_for_plan("free")).unwrap();
    fs::write(&paths.config_path, "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\n").unwrap();
    let proxy = RejectingProxy::start();
    // The official fixture rejects relay IDs exactly like the ChatGPT backend.
    let official = MockResponses::with_handler(ModelProtocol::Responses, official_stream, true);
    let relay = MockResponses::start();
    let mut store = Store {
        router: RouterSettings {
            enabled: true,
            bearer_token: LOCAL_TOKEN.into(),
            ..Default::default()
        },
        active_provider_id: Some("relay-a".into()),
        providers: vec![Provider {
            id: "relay-a".into(),
            name: "Relay A".into(),
            base_url: format!("http://127.0.0.1:{}", relay.port),
            api_key: "synthetic-relay-key".into(),
            model: "gpt-6.1-sol".into(),
            enabled: true,
            network_mode: "direct".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut entry =
        router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    entry["slug"] = json!("gpt-6-luna");
    entry["display_name"] = json!("Synthetic official Luna");
    fs::write(
        paths.codex_home.join("models_cache.json"),
        json!({"models":[entry]}).to_string(),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (shutdown, listener_task) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        store.router.port = listener.local_addr().unwrap().port();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let context = Arc::new(ArcContext {
            bound_port: store.router.port,
            store_path: paths.app_data_dir.join("newapi-providers.dat"),
            app_data_dir: paths.app_data_dir.clone(),
            codex_home: paths.codex_home.clone(),
            auth_path: paths.auth_path.clone(),
            outbound: Arc::new(RwLock::new(client.clone())),
            outbound_direct: client,
            official_base: Some(format!("http://127.0.0.1:{}/v1/", official.port)),
        });
        crate::core::secret_file::write(&context.store_path, &serde_json::to_vec(&store).unwrap())
            .unwrap();
        let (shutdown, receiver) = tokio::sync::watch::channel(false);
        (
            shutdown,
            tokio::spawn(accept_loop(listener, receiver, context)),
        )
    });
    router::apply_routing_config(&paths, &store).unwrap();
    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let started = server.request(
        2,
        "thread/start",
        json!({
            "cwd":cwd.to_string_lossy(), "model":"relay-a::gpt-6.1-sol",
            "approvalPolicy":"never", "sandbox":"read-only"
        }),
    );
    let id = started["thread"]["id"].as_str().unwrap().to_owned();
    let fail = |server: &AppServer, step: &str, error: Value| -> ! {
        panic!("{step}: {error}; stderr: {}", server.stderr.lock().unwrap())
    };

    // 1. A relay turn, then compaction on the relay: the router asks the relay
    //    for a readable summary instead of forwarding the V2 trigger.
    server
        .turn(
            &id,
            "relay-a::gpt-6.1-sol",
            "high",
            "FIRST_RELAY_MARKER",
            RPC_TIMEOUT,
        )
        .unwrap_or_else(|error| fail(&server, "relay turn", error));
    let _ = last_body(&relay);
    compact(&mut server, &id).unwrap_or_else(|error| fail(&server, "relay compaction", error));
    let summary_request = last_body(&relay);
    assert!(
        !texts(&summary_request).contains("compaction_trigger"),
        "{summary_request}"
    );
    assert!(
        texts(&summary_request).contains("CONTEXT CHECKPOINT COMPACTION"),
        "{summary_request}"
    );

    // 2. The official model reads that summary as plain text.
    server
        .turn(
            &id,
            "gpt-6-luna",
            "high",
            "SECOND_OFFICIAL_MARKER",
            RPC_TIMEOUT,
        )
        .unwrap_or_else(|error| fail(&server, "official turn after relay compaction", error));
    let official_body = last_body(&official);
    assert_eq!(official_body["model"], "gpt-6-luna");
    assert!(
        texts(&official_body).contains(STREAM_TEXT),
        "summary carried: {official_body}"
    );
    assert!(
        !texts(&official_body).contains("agentharbor.compaction"),
        "{official_body}"
    );

    // 3. Official compaction stays native on the account.
    compact(&mut server, &id).unwrap_or_else(|error| fail(&server, "official compaction", error));
    let trigger = last_body(&official);
    assert!(texts(&trigger).contains("compaction_trigger"), "{trigger}");

    // 4. Back on the relay: the encrypted official compaction is converted to
    //    readable text by the account, never sent to the relay as-is.
    server
        .turn(
            &id,
            "relay-a::gpt-6.1-sol",
            "high",
            "THIRD_RELAY_MARKER",
            RPC_TIMEOUT,
        )
        .unwrap_or_else(|error| fail(&server, "relay turn after official compaction", error));
    let conversion = last_body(&official);
    assert!(
        texts(&conversion).contains(OFFICIAL_COMPACTION),
        "{conversion}"
    );
    assert!(
        texts(&conversion).contains("CONTEXT CHECKPOINT COMPACTION"),
        "{conversion}"
    );
    let relay_body = last_body(&relay);
    assert!(
        !texts(&relay_body).contains(OFFICIAL_COMPACTION),
        "{relay_body}"
    );
    assert!(!relay_body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "reasoning"));
    assert!(
        texts(&relay_body).contains(STREAM_TEXT),
        "converted summary: {relay_body}"
    );
    assert!(texts(&relay_body).contains("THIRD_RELAY_MARKER"));

    drop(server);
    shutdown.send(true).unwrap();
    runtime.block_on(listener_task).unwrap();
}
