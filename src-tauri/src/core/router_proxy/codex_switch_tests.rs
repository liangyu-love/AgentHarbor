//! Real client, synthetic Free auth, production router, loopback upstreams.
use super::*;
use crate::core::router::{ModelProtocol, Provider, RouterSettings, Store};
use std::{fs, time::Duration};

#[path = "../../../tests/support/codex.rs"]
mod fixtures;
use fixtures::*;

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_accountless_catalog_hides_retained_account_models() {
    let binary = PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let auth = synthetic_auth_for_plan("free");
    fs::write(&paths.auth_path, &auth).unwrap();
    fs::write(&paths.config_path, "cli_auth_credentials_store = 'file'\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\n").unwrap();
    let proxy = RejectingProxy::start();
    // Reserve a private port; the model/list RPC should use the static catalog
    // without making an inference request or reaching another local service.
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let mut store = Store {
        router: RouterSettings {
            enabled: true,
            port: listener.local_addr().unwrap().port(),
            bearer_token: LOCAL_TOKEN.into(),
            ..Default::default()
        },
        active_provider_id: Some("relay".into()),
        providers: vec![Provider {
            id: "relay".into(),
            name: "Synthetic relay".into(),
            base_url: "https://relay.example.invalid".into(),
            api_key: "synthetic-relay-key".into(),
            model: "gpt-6.1-sol".into(),
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut official = router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    official["slug"] = json!("gpt-6-luna");
    official["display_name"] = json!("Synthetic official model");
    let catalog = json!({"models":[official]}).to_string();
    fs::write(paths.codex_home.join("models_cache.json"), &catalog).unwrap();
    let key = crate::core::auth::make_auth_snapshot(
        &crate::core::auth::load_auth_file(&paths.auth_path).unwrap(), &paths.auth_path,
    ).unwrap().account_key;
    crate::core::model_catalog::store_account_catalog(&paths.app_data_dir, &key, catalog.as_bytes()).unwrap();
    for accountless in [false, true, false] {
        store.router.accountless = accountless;
        router::apply_routing_config(&paths, &store).unwrap();
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let menu = server.request(2, "model/list", json!({}));
        let ids: Vec<_> = menu["data"].as_array().unwrap().iter().map(|model| model["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"relay::gpt-6.1-sol"), "{menu}");
        assert_eq!(ids.contains(&"gpt-6-luna"), !accountless, "{menu}");
        if accountless {
            assert_eq!(ids, ["relay::gpt-6.1-sol"], "no official fallback models in accountless mode");
        }
        assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    }
}

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
    // Routing only moves the built-in provider's endpoint; the thread keeps it.
    assert_eq!(resumed["modelProvider"], "openai");
    assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    shutdown.send(true).unwrap();
    runtime.block_on(listener_task).unwrap();
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_relay_thread_keeps_ultra_and_multi_agent_v2() {
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.auth_path, synthetic_auth_for_plan("free")).unwrap();
    fs::write(&paths.config_path, "cli_auth_credentials_store = 'file'
check_for_update_on_startup = false
[analytics]
enabled = false
[feedback]
enabled = false
[features]
apps = false
plugins = false
").unwrap();
    let proxy = RejectingProxy::start();
    let official = MockResponses::official_account();
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
            api_key: "synthetic-relay-0".into(),
            model: "gpt-6.1-sol".into(),
            enabled: true,
            network_mode: "direct".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    // The official entry of the same model declares Ultra and multi-agent V2.
    let mut entry =
        router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    entry["slug"] = json!("gpt-6.1-sol");
    entry["display_name"] = json!("Synthetic official Sol");
    entry["supported_reasoning_levels"] = json!(["low", "medium", "high", "xhigh", "max", "ultra"]
        .map(|effort| json!({"effort":effort,"description":effort})));
    entry["multi_agent_version"] = json!("v2");
    entry["multi_agent_reasoning_effort"] = json!("xhigh");
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
    {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let menu = server.request(2, "model/list", json!({}));
        let model = menu["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["id"] == "relay-a::gpt-6.1-sol")
            .unwrap_or_else(|| panic!("{menu}"));
        assert!(
            model["supportedReasoningEfforts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|level| level["reasoningEffort"] == "ultra"),
            "{model}"
        );
        // The thread starts on the relay, so its first turn pins the version.
        let started = server.request(
            3,
            "thread/start",
            json!({
                "cwd":cwd.to_string_lossy(), "model":"relay-a::gpt-6.1-sol",
                "approvalPolicy":"never", "sandbox":"read-only"
            }),
        );
        let id = started["thread"]["id"].as_str().unwrap().to_owned();
        server
            .turn(&id, "relay-a::gpt-6.1-sol", "ultra", "SYNTHETIC_ULTRA", RPC_TIMEOUT)
            .unwrap_or_else(|error| {
                panic!("{error}; stderr: {}", server.stderr.lock().unwrap())
            });
        let captured = relay.requests.recv_timeout(RPC_TIMEOUT).unwrap();
        let body: Value = serde_json::from_slice(&captured.body).unwrap();
        assert_eq!(body["model"], "gpt-6.1-sol");
        // Ultra never reaches the wire: Codex sends the declared multi-agent effort.
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        let tools = body["tools"].to_string();
        assert!(!tools.contains("multi_agent_v1"), "pinned to V1: {tools}");
        assert!(tools.contains("spawn_agent"), "{tools}");
    }
    shutdown.send(true).unwrap();
    runtime.block_on(listener_task).unwrap();
}

fn spawn_stream(namespace: Option<&str>) -> String {
    let mut item = json!({"id":"fc_child","type":"function_call","name":"spawn_agent",
        "call_id":"call_child","arguments":json!({"message":"SYNTHETIC_CHILD_ONLY","fork_context":false}).to_string()});
    if let Some(namespace) = namespace {
        item["namespace"] = json!(namespace);
    }
    [
        json!({"type":"response.created","response":{"id":"resp_spawn","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.done","output_index":0,"item":item.clone()}),
        json!({"type":"response.completed","response":{"id":"resp_spawn","status":"completed","output":[item],
            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
    ].into_iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect()
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_CODEX_BIN pointing to an installed Codex CLI"]
fn real_codex_spawns_saved_relay_child_without_changing_parent() {
    for accountless in [true, false] {
        assert_saved_child_route(accountless);
    }
}

fn assert_saved_child_route(accountless: bool) {
    use crate::core::router::subagents::{self, Settings};
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_CODEX_BIN").expect("installed binary"));
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let auth = (!accountless).then(|| synthetic_auth_for_plan("free"));
    if let Some(auth) = &auth {
        fs::write(&paths.auth_path, auth).unwrap();
    }
    fs::write(&paths.config_path, "check_for_update_on_startup = false\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[features]\napps = false\nplugins = false\nmulti_agent = true\nmulti_agent_v2 = false\n").unwrap();
    let proxy = RejectingProxy::start();
    let parent = MockResponses::scripted(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        if body["input"].as_array().is_some_and(|input| {
            input.iter().any(|item| {
                item["type"] == "function_call_output" && item["call_id"] == "call_child"
            })
        }) {
            return mock_stream(ModelProtocol::Responses);
        }
        let tools = body["tools"].as_array().expect("client tool list");
        let namespace = tools
            .iter()
            .find(|tool| {
                tool["type"] == "namespace"
                    && tool["tools"].as_array().is_some_and(|children| {
                        children.iter().any(|tool| tool["name"] == "spawn_agent")
                    })
            })
            .and_then(|tool| tool["name"].as_str());
        assert!(
            namespace.is_some() || tools.iter().any(|tool| tool["name"] == "spawn_agent"),
            "spawn tool absent"
        );
        spawn_stream(namespace)
    });
    let child = MockResponses::start();
    let mut store = Store {
        router: RouterSettings {
            enabled: true,
            accountless,
            bearer_token: LOCAL_TOKEN.into(),
            ..Default::default()
        },
        active_provider_id: Some("parent".into()),
        providers: vec![
            Provider {
                id: "parent".into(),
                name: "Parent".into(),
                base_url: format!("http://127.0.0.1:{}", parent.port),
                api_key: "synthetic-parent-key".into(),
                model: "gpt-6-sol".into(),
                enabled: true,
                network_mode: "direct".into(),
                ..Default::default()
            },
            Provider {
                id: "child".into(),
                name: "Child".into(),
                base_url: format!("http://127.0.0.1:{}", child.port),
                api_key: "synthetic-child-key".into(),
                model: "gpt-6-luna".into(),
                enabled: true,
                network_mode: "direct".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let parent_model = if accountless {
        "parent::gpt-6-sol"
    } else {
        "gpt-6-luna"
    };
    let mut official =
        router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    official["slug"] = json!("gpt-6-luna");
    official["display_name"] = json!("Synthetic Free Luna");
    fs::write(
        paths.codex_home.join("models_cache.json"),
        json!({"models":[official]}).to_string(),
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
        let context = Arc::new(ArcContext {
            bound_port: store.router.port,
            store_path: paths.app_data_dir.join("newapi-providers.dat"),
            app_data_dir: paths.app_data_dir.clone(),
            codex_home: paths.codex_home.clone(),
            auth_path: paths.auth_path.clone(),
            outbound: Arc::new(RwLock::new(client)),
            outbound_direct: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            official_base: Some(format!("http://127.0.0.1:{}/v1/", parent.port)),
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
    subagents::save(
        &paths,
        Some(Settings {
            model: Some("child::gpt-6-luna".into()),
            reasoning_effort: Some("xhigh".into()),
        }),
    )
    .unwrap();
    let before = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse::<toml::Value>()
        .unwrap();
    let child_thread = {
        let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
        let started = server.request(
            2,
            "thread/start",
            json!({"cwd":cwd.to_string_lossy(),
            "model":parent_model,"approvalPolicy":"never","sandbox":"read-only"}),
        );
        let thread_id = started["thread"]["id"].as_str().unwrap();
        server
            .turn(
                thread_id,
                parent_model,
                "high",
                "SYNTHETIC_PARENT_ONLY",
                RPC_TIMEOUT,
            )
            .unwrap_or_else(|error| panic!("{error}; stderr: {}", server.stderr.lock().unwrap()));
        let received = child
            .requests
            .recv_timeout(RPC_TIMEOUT)
            .expect("child request");
        let body: Value = serde_json::from_slice(&received.body).unwrap();
        assert_eq!(body["model"], "gpt-6-luna");
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        assert_eq!(
            received.header("authorization"),
            Some("Bearer synthetic-child-key")
        );
        assert!(received.header(router::LOCAL_AUTH_HEADER).is_none());
        assert_eq!(received.header("x-openai-subagent"), Some("collab_spawn"));
        assert!(body["input"].to_string().contains("SYNTHETIC_CHILD_ONLY"));
        assert!(!body["input"].to_string().contains("SYNTHETIC_PARENT_ONLY"));
        let parent_requests: Vec<_> = parent.requests.try_iter().collect();
        assert!(
            parent_requests.len() >= 2,
            "parent spawn and continuation observed"
        );
        for captured in parent_requests {
            let body: Value = serde_json::from_slice(&captured.body).unwrap();
            assert_eq!(
                body["model"],
                if accountless {
                    "gpt-6-sol"
                } else {
                    "gpt-6-luna"
                }
            );
            assert_eq!(body["reasoning"]["effort"], "high");
        }
        // Persisted child thread records the target independently of defaults.
        let threads = server.request(
            3,
            "thread/list",
            json!({"limit":20,"sourceKinds":["subAgent"]}),
        );
        threads["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|thread| thread["id"] != thread_id)
            .expect("child session persisted")["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    subagents::save(
        &paths,
        Some(Settings {
            model: Some("parent::gpt-6-sol".into()),
            reasoning_effort: Some("high".into()),
        }),
    )
    .unwrap();
    let after = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse::<toml::Value>()
        .unwrap();
    assert_eq!(after["model"], before["model"]);
    // Account mode keeps the built-in provider, so the key may be absent.
    assert_eq!(after.get("model_provider"), before.get("model_provider"));
    let mut server = AppServer::start(&binary, &paths, &cwd, &proxy);
    let resumed = server.request(4, "thread/resume", json!({"threadId":child_thread}));
    assert_eq!(resumed["model"], "child::gpt-6-luna", "{resumed}");
    assert_eq!(resumed["reasoningEffort"], "xhigh", "{resumed}");
    if let Some(auth) = auth {
        assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
    } else {
        assert!(!paths.auth_path.exists());
    }
    // Signed-in Codex may attempt account/service discovery. RejectingProxy
    // blocks those connections; every inference request above stays loopback.
    shutdown.send(true).unwrap();
    runtime.block_on(listener_task).unwrap();
}
