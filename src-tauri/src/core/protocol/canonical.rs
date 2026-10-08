//! Protocol-independent events and terminal state.
//!
//! The existing adapters still own their wire encoders.  This module is the
//! small shared vocabulary used by diagnostics, history and accounting while
//! those encoders are migrated incrementally.  A canonical event always keeps
//! its source protocol.  Source-specific fields are retained for a same-wire
//! replay, but are never projected into another protocol by accident.

use super::ModelProtocol;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Total retained semantic content per translated response; individual wire
/// frames and history stores retain their existing independent limits.
pub(super) const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;

/// SSE ignores a single UTF-8 BOM at the start of the stream. Observation
/// parsers strip it without changing the bytes forwarded to native callers.
pub(crate) fn strip_stream_bom<'a>(bytes: &'a [u8], at_start: &mut bool) -> &'a [u8] {
    if std::mem::take(at_start) {
        bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
    } else {
        bytes
    }
}

/// The semantic event families understood by the common pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    MessageStart,
    MessageDelta,
    ToolCallStart,
    ToolCallDelta,
    ToolResult,
    Usage,
    Terminal,
    /// An event added by a newer provider or a private extension.
    Unknown(String),
}

impl EventKind {
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    pub fn wire_name(&self) -> &str {
        match self {
            Self::MessageStart => "message_start",
            Self::MessageDelta => "message_delta",
            Self::ToolCallStart => "tool_call_start",
            Self::ToolCallDelta => "tool_call_delta",
            Self::ToolResult => "tool_result",
            Self::Usage => "usage",
            Self::Terminal => "terminal",
            Self::Unknown(name) => name,
        }
    }
}

/// Reasons for a response that reached a protocol terminal event but did not
/// complete normally.  `Truncated` is deliberately a separate terminal state:
/// an upstream that reports `response.incomplete` delivered a valid response,
/// while EOF before a terminal frame means the stream was cut off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IncompleteReason {
    MaxOutputTokens,
    ContentFilter,
    ContextWindow,
    Other(String),
}

impl IncompleteReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::MaxOutputTokens => "max_output_tokens",
            Self::ContentFilter => "content_filter",
            Self::ContextWindow => "model_context_window_exceeded",
            Self::Other(reason) => reason,
        }
    }
}

