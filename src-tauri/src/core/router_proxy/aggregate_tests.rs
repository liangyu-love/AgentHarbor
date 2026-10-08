use super::super::aggregation;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn mock(protocol: router::ModelProtocol, status: u16, streaming: bool) -> Mock {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(vec![]));
    let captured = requests.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let captured = captured.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new().serve_connection(TokioIo::new(socket), service_fn(move |request: Request<Incoming>| {
                    let captured = captured.clone(); let calls = calls.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        let auth = request.headers().get(AUTHORIZATION).or_else(|| request.headers().get("x-api-key"))
                            .and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                        let local = request.headers().contains_key(router::LOCAL_AUTH_HEADER);
                        let beta = request.headers().get("anthropic-beta").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                        let version = request.headers().get("anthropic-version").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let body: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
                        captured.lock().unwrap().push(json!({"path":path,"auth":auth,"local":local,"beta":beta,"version":version,"body":body}));
                        calls.fetch_add(1, Ordering::SeqCst);
                        let model = body["model"].as_str().unwrap_or("wire-native");
                        let id = if path.starts_with("/v1/responses/") { path.trim_start_matches("/v1/responses/").split('/').next().unwrap().to_owned() }
                            else { format!("resp_{}", uuid::Uuid::new_v4().simple()) };
                        let result = if status != 200 { json!({"error":{"message":"synthetic upstream rejection"}}) }
                            else { match protocol {
                                router::ModelProtocol::Responses => json!({"id":id,"object":"response","status":"completed","model":model,
                                    "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],
                                    "usage":{"input_tokens":4,"output_tokens":2,"input_tokens_details":{"cached_tokens":0}},"vendor":{"model":"must-stay"}}),
                                router::ModelProtocol::ChatCompletions => json!({"id":id,"object":"chat.completion","model":model,"choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":0}}}),
                                router::ModelProtocol::AnthropicMessages => json!({"id":id,"type":"message","model":model,"role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":4,"output_tokens":2,"cache_read_input_tokens":0}}),
                            } };
                        let output = if streaming && status == 200 {
                            match protocol {
                                router::ModelProtocol::Responses => {
                                    let mut created = result.clone(); created["status"] = json!("in_progress");
                                    format!("event: response.created\r\ndata: {}\r\n\r\nevent: response.future\ndata: {{\"type\":\"response.future\",\"model\":\"keep-unknown\"}}\n\nevent: response.completed\ndata: {}\n\n",
                                        json!({"type":"response.created","response":created}), json!({"type":"response.completed","response":result}))
                                }
                                router::ModelProtocol::ChatCompletions => format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                                    json!({"id":id,"object":"chat.completion.chunk","model":model,"choices":[{"index":0,"delta":{"role":"assistant","content":"hello"},"finish_reason":null}]}),
                                    json!({"id":id,"object":"chat.completion.chunk","model":model,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":0}}})),
                                router::ModelProtocol::AnthropicMessages => {
                                    let events = [
                                        json!({"type":"message_start","message":{"id":id,"type":"message","role":"assistant","model":model,"content":[],"usage":{"input_tokens":4,"output_tokens":0,"cache_read_input_tokens":0}}}),
                                        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                                        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}),
                                        json!({"type":"content_block_stop","index":0}),
                                        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
                                        json!({"type":"message_stop"}),
                                    ];
                                    events.iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect()
                                }
                            }
                        } else { result.to_string() };
                        Ok::<_, std::convert::Infallible>(Response::builder().status(status)
                            .header(CONTENT_TYPE, if streaming {"text/event-stream"} else {"application/json"})
                            .header("retry-after","30").body(Full::new(Bytes::from(output))).unwrap())
                    }
                })).await;
            });
        }
    });
    Mock {
        url,
        requests,
        count,
        task,
    }
}
fn fixture(upstreams: &[(&Mock, &str, router::ModelProtocol)]) -> router::Store {
    let providers: Vec<_> = upstreams
        .iter()
        .enumerate()
        .map(|(index, (upstream, model, protocol))| router::Provider {
            id: format!("p{index}"),
            name: format!("Provider {index}"),
            base_url: upstream.url.clone(),
            api_key: format!("upstream-{index}"),
            model: (*model).into(),
            enabled: true,
            protocol: *protocol,
            models: vec![super::super::newapi::Model {
                id: (*model).into(),
                protocol: Some(*protocol),
                overrides: super::super::newapi::ModelOverrides {
                    context_window: Some(1_000_000),
                    max_output_tokens: Some(8192),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        })
        .collect();
    let aggregate = aggregation::AggregateModel {
        id: uuid::Uuid::new_v4().to_string(),
        public_model_id: "official-model".into(),
        display_name: "Official".into(),
        members: providers
            .iter()
            .enumerate()
            .map(|(index, provider)| aggregation::AggregateMember {
                id: format!("member{index}"),
                provider_id: provider.id.clone(),
                actual_model_id: provider.model.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    router::Store {
        providers,
        aggregates: vec![aggregate],
        router: router::RouterSettings {
            bearer_token: "local-fixture".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn persist(paths: &CodexPaths, store: &router::Store) {
    super::routing_safety_tests::persist(paths, store);
}
// Protocol differential cases explicitly choose their egress; coverage must
// not depend on rotating a healthy channel on every inference request.
fn select_test_member(paths: &CodexPaths, store: &router::Store, member: usize) {
    let mut selected = store.clone();
    for (index, candidate) in selected.aggregates[0].members.iter_mut().enumerate() {
        candidate.enabled = index == member;
    }
    persist(paths, &selected);
}
struct Stop;
impl Drop for Stop {
    fn drop(&mut self) {
        stop_unchecked();
    }
}
async fn start(paths: &CodexPaths, store: &router::Store) -> (Client, String) {
    persist(paths, store);
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
    (
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        format!("http://127.0.0.1:{port}"),
    )
}

#[test]
fn managed_client_paths_record_source_without_changing_route_or_legacy_urls() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _stop = Stop;
        let upstream = mock(router::ModelProtocol::AnthropicMessages, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&upstream, "wire-model", router::ModelProtocol::AnthropicMessages)]);
        let (client, base) = start(&paths, &store).await;
        let cases = [
            ("/clients/claude-code/p/p0/v1/messages", "wire-model", Some("claudeCode")),
            ("/clients/opencode/p/p0/v1/messages", "wire-model", Some("opencode")),
            ("/p/p0/v1/messages", "wire-model", None),
            ("/clients/opencode/aggregate/v1/messages", "official-model", Some("opencode")),
        ];
        for (route, model, _) in cases {
            let response = client.post(format!("{base}{route}"))
                .bearer_auth("local-fixture")
                .json(&json!({"model":model,"max_tokens":64,"messages":[{"role":"user","content":"hello"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{route}");
            let answer: Value = response.json().await.unwrap();
            assert_eq!(answer["model"], if route.contains("/aggregate/") { "official-model" } else { "wire-model" });
        }
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.recent.len(), cases.len());
        let traces = route_trace::recent();
        for ((_, _, source), record) in cases.into_iter().zip(ledger.recent.iter().rev()) {
            assert_eq!(record.identity.client_source.as_deref(), source);
            let trace = traces.iter().find(|trace| trace.id == record.request_id).unwrap();
            assert_eq!(trace.client_source.as_deref(), source);
            assert_eq!(trace.attempts.len(), 1);
            assert_eq!(trace.attempts[0].upstream_model, "wire-model");
        }
        assert_eq!(upstream.count.load(Ordering::SeqCst), cases.len());
    });
}

async fn scripted_mock(
    respond: impl Fn(&Value) -> Response<ResponseBody> + Send + Sync + 'static,
) -> Mock {
    scripted_request_mock(move |request| respond(&request["body"])).await
}

async fn scripted_request_mock(
    respond: impl Fn(&Value) -> Response<ResponseBody> + Send + Sync + 'static,
) -> Mock {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(AtomicUsize::new(0));
    let captured = requests.clone();
    let calls = count.clone();
    let respond = Arc::new(respond);
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let captured = captured.clone();
            let calls = calls.clone();
            let respond = respond.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(move |request: Request<Incoming>| {
                            let captured = captured.clone();
                            let calls = calls.clone();
                            let respond = respond.clone();
                            async move {
                                let path = request.uri().path().to_owned();
                                let method = request.method().as_str().to_owned();
                                let auth = request
                                    .headers()
                                    .get(AUTHORIZATION)
                                    .or_else(|| request.headers().get("x-api-key"))
                                    .and_then(|value| value.to_str().ok())
                                    .unwrap_or_default()
                                    .to_owned();
                                let beta = request.headers().get("anthropic-beta").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                                let version = request.headers().get("anthropic-version").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                                let body: Value =
                                    serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                                let observation = json!({"method":method,"path":path,"auth":auth,"beta":beta,"version":version,"body":body,"bodyBytes":bytes.len()});
                                captured
                                    .lock()
                                    .unwrap()
                                    .push(observation.clone());
                                calls.fetch_add(1, Ordering::SeqCst);
                                Ok::<_, std::convert::Infallible>(respond(&observation))
                            }
                        }),
                    )
                    .await;
            });
        }
    });
    Mock {
        url,
        requests,
        count,
        task,
    }
}

fn scripted_response(
    status: u16,
    body: impl Into<Bytes>,
    content_type: &'static str,
) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header("retry-after", "30")
        .body(
            Full::new(body.into())
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap()
}

fn without_retry_after(mut response: Response<ResponseBody>) -> Response<ResponseBody> {
    response.headers_mut().remove("retry-after");
    response
}

#[test]
fn same_priority_new_requests_and_new_sessions_keep_the_same_channel() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for affinity in [aggregation::Affinity::Session, aggregation::Affinity::StatelessRotation] {
            for streaming in [false, true] {
                let _stop = Stop;
                let a = mock(router::ModelProtocol::ChatCompletions, 200, streaming).await;
                let b = mock(router::ModelProtocol::AnthropicMessages, 200, streaming).await;
                let temp = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(temp.path().into());
                let mut store = fixture(&[(&a, "wire-a", router::ModelProtocol::ChatCompletions),
                    (&b, "wire-b", router::ModelProtocol::AnthropicMessages)]);
                store.aggregates[0].affinity = affinity;
                let (client, base) = start(&paths, &store).await;
                for index in 0..12 {
                    let mut request = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture");
                    if index % 2 == 0 { request = request.header("session-id", format!("fresh-{index}")); }
                    let response = request.json(&json!({"model":"official-model", "stream":streaming,
                        "max_tokens":64,"messages":[{"role":"user","content":format!("request {index}")}]}))
                        .send().await.unwrap();
                    assert_eq!(response.status(), 200);
                    response.bytes().await.unwrap();
                    wait_for_released_lease().await;
                }
                assert_eq!(a.count.load(Ordering::SeqCst), 12);
                assert_eq!(b.count.load(Ordering::SeqCst), 0);
                assert_aggregate_accounting_counts(&paths, &store, &[12, 0]);
                let traces: Vec<_> = route_trace::recent().into_iter().filter(|route|
                    route.attempts.first().is_some_and(|attempt| attempt.aggregate_id.as_deref() == Some(&store.aggregates[0].id))).collect();
                assert_eq!(traces.len(), 12);
                assert!(traces.iter().all(|trace| trace.attempts.len() == 1 && trace.attempts[0].provider_id == "p0"));
            }
        }
    });
}

