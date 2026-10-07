//! Only actual model-channel failures affect switching. Request validation,
//! unsupported models/protocols, search backends and client cancellation do not.
use serde_json::Value;

pub fn http_failure(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 529)
}

pub fn channel_error(value: &Value) -> bool {
    [
        value.get("error"),
        value.pointer("/response/error"),
        (value["type"] == "error").then_some(value),
    ]
    .into_iter()
    .flatten()
    .any(|error| {
        error
            .get("status")
            .or_else(|| error.get("code"))
            .and_then(Value::as_u64)
            .is_some_and(|status| status <= u16::MAX as u64 && http_failure(status as u16))
            || ["type", "code"].iter().any(|key| {
                error.get(*key).and_then(Value::as_str).is_some_and(|code| {
                    matches!(
                        code,
                        "api_error"
                            | "server_error"
                            | "internal_server_error"
                            | "overloaded_error"
                            | "rate_limit_error"
                            | "rate_limit_exceeded"
                            | "upstream_stream_timeout"
                            | "upstream_stream_error"
                            | "upstream_stream_truncated"
                    )
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_parameter_and_local_adapter_errors_are_not_channel_failures() {
        for status in [200, 400, 401, 403, 404, 409, 422, 501] {
            assert!(!http_failure(status));
        }
        for code in [
            "invalid_request_error",
            "model_not_found",
            "invalid_tool_arguments",
            "unsupported_tool_choice",
            "response_ownership_conflict",
            "cancelled",
        ] {
            assert!(!channel_error(&json!({"error":{"code":code}})));
        }
        assert!(channel_error(
            &json!({"type":"response.failed","response":{"error":{"code":"server_error"}}})
        ));
        assert!(channel_error(&json!({"error":{"type":"overloaded_error"}})));
        assert!(channel_error(
            &json!({"error":{"code":"upstream_stream_truncated"}})
        ));
        assert!(channel_error(
            &json!({"type":"error","code":"server_error","message":"synthetic failure"})
        ));
        assert!(!channel_error(
            &json!({"type":"error","code":"invalid_request_error"})
        ));
        assert!(!channel_error(
            &json!({"object":"response","code":"server_error","status":"completed"})
        ));
        assert!(!channel_error(&json!({"error":{"code":65536 + 429}})));
    }
}
