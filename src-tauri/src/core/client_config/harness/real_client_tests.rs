//! Real DeepSeek Harness, production router, loopback upstream: each kind of
//! route streams and completes a tool call round trip.
use super::*;
use crate::{
    core::{
        aggregation,
        model_targets::{ClientModelSelection, ModelTargetRef},
        models::ApiProxyConfigPayload,
        newapi, route_trace, router_proxy,
    },
    platform::paths::CodexPaths,
};
use std::{
    process::Command,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    authorization: Option<String>,
    api_key: Option<String>,
    model: String,
    tool_result: bool,
}

/// One response per connection: a tool call until a tool result comes back,
/// then text naming the model, streamed in the upstream's own protocol.
async fn upstream(seen: Arc<StdMutex<Vec<Seen>>>) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let (head, body) = loop {
                    let mut chunk = [0u8; 8192];
                    let count = socket.read(&mut chunk).await.unwrap_or(0);
                    if count == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..count]);
                    if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buffer[..end]).to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if buffer.len() >= end + 4 + length {
                            break (head, buffer[end + 4..end + 4 + length].to_vec());
                        }
                    }
                };
                let header = |name: &str| {
                    head.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.trim()
                            .eq_ignore_ascii_case(name)
                            .then(|| value.trim().to_owned())
                    })
                };
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let model = payload["model"].as_str().unwrap_or("").to_owned();
                let history = if path.ends_with("/responses") {
                    payload["input"].to_string()
                } else {
                    payload["messages"].to_string()
                };
                let tool_result = history.contains("function_call_output")
                    || history.contains("\"role\":\"tool\"")
                    || history.contains("tool_result");
                let tools: Vec<String> = payload["tools"]
                    .as_array()
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|tool| {
                                tool["function"]["name"]
                                    .as_str()
                                    .or(tool["name"].as_str())
                                    .map(str::to_owned)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let tool = tools
                    .iter()
                    .find(|name| name.as_str() == "glob")
                    .or(tools.first())
                    .cloned();
                seen.lock().unwrap().push(Seen {
                    path: path.clone(),
                    authorization: header("authorization"),
                    api_key: header("x-api-key"),
                    model: model.clone(),
                    tool_result,
                });
                let text = format!("HARNESS-OK {model}");
                let call = tool.filter(|_| !tool_result);
                let args = json!({"pattern": "*.txt"}).to_string();
                let frames: Vec<String> = if path.ends_with("/chat/completions") {
                    let delta = match &call {
                        Some(name) => {
                            json!({"role": "assistant", "tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": name, "arguments": args}}]})
                        }
                        None => json!({"role": "assistant", "content": text}),
                    };
                    let finish = if call.is_some() { "tool_calls" } else { "stop" };
                    vec![
                        json!({"id": "c1", "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": null}]}),
                        json!({"id": "c1", "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
                            "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}}),
                    ]
                    .into_iter()
                    .map(|frame| format!("data: {frame}\n\n"))
                    .chain(std::iter::once("data: [DONE]\n\n".to_owned()))
                    .collect()
                } else if path.ends_with("/responses") {
                    let item = match &call {
                        Some(name) => {
                            json!({"type": "function_call", "id": "fc1", "call_id": "call_1", "name": name, "arguments": args, "status": "completed"})
                        }
                        None => {
                            json!({"type": "message", "id": "m1", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": text, "annotations": []}]})
                        }
                    };
                    let response = json!({"id": "r1", "object": "response", "status": "completed", "model": model, "output": [item],
                        "usage": {"input_tokens": 10, "output_tokens": 3, "total_tokens": 13}});
                    let mut frames = vec![
                        json!({"type": "response.created", "response": {"id": "r1", "object": "response", "status": "in_progress", "model": model, "output": []}}),
                    ];
                    if call.is_some() {
                        let mut added = item.clone();
                        added["arguments"] = json!("");
                        added["status"] = json!("in_progress");
                        frames.push(json!({"type": "response.output_item.added", "output_index": 0, "item": added}));
                        frames.push(json!({"type": "response.function_call_arguments.delta", "output_index": 0, "item_id": "fc1", "delta": args}));
                        frames.push(json!({"type": "response.function_call_arguments.done", "output_index": 0, "item_id": "fc1", "arguments": args}));
                    } else {
                        frames.push(json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "message", "id": "m1", "role": "assistant", "status": "in_progress", "content": []}}));
                        frames.push(json!({"type": "response.content_part.added", "output_index": 0, "item_id": "m1", "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}));
                        frames.push(json!({"type": "response.output_text.delta", "output_index": 0, "item_id": "m1", "content_index": 0, "delta": text}));
                        frames.push(json!({"type": "response.output_text.done", "output_index": 0, "item_id": "m1", "content_index": 0, "text": text}));
                    }
                    frames.push(json!({"type": "response.output_item.done", "output_index": 0, "item": item}));
                    frames.push(json!({"type": "response.completed", "response": response}));
                    frames
                        .into_iter()
                        .map(|frame| format!("data: {frame}\n\n"))
                        .collect()
                } else {
                    let (block, delta, stop) = match &call {
                        Some(name) => (
                            json!({"type": "tool_use", "id": "toolu_1", "name": name, "input": {}}),
                            json!({"type": "input_json_delta", "partial_json": args}),
                            "tool_use",
                        ),
                        None => (
                            json!({"type": "text", "text": ""}),
                            json!({"type": "text_delta", "text": text}),
                            "end_turn",
                        ),
                    };
                    [
                        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
                        json!({"type": "content_block_start", "index": 0, "content_block": block}),
                        json!({"type": "content_block_delta", "index": 0, "delta": delta}),
                        json!({"type": "content_block_stop", "index": 0}),
                        json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 3}}),
                        json!({"type": "message_stop"}),
                    ]
                    .into_iter()
                    .map(|frame| format!("event: {}\ndata: {frame}\n\n", frame["type"].as_str().unwrap()))
                    .collect()
                };
                let body = frames.concat();
                let response = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    port
}

