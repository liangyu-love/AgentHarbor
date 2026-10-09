//! Codex's built-in OpenAI provider reaches the router through `openai_base_url`.
//! A thread therefore keeps its `openai` provider forever and every request
//! picks its destination by model: bare slugs go to the account, `id::model`
//! to a relay. This module adapts that first-party wire to both destinations.
//!
//! - The provider cannot carry custom headers, so the local token is a path segment.
//! - WebSocket upgrades get 426 and Codex falls back to HTTP for the session.
//! - Request bodies may be zstd-compressed.
//! - Encrypted reasoning only goes back to the service that produced it.
//! - Remote compaction (V2) is emulated for relays with a readable summary,
//!   carried inside a compaction item that only this router understands.
use super::*;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;

const ENTRY_PREFIX: &str = "/openai/";
const COMPACTION_PREFIX: &str = "agentharbor.compaction.v1:";
/// Codex's own local compaction prompt and summary prefix (codex-rs/prompts).
const SUMMARIZATION_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";
const SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:";
pub(super) const UNREADABLE_COMPACTION: &str = "[Earlier conversation was compacted by the official OpenAI service into an encrypted summary that this model cannot read. Continue from the messages below and ask the user for any missing context.]";
/// Item types a relay can receive. Anything else is first-party wire state.
const RELAY_ITEM_TYPES: &[&str] = &[
    "message",
    "function_call",
    "function_call_output",
    "custom_tool_call",
    "custom_tool_call_output",
    "local_shell_call",
    "local_shell_call_output",
    "tool_search_call",
    "tool_search_output",
    "web_search_call",
    "image_generation_call",
];

/// `/openai/{token}/v1/...` → (`/codex/v1/...`, token).
pub(super) fn builtin_entry(path: &str) -> Option<(String, &str)> {
    let (token, rest) = path.strip_prefix(ENTRY_PREFIX)?.split_once('/')?;
    let rest = rest.strip_prefix("v1")?;
    (!token.is_empty() && (rest.is_empty() || rest.starts_with('/')))
        .then(|| (format!("/codex/v1{rest}"), token))
}

pub(super) fn websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

/// Decompress a zstd request body; other encodings are left untouched.
pub(super) fn decode_body(headers: &mut HeaderMap, body: Bytes) -> ForwardResult<Bytes> {
    let zstd = headers
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("zstd"));
    if !zstd {
        return Ok(body);
    }
    let mut decoded = Vec::new();
    zstd::stream::read::Decoder::new(body.as_ref())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid zstd request body"))?
        .take(MAX_REQUEST_BODY as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid zstd request body"))?;
    if decoded.len() > MAX_REQUEST_BODY {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large"));
    }
    headers.remove(hyper::header::CONTENT_ENCODING);
    Ok(Bytes::from(decoded))
}

fn parse(body: Option<&Bytes>) -> Option<Value> {
    body.and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .filter(|value| value["input"].is_array())
}

fn encode(value: &Value) -> ForwardResult<Option<Bytes>> {
    serde_json::to_vec(value)
        .map(|bytes| Some(Bytes::from(bytes)))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "cannot re-encode request",
            )
        })
}

fn own_summary(item: &Value) -> Option<String> {
    if item["type"] != "compaction" {
        return None;
    }
    let encoded = item["encrypted_content"]
        .as_str()?
        .strip_prefix(COMPACTION_PREFIX)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    String::from_utf8(bytes).ok()
}

fn foreign_compaction(item: &Value) -> Option<&str> {
    matches!(
        item["type"].as_str(),
        Some("compaction" | "compaction_summary")
    )
    .then(|| item["encrypted_content"].as_str())
    .flatten()
    .filter(|content| !content.starts_with(COMPACTION_PREFIX) && !content.is_empty())
}

fn summary_message(summary: &str) -> Value {
    json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": format!("{SUMMARY_PREFIX}\n{summary}")}],
    })
}

pub(super) fn compaction_trigger(body: Option<&Bytes>) -> bool {
    parse(body).is_some_and(|value| {
        value["input"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["type"] == "compaction_trigger")
        })
    })
}

/// Encrypted official compactions a relay cannot read, keyed for conversion.
pub(super) fn official_compactions(body: Option<&Bytes>) -> Vec<String> {
    let Some(value) = parse(body) else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for item in value["input"].as_array().into_iter().flatten() {
        if let Some(content) = foreign_compaction(item) {
            if !found.iter().any(|known| known == content) {
                found.push(content.to_owned());
            }
        }
    }
    found
}

