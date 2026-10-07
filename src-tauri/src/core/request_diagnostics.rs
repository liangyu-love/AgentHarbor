//! Bounded observations of the original upstream response and dispatch credential.
//! These values are diagnostic only; they never select a route or change a body.
use super::{protocol::canonical::TerminalObservation, router::ModelProtocol, secret_file};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

const MAX_OBSERVATION: usize = 256 * 1024;
const MAX_ID: usize = 200;

/// Entry-point identity supplements the actual model used for accounting. Old
/// records omit these fields and retain their original unknown source.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServedModelObservation {
    pub served_model: Option<String>,
    pub model_match_status: String,
    pub model_match_basis: Option<String>,
    pub unknown_reason: Option<String>,
    pub conflicting_model: Option<String>,
}

impl Default for ServedModelObservation {
    fn default() -> Self {
        Self {
            served_model: None,
            model_match_status: "unknown".into(),
            model_match_basis: None,
            unknown_reason: Some("not_recorded".into()),
            conflicting_model: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CredentialSnapshot {
    pub kind: String,
    pub status: String,
    pub reason: Option<String>,
    pub version_fingerprint: Option<String>,
    pub account_id: Option<String>,
    pub account_status: String,
    pub label: Option<String>,
}

impl Default for CredentialSnapshot {
    fn default() -> Self {
        Self {
            kind: "unknown".into(),
            status: "not_recorded".into(),
            reason: Some("not_recorded".into()),
            version_fingerprint: None,
            account_id: None,
            account_status: "not_recorded".into(),
            label: None,
        }
    }
}

impl CredentialSnapshot {
    pub fn not_sent() -> Self {
        Self {
            kind: "unknown".into(),
            status: "not_sent".into(),
            reason: Some("dispatch_not_completed".into()),
            ..Self::default()
        }
    }

    /// Fingerprints the actual outbound header with an application-local key.
    /// A missing or unreadable key never becomes an unkeyed hash.
    pub fn from_header(
        app_data_dir: &Path,
        kind: &str,
        scope: &str,
        header: Option<&str>,
        account: Option<&str>,
    ) -> Self {
        let mut snapshot = Self {
            kind: kind.into(),
            status: "not_recorded".into(),
            reason: Some("credential_unavailable".into()),
            ..Self::default()
        };
        let Some(header) = header.filter(|value| !value.is_empty()) else {
            return snapshot;
        };
        let Some(key) = fingerprint_key(app_data_dir) else {
            snapshot.reason = Some("protected_key_unavailable".into());
            return snapshot;
        };
        let mut scoped_header = Vec::with_capacity(scope.len() + header.len() + 1);
        scoped_header.extend_from_slice(scope.as_bytes());
        scoped_header.push(0);
        scoped_header.extend_from_slice(header.as_bytes());
        snapshot.version_fingerprint = Some(fingerprint(&key, kind.as_bytes(), &scoped_header));
        snapshot.account_id = account
            .filter(|value| !value.is_empty())
            .map(|value| fingerprint(&key, b"codex-account", value.as_bytes()));
        snapshot.account_status = if snapshot.account_id.is_some() {
            "recorded"
        } else {
            "not_recorded"
        }
        .into();
        snapshot.status = "recorded".into();
        snapshot.reason = None;
        snapshot
    }
}

fn fingerprint_key(app_data_dir: &Path) -> Option<Vec<u8>> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().ok()?;
    let path = app_data_dir.join("diagnostic-fingerprint.key");
    match secret_file::read(&path) {
        Ok(Some(key)) if key.len() == 32 => Some(key),
        Ok(Some(_)) | Err(_) => None,
        Ok(None) => {
            let mut key = Vec::with_capacity(32);
            key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
            key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
            secret_file::write(&path, &key).ok()?;
            Some(key)
        }
    }
}

fn fingerprint(key: &[u8], domain: &[u8], value: &[u8]) -> String {
    // HMAC-SHA256, truncated to 96 bits for display. Domain separation keeps
    // account identifiers distinct from credential versions.
    let mut ipad = [0x36_u8; 64];
    let mut opad = [0x5c_u8; 64];
    for (index, byte) in key.iter().enumerate() {
        ipad[index] ^= byte;
        opad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(domain);
    inner.update([0]);
    inner.update(value);
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner.finalize());
    let digest = outer.finalize();
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Opaque process-scoped revision for compare-before-apply. The store may
/// contain API keys, so its raw digest must never be sent to the frontend.
pub fn state_revision(bytes: &[u8]) -> String {
    static KEY: OnceLock<Vec<u8>> = OnceLock::new();
    let key = KEY.get_or_init(|| {
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        key
    });
    fingerprint(key, b"config-revision", bytes)
}

pub struct ServedModelObserver {
    protocol: ModelProtocol,
    upstream_model: String,
    request_model_issue: Option<&'static str>,
    status: u16,
    sse: bool,
    at_start: bool,
    buffer: Vec<u8>,
    event_data: Vec<u8>,
    event_name: String,
    served: Option<String>,
    conflict: Option<String>,
    reason: Option<&'static str>,
    terminal: TerminalObservation,
    oversized: bool,
    final_result: Option<ServedModelObservation>,
    response_error: Value,
}

impl ServedModelObserver {
    pub fn new(protocol: ModelProtocol, upstream_model: &str, status: u16, sse: bool) -> Self {
        let request_model_issue = if upstream_model.is_empty() {
            Some("request_model_missing")
        } else if upstream_model.len() > MAX_ID || upstream_model.chars().any(char::is_control) {
            Some("request_model_truncated")
        } else {
            None
        };
        Self {
            protocol,
            upstream_model: if request_model_issue.is_none() {
                upstream_model.into()
            } else {
                String::new()
            },
            request_model_issue,
            status,
            sse,
            at_start: true,
            buffer: Vec::new(),
            event_data: Vec::new(),
            event_name: String::new(),
            served: None,
            conflict: None,
            reason: None,
            terminal: TerminalObservation::default(),
            oversized: false,
            final_result: None,
            response_error: Value::Null,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.final_result.is_some() || !(200..300).contains(&self.status) || self.oversized {
            return;
        }
        if !self.sse {
            if self.buffer.len().saturating_add(bytes.len()) > MAX_OBSERVATION {
                self.oversized = true;
                self.buffer.clear();
            } else {
                self.buffer.extend_from_slice(bytes);
            }
            return;
        }
        for byte in bytes {
            if self.buffer.len() + self.event_data.len() >= MAX_OBSERVATION {
                self.oversized = true;
                self.buffer.clear();
                self.event_data.clear();
                break;
            }
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.buffer);
                self.line(line.strip_suffix(b"\r").unwrap_or(&line));
            } else {
                self.buffer.push(*byte);
            }
        }
    }

    fn line(&mut self, line: &[u8]) {
        let line = super::protocol::canonical::strip_stream_bom(line, &mut self.at_start);
        if line.is_empty() {
            self.event();
            return;
        }
        if line.starts_with(b":") {
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            self.reason = Some("invalid_utf8");
            return;
        };
        if let Some(data) = text.strip_prefix("data:") {
            if !self.event_data.is_empty() {
                self.event_data.push(b'\n');
            }
            self.event_data
                .extend_from_slice(data.strip_prefix(' ').unwrap_or(data).as_bytes());
        } else if let Some(name) = text.strip_prefix("event:") {
            self.event_name = name.trim().chars().take(64).collect();
        }
    }

    fn event(&mut self) {
        let data = std::mem::take(&mut self.event_data);
        let name = std::mem::take(&mut self.event_name);
        if data.is_empty() {
            return;
        }
        if data == b"[DONE]" {
            self.terminal.done();
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&data) else {
            self.reason = Some("invalid_json");
            return;
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or(&name);
        if let Some(error) = value
            .get("error")
            .filter(|error| !error.is_null())
            .or_else(|| value.pointer("/response/error").filter(|error| !error.is_null()))
            .or_else(|| (kind == "error").then_some(&value))
        {
            // Reuse the existing raw parse for channel-health classification.
            // Retain only bounded scalar error identifiers, never the message.
            self.response_error = serde_json::json!({});
            for key in ["type", "code", "status"] {
                if let Some(value) = error.get(key).filter(|value| {
                    value.as_u64().is_some() || value.as_str().is_some_and(|text| text.len() <= 128)
                }) {
                    self.response_error[key] = value.clone();
                }
            }
        }
        if self.sse {
            self.terminal.observe(self.protocol, &value, &name);
        } else {
            self.terminal.observe_body(self.protocol, &value);
        }
        if self.terminal.failed() {
            self.reason = Some("response_failed");
        }
        let declared = match self.protocol {
            ModelProtocol::Responses => {
                if !self.sse {
                    value
                        .get("model")
                        .or_else(|| value.pointer("/response/model"))
                        .and_then(Value::as_str)
                } else if kind.starts_with("response.") {
                    value.pointer("/response/model").and_then(Value::as_str)
                } else {
                    None
                }
            }
            ModelProtocol::ChatCompletions => value.get("model").and_then(Value::as_str),
            ModelProtocol::AnthropicMessages => {
                if kind == "message_start" {
                    value.pointer("/message/model").and_then(Value::as_str)
                } else if !self.sse {
                    value.get("model").and_then(Value::as_str)
                } else {
                    None
                }
            }
        };
        if let Some(declared) = declared {
            self.record_declaration(declared);
        }
        // Both positions are valid for a non-stream Responses body. If an
        // upstream emits both, do not silently prefer one conflicting name.
        if self.protocol == ModelProtocol::Responses && !self.sse {
            if let Some(nested) = value.pointer("/response/model").and_then(Value::as_str) {
                self.record_declaration(nested);
            }
        }
    }

    fn record_declaration(&mut self, declared: &str) {
        if !declared.is_empty() {
            if declared.len() > MAX_ID || declared.chars().any(char::is_control) {
                self.reason = Some("model_id_truncated");
                return;
            }
            match &self.served {
                Some(first) if first != declared => {
                    if self.conflict.is_none() {
                        self.conflict = Some(declared.into());
                    }
                    self.reason = Some("conflicting_declarations");
                }
                None => self.served = Some(declared.into()),
                _ => {}
            }
        }
    }

    pub fn error(&mut self) {
        self.reason = Some("upstream_stream_error");
    }

    pub fn response_error(&self) -> &Value {
        &self.response_error
    }

    /// Inspect after `finish(true)` so a final frame without trailing newlines
    /// has been consumed. An observation limit is unknown, not proof of EOF
    /// truncation, and an HTTP rejection is not a successful SSE response.
    pub fn stream_truncated(&self) -> bool {
        self.sse
            && (200..300).contains(&self.status)
            && !self.oversized
            && !self.terminal.received()
    }

    pub fn finish(&mut self, eof: bool) -> ServedModelObservation {
        if let Some(result) = &self.final_result {
            return result.clone();
        }
        {
            if self.sse {
                if eof && !self.buffer.is_empty() {
                    let line = std::mem::take(&mut self.buffer);
                    self.line(line.strip_suffix(b"\r").unwrap_or(&line));
                }
                if eof && !self.event_data.is_empty() {
                    self.event();
                }
            } else if !self.oversized {
                match serde_json::from_slice::<Value>(&self.buffer) {
                    Ok(value) => self.event_data = serde_json::to_vec(&value).unwrap_or_default(),
                    Err(_) => self.reason = Some("invalid_json"),
                }
                if !self.event_data.is_empty() {
                    self.event();
                }
            }
        }
        let reason = if !(200..300).contains(&self.status) {
            Some("upstream_http_error")
        } else if self.oversized {
            Some("observation_limit")
        } else if !eof {
            Some("stream_interrupted")
        } else if self.conflict.is_some() {
            Some("conflicting_declarations")
        } else if self.reason.is_some() {
            self.reason
        } else if self.sse && !self.terminal.received() {
            Some("incomplete_stream")
        } else if self.served.is_none() {
            Some("model_not_declared")
        } else if self.request_model_issue.is_some() {
            self.request_model_issue
        } else {
            None
        };
        let result = ServedModelObservation {
            served_model: self.served.clone(),
            model_match_status: match reason {
                Some(_) => "unknown",
                None if self.served.as_deref() == Some(self.upstream_model.as_str()) => "same",
                None => "declared_different",
            }
            .into(),
            model_match_basis: reason.is_none().then(|| {
                if self.served.as_deref() == Some(self.upstream_model.as_str()) {
                    "raw_exact".into()
                } else {
                    "raw_comparison".into()
                }
            }),
            unknown_reason: reason.map(str::to_owned),
            conflicting_model: self.conflict.clone(),
        };
        self.final_result = Some(result.clone());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_uses_only_protocol_fields_and_declared_difference_is_not_proof() {
        let mut observer =
            ServedModelObserver::new(ModelProtocol::Responses, "wire-v1", 200, false);
        observer.push(br#"{"model":"wire-v1","output":[{"model":"decoy"}]}"#);
        let result = observer.finish(true);
        assert_eq!(result.model_match_status, "same");
        assert_eq!(result.served_model.as_deref(), Some("wire-v1"));
        assert_eq!(observer.finish(false).model_match_status, "same");

        let mut observer =
            ServedModelObserver::new(ModelProtocol::Responses, "wire-v1", 200, false);
        observer.push(br#"{"response":{"model":"served-v2"}}"#);
        let result = observer.finish(true);
        assert_eq!(result.model_match_status, "declared_different");
        assert_eq!(result.served_model.as_deref(), Some("served-v2"));

        let mut conflict =
            ServedModelObserver::new(ModelProtocol::Responses, "wire-v1", 200, false);
        conflict.push(br#"{"model":"wire-v1","response":{"model":"served-v2"}}"#);
        assert_eq!(
            conflict.finish(true).unknown_reason.as_deref(),
            Some("conflicting_declarations")
        );
    }

    #[test]
    fn sse_joins_multiline_data_across_chunks_and_requires_completion() {
        let mut observer = ServedModelObserver::new(ModelProtocol::Responses, "wire", 200, true);
        observer.push(
            b": keepalive\r\nevent: response.created\r\ndata: {\"type\":\"response.created\",\r\n",
        );
        observer.push(b"data: \"response\":{\"model\":\"wire\"}}\r\n\r\n");
        observer.push(
            b"data: {\"type\":\"response.completed\",\"response\":{\"model\":\"wire\"}}\r\n\r\n",
        );
        assert_eq!(observer.finish(true).model_match_status, "same");

        let mut incomplete =
            ServedModelObserver::new(ModelProtocol::ChatCompletions, "wire", 200, true);
        incomplete.push(b"data: {\"model\":\"wire\"}\n\n");
        assert_eq!(
            incomplete.finish(true).unknown_reason.as_deref(),
            Some("incomplete_stream")
        );
    }

    #[test]
    fn incomplete_response_is_terminal_and_can_confirm_the_served_model() {
        let mut observer = ServedModelObserver::new(ModelProtocol::Responses, "wire", 200, true);
        observer.push(b"event: response.incomplete\ndata: {\"type\":\"response.incomplete\",\"response\":{\"model\":\"wire\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n");
        let result = observer.finish(true);
        assert_eq!(result.unknown_reason, None);
        assert_eq!(result.model_match_status, "same");
    }

    #[test]
    fn response_error_identifiers_are_retained_without_messages_or_unbounded_fields() {
        let mut observer = ServedModelObserver::new(ModelProtocol::Responses, "wire", 200, true);
        observer.push(b"event: error\ndata: {\"type\":\"error\",\"code\":\"server_error\",\"message\":\"private response text\",\"status\":503,\"provider_details\":{\"secret\":\"private\"}}\n\n");
        assert_eq!(
            observer.response_error(),
            &serde_json::json!({"type":"error","code":"server_error","status":503})
        );
        observer.push(
            format!(
                "event: error\ndata: {{\"code\":\"{}\",\"message\":\"private\"}}\n\n",
                "x".repeat(129)
            )
            .as_bytes(),
        );
        assert_eq!(observer.response_error(), &serde_json::json!({}));
    }

    #[test]
    fn missing_request_model_cannot_produce_a_raw_comparison() {
        let mut observer = ServedModelObserver::new(ModelProtocol::Responses, "", 200, false);
        observer.push(br#"{"model":"wire"}"#);
        let result = observer.finish(true);
        assert_eq!(result.model_match_status, "unknown");
        assert_eq!(
            result.unknown_reason.as_deref(),
            Some("request_model_missing")
        );
    }

    #[test]
    fn conflicting_failure_truncation_and_status_remain_unknown() {
        let mut conflict =
            ServedModelObserver::new(ModelProtocol::AnthropicMessages, "a", 200, true);
        conflict.push(b"data: {\"type\":\"message_start\",\"message\":{\"model\":\"a\"}}\n\n");
        conflict.push(b"data: {\"type\":\"message_start\",\"message\":{\"model\":\"b\"}}\n\n");
        conflict.push(b"data: {\"type\":\"message_stop\"}\n\n");
        assert_eq!(
            conflict.finish(true).unknown_reason.as_deref(),
            Some("conflicting_declarations")
        );

        let mut failed = ServedModelObserver::new(ModelProtocol::Responses, "a", 500, false);
        failed.push(br#"{"model":"a"}"#);
        assert_eq!(
            failed.finish(true).unknown_reason.as_deref(),
            Some("upstream_http_error")
        );

        let mut oversized = ServedModelObserver::new(ModelProtocol::Responses, "a", 200, false);
        oversized.push(&vec![b'x'; MAX_OBSERVATION + 1]);
        assert_eq!(
            oversized.finish(true).unknown_reason.as_deref(),
            Some("observation_limit")
        );

        let mut interrupted =
            ServedModelObserver::new(ModelProtocol::ChatCompletions, "a", 200, true);
        interrupted.push(b"data: {\"model\":\"a\"}\n\n");
        assert_eq!(
            interrupted.finish(false).unknown_reason.as_deref(),
            Some("stream_interrupted")
        );
    }

    #[test]
    fn protected_fingerprint_changes_with_key_and_never_contains_secret() {
        let dir = tempfile::tempdir().unwrap();
        let first = CredentialSnapshot::from_header(
            dir.path(),
            "providerApiKey",
            "p@endpoint",
            Some("Bearer secret-one"),
            None,
        );
        let again = CredentialSnapshot::from_header(
            dir.path(),
            "providerApiKey",
            "p@endpoint",
            Some("Bearer secret-one"),
            None,
        );
        let rotated = CredentialSnapshot::from_header(
            dir.path(),
            "providerApiKey",
            "p@endpoint",
            Some("Bearer secret-two"),
            None,
        );
        assert_eq!(first.status, "recorded");
        assert_eq!(first.version_fingerprint, again.version_fingerprint);
        assert_ne!(first.version_fingerprint, rotated.version_fingerprint);
        assert!(!serde_json::to_string(&first)
            .unwrap()
            .contains("secret-one"));
    }
}
