//! Bounded, memory-only diagnostics. No bodies, headers, URLs or remote error text.
use super::request_diagnostics::{CredentialSnapshot, ServedModelObservation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

const KEEP: usize = 60;
const MAX_EVENT: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub attempt_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub upstream_model: String,
    pub matched_model: String,
    pub protocol: String,
    pub sent: Value,
    pub status: Option<u16>,
    pub failure_stage: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    /// Metadata the request used at dispatch time: the catalog revision and
    /// resolver match kind never change with a later catalog refresh.
    pub catalog_revision: String,
    pub match_kind: String,
    pub pricing_status: String,
    pub pricing_source: String,
    pub selection_reason: String,
    pub retry_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_plan: Option<String>,
    pub credential: CredentialSnapshot,
    pub model_observation: ServedModelObservation,
    #[serde(skip)]
    started: Instant,
}

/// Metadata the request used at dispatch time: the catalog revision and
/// resolver match kind never change with a later catalog refresh. Built by the
/// proxy from the pricing snapshot so diagnostics do not depend on the ledger.
#[derive(Clone, Default)]
pub struct AttemptMetadata {
    pub attempt_id: String,
    pub catalog_revision: String,
    pub match_kind: String,
    pub pricing_status: String,
    pub pricing_source: String,
    pub selection_reason: String,
    pub retry_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub id: String,
    pub timestamp: i64,
    pub requested_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_revision: Option<u64>,
    pub requested: Value,
    pub attempts: Vec<Attempt>,
    pub skipped_members: Vec<SkippedMember>,
    pub stage: String,
    pub done: bool,
    pub status: Option<u16>,
    pub failure_stage: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub first_token_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedMember {
    pub member_id: String,
    pub reason: String,
}
fn routes() -> &'static Mutex<VecDeque<Route>> {
    static ROUTES: OnceLock<Mutex<VecDeque<Route>>> = OnceLock::new();
    ROUTES.get_or_init(|| Mutex::new(VecDeque::new()))
}
pub fn recent() -> Vec<Route> {
    routes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .cloned()
        .collect()
}
fn label(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

#[derive(Default, Deserialize)]
struct Effort {
    effort: Option<String>,
}
#[derive(Default, Deserialize)]
struct Thinking {
    r#type: Option<String>,
    budget_tokens: Option<u64>,
}
#[derive(Default, Deserialize)]
struct RequestMetadata {
    model: Option<String>,
    reasoning: Option<Effort>,
    reasoning_effort: Option<String>,
    thinking: Option<Thinking>,
    output_config: Option<Effort>,
    max_tokens: Option<u64>,
    max_output_tokens: Option<u64>,
    max_completion_tokens: Option<u64>,
}
fn metadata(body: Option<&[u8]>) -> RequestMetadata {
    // Serde skips unknown fields without materializing prompts/history in a Value.
    body.and_then(|b| serde_json::from_slice(b).ok())
        .unwrap_or_default()
}

/// Excludes reasoning summaries, thinking text and arbitrary keys.
pub(crate) fn parameters(body: Option<&[u8]>) -> Value {
    metadata_parameters(&metadata(body))
}
fn metadata_parameters(value: &RequestMetadata) -> Value {
    let mut out = Map::new();
    for (name, text, allowed) in [
        (
            "reasoning.effort",
            value.reasoning.as_ref().and_then(|r| r.effort.as_deref()),
            &[
                "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ][..],
        ),
        (
            "reasoning_effort",
            value.reasoning_effort.as_deref(),
            &[
                "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ][..],
        ),
        (
            "thinking.type",
            value.thinking.as_ref().and_then(|r| r.r#type.as_deref()),
            &["enabled", "disabled", "adaptive"][..],
        ),
        (
            "output_config.effort",
            value
                .output_config
                .as_ref()
                .and_then(|r| r.effort.as_deref()),
            &["low", "medium", "high", "max"][..],
        ),
    ] {
        if let Some(text) = text {
            out.insert(
                name.into(),
                Value::String(
                    if allowed.contains(&text) {
                        text
                    } else {
                        "[unrecognized]"
                    }
                    .into(),
                ),
            );
        }
    }
    for (name, number) in [
        (
            "thinking.budget_tokens",
            value.thinking.as_ref().and_then(|r| r.budget_tokens),
        ),
        ("max_tokens", value.max_tokens),
        ("max_output_tokens", value.max_output_tokens),
        ("max_completion_tokens", value.max_completion_tokens),
    ] {
        if let Some(n) = number {
            out.insert(name.into(), n.into());
        }
    }
    Value::Object(out)
}

struct Inner {
    id: String,
    started: Instant,
}
#[derive(Clone)]
pub(crate) struct Trace(Arc<Inner>);
impl Trace {
    pub fn request_id(&self) -> &str {
        &self.0.id
    }

    pub fn begin(model: &str, body: Option<&[u8]>) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = metadata(body);
        let model = raw.model.as_deref().unwrap_or(model);
        let mut routes = routes().lock().unwrap_or_else(|e| e.into_inner());
        routes.push_back(Route {
            id: id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
            requested_model: label(model),
            client_source: None,
            resolved_model: None,
            route_revision: None,
            requested: metadata_parameters(&raw),
            attempts: vec![],
            skipped_members: vec![],
            stage: "route".into(),
            done: false,
            status: None,
            failure_stage: None,
            error: None,
            duration_ms: 0,
            first_token_ms: None,
        });
        while routes.len() > KEEP {
            routes.pop_front();
        }
        Self(Arc::new(Inner {
            id,
            started: Instant::now(),
        }))
    }
    fn update(&self, f: impl FnOnce(&mut Route)) {
        if let Some(route) = routes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
            .find(|r| r.id == self.0.id)
        {
            route.duration_ms = self.0.started.elapsed().as_millis() as u64;
            f(route);
        }
    }
    pub fn stage(&self, stage: &str) {
        self.update(|r| r.stage = stage.into());
    }
    pub fn identity(&self, identity: &super::request_diagnostics::RequestIdentity) {
        self.update(|r| {
            r.client_source = identity.client_source.as_deref().map(label);
            r.resolved_model = identity.resolved_model.as_deref().map(label);
            r.route_revision = identity.route_revision;
        });
    }
    pub fn attempt(
        &self,
        provider_id: &str,
        provider_name: &str,
        model: &str,
        protocol: &str,
        metadata: AttemptMetadata,
    ) {
        self.update(|r| {
            r.stage = "build".into();
            r.attempts.push(Attempt {
                attempt_id: metadata.attempt_id,
                provider_id: label(provider_id),
                provider_name: label(provider_name),
                upstream_model: label(model),
                matched_model: label(&super::model_defaults::model_key(model)),
                protocol: protocol.into(),
                sent: Value::Object(Map::new()),
                status: None,
                failure_stage: None,
                error: None,
                duration_ms: 0,
                catalog_revision: metadata.catalog_revision,
                match_kind: metadata.match_kind,
                pricing_status: metadata.pricing_status,
                pricing_source: metadata.pricing_source,
                selection_reason: metadata.selection_reason,
                retry_reason: metadata.retry_reason,
                aggregate_id: None,
                route_revision: None,
                member_id: None,
                configured_protocol: None,
                effective_protocol: None,
                compatibility_plan: None,
                credential: CredentialSnapshot::not_sent(),
                model_observation: ServedModelObservation {
                    model_match_status: "unknown".into(),
                    unknown_reason: Some("not_sent".into()),
                    ..Default::default()
                },
                started: Instant::now(),
            });
        });
    }
    pub fn sent(&self, body: Option<&[u8]>, credential: CredentialSnapshot) {
        self.update(|r| {
            r.stage = "connect".into();
            if let Some(a) = r.attempts.last_mut() {
                a.sent = parameters(body);
                a.credential = credential;
                a.model_observation.unknown_reason = Some("response_not_received".into());
            }
        });
    }
    pub fn aggregate_attempt(
        &self,
        aggregate_id: &str,
        member_id: &str,
        revision: &str,
        configured: &str,
        effective: &str,
        compatibility: Option<&str>,
    ) {
        self.update(|route| {
            if let Some(attempt) = route.attempts.last_mut() {
                attempt.aggregate_id = Some(label(aggregate_id));
                attempt.route_revision = Some(label(revision));
                attempt.member_id = Some(label(member_id));
                attempt.configured_protocol = Some(label(configured));
                attempt.effective_protocol = Some(label(effective));
                attempt.compatibility_plan = compatibility.map(label);
            }
        });
    }
    pub fn skip_member(&self, member_id: &str, reason: &str) {
        self.update(|route| {
            if route.skipped_members.len() < 64 {
                route.skipped_members.push(SkippedMember {
                    member_id: label(member_id),
                    reason: label(reason),
                });
            }
        });
    }
    pub fn status(&self, status: u16) {
        self.update(|r| {
            r.status = Some(status);
            r.stage = "response".into();
            if let Some(a) = r.attempts.last_mut() {
                a.status = Some(status);
            }
        });
    }
    pub fn failed_attempt(&self, stage: &str, code: &str, millis: u64) {
        self.update(|r| {
            if let Some(a) = r.attempts.last_mut() {
                a.failure_stage = Some(stage.into());
                a.error = Some(code.into());
                a.duration_ms = millis;
                a.model_observation.unknown_reason = Some(if stage == "headers" {
                    "upstream_http_error".into()
                } else {
                    "response_not_received".into()
                });
            }
        });
    }
    pub fn fail(&self, stage: &str, code: &str) {
        self.update(|r| {
            if r.failure_stage.is_none() {
                r.done = true;
                r.failure_stage = Some(stage.into());
                r.error = Some(code.into());
                if let Some(a) = r.attempts.last_mut() {
                    a.failure_stage = Some(stage.into());
                    a.error = Some(code.into());
                }
            }
        });
    }
    pub fn reject(&self, status: u16, code: &str) {
        self.update(|r| {
            if !r.done {
                r.status = Some(status);
                r.done = true;
                r.failure_stage = Some(r.stage.clone());
                r.error = Some(code.into());
                if let Some(a) = r.attempts.last_mut() {
                    a.failure_stage = r.failure_stage.clone();
                    a.error = r.error.clone();
                }
            }
        });
    }
    pub fn finish(&self) {
        self.update(|r| {
            if !r.done {
                r.done = true;
                r.stage = "complete".into();
            }
        });
    }
    pub fn attempt_elapsed(&self) {
        self.update(|r| {
            if let Some(a) = r.attempts.last_mut() {
                a.duration_ms = a.started.elapsed().as_millis() as u64;
            }
        });
    }
    pub fn observed(&self, attempt_id: &str, observation: ServedModelObservation) {
        self.update(|route| {
            if let Some(attempt) = route
                .attempts
                .iter_mut()
                .find(|a| a.attempt_id == attempt_id)
            {
                attempt.model_observation = observation;
            }
        });
    }
    fn first_token(&self) {
        self.update(|r| {
            if r.first_token_ms.is_none() {
                r.first_token_ms = Some(r.duration_ms);
            }
        });
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(r) = routes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
            .find(|r| r.id == self.id)
        {
            if !r.done {
                r.done = true;
                r.failure_stage = Some(r.stage.clone());
                r.error = Some("interrupted".into());
                r.duration_ms = self.started.elapsed().as_millis() as u64;
            }
        }
    }
}

pub(crate) struct Observation {
    trace: Trace,
    sse: bool,
    pending: Vec<u8>,
    oversized: bool,
    terminal: bool,
    success: bool,
}
impl Observation {
    pub fn new(trace: Trace, sse: bool, status: u16) -> Self {
        trace.status(status);
        Self {
            trace,
            sse,
            pending: Vec::new(),
            oversized: false,
            terminal: false,
            success: (200..300).contains(&status),
        }
    }
    pub fn push(&mut self, bytes: &[u8]) {
        if !self.success {
            return;
        }
        if !self.sse {
            if self.pending.len().saturating_add(bytes.len()) <= MAX_EVENT && !self.oversized {
                self.pending.extend_from_slice(bytes);
            } else {
                self.pending.clear();
                self.oversized = true;
            }
            return;
        }
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            if !self.oversized && self.pending.len() + part.len() <= MAX_EVENT {
                self.pending.extend_from_slice(part);
            } else {
                self.pending.clear();
                self.oversized = true;
            }
            if part.last() == Some(&b'\n') {
                if !self.oversized {
                    let line = std::mem::take(&mut self.pending);
                    self.line(&line);
                }
                self.oversized = false;
            }
        }
    }
    fn line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        let Some(data) = text.trim().strip_prefix("data:") else {
            return;
        };
        if data.trim() == "[DONE]" {
            self.terminal = true;
            return;
        }
        if let Ok(value) = serde_json::from_str(data.trim()) {
            self.value(&value);
        }
    }
    fn value(&mut self, v: &Value) {
        let kind = v["type"].as_str().unwrap_or_default();
        if !v["error"].is_null()
            || matches!(kind, "error" | "response.failed")
            || v["status"] == "failed"
        {
            self.trace.fail("response", "upstream_error");
        }
        if matches!(
            kind,
            "response.completed" | "response.incomplete" | "message_stop"
        ) {
            self.terminal = true;
        }
        let text = |path| {
            v.pointer(path)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        };
        if (matches!(
            kind,
            "response.output_text.delta"
                | "response.reasoning_text.delta"
                | "response.reasoning_summary_text.delta"
                | "response.function_call_arguments.delta"
        ) && text("/delta"))
            || (kind == "content_block_delta"
                && ["/delta/text", "/delta/thinking", "/delta/partial_json"]
                    .iter()
                    .any(|p| text(p)))
            || (kind == "content_block_start"
                && v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use"))
            || (kind == "response.output_item.added"
                && v.pointer("/item/type").and_then(Value::as_str) == Some("function_call"))
            || [
                "/choices/0/delta/content",
                "/choices/0/delta/reasoning_content",
            ]
            .iter()
            .any(|p| text(p))
            || v.pointer("/choices/0/delta/tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        {
            self.trace.first_token();
        }
    }
    pub fn error(&mut self) {
        self.trace.fail("stream", "upstream_stream_error");
    }
    pub fn finish(&mut self) {
        if self.sse {
            let tail = std::mem::take(&mut self.pending);
            if !self.oversized {
                self.line(&tail);
            }
        } else if let Ok(v) = serde_json::from_slice::<Value>(&self.pending) {
            self.value(&v);
        }
        if !self.success {
            self.trace.fail("headers", "upstream_http_error");
        } else if self.sse && !self.terminal {
            self.trace.fail("stream", "incomplete_stream");
        } else {
            self.trace.finish();
        }
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        self.trace.attempt_elapsed();
        if self.terminal {
            self.trace.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_exclude_body_and_secrets() {
        let body = serde_json::json!({"input":"secret prompt","api_key":"sk-secret","reasoning":{"effort":"high","summary":"private"},"thinking":{"type":"enabled","budget_tokens":4096,"text":"private"}});
        assert_eq!(
            parameters(Some(body.to_string().as_bytes())),
            serde_json::json!({"reasoning.effort":"high","thinking.type":"enabled","thinking.budget_tokens":4096})
        );
    }
    #[test]
    fn ttft_ignores_role_frames_and_detects_reasoning() {
        let trace = Trace::begin("trace-test", None);
        let mut obs = Observation::new(trace.clone(), true, 200);
        obs.push(b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n");
        assert!(recent()
            .iter()
            .find(|r| r.id == trace.0.id)
            .unwrap()
            .first_token_ms
            .is_none());
        obs.push(
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"thinking\":\"hello\"}}\n\n",
        );
        obs.finish();
        let r = recent().into_iter().find(|r| r.id == trace.0.id).unwrap();
        assert!(r.first_token_ms.is_some());
        assert_eq!(r.error.as_deref(), Some("incomplete_stream"));
    }
}