/// Official: our compaction items become readable summaries; reasoning that
/// the account did not encrypt (e.g. from a relay) is not replayed.
pub(super) fn prepare_official(body: Option<Bytes>) -> ForwardResult<Option<Bytes>> {
    let Some(mut value) = parse(body.as_ref()) else {
        return Ok(body);
    };
    let input = value["input"].as_array().cloned().unwrap_or_default();
    let mut changed = false;
    let mut prepared = Vec::with_capacity(input.len());
    for item in input {
        if let Some(summary) = own_summary(&item) {
            prepared.push(summary_message(&summary));
            changed = true;
        } else if item["type"] == "reasoning"
            && !item["encrypted_content"]
                .as_str()
                .is_some_and(|content| !content.is_empty())
        {
            changed = true;
        } else {
            prepared.push(item);
        }
    }
    if !changed {
        return Ok(body);
    }
    value["input"] = Value::Array(prepared);
    encode(&value)
}

/// Retry body for an official request whose replayed reasoning was rejected,
/// e.g. after an account switch. Visible conversation content is unchanged.
pub(super) fn without_reasoning(body: &Bytes) -> Option<Bytes> {
    let mut value = parse(Some(body))?;
    let items = value["input"].as_array_mut()?;
    let before = items.len();
    items.retain(|item| item["type"] != "reasoning");
    (items.len() != before).then(|| Bytes::from(value.to_string()))
}

pub(super) fn rejected_reasoning(status: StatusCode, body: &[u8]) -> bool {
    if status != StatusCode::BAD_REQUEST {
        return false;
    }
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    text.contains("encrypted") || text.contains("could not be decrypted")
}

/// Relay: drop first-party state, keep every visible message and tool pair.
/// `converted` maps official compaction payloads to readable summaries.
pub(super) fn prepare_relay(
    body: Option<Bytes>,
    converted: &HashMap<String, String>,
) -> ForwardResult<Option<Bytes>> {
    let Some(mut value) = parse(body.as_ref()) else {
        return Ok(body);
    };
    let input = value["input"].as_array().cloned().unwrap_or_default();
    let mut prepared = Vec::with_capacity(input.len());
    for mut item in input {
        if let Some(summary) = own_summary(&item) {
            prepared.push(summary_message(&summary));
            continue;
        }
        if let Some(content) = foreign_compaction(&item) {
            prepared.push(match converted.get(content) {
                Some(summary) => summary_message(summary),
                None => json!({
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": UNREADABLE_COMPACTION}],
                }),
            });
            continue;
        }
        // Reasoning is dropped too: a relay cannot decrypt another service's.
        let kind = match item["type"].as_str() {
            Some(kind) => kind,
            None if item.get("role").is_some() => "message",
            None => continue,
        };
        if !RELAY_ITEM_TYPES.contains(&kind) {
            continue;
        }
        if let Some(object) = item.as_object_mut() {
            object.remove("internal_chat_message_metadata_passthrough");
            object.remove("encrypted_function_args");
        }
        prepared.push(item);
    }
    value["input"] = Value::Array(prepared);
    let object = value.as_object_mut().expect("request object");
    if let Some(include) = object.get_mut("include").and_then(Value::as_array_mut) {
        include.retain(|field| field.as_str() != Some("reasoning.encrypted_content"));
        if include.is_empty() {
            object.remove("include");
        }
    }
    // First-party streaming option for concurrent reasoning summaries.
    if let Some(options) = object
        .get_mut("stream_options")
        .and_then(Value::as_object_mut)
    {
        options.remove("reasoning_summary_delivery");
        if options.is_empty() {
            object.remove("stream_options");
        }
    }
    encode(&value)
}

/// Turn a V2 compaction request into an ordinary summarization turn, as
/// Codex itself does for providers without remote compaction.
pub(super) fn summarization_request(body: Option<Bytes>) -> ForwardResult<Option<Bytes>> {
    let Some(mut value) = parse(body.as_ref()) else {
        return Ok(body);
    };
    let items = value["input"].as_array_mut().expect("parsed input");
    items.retain(|item| item["type"] != "compaction_trigger");
    items.push(json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": SUMMARIZATION_PROMPT}],
    }));
    value["stream"] = json!(true);
    encode(&value)
}

