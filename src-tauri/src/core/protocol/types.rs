//! Protocol identity belongs to the adapters, independently of route storage.
use serde::{Deserialize, Serialize};

/// A wire protocol, shared by client ingress and provider egress.
///
/// The serialized values match the existing provider configuration. Keeping this
/// type in the protocol layer lets aggregation refer to it without owning or
/// importing the legacy router configuration.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ModelProtocol {
    #[default]
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

/// Error category safe to include in diagnostics. The error never owns request
/// content, credentials, or an upstream response body.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProtocolErrorKind {
    UnsupportedTranslation,
    InvalidRequest,
    InvalidHistory,
    UnsupportedFeature,
    InvalidResponse,
    ConversionRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    pub kind: ProtocolErrorKind,
    /// Existing local error identifier; preserve it for i18n and compatibility.
    pub code: String,
    pub from: ModelProtocol,
    pub to: ModelProtocol,
}

impl ProtocolError {
    pub(crate) fn from_legacy(code: String, from: ModelProtocol, to: ModelProtocol) -> Self {
        let kind = match code.as_str() {
            "protocol.errors.unsupportedTranslation" => ProtocolErrorKind::UnsupportedTranslation,
            "protocol.errors.invalidRequest" | "protocol.errors.invalidToolArguments" => {
                ProtocolErrorKind::InvalidRequest
            }
            "protocol.errors.unansweredToolCall"
            | "protocol.errors.unknownPreviousResponse"
            | "protocol.errors.historyUnavailable" => ProtocolErrorKind::InvalidHistory,
            "protocol.errors.invalidResponse" => ProtocolErrorKind::InvalidResponse,
            "protocol.errors.unsupportedContent"
            | "protocol.errors.unsupportedMessageRole"
            | "protocol.errors.unsupportedHostedTool"
            | "protocol.errors.unsupportedCustomTool"
            | "protocol.errors.unsupportedNamespacedTool"
            | "protocol.errors.unsupportedToolDefinition"
            | "protocol.errors.unsupportedSearchOptions" => ProtocolErrorKind::UnsupportedFeature,
            _ => ProtocolErrorKind::ConversionRejected,
        };
        Self {
            kind,
            code,
            from,
            to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn protocol_storage_values_remain_compatible() {
        for (protocol, text) in [
            (ModelProtocol::Responses, "responses"),
            (ModelProtocol::ChatCompletions, "chatCompletions"),
            (ModelProtocol::AnthropicMessages, "anthropicMessages"),
        ] {
            assert_eq!(serde_json::to_value(protocol).unwrap(), json!(text));
            assert_eq!(
                serde_json::from_value::<ModelProtocol>(json!(text)).unwrap(),
                protocol
            );
        }
        assert_eq!(ModelProtocol::default(), ModelProtocol::Responses);
    }

    #[test]
    fn structured_error_preserves_local_code_and_protocol_pair() {
        let error = ProtocolError::from_legacy(
            "protocol.errors.unsupportedToolDefinition".into(),
            ModelProtocol::AnthropicMessages,
            ModelProtocol::ChatCompletions,
        );
        assert_eq!(error.kind, ProtocolErrorKind::UnsupportedFeature);
        assert_eq!(error.code, "protocol.errors.unsupportedToolDefinition");
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({
                "kind":"unsupportedFeature", "code":"protocol.errors.unsupportedToolDefinition",
                "from":"anthropicMessages", "to":"chatCompletions"
            })
        );
    }

    #[test]
    fn tool_history_errors_are_not_reported_as_unsupported_features() {
        for (code, kind) in [
            (
                "protocol.errors.invalidToolArguments",
                ProtocolErrorKind::InvalidRequest,
            ),
            (
                "protocol.errors.unansweredToolCall",
                ProtocolErrorKind::InvalidHistory,
            ),
            (
                "protocol.errors.unknownPreviousResponse",
                ProtocolErrorKind::InvalidHistory,
            ),
        ] {
            assert_eq!(
                ProtocolError::from_legacy(
                    code.into(),
                    ModelProtocol::Responses,
                    ModelProtocol::ChatCompletions
                )
                .kind,
                kind
            );
        }
    }
}
