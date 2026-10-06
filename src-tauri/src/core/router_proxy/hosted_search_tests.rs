//! Real localhost requests exercise the Chat hosted-tool loop. The model and
//! search service deliberately use different endpoints and credentials.
use super::*;
use crate::core::router::ModelProtocol;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Reply {
    status: u16,
    body: Value,
    delay: Duration,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body,
            delay: Duration::ZERO,
        }
    }
}

struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Mock {
    async fn new(respond: impl Fn(usize, &Value) -> Reply + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let respond = Arc::new(respond);
        let calls = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let captured = captured.clone();
                let respond = respond.clone();
                let calls = calls.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let captured = captured.clone();
                        let respond = respond.clone();
                        let calls = calls.clone();
                        async move {
                            let path = request.uri().path().to_owned();
                            let authorization = request
                                .headers()
                                .get(AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or_default()
                                .to_owned();
                            let body: Value = serde_json::from_slice(
                                &request.into_body().collect().await.unwrap().to_bytes(),
                            )
                            .unwrap();
                            captured.lock().unwrap().push(
                                json!({"path":path, "authorization":authorization, "body":body}),
                            );
                            let reply = respond(calls.fetch_add(1, Ordering::SeqCst), &body);
                            if !reply.delay.is_zero() {
                                tokio::time::sleep(reply.delay).await;
                            }
                            Ok::<_, std::convert::Infallible>(
                                Response::builder()
                                    .status(reply.status)
                                    .header(CONTENT_TYPE, "application/json")
                                    .body(Full::new(Bytes::from(reply.body.to_string())))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn captured(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

fn request(streaming: bool) -> Value {
    json!({"model":"fixture-chat", "max_tokens":256, "stream":streaming,
        "tools":[{"type":"web_search_20250305", "name":"web_search", "max_uses":2}],
        "tool_choice":{"type":"tool", "name":"web_search"},
        "messages":[{"role":"user", "content":"Find the current Rust release."}]})
}

fn chat_reply(message: Value, finish: &str) -> Value {
    json!({"id":"chat-fixture", "object":"chat.completion", "model":"fixture-chat",
        "choices":[{"index":0, "message":message, "finish_reason":finish}],
        "usage":{"prompt_tokens":3, "completion_tokens":4, "total_tokens":7}})
}

fn search_call(body: &Value, id: &str, query: &str) -> Value {
    let name = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|tool| {
            tool["function"]["name"]
                .as_str()
                .filter(|name| name.starts_with("__agentharbor_web_search"))
        })
        .unwrap();
    json!({"id":id, "type":"function", "function":{"name":name, "arguments":json!({"query":query}).to_string()}})
}

fn search_reply() -> Value {
    json!({"results":[
        {"title":"Rust release", "url":"https://www.rust-lang.org/releases", "snippet":"Rust 1.99 is the fixture release."},
        {"title":"bad", "url":"javascript:alert(1)", "snippet":"should be discarded"},
        {"title":"duplicate", "url":"https://www.rust-lang.org/releases", "snippet":"duplicate"}
    ]})
}

fn bearer(value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static(value));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn execution(
    inbound: &Value,
    model: &Mock,
    search: &Mock,
    app_data: &Path,
    history: Option<protocol::message_history::Context>,
) -> hosted_search::Execution {
    let plan = hosted_search::Plan::prepare(inbound.clone()).unwrap();
    let mut body = protocol::request_body(
        protocol::Translation {
            from: ModelProtocol::AnthropicMessages,
            to: ModelProtocol::ChatCompletions,
        },
        &serde_json::from_slice::<Value>(&plan.normalized).unwrap(),
    )
    .unwrap()
    .body;
    if let Some(history) = &history {
        history.restore(&mut body);
    }
    let trace = route_trace::Trace::begin("fixture-chat", Some(inbound.to_string().as_bytes()));
    let attempt = route_trace::AttemptMetadata {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    trace.attempt(
        "model-card",
        "Fixture model",
        "fixture-chat",
        "chatCompletions",
        attempt.clone(),
    );
    let credential = request_diagnostics::CredentialSnapshot::from_header(
        app_data,
        "providerApiKey",
        "fixture-model-scope",
        Some("Bearer model-only"),
        None,
    );
    trace.sent(Some(body.to_string().as_bytes()), credential.clone());
    let usage = relay_usage::RelayUsageRequest::new(
        app_data.to_owned(),
        "model-card".into(),
        "Fixture model".into(),
        "fixture-chat".into(),
        "chatCompletions".into(),
        relay_usage::PricingSnapshot {
            input: Some(1.0),
            output: Some(2.0),
            complete: true,
            currency: "USD".into(),
            source: "manual".into(),
            ..Default::default()
        },
    )
    .with_request_id(trace.request_id())
    .with_attempt_id(&attempt.attempt_id)
    .with_credential(credential.clone());
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    hosted_search::Execution {
        plan,
        backend: hosted_search::SearchBackend {
            client: client.clone(),
            endpoint: format!("{}/alpha/web-search", search.url),
            headers: bearer("Bearer search-only"),
        },
        client,
        url: format!("{}/v1/chat/completions", model.url),
        headers: bearer("Bearer model-only"),
        body: Bytes::from(body.to_string()),
        trace,
        usage: Some(usage),
        attempt,
        provider_id: "model-card".into(),
        provider_name: "Fixture model".into(),
        model: "fixture-chat".into(),
        history,
        adapter: tool_schema::Adapter::default(),
        credential,
    }
}

async fn json_response(execution: hosted_search::Execution) -> Value {
    let response = hosted_search::relay(execution).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[test]
fn hosted_chat_plan_is_protocol_based_and_preserves_native_search_paths() {
    let inbound = request(false).to_string();
    let destination = |target| Destination::Provider {
        provider: Box::new(router::Provider {
            base_url: "https://unrelated-model-provider.example/v1".into(),
            ..Default::default()
        }),
        upstream_protocol: target,
        model_override: None,
        translation: protocol::Translation::between(ModelProtocol::AnthropicMessages, target),
    };
    assert!(hosted_search::Plan::for_destination(
        &destination(ModelProtocol::ChatCompletions),
        Some(inbound.as_bytes())
    )
    .unwrap()
    .is_some());
    for protocol in [ModelProtocol::AnthropicMessages, ModelProtocol::Responses] {
        assert!(hosted_search::Plan::for_destination(
            &destination(protocol),
            Some(inbound.as_bytes())
        )
        .unwrap()
        .is_none());
    }
    let plain = json!({"model":"fixture-chat", "max_tokens":20, "messages":[{"role":"user","content":"hello"}]}).to_string();
    assert!(hosted_search::Plan::for_destination(
        &destination(ModelProtocol::ChatCompletions),
        Some(plain.as_bytes())
    )
    .unwrap()
    .is_none());
}

#[test]
fn hosted_chat_executes_search_with_separate_credentials_and_accounts_each_model_round() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant", "content":null,
                "reasoning_content":"private search planning", "tool_calls":[search_call(body,"search-1","Rust release")]}),"tool_calls"))
        } else {
            Reply::ok(chat_reply(json!({"role":"assistant", "content":"Rust 1.99: https://www.rust-lang.org/releases",
                "reasoning_content":"private final reasoning"}),"stop"))
        }).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let visible = json_response(execution(&request(false), &model, &search, temp.path(), None)).await;
        assert_eq!(visible["stop_reason"], "end_turn");
        let content = visible["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "server_tool_use");
        assert_eq!(content[0]["name"], "web_search");
        assert_eq!(content[1]["type"], "web_search_tool_result");
        assert_eq!(content[1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(content[1]["content"][0]["url"], "https://www.rust-lang.org/releases");
        assert_eq!(content[1]["content"][0]["title"], "Rust release");
        assert!(content.iter().any(|block| block["text"].as_str().is_some_and(|text| text.contains("Rust 1.99"))));
        assert!(!visible.to_string().contains("private search planning"));
        let calls = model.captured();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call["authorization"] == "Bearer model-only"));
        assert!(calls.iter().all(|call| call["path"] == "/v1/chat/completions"));
        assert_eq!(calls[0]["body"]["stream"], false);
        assert_eq!(calls[0]["body"]["max_tokens"], 256);
        assert_eq!(calls[1]["body"]["max_tokens"], 252, "Internal continuations share the caller's output budget");
        assert_eq!(calls[1]["body"]["tool_choice"], "auto");
        let tool = calls[1]["body"]["messages"].as_array().unwrap().iter().find(|m|m["role"] == "tool").unwrap();
        let result:Value = serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
        assert_eq!(result["results"][0]["snippet"], "Rust 1.99 is the fixture release.");
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(calls[1]["body"]["messages"][1]["reasoning_content"], "private search planning");
        let searches = search.captured();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0]["authorization"], "Bearer search-only");
        assert_eq!(searches[0]["path"], "/alpha/web-search");
        assert_eq!(searches[0]["body"], json!({"query":"Rust release","numResults":5}));
        let ledger = relay_usage::load(temp.path()).unwrap();
        assert_eq!(ledger.recent.len(), 2, "Only model calls contribute token usage; REST search is not a token request");
        assert!(ledger.recent.iter().all(|record| record.provider_id == "model-card" && record.tokens.total_tokens == 7));
        assert_eq!(ledger.recent[0].request_id, ledger.recent[1].request_id);
        assert_ne!(ledger.recent[0].attempt_id, ledger.recent[1].attempt_id);
        assert!(ledger.recent.iter().all(|record| record.cost_usd == Some(0.000011)));
        assert_eq!(visible["usage"]["input_tokens"], 6);
        assert_eq!(visible["usage"]["output_tokens"], 8);
    });
}