#[test]
fn actual_5xx_streak_resets_on_success_ignores_400_and_replaces_a_portable_session_by_priority() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let sequence = Arc::new(AtomicUsize::new(0));
        let a = scripted_mock(move |body| {
            let status = [503, 503, 400, 200, 503, 503, 400, 503][sequence.fetch_add(1, Ordering::SeqCst)];
            without_retry_after(scripted_response(status, if status == 200 {
                json!({"id":"resp_reset","object":"response","status":"completed","model":body["model"],"output":[],
                    "usage":{"input_tokens":4,"output_tokens":2}})
            } else {
                json!({"error":{"type":if status == 400 {"invalid_request_error"} else {"server_error"},
                    "message":"synthetic model result"}})
            }.to_string(), "application/json"))
        }).await;
        let b = mock(router::ModelProtocol::Responses, 200, false).await;
        let c = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let mut store = fixture(&[(&a, "wire-a", router::ModelProtocol::Responses),
            (&b, "wire-b", router::ModelProtocol::Responses), (&c, "wire-c", router::ModelProtocol::ChatCompletions)]);
        store.aggregates[0].members[1].priority = 2;
        store.aggregates[0].members[2].priority = 1;
        let (client, base) = start(&paths, &store).await;
        let send = || client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .header("session-id", "same-portable-session").json(&json!({"model":"official-model","input":"hello"})).send();
        for status in [503, 503, 400, 200, 503, 503, 400, 503] {
            let response = send().await.unwrap();
            assert_eq!(response.status(), status);
            response.bytes().await.unwrap();
            wait_for_released_lease().await;
            assert_eq!(b.count.load(Ordering::SeqCst) + c.count.load(Ordering::SeqCst), 0,
                "5xx/parameter errors must not replay an already-sent request");
        }
        for _ in 0..3 {
            let response = send().await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(response.json::<Value>().await.unwrap()["model"], "official-model");
            wait_for_released_lease().await;
        }
        assert_eq!(a.count.load(Ordering::SeqCst), 8);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        assert_eq!(c.count.load(Ordering::SeqCst), 3, "replacement chooses the smallest available priority");
        let traces: Vec<_> = route_trace::recent().into_iter().filter(|trace|
            trace.attempts.first().is_some_and(|a| a.aggregate_id.as_deref() == Some(&store.aggregates[0].id))).collect();
        assert_eq!(traces.len(), 11);
        assert!(traces.iter().all(|trace| trace.attempts.len() == 1));
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.request_count, 11);
        assert_eq!(ledger.all_time.successful_count, 4);
        assert!(ledger.recent.iter().all(|record|
            traces.iter().any(|trace| trace.id == record.request_id && trace.attempts[0].attempt_id == record.attempt_id
                && trace.attempts[0].upstream_model == record.model)));
    });
}

#[test]
fn failing_token_counts_do_not_reset_or_increment_inference_failures() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = scripted_request_mock(|request| without_retry_after(scripted_response(
            if request["path"] == "/v1/messages/count_tokens" {500} else {503},
            r#"{"type":"error","error":{"type":"api_error","message":"synthetic channel failure"}}"#, "application/json"))).await;
        let b = mock(router::ModelProtocol::AnthropicMessages, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-a", router::ModelProtocol::AnthropicMessages),
            (&b, "wire-b", router::ModelProtocol::AnthropicMessages)]);
        let (client, base) = start(&paths, &store).await;
        for round in 0..3 {
            for _ in 0..3 {
                let response = client.post(format!("{base}/aggregate/v1/messages/count_tokens"))
                    .bearer_auth("local-fixture").json(&json!({"model":"official-model","messages":[]})).send().await.unwrap();
                assert_eq!(response.status(), 500);
                response.bytes().await.unwrap();
                wait_for_released_lease().await;
            }
            let response = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","max_tokens":64,"messages":[{"role":"user","content":format!("failure {round}")}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 503);
            response.bytes().await.unwrap();
            wait_for_released_lease().await;
            assert_eq!(b.count.load(Ordering::SeqCst), 0);
        }
        let response = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","max_tokens":64,"messages":[{"role":"user","content":"after threshold"}]}))
            .send().await.unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        wait_for_released_lease().await;
        assert_eq!(a.count.load(Ordering::SeqCst), 12);
        assert_eq!(b.count.load(Ordering::SeqCst), 1);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 4);
    });
}

#[test]
fn streaming_channel_errors_and_eof_switch_only_later_requests_across_all_egresses() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
            for scenario in ["channel", "truncated", "parameter"] {
                let _stop = Stop;
                let a = scripted_mock(move |body| {
                    let opening = match protocol {
                        router::ModelProtocol::Responses => json!({"type":"response.created","response":{
                            "id":"resp_fault","model":body["model"],"status":"in_progress","output":[]}}),
                        router::ModelProtocol::ChatCompletions => json!({"id":"chat_fault","object":"chat.completion.chunk",
                            "model":body["model"],"choices":[{"index":0,"delta":{"role":"assistant","content":"started"},"finish_reason":null}]}),
                        router::ModelProtocol::AnthropicMessages => json!({"type":"message_start","message":{
                            "type":"message","id":"msg_fault","role":"assistant","model":body["model"],"content":[]}}),
                    };
                    let terminal = if scenario == "truncated" { String::new() } else {
                        let error = if protocol == router::ModelProtocol::Responses {
                            json!({"type":"error","code":if scenario == "parameter" {"invalid_request_error"} else {"server_error"},
                                "message":"synthetic failure"})
                        } else {
                            json!({"type":"error","error":{"type":if scenario == "parameter" {"invalid_request_error"} else {"api_error"},
                                "message":"synthetic failure"}})
                        };
                        format!("event: error\ndata: {error}\n\n")
                    };
                    scripted_response(200, format!("data: {opening}\n\n{terminal}"), "text/event-stream")
                }).await;
                let b = mock(protocol, 200, true).await;
                let temp = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(temp.path().into());
                let store = fixture(&[(&a, "wire-a", protocol), (&b, "wire-b", protocol)]);
                let (client, base) = start(&paths, &store).await;
                for index in 0..4 {
                    let response = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                        .json(&json!({"model":"official-model","stream":true,"max_tokens":64,
                            "messages":[{"role":"user","content":format!("request {index}")}]})).send().await.unwrap();
                    assert_eq!(response.status(), 200);
                    let text = response.text().await.unwrap();
                    if index < 3 || scenario == "parameter" {
                        assert!(text.contains("error"), "{protocol:?}/{scenario}: {text}");
                        assert_eq!(b.count.load(Ordering::SeqCst), 0, "a committed stream cannot be replayed");
                    } else {
                        assert!(text.contains("message_stop"), "{protocol:?}/{scenario}: {text}");
                    }
                    wait_for_released_lease().await;
                }
                assert_eq!(a.count.load(Ordering::SeqCst), if scenario == "parameter" {4} else {3},
                    "parameter rejection must not become a channel fault after conversion");
                assert_eq!(b.count.load(Ordering::SeqCst), usize::from(scenario != "parameter"));
                let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
                assert_eq!(ledger.all_time.request_count, 4);
                let traces: Vec<_> = route_trace::recent().into_iter().filter(|trace| trace.attempts.first()
                    .is_some_and(|a| a.aggregate_id.as_deref() == Some(&store.aggregates[0].id))).collect();
                assert_eq!(traces.len(), 4);
                assert!(traces.iter().all(|trace| trace.attempts.len() == 1));
            }
        }
    });
}

