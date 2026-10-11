//! The general gateway against the live router: named keys only, every model
//! under `<short name>/<model>`, protocol translation as for any route.
use super::*;
use crate::core::account_sources::tests::mock;
use crate::core::aggregation;
use crate::platform::paths::CodexPaths;

const ROUTER_TOKEN: &str = "internal-router-token";
const GATEWAY_KEY: &str = "sk-ah-fixture-gateway-key-for-hermes";

fn chat_reply(model: &str) -> String {
    json!({"id":"chatcmpl-1","object":"chat.completion","model":model,"choices":[{"index":0,"finish_reason":"stop",
        "message":{"role":"assistant","content":"hello through the gateway"}}],
        "usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}).to_string()
}

#[test]
fn gateway_keys_reach_every_model_by_short_name_with_translation() {
    let _guard = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {
        let upstream = mock(|seen| match seen.path.as_str() {
            "/v1/chat/completions" => {
                let body: Value = serde_json::from_str(&seen.body).unwrap();
                (200, chat_reply(body["model"].as_str().unwrap_or_default()))
            }
            _ => (404, "{}".into()),
        }).await;
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        paths.ensure_directories().unwrap();
        let store = router::mutate_store(&paths, |store| {
            store.router.bearer_token = ROUTER_TOKEN.into();
            store.router.gateway_keys.push(router::GatewayKey {
                id: "key-1".into(), name: "Hermes".into(), key: GATEWAY_KEY.into(), created_at: 1,
            });
            store.providers.push(router::Provider {
                id: "relay-chat-id".into(), name: "Relay Chat".into(), base_url: format!("{}/v1", upstream.base),
                api_key: "sk-relay-secret".into(), protocol: router::ModelProtocol::ChatCompletions, model: "chat-model".into(),
                enabled: true, models: vec![
                    crate::core::newapi::Model { id: "chat-model".into(), ..Default::default() },
                    crate::core::newapi::Model { id: "vendor/deep-model".into(), ..Default::default() },
                    crate::core::newapi::Model { id: "excluded-model".into(), included: false, ..Default::default() },
                ],
                ..Default::default()
            });
            store.aggregates.push(aggregation::AggregateModel {
                id: "pool".into(), public_model_id: "harbor-pool".into(), display_name: "Harbor Pool".into(),
                members: vec![aggregation::AggregateMember { id: "m1".into(), provider_id: "relay-chat-id".into(),
                    actual_model_id: "chat-model".into(), ..Default::default() }],
                ..Default::default()
            });
            Ok(())
        }).unwrap();
        assert_eq!(store.providers[0].gateway_slug.as_deref(), Some("relay-chat"), "a short name is assigned on save");
        aggregation::reconcile_runtime(&store);
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let base = format!("http://127.0.0.1:{port}");

        let models: Value = client.get(format!("{base}/gateway/v1/models")).bearer_auth(GATEWAY_KEY)
            .send().await.unwrap().json().await.unwrap();
        let ids: Vec<&str> = models["data"].as_array().unwrap().iter().filter_map(|entry| entry["id"].as_str()).collect();
        assert_eq!(ids, ["relay-chat/chat-model", "relay-chat/vendor/deep-model", "harbor-pool"]);
        assert_eq!(models["data"][0]["owned_by"], "Relay Chat");
        let anthropic: Value = client.get(format!("{base}/gateway/v1/models")).header("x-api-key", GATEWAY_KEY)
            .header("anthropic-version", "2023-06-01").send().await.unwrap().json().await.unwrap();
        assert_eq!(anthropic["data"][0]["type"], "model");
        assert_eq!(anthropic["first_id"], "relay-chat/chat-model");
        let one: Value = client.get(format!("{base}/gateway/v1/models/harbor-pool")).bearer_auth(GATEWAY_KEY)
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(one["id"], "harbor-pool");

        for (token, path) in [(None, "/gateway/v1/models"), (Some(ROUTER_TOKEN), "/gateway/v1/models"),
            (Some("sk-ah-wrong"), "/gateway/v1/models"), (Some(GATEWAY_KEY), "/v1/models")] {
            let mut request = client.get(format!("{base}{path}"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            assert_eq!(request.send().await.unwrap().status(), 401, "{token:?} at {path}");
        }

        let responses: Value = client.post(format!("{base}/gateway/v1/responses")).bearer_auth(GATEWAY_KEY)
            .json(&json!({"model":"relay-chat/vendor/deep-model","input":"hi","stream":false}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(responses["object"], "response", "{responses}");
        assert!(responses.to_string().contains("hello through the gateway"));
        let sent = upstream.seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&sent.body).unwrap()["model"], "vendor/deep-model");
        assert_eq!(sent.headers["authorization"], "Bearer sk-relay-secret");
        assert!(!sent.headers.values().any(|value| value.contains(GATEWAY_KEY)), "the gateway key never leaves");

        let messages = client.post(format!("{base}/gateway/v1/messages")).header("x-api-key", GATEWAY_KEY)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({"model":"relay-chat/chat-model","max_tokens":64,"messages":[{"role":"user","content":"hi"}]}))
            .send().await.unwrap();
        assert_eq!(messages.status(), 200);
        let messages: Value = messages.json().await.unwrap();
        assert_eq!(messages["type"], "message", "{messages}");
        assert_eq!(messages["content"][0]["text"], "hello through the gateway");

        let pooled = client.post(format!("{base}/gateway/v1/chat/completions")).bearer_auth(GATEWAY_KEY)
            .json(&json!({"model":"harbor-pool","messages":[{"role":"user","content":"hi"}]}))
            .send().await.unwrap();
        assert_eq!(pooled.status(), 200);
        assert_eq!(pooled.json::<Value>().await.unwrap()["choices"][0]["message"]["content"], "hello through the gateway");

        for model in ["relay-chat/excluded-model", "chat-model", "nobody/chat-model"] {
            let unknown = client.post(format!("{base}/gateway/v1/chat/completions")).bearer_auth(GATEWAY_KEY)
                .json(&json!({"model":model,"messages":[{"role":"user","content":"hi"}]})).send().await.unwrap();
            assert_eq!(unknown.status(), 404, "{model}");
            assert!(unknown.text().await.unwrap().contains("/gateway/v1/models"));
        }

        let traced = route_trace::recent().into_iter()
            .find(|route| route.requested_model == "relay-chat/vendor/deep-model").expect("traced gateway request");
        assert_eq!(traced.client_source.as_deref(), Some("gateway:Hermes"));
        stop_unchecked();
    });
}
