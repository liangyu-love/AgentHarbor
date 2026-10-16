//! Codex on Chat relays that think: real localhost HTTP, no external upstream.
//! DeepSeek refuses a tool-enabled history whose assistant turns lack their
//! own `reasoning_content`, which Codex cannot send back through a relay; a
//! host answering in `reasoning` refuses the other field.
use super::*;
use crate::core::router::{ModelProtocol, Provider, RouterSettings, Store};
use std::convert::Infallible;

const PLAN: &str = "plan: read the file first";
const READ: &str = "the file says it is done";

/// The reasoning a fixture turn gives, by what the history holds so far.
fn turn(messages: &[Value]) -> (&'static str, Option<&'static str>, &'static str) {
    match messages
        .last()
        .map(|message| message["role"].as_str().unwrap_or_default())
    {
        Some("tool") => (READ, None, "done"),
        _ if messages.iter().any(|message| message["role"] == "tool") => {
            ("closing", None, "you're welcome")
        }
        _ => (PLAN, Some("call_1"), ""),
    }
}

/// The fixture's own rules, then its reply in the field it uses.
fn chat_reply(body: &Value) -> (StatusCode, String, &'static str) {
    let deepseek = body["model"] == "deepseek-v4";
    let field = if deepseek {
        "reasoning_content"
    } else {
        "reasoning"
    };
    let messages = body["messages"].as_array().unwrap();
    let assistants: Vec<_> = messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .collect();
    let refused = if deepseek {
        // Each assistant turn carries exactly the reasoning it came with.
        let expected = [PLAN, READ];
        body["tools"].is_array()
            && assistants.iter().enumerate().any(|(index, message)| {
                message["reasoning_content"].as_str() != expected.get(index).copied()
            })
    } else {
        messages
            .iter()
            .any(|message| message.get("reasoning_content").is_some())
    };
    if refused {
        let message = if deepseek {
            "The reasoning_content in the thinking mode must be passed back to the API."
        } else {
            "property 'reasoning_content' is unsupported"
        };
        return (
            StatusCode::BAD_REQUEST,
            json!({"error":{"message":message}}).to_string(),
            "application/json",
        );
    }
    let (reasoning, call, text) = turn(messages);
    let calls = call.map(|id| json!([{"index":0,"id":id,"type":"function","function":{"name":"read_file","arguments":"{}"}}]));
    if body["stream"] != true {
        let mut message = json!({"role":"assistant","content":if text.is_empty() {Value::Null} else {json!(text)}});
        message[field] = json!(reasoning);
        if let Some(calls) = calls {
            message["tool_calls"] = calls;
        }
        let finish = if call.is_some() { "tool_calls" } else { "stop" };
        let reply = json!({"model":body["model"],"choices":[{"index":0,"finish_reason":finish,"message":message}]});
        return (StatusCode::OK, reply.to_string(), "application/json");
    }
    let mut delta = json!({"role":"assistant"});
    delta[field] = json!(reasoning);
    let mut frames = vec![
        json!({"model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
    ];
    if let Some(calls) = calls {
        frames.push(
            json!({"choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":null}]}),
        );
        frames.push(json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}));
    } else {
        frames.push(json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}));
        frames.push(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}));
    }
    let mut stream: String = frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect();
    stream.push_str("data: [DONE]\n\n");
    (StatusCode::OK, stream, "text/event-stream")
}

/// The output items of a Responses reply, as Codex keeps them for its history.
async fn output(response: reqwest::Response, streamed: bool) -> Vec<Value> {
    let value: Value = if streamed {
        let text = response.text().await.unwrap();
        text.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find(|event| event["type"] == "response.completed")
            .map(|event| event["response"].clone())
            .unwrap_or_else(|| panic!("no completed response in {text}"))
    } else {
        response.json().await.unwrap()
    };
    value["output"].as_array().cloned().unwrap_or_default()
}

#[test]
fn codex_tool_loops_keep_deepseek_reasoning_and_never_send_it_where_refused() {
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let service = service_fn(|request: Request<Incoming>| async move {
                        let body: Value = serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
                        let (status, reply, kind) = chat_reply(&body);
                        Ok::<_, Infallible>(Response::builder().status(status).header(CONTENT_TYPE, kind).body(Full::new(Bytes::from(reply))).unwrap())
                    });
                    http1::Builder::new().serve_connection(TokioIo::new(socket), service).await.unwrap();
                });
            }
        });
        let provider = |id: &str, model: &str| Provider {
            id: id.into(), name: id.into(), base_url: format!("http://127.0.0.1:{upstream}"), api_key: "fixture-key".into(),
            model: model.into(), protocol: ModelProtocol::ChatCompletions, enabled: true, network_mode: "direct".into(), ..Default::default()
        };
        let store = Store {
            providers: vec![provider("deep", "deepseek-v4"), provider("groq", "gpt-oss-groq")],
            router: RouterSettings { enabled: true, bearer_token: "local-fixture".into(), ..Default::default() },
            ..Default::default()
        };
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &serde_json::to_vec(&store).unwrap()).unwrap();
        let port = ensure_running(0, paths, ApiProxyConfigPayload::default()).await.unwrap();
        struct Stop;
        impl Drop for Stop { fn drop(&mut self) { stop(); } }
        let _stop = Stop;
        let client = Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{port}/codex/v1/responses");
        for model in ["deep::deepseek-v4", "groq::gpt-oss-groq"] {
            for streamed in [false, true] {
                let mut input = vec![json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("fix it {}", uuid::Uuid::new_v4())}]})];
                let send = |input: Vec<Value>| {
                    let request = json!({"model":model,"stream":streamed,"store":false,"instructions":"rules","input":input,
                        "tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{}}}]});
                    client.post(&url).bearer_auth("local-fixture").json(&request).send()
                };
                // Plan and call; read the result; then a plain follow-up.
                for (round, next) in [(1, json!({"type":"function_call_output","call_id":"call_1","output":"done"})),
                    (2, json!({"type":"message","role":"user","content":[{"type":"input_text","text":"thanks"}]})), (3, Value::Null)] {
                    let response = send(input.clone()).await.unwrap();
                    let status = response.status();
                    assert!(status.is_success(), "{model} stream={streamed} round {round}: {status} {}", response.text().await.unwrap());
                    let items = output(response, streamed).await;
                    assert!(items.iter().any(|item| item["type"] == "reasoning"), "{model} round {round}: Codex sees the reasoning");
                    // Codex replays every item it was given, reasoning included.
                    input.extend(items);
                    if !next.is_null() {
                        input.push(next);
                    }
                }
            }
        }
        server.abort();
    });
}
