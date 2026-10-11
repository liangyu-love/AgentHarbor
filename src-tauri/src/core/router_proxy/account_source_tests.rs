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