#[test]
fn hosted_chat_max_uses_limits_each_request_and_does_not_force_search_forever() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round % 2 == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant", "content":null,
                "tool_calls":[search_call(body,"search-1","first"),search_call(body,"search-2","second")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Budget respected"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let mut inbound = request(false); inbound["tools"][0]["max_uses"] = json!(1);
        for _ in 0..2 {
            let visible = json_response(execution(&inbound, &model, &search, temp.path(), None)).await;
            let error = visible["content"].as_array().unwrap().iter()
                .find(|item| item["type"] == "web_search_tool_result" && item["tool_use_id"] == "search-2").unwrap();
            assert_eq!(error["content"]["error_code"], "max_uses_exceeded");
        }
        assert_eq!(search.captured().len(), 2, "The second API request gets a fresh max_uses budget");
        let calls = model.captured();
        assert_eq!(calls.len(), 4);
        assert!(calls[1]["body"]["tool_choice"] != json!({"type":"function","function":{"name":"__agentharbor_web_search"}}));
        assert!(calls[1]["body"]["messages"].to_string().contains("max_uses_exceeded"));
    });
}

#[test]
fn hosted_chat_mixed_client_tools_return_without_an_orphan_followup_request() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|_, body| Reply::ok(chat_reply(json!({"role":"assistant", "content":null,
            "tool_calls":[search_call(body,"search-1","Rust"),
                {"id":"file-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}),"tool_calls"))).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let mut inbound = request(false);
        inbound["tools"].as_array_mut().unwrap().push(json!({"name":"read_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}));
        let visible = json_response(execution(&inbound, &model, &search, temp.path(), None)).await;
        assert_eq!(model.captured().len(), 1);
        assert_eq!(search.captured().len(), 1);
        assert_eq!(visible["stop_reason"], "tool_use");
        assert!(visible["content"].as_array().unwrap().iter().any(|block|
            block["type"] == "tool_use" && block["id"] == "file-1" && block["name"] == "read_file"));
        assert!(visible["content"].as_array().unwrap().iter().any(|block| block["type"] == "web_search_tool_result"));
    });
}

#[test]
fn hosted_chat_search_failure_is_a_tool_error_without_leaking_service_response() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant","content":null,"tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Search unavailable"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply { status:401, body:json!({"error":"private account response"}), delay:Duration::ZERO }).await;
        let temp = tempfile::tempdir().unwrap();
        let visible = json_response(execution(&request(false), &model, &search, temp.path(), None)).await;
        assert_eq!(visible["content"][1]["content"]["error_code"], "unavailable");
        assert!(!visible.to_string().contains("private account response"));
        assert!(model.captured()[1]["body"]["messages"].to_string().contains("unavailable"));
        assert_eq!(relay_usage::load(temp.path()).unwrap().recent.len(), 2);
    });
}

#[test]
fn hosted_chat_stream_emits_messages_hosted_blocks_and_balanced_terminal_events() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant","content":null,"tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Rust answer"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let response = hosted_search::relay(execution(&request(true), &model, &search, temp.path(), None)).await.unwrap();
        assert_eq!(response.headers()[CONTENT_TYPE], "text/event-stream");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let stream = std::str::from_utf8(&bytes).unwrap();
        assert!(stream.starts_with("event: ping\n"));
        assert_eq!(stream.matches("event: message_start\n").count(),1);
        assert_eq!(stream.matches("event: message_stop\n").count(),1);
        assert_eq!(stream.matches("event: content_block_start\n").count(),stream.matches("event: content_block_stop\n").count());
        let content = claude_continuation_tests::streamed_content(stream);
        assert_eq!(content[0]["type"], "server_tool_use");
        assert_eq!(content[1]["type"], "web_search_tool_result");
        assert_eq!(content[1]["content"][0]["url"], "https://www.rust-lang.org/releases");
        assert!(content.as_array().unwrap().iter().any(|block|block["text"] == "Rust answer"));
        assert_eq!(model.captured().len(),2);
    });
}

