//! Real client, synthetic Free auth, production router, loopback upstreams.
use super::*;
use crate::core::router::{ModelProtocol, Provider, RouterSettings, Store};
use std::{fs, time::Duration};

#[path = "../../../tests/support/codex.rs"]
mod fixtures;
use fixtures::*;

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_same_free_thread_switches_official_and_relays_repeatedly() {
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let auth = synthetic_auth_for_plan("free");
    fs::write(&paths.auth_path, &auth).unwrap();
    fs::write(&paths.config_path, "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\n").unwrap();
    let proxy = RejectingProxy::start();
    let official = MockResponses::official_account();
    let a = MockResponses::start();
    let b = MockResponses::start();
    let mut store = Store {
        router: RouterSettings {
            enabled: true,
            bearer_token: LOCAL_TOKEN.into(),
            ..Default::default()
        },
        active_provider_id: Some("relay-a".into()),
        providers: [&a, &b]
            .iter()
            .enumerate()
            .map(|(index, endpoint)| Provider {
                id: format!("relay-{}", if index == 0 { "a" } else { "b" }),
                name: format!("Relay {index}"),
                base_url: format!("http://127.0.0.1:{}", endpoint.port),
                api_key: format!("synthetic-relay-{index}"),
                model: "gpt-6.1-sol".into(),
                enabled: true,
                network_mode: "direct".into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut entry =
        router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    entry["slug"] = json!("gpt-6-luna");
    entry["display_name"] = json!("Synthetic official Luna");
    entry["supported_reasoning_levels"] = json!(["low", "medium", "high", "xhigh", "max"]
        .map(|effort| json!({"effort":effort,"description":effort})));
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
        // The router's official fixture is also loopback. Reject non-local
        // traffic while allowing the production request path to reach it.
        let proxy_url = proxy.url.clone();
        let client = Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::custom(move |url| {
                (!matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
                    .then(|| proxy_url.clone())
            }))
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let direct = Client::builder()
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
            outbound: Arc::new(RwLock::new(client)),
            outbound_direct: direct,
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
    let thread_id = {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let account = server.request(2, "account/read", json!({}));
        assert_eq!(account["account"]["planType"], "free", "{account}");
        let menu = server.request(3, "model/list", json!({}));
        for alias in ["relay-a::gpt-6.1-sol", "relay-b::gpt-6.1-sol"] {
            let model = menu["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["id"] == alias)
                .unwrap();
            assert!(
                model["supportedReasoningEfforts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|level| level["reasoningEffort"] == "xhigh"),
                "{model}"
            );
        }
        let started = server.request(
            4,
            "thread/start",
            json!({
                "cwd":cwd.to_string_lossy(), "model":"gpt-6-luna",
                "approvalPolicy":"never", "sandbox":"read-only"
            }),
        );
        let id = started["thread"]["id"].as_str().unwrap().to_owned();
        for (model, effort, endpoint, upstream_model, wire_effort, relay_key) in [
            ("gpt-6-luna", "high", &official, "gpt-6-luna", "high", None),
            (
                "relay-a::gpt-6.1-sol",
                "xhigh",
                &a,
                "gpt-6.1-sol",
                "xhigh",
                Some("synthetic-relay-0"),
            ),
            (
                "relay-b::gpt-6.1-sol",
                "high",
                &b,
                "gpt-6.1-sol",
                "high",
                Some("synthetic-relay-1"),
            ),
            ("gpt-6-luna", "max", &official, "gpt-6-luna", "max", None),
            (
                "relay-a::gpt-6.1-sol",
                "max",
                &a,
                "gpt-6.1-sol",
                "max",
                Some("synthetic-relay-0"),
            ),
            (
                "relay-b::gpt-6.1-sol",
                "xhigh",
                &b,
                "gpt-6.1-sol",
                "xhigh",
                Some("synthetic-relay-1"),
            ),
            ("gpt-6-luna", "high", &official, "gpt-6-luna", "high", None),
        ] {
            server.turn(&id, model, effort, "SYNTHETIC_SWITCH_HISTORY", RPC_TIMEOUT)
                .unwrap_or_else(|error| panic!("{model}/{effort}: {error}; stderr: {}; rejected proxy requests: {:?}; received fixture requests: {:?}",
                    server.stderr.lock().unwrap(), proxy.requests.try_iter().collect::<Vec<_>>(),
                    endpoint.requests.try_iter().map(|request| request.request_line).collect::<Vec<_>>()));
            let captured = endpoint.requests.recv_timeout(RPC_TIMEOUT).unwrap();
            let body: Value = serde_json::from_slice(&captured.body).unwrap();
            assert_eq!(body["model"], upstream_model);
            assert_eq!(body["reasoning"]["effort"], wire_effort);
            assert!(body["input"]
                .to_string()
                .contains("SYNTHETIC_SWITCH_HISTORY"));
            assert!(body.get("previous_response_id").is_none());
            assert!(captured.header(router::LOCAL_AUTH_HEADER).is_none());
            if let Some(key) = relay_key {
                assert_eq!(
                    captured.header("authorization"),
                    Some(format!("Bearer {key}").as_str())
                );
                assert!(captured.header("chatgpt-account-id").is_none());
            } else {
                let auth: Value = serde_json::from_slice(&auth).unwrap();
                assert_eq!(
                    captured.header("authorization"),
                    Some(
                        format!(
                            "Bearer {}",
                            auth["tokens"]["access_token"].as_str().unwrap()
                        )
                        .as_str()
                    )
                );
            }
        }
        id
    };
    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let resumed = server.request(2, "thread/resume", json!({"threadId":thread_id}));
    assert_eq!(resumed["thread"]["id"], thread_id);
    assert_eq!(resumed["modelProvider"], router::CATALOG_PROVIDER_KEY);
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    shutdown.send(true).unwrap();
    runtime.block_on(listener_task).unwrap();
}