async fn wait_for_released_lease() {
    tokio::time::timeout(Duration::from_secs(3), async {
        while aggregation::runtime::shared().stats().in_flight != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropping the downstream body must release the physical upstream slot");
}

fn desktop_aggregate_state(
    paths: &CodexPaths,
    port: u16,
    aggregate: &aggregation::AggregateModel,
) -> String {
    use crate::core::claude_desktop::{
        constants,
        gateway::RouterConfig,
        paths::DesktopPaths,
        provider::DesktopVersionPolicy,
        transaction,
        types::{DesktopRouteChoice, OneMMode},
    };
    let alias = constants::route_alias(
        "sonnet",
        DesktopVersionPolicy {
            fable_allowed: true,
        },
    )
    .unwrap()
    .to_owned();
    let roots = DesktopPaths::from_roots(
        paths.app_data_dir.join("desktop-normal"),
        paths.app_data_dir.join("desktop-third-party"),
    )
    .unwrap();
    let mut state = transaction::next_state(
        &[],
        &[],
        &roots,
        7,
        "aggregate-fixture",
        "",
        "",
        None,
        &[],
        transaction::AppliedMode::Router,
    )
    .unwrap();
    state.router = Some(RouterConfig {
        routes: vec![DesktopRouteChoice {
            target_ref: Some(crate::core::model_targets::ModelTargetRef::aggregate(
                &aggregate.id,
            )),
            role: "sonnet".into(),
            role_alias: alias.clone(),
            provider_id: String::new(),
            model_id: aggregate.public_model_id.clone(),
            label_override: None,
            one_m_mode: OneMMode::Auto,
        }],
        route_views: vec![],
        file_projection: String::new(),
        route_revision: 7,
        gateway_token_version: 7,
        gateway_token: "desktop-fixture".into(),
        bound_port: port,
    });
    crate::core::secret_file::write(
        &paths.app_data_dir.join("claude-desktop/state.dat"),
        &serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    alias
}

fn assert_one_m_wire(
    request: &Value,
    protocol: router::ModelProtocol,
    expected_model: &str,
    one_m: bool,
) {
    assert_eq!(request["body"]["model"], expected_model);
    assert!(request["body"]["supports1m"].is_null());
    assert!(request["body"]["context_window"].is_null());
    let beta = request["beta"].as_str().unwrap();
    if protocol == router::ModelProtocol::AnthropicMessages {
        assert!(beta.contains("existing-beta"));
        assert_eq!(beta.contains(CLAUDE_ONE_M_BETA), one_m);
        assert_eq!(request["version"], "2023-06-01");
        assert!(request["auth"].as_str().unwrap().starts_with("upstream-"));
    } else {
        assert!(
            beta.is_empty(),
            "Claude beta must not leak into Chat or Responses egress"
        );
        assert_eq!(request["version"], "");
        assert!(request["auth"]
            .as_str()
            .unwrap()
            .starts_with("Bearer upstream-"));
    }
}

// This is the same payload consumed by the account page, loaded from the
// persisted ledger after downstream JSON or SSE has been fully delivered.
fn assert_aggregate_accounting(paths: &CodexPaths, store: &router::Store, per_provider: u64) {
    assert_aggregate_accounting_counts(paths, store, &vec![per_provider; store.providers.len()]);
}
fn assert_aggregate_accounting_counts(paths: &CodexPaths, store: &router::Store, counts: &[u64]) {
    let usage = relay_usage::load(&paths.app_data_dir).unwrap();
    let count: u64 = counts.iter().sum();
    assert_eq!(usage.all_time.request_count, count);
    assert_eq!(usage.all_time.reported_count, count);
    assert_eq!(usage.all_time.successful_count, count);
    assert_eq!(usage.all_time.total_tokens, count * 6);
    assert_eq!(usage.recent.len() as u64, count);
    assert_eq!(usage.by_provider.len(), counts.iter().filter(|count| **count > 0).count());
    assert_eq!(usage.by_model.len(), counts.iter().filter(|count| **count > 0).count());
    let attempts: std::collections::HashSet<_> = usage.recent.iter()
        .map(|record| record.attempt_id.as_str()).collect();
    assert_eq!(attempts.len() as u64, count, "one record per actual inference attempt");
    for (provider, per_provider) in store.providers.iter().zip(counts.iter().copied()) {
        if per_provider == 0 {
            assert!(!usage.by_provider.iter().any(|row| row.provider_id == provider.id));
            continue;
        }
        let by_provider = usage.by_provider.iter().find(|row| row.provider_id == provider.id).unwrap();
        assert_eq!(by_provider.provider_name, provider.name);
        assert_eq!(by_provider.totals.request_count, per_provider);
        assert_eq!(by_provider.totals.total_tokens, per_provider * 6);
        let by_model = usage.by_model.iter().find(|row| row.provider_id == provider.id).unwrap();
        assert_eq!(by_model.model, provider.model, "group by billed wire model, not public alias");
        assert_eq!(by_model.totals.request_count, per_provider);
        assert_eq!(by_model.totals.total_tokens, per_provider * 6);
        assert!(usage.recent.iter().filter(|record| record.provider_id == provider.id)
            .all(|record| record.model == provider.model && record.usage_reported
                && record.tokens.input_tokens == 4 && record.tokens.output_tokens == 2));
    }
}

#[test]
fn mixed_protocol_selection_preserves_wire_models_public_projection_and_original_observation() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::Responses, 200, false).await;
        let b = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[
            (&a, "wire-native", router::ModelProtocol::Responses),
            (
                &b,
                "platform/wire-fast",
                router::ModelProtocol::ChatCompletions,
            ),
        ]);
        let (client, base) = start(&paths, &store).await;
        for index in 0..4 {
            select_test_member(&paths, &store, index / 2);
            let response = client
                .post(format!("{base}/aggregate/v1/responses"))
                .bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","input":"hello"}))
                .send()
                .await
                .unwrap();
            let status = response.status();
            let value: Value = response.json().await.unwrap();
            assert_eq!(status, 200, "{value}");
            assert_eq!(value["model"], "official-model");
        }
        assert_eq!(a.count.load(Ordering::SeqCst), 2);
        assert_eq!(b.count.load(Ordering::SeqCst), 2);
        for request in a.requests.lock().unwrap().iter() {
            assert_eq!(request["path"], "/v1/responses");
            assert_eq!(request["body"]["model"], "wire-native");
            assert_eq!(request["auth"], "Bearer upstream-0");
            assert_eq!(request["local"], false);
        }
        for request in b.requests.lock().unwrap().iter() {
            assert_eq!(request["path"], "/v1/chat/completions");
            assert_eq!(request["body"]["model"], "platform/wire-fast");
            assert_eq!(request["auth"], "Bearer upstream-1");
        }
        let traces: Vec<_> = route_trace::recent()
            .into_iter()
            .filter(|route| {
                route
                    .attempts
                    .iter()
                    .any(|attempt| attempt.aggregate_id.as_deref() == Some(&store.aggregates[0].id))
            })
            .collect();
        assert_eq!(traces.len(), 4);
        assert!(traces.iter().all(|route| route.attempts.len() == 1
            && route.attempts[0].model_observation.served_model.as_deref()
                == Some(route.attempts[0].upstream_model.as_str())));
        let unknown = client
            .post(format!("{base}/aggregate/v1/responses"))
            .bearer_auth("local-fixture")
            .json(&json!({"model":"unknown","input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 404);
        assert_eq!(
            a.count.load(Ordering::SeqCst) + b.count.load(Ordering::SeqCst),
            4
        );
        assert_aggregate_accounting(&paths, &store, 2);
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.cache_usage.reported_count, 4);
        assert_eq!(ledger.all_time.cache_usage.prompt_tokens, 16);
        assert_eq!(ledger.all_time.cache_usage.read_tokens, 0);
        assert!(ledger.by_provider.iter().all(|row| row.totals.cache_usage.reported_count == 2
            && row.totals.cache_usage.prompt_tokens == 8));
    });
}

#[test]
fn aggregate_chat_ingress_covers_all_egresses_with_json_and_sse() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for streaming in [false, true] {
            let responses = mock(router::ModelProtocol::Responses, 200, streaming).await;
            let messages = mock(router::ModelProtocol::AnthropicMessages, 200, streaming).await;
            let chat = mock(router::ModelProtocol::ChatCompletions, 200, streaming).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().into());
            let store = fixture(&[
                (&responses, "wire-responses", router::ModelProtocol::Responses),
                (&messages, "wire-messages", router::ModelProtocol::AnthropicMessages),
                (&chat, "wire-chat", router::ModelProtocol::ChatCompletions),
            ]);
            let (client, base) = start(&paths, &store).await;
            for index in 0..6 {
                select_test_member(&paths, &store, index / 2);
                let response = client.post(format!("{base}/aggregate/v1/chat/completions"))
                    .bearer_auth("local-fixture")
                    .json(&json!({"model":"official-model", "stream":streaming,
                        "messages":[{"role":"system","content":"Be brief"},{"role":"user","content":"hello"}],
                        "max_completion_tokens":64, "stream_options":{"include_usage":true}}))
                    .send().await.unwrap();
                let status = response.status();
                let bytes = response.bytes().await.unwrap();
                assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
                if streaming {
                    let text = String::from_utf8(bytes.to_vec()).unwrap();
                    let values: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
                        .filter(|data| *data != "[DONE]")
                        .map(|data| serde_json::from_str(data).expect("each Chat SSE data frame is JSON")).collect();
                    assert!(text.contains("data: [DONE]"), "{text}");
                    assert!(values.iter().any(|value| value["choices"][0]["delta"]["content"] == "hello"), "{text}");
                    assert!(values.iter().filter(|value| value["object"] == "chat.completion.chunk")
                        .all(|value| value["model"] == "official-model"), "{text}");
                    assert!(values.iter().any(|value| value["choices"][0]["finish_reason"] == "stop"), "{text}");
                    assert!(values.iter().any(|value| value["usage"]["prompt_tokens"] == 4 && value["usage"]["completion_tokens"] == 2), "{text}");
                } else {
                    let value: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(value["object"], "chat.completion");
                    assert_eq!(value["model"], "official-model");
                    assert_eq!(value["choices"][0]["message"]["content"], "hello");
                    assert_eq!(value["choices"][0]["finish_reason"], "stop");
                    assert_eq!(value["usage"]["prompt_tokens"], 4);
                    assert_eq!(value["usage"]["completion_tokens"], 2);
                }
            }
            for (upstream, suffix, wire) in [(&responses, "/v1/responses", "wire-responses"),
                (&messages, "/v1/messages", "wire-messages"), (&chat, "/v1/chat/completions", "wire-chat")] {
                assert_eq!(upstream.count.load(Ordering::SeqCst), 2);
                for request in upstream.requests.lock().unwrap().iter() {
                    assert_eq!(request["path"], suffix);
                    assert_eq!(request["body"]["model"], wire);
                    assert_eq!(request["body"]["stream"], streaming);
                    assert_eq!(request["local"], false);
                }
            }
            wait_for_released_lease().await;
            assert_aggregate_accounting(&paths, &store, 2);
        }
    });
}