impl From<&str> for IncompleteReason {
    fn from(reason: &str) -> Self {
        match reason {
            "length" | "max_tokens" | "max_output_tokens" => Self::MaxOutputTokens,
            "content_filter" | "refusal" => Self::ContentFilter,
            "model_context_window_exceeded" | "context_window" => Self::ContextWindow,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Shared terminal semantics consumed by history, diagnostics and usage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TerminalState {
    Completed,
    Incomplete { reason: IncompleteReason },
    Failed { code: String },
    Truncated { reason: String },
}

impl TerminalState {
    pub fn completed() -> Self {
        Self::Completed
    }

    pub fn incomplete(reason: impl Into<IncompleteReason>) -> Self {
        Self::Incomplete {
            reason: reason.into(),
        }
    }

    pub fn failed(code: impl Into<String>) -> Self {
        Self::Failed { code: code.into() }
    }

    pub fn truncated(reason: impl Into<String>) -> Self {
        Self::Truncated {
            reason: reason.into(),
        }
    }

    /// A valid incomplete response was delivered and may be shown to the
    /// caller, even though it is not eligible for full history replay.
    pub fn delivered(&self) -> bool {
        matches!(self, Self::Completed | Self::Incomplete { .. })
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    pub fn is_incomplete(&self) -> bool {
        matches!(self, Self::Incomplete { .. })
    }

    pub fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Completed => None,
            Self::Incomplete { reason } => Some(reason.as_str()),
            Self::Failed { code } | Self::Truncated { reason: code } => Some(code),
        }
    }

    /// Derive a terminal state from a complete Responses-style status.  This
    /// accepts old history records that have no `incomplete_details` field.
    pub fn from_response(status: Option<&str>, reason: Option<&str>, error: Option<&str>) -> Self {
        match status {
            Some("completed") => Self::Completed,
            Some("incomplete") => Self::incomplete(reason.unwrap_or("unknown")),
            Some("failed" | "cancelled") => Self::failed(error.unwrap_or("upstream_error")),
            _ => Self::failed(error.unwrap_or("missing_terminal_status")),
        }
    }

    /// Legacy canonical JSON keeps its storage shape; typed state is a view,
    /// so old history entries do not require a migration or a second copy.
    pub fn from_canonical_response(value: &Value) -> Self {
        Self::from_response(
            value.get("status").and_then(Value::as_str),
            value
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str),
            value.pointer("/error/code").and_then(Value::as_str),
        )
    }

    pub fn from_finish_reason(protocol: ModelProtocol, reason: Option<&str>) -> Self {
        match (protocol, reason) {
            (ModelProtocol::ChatCompletions, Some("stop" | "tool_calls"))
            | (ModelProtocol::AnthropicMessages, Some("end_turn" | "stop_sequence" | "tool_use")) => {
                Self::Completed
            }
            (_, Some("length" | "max_tokens")) => Self::incomplete("max_output_tokens"),
            (_, Some("model_context_window_exceeded")) => {
                Self::incomplete("model_context_window_exceeded")
            }
            (_, Some("content_filter" | "refusal")) => Self::incomplete("content_filter"),
            (_, None) => Self::failed("missing_finish_reason"),
            _ => Self::failed("unsupported_finish_reason"),
        }
    }

    /// Inspect a protocol event without treating an EOF as a valid terminal
    /// event.  Call [`TerminalState::truncated`] when the byte stream ends
    /// before this returns `Some`.
    pub fn from_event(protocol: ModelProtocol, value: &Value) -> Option<Self> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| value.get("event").and_then(Value::as_str))
            .or_else(|| value.get("object").and_then(Value::as_str))?;
        match kind {
            "response.completed" => Some(Self::Completed),
            "response.incomplete" => Some(Self::incomplete(
                value
                    .pointer("/response/incomplete_details/reason")
                    .or_else(|| value.pointer("/incomplete_details/reason"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            )),
            "response.failed" | "response.cancelled" | "error" => Some(Self::failed(
                value
                    .pointer("/error/code")
                    .or_else(|| value.pointer("/response/error/code"))
                    .and_then(Value::as_str)
                    .unwrap_or("upstream_error"),
            )),
            "message_stop" => {
                let reason = value
                    .pointer("/message/stop_reason")
                    .or_else(|| value.get("stop_reason"))
                    .and_then(Value::as_str);
                Some(match protocol {
                    ModelProtocol::AnthropicMessages => match reason {
                        Some("max_tokens") => Self::incomplete("max_output_tokens"),
                        Some("refusal") => Self::incomplete("content_filter"),
                        Some("end_turn" | "stop_sequence" | "tool_use") | None => Self::Completed,
                        Some(other) => Self::failed(other),
                    },
                    _ => Self::Completed,
                })
            }
            "chat.completion" | "chat.completion.chunk" => {
                let reason = value
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str);
                match reason {
                    Some("length") => Some(Self::incomplete("max_output_tokens")),
                    Some("content_filter") => Some(Self::incomplete("content_filter")),
                    Some("stop" | "tool_calls") => Some(Self::Completed),
                    Some(other) => Some(Self::failed(other)),
                    None => None,
                }
            }
            _ => None,
        }
    }
}

/// Tiny state machine shared by the raw-byte observers. Finish reasons arrive
/// before the actual terminal marker in Chat and Messages streams; remember
/// them without allowing a disconnect to masquerade as a successful response.
#[derive(Clone, Debug, Default)]
pub struct TerminalObservation {
    state: Option<TerminalState>,
    pending: Option<TerminalState>,
    pending_protocol: Option<ModelProtocol>,
}

impl TerminalObservation {
    pub fn state(&self) -> Option<&TerminalState> {
        self.state.as_ref()
    }

    pub fn received(&self) -> bool {
        self.state.is_some()
    }

    pub fn failed(&self) -> bool {
        self.state.as_ref().is_some_and(|state| !state.delivered())
    }

    pub fn done(&mut self) {
        // [DONE] only closes a Chat response that already supplied a finish
        // reason. It cannot replace Messages/Responses terminal events.
        if self.state.is_none() && self.pending_protocol == Some(ModelProtocol::ChatCompletions) {
            self.state = self.pending.take();
        }
    }

