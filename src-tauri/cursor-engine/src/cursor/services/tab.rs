//! Cursor Tab. Left to Cursor, these calls pass through untouched. With a
//! model chosen, they are answered here: file sync is reported off so Cursor
//! sends the file with each request, edit history is accepted, and StreamCpp
//! asks the model to rewrite the few lines around the cursor and streams the
//! result back as one suggested edit.
use std::{io::Read, sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Extension, State},
    http::{header, HeaderValue, Request, Response, StatusCode},
    routing::post,
    Router,
};
use futures_util::StreamExt;
use prost::Message;
use tokio_util::sync::CancellationToken;

use crate::{
    api::cursor::proxy::{self, CursorProxy},
    cursor::{protocol::connect, transport::TransportRegistry},
    host::TabMode,
    model::{
        ContentPart, ModelConfig, ModelInvocation, ModelRequest, ModelSpec, ProjectedContent,
        ProjectedMessage, PromptSpec, ReasoningSpec, Role,
    },
    provider::{ModelEvent, Provider},
    Error, Result,
};

const STREAM_CPP: &str = "/aiserver.v1.AiService/StreamCpp";
const NEXT_CURSOR: &str = "/aiserver.v1.AiService/StreamNextCursorPrediction";

/// Calls answered here while a model serves Tab.
pub const TAB_PATHS: [&str; 9] = [
    STREAM_CPP,
    NEXT_CURSOR,
    "/aiserver.v1.AiService/CppAppend",
    "/aiserver.v1.AiService/CppEditHistoryAppend",
    "/aiserver.v1.AiService/CppEditHistoryStatus",
    "/aiserver.v1.CppService/RecordCppFate",
    "/aiserver.v1.FileSyncService/FSIsEnabledForUser",
    "/aiserver.v1.FileSyncService/FSSyncFile",
    "/aiserver.v1.FileSyncService/FSUploadFile",
];

/// Lines of the file shown above and below the cursor.
const CONTEXT_ABOVE: usize = 60;
const CONTEXT_BELOW: usize = 30;
/// The lines the model may rewrite, around the cursor.
const EDIT_ABOVE: usize = 2;
const EDIT_BELOW: usize = 6;
const MAX_LINE_CHARS: usize = 400;
const MAX_HISTORY_CHARS: usize = 6_000;
const MAX_OUTPUT_TOKENS: u64 = 1_024;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(20);

const CURSOR_MARK: &str = "<|cursor|>";
const REGION_START: &str = "<|editable_region_start|>";
const REGION_END: &str = "<|editable_region_end|>";

const INSTRUCTIONS: &str = "You predict the next edit in a code editor. You are shown a file with an editable region marked by <|editable_region_start|> and <|editable_region_end|>, the cursor position marked <|cursor|>, and the user's recent edits.

Rewrite the editable region the way the user is most likely to write it next: complete the line or statement at the cursor, or continue the change their recent edits are making. Keep everything else in the region as it is. If there is nothing useful to add, return the region unchanged.

Reply with the rewritten region only: the lines between the two region markers, without the markers, without the cursor mark, without code fences and without any explanation.";

pub fn is_tab_path(path: &str) -> bool {
    TAB_PATHS.contains(&path)
}

pub fn router() -> Router<TransportRegistry> {
    TAB_PATHS.into_iter().fold(Router::new(), |router, path| router.route(path, post(handle)))
}