#[test]
fn hosted_chat_continuation_replays_exact_private_turns_and_cache_miss_retains_sources() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant","content":null,"reasoning_content":"private search planning",
                "tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Rust answer","reasoning_content":"private final reasoning"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let scope = uuid::Uuid::new_v4().to_string();
        let inbound = request(false);
        let context = protocol::message_history::Context::new(&[&scope], ModelProtocol::ChatCompletions, inbound.to_string().as_bytes()).unwrap();
        let visible = json_response(execution(&inbound,&model,&search,temp.path(),Some(context))).await;
        let mut next = inbound.clone();
        next["tool_choice"] = json!({"type":"auto"});
        next["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":visible["content"]}),json!({"role":"user","content":"More details"})]);
        let context = protocol::message_history::Context::new(&[&scope], ModelProtocol::ChatCompletions, next.to_string().as_bytes()).unwrap();
        json_response(execution(&next,&model,&search,temp.path(),Some(context))).await;
        let captured = model.captured();
        let messages = captured[2]["body"]["messages"].as_array().unwrap();
        assert_eq!(messages[1],captured[1]["body"]["messages"][1]);
        assert_eq!(messages[2],captured[1]["body"]["messages"][2]);
        assert_eq!(messages[3]["reasoning_content"],"private final reasoning");
        assert_eq!(messages[4]["content"],"More details");
        let different_scope = uuid::Uuid::new_v4().to_string();
        let context = protocol::message_history::Context::new(&[&different_scope], ModelProtocol::ChatCompletions, next.to_string().as_bytes()).unwrap();
        json_response(execution(&next,&model,&search,temp.path(),Some(context))).await;
        let captured = model.captured();
        let miss = &captured[3]["body"]["messages"];
        assert!(!miss.to_string().contains("private search planning"));
        assert!(!miss.to_string().contains("private final reasoning"));
        assert!(miss.to_string().contains("https://www.rust-lang.org/releases"));
        assert_eq!(search.captured().len(),1,"A continuation must not execute past hosted searches again");
    });
}

