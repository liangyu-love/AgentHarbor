//! Ordinary single-provider session identity protection. No hidden alternatives.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const KEEP: usize = 1024;
const IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
#[derive(Clone)]
pub(super) struct Binding {
    pub provider_id: String,
    pub(super) model: String,
    protocol: router::ModelProtocol,
    endpoint: String,
    last_used: Instant,
}
static AFFINITY: OnceLock<Mutex<VecDeque<(String, Binding)>>> = OnceLock::new();
// Serialize route decisions through response headers, not the long-lived stream.
// Prevents concurrent requests for one session racing its first successful route.
type Decisions = std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>;
static DECISIONS: OnceLock<Mutex<Decisions>> = OnceLock::new();
pub(super) async fn decision_lock(key: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = DECISIONS
            .get_or_init(|| Mutex::new(Decisions::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(key).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key.into(), Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}
pub(super) fn key(scope: &std::path::Path, headers: &HeaderMap, model: &str) -> Option<String> {
    let session = [
        "session_id",
        "x-session-id",
        "x-codex-session-id",
        "conversation_id",
        "x-opencode-session",
    ]
    .iter()
    .find_map(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 256)
    })?;
    let mut hash = Sha256::new();
    for field in [scope.to_string_lossy().as_ref(), model, session] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    Some(format!("{:x}", hash.finalize()))
}
pub(super) fn bound(key: &str) -> Option<Binding> {
    let mut cache = AFFINITY
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache.retain(|(_, binding)| binding.last_used.elapsed() < IDLE_TTL);
    let (_, binding) = cache.iter_mut().find(|(k, _)| k == key)?;
    binding.last_used = Instant::now();
    Some(binding.clone())
}
#[cfg(test)]
fn release(key: &str) {
    AFFINITY
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| k != key);
}
fn endpoint(provider: &router::Provider) -> String {
    let mut hash = Sha256::new();
    for s in [
        &provider.base_url,
        &provider.model_credential().api_key.to_owned(),
    ] {
        hash.update((s.len() as u64).to_le_bytes());
        hash.update(s.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
pub(super) fn agrees(binding: &Binding, destination: &Destination) -> bool {
    matches!(destination, Destination::Provider { provider, model_override, .. }
        if provider.id == binding.provider_id && model_override.as_deref().unwrap_or(&provider.model) == binding.model && provider.protocol_for_model(model_override.as_deref().unwrap_or(&provider.model)) == binding.protocol && endpoint(provider) == binding.endpoint)
}
pub(super) fn bind(key: &str, destination: &Destination) -> bool {
    let Destination::Provider {
        provider,
        model_override,
        ..
    } = destination
    else {
        return false;
    };
    let mut cache = AFFINITY
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache.retain(|(_, binding)| binding.last_used.elapsed() < IDLE_TTL);
    // Never evict a live binding and silently reassign a later tool turn. At
    // capacity, the caller must reject a new session before contacting upstream.
    if cache.len() >= KEEP && !cache.iter().any(|(k, _)| k == key) {
        return false;
    }
    cache.retain(|(k, _)| k != key);
    cache.push_back((
        key.into(),
        Binding {
            provider_id: provider.id.clone(),
            model: model_override
                .clone()
                .unwrap_or_else(|| provider.model.clone()),
            protocol: provider
                .protocol_for_model(model_override.as_deref().unwrap_or(&provider.model)),
            endpoint: endpoint(provider),
            last_used: Instant::now(),
        },
    ));
    true
}
pub(super) fn stateless(body: Option<&[u8]>) -> bool {
    let Some(v) = body
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        .filter(Value::is_object)
    else {
        return false;
    };
    if ["previous_response_id", "conversation", "background"]
        .iter()
        .any(|k| {
            v.get(*k)
                .is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
        })
    {
        return false;
    }
    if v.get("tools")
        .is_some_and(|t| t.as_array().is_none_or(|a| !a.is_empty()))
    {
        return false;
    }
    for key in ["input", "messages"] {
        if let Some(items) = v.get(key).and_then(Value::as_array) {
            if items.iter().any(|i| {
                !matches!(i["role"].as_str(), Some("user" | "system" | "developer"))
                    || i.get("tool_calls").is_some()
                    || i.get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|p| {
                                matches!(p["type"].as_str(), Some("tool_use" | "tool_result"))
                            })
                        })
            }) {
                return false;
            }
        }
    }
    true
}
pub(super) fn continuation(body: Option<&[u8]>) -> bool {
    let Some(mut v) = body.and_then(|b| serde_json::from_slice::<Value>(b).ok()) else {
        return false;
    };
    if let Some(object) = v.as_object_mut() {
        object.remove("tools");
        object.remove("background");
    }
    !stateless(Some(v.to_string().as_bytes()))
}
pub(super) fn fresh_turn(body: Option<&[u8]>) -> bool {
    body.and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .is_some_and(|value| value.is_object())
        && !continuation(body)
}
#[cfg(test)]
pub(super) fn expire(key: &str) {
    if let Some((_, binding)) = AFFINITY
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .iter_mut()
        .find(|(k, _)| k == key)
    {
        binding.last_used = Instant::now() - IDLE_TTL - Duration::from_secs(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_bindings_release_capacity_and_active_sessions_refresh_their_idle_time() {
        let _guard = global_router_test_lock();
        let provider = router::Provider {
            id: "expiry".into(),
            enabled: true,
            api_key: "synthetic".into(),
            model: "m".into(),
            ..Default::default()
        };
        let store = router::Store {
            providers: vec![provider],
            ..Default::default()
        };
        let destination = provider_destination(
            &store,
            "expiry",
            Some(router::ModelProtocol::Responses),
            None,
        )
        .unwrap();
        let previous = std::mem::take(
            &mut *AFFINITY
                .get_or_init(|| Mutex::new(VecDeque::new()))
                .lock()
                .unwrap(),
        );
        for i in 0..KEEP {
            assert!(bind(&format!("idle-{i}"), &destination));
        }
        assert!(!bind("overflow", &destination));
        expire("idle-0");
        {
            let mut cache = AFFINITY.get().unwrap().lock().unwrap();
            cache[1].1.last_used = Instant::now() - Duration::from_secs(60);
        }
        assert!(bound("idle-0").is_none());
        assert!(bound("idle-1").unwrap().last_used.elapsed() < Duration::from_secs(1));
        assert!(bind("new-session", &destination));
        release("new-session");
        assert!(bound("new-session").is_none());
        *AFFINITY.get().unwrap().lock().unwrap() = previous;
    }
    #[test]
    fn full_registry_rejects_new_sessions_before_upstream_and_keeps_existing_bindings() {
        use super::super::routing_safety_tests::{mock, persist, RESPONSE};
        use std::sync::atomic::Ordering;
        let _guard = global_router_test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (url, calls, _, server) = mock(200, RESPONSE, "application/json").await;
            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().into());
            let store = router::Store {
                providers: vec![router::Provider {
                    id: "a".into(), base_url: url, model: "gpt-6-astra".into(),
                    api_key: "synthetic".into(), enabled: true, ..Default::default()
                }],
                router: router::RouterSettings {
                    bearer_token: "local".into(), ..Default::default()
                },
                ..Default::default()
            };
            persist(&paths, &store);
            let destination = provider_destination(&store, "a", Some(router::ModelProtocol::Responses), None).unwrap();
            let mut headers = HeaderMap::new();
            headers.insert("session_id", HeaderValue::from_static("existing"));
            let existing_key = key(&paths.app_data_dir.join("newapi-providers.dat"), &headers, "a::gpt-6-astra").unwrap();
            let previous = std::mem::take(&mut *AFFINITY.get_or_init(|| Mutex::new(VecDeque::new())).lock().unwrap());
            assert!(bind(&existing_key, &destination));
            for i in 1..KEEP { assert!(bind(&format!("capacity-{i}"), &destination)); }
            let port = ensure_running(0, paths, ApiProxyConfigPayload {
                mode: super::super::super::models::ApiProxyMode::Direct, url: None,
            }).await.unwrap();
            let client = Client::builder().no_proxy().build().unwrap();
            // Codex resends full history and needs no binding; other clients do.
            let url = format!("http://127.0.0.1:{port}/p/a/v1/responses");
            let response = client.post(&url).bearer_auth("local").header("session_id", "new")
                .json(&json!({"model":"a::gpt-6-astra", "input":"hello"})).send().await.unwrap();
            let new_status = response.status();
            let new_error = response.text().await.unwrap();
            let new_calls = calls.load(Ordering::SeqCst);
            let response = client.post(&url).bearer_auth("local").header("session_id", "existing")
                .json(&json!({"model":"a::gpt-6-astra", "previous_response_id":"response-test", "input":"next"})).send().await.unwrap();
            let existing_status = response.status();
            response.bytes().await.unwrap();
            stop(); server.abort();
            *AFFINITY.get().unwrap().lock().unwrap() = previous;
            assert_eq!(new_status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(new_error.contains("capacity reached"));
            assert_eq!(new_calls, 0, "A rejected first turn must not spend upstream quota");
            assert_eq!(existing_status, StatusCode::OK);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        });
    }
    #[test]
    fn tools_and_continuations_cannot_be_replayed() {
        for v in [
            json!({"previous_response_id":"r"}),
            json!({"tools":[{"type":"function"}]}),
            json!({"input":[{"type":"function_call_output","output":"done"}]}),
            json!({"messages":[{"role":"assistant","content":"earlier"}]}),
            json!({"messages":[{"role":"user","content":[{"type":"tool_result"}]}]}),
        ] {
            assert!(!stateless(Some(v.to_string().as_bytes())));
        }
        assert!(stateless(Some(br#"{"input":"hello"}"#)));
    }
}
