//! Desktop uses real localhost HTTP and the shared response/accounting pipeline.
//! All state, credentials and upstreams are fixtures under temporary roots.
use super::*;
use crate::core::{
    claude_desktop::{
        gateway::RouterConfig,
        paths::DesktopPaths,
        transaction,
        types::{DesktopRouteChoice, OneMMode},
    },
    newapi::{Model, ModelOverrides, PriceOverrides},
    router::{ModelProtocol, Provider, RouterSettings, Store},
};

fn write_store(paths: &CodexPaths, store: &Store) {
    crate::core::secret_file::write(
        &paths.app_data_dir.join("newapi-providers.dat"),
        &serde_json::to_vec(store).unwrap(),
    )
    .unwrap();
}

fn choices() -> Vec<DesktopRouteChoice> {
    [
        ("sonnet", "claude-sonnet-4-6"),
        ("opus", "claude-opus-4-7"),
        ("haiku", "claude-haiku-4-5"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (role, alias))| DesktopRouteChoice {
        target_ref: None,
        role: role.into(),
        role_alias: alias.into(),
        provider_id: format!("desktop{i}"),
        model_id: "fixture-model".into(),
        label_override: None,
        one_m_mode: OneMMode::Auto,
    })
    .collect()
}

fn write_state(
    paths: &CodexPaths,
    port: u16,
    routes: Vec<DesktopRouteChoice>,
    revision: u64,
    token: &str,
) {
    let roots = DesktopPaths::from_roots(
        paths.app_data_dir.join("desktop-normal"),
        paths.app_data_dir.join("desktop-third-party"),
    )
    .unwrap();
    let mut state = transaction::next_state(
        &[],
        &[],
        &roots,
        revision,
        &format!("tx{revision}"),
        "",
        "",
        None,
        &[],
        transaction::AppliedMode::Router,
    )
    .unwrap();
    state.router = Some(RouterConfig {
        routes,
        route_views: vec![],
        file_projection: String::new(),
        route_revision: revision,
        gateway_token_version: revision,
        gateway_token: token.into(),
        bound_port: port,
    });
    crate::core::secret_file::write(
        &paths.app_data_dir.join("claude-desktop/state.dat"),
        &serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
}

struct Stop;
impl Drop for Stop {
    fn drop(&mut self) {
        set_desktop_pin(None).unwrap();
        stop_unchecked();
    }
}

#[test]
fn hosted_search_reaches_responses_through_both_messages_ingresses_and_replays_history() {
    let _lock = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        const REPLY: &str = r#"{"object":"response","id":"resp-search","status":"completed","model":"fixture-model","output":[{"type":"web_search_call","id":"ws1","status":"completed","action":{"type":"search","query":"Rust","sources":[{"type":"url","url":"https://rust-lang.org"}]}},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Rust answer","annotations":[{"type":"url_citation","url":"https://rust-lang.org","title":"Rust","start_index":0,"end_index":4}]}]}],"usage":{"input_tokens":3,"output_tokens":4}}"#;
        let stream = format!("data: {{\"type\":\"response.completed\",\"response\":{REPLY}}}\n\n");
        for streaming in [false, true] {
            let (url, calls, captured, server) = routing_safety_tests::mock(200, if streaming { stream.clone() } else { REPLY.to_owned() }, if streaming { "text/event-stream" } else { "application/json" }).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            let store = Store { providers: vec![Provider { id:"desktop0".into(), api_key:"fixture-upstream".into(), base_url:url,
                protocol:ModelProtocol::Responses, enabled:true, model:"fixture-model".into(), network_mode:"direct".into(), ..Default::default() }],
                router:RouterSettings { bearer_token:"router-fixture".into(), ..Default::default() }, ..Default::default() };
            write_store(&paths, &store);
            let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
            let _stop = Stop;
            let mut routes = choices(); routes.retain(|r| r.role == "haiku"); routes[0].provider_id = "desktop0".into();
            write_state(&paths, port, routes, 1, "desktop-fixture");
            let client = Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
            for desktop in [false, true] {
                let endpoint = format!("http://127.0.0.1:{port}/{}", if desktop { "claude-desktop/v1/messages" } else { "v1/messages" });
                let token = if desktop { "desktop-fixture" } else { "router-fixture" };
                let mut request = json!({"model":if desktop { "claude-haiku-4-5-20251001" } else { "desktop0::fixture-model" }, "max_tokens":128,"stream":streaming,
                    "tools":[{"type":"web_search_20250305","name":"web_search_next","max_uses":2}],
                    "tool_choice":{"type":"tool","name":"web_search_next"},
                    "messages":[{"role":"user","content":format!("Rust search {desktop}/{streaming}")}]});
                let reply = client.post(&endpoint).bearer_auth(token).json(&request).send().await.unwrap();
                assert_eq!(reply.status(), StatusCode::OK);
                let text = reply.text().await.unwrap();
                assert!(text.contains("web_search_next") && text.contains("https://rust-lang.org"));
                let content = if streaming { claude_continuation_tests::streamed_content(&text) } else { serde_json::from_str::<Value>(&text).unwrap()["content"].clone() };
                assert_eq!(content[0]["type"], "server_tool_use");
                request["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":content}), json!({"role":"user","content":"Details"})]);
                let reply = client.post(&endpoint).bearer_auth(token).json(&request).send().await.unwrap();
                assert_eq!(reply.status(), StatusCode::OK); reply.bytes().await.unwrap();
                let sent = captured.lock().unwrap().last().unwrap().clone();
                assert_eq!(sent["model"], "fixture-model");
                assert_eq!(sent["tools"][0]["type"], "web_search");
                assert_eq!(sent["tool_choice"], json!("required"));
                assert_eq!(sent["max_tool_calls"], 2);
                assert_eq!(sent["input"][1]["id"], "ws1");
                assert!(sent["input"][1]["action"]["sources"].is_array());
            }
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
            server.abort();
        }
    });
}

