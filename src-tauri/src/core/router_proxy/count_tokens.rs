//! Approximate local token estimates for Claude Desktop's Messages gateway.
//!
//! Claude Code falls back to billable `messages.create(max_tokens: 1)` calls
//! when this endpoint is missing. Estimating at the authenticated local
//! boundary avoids both those hidden inference calls and diagnostic noise.
use serde_json::{Map, Value};

const TEXT_BYTES_PER_TOKEN: usize = 3;
const IMAGE_TOKEN_FLOOR: u64 = 2_048;
const DOCUMENT_TOKEN_FLOOR: u64 = 8_192;
const AUDIO_TOKEN_FLOOR: u64 = 8_192;

pub(super) fn estimate_input_tokens(body: &[u8]) -> Result<u64, &'static str> {
    let request: Value = serde_json::from_slice(body).map_err(|_| "Request body must be valid JSON")?;
    if request.get("model").and_then(Value::as_str).is_none_or(str::is_empty) {
        return Err("A model is required");
    }
    if !request.get("messages").is_some_and(Value::is_array) {
        return Err("Messages must be an array");
    }

    let count = request.as_object().map(estimate_request_object).unwrap_or(0);
    Ok(count.max(1))
}

fn estimate_request_object(request: &Map<String, Value>) -> u64 {
    request.iter().fold(2_u64, |total, (key, value)| {
        total
            .saturating_add(text_tokens(key))
            .saturating_add(estimate_value(value))
            .saturating_add(1)
    })
}

fn estimate_value(value: &Value) -> u64 {
    match value {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) => 1,
        Value::String(text) => text_tokens(text),
        Value::Array(items) => items.iter().fold(1_u64, |total, item| {
            total.saturating_add(estimate_value(item)).saturating_add(1)
        }),
        Value::Object(object) => estimate_object(object),
    }
}

fn estimate_object(object: &Map<String, Value>) -> u64 {
    let kind = object.get("type").and_then(Value::as_str).unwrap_or_default();
    let citation = matches!(
        kind,
        "char_location"
            | "page_location"
            | "content_block_location"
            | "search_result_location"
            | "web_search_result_location"
    );
    let media_floor = match kind {
        "image" => IMAGE_TOKEN_FLOOR,
        "document" => DOCUMENT_TOKEN_FLOOR,
        "audio" | "input_audio" | "audio_input" => AUDIO_TOKEN_FLOOR,
        _ => 0,
    };
    let mut count = object.iter().fold(1_u64, |total, (key, value)| {
        let field_tokens = if citation && key == "cited_text" {
            0
        } else if media_floor > 0 && key == "source" {
            value.as_object().map_or_else(|| estimate_value(value), estimate_media_source)
        } else {
            estimate_value(value)
        };
        total
            .saturating_add(text_tokens(key))
            .saturating_add(field_tokens)
            .saturating_add(1)
    });

    if media_floor > 0 {
        // Base64 payloads are not prose. A byte-ratio estimate on them would
        // vastly overcount images; use a conservative modality floor instead.
        let encoded_len = object
            .get("source")
            .and_then(Value::as_object)
            .filter(|source| source.get("type").and_then(Value::as_str) == Some("base64"))
            .and_then(|source| source.get("data"))
            .and_then(Value::as_str)
            .map_or(0, str::len);
        let size_estimate = u64::try_from(encoded_len.div_ceil(24)).unwrap_or(u64::MAX);
        count = count.saturating_add(media_floor.max(size_estimate));
    }
    count
}

fn estimate_media_source(source: &Map<String, Value>) -> u64 {
    let base64 = source.get("type").and_then(Value::as_str) == Some("base64");
    source.iter().fold(1_u64, |total, (key, value)| {
        let value_tokens = if base64 && key == "data" {
            0
        } else {
            estimate_value(value)
        };
        total
            .saturating_add(text_tokens(key))
            .saturating_add(value_tokens)
            .saturating_add(1)
    })
}

fn text_tokens(text: &str) -> u64 {
    u64::try_from(text.len().div_ceil(TEXT_BYTES_PER_TOKEN)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn count(value: Value) -> u64 {
        estimate_input_tokens(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn validates_required_count_tokens_fields() {
        assert!(estimate_input_tokens(b"{").is_err());
        assert!(estimate_input_tokens(br#"{"messages":[]}"#).is_err());
        assert!(estimate_input_tokens(br#"{"model":"claude","messages":{}}"#).is_err());
    }

    #[test]
    fn estimates_text_unicode_and_tool_schemas_without_returning_zero() {
        let base = count(json!({"model":"claude-haiku-4-5","messages":[{"role":"user","content":"hello"}]}));
        let longer = count(json!({"model":"claude-haiku-4-5","messages":[{"role":"user","content":"hello world, this is a longer request"}]}));
        let unicode = count(json!({"model":"claude-haiku-4-5","messages":[{"role":"user","content":"你好世界"}]}));
        let with_tool = count(json!({"model":"claude-haiku-4-5","messages":[{"role":"user","content":"hello"}],
            "tools":[{"name":"lookup","description":"Search indexed records","input_schema":{"type":"object","properties":{"query":{"type":"string"}}}}]}));
        assert!(base > 0);
        assert!(longer > base);
        assert!(unicode > 0);
        assert!(with_tool > base);
    }

    #[test]
    fn does_not_count_citation_text_replayed_from_a_prior_response() {
        let base = json!({"model":"claude","messages":[{"role":"user","content":[
            {"type":"text","text":"Use this citation"},
            {"type":"web_search_result_location","url":"https://example.com","cited_text":""}
        ]}]});
        let mut replayed = base.clone();
        replayed["messages"][0]["content"][1]["cited_text"] = Value::String("citation ".repeat(50_000));
        assert_eq!(count(base), count(replayed));
    }

    #[test]
    fn estimates_multimodal_payloads_without_counting_base64_as_text() {
        let make_image = |data: String| json!({"model":"claude","messages":[{"role":"user","content":[
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":data}}
        ]}]});
        let small = count(make_image("A".repeat(32)));
        let large = count(make_image("A".repeat(240_000)));
        assert!(large > small);
        assert!(large < 20_000, "image estimate should not scale as raw base64 text: {large}");

        let document = count(json!({"model":"claude","messages":[{"role":"user","content":[
            {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"A".repeat(80_000)}}
        ]}]}));
        assert!(document >= DOCUMENT_TOKEN_FLOOR);
    }
}
