//! An account source served through the live router: signed as the account,
//! shaped as the vendor takes it, refusals explained, aggregates failing over.
use super::*;
use crate::core::account_sources::{self, store as sources, tests::mock, SourceKind};
use crate::core::aggregation;
use crate::platform::paths::CodexPaths;

const ROUTER_TOKEN: &str = "router-test-token";
const CODEX_PROMPT: &str = "You are Codex, a coding agent based on GPT-5.";

fn chat(content: &str) -> String {
    json!({"id":"chatcmpl-1","object":"chat.completion","model":"m","choices":[{"index":0,"finish_reason":"stop",
        "message":{"role":"assistant","content":content}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}).to_string()
}

#[test]
fn workbuddy_requests_are_signed_explained_and_fail_over_in_an_aggregate() {
    let _guard = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        let upstream = mock(|seen| {
            let body: Value = serde_json::from_str(&seen.body).unwrap_or(Value::Null);
            match seen.path.as_str() {
                "/v2/chat/completions" if body["messages"].to_string().contains(CODEX_PROMPT) => (400,
                    json!({"code":400,"msg":"WorkBuddy: Illegal API invocation from an unapproved channel"}).to_string()),
                "/v2/chat/completions" => (200, chat("from workbuddy")),
                _ => (404, "{}".into()),
            }
        }).await;
        let backup = mock(|seen| match seen.path.as_str() {
            "/v1/chat/completions" => (200, chat("from backup")),
            _ => (404, "{}".into()),
        }).await;
        account_sources::set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        paths.ensure_directories().unwrap();
        sources::mutate(&paths.app_data_dir, |store| {
            store.keep_sign_in(sources::Account {
                id: "wb-account".into(), kind: SourceKind::Workbuddy, name: "Harbor".into(), subject: "uid-9".into(),
                plan: None, active: true, added_at: 1, lapsed: None, quota: None,
                credentials: sources::Credentials::WorkBuddy(account_sources::workbuddy::Creds {
                    uid: "uid-9".into(), access: "wb-token".into(), refresh: "wb-refresh".into(),
                    expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000, ..Default::default()
                }),
            });
            Ok(())
        }).unwrap();
        account_sources::set_exposed(&paths, SourceKind::Workbuddy, true).unwrap();
        let store = router::mutate_store(&paths, |store| {
            store.router.bearer_token = ROUTER_TOKEN.into();
            store.providers.push(router::Provider {
                id: "backup".into(), name: "Backup".into(), base_url: backup.base.clone(), api_key: "sk-backup".into(),
                protocol: router::ModelProtocol::ChatCompletions, model: "backup-model".into(), enabled: true,
                models: vec![crate::core::newapi::Model { id: "backup-model".into(), ..Default::default() }],
                ..Default::default()
            });
            store.aggregates.push(aggregation::AggregateModel {
                id: "agg-1".into(), public_model_id: "harbor-glm".into(), display_name: "Harbor GLM".into(),
                members: vec![
                    aggregation::AggregateMember { id: "m-wb".into(), provider_id: "account-workbuddy".into(),
                        actual_model_id: "glm-5.3".into(), priority: 0, ..Default::default() },
                    aggregation::AggregateMember { id: "m-backup".into(), provider_id: "backup".into(),
                        actual_model_id: "backup-model".into(), priority: 1, ..Default::default() },
                ],
                ..Default::default()
            });
            Ok(())
        }).unwrap();
        aggregation::reconcile_runtime(&store);
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        let send = |path: &str, model: &str, system: Option<&str>| {
            let mut messages = Vec::new();
            if let Some(system) = system {
                messages.push(json!({"role":"system","content":system}));
            }
            messages.push(json!({"role":"user","content":"hi"}));
            client.post(format!("{base}{path}")).bearer_auth(ROUTER_TOKEN)
                .json(&json!({"model":model,"messages":messages,"stream":false})).send()
        };

        let answer = send("/p/account-workbuddy/v1/chat/completions", "glm-5.3", None).await.unwrap();
        assert_eq!(answer.status(), 200);
        assert_eq!(answer.json::<Value>().await.unwrap()["choices"][0]["message"]["content"], "from workbuddy");
        let sent = upstream.seen.lock().unwrap().iter().rev().find(|seen| seen.path == "/v2/chat/completions").cloned().unwrap();
        assert_eq!(sent.headers["authorization"], "Bearer wb-token");
        assert_eq!(sent.headers["x-user-id"], "uid-9");
        assert_eq!(sent.headers["x-ide-type"], "WorkBuddy");
        assert!(sent.headers["user-agent"].starts_with("WorkBuddy/"));
        assert!(!sent.headers.values().any(|value| value.contains(ROUTER_TOKEN)), "the router token never leaves");
        let body: Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["model"], "glm-5.3");
        assert_eq!(body["messages"][0]["role"], "system", "WorkBuddy is always sent a system message first");

        let refused = send("/p/account-workbuddy/v1/chat/completions", "glm-5.3", Some(CODEX_PROMPT)).await.unwrap();
        assert_eq!(refused.status(), 400);
        let text = refused.text().await.unwrap();
        assert!(text.contains("unapproved channel") && text.contains(account_sources::workbuddy::REFUSED_HINT), "{text}");

        let moved = send("/aggregate/v1/chat/completions", "harbor-glm", Some(CODEX_PROMPT)).await.unwrap();
        assert_eq!(moved.status(), 200);
        assert_eq!(moved.json::<Value>().await.unwrap()["choices"][0]["message"]["content"], "from backup");
        assert_eq!(backup.seen.lock().unwrap().len(), 1);

        sources::mutate(&paths.app_data_dir, |store| {
            store.accounts.clear();
            Ok(())
        }).unwrap();
        let signed_out = send("/p/account-workbuddy/v1/chat/completions", "glm-5.3", None).await.unwrap();
        assert_eq!(signed_out.status(), 401);
        assert!(signed_out.text().await.unwrap().contains("Accounts page"));
        let still_served = send("/aggregate/v1/chat/completions", "harbor-glm", None).await.unwrap();
        assert_eq!(still_served.status(), 200, "an aggregate passes over a source with no account");

        stop_unchecked();
        account_sources::set_endpoint_override(SourceKind::Workbuddy, None);
    });
}

