use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) async fn mock(
    status: u16,
    body: impl AsRef<[u8]>,
    content_type: &'static str,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let calls = count.clone();
    let body = Bytes::copy_from_slice(body.as_ref());
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let captured = captured.clone();
            let calls = calls.clone();
            let body = body.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(move |request: Request<Incoming>| {
                            let captured = captured.clone();
                            let calls = calls.clone();
                            let body = body.clone();
                            async move {
                                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                                captured.lock().unwrap().push(if bytes.is_empty() {
                                    Value::Null
                                } else {
                                    serde_json::from_slice(&bytes).unwrap()
                                });
                                calls.fetch_add(1, Ordering::SeqCst);
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .header(CONTENT_TYPE, content_type)
                                        .body(Full::new(body))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await;
            });
        }
    });
    (url, count, requests, task)
}

pub(super) const RESPONSE: &str = r#"{"id":"response-test","object":"response","status":"completed","model":"gpt-6-astra","output":[],"usage":{"input_tokens":1,"output_tokens":1}}"#;
pub(super) fn persist(paths: &CodexPaths, store: &router::Store) {
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    crate::core::secret_file::write(
        &paths.app_data_dir.join("newapi-providers.dat"),
        &serde_json::to_vec(store).unwrap(),
    )
    .unwrap();
}