    pub fn observe(&mut self, protocol: ModelProtocol, value: &Value, event_name: &str) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);
        if !value["error"].is_null()
            || matches!(kind, "error" | "response.failed" | "response.cancelled")
            || value["status"] == "failed"
        {
            self.state = Some(TerminalState::failed("upstream_error"));
            return;
        }
        if self.failed() {
            return;
        }
        if protocol == ModelProtocol::ChatCompletions {
            if let Some(reason) = value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                self.pending = Some(TerminalState::from_finish_reason(protocol, Some(reason)));
                self.pending_protocol = Some(protocol);
            }
        } else if protocol == ModelProtocol::AnthropicMessages && kind == "message_delta" {
            if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                self.pending = Some(TerminalState::from_finish_reason(protocol, Some(reason)));
                self.pending_protocol = Some(protocol);
            }
        }
        self.state = match (protocol, kind) {
            (ModelProtocol::Responses, "response.completed") => Some(TerminalState::Completed),
            (ModelProtocol::Responses, "response.incomplete") => Some(TerminalState::incomplete(
                value
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            )),
            (ModelProtocol::AnthropicMessages, "message_stop") => {
                Some(self.pending.take().unwrap_or(TerminalState::Completed))
            }
            _ => self.state.take(),
        };
    }

    /// Body observations have no framing marker. A status/finish reason is
    /// enough there; unknown native response shapes preserve existing behavior.
    pub fn observe_body(&mut self, protocol: ModelProtocol, value: &Value) {
        self.observe(protocol, value, "");
        if self.failed() {
            return;
        }
        match protocol {
            ModelProtocol::Responses
                if matches!(
                    value["status"].as_str(),
                    Some("completed" | "incomplete" | "failed" | "cancelled")
                ) =>
            {
                self.state = Some(TerminalState::from_canonical_response(value));
            }
            ModelProtocol::ChatCompletions if self.pending.is_some() => self.done(),
            ModelProtocol::AnthropicMessages if value.get("stop_reason").is_some() => {
                self.state = Some(TerminalState::from_finish_reason(
                    protocol,
                    value["stop_reason"].as_str(),
                ));
            }
            _ => {}
        }
    }

    pub fn observe_auto(&mut self, value: &Value, event_name: &str, stream: bool) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);
        let protocol = if value.get("choices").is_some() {
            ModelProtocol::ChatCompletions
        } else if kind.starts_with("message_")
            || kind.starts_with("content_block_")
            || value["type"] == "message"
        {
            ModelProtocol::AnthropicMessages
        } else {
            ModelProtocol::Responses
        };
        if stream {
            self.observe(protocol, value, event_name);
        } else {
            self.observe_body(protocol, value);
        }
    }

    pub fn finish(&mut self, eof: bool, stream: bool) {
        if !eof {
            self.state = Some(TerminalState::truncated("stream_interrupted"));
        } else if stream && !self.received() {
            self.state = Some(TerminalState::truncated("upstream_stream_truncated"));
        }
    }
}

/// Typed token usage shared by the three wire protocols.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