#[test]
fn copilot_requests_carry_the_editor_headers_and_reach_each_models_own_api() {
    let _guard = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        let upstream = mock(|seen| {
            let body: Value = serde_json::from_str(&seen.body).unwrap_or(Value::Null);
            match seen.path.as_str() {
                "/copilot_internal/v2/token" => (200, json!({"token":"cop-1","expires_at":chrono::Utc::now().timestamp() + 1800}).to_string()),
                "/chat/completions" if body["model"] == "gpt-locked" => (400,
                    json!({"error":{"message":"The requested model is not supported.","code":"model_not_supported"}}).to_string()),
                "/chat/completions" => (200, chat("from copilot chat")),
                "/v1/messages" => (200, json!({"id":"msg_1","type":"message","role":"assistant","model":body["model"],
                    "content":[{"type":"text","text":"from copilot messages"}],"stop_reason":"end_turn",
                    "usage":{"input_tokens":3,"output_tokens":2}}).to_string()),
                "/responses" => (200, json!({"id":"resp_1","object":"response","status":"completed","model":body["model"],
                    "output":[{"type":"message","id":"m1","role":"assistant","status":"completed",
                        "content":[{"type":"output_text","text":"from copilot responses","annotations":[]}]}],
                    "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}).to_string()),
                _ => (404, "{}".into()),
            }
        }).await;
        account_sources::set_endpoint_override(SourceKind::Copilot, Some(upstream.base.clone()));
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        paths.ensure_directories().unwrap();
        let model = |id: &str, protocol| sources::SourceModel { id: id.into(), name: id.into(), protocol: Some(protocol), ..Default::default() };
        sources::mutate(&paths.app_data_dir, |store| {
            store.keep_sign_in(sources::Account {
                id: "cp-account".into(), kind: SourceKind::Copilot, name: "Octo".into(), subject: "octo".into(),
                plan: Some("Pro".into()), active: true, added_at: 1, lapsed: None, quota: None,
                credentials: sources::Credentials::Copilot(account_sources::copilot::Creds { token: "gho-route".into(), login: "Octo".into() }),
            });
            store.models.insert(SourceKind::Copilot, vec![
                model("claude-sonnet-4.6", router::ModelProtocol::AnthropicMessages),
                model("gpt-5.5", router::ModelProtocol::Responses),
                model("gpt-4.1", router::ModelProtocol::ChatCompletions),
                model("gpt-locked", router::ModelProtocol::ChatCompletions),
            ]);
            Ok(())
        }).unwrap();
        account_sources::set_exposed(&paths, SourceKind::Copilot, true).unwrap();
        router::mutate_store(&paths, |store| {
            store.router.bearer_token = ROUTER_TOKEN.into();
            Ok(())
        }).unwrap();
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");
        let chat_request = |model: &str, content: Value| client.post(format!("{base}/p/account-copilot/v1/chat/completions"))
            .bearer_auth(ROUTER_TOKEN).json(&json!({"model":model,"messages":[{"role":"user","content":content}],"stream":false})).send();
        let last = |path: &str| upstream.seen.lock().unwrap().iter().rev().find(|seen| seen.path == path).cloned().unwrap();

        let answer = chat_request("claude-sonnet-4.6", json!("hi")).await.unwrap();
        let status = answer.status();
        let answer_text = answer.text().await.unwrap();
        assert_eq!(status, 200, "{answer_text}");
        let answer: Value = serde_json::from_str(&answer_text).unwrap();
        assert_eq!(answer["choices"][0]["message"]["content"], "from copilot messages");
        let sent = last("/v1/messages");
        assert_eq!(sent.headers["authorization"], "Bearer cop-1");
        assert_eq!(sent.headers["copilot-integration-id"], "vscode-chat");
        assert_eq!(sent.headers["x-github-api-version"], "2026-01-09");
        assert_eq!(sent.headers["x-initiator"], "user");
        assert_eq!(sent.headers["anthropic-version"], "2023-06-01");
        assert!(!sent.headers.values().any(|value| value.contains(ROUTER_TOKEN) || value.contains("gho-route")),
            "neither the router token nor the GitHub token reaches Copilot's API");

        let image = json!([{"type":"text","text":"what is this"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]);
        let answer = chat_request("gpt-4.1", image).await.unwrap();
        assert_eq!(answer.json::<Value>().await.unwrap()["choices"][0]["message"]["content"], "from copilot chat");
        assert_eq!(last("/chat/completions").headers["copilot-vision-request"], "true");

        let responses = client.post(format!("{base}/p/account-copilot/v1/responses")).bearer_auth(ROUTER_TOKEN)
            .json(&json!({"model":"gpt-5.5","input":"hi","stream":false})).send().await.unwrap();
        assert_eq!(responses.status(), 200);
        assert!(responses.text().await.unwrap().contains("from copilot responses"));
        assert_eq!(serde_json::from_str::<Value>(&last("/responses").body).unwrap()["model"], "gpt-5.5");

        let refused = chat_request("gpt-locked", json!("hi")).await.unwrap();
        assert_eq!(refused.status(), 400);
        assert!(refused.text().await.unwrap().contains(account_sources::copilot::SETTINGS_URL), "a plan refusal says where to enable the model");
        let exchanges = upstream.seen.lock().unwrap().iter().filter(|seen| seen.path == "/copilot_internal/v2/token").count();
        assert_eq!(exchanges, 1, "one Copilot token serves every request until it nears expiry");

        stop_unchecked();
        account_sources::set_endpoint_override(SourceKind::Copilot, None);
    });
}

fn sse(chunks: &[Value]) -> Vec<u8> {
    chunks.iter().map(|chunk| format!("data: {chunk}\r\n\r\n")).collect::<String>().into_bytes()
}

/// Keep a signed-in account with its models and share the source.
fn keep_shared(paths: &CodexPaths, kind: SourceKind, credentials: sources::Credentials, models: &[&str]) {
    sources::mutate(&paths.app_data_dir, |store| {
        store.keep_sign_in(sources::Account {
            id: format!("{kind:?}-account"), kind, name: "me@example.com".into(), subject: "me".into(),
            plan: None, active: true, added_at: 1, lapsed: None, quota: None, credentials,
        });
        store.models.insert(kind, models.iter().map(|id| sources::SourceModel { id: (*id).into(), name: (*id).into(),
            context_window: Some(200_000), ..Default::default() }).collect());
        Ok(())
    }).unwrap();
    account_sources::set_exposed(paths, kind, true).unwrap();
    router::mutate_store(paths, |store| {
        store.router.bearer_token = ROUTER_TOKEN.into();
        Ok(())
    }).unwrap();
}

#[test]
fn antigravity_is_reached_through_code_assist_from_every_ingress_and_keeps_thought_signatures() {
    let _guard = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        let upstream = account_sources::tests::mock_raw(|seen| {
            if seen.path != "/v1internal:streamGenerateContent?alt=sse" {
                return (404, "application/json", b"{}".to_vec());
            }
            let body: Value = serde_json::from_str(&seen.body).unwrap();
            let text = body.to_string();
            let reply = if text.contains("functionResponse") {
                vec![json!({"response":{"candidates":[{"content":{"role":"model","parts":[{"text":"listed"}]},"finishReason":"STOP"}]}})]
            } else if text.contains("use the tool") {
                vec![json!({"response":{"candidates":[{"content":{"role":"model","parts":[
                    {"functionCall":{"id":"fc-ag","name":"bash","args":{"cmd":"ls"}},"thoughtSignature":"sig-ag"}]},"finishReason":"STOP"}]}})]
            } else {
                vec![
                    json!({"response":{"candidates":[{"content":{"role":"model","parts":[{"text":"weighing it","thought":true}]}}]}}),
                    json!({"response":{"candidates":[{"content":{"role":"model","parts":[{"text":"from antigravity"}]}}]}}),
                    json!({"response":{"candidates":[{"content":{"role":"model","parts":[]},"finishReason":"STOP"}],
                        "usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":3,"thoughtsTokenCount":2}}}),
                ]
            };
            (200, "text/event-stream", sse(&reply))
        }).await;
        account_sources::set_endpoint_override(SourceKind::Antigravity, Some(upstream.base.clone()));
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        paths.ensure_directories().unwrap();
        keep_shared(&paths, SourceKind::Antigravity, sources::Credentials::Google(account_sources::google::Creds {
            email: "me@example.com".into(), access: "ya29-route".into(), refresh: "1//route".into(),
            expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000, project: "proj-route".into(), ..Default::default()
        }), &["gemini-3-flash", "claude-sonnet-4-6"]);
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}/p/account-antigravity/v1");
        let last = || {
            let seen = upstream.seen.lock().unwrap().last().cloned().unwrap();
            (seen.headers.clone(), serde_json::from_str::<Value>(&seen.body).unwrap())
        };

        // Chat, not streamed: one completion, with the thinking beside the answer.
        let chat: Value = client.post(format!("{base}/chat/completions")).bearer_auth(ROUTER_TOKEN)
            .json(&json!({"model":"gemini-3-flash","messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}],"stream":false}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(chat["choices"][0]["message"]["content"], "from antigravity", "{chat}");
        assert_eq!(chat["choices"][0]["message"]["reasoning_content"], "weighing it");
        assert_eq!(chat["usage"]["prompt_tokens"], 9);
        let (headers, envelope) = last();
        assert_eq!(headers["authorization"], "Bearer ya29-route");
        assert!(headers["user-agent"].starts_with("antigravity/hub/"));
        assert!(!headers.values().any(|value| value.contains(ROUTER_TOKEN)), "the router token never leaves");
        assert_eq!((envelope["project"].as_str(), envelope["model"].as_str(), envelope["requestType"].as_str()),
            (Some("proj-route"), Some("gemini-3-flash"), Some("agent")));
        assert_eq!(envelope["request"]["systemInstruction"]["parts"][0]["text"], "be brief");

        // Responses, streamed (Codex): Gemini's chunks become Responses events.
        let streamed = client.post(format!("{base}/responses")).bearer_auth(ROUTER_TOKEN)
            .json(&json!({"model":"gemini-3-flash","input":"hi","stream":true})).send().await.unwrap();
        assert_eq!(streamed.status(), 200);
        let events = streamed.text().await.unwrap();
        assert!(events.contains("response.output_text.delta") && events.contains("from antigravity") && events.contains("response.completed"), "{events}");

        // Messages (Claude Code): a call made, its signature back on the next turn.
        let called: Value = client.post(format!("{base}/chat/completions")).bearer_auth(ROUTER_TOKEN)
            .json(&json!({"model":"gemini-3-flash","messages":[{"role":"user","content":"use the tool"}],
                "tools":[{"type":"function","function":{"name":"bash","parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}}]}))
            .send().await.unwrap().json().await.unwrap();
        let call = &called["choices"][0]["message"]["tool_calls"][0];
        assert_eq!((call["id"].as_str(), call["function"]["name"].as_str()), (Some("fc-ag"), Some("bash")), "{called}");
        assert_eq!(called["choices"][0]["finish_reason"], "tool_calls");
        let answered = client.post(format!("{base}/messages")).bearer_auth(ROUTER_TOKEN).header("anthropic-version", "2023-06-01")
            .json(&json!({"model":"gemini-3-flash","max_tokens":256,"messages":[
                {"role":"user","content":"use the tool"},
                {"role":"assistant","content":[{"type":"tool_use","id":"fc-ag","name":"bash","input":{"cmd":"ls"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"fc-ag","content":"a.txt"}]}],
                "tools":[{"name":"bash","input_schema":{"type":"object","properties":{"cmd":{"type":"string"}}}}]}))
            .send().await.unwrap();
        assert_eq!(answered.status(), 200);
        let answered: Value = answered.json().await.unwrap();
        assert_eq!(answered["content"][0]["text"], "listed", "{answered}");
        let (_, envelope) = last();
        let call = envelope["request"]["contents"].as_array().unwrap().iter()
            .flat_map(|content| content["parts"].as_array().unwrap().iter())
            .find(|part| part.get("functionCall").is_some()).cloned().unwrap();
        assert_eq!(call["thoughtSignature"], "sig-ag", "Gemini gets its own signature back on its call");

        stop_unchecked();
        account_sources::set_endpoint_override(SourceKind::Antigravity, None);
    });
}

#[test]
fn kiro_answers_through_its_event_stream_to_chat_and_messages_clients() {
    let _guard = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        use account_sources::kiro::tests::frame;
        let upstream = account_sources::tests::mock_raw(|seen| {
            if seen.path != "/generateAssistantResponse" {
                return (404, "application/json", b"{}".to_vec());
            }
            let mut stream = Vec::new();
            stream.extend(frame("assistantResponseEvent", &json!({"content":"from "})));
            stream.extend(frame("assistantResponseEvent", &json!({"content":"kiro"})));
            stream.extend(frame("metadataEvent", &json!({"tokenUsage":{"uncachedInputTokens":11,"outputTokens":4}})));
            (200, "application/vnd.amazon.eventstream", stream)
        }).await;
        account_sources::set_endpoint_override(SourceKind::Kiro, Some(upstream.base.clone()));
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        paths.ensure_directories().unwrap();
        keep_shared(&paths, SourceKind::Kiro, sources::Credentials::Kiro(account_sources::kiro::Creds {
            access: "kat-route".into(), refresh: "krt".into(), expires_at: chrono::Utc::now().timestamp() + 3600,
            method: "social".into(), provider: "Github".into(), region: "us-east-1".into(),
            profile: "arn:aws:codewhisperer:us-east-1:7:profile/P".into(), ..Default::default()
        }), &["claude-sonnet-4.5"]);
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}/p/account-kiro/v1");

        let chat: Value = client.post(format!("{base}/chat/completions")).bearer_auth(ROUTER_TOKEN)
            .json(&json!({"model":"claude-sonnet-4.5","messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}]}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(chat["choices"][0]["message"]["content"], "from kiro", "{chat}");
        assert_eq!(chat["usage"]["prompt_tokens"], 11);
        let seen = upstream.seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(seen.headers["authorization"], "Bearer kat-route");
        assert_eq!(seen.headers["x-amzn-kiro-agent-mode"], "vibe");
        assert!(seen.headers["user-agent"].starts_with("aws-sdk-rust/"));
        assert!(!seen.headers.values().any(|value| value.contains(ROUTER_TOKEN)), "the router token never leaves");
        let body: Value = serde_json::from_str(&seen.body).unwrap();
        let current = &body["conversationState"]["currentMessage"]["userInputMessage"];
        assert_eq!(current["content"], "be brief\n\nhi");
        assert_eq!(current["modelId"], "claude-sonnet-4.5");
        assert_eq!(body["profileArn"], "arn:aws:codewhisperer:us-east-1:7:profile/P");

        // Claude Code, streamed: Messages events out of Kiro's frames.
        let streamed = client.post(format!("{base}/messages")).bearer_auth(ROUTER_TOKEN).header("anthropic-version", "2023-06-01")
            .json(&json!({"model":"claude-sonnet-4.5","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"hi"}]}))
            .send().await.unwrap();
        assert_eq!(streamed.status(), 200);
        let events = streamed.text().await.unwrap();
        assert!(events.contains("content_block_delta") && events.contains("kiro") && events.contains("message_stop"), "{events}");

        stop_unchecked();
        account_sources::set_endpoint_override(SourceKind::Kiro, None);
    });
}