fn native_reply(body: &Value) -> (String, String) {
    let continued = body["messages"].to_string().contains("tool_result");
    let content = if continued {
        json!([{"type":"text", "text":"done"}])
    } else {
        json!([{"type":"tool_use", "id":"c1", "name":"lookup", "input":{}}])
    };
    let stop = if continued { "end_turn" } else { "tool_use" };
    if body["stream"] != true {
        return ("application/json".into(), json!({"type":"message", "id":"msg-fixture", "role":"assistant",
            "model":"fixture-model", "content":content, "stop_reason":stop,
            "usage":{"input_tokens":3, "output_tokens":4, "cache_read_input_tokens":2, "cache_creation_input_tokens":1}}).to_string());
    }
    let frames = [
        json!({"type":"message_start", "message":{"type":"message", "id":"msg-fixture", "role":"assistant",
            "model":"fixture-model", "content":[], "usage":{"input_tokens":3,"output_tokens":0,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}}),
        json!({"type":"content_block_start", "index":0, "content_block":content[0]}),
        json!({"type":"content_block_stop", "index":0}),
        json!({"type":"message_delta", "delta":{"stop_reason":stop}, "usage":{"output_tokens":4}}),
        json!({"type":"message_stop"}),
    ];
    (
        "text/event-stream".into(),
        frames
            .iter()
            .map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap()))
            .collect(),
    )
}