/// The final assistant text in a Responses SSE body.
fn output_text(sse: &[u8]) -> (Option<String>, Option<Value>) {
    let mut deltas = String::new();
    let mut completed_text = None;
    let mut usage = None;
    for line in String::from_utf8_lossy(sse).lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        match event["type"].as_str() {
            Some("response.output_text.delta") => {
                deltas.push_str(event["delta"].as_str().unwrap_or_default());
            }
            Some("response.completed") => {
                usage = event["response"]
                    .get("usage")
                    .filter(|u| !u.is_null())
                    .cloned();
                let text: String = event["response"]["output"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"] == "message")
                    .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
                    .filter_map(|part| part["text"].as_str().map(str::to_owned))
                    .collect();
                if !text.trim().is_empty() {
                    completed_text = Some(text);
                }
            }
            _ => {}
        }
    }
    let text = completed_text.or_else(|| (!deltas.trim().is_empty()).then_some(deltas));
    (text, usage)
}

async fn collect(
    response: Response<ResponseBody>,
) -> ForwardResult<(StatusCode, HeaderMap, Bytes)> {
    let (parts, body) = response.into_parts();
    let bytes = body
        .collect()
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "provider stream failed during compaction",
            )
        })?
        .to_bytes();
    Ok((parts.status, parts.headers, bytes))
}

fn compaction_item(summary: &str) -> Value {
    json!({
        "type": "compaction",
        "encrypted_content": format!(
            "{COMPACTION_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(summary.as_bytes())
        ),
    })
}

/// Wrap a relay's summarization answer as the single compaction item that
/// Codex's remote compaction expects.
pub(super) async fn compaction_response(response: Response<ResponseBody>) -> ForwardResult {
    let (status, headers, bytes) = collect(response).await?;
    if !status.is_success() {
        let mut rebuilt = Response::builder().status(status);
        for (name, value) in &headers {
            if name != CONTENT_LENGTH && name != TRANSFER_ENCODING {
                rebuilt = rebuilt.header(name, value);
            }
        }
        return Ok(rebuilt
            .body(full_body(String::from_utf8_lossy(&bytes).into_owned()))
            .expect("response parts"));
    }
    let (Some(summary), usage) = output_text(&bytes) else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "provider returned no compaction summary",
        ));
    };
    let id = format!("resp_agentharbor_{}", uuid::Uuid::new_v4().simple());
    let usage =
        usage.unwrap_or_else(|| json!({"input_tokens":0,"output_tokens":0,"total_tokens":0}));
    let events = [
        json!({"type":"response.created","response":{"id":id,"status":"in_progress"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":compaction_item(&summary)}),
        json!({"type":"response.completed","response":{"id":id,"status":"completed","usage":usage}}),
    ];
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap_or_default()
            )
        })
        .collect();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .body(full_body(body))
        .expect("static response parts"))
}

// ---- Converting official compactions for relays -------------------------

const CACHE_LIMIT: usize = 64;
static SUMMARIES: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
static OFFICIAL_MODELS: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

fn cache_key(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn remember(store: &OnceLock<Mutex<Vec<(String, String)>>>, key: String, value: String) {
    let mut entries = store
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    entries.retain(|(known, _)| *known != key);
    entries.push((key, value));
    if entries.len() > CACHE_LIMIT {
        entries.remove(0);
    }
}

fn recall(store: &OnceLock<Mutex<Vec<(String, String)>>>, key: &str) -> Option<String> {
    store
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, value)| value.clone())
}

pub(super) fn session_id(headers: &HeaderMap) -> Option<String> {
    [
        "session_id",
        "x-session-id",
        "x-codex-session-id",
        "conversation_id",
    ]
    .iter()
    .find_map(|name| headers.get(*name)?.to_str().ok())
    .filter(|value| !value.is_empty() && value.len() <= 256)
    .map(str::to_owned)
}

/// The account model last used in a session can decrypt its own compaction.
pub(super) fn remember_official_model(headers: &HeaderMap, model: Option<&str>) {
    if let (Some(session), Some(model)) = (session_id(headers), model.filter(|m| !m.is_empty())) {
        remember(&OFFICIAL_MODELS, session, model.to_owned());
    }
}

fn official_model(context: &ArcContext, headers: &HeaderMap) -> Option<String> {
    session_id(headers)
        .and_then(|session| recall(&OFFICIAL_MODELS, &session))
        .or_else(|| {
            super::super::model_catalog::official(&context.codex_home, &context.app_data_dir)
                .models
                .iter()
                .find(|model| model["visibility"].as_str() != Some("hide"))
                .and_then(|model| model["slug"].as_str().map(str::to_owned))
        })
}