async fn handle(
    State(registry): State<TransportRegistry>,
    Extension(upstream): Extension<CursorProxy>,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let TabMode::Model { model_id } = registry.host().settings().tab else {
        return proxy::forward(Extension(upstream), request).await;
    };
    let path = request.uri().path().to_owned();
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX)
        .await
        .map_err(|error| Error::Protocol(format!("cannot read request body: {error}")))?;
    let streaming = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/connect+"));
    match path.as_str() {
        STREAM_CPP => {
            let request: StreamCppRequest = decode(&body)?;
            let Some(model) = registry.host().model(&model_id) else {
                return Ok(stream_response(Vec::new()));
            };
            let provider = registry.conversations().dependencies().provider.clone();
            Ok(stream_cpp(provider, model, request))
        }
        NEXT_CURSOR => Ok(stream_response(Vec::new())),
        "/aiserver.v1.AiService/CppAppend" => Ok(unary(&CppAppendResponse { success: true }, streaming)),
        "/aiserver.v1.AiService/CppEditHistoryStatus" => {
            Ok(unary(&CppEditHistoryStatusResponse { on: true, only_if_explicit: false }, streaming))
        }
        "/aiserver.v1.FileSyncService/FSIsEnabledForUser" => Ok(unary(&FsIsEnabledForUserResponse { enabled: false }, streaming)),
        _ => Ok(unary(&Empty {}, streaming)),
    }
}

/// A unary answer, framed when the call came as a Connect stream.
fn unary(message: &impl Message, streaming: bool) -> Response<Body> {
    if streaming {
        let frames = vec![connect::encode_message(message).expect("encoding to a Vec cannot fail")];
        return stream_response(frames);
    }
    let mut response = Response::new(Body::from(message.encode_to_vec()));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/proto"));
    response
}

fn stream_response(mut frames: Vec<Bytes>) -> Response<Body> {
    frames.push(connect::encode_end_stream());
    streaming_body(Body::from_stream(futures_util::stream::iter(
        frames.into_iter().map(Ok::<_, std::convert::Infallible>),
    )))
}

fn streaming_body(body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/connect+proto"));
    response
}

/// One enveloped message, gzip-compressed or not, or a bare one.
fn decode<M: Message + Default>(body: &[u8]) -> Result<M> {
    if body.len() >= 5 {
        let flags = body[0];
        let length = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
        if length == body.len() - 5 && flags & connect::END_STREAM_FLAG == 0 {
            let payload = &body[5..];
            if flags & 1 != 0 {
                let mut plain = Vec::new();
                flate2::read::GzDecoder::new(payload)
                    .read_to_end(&mut plain)
                    .map_err(|error| Error::Protocol(format!("cannot inflate Tab request: {error}")))?;
                return Ok(M::decode(plain.as_slice())?);
            }
            return Ok(M::decode(payload)?);
        }
    }
    Ok(M::decode(body)?)
}

fn stream_cpp(provider: Arc<dyn Provider>, model: ModelConfig, request: StreamCppRequest) -> Response<Body> {
    let body = async_stream::stream! {
        let frames = match suggest(provider, &model, &request).await {
            Ok(Some(edit)) => edit_frames(edit),
            Ok(None) => Vec::new(),
            Err(error) => {
                tracing::debug!(%error, "Tab suggestion failed");
                Vec::new()
            }
        };
        for frame in frames {
            yield Ok::<_, std::convert::Infallible>(frame);
        }
        yield Ok(connect::encode_end_stream());
    };
    streaming_body(Body::from_stream(body))
}

fn edit_frames(edit: Edit) -> Vec<Bytes> {
    let binding_id = uuid::Uuid::new_v4().to_string();
    let messages = [
        StreamCppResponse {
            range_to_replace: Some(LineRange {
                start_line_number: edit.start_line as i32,
                end_line_number_inclusive: edit.end_line as i32,
            }),
            binding_id: Some(binding_id.clone()),
            should_remove_leading_eol: Some(false),
            ..Default::default()
        },
        StreamCppResponse { text: edit.text, ..Default::default() },
        StreamCppResponse { done_edit: Some(true), binding_id: Some(binding_id), ..Default::default() },
        StreamCppResponse { done_stream: Some(true), ..Default::default() },
    ];
    messages.iter().filter_map(|message| connect::encode_message(message).ok()).collect()
}

/// A suggested edit: replace lines `start_line..=end_line` (1-based) with `text`.
#[derive(Debug, PartialEq)]
struct Edit {
    start_line: usize,
    end_line: usize,
    text: String,
}