#[test]
fn ordinary_auxiliary_endpoints_keep_diagnostics_without_duplicate_usage() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (url, calls, _, server) = mock(200, RESPONSE, "application/json").await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let provider_id = format!("auxiliary-{}", uuid::Uuid::new_v4().simple());
        let mut store = router::Store {
            active_provider_id: Some(provider_id.clone()),
            providers: vec![router::Provider {
                id: provider_id.clone(), name: "Auxiliary fixture".into(), base_url: url,
                model: "gpt-6-astra".into(), api_key: "upstream-fixture".into(),
                enabled: true, ..Default::default()
            }],
            router: router::RouterSettings { bearer_token: "local-fixture".into(), ..Default::default() },
            ..Default::default()
        };
        persist(&paths, &store);
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        for prefix in [format!("p/{provider_id}/v1"), "v1".into()] {
            for suffix in ["responses", "responses/compact"] {
                let response = client.post(format!("{base}/{prefix}/{suffix}"))
                    .bearer_auth("local-fixture")
                    .json(&json!({"model":"gpt-6-astra","input":"fixture"}))
                    .send().await.unwrap();
                assert_eq!(response.status(), 200);
                response.bytes().await.unwrap();
            }
            let inferred = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
            for (method, suffix) in [
                (hyper::Method::GET, "responses/response-test"),
                (hyper::Method::HEAD, "responses/response-test"),
                (hyper::Method::POST, "responses/response-test/cancel"),
                (hyper::Method::GET, "responses/response-test/input_items"),
                (hyper::Method::DELETE, "responses/response-test"),
            ] {
                let response = client.request(method, format!("{base}/{prefix}/{suffix}"))
                    .bearer_auth("local-fixture").send().await.unwrap();
                assert_eq!(response.status(), 200);
                response.bytes().await.unwrap();
                let usage = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
                assert_eq!(usage.request_count, inferred.request_count, "{prefix}/{suffix}");
                assert_eq!(usage.total_tokens, inferred.total_tokens, "{prefix}/{suffix}");
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 14);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 4);
        server.abort();

        let (url, calls, _, server) = mock(200, r#"{"input_tokens":23}"#, "application/json").await;
        store.providers[0].base_url = url;
        store.providers[0].protocol = router::ModelProtocol::AnthropicMessages;
        persist(&paths, &store);
        for prefix in [format!("p/{provider_id}/v1"), "v1".into()] {
            let response = client.post(format!("{base}/{prefix}/messages/count_tokens"))
                .bearer_auth("local-fixture")
                .json(&json!({"model":"gpt-6-astra","messages":[{"role":"user","content":"Count this"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(response.json::<Value>().await.unwrap(), json!({"input_tokens":23}));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 4);
        let traces: Vec<_> = route_trace::recent().into_iter()
            .filter(|trace| trace.attempts.iter().any(|attempt| attempt.provider_id == provider_id)).collect();
        assert_eq!(traces.len(), 16, "auxiliary operations must still be observed");
        assert!(traces.iter().all(|trace| trace.done && trace.status == Some(200)));
        stop();
        server.abort();
    });
}

#[test]
fn raw_upstream_model_declaration_is_shared_without_changing_response() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let body = r#"{"model":"declared-other","usage":{"input_tokens":3,"output_tokens":2}}"#;
        let (upstream, _, captured, server) = mock(200, body, "application/json").await;
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().into());
        let store = router::Store {
            providers: vec![router::Provider {
                id: "source".into(),
                name: "Source".into(),
                base_url: upstream,
                api_key: "secret-version".into(),
                model: "wire-model".into(),
                enabled: true,
                ..Default::default()
            }],
            router: router::RouterSettings {
                bearer_token: "local-only".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        persist(&paths, &store);
        let port = ensure_running(
            0,
            paths.clone(),
            ApiProxyConfigPayload {
                mode: super::super::models::ApiProxyMode::Direct,
                url: None,
            },
        )
        .await
        .unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("http://127.0.0.1:{port}/p/source/v1/responses"))
            .bearer_auth("local-only")
            .json(&json!({"model":"wire-model","input":"fixture"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), body);
        assert_eq!(
            captured.lock().unwrap()[0],
            json!({"model":"wire-model","input":"fixture"})
        );
        let record = relay_usage::load(&paths.app_data_dir)
            .unwrap()
            .recent
            .pop()
            .unwrap();
        let trace = route_trace::recent()
            .into_iter()
            .find(|route| route.id == record.request_id)
            .unwrap();
        let attempt = &trace.attempts[0];
        assert_eq!(attempt.attempt_id, record.attempt_id);
        assert_eq!(
            attempt.model_observation.served_model.as_deref(),
            Some("declared-other")
        );
        assert_eq!(
            attempt.model_observation.model_match_status,
            "declared_different"
        );
        assert_eq!(
            record.model_observation.served_model,
            attempt.model_observation.served_model
        );
        assert_eq!(record.credential.status, "recorded");
        assert!(!serde_json::to_string(&record)
            .unwrap()
            .contains("secret-version"));
        stop();
        server.abort();
    });
}

#[test]
fn responses_namespace_tools_keep_identity_through_http_and_cached_followups() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
            for streaming in [false, true] {
                let translation = protocol::Translation { from:router::ModelProtocol::Responses, to:protocol };
                let request = json!({"model":"wire-model","input":"fixture","stream":streaming,"tools":[
                    {"type":"namespace","name":"mcp_fixture","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}
                ]});
                let prepared = protocol::request_body(translation, &request).unwrap();
                let encoded = if protocol == router::ModelProtocol::ChatCompletions {
                    prepared.body["tools"][0]["function"]["name"].as_str().unwrap()
                } else { prepared.body["tools"][0]["name"].as_str().unwrap() };
                let json_response = if protocol == router::ModelProtocol::ChatCompletions {
                    json!({"id":"fixture","model":"wire-model","choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_fixture","type":"function","function":{"name":encoded,"arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
                } else {
                    json!({"id":"fixture","model":"wire-model","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call_fixture","name":encoded,"input":{}}],"stop_reason":"tool_use"})
                };
                let raw = if !streaming { json_response.to_string() } else if protocol == router::ModelProtocol::ChatCompletions {
                    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"model":"wire-model","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_fixture","type":"function","function":{"name":encoded,"arguments":"{}"}}]},"finish_reason":null}]}),
                        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}))
                } else {
                    format!("event: message_start\ndata: {}\n\nevent: content_block_start\ndata: {}\n\nevent: content_block_stop\ndata: {}\n\nevent: message_delta\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
                        json!({"type":"message_start","message":{"id":"fixture","model":"wire-model","usage":{"input_tokens":1}}}),
                        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_fixture","name":encoded,"input":{}}}),
                        json!({"type":"content_block_stop","index":0}),
                        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}),
                        json!({"type":"message_stop"}))
                };
            let (upstream, calls, captured, server) = mock(200, raw, if streaming { "text/event-stream" } else { "application/json" }).await;
                let dir = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(dir.path().into());
                let store = router::Store { providers:vec![router::Provider { id:"namespace-fixture".into(), base_url:upstream, model:"wire-model".into(), protocol, api_key:"synthetic".into(), enabled:true, ..Default::default() }],
                    router:router::RouterSettings { bearer_token:"local".into(), ..Default::default() }, ..Default::default() };
                persist(&paths, &store);
                let port = ensure_running(0, paths, ApiProxyConfigPayload { mode:super::super::models::ApiProxyMode::Direct, url:None }).await.unwrap();
                let client = Client::builder().no_proxy().build().unwrap();
                let url = format!("http://127.0.0.1:{port}/p/namespace-fixture/v1/responses");
                let response = client.post(&url).bearer_auth("local").json(&request).send().await.unwrap();
                assert_eq!(response.status(), 200);
                let text = response.text().await.unwrap();
                let response: Value = if streaming {
                    text.lines().filter_map(|line| line.strip_prefix("data: ")).filter_map(|data| serde_json::from_str::<Value>(data).ok())
                        .find(|event| event["type"] == "response.completed").unwrap()["response"].clone()
                } else { serde_json::from_str(&text).unwrap() };
                assert_eq!(response["output"][0]["name"], "lookup");
                assert_eq!(response["output"][0]["namespace"], "mcp_fixture");
                assert_eq!(response["output"][0]["call_id"], "call_fixture");
                let followup = json!({"model":"wire-model","previous_response_id":response["id"],"input":[{"type":"function_call_output","call_id":"call_fixture","output":"done"}]});
                let next = client.post(&url).bearer_auth("local").json(&followup).send().await.unwrap();
                assert_eq!(next.status(), 200);
                next.bytes().await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 2);
                let captured = captured.lock().unwrap();
                let call = if protocol == router::ModelProtocol::ChatCompletions {
                    &captured[1]["messages"][1]["tool_calls"][0]["function"]["name"]
                } else { &captured[1]["messages"][1]["content"][0]["name"] };
                assert_eq!(call.as_str(), Some(encoded));
                stop(); server.abort();
            }
        }
    });
}

