//! Request compatibility without inventing or discarding conversation history.

use super::{Bytes, ForwardResult, StatusCode, Value};

/// A model switch can leave the previous model's effort (notably `none`) in
/// the request. Remove only an effort excluded by a known, nonempty catalog
/// list. Unknown/empty capabilities are not evidence to change the request.
/// Keep summaries and every other reasoning/history field intact.
pub(super) fn prepare_reasoning(
    body: Option<Bytes>,
    catalog: impl FnOnce() -> Value,
) -> ForwardResult<Option<Bytes>> {
    let Some(bytes) = &body else { return Ok(body) };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(body);
    };
    let Some(effort) = value.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return Ok(body);
    };
    let Some(model) = value.get("model").and_then(Value::as_str) else {
        return Ok(body);
    };
    let catalog = catalog();
    let levels = catalog
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|entry| entry["slug"].as_str() == Some(model))
        })
        .and_then(|entry| entry.get("supported_reasoning_levels"))
        .and_then(Value::as_array);
    let Some(levels) = levels.filter(|levels| {
        !levels.is_empty()
            && levels.iter().all(|level| {
                level
                    .get("effort")
                    .and_then(Value::as_str)
                    .is_some_and(|effort| !effort.is_empty())
            })
    }) else {
        return Ok(body);
    };
    if levels
        .iter()
        .any(|level| level["effort"].as_str() == Some(effort))
    {
        return Ok(body);
    }
    let reasoning = value["reasoning"]
        .as_object_mut()
        .expect("string effort in object");
    reasoning.remove("effort");
    if reasoning.is_empty() {
        value
            .as_object_mut()
            .expect("request object")
            .remove("reasoning");
    }
    serde_json::to_vec(&value)
        .map(|bytes| Some(Bytes::from(bytes)))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot re-encode request",
            )
        })
}

/// The translator cannot represent these opaque Responses items. Refuse the lossy
/// conversion, including items replayed from its continuation cache. Native
/// Responses traffic keeps them verbatim; only its upstream can validate them.
pub(super) fn validate_translated_input(input: &[Value]) -> ForwardResult<()> {
    if input.iter().any(|item| {
        matches!(item["type"].as_str(), Some("compaction" | "item_reference"))
            || item
                .get("encrypted_content")
                .is_some_and(|content| !content.is_null())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "conversation history cannot be translated without loss; use a native Responses provider or resend the full conversation without provider-specific items",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog(levels: Value) -> Value {
        json!({"models": [{"slug": "relay::model", "supported_reasoning_levels": levels}]})
    }

    #[test]
    fn unsupported_effort_is_removed_without_changing_history_or_summary() {
        let input = json!({
            "model": "relay::model", "reasoning": {"effort": "none", "summary": "auto"},
            "previous_response_id": "provider-response",
            "input": [
                {"type":"reasoning", "summary":[{"type":"summary_text","text":"keep me"}], "encrypted_content":"opaque"},
                {"type":"compaction", "encrypted_content":"compressed-history"}
            ]
        });
        let body = prepare_reasoning(Some(Bytes::from(input.to_string())), || {
            catalog(json!([{"effort":"high"}]))
        })
        .unwrap()
        .unwrap();
        let mut expected = input;
        expected["reasoning"]
            .as_object_mut()
            .unwrap()
            .remove("effort");
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);

        let body = prepare_reasoning(
            Some(Bytes::from_static(
                br#"{"model":"relay::model","reasoning":{"effort":"none"}}"#,
            )),
            || catalog(json!([{"effort":"high"}])),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"model":"relay::model"})
        );
    }

    #[test]
    fn supported_unknown_or_malformed_capabilities_leave_bytes_unchanged() {
        let body =
            Bytes::from_static(br#"{ "model":"relay::model", "reasoning":{"effort":"none"} }"#);
        for levels in [
            json!([{"effort":"none"}]),
            json!([]),
            Value::Null,
            json!("high"),
            json!([{"effort":"high"},{}]),
        ] {
            assert_eq!(
                prepare_reasoning(Some(body.clone()), || catalog(levels))
                    .unwrap()
                    .unwrap(),
                body
            );
        }
        assert_eq!(
            prepare_reasoning(Some(body.clone()), || json!({"models":[]}))
                .unwrap()
                .unwrap(),
            body
        );
        let body = Bytes::from_static(br#"{"model":"relay::model","input":"hi"}"#);
        assert_eq!(
            prepare_reasoning(Some(body.clone()), || panic!("no effort: no catalog read"))
                .unwrap()
                .unwrap(),
            body
        );
    }
}
