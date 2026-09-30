use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn mock(
    status: u16,
    body: &'static str,
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
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let captured = captured.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(move |request: Request<Incoming>| {
                            let captured = captured.clone();
                            let calls = calls.clone();
                            async move {
                                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                                captured
                                    .lock()
                                    .unwrap()
                                    .push(serde_json::from_slice(&bytes).unwrap());
                                calls.fetch_add(1, Ordering::SeqCst);
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .header(CONTENT_TYPE, content_type)
                                        .body(Full::new(Bytes::from_static(body.as_bytes())))
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

const RESPONSE: &str = r#"{"id":"response-test","object":"response","status":"completed","model":"gpt-6-astra","output":[],"usage":{"input_tokens":1,"output_tokens":1}}"#;
fn persist(paths: &CodexPaths, store: &router::Store) {
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    crate::core::secret_file::write(
        &paths.app_data_dir.join("newapi-providers.dat"),
        &serde_json::to_vec(store).unwrap(),
    )
    .unwrap();
}

#[test]
fn failover_is_opt_in_bounded_and_keeps_followups_on_the_actual_channel() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (url_a,a,_,server_a) = mock(429,r#"{"error":{"message":"rate limit; sk-sensitive"}}"#,"application/json").await;
        let (url_b,b,requests,server_b) = mock(200,RESPONSE,"application/json").await;
        let dir = tempfile::tempdir().unwrap(); let paths = CodexPaths::from_home(dir.path().into());
        let provider = |id: &str, base_url| router::Provider { id:id.into(),name:id.into(),base_url,model:"gpt-6-astra".into(),api_key:"sk-sensitive".into(),enabled:true,..Default::default() };
        let mut store = router::Store { providers:vec![provider("a",url_a),provider("b",url_b)],router:router::RouterSettings { bearer_token:"local-only".into(),..Default::default() },..Default::default() };
        persist(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload { mode:super::super::models::ApiProxyMode::Direct,url:None }).await.unwrap();
        let client = Client::builder().no_proxy().build().unwrap(); let url = format!("http://127.0.0.1:{port}/codex/v1/responses");
        let body = json!({"model":"a::gpt-6-astra","input":"PRIVATE BODY","reasoning":{"effort":"high"},"stream":false});
        let send = |session: &'static str, body: Value| client.post(&url).bearer_auth("local-only").header("session_id",session).json(&body).send();
        let response = send("disabled",body.clone()).await.unwrap(); assert_eq!(response.status(),429); response.bytes().await.unwrap();
        assert_eq!(b.load(Ordering::SeqCst),0);
        store.router.same_model_failover=true; persist(&paths,&store);
        let response = send("new-turn",body.clone()).await.unwrap(); assert_eq!(response.status(),200); response.bytes().await.unwrap();
        assert_eq!(a.load(Ordering::SeqCst),2); assert_eq!(b.load(Ordering::SeqCst),1);
        assert_eq!(requests.lock().unwrap()[0]["model"],"gpt-6-astra");
        assert_eq!(requests.lock().unwrap()[0]["reasoning"]["effort"],"high");
        let followup=json!({"model":"a::gpt-6-astra","previous_response_id":"response-test","input":[{"type":"function_call_output","call_id":"tool","output":"DONE ONCE"}]});
        let response=send("new-turn",followup).await.unwrap(); assert_eq!(response.status(),200);response.bytes().await.unwrap();
        assert_eq!(a.load(Ordering::SeqCst),2);assert_eq!(b.load(Ordering::SeqCst),2);
        let mut tools=body.clone();tools["tools"]=json!([{"type":"function","name":"execute"}]);
        let response=send("tool-first",tools).await.unwrap();assert_eq!(response.status(),429);response.bytes().await.unwrap();
        assert_eq!(b.load(Ordering::SeqCst),2);
        store.providers[1].enabled=false;persist(&paths,&store);
        let response=send("new-turn",body.clone()).await.unwrap();assert_eq!(response.status(),503);
        assert_eq!(a.load(Ordering::SeqCst),3);assert_eq!(b.load(Ordering::SeqCst),2);
        let traces=super::super::route_trace::recent();
        let route=traces.iter().find(|r| r.attempts.len()==2 && r.requested_model=="a::gpt-6-astra").unwrap();
        assert_eq!(route.attempts[0].status,Some(429));assert_eq!(route.attempts[1].provider_id,"b");
        assert_eq!(route.requested["reasoning.effort"],"high");assert_eq!(route.attempts[1].sent["reasoning.effort"],"high");
        let serialized=serde_json::to_string(route).unwrap();
        for secret in ["PRIVATE BODY","sk-sensitive","local-only","DONE ONCE"] { assert!(!serialized.contains(secret)); }
        stop();server_a.abort();server_b.abort();
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
                    same_model_failover: true,
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