/// Ask the account that produced an encrypted compaction for a readable
/// summary. Failures leave the item unconverted; the relay then receives a
/// notice instead of an opaque payload.
pub(super) async fn convert_official_compactions(
    context: &ArcContext,
    codex_auth: Option<(HeaderValue, Option<HeaderValue>)>,
    headers: &HeaderMap,
    body: Option<&Bytes>,
) -> HashMap<String, String> {
    let mut converted = HashMap::new();
    let pending = official_compactions(body);
    if pending.is_empty() {
        return converted;
    }
    let Some(value) = parse(body) else {
        return converted;
    };
    let Some(model) = official_model(context, headers) else {
        return converted;
    };
    for content in pending {
        let key = cache_key(&content);
        if let Some(summary) = recall(&SUMMARIES, &key) {
            converted.insert(content, summary);
            continue;
        }
        // Everything up to and including that compaction is its context.
        let mut input: Vec<Value> = value["input"]
            .as_array()
            .into_iter()
            .flatten()
            .take_while(|item| foreign_compaction(item) != Some(content.as_str()))
            .filter(|item| own_summary(item).is_none() && item["type"] != "compaction_trigger")
            .cloned()
            .collect();
        input.push(json!({"type":"compaction","encrypted_content":content}));
        input.push(json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": SUMMARIZATION_PROMPT}],
        }));
        let mut request = value.clone();
        request["model"] = json!(model);
        request["input"] = Value::Array(input);
        request["stream"] = json!(true);
        request["store"] = json!(false);
        let Ok((url, attempt_headers)) = official_target(
            context,
            codex_auth.clone(),
            "responses",
            None,
            headers.clone(),
        ) else {
            break;
        };
        let Ok(client) = client_for(context, &Destination::Official) else {
            break;
        };
        let response = client
            .request(hyper::Method::POST, url)
            .headers(attempt_headers)
            .header(CONTENT_TYPE, "application/json")
            .body(request.to_string())
            .send()
            .await;
        let Ok(response) = response else { break };
        if !response.status().is_success() {
            log::warn!(
                "official compaction conversion failed: {}",
                response.status()
            );
            break;
        }
        let Ok(bytes) = response.bytes().await else {
            break;
        };
        if let (Some(summary), _) = output_text(&bytes) {
            remember(&SUMMARIES, key, summary.clone());
            converted.insert(content, summary);
        }
    }
    converted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(value: Value) -> Option<Bytes> {
        Some(Bytes::from(value.to_string()))
    }

    fn parsed(bytes: Option<Bytes>) -> Value {
        serde_json::from_slice(&bytes.unwrap()).unwrap()
    }

    #[test]
    fn builtin_entry_requires_token_and_v1_prefix() {
        assert_eq!(
            builtin_entry("/openai/tok/v1/responses"),
            Some(("/codex/v1/responses".into(), "tok"))
        );
        assert_eq!(
            builtin_entry("/openai/tok/v1"),
            Some(("/codex/v1".into(), "tok"))
        );
        assert_eq!(builtin_entry("/openai//v1/responses"), None);
        assert_eq!(builtin_entry("/openai/tok/v2/responses"), None);
        assert_eq!(builtin_entry("/openai/tok/v1x/responses"), None);
        assert_eq!(builtin_entry("/codex/v1/responses"), None);
    }

    #[test]
    fn zstd_bodies_are_decoded_and_the_encoding_header_removed() {
        let original = br#"{"model":"gpt","input":[]}"#;
        let compressed = zstd::stream::encode_all(&original[..], 3).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::header::CONTENT_ENCODING,
            HeaderValue::from_static("zstd"),
        );
        let decoded = decode_body(&mut headers, Bytes::from(compressed)).unwrap();
        assert_eq!(decoded.as_ref(), original);
        assert!(headers.get(hyper::header::CONTENT_ENCODING).is_none());
        let mut plain = HeaderMap::new();
        assert_eq!(
            decode_body(&mut plain, Bytes::from_static(original))
                .unwrap()
                .as_ref(),
            original
        );
        let mut broken = headers.clone();
        broken.insert(
            hyper::header::CONTENT_ENCODING,
            HeaderValue::from_static("zstd"),
        );
        assert!(decode_body(&mut broken, Bytes::from_static(b"not zstd")).is_err());
    }

    #[test]
    fn relay_history_keeps_visible_items_and_drops_first_party_state() {
        let request = json!({
            "model": "relay::m",
            "include": ["reasoning.encrypted_content"],
            "stream_options": {"reasoning_summary_delivery": "sequential_cutoff"},
            "input": [
                {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}],
                 "internal_chat_message_metadata_passthrough":{"k":1}},
                {"type":"reasoning","summary":[],"encrypted_content":"official"},
                {"type":"function_call","call_id":"c1","name":"shell","arguments":"{}","encrypted_function_args":"x"},
                {"type":"function_call_output","call_id":"c1","output":"ok"},
                {"type":"configuration_update","settings":{}},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]},
            ],
        });
        let value = parsed(prepare_relay(body(request), &HashMap::new()).unwrap());
        let types: Vec<_> = value["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "message",
                "function_call",
                "function_call_output",
                "message"
            ]
        );
        assert!(value["input"][0]
            .get("internal_chat_message_metadata_passthrough")
            .is_none());
        assert!(value["input"][1].get("encrypted_function_args").is_none());
        assert!(value.get("include").is_none());
        assert!(value.get("stream_options").is_none());
    }

    #[test]
    fn compactions_become_readable_for_each_destination() {
        let ours = compaction_item("progress so far");
        let request = json!({"model":"m","input":[
            ours,
            {"type":"compaction","encrypted_content":"official-opaque"},
            {"type":"reasoning","summary":[],"encrypted_content":"official"},
            {"type":"reasoning","summary":[{"type":"summary_text","text":"relay"}]},
        ]});
        let official = parsed(prepare_official(body(request.clone())).unwrap());
        let items = official["input"].as_array().unwrap();
        assert_eq!(items.len(), 3, "{official}");
        assert!(items[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("progress so far"));
        assert_eq!(items[1]["encrypted_content"], "official-opaque");
        assert_eq!(items[2]["encrypted_content"], "official");

        assert_eq!(
            official_compactions(body(request.clone()).as_ref()),
            ["official-opaque"]
        );
        let mut converted = HashMap::new();
        converted.insert("official-opaque".to_owned(), "account summary".to_owned());
        let relay = parsed(prepare_relay(body(request.clone()), &converted).unwrap());
        let items = relay["input"].as_array().unwrap();
        assert_eq!(items.len(), 2, "{relay}");
        assert!(items[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("account summary"));
        let fallback = parsed(prepare_relay(body(request), &HashMap::new()).unwrap());
        assert_eq!(
            fallback["input"][1]["content"][0]["text"],
            UNREADABLE_COMPACTION
        );
    }

    #[test]
    fn official_retry_strips_only_reasoning() {
        let request = Bytes::from(
            json!({"input":[
                {"type":"reasoning","encrypted_content":"x"},
                {"type":"message","role":"user","content":[]}
            ]})
            .to_string(),
        );
        let retried: Value = serde_json::from_slice(&without_reasoning(&request).unwrap()).unwrap();
        assert_eq!(retried["input"].as_array().unwrap().len(), 1);
        assert!(rejected_reasoning(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"code":"invalid_encrypted_content"}}"#
        ));
        assert!(!rejected_reasoning(
            StatusCode::BAD_REQUEST,
            br#"{"error":"bad model"}"#
        ));
    }

    #[tokio::test]
    async fn relay_summary_is_wrapped_as_one_compaction_item() {
        let request = json!({"model":"relay::m","input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"task"}]},
            {"type":"compaction_trigger"}
        ]});
        assert!(compaction_trigger(body(request.clone()).as_ref()));
        let summary_request = parsed(summarization_request(body(request)).unwrap());
        let items = summary_request["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1]["content"][0]["text"], SUMMARIZATION_PROMPT);

        let upstream = concat!(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"sum\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"summary text\"}]}],\"usage\":{\"input_tokens\":5,\"output_tokens\":2,\"total_tokens\":7}}}\n\n",
        );
        let response = Response::builder()
            .status(200)
            .body(full_body(upstream.into()))
            .unwrap();
        let wrapped = compaction_response(response).await.unwrap();
        let (_, _, bytes) = collect(wrapped).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let item = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str::<Value>(data).unwrap())
            .find(|event| event["type"] == "response.output_item.done")
            .unwrap()["item"]
            .clone();
        assert_eq!(own_summary(&item).as_deref(), Some("summary text"));
        assert!(text.contains("\"input_tokens\":5"));

        let empty = Response::builder()
            .status(200)
            .body(full_body("data: {}\n\n".into()))
            .unwrap();
        assert!(compaction_response(empty).await.is_err());
    }
}