impl Usage {
    /// Raw counters are merged before cache subtraction because some providers
    /// send their cache details in a later usage-only stream frame.
    pub fn raw_from_wire(value: &Value) -> Option<Self> {
        let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        let input = number("input_tokens").max(number("prompt_tokens"));
        let output = number("output_tokens").max(number("completion_tokens"));
        let cache_read = number("cache_read_input_tokens")
            .max(number("prompt_cache_hit_tokens"))
            .max(
                value
                    .pointer("/input_tokens_details/cached_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
            .max(
                value
                    .pointer("/prompt_tokens_details/cached_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
        let cache_write = number("cache_creation_input_tokens");
        let reasoning = number("reasoning_tokens")
            .max(
                value
                    .pointer("/output_tokens_details/reasoning_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
            .max(
                value
                    .pointer("/completion_tokens_details/reasoning_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
        let total = number("total_tokens");
        let reported = [
            "input_tokens",
            "prompt_tokens",
            "output_tokens",
            "completion_tokens",
            "total_tokens",
            "reasoning_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .any(|key| value.get(*key).and_then(Value::as_u64).is_some())
            || Self::cache_read_reported(value);
        if !reported {
            None
        } else {
            Some(Self {
                input_tokens: input,
                output_tokens: output,
                reasoning_tokens: reasoning,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                total_tokens: total,
            })
        }
    }

    pub fn from_wire(value: &Value) -> Option<Self> {
        let mut usage = Self::raw_from_wire(value)?;
        if Self::cache_is_in_input(value) {
            usage.input_tokens = usage.input_tokens.saturating_sub(usage.cache_read_tokens);
        }
        usage.total_tokens = usage.total_tokens.max(
            usage
                .input_tokens
                .saturating_add(usage.cache_read_tokens)
                .saturating_add(usage.cache_write_tokens)
                .saturating_add(usage.output_tokens),
        );
        Some(usage)
    }

    /// Explicit zero is a measured cache miss, while an absent field is unknown.
    pub fn cache_read_reported(value: &Value) -> bool {
        value
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .is_some()
            || Self::cache_is_in_input(value)
    }

    /// OpenAI totals and DeepSeek's prompt_tokens include cache hits; Anthropic
    /// input_tokens excludes them. DeepSeek may send both aliases for the same
    /// counter (https://api-docs.deepseek.com/api/create-chat-completion).
    pub fn cache_is_in_input(value: &Value) -> bool {
        [
            "/input_tokens_details/cached_tokens",
            "/prompt_tokens_details/cached_tokens",
            "/prompt_cache_hit_tokens",
        ]
        .iter()
        .any(|path| value.pointer(path).and_then(Value::as_u64).is_some())
    }
}

/// Borrowed views keep legacy JSON history readable without building another
/// complete conversation or allocating a second copy in the hot path.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub role: &'a str,
    pub content: &'a Value,
    pub wire: &'a Value,
}

impl<'a> Message<'a> {
    pub fn from_response_item(value: &'a Value) -> Option<Self> {
        (value["type"] == "message").then(|| Self {
            role: value["role"].as_str().unwrap_or("assistant"),
            content: &value["content"],
            wire: value,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolCall<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub arguments: &'a str,
    pub wire: &'a Value,
}

impl<'a> ToolCall<'a> {
    pub fn from_response_item(value: &'a Value) -> Option<Self> {
        if value["type"] != "function_call" {
            return None;
        }
        Some(Self {
            id: value["call_id"].as_str().unwrap_or_default(),
            name: value["name"].as_str().unwrap_or_default(),
            arguments: value["arguments"].as_str().unwrap_or_default(),
            wire: value,
        })
    }

    pub fn is_valid(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.name.trim().is_empty()
            && serde_json::from_str::<Value>(self.arguments)
                .ok()
                .is_some_and(|value| value.is_object())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OutputItem<'a> {
    Message(Message<'a>),
    ToolCall(ToolCall<'a>),
    Reasoning(&'a Value),
    Other(&'a Value),
}

impl<'a> OutputItem<'a> {
    pub fn from_legacy(value: &'a Value) -> Self {
        if let Some(message) = Message::from_response_item(value) {
            Self::Message(message)
        } else if let Some(call) = ToolCall::from_response_item(value) {
            Self::ToolCall(call)
        } else if value["type"] == "reasoning" {
            Self::Reasoning(value)
        } else {
            Self::Other(value)
        }
    }

    pub fn wire(&self) -> &'a Value {
        match self {
            Self::Message(message) => message.wire,
            Self::ToolCall(call) => call.wire,
            Self::Reasoning(value) | Self::Other(value) => value,
        }
    }

    pub fn replayable(&self) -> bool {
        if self
            .wire()
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "completed")
        {
            return false;
        }
        match self {
            Self::ToolCall(call) => call.is_valid(),
            _ => true,
        }
    }
}

/// A source-specific field retained for same-protocol replay. It is never
/// silently copied to another protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceExtension {
    pub source: ModelProtocol,
    pub name: String,
    pub value: Value,
}

/// An event tracks both upstream provenance and payload representation. Raw
/// events retain the original wire value; decoded events carry only the
/// portable representation produced by their adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalEvent {
    pub source: ModelProtocol,
    pub payload_protocol: ModelProtocol,
    pub original_wire: bool,
    pub kind: EventKind,
    pub sequence: Option<u64>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<SourceExtension>,
}

impl CanonicalEvent {
    pub fn from_wire(source: ModelProtocol, value: &Value, sequence: Option<u64>) -> Self {
        Self::from_owned(source, value.clone(), sequence)
    }

    /// The stream decoders already own the payload; move it through the common
    /// event path instead of cloning a second complete event value.
    pub fn from_owned(source: ModelProtocol, value: Value, sequence: Option<u64>) -> Self {
        let wire_name = value
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| value.get("object").and_then(Value::as_str))
            .unwrap_or("unknown");
        let kind = event_kind(wire_name, &value);
        let extensions = value
            .as_object()
            .into_iter()
            .flat_map(|object| object.iter())
            .filter(|(name, _)| is_source_extension(source, name))
            .map(|(name, value)| SourceExtension {
                source,
                name: name.clone(),
                value: value.clone(),
            })
            .collect();
        Self {
            source,
            payload_protocol: source,
            original_wire: true,
            kind,
            sequence,
            payload: value,
            extensions,
        }
    }

    pub fn from_decoded(
        source: ModelProtocol,
        payload_protocol: ModelProtocol,
        value: Value,
        sequence: Option<u64>,
    ) -> Self {
        let mut event = Self::from_owned(payload_protocol, value, sequence);
        event.source = source;
        event.original_wire = false;
        // Decoding is not evidence that the original opaque source fields
        // survived. Private extensions stay in their native adapter/history.
        event.extensions.clear();
        event
    }

    pub fn usage(&self) -> Option<Usage> {
        self.payload
            .get("usage")
            .or_else(|| self.payload.pointer("/response/usage"))
            .and_then(Usage::from_wire)
    }

    pub fn terminal(&self) -> Option<TerminalState> {
        TerminalState::from_event(self.payload_protocol, &self.payload)
    }

    /// Only a same-protocol replay can use the opaque source payload. Cross
    /// protocol encoders must consume typed fields through their own adapter.
    pub fn project_to(&self, target: ModelProtocol) -> Result<Value, String> {
        if self.original_wire && target == self.source && target == self.payload_protocol {
            return Ok(self.payload.clone());
        }
        Err("protocol.errors.unsupportedEvent".into())
    }

    /// Returns top-level fields which were present in the source but lost by a
    /// same-protocol rewrite. `model` is intentionally ignored because callers
    /// may patch it to the logical public model ID.
    pub fn missing_same_protocol_fields(&self, rewritten: &Value) -> Vec<String> {
        let Some(original) = self.payload.as_object() else {
            return Vec::new();
        };
        let Some(rewritten) = rewritten.as_object() else {
            return original
                .keys()
                .filter(|key| *key != "model")
                .cloned()
                .collect();
        };
        original
            .keys()
            .filter(|key| key.as_str() != "model" && !rewritten.contains_key(*key))
            .cloned()
            .collect()
    }
}

fn event_kind(name: &str, value: &Value) -> EventKind {
    match name {
        "message_start" | "response.created" | "response.in_progress" => EventKind::MessageStart,
        "content_block_delta"
        | "response.output_text.delta"
        | "response.reasoning_text.delta"
        | "chat.completion.chunk" => EventKind::MessageDelta,
        "content_block_start"
            if matches!(
                value["content_block"]["type"].as_str(),
                Some("tool_use" | "server_tool_use")
            ) =>
        {
            EventKind::ToolCallStart
        }
        "response.output_item.added"
            if matches!(
                value["item"]["type"].as_str(),
                Some("function_call" | "web_search_call")
            ) =>
        {
            EventKind::ToolCallStart
        }
        "content_block_start" | "response.output_item.added" => EventKind::MessageStart,
        "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
            EventKind::ToolCallDelta
        }
        "tool_result" | "function_call_output" | "web_search_tool_result" => EventKind::ToolResult,
        "usage" | "response.usage" => EventKind::Usage,
        "response.completed"
        | "response.incomplete"
        | "response.failed"
        | "response.cancelled"
        | "message_stop" => EventKind::Terminal,
        other => EventKind::Unknown(other.to_owned()),
    }
}

fn is_source_extension(source: ModelProtocol, name: &str) -> bool {
    // These fields are provider/client extensions rather than portable
    // protocol fields. They remain available to the same-wire path.
    match source {
        ModelProtocol::Responses => matches!(
            name,
            "encrypted_content" | "reasoning_content" | "signature"
        ),
        ModelProtocol::ChatCompletions => matches!(
            name,
            "reasoning_content" | "system_fingerprint" | "provider_specific"
        ),
        ModelProtocol::AnthropicMessages => matches!(
            name,
            "signature" | "thinking" | "cache_control" | "provider_specific"
        ),
    }
}

/// Compare only fields that are not explicitly patched by the adapter. This
/// keeps differential checks useful when IDs/model names are intentionally
/// regenerated while unknown provider fields must survive a same-wire pass.
pub fn validate_same_protocol(
    original: &Value,
    rewritten: &Value,
    patched_fields: &[&str],
) -> Result<(), String> {
    let original = original
        .as_object()
        .ok_or("protocol.errors.invalidResponse")?;
    let rewritten = rewritten
        .as_object()
        .ok_or("protocol.errors.invalidResponse")?;
    for (name, value) in original {
        if patched_fields.contains(&name.as_str()) {
            continue;
        }
        if rewritten.get(name) != Some(value) {
            return Err(format!("protocol.errors.fieldChanged:{name}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terminal_state_distinguishes_incomplete_and_truncated() {
        assert_eq!(
            TerminalState::from_event(
                ModelProtocol::Responses,
                &json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}})
            ),
            Some(TerminalState::incomplete("max_output_tokens"))
        );
        let truncated = TerminalState::truncated("upstream_stream_truncated");
        assert!(!truncated.delivered());
        assert!(truncated.is_truncated());
        assert!(TerminalState::incomplete("max_output_tokens").delivered());
    }

    #[test]
    fn usage_normalizes_openai_and_anthropic_field_names() {
        let usage = Usage::from_wire(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "prompt_tokens_details":{"cached_tokens":3},
            "total_tokens":14
        }))
        .unwrap();
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.cache_read_tokens, 3);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.total_tokens, 14);
    }

    #[test]
    fn deepseek_cache_aliases_and_nested_details_describe_one_counter() {
        for details in [json!({}), json!({"cached_tokens":80})] {
            let usage = Usage::from_wire(&json!({"prompt_tokens":100, "completion_tokens":10,
                "prompt_cache_hit_tokens":80, "prompt_cache_miss_tokens":20, "prompt_tokens_details":details})).unwrap();
            assert_eq!(usage.input_tokens, 20);
            assert_eq!(usage.cache_read_tokens, 80);
            assert_eq!(usage.total_tokens, 110);
        }
        assert_eq!(
            Usage::raw_from_wire(&json!({"prompt_cache_hit_tokens":0}))
                .unwrap()
                .cache_read_tokens,
            0
        );
        assert!(Usage::cache_read_reported(
            &json!({"prompt_cache_hit_tokens":0})
        ));
        assert!(!Usage::cache_read_reported(
            &json!({"prompt_cache_hit_tokens":null})
        ));
    }

    #[test]
    fn same_protocol_keeps_unknown_fields_and_events() {
        let value = json!({"type":"vendor.future_event","model":"m","opaque":{"x":1},"signature":"private"});
        let event = CanonicalEvent::from_wire(ModelProtocol::Responses, &value, Some(4));
        assert!(matches!(event.kind, EventKind::Unknown(_)));
        assert_eq!(event.project_to(ModelProtocol::Responses).unwrap(), value);
        assert_eq!(event.sequence, Some(4));
    }

    #[test]
    fn private_extensions_do_not_cross_protocol() {
        let value =
            json!({"type":"message_start","model":"m","reasoning_content":"opaque","opaque":true});
        let event = CanonicalEvent::from_wire(ModelProtocol::ChatCompletions, &value, None);
        assert!(event.project_to(ModelProtocol::AnthropicMessages).is_err());
        assert_eq!(event.extensions[0].source, ModelProtocol::ChatCompletions);
        assert_eq!(
            event.project_to(ModelProtocol::ChatCompletions).unwrap(),
            value
        );
    }

    #[test]
    fn decoded_payload_is_not_replayable_as_its_upstream_wire_protocol() {
        let event = CanonicalEvent::from_decoded(
            ModelProtocol::ChatCompletions,
            ModelProtocol::Responses,
            json!({"type":"response.completed","response":{"status":"completed"}}),
            None,
        );
        assert_eq!(event.source, ModelProtocol::ChatCompletions);
        assert_eq!(event.payload_protocol, ModelProtocol::Responses);
        assert!(!event.original_wire);
        assert_eq!(event.terminal(), Some(TerminalState::Completed));
        assert!(event.project_to(ModelProtocol::ChatCompletions).is_err());
        assert!(event.project_to(ModelProtocol::Responses).is_err());
        assert!(event.extensions.is_empty());
    }

    #[test]
    fn terminal_markers_keep_incomplete_and_require_chat_done() {
        let mut chat = TerminalObservation::default();
        chat.observe(
            ModelProtocol::ChatCompletions,
            &json!({"choices":[{"finish_reason":"length"}]}),
            "",
        );
        assert!(!chat.received());
        chat.finish(true, true);
        assert!(chat.state().unwrap().is_truncated());

        let mut chat = TerminalObservation::default();
        chat.observe(
            ModelProtocol::ChatCompletions,
            &json!({"choices":[{"finish_reason":"length"}]}),
            "",
        );
        chat.done();
        assert_eq!(
            chat.state(),
            Some(&TerminalState::incomplete("max_output_tokens"))
        );

        let mut responses = TerminalObservation::default();
        responses.observe(ModelProtocol::Responses,
            &json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}), "");
        responses.done();
        assert_eq!(
            responses.state(),
            Some(&TerminalState::incomplete("content_filter"))
        );
    }

    #[test]
    fn messages_stop_uses_the_prior_delta_reason() {
        let mut state = TerminalObservation::default();
        state.observe(
            ModelProtocol::AnthropicMessages,
            &json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
            "",
        );
        assert!(!state.received());
        state.observe(
            ModelProtocol::AnthropicMessages,
            &json!({"type":"message_stop"}),
            "",
        );
        assert_eq!(
            state.state(),
            Some(&TerminalState::incomplete("max_output_tokens"))
        );
    }

    #[test]
    fn done_without_a_chat_finish_reason_cannot_create_success() {
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            let mut state = TerminalObservation::default();
            state.observe(protocol, &json!({"model":"m"}), "");
            state.done();
            assert!(!state.received(), "{protocol:?}");
            state.finish(true, true);
            assert!(state.state().unwrap().is_truncated(), "{protocol:?}");
        }
        let mut messages = TerminalObservation::default();
        messages.observe(
            ModelProtocol::AnthropicMessages,
            &json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
            "",
        );
        messages.done();
        assert!(!messages.received());

        let mut chat = TerminalObservation::default();
        chat.observe(
            ModelProtocol::ChatCompletions,
            &json!({"type":"response.completed"}),
            "response.completed",
        );
        chat.done();
        assert!(!chat.received());
    }

    #[test]
    fn old_json_items_have_typed_views_without_losing_extensions() {
        let message = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}],"vendor":{"x":1}});
        let item = OutputItem::from_legacy(&message);
        let OutputItem::Message(view) = item else {
            panic!("message view")
        };
        assert_eq!(view.role, "assistant");
        assert_eq!(view.content[0]["text"], "ok");
        assert_eq!(item.wire(), &message);
        assert!(item.replayable());
        let call =
            json!({"type":"function_call","call_id":"c1","name":"lookup","arguments":"{\"q\":1}"});
        assert!(OutputItem::from_legacy(&call).replayable());
        let bad = json!({"type":"function_call","call_id":"c1","name":"lookup","arguments":"{"});
        assert!(!OutputItem::from_legacy(&bad).replayable());
    }

    #[test]
    fn same_protocol_diff_allows_only_explicit_model_patch() {
        let original = json!({"type":"response.completed","model":"wire","opaque":{"x":1}});
        let mut rewritten = original.clone();
        rewritten["model"] = json!("public");
        validate_same_protocol(&original, &rewritten, &["model"]).unwrap();
        rewritten["opaque"] = json!({"x":2});
        assert!(validate_same_protocol(&original, &rewritten, &["model"]).is_err());
    }
}