#[test]
fn no_retry_for_5xx_or_a_truncated_stream_after_output() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for (status, body, kind) in [
            (
                503,
                r#"{"error":{"message":"ambiguous server error"}}"#,
                "application/json",
            ),
            (
                200,
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"started\"}\n\n",
                "text/event-stream",
            ),
        ] {
            let (url_a, a, _, server_a) = mock(status, body, kind).await;
            let (url_b, b, _, server_b) = mock(200, RESPONSE, "application/json").await;
            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().into());
            let provider = |id: &str, base_url| router::Provider {
                id: id.into(),
                name: id.into(),
                base_url,
                model: "gpt-6-astra".into(),
                api_key: "secret".into(),
                enabled: true,
                ..Default::default()
            };
            let store = router::Store {
                providers: vec![provider("a", url_a), provider("b", url_b)],
                router: router::RouterSettings {
                    bearer_token: "local".into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            persist(&paths, &store);
            let port = ensure_running(
                0,
                paths,
                ApiProxyConfigPayload {
                    mode: super::super::models::ApiProxyMode::Direct,
                    url: None,
                },
            )
            .await
            .unwrap();
            let response = Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://127.0.0.1:{port}/codex/v1/responses"))
                .bearer_auth("local")
                .header("session_id", "no-replay")
                .json(&json!({"model":"a::gpt-6-astra","input":"hello","stream":true}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            response.bytes().await.unwrap();
            assert_eq!(a.load(Ordering::SeqCst), 1);
            assert_eq!(b.load(Ordering::SeqCst), 0);
            stop();
            server_a.abort();
            server_b.abort();
        }
    });
}