fn dsh(binary: &Path, home: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut command = if binary
        .extension()
        .is_some_and(|extension| extension == "js")
    {
        let mut command = Command::new("node");
        command.arg(binary);
        command
    } else {
        Command::new(binary)
    };
    command
        .args(args)
        .current_dir(cwd)
        .env("DSH_HOME", home)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .output()
        .expect("dsh starts")
}

#[test]
#[ignore = "requires AGENTHARBOR_TEST_DSH_BIN pointing to an installed dsh (or its lib/bin.js)"]
fn real_harness_streams_and_calls_tools_through_every_route_kind() {
    let _router = router_proxy::global_router_test_lock();
    let binary =
        PathBuf::from(std::env::var_os("AGENTHARBOR_TEST_DSH_BIN").expect("installed dsh"));
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("dsh");
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("hello.txt"), "hello from the workspace").unwrap();
    // The shipped template initializes the profile on first use.
    let init = dsh(
        &binary,
        &home,
        &cwd,
        &["--profile", "headless", "--dump-config"],
    );
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let patch = home.join("profiles/headless/cordis.patch.yml");
    let original = std::fs::read_to_string(&patch).unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let upstream_port = runtime.block_on(upstream(seen.clone()));
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    let mut store = Store {
        providers: vec![Provider {
            id: "relay".into(),
            name: "Loopback relay".into(),
            base_url: format!("http://127.0.0.1:{upstream_port}/"),
            api_key: "sk-upstream".into(),
            model: "chat-model".into(),
            protocol: ModelProtocol::ChatCompletions,
            enabled: true,
            network_mode: "direct".into(),
            test_status: "ok".into(),
            models: [
                ModelProtocol::ChatCompletions,
                ModelProtocol::Responses,
                ModelProtocol::AnthropicMessages,
            ]
            .into_iter()
            .zip(["chat-model", "responses-model", "messages-model"])
            .map(|(protocol, id)| newapi::Model {
                id: id.into(),
                included: true,
                protocol: Some(protocol),
                context_window: Some(128_000),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        }],
        aggregates: vec![aggregation::AggregateModel {
            id: "pool".into(),
            public_model_id: "public-pool".into(),
            display_name: "Pool".into(),
            enabled: true,
            members: vec![aggregation::AggregateMember {
                id: "member".into(),
                provider_id: "relay".into(),
                actual_model_id: "chat-model".into(),
                enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        router: super::super::super::router::RouterSettings {
            bearer_token: "router-test-token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    secret_file::write(
        &paths.app_data_dir.join("newapi-providers.dat"),
        &serde_json::to_vec(&store).unwrap(),
    )
    .unwrap();
    store.router.port = runtime
        .block_on(router_proxy::ensure_running(
            0,
            paths.clone(),
            ApiProxyConfigPayload::default(),
        ))
        .unwrap();

    let mut clients = ClientPaths::from_home(temp.path());
    clients.harness = home.clone();
    let manager = ConfigManager::new(&paths.app_data_dir, clients);
    let base = ConfigRequest {
        target_id: TargetId::Harness,
        provider_id: "relay".into(),
        model: "chat-model".into(),
        context_window: None,
        max_output_tokens: None,
        options: ClientOptions {
            access_mode: ClientAccessMode::Router,
            provider_ids: vec!["relay".into()],
            harness_profile: Some("headless".into()),
            ..Default::default()
        },
    };
    let mut requests = Vec::new();
    for model in ["chat-model", "responses-model", "messages-model"] {
        let mut request = base.clone();
        request.model = model.into();
        requests.push((model.to_owned(), request));
    }
    for ingress in [ModelProtocol::AnthropicMessages, ModelProtocol::Responses] {
        let mut request = base.clone();
        request.provider_id.clear();
        request.model = "public-pool".into();
        request.options.aggregate_ingress_protocol = Some(ingress);
        request.options.model_selection = Some(ClientModelSelection {
            default_target: Some(ModelTargetRef::aggregate("pool")),
            available_targets: vec![
                ModelTargetRef::aggregate("pool"),
                ModelTargetRef::provider("relay", "chat-model"),
            ],
        });
        requests.push(("public-pool".to_owned(), request));
    }
    for (model, request) in requests {
        seen.lock().unwrap().clear();
        let preview = manager.preview(&store, &request).unwrap();
        manager.apply(&store, &request, &preview.revision).unwrap();
        let output = dsh(
            &binary,
            &home,
            &cwd,
            &["--profile", "headless", "List the text files here."],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{model}: {}\n{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        let seen = seen.lock().unwrap().clone();
        // An aggregate answers with its member's model.
        let upstream_model = if model == "public-pool" {
            "chat-model"
        } else {
            model.as_str()
        };
        assert!(
            stdout.contains(&format!("HARNESS-OK {upstream_model}")),
            "{model}: {stdout}\n{seen:#?}"
        );
        assert!(
            seen.iter().any(|request| request.tool_result),
            "{model}: the tool result went back upstream: {seen:#?}"
        );
        for request in &seen {
            assert_eq!(request.model, upstream_model);
            // The router swapped the local token for the relay's key.
            let key = request
                .authorization
                .as_deref()
                .map(|value| value.trim_start_matches("Bearer "))
                .or(request.api_key.as_deref());
            assert_eq!(key, Some("sk-upstream"), "{request:?}");
        }
        assert!(
            route_trace::recent()
                .iter()
                .any(|route| route.client_source.as_deref() == Some("harness")
                    && route.status == Some(200)),
            "{model}: requests carry Harness's source"
        );
        if model == "messages-model" {
            assert!(
                seen.iter().all(|request| request.path == "/v1/messages"),
                "{seen:#?}"
            );
        }
    }
    manager.restore(TargetId::Harness).unwrap();
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), original);
    assert!(!home.join(".credentials.yaml").exists());
    router_proxy::stop();
}