fn chat_tool_result(protocol: router::ModelProtocol, streaming: bool) -> Response<ResponseBody> {
    let usage = json!({"input_tokens":4,"output_tokens":2});
    let result = match protocol {
        router::ModelProtocol::Responses => {
            json!({"id":"resp_chat_tools","object":"response","model":"wire-tool",
            "status":"completed","output":[{"type":"function_call","id":"item-next","call_id":"call-next",
                "name":"lookup","arguments":"{\"query\":\"next\"}"}],"usage":usage})
        }
        router::ModelProtocol::AnthropicMessages => {
            json!({"id":"msg_chat_tools","type":"message","model":"wire-tool","role":"assistant",
            "content":[{"type":"tool_use","id":"call-next","name":"lookup","input":{"query":"next"}}],
            "stop_reason":"tool_use","usage":usage})
        }
        router::ModelProtocol::ChatCompletions => {
            json!({"id":"chat_tools","object":"chat.completion","model":"wire-tool",
            "choices":[{"index":0,"message":{"role":"assistant","content":null,
                "tool_calls":[{"id":"call-next","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"next\"}"}}]},
                "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4,"completion_tokens":2}})
        }
    };
    if !streaming {
        return scripted_response(200, result.to_string(), "application/json");
    }
    let events = match protocol {
        router::ModelProtocol::Responses => vec![
            json!({"type":"response.created","response":{"id":"resp_chat_tools","model":"wire-tool","status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"item-next","call_id":"call-next","name":"lookup","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"item-next","output_index":0,"delta":"{\"query\":\"next\"}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":result["output"][0]}),
            json!({"type":"response.completed","response":result}),
        ],
        router::ModelProtocol::AnthropicMessages => vec![
            json!({"type":"message_start","message":{"id":"msg_chat_tools","type":"message","role":"assistant","model":"wire-tool","content":[],"usage":{"input_tokens":4,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call-next","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"next\"}"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":2}}),
            json!({"type":"message_stop"}),
        ],
        router::ModelProtocol::ChatCompletions => vec![
            json!({"id":"chat_tools","object":"chat.completion.chunk","model":"wire-tool","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call-next","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]}),
            json!({"id":"chat_tools","object":"chat.completion.chunk","model":"wire-tool","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":\"next\"}"}}]},"finish_reason":null}]}),
            json!({"id":"chat_tools","object":"chat.completion.chunk","model":"wire-tool","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4,"completion_tokens":2}}),
        ],
    };
    let mut output: String = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    if protocol == router::ModelProtocol::ChatCompletions {
        output.push_str("data: [DONE]\n\n");
    }
    scripted_response(200, output, "text/event-stream")
}

#[test]
fn aggregate_chat_tools_preserve_ids_arguments_choice_and_completed_history_across_egresses() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::AnthropicMessages, router::ModelProtocol::ChatCompletions] {
            for streaming in [false, true] {
                let upstream = scripted_mock(move |_| chat_tool_result(protocol, streaming)).await;
                let temp = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(temp.path().into());
                let store = fixture(&[(&upstream, "wire-tool", protocol)]);
                let (client, base) = start(&paths, &store).await;
                let response = client.post(format!("{base}/aggregate/v1/chat/completions"))
                    .bearer_auth("local-fixture")
                    .json(&json!({"model":"official-model","stream":streaming,"max_tokens":64,
                        "messages":[{"role":"user","content":"lookup"},
                            {"role":"assistant","content":null,"tool_calls":[{"id":"call-prior","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"prior\"}"}}]},
                            {"role":"tool","tool_call_id":"call-prior","content":"prior result"},
                            {"role":"user","content":"next"}],
                        "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}}],
                        "tool_choice":{"type":"function","function":{"name":"lookup"}},"parallel_tool_calls":false,
                        "stream_options":{"include_usage":true}})).send().await.unwrap();
                let status = response.status();
                let text = response.text().await.unwrap();
                assert_eq!(status, 200, "{protocol:?}: {text}");
                if streaming {
                    let values: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
                        .filter(|data| *data != "[DONE]").map(|data| serde_json::from_str(data).unwrap()).collect();
                    assert!(text.contains("data: [DONE]"), "{text}");
                    let calls: Vec<_> = values.iter().flat_map(|v| v["choices"][0]["delta"]["tool_calls"].as_array().into_iter().flatten()).collect();
                    assert!(calls.iter().any(|call| call["id"] == "call-next" && call["function"]["name"] == "lookup"), "{text}");
                    let arguments: String = calls.iter().filter_map(|call| call["function"]["arguments"].as_str()).collect();
                    assert_eq!(arguments, "{\"query\":\"next\"}", "{text}");
                    assert!(values.iter().any(|value| value["choices"][0]["finish_reason"] == "tool_calls"), "{text}");
                    assert!(values.iter().filter(|value| value["object"] == "chat.completion.chunk").all(|value| value["model"] == "official-model"));
                } else {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(value["model"], "official-model");
                    assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
                    assert_eq!(value["choices"][0]["message"]["tool_calls"][0]["id"], "call-next");
                    assert_eq!(value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"], "{\"query\":\"next\"}");
                }
                let captured = upstream.requests.lock().unwrap().last().unwrap().clone();
                let body = &captured["body"];
                assert_eq!(body["model"], "wire-tool");
                match protocol {
                    router::ModelProtocol::Responses => {
                        let input = body["input"].as_array().unwrap();
                        assert!(input.iter().any(|item| item["type"] == "function_call" && item["call_id"] == "call-prior"));
                        assert!(input.iter().any(|item| item["type"] == "function_call_output" && item["call_id"] == "call-prior" && item["output"] == "prior result"));
                        assert_eq!(body["tool_choice"]["name"], "lookup");
                    }
                    router::ModelProtocol::AnthropicMessages => {
                        assert_eq!(body["tool_choice"]["type"], "tool");
                        assert_eq!(body["tool_choice"]["name"], "lookup");
                        let content: Vec<_> = body["messages"].as_array().unwrap().iter().flat_map(|message| message["content"].as_array().into_iter().flatten()).collect();
                        assert!(content.iter().any(|item| item["type"] == "tool_use" && item["id"] == "call-prior"));
                        assert!(content.iter().any(|item| item["type"] == "tool_result" && item["tool_use_id"] == "call-prior" && item["content"] == "prior result"));
                    }
                    router::ModelProtocol::ChatCompletions => {
                        assert_eq!(body["tool_choice"]["function"]["name"], "lookup");
                        assert!(body["messages"].as_array().unwrap().iter().any(|message| message["role"] == "tool" && message["tool_call_id"] == "call-prior"));
                    }
                }
                wait_for_released_lease().await;
            }
        }
    });
}

#[test]
fn aggregate_chat_errors_and_hosted_tool_preflight_stay_at_protocol_boundaries() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::AnthropicMessages, router::ModelProtocol::ChatCompletions] {
            let upstream = mock(protocol, 400, false).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().into());
            let store = fixture(&[(&upstream, "wire-error", protocol)]);
            let (client, base) = start(&paths, &store).await;
            let response = client.post(format!("{base}/aggregate/v1/chat/completions")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","messages":[{"role":"user","content":"hello"}],"max_tokens":64}))
                .send().await.unwrap();
            assert_eq!(response.status(), 400);
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["error"]["message"], "synthetic upstream rejection");
            assert_eq!(upstream.count.load(Ordering::SeqCst), 1);
            // A hosted tool is not assumed portable just because ordinary
            // function calls work. Cross-protocol admission rejects it before
            // touching an upstream; the native Chat request is preserved.
            let unsupported = client.post(format!("{base}/aggregate/v1/chat/completions")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","messages":[{"role":"user","content":"search"}],
                    "tools":[{"type":"vendor_hosted_search","name":"search"}]})).send().await.unwrap();
            assert_eq!(unsupported.status(), 400);
            assert_eq!(upstream.count.load(Ordering::SeqCst), if protocol == router::ModelProtocol::ChatCompletions { 2 } else { 1 });
        }
    });
}

#[test]
fn aggregate_native_chat_keeps_unknown_fields_events_and_rejects_missing_terminal() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let upstream = scripted_mock(|body| {
            assert_eq!(body["vendor_control"]["mode"], "native");
            if body["stream"] == true {
                let known = json!({"id":"chat_native","object":"chat.completion.chunk","model":"wire-native",
                    "choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}],"vendor":{"model":"keep-nested"}});
                let future = "event: vendor.future\r\ndata: {\"type\":\"vendor.future\",\"model\":\"keep-unknown\"}\r\n\r\n";
                let mut output = format!("{future}data: {known}\n\n");
                if body["vendor_control"]["complete"] == true {
                    let terminal = json!({"id":"chat_native","object":"chat.completion.chunk","model":"wire-native",
                        "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                    output.push_str(&format!("data: {terminal}\n\ndata: [DONE]\n\n"));
                }
                scripted_response(200, output, "text/event-stream")
            } else {
                scripted_response(200, json!({"id":"chat_native","object":"chat.completion","model":"wire-native",
                    "choices":[{"index":0,"message":{"role":"assistant","content":"hello","vendor_message":{"raw":true}},"finish_reason":"stop"}],
                    "vendor":{"model":"keep-nested","opaque":[1,2,3]},"usage":{"prompt_tokens":4,"completion_tokens":2}}).to_string(), "application/json")
            }
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&upstream, "wire-native", router::ModelProtocol::ChatCompletions)]);
        let (client, base) = start(&paths, &store).await;
        for (streaming, complete) in [(false,true), (true,true), (true,false)] {
            let response = client.post(format!("{base}/aggregate/v1/chat/completions")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","stream":streaming,"messages":[{"role":"user","content":"hello"}],
                    "vendor_control":{"mode":"native","complete":complete}})).send().await.unwrap();
            assert_eq!(response.status(), 200);
            let text = response.text().await.unwrap();
            if streaming {
                assert!(text.contains("event: vendor.future") && text.contains("keep-unknown"), "{text}");
                assert!(text.contains("keep-nested"));
                assert!(text.contains("official-model"));
                if complete { assert!(text.contains("data: [DONE]"), "{text}"); }
                else {
                    assert!(!text.contains("data: [DONE]"), "EOF cannot invent successful completion: {text}");
                    let errors: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
                        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
                        .filter(|value| !value["error"].is_null()).collect();
                    assert!(errors.iter().any(|error| error["error"]["message"] == "upstream_stream_truncated"), "{text}");
                }
            } else {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["model"], "official-model");
                assert_eq!(value["vendor"], json!({"model":"keep-nested","opaque":[1,2,3]}));
                assert_eq!(value["choices"][0]["message"]["vendor_message"], json!({"raw":true}));
            }
        }
        wait_for_released_lease().await;
    });
}

#[test]
fn resource_operations_affinity_and_identity_revocation_do_not_reselect() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::Responses, 200, false).await;
        let b = mock(router::ModelProtocol::Responses, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let mut store = fixture(&[
            (&a, "wire-native", router::ModelProtocol::Responses),
            (&b, "other-wire", router::ModelProtocol::Responses),
        ]);
        let (client, base) = start(&paths, &store).await;
        let send = |body: Value| {
            client
                .post(format!("{base}/aggregate/v1/responses"))
                .bearer_auth("local-fixture")
                .header("session_id", "same-session")
                .json(&body)
                .send()
        };
        let first: Value = send(json!({"model":"official-model","input":"hello"}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = first["id"].as_str().unwrap();
        let created_usage = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
        assert_eq!(created_usage.request_count, 1);
        assert_eq!(created_usage.total_tokens, 6);
        let resource = client
            .get(format!("{base}/aggregate/v1/responses/{id}"))
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(resource.status(), 200);
        resource.bytes().await.unwrap();
        let retrieved_usage = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
        assert_eq!(
            retrieved_usage.request_count, created_usage.request_count,
            "retrieving a response must not count the original inference twice"
        );
        assert_eq!(retrieved_usage.total_tokens, created_usage.total_tokens);
        store.providers[0].name = "Renamed".into();
        store.providers[0].updated_at += 1;
        store.aggregates[0].members[0].weight = 3;
        persist(&paths, &store);
        let next = send(json!({"model":"official-model","previous_response_id":id,"input":"next"}))
            .await
            .unwrap();
        assert_eq!(next.status(), 200);
        next.bytes().await.unwrap();
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        assert_eq!(a.count.load(Ordering::SeqCst), 3);
        store.providers[0].api_key = "changed-key".into();
        persist(&paths, &store);
        let rejected =
            send(json!({"model":"official-model","previous_response_id":id,"input":"next"}))
                .await
                .unwrap();
        assert_eq!(rejected.status(), 409);
        let unknown = client
            .get(format!("{base}/aggregate/v1/responses/unknown"))
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 409);
        assert_eq!(a.count.load(Ordering::SeqCst), 3);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        assert_eq!(
            relay_usage::load(&paths.app_data_dir)
                .unwrap()
                .all_time
                .request_count,
            2,
            "only the two inference creates belong in the ledger"
        );
    });
}