#[test]
fn desktop_namespace_routes_three_protocols_tools_and_actual_model_accounting() {
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_port = listener.local_addr().unwrap().port();
        let capture = captured.clone();
        let server = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let capture = capture.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let capture = capture.clone();
                        async move {
                            let path = request.uri().path().to_owned();
                            let native = path.starts_with("/v1/messages");
                            let count_tokens = path == "/v1/messages/count_tokens";
                            let mode = request.headers().get("x-fixture-mode").and_then(|v|v.to_str().ok()).unwrap_or("normal").to_owned();
                            let expected = if native { request.headers().get("x-api-key") } else { request.headers().get(AUTHORIZATION) };
                            assert_eq!(expected.and_then(|v|v.to_str().ok()), Some(if native { "upstream-fixture" }
                                else if mode == "rotated-key" { "Bearer rotated-upstream-fixture" } else { "Bearer upstream-fixture" }));
                            assert!(request.headers().get(router::LOCAL_AUTH_HEADER).is_none());
                            let beta = request.headers().get("anthropic-beta").and_then(|v|v.to_str().ok()).unwrap_or_default().to_owned();
                            let body: Value = serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
                            assert_eq!(body["model"], "fixture-model", "Desktop menu alias must be resolved before translation");
                            assert!(!body.to_string().contains("claude-opus-4-7"));
                            if count_tokens {
                                capture.lock().unwrap().push(json!({"path":path,"body":body,"beta":beta}));
                                return Ok::<_, std::convert::Infallible>(Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .header(CONTENT_TYPE, "application/json")
                                    .body(full_body(r#"{"error":{"type":"not_found_error","message":"count endpoint unavailable"}}"#.into()))
                                    .unwrap());
                            }
                            let (kind, reply) = if native { native_reply(&body) }
                                else { claude_continuation_tests::reply(&body, path == "/v1/chat/completions", &mode) };
                            capture.lock().unwrap().push(json!({"path":path,"body":body,"beta":beta}));
                            let payload = if mode == "cancel" {
                                let first = futures_util::stream::once(async {
                                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(Bytes::from_static(
                                        b"data: {\"model\":\"fixture-model\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4}}\n\n")))
                                });
                                BodyExt::boxed(StreamBody::new(first.chain(futures_util::stream::pending())))
                            } else { full_body(reply) };
                            Ok::<_, std::convert::Infallible>(Response::builder().header(CONTENT_TYPE,kind).body(payload).unwrap())
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(TokioIo::new(socket),service).await;
                });
            }
        });
        let protocols = [ModelProtocol::ChatCompletions, ModelProtocol::Responses, ModelProtocol::AnthropicMessages];
        let mut store = Store { providers: protocols.into_iter().enumerate().map(|(i,protocol)| Provider {
            id:format!("desktop{i}"), name:format!("fixture{i}"), base_url:format!("http://127.0.0.1:{upstream_port}"),
            api_key:"upstream-fixture".into(), model:"fixture-model".into(), protocol, enabled:true, network_mode:"direct".into(),
            models:vec![Model { id:"fixture-model".into(), protocol:Some(protocol), context_window:Some(1_000_000), metadata_source:"upstream".into(),
                overrides:ModelOverrides { price:if i == 0 {Some(PriceOverrides {input:Some(0.0),output:Some(0.0),..Default::default()})}
                    else if i == 2 {Some(PriceOverrides {input:Some(4.0),output:Some(20.0),cache_read:Some(0.4),cache_write:Some(5.0)})} else {None}, ..Default::default() }, ..Default::default() }],
            ..Default::default()
        }).collect(), router:RouterSettings {bearer_token:"router-fixture".into(), ..Default::default()}, ..Default::default() };
        write_store(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let _stop = Stop;
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(client.get(format!("{base}/claude-desktop/v1/models")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        assert!(!paths.app_data_dir.join("claude-desktop/state.dat").exists(), "HTTP auth must never create a gateway credential");
        let routes = choices();
        write_state(&paths,port,routes.clone(),1,"desktop-fixture");
        for endpoint in ["/claude-desktop/v1/models", "/claude-desktop/v1/messages"] {
            let response = if endpoint.ends_with("models") {client.get(format!("{base}{endpoint}"))} else {client.post(format!("{base}{endpoint}"))};
            assert_eq!(response.bearer_auth("router-fixture").send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        }
        for endpoint in ["/v1/models", "/p/desktop0/v1/models", "/codex/v1/models"] {
            assert_eq!(client.get(format!("{base}{endpoint}")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        }
        assert_eq!(client.get(format!("{base}/claude-desktop/v1/models")).header("x-api-key","desktop-fixture").send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        for endpoint in ["/claude-desktop//v1/models", "/claude-desktop/v1/responses"] {
            assert_eq!(client.get(format!("{base}{endpoint}")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::NOT_FOUND);
        }
        assert_eq!(client.get(format!("{base}/claude-desktop/v1/messages/count_tokens")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(client.get(format!("{base}/claude-desktop/v1/messages")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::METHOD_NOT_ALLOWED);
        let models: Value = client.get(format!("{base}/claude-desktop/v1/models")).bearer_auth("desktop-fixture").send().await.unwrap().json().await.unwrap();
        assert_eq!(models["data"].as_array().unwrap().len(),3);
        assert!(!models.to_string().contains("fixture-model"));
        assert!(models["data"].as_array().unwrap().iter().all(|model| model["supports1m"] == true));
        let latest_trace = route_trace::recent().first().map(|route| route.id.clone());
        let count_url = format!("{base}/claude-desktop/v1/messages/count_tokens");
        let chat_count = client.post(&count_url).bearer_auth("desktop-fixture").json(&json!({
            "model":"claude-sonnet-4-6",
            "system":"Count this system prompt",
            "messages":[{"role":"user","content":"Count this message"}],
            "tools":[{"name":"lookup","input_schema":{"type":"object","properties":{"query":{"type":"string"}}}}]
        })).send().await.unwrap();
        assert_eq!(chat_count.status(), StatusCode::OK);
        assert!(chat_count.json::<Value>().await.unwrap()["input_tokens"].as_u64().unwrap() > 0);
        assert!(captured.lock().unwrap().is_empty(), "Chat Completions cannot serve a Messages token-count request");

        // A native Messages provider gets an exact count attempt first; an
        // unsupported endpoint falls back locally instead of provoking the
        // client's billable max_tokens:1 completion fallback.
        let native_count = client.post(&count_url).bearer_auth("desktop-fixture").json(&json!({
            "model":"claude-haiku-4-5-20251001",
            "messages":[{"role":"user","content":"Count this native request"}]
        })).send().await.unwrap();
        assert_eq!(native_count.status(), StatusCode::OK);
        assert!(native_count.json::<Value>().await.unwrap()["input_tokens"].as_u64().unwrap() > 0);
        assert_eq!(captured.lock().unwrap()[0]["path"], "/v1/messages/count_tokens");
        assert_eq!(route_trace::recent().first().map(|route| route.id.clone()), latest_trace,
            "count-token calls should not create request-level inference diagnostics");
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 0);
        let url = format!("{base}/claude-desktop/v1/messages");
        for route in &routes { for streaming in [false,true] { for one_m in [false,true] {
            // Exercise both Desktop spellings without ever changing the real upstream ID.
            let alias = if one_m && !streaming {format!("{}[1M]",route.role_alias)} else {route.role_alias.clone()};
            let beta = if one_m && streaming {format!("existing-beta,{CLAUDE_ONE_M_BETA}")} else {"existing-beta".into()};
            let mut body = json!({"model":alias,"max_tokens":128,"stream":streaming,
                "tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":format!("case {}/{}/{}",route.role,streaming,one_m)}]});
            let first = client.post(&url).bearer_auth("desktop-fixture").header("anthropic-beta",&beta).json(&body).send().await.unwrap();
            assert_eq!(first.status(),StatusCode::OK);
            let text = first.text().await.unwrap();
            let content = if streaming {claude_continuation_tests::streamed_content(&text)} else {serde_json::from_str::<Value>(&text).unwrap()["content"].clone()};
            assert_eq!(content[0]["type"],"tool_use");
            body["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":content}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"result"}]})]);
            let second = client.post(&url).bearer_auth("desktop-fixture").header("anthropic-beta",&beta).json(&body).send().await.unwrap();
            assert_eq!(second.status(),StatusCode::OK);
            assert!(second.text().await.unwrap().contains("done"));
            let captured = captured.lock().unwrap();
            for attempt in captured.iter().rev().take(2) {
                assert_eq!(attempt["body"]["model"],"fixture-model");
                assert!(attempt["body"]["supports1m"].is_null());
                assert!(attempt["body"]["context_window"].is_null());
                assert!(attempt["body"]["1m"].is_null());
                let output_key = if attempt["path"] == "/v1/responses" {"max_output_tokens"} else {"max_tokens"};
                assert_eq!(attempt["body"][output_key],128,"translation must preserve the requested output budget");
                let forwarded_beta = attempt["beta"].as_str().unwrap();
                if route.role == "haiku" {
                    assert!(forwarded_beta.contains("existing-beta"));
                    assert_eq!(forwarded_beta.contains(CLAUDE_ONE_M_BETA),one_m);
                } else {
                    assert!(forwarded_beta.is_empty(),"Claude beta must not leak into Chat or Responses");
                }
            }
        }}}
        let usage = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.request_count,24);
        assert_eq!(usage.all_time.successful_count,24);
        for record in &usage.recent {
            assert_eq!(record.model,"fixture-model");
            assert_eq!(record.identity.client_source.as_deref(),Some("claudeDesktop"));
            assert_eq!(record.identity.resolved_model.as_deref(),Some("fixture-model"));
            assert_eq!(record.identity.route_revision,Some(1));
            if record.provider_id == "desktop0" { assert_eq!(record.cost_usd,Some(0.0)); }
            if record.provider_id == "desktop1" { assert_eq!(record.cost_usd,None); }
            if record.provider_id == "desktop2" {
                assert_eq!(record.tokens.cache_read_tokens,2);
                assert_eq!(record.tokens.cache_write_tokens,1);
                assert!((record.cost_usd.unwrap() - 0.0000978).abs() < 1e-12);
            }
        }
        assert!(route_trace::recent().iter().any(|r| r.client_source.as_deref()==Some("claudeDesktop") && r.requested_model=="claude-opus-4-7" && r.resolved_model.as_deref()==Some("fixture-model") && r.attempts[0].upstream_model=="fixture-model"));
        // Ordinary Claude Code ingress uses the same capacity admission as
        // Desktop. The marker and the stripped-marker beta spelling both work
        // for all egress protocols, without sending Claude headers to OpenAI.
        for (i, protocol) in protocols.into_iter().enumerate() { for beta_spelling in [false, true] {
            let model = format!("desktop{i}::fixture-model{}", if beta_spelling { "" } else { "[1M]" });
            let beta = if beta_spelling { format!("existing-beta,{CLAUDE_ONE_M_BETA}") } else { "existing-beta".into() };
            let reply = client.post(format!("{base}/v1/messages")).bearer_auth("router-fixture")
            .header("anthropic-beta", beta)
                .json(&json!({"model":model,"max_tokens":128,
                    "tools":[{"name":"lookup","input_schema":{"type":"object"}}],
                    "messages":[{"role":"user","content":format!("ordinary 1M {i}/{beta_spelling}")}]})).send().await.unwrap();
            assert_eq!(reply.status(),StatusCode::OK, "{protocol:?}, beta={beta_spelling}");
            reply.bytes().await.unwrap();
            let attempt = captured.lock().unwrap().last().unwrap().clone();
            assert_eq!(attempt["body"]["model"], "fixture-model");
            assert_eq!(attempt["path"], match protocol {
                ModelProtocol::Responses => "/v1/responses",
                ModelProtocol::ChatCompletions => "/v1/chat/completions",
                ModelProtocol::AnthropicMessages => "/v1/messages",
            });
            let forwarded_beta = attempt["beta"].as_str().unwrap();
            if protocol == ModelProtocol::AnthropicMessages {
                assert!(forwarded_beta.contains("existing-beta"));
                assert!(forwarded_beta.contains(CLAUDE_ONE_M_BETA));
            } else {
                assert!(forwarded_beta.is_empty());
            }
        }}
        for alias in ["claude-opus-5", "unknown"] {
            let response = client.post(&url).bearer_auth("desktop-fixture").json(&json!({"model":alias,"messages":[]})).send().await.unwrap();
            assert_eq!(response.status(),StatusCode::BAD_REQUEST, "{alias}");
        }
        let reply = client.post(&url).bearer_auth("desktop-fixture").json(&json!({"model":" claude-haiku-4-5[1M] ","messages":[],"max_tokens":128})).send().await.unwrap();
        assert_eq!(reply.status(),StatusCode::OK);
        reply.bytes().await.unwrap();
        assert!(captured.lock().unwrap().last().unwrap()["beta"].as_str().unwrap().contains(CLAUDE_ONE_M_BETA));
        store.providers[2].api_key = "".into();
        write_store(&paths,&store);
        assert_eq!(client.post(&url).bearer_auth("desktop-fixture").json(&json!({"model":"claude-haiku-4-5","messages":[]})).send().await.unwrap().status(),StatusCode::BAD_REQUEST);
        let mut config = routes.clone();
        for route in &mut config { route.one_m_mode = OneMMode::Off; }
        store.providers[2].api_key = "upstream-fixture".into(); write_store(&paths,&store);
        write_state(&paths,port,config,2,"rotated-fixture");
        assert_eq!(client.get(format!("{base}/claude-desktop/v1/models")).bearer_auth("desktop-fixture").send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        for route in &routes {
            assert_eq!(client.post(&url).bearer_auth("rotated-fixture").json(&json!({"model":format!("{}[1m]",route.role_alias),"messages":[]})).send().await.unwrap().status(),StatusCode::BAD_REQUEST);
            assert_eq!(client.post(&url).bearer_auth("rotated-fixture").header("anthropic-beta",CLAUDE_ONE_M_BETA)
                .json(&json!({"model":route.role_alias,"messages":[]})).send().await.unwrap().status(),StatusCode::BAD_REQUEST,
                "cached beta headers must not bypass the disabled 1M option");
        }
        write_state(&paths,port,routes.clone(),2,"rotated-fixture");
        for card in &mut store.providers { card.models[0].context_window = Some(200_000); }
        write_store(&paths,&store);
        let models:Value = client.get(format!("{base}/claude-desktop/v1/models")).bearer_auth("rotated-fixture").send().await.unwrap().json().await.unwrap();
        assert!(models["data"].as_array().unwrap().iter().all(|model| model["supports1m"].is_null()),"saved Auto intent must not claim stale 1M capacity");
        for route in &routes {
            assert_eq!(client.post(&url).bearer_auth("rotated-fixture").json(&json!({"model":format!("{}[1m]",route.role_alias),"messages":[]})).send().await.unwrap().status(),StatusCode::BAD_REQUEST);
            assert_eq!(client.post(&url).bearer_auth("rotated-fixture").header("anthropic-beta",CLAUDE_ONE_M_BETA)
                .json(&json!({"model":route.role_alias,"messages":[]})).send().await.unwrap().status(),StatusCode::BAD_REQUEST,
                "current capacity must be checked for cached beta headers as well as aliases");
        }
        for i in 0..protocols.len() { for beta_spelling in [false, true] {
            let model = format!("desktop{i}::fixture-model{}", if beta_spelling { "" } else { "[1m]" });
            let reply = client.post(format!("{base}/v1/messages")).bearer_auth("router-fixture")
                .header("anthropic-beta", if beta_spelling { CLAUDE_ONE_M_BETA } else { "existing-beta" })
                .json(&json!({"model":model,"max_tokens":128,"messages":[{"role":"user","content":"ordinary small model"}]}))
                .send().await.unwrap();
            assert_eq!(reply.status(), StatusCode::BAD_REQUEST, "ordinary 1M requires current model capacity");
            assert!(reply.text().await.unwrap().contains("selected model does not support 1M context"));
        }}
        for card in &mut store.providers { card.models[0].context_window = Some(1_000_000); }
        write_store(&paths,&store);
        let requests = captured.lock().unwrap();
        assert_eq!(requests.iter().filter(|request| request["path"] == "/v1/messages/count_tokens").count(), 1, "the native token-count request must reach the upstream exactly once");
        assert_eq!(requests.iter().filter(|request| request["path"] != "/v1/messages/count_tokens").count(), 31, "24 Desktop turns, 6 ordinary 1M calls and one native alias call; rejected requests must produce no model calls");
        drop(requests);

        // Identical histories and session IDs sharing one real target must not
        // cross role, committed revision, or ordinary router entry points.
        let mut shared = routes.clone();
        shared[1].provider_id = "desktop0".into();
        write_state(&paths,port,shared.clone(),3,"rotated-fixture");
        let mut body = json!({"model":"claude-sonnet-4-6","metadata":{"user_id":"same-session"},
            "max_tokens":128,"messages":[{"role":"user","content":"same visible history"}]});
        let first:Value = client.post(&url).bearer_auth("rotated-fixture").json(&body).send().await.unwrap().json().await.unwrap();
        body["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":first["content"]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"result"}]})]);
        client.post(&url).bearer_auth("rotated-fixture").json(&body).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(captured.lock().unwrap().last().unwrap()["body"]["messages"][1]["reasoning_content"],"required reasoning");
        body["model"] = json!("claude-opus-4-7");
        client.post(&url).bearer_auth("rotated-fixture").json(&body).send().await.unwrap().bytes().await.unwrap();
        assert!(captured.lock().unwrap().last().unwrap()["body"]["messages"][1]["reasoning_content"].is_null(), "different role must not replay private output");
        write_state(&paths,port,shared,4,"rotated-fixture");
        body["model"] = json!("claude-sonnet-4-6");
        client.post(&url).bearer_auth("rotated-fixture").json(&body).send().await.unwrap().bytes().await.unwrap();
        assert!(captured.lock().unwrap().last().unwrap()["body"]["messages"][1]["reasoning_content"].is_null(), "new route revision must not reuse old private output");
        body["model"] = json!("desktop0::fixture-model");
        client.post(format!("{base}/v1/messages")).bearer_auth("router-fixture").json(&body).send().await.unwrap().bytes().await.unwrap();
        assert!(captured.lock().unwrap().last().unwrap()["body"]["messages"][1]["reasoning_content"].is_null(), "CLI namespace must not reuse Desktop private output");

        write_state(&paths,port,routes,5,"rotated-fixture");
        let before_errors = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
        for mode in ["bad-tool","truncated","upstream-error","bad-json"] {
            let body = json!({"model":"claude-sonnet-4-6","stream":mode!="bad-json","max_tokens":128,
                "messages":[{"role":"user","content":mode}]});
            let reply = client.post(&url).bearer_auth("rotated-fixture").header("x-fixture-mode",mode).json(&body).send().await.unwrap();
            if mode=="bad-json" { assert_eq!(reply.status(),StatusCode::BAD_GATEWAY); }
            else { assert!(reply.text().await.unwrap().contains("event: error"),"{mode}"); }
        }
        let usage = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.request_count,before_errors.request_count+4,"errors must each be accounted exactly once");
        assert_eq!(usage.all_time.successful_count,before_errors.successful_count,"stream errors must not count as successful");
        assert!(usage.recent.iter().take(4).all(|r| r.identity.client_source.as_deref()==Some("claudeDesktop") && !r.success));
        let before_cancel = usage.all_time;
        let mut stream = client.post(&url).bearer_auth("rotated-fixture").header("x-fixture-mode","cancel")
            .json(&json!({"model":"claude-sonnet-4-6","stream":true,"max_tokens":128,"messages":[{"role":"user","content":"cancel"}]}))
            .send().await.unwrap().bytes_stream();
        assert!(stream.next().await.unwrap().is_ok());
        drop(stream);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let usage = relay_usage::load(&paths.app_data_dir).unwrap();
            if usage.all_time.request_count == before_cancel.request_count + 1 {
                assert_eq!(usage.all_time.successful_count,before_cancel.successful_count);
                assert_eq!(usage.all_time.total_tokens,before_cancel.total_tokens+7);
                assert_eq!(usage.recent[0].identity.client_source.as_deref(),Some("claudeDesktop"));
                assert!(!usage.recent[0].success);
                break;
            }
            assert!(tokio::time::Instant::now()<deadline,"cancellation must drop the response observer and persist once");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        store.providers[0].api_key = "rotated-upstream-fixture".into();
        write_store(&paths,&store);
        let response = client.post(&url).bearer_auth("rotated-fixture").header("x-fixture-mode","rotated-key")
            .json(&json!({"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"rotated key"}],"max_tokens":128}))
            .send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        response.bytes().await.unwrap();
        assert_eq!(super::super::claude_desktop::gateway::committed_snapshot(&paths.app_data_dir).unwrap().unwrap().token,"rotated-fixture",
            "upstream key rotation must not rewrite the local Desktop credential");
        server.abort();
    });
}

