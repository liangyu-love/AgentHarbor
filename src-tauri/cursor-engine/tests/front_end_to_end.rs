//! Drives the engine the way Cursor does, through its local port: a Run is
//! appended with BidiAppend, its output read from RunSSE, and the model behind
//! it is an OpenAI Chat endpoint on loopback.
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{http::header, response::IntoResponse, routing::post, Json, Router};
use cursor_engine::{
    cursor::protocol::{
        connect,
        proto::{agent::v1 as pb, aiserver::v1 as ai},
    },
    host::Host,
    model::{ModelConfig, ModelConfigInput, ModelType, OPENAI_CHAT_ENDPOINT},
    Engine, EngineConfig,
};
use prost::Message;
use serde_json::{json, Value};

async fn fake_chat(seen: Arc<Mutex<Vec<Value>>>) -> SocketAddr {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(body);
                let chunks = [
                    json!({"id": "c", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hello "}}]}),
                    json!({"id": "c", "choices": [{"index": 0, "delta": {"content": "from the fake"}}]}),
                    json!({"id": "c", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                           "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}}),
                ];
                let mut body = String::new();
                for chunk in chunks {
                    body.push_str(&format!("data: {chunk}\n\n"));
                }
                body.push_str("data: [DONE]\n\n");
                ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

fn offered_model(address: SocketAddr) -> ModelConfig {
    ModelConfig::from_input(&ModelConfigInput {
        sort_order: 0,
        display_name: "Fake Chat".into(),
        group_name: Some("Loopback".into()),
        model_type: ModelType::OpenAi,
        base_url: format!("http://{address}/v1/chat/completions"),
        use_full_url: true,
        api_key: "router-token".into(),
        tooltip_data: "Fake Chat".into(),
        model_id: "relay-a/upstream-model".into(),
        reasoning_effort: None,
        openai_endpoint: OPENAI_CHAT_ENDPOINT.into(),
        openai_extra_params_enabled: false,
        openai_extra_params: json!({}),
        custom_headers_enabled: false,
        custom_headers: json!({}),
        anthropic_extra_params_enabled: false,
        anthropic_extra_params: json!({}),
        context_window_tokens: Some(200_000),
        max_completion_tokens: None,
        anthropic_max_tokens: None,
        anthropic_thinking_effort: None,
        thinking_budget_tokens: None,
    })
    .unwrap()
}

fn run_request(conversation_id: &str, model_id: &str) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::RunRequest(pb::AgentRunRequest {
            action: Some(pb::ConversationAction {
                action: Some(pb::conversation_action::Action::UserMessageAction(pb::UserMessageAction {
                    user_message: Some(pb::UserMessage {
                        text: "say hello".into(),
                        message_id: "user-1".into(),
                        mode: pb::AgentMode::Agent as i32,
                        ..Default::default()
                    }),
                    request_context: Some(pb::RequestContext::default()),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            conversation_id: Some(conversation_id.into()),
            requested_model: Some(pb::RequestedModel { model_id: model_id.into(), ..Default::default() }),
            ..Default::default()
        })),
    }
}

async fn append(client: &reqwest::Client, url: &str, seqno: i64, message: &pb::AgentClientMessage) {
    let append = ai::BidiAppendRequest {
        data: hex::encode(message.encode_to_vec()),
        request_id: Some(ai::BidiRequestId { request_id: "request-1".into() }),
        append_seqno: seqno,
        data_binary: Vec::new(),
    };
    let response = client
        .post(format!("{url}/aiserver.v1.BidiService/BidiAppend"))
        .header(header::CONTENT_TYPE, "application/proto")
        .body(append.encode_to_vec())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "BidiAppend: {}", response.status());
}

/// Plays Cursor's side of RunSSE: stores every blob the engine hands over,
/// and returns the reply text once the stream ends.
async fn play_cursor(client: &reqwest::Client, url: &str, mut response: reqwest::Response) -> String {
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut seqno = 1;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let chunk = tokio::time::timeout_at(deadline, response.chunk()).await.expect("RunSSE ends in time").unwrap();
        let Some(chunk) = chunk else { panic!("RunSSE closed without an end-of-stream frame") };
        buffer.extend_from_slice(&chunk);
        while buffer.len() >= 5 {
            let length = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;
            if buffer.len() < 5 + length {
                break;
            }
            let flags = buffer[0];
            let payload = buffer[5..5 + length].to_vec();
            buffer.drain(..5 + length);
            if flags & connect::END_STREAM_FLAG != 0 {
                let end: Value = serde_json::from_slice(&payload).unwrap();
                assert!(end.get("error").is_none(), "the run failed: {end}");
                return text;
            }
            match pb::AgentServerMessage::decode(payload.as_slice()).unwrap().message {
                Some(pb::agent_server_message::Message::InteractionUpdate(update)) => {
                    if let Some(pb::interaction_update::Message::TextDelta(delta)) = update.message {
                        text.push_str(&delta.text);
                    }
                }
                Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                    let result = match kv.message {
                        Some(pb::kv_server_message::Message::SetBlobArgs(_)) => {
                            pb::kv_client_message::Message::SetBlobResult(pb::SetBlobResult { error: None })
                        }
                        Some(pb::kv_server_message::Message::GetBlobArgs(_)) => {
                            pb::kv_client_message::Message::GetBlobResult(pb::GetBlobResult::default())
                        }
                        _ => continue,
                    };
                    let reply = pb::AgentClientMessage {
                        message: Some(pb::agent_client_message::Message::KvClientMessage(pb::KvClientMessage {
                            id: kv.id,
                            message: Some(result),
                            ..Default::default()
                        })),
                    };
                    append(client, url, seqno, &reply).await;
                    seqno += 1;
                }
                _ => {}
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_run_through_the_port_reaches_the_model_and_streams_back() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let address = fake_chat(seen.clone()).await;
    let model = offered_model(address);
    let host = Host::new();
    host.set_egress(cursor_engine::Egress::Direct);
    host.set_models(vec![model.clone()]);
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::start(EngineConfig { data_dir: directory.path().to_owned(), port: 0 }, host).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    append(&client, &engine.url(), 0, &run_request("conversation-1", &model.model_hash)).await;

    let subscribe = connect::encode_message(&pb::BidiRequestId { request_id: "request-1".into() }).unwrap();
    let stream = client
        .post(format!("{}/agent.v1.AgentService/RunSSE", engine.url()))
        .header(header::CONTENT_TYPE, "application/connect+proto")
        .body(subscribe.to_vec())
        .send()
        .await
        .unwrap();
    assert!(stream.status().is_success());
    assert_eq!(play_cursor(&client, &engine.url(), stream).await, "Hello from the fake");

    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "relay-a/upstream-model");
    assert_eq!(requests[0]["stream"], true);
    let sent = requests[0]["messages"].to_string();
    assert!(sent.contains("say hello"));
    assert!(sent.contains("AgentHarbor"), "the system prompt is AgentHarbor's");
    engine.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn models_appear_in_cursors_lists_even_without_cursor_reachable() {
    let host = Host::new();
    host.set_egress(cursor_engine::Egress::Direct);
    host.set_models(vec![offered_model("127.0.0.1:9".parse().unwrap())]);
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::start(EngineConfig { data_dir: directory.path().to_owned(), port: 0 }, host).await.unwrap();
    // Read through the front as the IDE would, but toward a Cursor server
    // name nothing answers for: the local list still comes back.
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(engine.url()).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(engine.authority().certificate_pem().as_bytes()).unwrap())
        .build()
        .unwrap();
    let response = client
        .post("https://unreachable.invalid.cursor.sh/aiserver.v1.AiService/AvailableModels")
        .header(header::CONTENT_TYPE, "application/proto")
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = response.bytes().await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Fake Chat"));
    engine.stop().await;
}