#[test]
fn native_resource_head_cancel_delete_keep_empty_bodies_and_success_status() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let owner = scripted_request_mock(|request| match request["method"].as_str().unwrap() {
            "HEAD" => scripted_response(200, "", "application/json"),
            "DELETE" => scripted_response(204, "", "application/json"),
            "POST" if request["path"].as_str().unwrap().ends_with("/cancel") => {
                scripted_response(204, "", "application/json")
            }
            _ => scripted_response(
                200,
                json!({"id":"resp_resource_ops","object":"response",
                "status":"completed","model":"owner-wire","output":[]})
                .to_string(),
                "application/json",
            ),
        })
        .await;
        let alternate = mock(router::ModelProtocol::Responses, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[
            (&owner, "owner-wire", router::ModelProtocol::Responses),
            (
                &alternate,
                "alternate-wire",
                router::ModelProtocol::Responses,
            ),
        ]);
        let (client, base) = start(&paths, &store).await;
        let create = client
            .post(format!("{base}/aggregate/v1/responses"))
            .bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(create.status(), 200);
        assert_eq!(
            create.json::<Value>().await.unwrap()["model"],
            "official-model"
        );
        let resource = format!("{base}/aggregate/v1/responses/resp_resource_ops");
        let head = client
            .head(&resource)
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(head.status(), 200);
        assert!(head.bytes().await.unwrap().is_empty());
        let cancel = client
            .post(format!("{resource}/cancel"))
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(cancel.status(), 204);
        assert!(cancel.bytes().await.unwrap().is_empty());
        let delete = client
            .delete(&resource)
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(delete.status(), 204);
        assert!(delete.bytes().await.unwrap().is_empty());
        assert_eq!(owner.count.load(Ordering::SeqCst), 4);
        assert_eq!(alternate.count.load(Ordering::SeqCst), 0);
        assert_eq!(
            relay_usage::load(&paths.app_data_dir)
                .unwrap()
                .all_time
                .request_count,
            1,
            "HEAD, cancel and delete must not add inference records"
        );
        let captured = owner.requests.lock().unwrap();
        assert_eq!(captured[0]["body"]["model"], "owner-wire");
        for (request, method, path) in [
            (&captured[1], "HEAD", "/v1/responses/resp_resource_ops"),
            (
                &captured[2],
                "POST",
                "/v1/responses/resp_resource_ops/cancel",
            ),
            (&captured[3], "DELETE", "/v1/responses/resp_resource_ops"),
        ] {
            assert_eq!(request["method"], method);
            assert_eq!(request["path"], path);
            assert_eq!(request["auth"], "Bearer upstream-0");
            assert_eq!(
                request["bodyBytes"], 0,
                "native resource operations must preserve the empty body"
            );
        }
        drop(captured);
        wait_for_released_lease().await;
    });
}

#[test]
fn aggregate_count_tokens_selects_only_native_messages_and_does_not_pin_inference() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let chat = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let messages = scripted_request_mock(|request| {
            assert_eq!(request["path"], "/v1/messages/count_tokens");
            scripted_response(200, r#"{"input_tokens":23}"#, "application/json")
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let mut store = fixture(&[(&chat, "chat-wire", router::ModelProtocol::ChatCompletions),
            (&messages, "messages-wire", router::ModelProtocol::AnthropicMessages)]);
        let (client, base) = start(&paths, &store).await;
        let body = json!({"model":"official-model","messages":[{"role":"user","content":"Count this"},
            {"role":"assistant","content":[{"type":"thinking","thinking":"old plan","signature":"unknown-signature"},{"type":"text","text":"answer"}]},
            {"role":"user","content":"Count the complete history"}],
            "system":"native options","tools":[{"name":"lookup","input_schema":{"type":"object"}}]});
        let counted = client.post(format!("{base}/aggregate/v1/messages/count_tokens"))
            .bearer_auth("local-fixture").header("session_id", "count-before-create")
            .json(&body).send().await.unwrap();
        assert_eq!(counted.status(), 200);
        assert_eq!(counted.json::<Value>().await.unwrap(), json!({"input_tokens":23}));
        assert_eq!(chat.count.load(Ordering::SeqCst), 0);
        assert_eq!(messages.count.load(Ordering::SeqCst), 1);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 0,
            "token counting is diagnostic-only, not a billable inference");
        let request = messages.requests.lock().unwrap()[0].clone();
        assert_eq!(request["method"], "POST");
        assert_eq!(request["auth"], "upstream-1");
        assert_eq!(request["version"], "2023-06-01");
        let mut expected = body;
        expected["model"] = json!("messages-wire");
        assert_eq!(request["body"], expected);
        let inference = client.post(format!("{base}/aggregate/v1/messages"))
            .bearer_auth("local-fixture").header("session_id", "count-before-create")
            .json(&json!({"model":"official-model","max_tokens":128,"messages":[{"role":"user","content":"hello"}]}))
            .send().await.unwrap();
        assert_eq!(inference.status(), 200);
        assert_eq!(inference.json::<Value>().await.unwrap()["model"], "official-model");
        assert_eq!(chat.count.load(Ordering::SeqCst), 1, "token count must not bind the subsequent inference to Messages");
        let request = chat.requests.lock().unwrap()[0].clone();
        assert_eq!(request["path"], "/v1/chat/completions");
        assert_eq!(request["auth"], "Bearer upstream-0");
        assert_eq!(request["body"]["model"], "chat-wire");
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 1);
        store.aggregates[0].members[1].enabled = false;
        persist(&paths, &store);
        let unsupported = client.post(format!("{base}/aggregate/v1/messages/count_tokens"))
            .bearer_auth("local-fixture").json(&json!({"model":"official-model","messages":[]}))
            .send().await.unwrap();
        assert_eq!(unsupported.status(), 501);
        assert!(unsupported.text().await.unwrap().contains("native Anthropic Messages"));
        assert_eq!(chat.count.load(Ordering::SeqCst), 1);
        assert_eq!(messages.count.load(Ordering::SeqCst), 1);
        wait_for_released_lease().await;
    });
}

#[test]
fn desktop_aggregate_counts_chat_locally_without_binding_or_billing() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let chat = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&chat, "chat-wire", router::ModelProtocol::ChatCompletions)]);
        let (client, base) = start(&paths, &store).await;
        let alias = desktop_aggregate_state(&paths, running_port().unwrap(), &store.aggregates[0]);
        let latest_trace = route_trace::recent().first().map(|route| route.id.clone());
        let counted = client.post(format!("{base}/claude-desktop/v1/messages/count_tokens"))
            .bearer_auth("desktop-fixture")
            .json(&json!({"model":alias.clone(),"messages":[{"role":"user","content":"hello"}]}))
            .send().await.unwrap();
        assert_eq!(counted.status(), 200);
        assert!(counted.json::<Value>().await.unwrap()["input_tokens"].as_u64().unwrap() > 0);
        assert_eq!(chat.count.load(Ordering::SeqCst), 0);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 0);
        assert_eq!(route_trace::recent().first().map(|route| route.id.clone()), latest_trace);

        let inference = client.post(format!("{base}/claude-desktop/v1/messages"))
            .bearer_auth("desktop-fixture")
            .json(&json!({"model":alias,"max_tokens":128,"messages":[{"role":"user","content":"hello"}]}))
            .send().await.unwrap();
        assert_eq!(inference.status(), 200);
        inference.bytes().await.unwrap();
        assert_eq!(chat.count.load(Ordering::SeqCst), 1);
        wait_for_released_lease().await;
    });
}

#[test]
fn third_consecutive_429_switches_with_actual_member_price_snapshots() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = scripted_mock(|_| {
            let mut response = scripted_response(429,
                r#"{"error":{"message":"synthetic upstream rejection"}}"#, "application/json");
            response.headers_mut().remove("retry-after");
            response
        }).await;
        let b = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let mut store = fixture(&[
            (&a, "wire-native", router::ModelProtocol::Responses),
            (&b, "other-wire", router::ModelProtocol::ChatCompletions),
        ]);
        for (provider, (input, output)) in
            store.providers.iter_mut().zip([(3.0, 5.0), (11.0, 13.0)])
        {
            provider.models[0].overrides.price = Some(super::super::newapi::PriceOverrides {
                input: Some(input),
                output: Some(output),
                ..Default::default()
            });
        }
        let (client, base) = start(&paths, &store).await;
        for _ in 0..2 {
            let failed = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","input":"hello"})).send().await.unwrap();
            assert_eq!(failed.status(), 429);
            failed.bytes().await.unwrap();
            assert_eq!(b.count.load(Ordering::SeqCst), 0, "a single failure must not use a backup");
            wait_for_released_lease().await;
        }
        let response = client
            .post(format!("{base}/aggregate/v1/responses"))
            .bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        assert_eq!(a.count.load(Ordering::SeqCst), 3);
        assert_eq!(b.count.load(Ordering::SeqCst), 1);
        let route = route_trace::recent()
            .into_iter()
            .find(|route| route.attempts.len() == 2 && {
                route.attempts.first().is_some_and(|attempt| {
                    attempt.aggregate_id.as_deref() == Some(&store.aggregates[0].id)
                })
            })
            .unwrap();
        assert_eq!(route.attempts.len(), 2);
        assert_ne!(route.attempts[0].attempt_id, route.attempts[1].attempt_id);
        assert_eq!(route.attempts[0].status, Some(429));
        assert_eq!(
            route.attempts[1].retry_reason.as_deref(),
            Some("aggregate_rate_limited")
        );
        assert_eq!(route.attempts[1].upstream_model, "other-wire");
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        let records: Vec<_> = ledger
            .recent
            .iter()
            .filter(|record| record.request_id == route.id)
            .collect();
        assert_eq!(records.len(), 2);
        for (index, (input, output)) in [(3.0, 5.0), (11.0, 13.0)].into_iter().enumerate() {
            let record = records
                .iter()
                .find(|record| record.attempt_id == route.attempts[index].attempt_id)
                .unwrap();
            assert_eq!(record.provider_id, format!("p{index}"));
            assert_eq!(record.model, route.attempts[index].upstream_model);
            assert_eq!(
                record.identity.requested_model.as_deref(),
                Some("official-model")
            );
            assert_eq!(
                record.identity.resolved_model.as_deref(),
                Some(record.model.as_str())
            );
            let pricing = record.pricing_snapshot.as_ref().unwrap();
            assert_eq!(pricing.input, Some(input));
            assert_eq!(pricing.output, Some(output));
            assert_eq!(pricing.source, "manual");
        }
        let completed = records
            .iter()
            .find(|record| record.status_code == 200)
            .unwrap();
        assert_eq!(completed.tokens.input_tokens, 4);
        assert_eq!(completed.tokens.output_tokens, 2);
        assert!((completed.cost_usd.unwrap() - 0.000070).abs() < 1e-12);
        assert_eq!(
            completed.model_observation.served_model.as_deref(),
            Some("other-wire")
        );
        assert_eq!(aggregation::runtime::shared().stats().in_flight, 0);
    });
}