/// The file around the cursor, cut for one request.
#[derive(Debug, PartialEq)]
struct Window {
    /// 0-based index of the first editable line in the file.
    edit_start: usize,
    /// 0-based index of the last editable line.
    edit_end: usize,
    /// The editable lines as they are now.
    original: Vec<String>,
    /// The prompt's view of the file, with the region and cursor marked.
    marked: String,
}

fn window(contents: &str, cursor_line: usize, cursor_column: usize) -> Option<Window> {
    let lines: Vec<&str> = contents.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    if lines.is_empty() || cursor_line >= lines.len() {
        return None;
    }
    let edit_start = cursor_line.saturating_sub(EDIT_ABOVE);
    let edit_end = (cursor_line + EDIT_BELOW).min(lines.len() - 1);
    let first = cursor_line.saturating_sub(CONTEXT_ABOVE);
    let last = (cursor_line + CONTEXT_BELOW).min(lines.len() - 1);
    let mut marked = String::new();
    for (index, line) in lines.iter().enumerate().take(last + 1).skip(first) {
        if index == edit_start {
            marked.push_str(REGION_START);
            marked.push('\n');
        }
        let line: String = line.chars().take(MAX_LINE_CHARS).collect();
        if index == cursor_line {
            let split = line.char_indices().nth(cursor_column).map_or(line.len(), |(at, _)| at);
            marked.push_str(&line[..split]);
            marked.push_str(CURSOR_MARK);
            marked.push_str(&line[split..]);
        } else {
            marked.push_str(&line);
        }
        marked.push('\n');
        if index == edit_end {
            marked.push_str(REGION_END);
            marked.push('\n');
        }
    }
    Some(Window {
        edit_start,
        edit_end,
        original: lines[edit_start..=edit_end].iter().map(|line| (*line).to_owned()).collect(),
        marked,
    })
}

fn user_message(request: &StreamCppRequest, file: &CurrentFileInfo, window: &Window) -> String {
    let mut sections = Vec::new();
    let history = recent_edits(request);
    if !history.is_empty() {
        sections.push(format!("Recent edits, oldest first:\n{history}"));
    }
    let language = if file.language_id.is_empty() { String::new() } else { format!(" ({})", file.language_id) };
    sections.push(format!("File: {}{language}\n{}", file.relative_workspace_path, window.marked));
    sections.join("\n\n")
}

fn recent_edits(request: &StreamCppRequest) -> String {
    let mut edits: Vec<String> = request.diff_history.iter().map(|diff| diff.trim().to_owned()).collect();
    for file in &request.file_diff_histories {
        for diff in &file.diff_history {
            edits.push(format!("{}:\n{}", file.file_name, diff.trim()));
        }
    }
    edits.retain(|edit| !edit.is_empty());
    let mut kept = Vec::new();
    let mut size = 0;
    for edit in edits.iter().rev() {
        size += edit.len();
        if size > MAX_HISTORY_CHARS {
            break;
        }
        kept.push(edit.as_str());
    }
    kept.reverse();
    kept.join("\n\n")
}

async fn suggest(provider: Arc<dyn Provider>, model: &ModelConfig, request: &StreamCppRequest) -> Result<Option<Edit>> {
    let Some(file) = request.current_file.as_ref() else { return Ok(None) };
    if file.contents.is_empty() {
        return Ok(None);
    }
    let cursor = file.cursor_position.clone().unwrap_or_default();
    let offset = usize::try_from(file.contents_start_at_line).unwrap_or(0).saturating_sub(1);
    let cursor_line = usize::try_from(cursor.line).unwrap_or(0).saturating_sub(offset);
    let cursor_column = usize::try_from(cursor.column).unwrap_or(0);
    let Some(window) = window(&file.contents, cursor_line, cursor_column) else { return Ok(None) };
    let invocation = invocation(model, user_message(request, file, &window));
    let output = generate(provider, invocation).await?;
    let replacement = clean(&output);
    let Some(replacement) = replacement else { return Ok(None) };
    if replacement.split('\n').eq(window.original.iter().map(String::as_str)) {
        return Ok(None);
    }
    Ok(Some(Edit {
        start_line: offset + window.edit_start + 1,
        end_line: offset + window.edit_end + 1,
        text: replacement,
    }))
}