#[test]
fn hosted_chat_domain_constraints_are_sent_to_search_and_enforced_on_results() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant","content":null,"tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Rust answer"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply::ok(json!({"results":[
            {"url":"https://rust-lang.org/", "title":"Root", "snippet":"root result"},
            {"url":"https://www.rust-lang.org/", "title":"Subdomain", "snippet":"subdomain result"},
            {"url":"https://rust-lang.org.evil.example/", "title":"Suffix attack"},
            {"url":"https://notrust-lang.org/", "title":"Prefix attack"},
            {"url":"https://rust-lang.org@evil.example/", "title":"Userinfo attack"},
            {"url":"javascript:alert(1)", "title":"Wrong scheme"}
        ]}))).await;
        let temp = tempfile::tempdir().unwrap();
        let mut inbound = request(false);
        inbound["tools"][0]["allowed_domains"] = json!(["rust-lang.org"]);
        let visible = json_response(execution(&inbound,&model,&search,temp.path(),None)).await;
        assert_eq!(search.captured()[0]["body"]["allowedDomains"],json!(["rust-lang.org"]));
        let results = visible["content"][1]["content"].as_array().unwrap();
        assert_eq!(results.len(),2);
        assert_eq!(results[0]["url"],"https://rust-lang.org/");
        assert_eq!(results[1]["url"],"https://www.rust-lang.org/");
        let calls = model.captured();
        let tool = calls[1]["body"]["messages"].as_array().unwrap().iter().find(|m|m["role"] == "tool").unwrap();
        assert!(!tool.to_string().contains("evil.example"));
        assert!(!tool.to_string().contains("notrust-lang.org"));
        assert!(!tool.to_string().contains("javascript:"));
    });
}