#[test]
fn declared_tools_never_replay_a_rate_limited_request_on_another_member() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::Responses, 429, false).await;
        let b = mock(router::ModelProtocol::Responses, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses), (&b, "other-wire", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let response = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"read the file",
                "tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}))
            .send().await.unwrap();
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers()["retry-after"], "30");
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["error"]["message"], "synthetic upstream rejection");
        assert_eq!(a.count.load(Ordering::SeqCst), 1);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        assert_eq!(a.requests.lock().unwrap()[0]["body"]["tools"][0]["name"], "read_file");
        wait_for_released_lease().await;
    });
}

#[test]
fn no_alternative_preserves_upstream_429_and_cooling_request_has_retry_after() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::Responses, 429, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let request = || {
            client
                .post(format!("{base}/aggregate/v1/responses"))
                .bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","input":"hello"}))
                .send()
        };
        let first = request().await.unwrap();
        assert_eq!(first.status(), 429);
        assert_eq!(first.headers()["retry-after"], "30");
        assert_eq!(
            first.json::<Value>().await.unwrap()["error"]["message"],
            "synthetic upstream rejection"
        );
        let second = request().await.unwrap();
        assert_eq!(second.status(), 429);
        let retry_after = second.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!((1..=30).contains(&retry_after));
        assert_eq!(
            second.json::<Value>().await.unwrap()["error"]["message"],
            "aggregate members are cooling down"
        );
        assert_eq!(a.count.load(Ordering::SeqCst), 1);
        wait_for_released_lease().await;
    });
}

#[test]
fn revocation_between_429_and_retry_vetoes_frozen_credentials_before_dispatch() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let revoked = Arc::new(Mutex::new(None::<router::Store>));
        let changed = revoked.clone();
        let write_paths = paths.clone();
        let a = scripted_mock(move |_| {
            let mut current = changed.lock().unwrap().as_ref().unwrap().clone();
            current.providers[1].api_key = format!("new-credential-{}", uuid::Uuid::new_v4());
            persist(&write_paths, &current);
            let mut response = scripted_response(
                429,
                r#"{"error":{"message":"retry is now revoked"}}"#,
                "application/json",
            );
            response.headers_mut().remove("retry-after");
            response
        })
        .await;
        let b = mock(router::ModelProtocol::Responses, 200, false).await;
        let store = fixture(&[
            (&a, "wire-native", router::ModelProtocol::Responses),
            (&b, "other-wire", router::ModelProtocol::Responses),
        ]);
        *revoked.lock().unwrap() = Some(store.clone());
        let (client, base) = start(&paths, &store).await;
        for _ in 0..2 {
            let failed = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","input":"hello"})).send().await.unwrap();
            assert_eq!(failed.status(), 429);
            failed.bytes().await.unwrap();
            wait_for_released_lease().await;
        }
        let response = client
            .post(format!("{base}/aggregate/v1/responses"))
            .bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert!(response
            .text()
            .await
            .unwrap()
            .contains("changed before dispatch"));
        assert_eq!(a.count.load(Ordering::SeqCst), 3);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        wait_for_released_lease().await;
    });
}

#[test]
fn oversized_second_sse_event_in_same_chunk_is_replaced_before_delivery() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = scripted_mock(|_| {
            let opening = json!({"type":"response.created","response":{"id":"resp_size_limit","object":"response","status":"in_progress","model":"wire-native","output":[]}});
            let oversized = json!({"type":"response.output_text.delta","delta":"z".repeat(256 * 1024 + 1)});
            scripted_response(200, format!("event: response.created\ndata: {opening}\n\nevent: response.output_text.delta\ndata: {oversized}\n\n"), "text/event-stream")
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let response = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello","stream":true})).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let text = response.text().await.unwrap();
        assert!(text.contains("\"model\":\"official-model\""));
        assert!(text.contains("aggregate_event_limit"));
        assert!(!text.contains("response.output_text.delta"));
        assert!(text.len() < 4096);
        assert_eq!(a.count.load(Ordering::SeqCst), 1);
        wait_for_released_lease().await;
    });
}

#[test]
fn cancelling_downstream_native_stream_releases_its_physical_lease() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = scripted_mock(|_| {
            let opening = Bytes::from(format!("event: response.created\ndata: {}\n\n", json!({"type":"response.created","response":{"id":"resp_cancelled_stream","object":"response","status":"in_progress","model":"wire-native","output":[]}})));
            let stream = futures_util::stream::once(async move {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(opening))
            }).chain(futures_util::stream::pending());
            Response::builder().status(200).header(CONTENT_TYPE, "text/event-stream")
                .body(BodyExt::boxed(StreamBody::new(stream))).unwrap()
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let response = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello","stream":true})).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let mut stream = response.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(3), stream.next()).await.unwrap().unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("official-model"));
        assert_eq!(aggregation::runtime::shared().stats().in_flight, 1);
        drop(stream);
        wait_for_released_lease().await;
        assert_eq!(a.count.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn older_completed_stream_does_not_clear_a_newer_rate_limit_cooldown() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let gate = Arc::new(tokio::sync::Notify::new());
        let release = gate.clone();
        let request_index = Arc::new(AtomicUsize::new(0));
        let a = scripted_mock(move |_| {
            match request_index.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    let release = release.clone();
                    let response = json!({"id":"resp_before_limit","object":"response","status":"in_progress","model":"wire-native","output":[]});
                    let opening = Bytes::from(format!("event: response.created\ndata: {}\n\n", json!({"type":"response.created","response":response})));
                    let mut completed = response;
                    completed["status"] = json!("completed");
                    let terminal = Bytes::from(format!("event: response.completed\ndata: {}\n\n", json!({"type":"response.completed","response":completed})));
                    let stream = futures_util::stream::once(async move {
                        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(opening))
                    }).chain(futures_util::stream::once(async move {
                        release.notified().await;
                        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(terminal))
                    }));
                    Response::builder().status(200).header(CONTENT_TYPE, "text/event-stream")
                        .body(BodyExt::boxed(StreamBody::new(stream))).unwrap()
                }
                1 => scripted_response(429, r#"{"error":{"message":"newer rate limit"}}"#, "application/json"),
                _ => scripted_response(200, r#"{"id":"resp_after_limit","object":"response","status":"completed","model":"wire-native","output":[]}"#, "application/json"),
            }
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let first = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello","stream":true})).send().await.unwrap();
        let mut first_stream = first.bytes_stream();
        let opening = tokio::time::timeout(Duration::from_secs(3), first_stream.next()).await.unwrap().unwrap().unwrap();
        assert!(String::from_utf8_lossy(&opening).contains("response.created"));
        let second = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"other request"})).send().await.unwrap();
        assert_eq!(second.status(), 429);
        second.bytes().await.unwrap();
        gate.notify_one();
        let mut rest = Vec::new();
        while let Some(chunk) = first_stream.next().await { rest.extend_from_slice(&chunk.unwrap()); }
        assert!(String::from_utf8_lossy(&rest).contains("response.completed"));
        wait_for_released_lease().await;
        let third = client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"still cooling"})).send().await.unwrap();
        assert_eq!(third.status(), 429, "an older successful request must not erase the later 429");
        assert_eq!(third.json::<Value>().await.unwrap()["error"]["message"], "aggregate members are cooling down");
        assert_eq!(a.count.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn duplicate_response_id_from_another_member_is_rejected_before_it_is_exposed() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixed_response = |body: &Value| scripted_response(200, json!({"id":"resp_collision","object":"response","status":"completed","model":body["model"],"output":[]}).to_string(), "application/json");
        let a = scripted_mock(fixed_response).await;
        let b = scripted_mock(fixed_response).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses), (&b, "other-wire", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let request = || client.post(format!("{base}/aggregate/v1/responses")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello"})).send();
        let first = request().await.unwrap();
        assert_eq!(first.status(), 200);
        assert_eq!(first.json::<Value>().await.unwrap()["id"], "resp_collision");
        select_test_member(&paths, &store, 1);
        let second = request().await.unwrap();
        assert_eq!(second.status(), 409);
        let rejected = second.text().await.unwrap();
        assert!(rejected.contains("ownership is ambiguous"));
        assert!(!rejected.contains("resp_collision"));
        let resource = client.get(format!("{base}/aggregate/v1/responses/resp_collision")).bearer_auth("local-fixture").send().await.unwrap();
        assert_eq!(resource.status(), 409);
        resource.bytes().await.unwrap();
        assert_eq!(a.count.load(Ordering::SeqCst), 1);
        assert_eq!(b.count.load(Ordering::SeqCst), 1);
        wait_for_released_lease().await;
    });
}

#[test]
fn native_stream_projects_known_fields_preserves_unknown_event_and_indexes_id() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::Responses, 200, true).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-native", router::ModelProtocol::Responses)]);
        let (client, base) = start(&paths, &store).await;
        let response = client
            .post(format!("{base}/aggregate/v1/responses"))
            .bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":"hello","stream":true}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let text = response.text().await.unwrap();
        assert!(text.contains("\"model\":\"official-model\""));
        assert!(text.contains("\"model\":\"keep-unknown\""));
        assert!(!text.contains("\"model\":\"wire-native\""));
        let created = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .and_then(|data| serde_json::from_str::<Value>(data).ok())
                    .filter(|event| event["type"] == "response.created")
            })
            .unwrap();
        let id = created["response"]["id"].as_str().unwrap();
        let resource = client
            .get(format!("{base}/aggregate/v1/responses/{id}"))
            .bearer_auth("local-fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(resource.status(), 200);
        resource.bytes().await.unwrap();
        assert_eq!(a.count.load(Ordering::SeqCst), 2);
        assert_eq!(aggregation::runtime::shared().stats().in_flight, 0);
    });
}