fn invocation(model: &ModelConfig, user: String) -> ModelInvocation {
    let call_id = format!("tab-{}", uuid::Uuid::new_v4());
    // As little thinking as the model allows: Tab has to be quick.
    let effort = model
        .efforts
        .iter()
        .find(|effort| matches!(effort.as_str(), "none" | "minimal"))
        .or_else(|| model.efforts.first())
        .cloned();
    ModelInvocation {
        call_id: call_id.clone(),
        run_id: call_id.clone(),
        conversation_id: call_id,
        provider_call_index: 0,
        request: ModelRequest {
            prompt: PromptSpec { instructions: INSTRUCTIONS.into(), tools: Vec::new() },
            model: ModelSpec {
                max_output_tokens: Some(MAX_OUTPUT_TOKENS),
                reasoning: ReasoningSpec { enabled: effort.as_deref().is_some_and(|effort| effort != "none"), effort },
                ..ModelSpec::new(model.model_hash.clone())
            },
            history: vec![ProjectedMessage {
                message_id: "tab".into(),
                role: Role::User,
                content: ProjectedContent::Parts(vec![ContentPart::Text { text: user }]),
            }],
        },
    }
}

async fn generate(provider: Arc<dyn Provider>, invocation: ModelInvocation) -> Result<String> {
    let cancellation = CancellationToken::new();
    let stream = provider.stream(invocation, cancellation.clone());
    let collected = tokio::time::timeout(GENERATION_TIMEOUT, async {
        futures_util::pin_mut!(stream);
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                ModelEvent::TextDelta(delta) => text.push_str(&delta),
                ModelEvent::Done(_) => return Ok(text),
                _ => {}
            }
        }
        Err(Error::Provider("Tab model stream ended early".into()))
    })
    .await;
    match collected {
        Ok(result) => result,
        Err(_) => {
            cancellation.cancel();
            Err(Error::Provider("Tab model timed out".into()))
        }
    }
}

/// The model's region without fences, markers or the cursor mark.
fn clean(output: &str) -> Option<String> {
    let mut text = output.replace('\r', "");
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let mut lines: Vec<&str> = trimmed.lines().skip(1).collect();
        if lines.last().is_some_and(|line| line.trim_start().starts_with("```")) {
            lines.pop();
        }
        text = lines.join("\n");
    }
    for marker in [REGION_START, REGION_END, CURSOR_MARK] {
        text = text.replace(marker, "");
    }
    let text = text.trim_matches('\n').to_owned();
    (!text.trim().is_empty()).then_some(text)
}

// ---- wire messages (aiserver.v1, the fields Tab uses) -----------------------

#[derive(Clone, PartialEq, Message)]
struct StreamCppRequest {
    #[prost(message, optional, tag = "1")]
    current_file: Option<CurrentFileInfo>,
    #[prost(string, repeated, tag = "2")]
    diff_history: Vec<String>,
    #[prost(message, repeated, tag = "7")]
    file_diff_histories: Vec<CppFileDiffHistory>,
}

#[derive(Clone, PartialEq, Message)]
struct CurrentFileInfo {
    #[prost(string, tag = "1")]
    relative_workspace_path: String,
    #[prost(string, tag = "2")]
    contents: String,
    #[prost(message, optional, tag = "3")]
    cursor_position: Option<CursorPosition>,
    #[prost(string, tag = "5")]
    language_id: String,
    #[prost(int32, tag = "9")]
    contents_start_at_line: i32,
}

#[derive(Clone, PartialEq, Message)]
struct CursorPosition {
    #[prost(int32, tag = "1")]
    line: i32,
    #[prost(int32, tag = "2")]
    column: i32,
}