#[test]
fn hosted_chat_does_not_silently_drop_unsupported_search_options() {
    for (key, value) in [
        (
            "user_location",
            json!({"type":"approximate","country":"US"}),
        ),
        ("blocked_domains", json!(["example.com"])),
        ("allowed_domains", json!(["https://example.com/path"])),
        ("allowed_domains", json!(["user@example.com"])),
    ] {
        let mut inbound = request(false);
        inbound["tools"][0][key] = value;
        assert!(
            hosted_search::Plan::prepare(inbound).is_err(),
            "Search option {key} must fail explicitly if it cannot be preserved"
        );
    }
}

#[test]
fn hosted_chat_dropping_stream_cancels_search_without_starting_another_model_round() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|_, body| Reply::ok(chat_reply(json!({"role":"assistant","content":null,
            "tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))).await;
        let search = Mock::new(|_, _| Reply { status:200, body:search_reply(), delay:Duration::from_millis(250) }).await;
        let temp = tempfile::tempdir().unwrap();
        let mut body = hosted_search::relay(execution(&request(true),&model,&search,temp.path(),None)).await.unwrap().into_body();
        let ping=body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert!(ping.starts_with(b"event: ping"));
        tokio::select! {
            frame=body.frame()=>panic!("Delayed search must still be active before cancellation: {frame:?}"),
            _=async {
                tokio::time::timeout(Duration::from_secs(2),async {
                    while search.captured().is_empty() { tokio::time::sleep(Duration::from_millis(5)).await; }
                }).await.unwrap();
            }=>{}
        }
        drop(body);
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(model.captured().len(),1,"A disconnected Desktop must not trigger a billable continuation");
        assert_eq!(search.captured().len(),1);
        assert_eq!(relay_usage::load(temp.path()).unwrap().recent.len(),1);
    });
}

#[test]
fn hosted_chat_upstream_errors_preserve_http_status_and_do_not_leak_private_bodies() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|_, _| Reply {
            status: 429,
            body: json!({"error":{"message":"private model account details"}}),
            delay: Duration::ZERO,
        })
        .await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let result = hosted_search::relay(execution(
            &request(false),
            &model,
            &search,
            temp.path(),
            None,
        ))
        .await;
        match result {
            Err((status, message)) => {
                assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
                assert!(!message.contains("private model account"));
            }
            Ok(_) => panic!("A failed model request must not become a successful Messages reply"),
        }
        let response = hosted_search::relay(execution(
            &request(true),
            &model,
            &search,
            temp.path(),
            None,
        ))
        .await
        .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("event: error\n"));
        assert!(!text.contains("event: message_stop\n"));
        assert!(!text.contains("private model account details"));
        assert!(search.captured().is_empty());
        let ledger = relay_usage::load(temp.path()).unwrap();
        assert_eq!(ledger.recent.len(), 2);
        assert!(ledger
            .recent
            .iter()
            .all(|row| row.status_code == 429 && !row.success && !row.usage_reported));
    });
}