#[test]
fn standalone_compact_selects_native_responses_without_a_previous_resource() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let incompatible = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let native = scripted_mock(|body| scripted_response(200, json!({
            "id":"cmp_fixture", "object":"response.compaction", "model":body["model"],
            "output":[{"type":"compaction","encrypted_content":"native-only-opaque"}],
            "usage":{"input_tokens":4,"output_tokens":2}
        }).to_string(), "application/json")).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[
            (&incompatible, "chat-wire", router::ModelProtocol::ChatCompletions),
            (&native, "responses-wire", router::ModelProtocol::Responses),
        ]);
        let (client, base) = start(&paths, &store).await;
        let response = client.post(format!("{base}/aggregate/v1/responses/compact")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","input":[{"role":"user","content":"Compact this complete history"}],"instructions":"Keep the key fact"}))
            .send().await.unwrap();
        assert_eq!(response.status(), 200);
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["object"], "response.compaction");
        assert_eq!(value["model"], "official-model");
        assert_eq!(value["output"][0]["encrypted_content"], "native-only-opaque");
        assert_eq!(incompatible.count.load(Ordering::SeqCst), 0);
        assert_eq!(native.count.load(Ordering::SeqCst), 1);
        let captured = native.requests.lock().unwrap()[0].clone();
        assert_eq!(captured["path"], "/v1/responses/compact");
        assert_eq!(captured["auth"], "Bearer upstream-1");
        assert_eq!(captured["body"]["model"], "responses-wire");
        assert!(captured["body"]["previous_response_id"].is_null());
        assert_eq!(captured["body"]["instructions"], "Keep the key fact");
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.recent.len(), 1);
        assert_eq!(ledger.recent[0].provider_id, "p1");
        assert_eq!(ledger.recent[0].model, "responses-wire");
        wait_for_released_lease().await;
    });
}

#[test]
fn aggregate_code_one_m_selects_qualified_capacity_across_egresses() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
            let _stop = Stop;
            let small = mock(protocol, 200, false).await;
            let large = mock(protocol, 200, false).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().into());
            let mut store = fixture(&[(&small, "small-wire", protocol), (&large, "large-wire", protocol)]);
            store.providers[0].models[0].overrides.context_window = Some(128_000);
            let (client, base) = start(&paths, &store).await;
            for (marker, beta) in [("official-model[1m]", "existing-beta".to_owned()), ("official-model", format!("existing-beta,{CLAUDE_ONE_M_BETA}"))] {
                let response = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                    .header("x-agentharbor-client", "claudeCode").header("anthropic-beta", beta)
                    .json(&json!({"model":marker,"max_tokens":128,"messages":[{"role":"user","content":"hello"}]}))
                    .send().await.unwrap();
                assert_eq!(response.status(), 200, "qualified 1M {protocol:?} member must be usable");
                assert_eq!(response.json::<Value>().await.unwrap()["model"], "official-model");
            }
            assert_eq!(large.count.load(Ordering::SeqCst), 2);
            for captured in large.requests.lock().unwrap().iter() { assert_one_m_wire(captured, protocol, "large-wire", true); }
            assert_eq!(small.count.load(Ordering::SeqCst), 0, "a 128K member must never receive a 1M request");
            let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
            assert_eq!(ledger.recent.len(), 2);
            assert!(ledger.recent.iter().all(|record| record.provider_id == "p1" && record.model == "large-wire"
                && record.identity.client_source.as_deref() == Some("claudeCode")
                && record.identity.requested_model.as_deref() == Some("official-model")));
            store.providers[1].models[0].overrides.context_window = Some(200_000);
            persist(&paths, &store);
            let unavailable = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model[1m]","max_tokens":128,"messages":[{"role":"user","content":"capacity changed"}]}))
                .send().await.unwrap();
            assert_eq!(unavailable.status(), 400);
            assert!(unavailable.text().await.unwrap().contains("1M context"));
            assert_eq!(small.count.load(Ordering::SeqCst), 0);
            assert_eq!(large.count.load(Ordering::SeqCst), 2);
            store.providers[1].models[0].overrides.context_window = None;
            persist(&paths, &store);
            let unknown_capacity = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model[1m]","max_tokens":128,"messages":[{"role":"user","content":"unknown capacity"}]}))
                .send().await.unwrap();
            assert_eq!(unknown_capacity.status(), 400);
            unknown_capacity.bytes().await.unwrap();
            assert_eq!(small.count.load(Ordering::SeqCst), 0);
            assert_eq!(large.count.load(Ordering::SeqCst), 2);
            wait_for_released_lease().await;
        }
    });
}

#[test]
fn desktop_aggregate_projects_public_identity_and_keeps_one_m_wire_contract_across_egresses() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
            for streaming in [false, true] {
                let _stop = Stop;
                let upstream = mock(protocol, 200, streaming).await;
                let temp = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(temp.path().into());
                let mut store = fixture(&[(&upstream, "platform/real-wire-fast", protocol)]);
                let (client, base) = start(&paths, &store).await;
                let alias = desktop_aggregate_state(&paths, running_port().unwrap(), &store.aggregates[0]);
                let models: Value = client.get(format!("{base}/claude-desktop/v1/models")).bearer_auth("desktop-fixture")
                    .send().await.unwrap().json().await.unwrap();
                assert_eq!(models["data"][0]["id"], alias);
                assert_eq!(models["data"][0]["supports1m"], true);
                for one_m in [false, true] {
                    let request_alias = if one_m { format!("{alias}[1m]") } else { alias.clone() };
                    let response = client.post(format!("{base}/claude-desktop/v1/messages")).bearer_auth("desktop-fixture")
                        .header("anthropic-beta", "existing-beta")
                        .json(&json!({"model":request_alias,"max_tokens":128,"stream":streaming,"messages":[{"role":"user","content":"hello"}]}))
                        .send().await.unwrap();
                    assert_eq!(response.status(), 200, "Desktop {protocol:?} streaming={streaming} 1M={one_m}");
                    let text = response.text().await.unwrap();
                    if streaming {
                        let message = text.lines().filter_map(|line| line.strip_prefix("data: ").and_then(|data| serde_json::from_str::<Value>(data).ok()))
                            .find(|value| value["type"] == "message_start").unwrap();
                        assert_eq!(message["message"]["model"], "official-model", "the aggregate public identity must replace the upstream wire model");
                        assert!(text.contains("message_stop"));
                    } else {
                        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["model"], "official-model");
                    }
                    let captured = upstream.requests.lock().unwrap().last().unwrap().clone();
                    assert_one_m_wire(&captured, protocol, "platform/real-wire-fast", one_m);
                }
                assert_eq!(upstream.count.load(Ordering::SeqCst), 2);
                let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
                assert_eq!(ledger.recent.len(), 2);
                assert!(ledger.recent.iter().all(|record| record.identity.client_source.as_deref() == Some("claudeDesktop")
                    && record.identity.requested_model.as_deref() == Some(alias.as_str())
                    && record.identity.resolved_model.as_deref() == Some("platform/real-wire-fast")
                    && record.identity.route_revision == Some(7)
                    && record.model_observation.served_model.as_deref() == Some("platform/real-wire-fast")));
                assert_eq!(client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("desktop-fixture")
                    .json(&json!({"model":"official-model","messages":[]})).send().await.unwrap().status(), 401);
                store.providers[0].models[0].overrides.context_window = Some(128_000);
                persist(&paths, &store);
                let no_longer_large = client.post(format!("{base}/claude-desktop/v1/messages")).bearer_auth("desktop-fixture")
                    .json(&json!({"model":format!("{alias}[1m]"),"max_tokens":128,"messages":[{"role":"user","content":"capacity changed"}]}))
                    .send().await.unwrap();
                assert_eq!(no_longer_large.status(), 400);
                no_longer_large.bytes().await.unwrap();
                assert_eq!(upstream.count.load(Ordering::SeqCst), 2);
                wait_for_released_lease().await;
                assert_aggregate_accounting(&paths, &store, 2);
            }
        }
    });
}

#[test]
fn desktop_headerless_signed_history_and_new_conversations_stay_on_the_hit_member() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let sequence = Arc::new(AtomicUsize::new(0));
        let a = scripted_mock(move |_| scripted_response(200, json!({
            "id":"msg_a","type":"message","model":"wire-a","role":"assistant",
            "content":[{"type":"thinking","thinking":"private a","signature":"signature-a"},
                {"type":"text","text":format!("hello from a {}", sequence.fetch_add(1, Ordering::SeqCst))}],
            "stop_reason":"end_turn","usage":{"input_tokens":4,"output_tokens":2}
        }).to_string(), "application/json")).await;
        let b = scripted_mock(|_| scripted_response(200, json!({
            "id":"msg_b","type":"message","model":"wire-b","role":"assistant",
            "content":[{"type":"thinking","thinking":"private b","signature":"signature-b"},
                {"type":"text","text":"hello from b"}],
            "stop_reason":"end_turn","usage":{"input_tokens":4,"output_tokens":2}
        }).to_string(), "application/json")).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let mut store = fixture(&[(&a, "wire-a", router::ModelProtocol::AnthropicMessages),
            (&b, "wire-b", router::ModelProtocol::AnthropicMessages)]);
        let (client, base) = start(&paths, &store).await;
        let alias = desktop_aggregate_state(&paths, running_port().unwrap(), &store.aggregates[0]);
        let send = |messages: Value| client.post(format!("{base}/claude-desktop/v1/messages"))
            .bearer_auth("desktop-fixture").json(&json!({"model":alias,"max_tokens":128,"messages":messages})).send();
        let first = json!({"role":"user","content":"hello"});
        let response_a: Value = send(json!([first])).await.unwrap().json().await.unwrap();
        let response_b: Value = send(json!([first])).await.unwrap().json().await.unwrap();
        assert_eq!(response_a["content"][1]["text"], "hello from a 0");
        assert_eq!(response_b["content"][1]["text"], "hello from a 1",
            "new conversations must reuse the preferred channel too");
        let assistant_a = json!({"role":"assistant","content":response_a["content"]});
        let assistant_b = json!({"role":"assistant","content":response_b["content"]});
        for assistant in [&assistant_a, &assistant_b] {
            let response = send(json!([{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]},
                assistant,{"role":"user","content":"again"}])).await.unwrap();
            let status = response.status();
            let answer: Value = response.json().await.unwrap();
            assert_eq!(status, 200, "{answer}");
            assert_eq!(answer["content"][0]["signature"], "signature-a");
        }
        assert_eq!(a.count.load(Ordering::SeqCst), 4);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        let request = a.requests.lock().unwrap()[2]["body"].clone();
        assert_eq!(request["messages"][1]["content"], response_a["content"],
            "same-wire signed thinking must remain intact");
        let foreign_client = client.post(format!("{base}/clients/claude-code/aggregate/v1/messages"))
            .bearer_auth("local-fixture").json(&json!({"model":"official-model","max_tokens":128,
                "messages":[first,assistant_a,{"role":"user","content":"same history, different client"}]}))
            .send().await.unwrap();
        assert_eq!(foreign_client.status(), 409, "Desktop ownership must not cross client/config scopes");
        store.providers[0].api_key = "rotated-credential".into();
        persist(&paths, &store);
        let rejected = send(json!([first, assistant_a, {"role":"user","content":"changed credentials"}])).await.unwrap();
        assert_eq!(rejected.status(), 409);
        assert_eq!(a.count.load(Ordering::SeqCst), 4);
        assert_eq!(b.count.load(Ordering::SeqCst), 0,
            "an unavailable owner must never be replaced for signed history");
        assert_aggregate_accounting_counts(&paths, &store, &[4, 0]);
    });
}