#[derive(Clone, PartialEq, Message)]
struct CppFileDiffHistory {
    #[prost(string, tag = "1")]
    file_name: String,
    #[prost(string, repeated, tag = "2")]
    diff_history: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
struct StreamCppResponse {
    #[prost(string, tag = "1")]
    text: String,
    #[prost(bool, optional, tag = "4")]
    done_stream: Option<bool>,
    #[prost(message, optional, tag = "11")]
    range_to_replace: Option<LineRange>,
    #[prost(bool, optional, tag = "13")]
    done_edit: Option<bool>,
    #[prost(bool, optional, tag = "16")]
    should_remove_leading_eol: Option<bool>,
    #[prost(string, optional, tag = "17")]
    binding_id: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct LineRange {
    #[prost(int32, tag = "1")]
    start_line_number: i32,
    #[prost(int32, tag = "2")]
    end_line_number_inclusive: i32,
}

#[derive(Clone, Copy, PartialEq, Message)]
struct Empty {}

#[derive(Clone, Copy, PartialEq, Message)]
struct CppAppendResponse {
    #[prost(bool, tag = "1")]
    success: bool,
}

#[derive(Clone, Copy, PartialEq, Message)]
struct CppEditHistoryStatusResponse {
    #[prost(bool, tag = "1")]
    on: bool,
    #[prost(bool, tag = "2")]
    only_if_explicit: bool,
}

#[derive(Clone, Copy, PartialEq, Message)]
struct FsIsEnabledForUserResponse {
    #[prost(bool, tag = "1")]
    enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_marks_the_region_and_the_cursor() {
        let contents = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let window = window(&contents, 9, 4).unwrap();
        assert_eq!(window.edit_start, 7);
        assert_eq!(window.edit_end, 15);
        assert_eq!(window.original.first().unwrap(), "line 8");
        assert!(window.marked.contains("line<|cursor|> 10"));
        assert!(window.marked.contains("<|editable_region_start|>\nline 8\n"));
        assert!(window.marked.contains("line 16\n<|editable_region_end|>\n"));
    }

    #[test]
    fn the_cursor_at_the_edges_stays_in_range() {
        let window = window("only line", 0, 99).unwrap();
        assert_eq!((window.edit_start, window.edit_end), (0, 0));
        assert!(window.marked.contains("only line<|cursor|>"));
        assert!(super::window("a\nb", 5, 0).is_none());
    }

    #[test]
    fn model_output_loses_fences_and_markers() {
        assert_eq!(clean("```rust\nlet a = 1;<|cursor|>\n```").as_deref(), Some("let a = 1;"));
        assert_eq!(clean("<|editable_region_start|>\nx\n<|editable_region_end|>").as_deref(), Some("x"));
        assert_eq!(clean("  \n"), None);
    }

    #[test]
    fn an_edit_is_range_text_done_then_end_of_stream() {
        let frames = edit_frames(Edit { start_line: 3, end_line: 5, text: "new".into() });
        assert_eq!(frames.len(), 4);
        let decoded: Vec<StreamCppResponse> = frames
            .iter()
            .map(|frame| StreamCppResponse::decode(&frame[5..]).unwrap())
            .collect();
        assert_eq!(decoded[0].range_to_replace, Some(LineRange { start_line_number: 3, end_line_number_inclusive: 5 }));
        assert_eq!(decoded[0].binding_id, decoded[2].binding_id);
        assert_eq!(decoded[1].text, "new");
        assert_eq!(decoded[2].done_edit, Some(true));
        assert_eq!(decoded[3].done_stream, Some(true));
    }

    #[test]
    fn gzip_requests_are_inflated() {
        use std::io::Write;
        let message = StreamCppRequest { diff_history: vec!["edit".into()], ..Default::default() }.encode_to_vec();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&message).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut framed = vec![1];
        framed.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        framed.extend_from_slice(&compressed);
        let decoded: StreamCppRequest = decode(&framed).unwrap();
        assert_eq!(decoded.diff_history, vec!["edit".to_owned()]);
    }
}
