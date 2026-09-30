//! Retry only a new, identified, stateless turn before any output is exposed.
//! Tool/history turns never retry; an existing conversation binding always wins.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

const KEEP: usize = 1024;
#[derive(Clone)]
pub(super) struct Binding {
    pub provider_id: String,
    model: String,
    protocol: router::ModelProtocol,
    endpoint: String,
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
    AFFINITY
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, id)| id.clone())
}
fn endpoint(provider: &router::Provider) -> String {
    let mut hash = Sha256::new();
    for s in [&provider.base_url, &provider.api_key] {
        hash.update((s.len() as u64).to_le_bytes());
        hash.update(s.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
pub(super) fn agrees(binding: &Binding, destination: &Destination) -> bool {
    matches!(destination, Destination::Provider { provider, model_override, upstream_protocol, .. }
        if provider.id == binding.provider_id && model_override.as_deref().unwrap_or(&provider.model) == binding.model && *upstream_protocol == binding.protocol && endpoint(provider) == binding.endpoint)
}
pub(super) fn bind(key: &str, destination: &Destination) -> bool {
    let Destination::Provider {
        provider,
        model_override,
        upstream_protocol,
        ..
    } = destination
    else {
        return false;
    };
    let mut cache = AFFINITY
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Never evict a live binding and silently reassign a later tool turn. At
    // capacity, new sessions simply cannot opt into automatic failover.
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
            protocol: *upstream_protocol,
            endpoint: endpoint(provider),
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
pub(super) fn alternative(
    store: &router::Store,
    current: &Destination,
    spoken: Option<router::ModelProtocol>,
) -> Option<Destination> {
    let Destination::Provider {
        provider,
        model_override,
        upstream_protocol,
        ..
    } = current
    else {
        return None;
    };
    let model = model_override.as_deref().unwrap_or(&provider.model);
    // Exact upstream ID AND protocol, never the looser capability lookup key.
    store
        .providers
        .iter()
        .filter(|p| {
            p.id != provider.id
                && (p.model == model || p.models.iter().any(|m| m.id == model && m.included))
                && p.can_route_model(model)
                && p.protocol_for_model(model) == *upstream_protocol
        })
        .find_map(|p| provider_destination(store, &p.id, spoken, Some(model.into())).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn candidates_require_exact_upstream_identity() {
        let a = router::Provider {
            id: "a".into(),
            model: "claude-opus-5-5-kiro".into(),
            enabled: true,
            api_key: "test".into(),
            ..Default::default()
        };
        let mut b = a.clone();
        b.id = "b".into();
        b.model = "claude-opus-5-5".into();
        let mut store = router::Store {
            providers: vec![a, b],
            ..Default::default()
        };
        let first = provider_destination(
            &store,
            "a",
            Some(router::ModelProtocol::AnthropicMessages),
            None,
        )
        .unwrap();
        assert!(alternative(
            &store,
            &first,
            Some(router::ModelProtocol::AnthropicMessages)
        )
        .is_none());
        store.providers[1].model = "claude-opus-5-5-kiro".into();
        assert!(alternative(
            &store,
            &first,
            Some(router::ModelProtocol::AnthropicMessages)
        )
        .is_some());
    }
}