fn private_tool_reply(protocol: router::ModelProtocol, streaming: bool) -> Response<ResponseBody> {
    let result = match protocol {
        router::ModelProtocol::AnthropicMessages => json!({
            "id":"msg_private","type":"message","role":"assistant","model":"wire-tool",
            "content":[{"type":"thinking","thinking":"private plan","signature":"original-signature"},
                {"type":"tool_use","id":"call-next","name":"lookup","input":{"query":"next"}}],
            "stop_reason":"tool_use","usage":{"input_tokens":4,"output_tokens":2}
        }),
        router::ModelProtocol::ChatCompletions => json!({
            "id":"chat_private","object":"chat.completion","model":"wire-tool",
            "choices":[{"index":0,"message":{"role":"assistant","content":null,"reasoning_content":"private plan",
                "tool_calls":[{"id":"call-next","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"next\"}"}}]},
                "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4,"completion_tokens":2}
        }),
        router::ModelProtocol::Responses => json!({
            "id":"resp_private","object":"response","model":"wire-tool","status":"completed",
            "output":[{"type":"reasoning","id":"rs-exact","encrypted_content":"opaque-original","summary":[]},
                {"type":"function_call","id":"fc-exact","call_id":"call-next","name":"lookup","arguments":"{\"query\":\"next\"}","status":"completed"}],
            "usage":{"input_tokens":4,"output_tokens":2}
        }),
    };
    if !streaming {
        return scripted_response(200, result.to_string(), "application/json");
    }
    let events = match protocol {
        router::ModelProtocol::Responses => vec![
            json!({"type":"response.created","response":{"id":"resp_private","model":"wire-tool","status":"in_progress","output":[]}}),
            json!({"type":"response.completed","response":result}),
        ],
        router::ModelProtocol::ChatCompletions => {
            let mut delta = result["choices"][0]["message"].clone();
            delta["tool_calls"][0]["index"] = json!(0);
            vec![
                json!({"id":"chat_private","object":"chat.completion.chunk","model":"wire-tool","choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
                json!({"id":"chat_private","object":"chat.completion.chunk","model":"wire-tool","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":result["usage"]}),
            ]
        }
        router::ModelProtocol::AnthropicMessages => vec![
            json!({"type":"message_start","message":{"id":"msg_private","type":"message","role":"assistant","model":"wire-tool","content":[],"usage":{"input_tokens":4,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"original-"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-next","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"next\"}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":2}}),
            json!({"type":"message_stop"}),
        ],
    };
    let mut output: String = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    if protocol == router::ModelProtocol::ChatCompletions {
        output.push_str("data: [DONE]\n\n");
    }
    // Split every SSE response into tiny frames, including within JSON strings.
    let frames = output
        .into_bytes()
        .chunks(7)
        .map(|chunk| {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(Bytes::copy_from_slice(
                chunk,
            )))
        })
        .collect::<Vec<_>>();
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, "text/event-stream")
        .body(BodyExt::boxed(StreamBody::new(futures_util::stream::iter(
            frames,
        ))))
        .unwrap()
}

#[test]
fn desktop_headerless_tools_and_reasoning_history_stay_pinned_across_json_and_split_sse() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for protocol in [router::ModelProtocol::AnthropicMessages, router::ModelProtocol::ChatCompletions, router::ModelProtocol::Responses] {
            for streaming in [false, true] {
                let _stop = Stop;
                let a = scripted_mock(move |_| private_tool_reply(protocol, streaming)).await;
                let b = mock(protocol, 200, false).await;
                let temp = tempfile::tempdir().unwrap();
                let paths = CodexPaths::from_home(temp.path().into());
                let store = fixture(&[(&a, "wire-tool", protocol), (&b, "wire-other", protocol)]);
                let (client, base) = start(&paths, &store).await;
                let alias = desktop_aggregate_state(&paths, running_port().unwrap(), &store.aggregates[0]);
                let tools = json!([{"name":"lookup","input_schema":{"type":"object","properties":{"query":{"type":"string"}}}}]);
                let first_user = json!({"role":"user","content":"lookup"});
                let send = |messages: Value, stream: bool| client.post(format!("{base}/claude-desktop/v1/messages"))
                    .bearer_auth("desktop-fixture").json(&json!({"model":alias,"max_tokens":128,"messages":messages,"tools":tools,"stream":stream})).send();
                let first = send(json!([first_user]), streaming).await.unwrap();
                let status = first.status();
                let bytes = first.bytes().await.unwrap();
                assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
                let assistant = if streaming {
                    let mut reply = super::aggregate_session::Reply::default();
                    for line in std::str::from_utf8(&bytes).unwrap().lines() {
                        if let Some(value) = line.strip_prefix("data: ").and_then(|v| serde_json::from_str::<Value>(v).ok()) {
                            assert_ne!(value["type"], "error", "{protocol:?} streaming={streaming}: {value}");
                            reply.event(&value);
                        }
                    }
                    reply.assistant().expect("complete streamed assistant")
                } else {
                    let response: Value = serde_json::from_slice(&bytes).unwrap();
                    json!({"role":"assistant","content":response["content"]})
                };
                assert!(assistant["content"].as_array().unwrap().iter().any(|b| b["type"] == "tool_use" && b["id"] == "call-next"));
                let next = send(json!([first_user,assistant,
                    {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-next","content":"result"}]}]), streaming).await.unwrap();
                let status = next.status();
                let bytes = next.bytes().await.unwrap();
                assert_eq!(status, 200, "{protocol:?} stream={streaming}: {}", String::from_utf8_lossy(&bytes));
                assert_eq!(a.count.load(Ordering::SeqCst), 2);
                assert_eq!(b.count.load(Ordering::SeqCst), 0, "a tool continuation must not rotate");
                let body = a.requests.lock().unwrap()[1]["body"].clone();
                match protocol {
                    router::ModelProtocol::AnthropicMessages => {
                        assert_eq!(body["messages"][1]["content"][0]["signature"], "original-signature");
                        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call-next");
                    }
                    router::ModelProtocol::ChatCompletions => {
                        let assistant = body["messages"].as_array().unwrap().iter().find(|m| m["role"] == "assistant").unwrap();
                        assert_eq!(assistant["reasoning_content"], "private plan", "the exact selected member's retained reasoning must be replayed");
                    }
                    router::ModelProtocol::Responses => {
                        let input = body["input"].as_array().unwrap();
                        assert!(input.iter().any(|v| v["id"] == "rs-exact" && v["encrypted_content"] == "opaque-original"));
                        assert!(input.iter().any(|v| v["id"] == "fc-exact" && v["call_id"] == "call-next"));
                        assert!(input.iter().any(|v| v["type"] == "function_call_output" && v["call_id"] == "call-next"));
                    }
                }
                let traces = route_trace::recent();
                assert!(traces.iter().filter(|trace| trace.client_source.as_deref() == Some("claudeDesktop"))
                    .any(|trace| trace.attempts.iter().any(|attempt| attempt.selection_reason == "aggregate_session_binding"
                        && attempt.member_id.as_deref() == Some("member0"))));
                wait_for_released_lease().await;
                let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
                assert_eq!(ledger.all_time.request_count, 2);
                assert!(ledger.recent.iter().all(|record| record.provider_id == "p0"));
            }
        }
    });
}

#[test]
fn body_sessions_keep_same_prompts_separate_and_unknown_headerless_history_is_rejected() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let b = mock(router::ModelProtocol::ChatCompletions, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-a", router::ModelProtocol::ChatCompletions), (&b, "wire-b", router::ModelProtocol::ChatCompletions)]);
        let (client, base) = start(&paths, &store).await;
        let send = |session: Option<&str>, messages: Value| client.post(format!("{base}/clients/claude-code/aggregate/v1/messages"))
            .bearer_auth("local-fixture").json(&json!({"model":"official-model","max_tokens":128,"messages":messages,
                "metadata":{"user_id":session.map(|id| json!({"device_id":"shared","session_id":id}).to_string())}})).send();
        for session in ["body-a", "body-b"] {
            assert_eq!(send(Some(session), json!([{"role":"user","content":"hello"}])).await.unwrap().status(), 200);
        }
        for session in ["body-a", "body-b"] {
            let response = send(Some(session), json!([{"role":"user","content":"hello"},
                {"role":"assistant","content":[{"type":"thinking","thinking":"client plan","signature":"client-signature"},{"type":"text","text":"hello"}]},
                {"role":"user","content":"again"}])).await.unwrap();
            assert_eq!(response.status(), 200, "a pinned Messages adapter may reconstruct its original egress history");
        }
        assert_eq!(a.count.load(Ordering::SeqCst), 4);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        let unknown = send(None, json!([{"role":"user","content":"unseen"},
            {"role":"assistant","content":[{"type":"tool_use","id":"foreign-call","name":"lookup","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"foreign-call","content":"result"}]}])).await.unwrap();
        assert_eq!(unknown.status(), 409, "unknown tool ownership must not silently rotate");
        assert_eq!(a.count.load(Ordering::SeqCst) + b.count.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn identical_headerless_delivered_histories_become_ambiguous_and_never_dispatch() {
    let _guard = global_router_test_lock();
    let _stop = Stop;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let a = mock(router::ModelProtocol::AnthropicMessages, 200, false).await;
        let b = mock(router::ModelProtocol::AnthropicMessages, 200, false).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().into());
        let store = fixture(&[(&a, "wire-a", router::ModelProtocol::AnthropicMessages),
            (&b, "wire-b", router::ModelProtocol::AnthropicMessages)]);
        let (client, base) = start(&paths, &store).await;
        for _ in 0..2 {
            let response = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
                .json(&json!({"model":"official-model","max_tokens":128,"messages":[{"role":"user","content":"hello"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            response.bytes().await.unwrap();
        }
        let known: std::collections::HashSet<_> = route_trace::recent().into_iter().map(|trace| trace.id).collect();
        let ambiguous = client.post(format!("{base}/aggregate/v1/messages")).bearer_auth("local-fixture")
            .json(&json!({"model":"official-model","max_tokens":128,"messages":[{"role":"user","content":"hello"},
                {"role":"assistant","content":"hello"},{"role":"user","content":"continue"}]})).send().await.unwrap();
        assert_eq!(ambiguous.status(), 409);
        assert!(ambiguous.text().await.unwrap().contains("ambiguous"));
        assert_eq!(a.count.load(Ordering::SeqCst), 2);
        assert_eq!(b.count.load(Ordering::SeqCst), 0);
        let traces = route_trace::recent();
        assert!(traces.iter().find(|trace| !known.contains(&trace.id)).unwrap().attempts.is_empty(),
            "the rejection must occur before an inference/accounting attempt");
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count, 2);
    });
}