#[test]
fn hosted_chat_malformed_model_messages_return_502_without_panicking() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for message in [json!("bad"), json!([]), json!(42), Value::Null] {
            let model = Mock::new(move |_, _| Reply::ok(chat_reply(message.clone(), "stop"))).await;
            let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
            let temp = tempfile::tempdir().unwrap();
            match hosted_search::relay(execution(
                &request(false),
                &model,
                &search,
                temp.path(),
                None,
            ))
            .await
            {
                Err((status, _)) => assert_eq!(status, StatusCode::BAD_GATEWAY),
                Ok(_) => panic!("Malformed upstream messages must be rejected"),
            }
            assert!(search.captured().is_empty());
            let ledger = relay_usage::load(temp.path()).unwrap();
            assert_eq!(ledger.recent.len(), 1);
            assert!(!ledger.recent[0].success);
        }
    });
}

#[test]
fn hosted_chat_search_can_be_disabled_after_a_search_reply_without_losing_history() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| if round == 0 {
            Reply::ok(chat_reply(json!({"role":"assistant","content":null,"reasoning_content":"private plan",
                "tool_calls":[search_call(body,"search-1","Rust")]}),"tool_calls"))
        } else { Reply::ok(chat_reply(json!({"role":"assistant","content":"Rust answer","reasoning_content":"private answer"}),"stop")) }).await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let inbound = request(false);
        let first = protocol::message_history::Context::new(&["disable-search"], ModelProtocol::ChatCompletions, inbound.to_string().as_bytes());
        let visible = json_response(execution(&inbound, &model, &search, temp.path(), first)).await;
        let mut next = inbound;
        next.as_object_mut().unwrap().remove("tools");
        next.as_object_mut().unwrap().remove("tool_choice");
        next["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":visible["content"]}),
            json!({"role":"user","content":"Continue without searching"}),
        ]);
        let destination = Destination::Provider { provider:Box::new(router::Provider::default()), model_override:None,
            upstream_protocol:ModelProtocol::ChatCompletions,
            translation:protocol::Translation::between(ModelProtocol::AnthropicMessages, ModelProtocol::ChatCompletions) };
        let bytes = Bytes::from(next.to_string());
        assert!(hosted_search::Plan::for_destination(&destination, Some(&bytes)).unwrap().is_none());
        let normalized = hosted_search::Plan::history_only(&destination, Some(bytes)).unwrap().unwrap();
        let body = upstream_body(&destination, Some(normalized)).unwrap().body.unwrap();
        let mut value:Value = serde_json::from_slice(&body).unwrap();
        let history = protocol::message_history::Context::new(&["disable-search"], ModelProtocol::ChatCompletions, next.to_string().as_bytes()).unwrap();
        history.restore(&mut value);
        assert!(value.get("tools").is_none());
        assert!(value["messages"].to_string().contains("private plan"));
        assert!(value["messages"].to_string().contains("Rust 1.99 is the fixture release"));
        assert_eq!(search.captured().len(), 1);
    });
}