#[test]
fn desktop_listener_leases_pin_preview_and_managed_ports() {
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().join("codex"));
            let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default())
                .await
                .unwrap();
            let _stop = Stop;
            let lease = current_lease().unwrap();
            let reservation = reserve_listener(&lease).unwrap();
            struct Reservation(String);
            impl Drop for Reservation {
                fn drop(&mut self) {
                    release_listener_reservation(&self.0);
                }
            }
            let _reservation = Reservation(reservation.clone());
            assert_eq!(
                ensure_running(port, paths.clone(), ApiProxyConfigPayload::default())
                    .await
                    .unwrap(),
                port
            );
            assert_eq!(current_lease(), Some(lease.clone()));
            assert!(
                ensure_running(0, paths.clone(), ApiProxyConfigPayload::default())
                    .await
                    .is_err()
            );
            stop();
            assert!(validate_lease(&lease));
            set_desktop_pin(Some(port)).unwrap();
            release_listener_reservation(&reservation);
            stop();
            assert!(validate_lease(&lease));
            set_desktop_pin(None).unwrap();
            stop();
            assert!(!validate_lease(&lease));
            assert!(reserve_listener(&lease).is_err());
            ensure_running(port, paths, ApiProxyConfigPayload::default())
                .await
                .unwrap();
            let next = current_lease().unwrap();
            assert_eq!(next.bound_port, lease.bound_port);
            assert_eq!(next.process_epoch, lease.process_epoch);
            assert!(next.generation > lease.generation);
            assert!(!validate_lease(&lease));
        });
}