#[test]
fn hosted_chat_search_reaches_the_executor_from_desktop_and_messages_ingresses() {
    use crate::core::claude_desktop::{
        gateway::RouterConfig,
        paths::DesktopPaths,
        transaction,
        types::{DesktopRouteChoice, OneMMode},
    };
    let _guard = global_router_test_lock();
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            *hosted_search::TEST_SEARCH_ENDPOINT.lock().unwrap() = None;
            set_desktop_pin(None).unwrap();
            stop_unchecked();
        }
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let model = Mock::new(|round, body| {
            if round % 2 == 0 {
                Reply::ok(chat_reply(
                    json!({"role":"assistant","content":null,
                "tool_calls":[search_call(body,"search-1","Rust")]}),
                    "tool_calls",
                ))
            } else {
                Reply::ok(chat_reply(
                    json!({"role":"assistant","content":"Rust answer"}),
                    "stop",
                ))
            }
        })
        .await;
        let search = Mock::new(|_, _| Reply::ok(search_reply())).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let store = router::Store {
            providers: vec![
                router::Provider {
                    id: "model-card".into(),
                    name: "Unrelated Chat vendor".into(),
                    api_key: "model-only".into(),
                    base_url: model.url.clone(),
                    model: "fixture-chat".into(),
                    protocol: ModelProtocol::ChatCompletions,
                    enabled: true,
                    network_mode: "direct".into(),
                    ..Default::default()
                },
                router::Provider {
                    id: "search-card".into(),
                    name: "Search account".into(),
                    api_key: "search-only".into(),
                    base_url: "https://api.commandcode.ai/provider/v1".into(),
                    protocol: ModelProtocol::AnthropicMessages,
                    network_mode: "direct".into(),
                    ..Default::default()
                },
            ],
            router: router::RouterSettings {
                bearer_token: "router-local".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        crate::core::secret_file::write(
            &paths.app_data_dir.join("newapi-providers.dat"),
            &serde_json::to_vec(&store).unwrap(),
        )
        .unwrap();
        *hosted_search::TEST_SEARCH_ENDPOINT.lock().unwrap() =
            Some(format!("{}/alpha/web-search", search.url));
        let _cleanup = Cleanup;
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default())
            .await
            .unwrap();
        let roots = DesktopPaths::from_roots(
            paths.app_data_dir.join("desktop-normal"),
            paths.app_data_dir.join("desktop-third-party"),
        )
        .unwrap();
        let mut state = transaction::next_state(
            &[],
            &[],
            &roots,
            1,
            "search-fixture",
            "",
            "",
            None,
            &[],
            transaction::AppliedMode::Router,
        )
        .unwrap();
        state.router = Some(RouterConfig {
            routes: vec![DesktopRouteChoice {
                target_ref: None,
                role: "haiku".into(),
                role_alias: "claude-haiku-4-5".into(),
                provider_id: "model-card".into(),
                model_id: "fixture-chat".into(),
                label_override: None,
                one_m_mode: OneMMode::Auto,
            }],
            route_views: vec![],
            file_projection: String::new(),
            route_revision: 1,
            gateway_token_version: 1,
            gateway_token: "desktop-local".into(),
            bound_port: port,
        });
        crate::core::secret_file::write(
            &paths.app_data_dir.join("claude-desktop/state.dat"),
            &serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for desktop in [false, true] {
            for streaming in [false, true] {
                let mut inbound = request(streaming);
                inbound["model"] = json!(if desktop {
                    "claude-haiku-4-5-20251001"
                } else {
                    "model-card::fixture-chat"
                });
                inbound["messages"][0]["content"] = json!(format!("Search {desktop}/{streaming}"));
                let response = client
                    .post(format!(
                        "http://127.0.0.1:{port}/{}",
                        if desktop {
                            "claude-desktop/v1/messages"
                        } else {
                            "v1/messages"
                        }
                    ))
                    .bearer_auth(if desktop {
                        "desktop-local"
                    } else {
                        "router-local"
                    })
                    .json(&inbound)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let text = response.text().await.unwrap();
                assert!(
                    text.contains("web_search_tool_result")
                        && text.contains("https://www.rust-lang.org/releases"),
                    "{text}"
                );
            }
        }
        assert_eq!(model.captured().len(), 8);
        assert_eq!(search.captured().len(), 4);
        assert!(model
            .captured()
            .iter()
            .all(|call| call["authorization"] == "Bearer model-only"));
        assert!(search
            .captured()
            .iter()
            .all(|call| call["authorization"] == "Bearer search-only"));
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.recent.len(), 8);
        assert_eq!(
            ledger
                .recent
                .iter()
                .filter(|row| row.identity.client_source.as_deref() == Some("claudeDesktop"))
                .count(),
            4
        );
        assert!(ledger
            .recent
            .iter()
            .all(|row| row.provider_id == "model-card" && row.model == "fixture-chat"));
    });
}
