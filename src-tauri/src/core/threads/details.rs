//! Read-only visualization of an already located Codex JSONL rollout.
//! List scanning stays unchanged; transcript bodies only leave the backend when
//! a message page is requested. Statistics scan the file without retaining text.
use super::{extract_chat_message, record_message, CoreError, SeenMessage};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

#[path = "details_index.rs"]
mod index;

const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 20_000;
const PAGE_SIZE: usize = 50;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: Option<u64>,
    pub reasoning: Option<u64>,
    pub total: u64,
}

impl Usage {
    fn parse(value: &Value) -> Option<Self> {
        let input = value.get("input_tokens")?.as_u64()?;
        let output = value.get("output_tokens")?.as_u64()?;
        Some(Self {
            input,
            output,
            cached: value
                .get("cached_input_tokens")
                .and_then(Value::as_u64)
                .filter(|n| *n <= input),
            reasoning: value
                .get("reasoning_output_tokens")
                .and_then(Value::as_u64)
                .filter(|n| *n <= output),
            total: value
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .max(input.saturating_add(output)),
        })
    }

    fn delta(&self, previous: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            output: self.output.saturating_sub(previous.output),
            cached: self
                .cached
                .zip(previous.cached)
                .map(|(a, b)| a.saturating_sub(b)),
            reasoning: self
                .reasoning
                .zip(previous.reasoning)
                .map(|(a, b)| a.saturating_sub(b)),
            total: self.total.saturating_sub(previous.total),
        }
    }

    fn add(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.total = self.total.saturating_add(other.total);
        self.cached = self
            .cached
            .zip(other.cached)
            .map(|(a, b)| a.saturating_add(b));
        self.reasoning = self
            .reasoning
            .zip(other.reasoning)
            .map(|(a, b)| a.saturating_add(b));
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub label: String,
    pub usage: Usage,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCount {
    pub name: String,
    pub calls: usize,
}

#[derive(Clone, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub invalid_lines: usize,
    pub oversized_lines: usize,
    pub encrypted_records: usize,
    pub compactions: usize,
    pub counter_resets: usize,
    pub last_usage_only: usize,
    pub undated_usage: usize,
    pub inherited_sessions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetails {
    pub snapshot_bytes: u64,
    pub snapshot_id: String,
    pub usage: Option<Usage>,
    pub by_model: Vec<UsageBucket>,
    pub by_date: Vec<UsageBucket>,
    pub tools: Vec<ToolCount>,
    pub diagnostics: Diagnostics,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String,
    pub timestamp: String,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub next_offset: Option<usize>,
    pub diagnostics: Diagnostics,
}

// No arbitrary path IPC. Reject traversal, symlinks and a different session ID.
fn resolve_path(sessions: &Path, archived: &Path, key: &str) -> Result<PathBuf, CoreError> {
    let (scope, relative) = key
        .split_once(':')
        .ok_or_else(|| CoreError::InvalidData("invalid thread key".into()))?;
    let root = match scope {
        "active" => sessions,
        "archived" => archived,
        _ => return Err(CoreError::NotFound("thread has no local rollout".into())),
    };
    if relative.contains('\\')
        || relative.contains(':')
        || relative
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(CoreError::InvalidData("invalid thread locator".into()));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            return Err(CoreError::InvalidData("invalid thread locator".into()));
        };
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(CoreError::InvalidData(
                "linked rollout paths are not supported".into(),
            ));
        }
    }
    if !super::is_rollout_file(&path) || !path.canonicalize()?.starts_with(root.canonicalize()?) {
        return Err(CoreError::InvalidData(
            "rollout outside thread source".into(),
        ));
    }
    Ok(path)
}

fn open_snapshot(
    path: &Path,
    snapshot: Option<u64>,
) -> Result<(BufReader<std::io::Take<File>>, u64), CoreError> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let bytes = snapshot.unwrap_or(length);
    if bytes > MAX_FILE_BYTES || length < bytes || bytes == 0 {
        return Err(CoreError::InvalidData(
            "rollout too large, empty or changed; refresh or open the original file".into(),
        ));
    }
    Ok((BufReader::new(file.take(bytes)), bytes))
}

// Discard oversized lines without allocating their full body.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<(bool, bool, u64)> {
    line.clear();
    let mut oversized = false;
    let mut read = false;
    let mut consumed = 0;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok((read, oversized, consumed));
        }
        read = true;
        let end = buffer.iter().position(|b| *b == b'\n').map(|i| i + 1);
        let count = end.unwrap_or(buffer.len());
        if !oversized && line.len() + count <= MAX_LINE_BYTES {
            line.extend_from_slice(&buffer[..count]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(count);
        consumed += count as u64;
        if end.is_some() {
            return Ok((true, oversized, consumed));
        }
    }
}

fn visit(
    reader: &mut impl BufRead,
    id: &str,
    diagnostics: &mut Diagnostics,
    mut callback: impl FnMut(&Value, &mut Diagnostics, u64) -> bool,
) -> Result<(), CoreError> {
    let mut line = Vec::new();
    let mut final_session_id = None;
    let mut visiting = true;
    let mut position = 0;
    loop {
        let start = position;
        let (read, oversized, consumed) = read_line(reader, &mut line)?;
        position += consumed;
        if !read {
            break;
        }
        if oversized {
            diagnostics.oversized_lines += 1;
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            diagnostics.invalid_lines += 1;
            continue;
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "session_meta"
            || value.pointer("/payload/type").and_then(Value::as_str) == Some("session_meta")
        {
            let actual = value.get("payload").and_then(super::session_metadata_id);
            final_session_id = actual.map(str::to_owned);
            if actual.is_some() && actual != Some(id) {
                diagnostics.inherited_sessions += 1;
            }
        }
        // Forked rollouts can contain ancestor metadata before their own final
        // session_meta. Collect in memory, but return nothing until the final ID
        // is verified, even when the requested message page is already full.
        if value.pointer("/payload/encrypted_content").is_some() {
            diagnostics.encrypted_records += 1;
        }
        if kind == "compacted" {
            diagnostics.compactions += 1;
        }
        if visiting {
            visiting = callback(&value, diagnostics, start);
        }
    }
    if final_session_id.as_deref() != Some(id) {
        return Err(CoreError::InvalidData(
            "rollout final session ID mismatch or missing metadata".into(),
        ));
    }
    Ok(())
}

fn timestamp(value: &Value) -> &str {
    value.get("timestamp").and_then(Value::as_str).unwrap_or("")
}

fn tool_name(value: &Value) -> Option<&str> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    let payload = value.get("payload")?;
    match payload.get("type")?.as_str()? {
        "function_call" | "custom_tool_call" => payload.get("name")?.as_str(),
        "local_shell_call" => Some("local_shell"),
        _ => None,
    }
}

pub fn read_details(
    sessions: &Path,
    archived: &Path,
    key: &str,
    id: &str,
) -> Result<ThreadDetails, CoreError> {
    let path = resolve_path(sessions, archived, key)?;
    let (mut reader, snapshot_bytes) = open_snapshot(&path, None)?;
    let mut diagnostics = Diagnostics::default();
    let mut usage: Option<Usage> = None;
    let mut previous: Option<Usage> = None;
    let mut last_only = None;
    let mut model = "unknown".to_owned();
    let mut by_model = BTreeMap::<String, Usage>::new();
    let mut by_date = BTreeMap::<String, Usage>::new();
    let mut tools = BTreeMap::<String, usize>::new();
    let fingerprint = index::stamp(reader.get_ref().get_ref())?;
    let mut parser = MessageParser::default();
    let mut positions = Vec::new();
    visit(
        &mut reader,
        id,
        &mut diagnostics,
        |value, diagnostics, position| {
            if parser.candidate(value).is_some() {
                positions.push(position);
            }
            if let Some(name) = tool_name(value) {
                let name: String = name.chars().take(200).collect();
                if tools.len() < 1000 || tools.contains_key(&name) {
                    *tools.entry(name).or_default() += 1;
                }
            }
            let record_type = value.get("type").and_then(Value::as_str);
            if record_type == Some("session_meta") {
                model = "unknown".into();
            }
            if matches!(record_type, Some("turn_context" | "session_meta")) {
                if let Some(next) = value.pointer("/payload/model").and_then(Value::as_str) {
                    model = next.chars().take(200).collect();
                }
            }
            if value.pointer("/payload/type").and_then(Value::as_str)
                == Some("thread_settings_applied")
            {
                if let Some(next) = value
                    .pointer("/payload/thread_settings/model")
                    .and_then(Value::as_str)
                {
                    model = next.chars().take(200).collect();
                }
            }
            if value.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
                return true;
            }
            if let Some(next) = value.pointer("/payload/info/model").and_then(Value::as_str) {
                model = next.chars().take(200).collect();
            }
            if value
                .pointer("/payload/info/total_token_usage")
                .is_none_or(Value::is_null)
                && value
                    .pointer("/payload/info/last_token_usage")
                    .is_none_or(Value::is_null)
            {
                return true;
            }
            let delta = if let Some(total) = value
                .pointer("/payload/info/total_token_usage")
                .and_then(Usage::parse)
            {
                if previous.as_ref() == Some(&total) {
                    return true;
                }
                let delta = match &previous {
                    Some(old)
                        if total.total >= old.total
                            && total.input >= old.input
                            && total.output >= old.output =>
                    {
                        total.delta(old)
                    }
                    Some(_) => {
                        diagnostics.counter_resets += 1;
                        total.clone()
                    }
                    None => total.clone(),
                };
                previous = Some(total);
                delta
            } else if let Some(last) = value
                .pointer("/payload/info/last_token_usage")
                .and_then(Usage::parse)
            {
                let fingerprint = (timestamp(value).to_owned(), last.clone());
                if last_only.as_ref() == Some(&fingerprint) {
                    return true;
                }
                last_only = Some(fingerprint);
                diagnostics.last_usage_only += 1;
                // A later cumulative snapshot includes these already-accounted calls.
                match &mut previous {
                    Some(sum) => sum.add(&last),
                    None => previous = Some(last.clone()),
                }
                last
            } else {
                diagnostics.invalid_lines += 1;
                return true;
            };
            if delta.total == 0 {
                return true;
            }
            match &mut usage {
                Some(sum) => sum.add(&delta),
                None => usage = Some(delta.clone()),
            }
            let model_key = if by_model.len() < 1000 || by_model.contains_key(&model) {
                model.clone()
            } else {
                "unknown".into()
            };
            by_model
                .entry(model_key)
                .and_modify(|sum| sum.add(&delta))
                .or_insert_with(|| delta.clone());
            let date = chrono::DateTime::parse_from_rfc3339(timestamp(value))
                .ok()
                .map(|t| t.date_naive().to_string());
            let date = date.unwrap_or_else(|| {
                diagnostics.undated_usage += 1;
                "unknown".into()
            });
            let date = if by_date.len() < 5000 || by_date.contains_key(&date) {
                date
            } else {
                "unknown".into()
            };
            by_date
                .entry(date)
                .and_modify(|sum| sum.add(&delta))
                .or_insert(delta);
            true
        },
    )?;
    index::ensure_unchanged(&path, reader.get_ref().get_ref(), &fingerprint)?;
    let snapshot_id = index::token(&fingerprint);
    index::remember(
        &path,
        fingerprint,
        snapshot_bytes,
        id,
        positions,
        diagnostics.clone(),
    );
    let buckets = |map: BTreeMap<String, Usage>| {
        map.into_iter()
            .map(|(label, usage)| UsageBucket { label, usage })
            .collect()
    };
    Ok(ThreadDetails {
        snapshot_bytes,
        snapshot_id,
        usage,
        by_model: buckets(by_model),
        by_date: buckets(by_date),
        tools: tools
            .into_iter()
            .map(|(name, calls)| ToolCount { name, calls })
            .collect(),
        diagnostics,
    })
}

#[derive(Default)]
struct MessageParser {
    seen: Vec<SeenMessage>,
    parsed: Vec<super::ChatMessage>,
    title: Option<String>,
    users: usize,
    agents: usize,
}
impl MessageParser {
    fn candidate(&mut self, value: &Value) -> Option<(String, String)> {
        let mut candidate = None;
        if let Some((role, text, format)) = extract_chat_message(value) {
            record_message(
                role,
                timestamp(value),
                text,
                format,
                true,
                &mut self.seen,
                &mut self.title,
                &mut self.users,
                &mut self.agents,
                &mut self.parsed,
            );
            if let Some(message) = self.parsed.pop() {
                candidate = Some((message.role.to_owned(), message.text));
            }
            if self.seen.len() > 512 {
                self.seen.drain(..self.seen.len() - 512);
            }
        } else if let Some(payload) = value.get("payload") {
            let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
            if value.get("type").and_then(Value::as_str) == Some("response_item") {
                match kind {
                    "function_call" | "custom_tool_call" | "local_shell_call" => {
                        let name = tool_name(value).unwrap_or("tool");
                        let args = payload
                            .get("arguments")
                            .or_else(|| payload.get("input"))
                            .or_else(|| payload.get("action"));
                        candidate = Some((
                            "toolCall".into(),
                            format!(
                                "{name}\n{}",
                                args.map(|v| v
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| v.to_string()))
                                    .unwrap_or_default()
                            ),
                        ));
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let text = payload
                            .get("output")
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| v.to_string())
                            })
                            .unwrap_or_default();
                        candidate = Some(("toolResult".into(), text));
                    }
                    "reasoning" => {
                        let text = payload
                            .get("summary")
                            .and_then(Value::as_array)
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        if !text.is_empty() {
                            candidate = Some(("reasoning".into(), text));
                        }
                    }
                    _ => {}
                }
            } else if value.get("type").and_then(Value::as_str) == Some("compacted") {
                if let Some(text) = payload.get("message").and_then(Value::as_str) {
                    candidate = Some(("compaction".into(), text.into()));
                }
            }
        }
        candidate
    }
}

pub fn read_messages(
    sessions: &Path,
    archived: &Path,
    key: &str,
    id: &str,
    snapshot_bytes: u64,
    offset: usize,
) -> Result<MessagePage, CoreError> {
    read_messages_versioned(sessions, archived, key, id, snapshot_bytes, offset, None)
}

pub fn read_messages_versioned(
    sessions: &Path,
    archived: &Path,
    key: &str,
    id: &str,
    snapshot_bytes: u64,
    offset: usize,
    snapshot_id: Option<&str>,
) -> Result<MessagePage, CoreError> {
    let path = resolve_path(sessions, archived, key)?;
    index::page(&path, id, snapshot_bytes, offset, snapshot_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(records: Vec<Value>) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let archived = dir.path().join("archived");
        fs::create_dir_all(&sessions).unwrap();
        let meta = json!({"type":"session_meta","payload":{"id":"test"}});
        let text = std::iter::once(meta)
            .chain(records)
            .map(|v| v.to_string() + "\n")
            .collect::<String>();
        fs::write(sessions.join("rollout-test.jsonl"), text).unwrap();
        (dir, sessions, archived)
    }
    fn token(input: u64, output: u64, cached: u64, reasoning: u64) -> Value {
        json!({"type":"event_msg","timestamp":"2026-09-30T01:00:00Z","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":cached,"reasoning_output_tokens":reasoning,"total_tokens":input+output},
            "last_token_usage":{"input_tokens":input,"output_tokens":output}
        }}})
    }
    fn context(model: &str) -> Value {
        json!({"type":"turn_context","payload":{"model":model}})
    }
    #[test]
    fn indexed_pages_invalidate_truncation_replacement_and_same_length_edits() {
        use std::io::Write;
        let (_dir,s,a)=fixture((0..120).map(|i| json!({"type":"event_msg","payload":{"type":"user_message","message":format!("message {i:03}")}})).collect());
        let path = s.join("rollout-test.jsonl");
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        let page = read_messages_versioned(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            100,
            Some(&detail.snapshot_id),
        )
        .unwrap();
        assert_eq!(page.messages[0].text, "message 100");
        assert_eq!(page.messages.len(), 20);
        let original = fs::read(&path).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let changed = String::from_utf8(original.clone())
            .unwrap()
            .replace("message 100", "CHANGED 100");
        assert_eq!(changed.len(), original.len());
        fs::write(&path, changed).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(read_messages_versioned(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            100,
            Some(&detail.snapshot_id)
        )
        .is_err());
        let next = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(
            read_messages_versioned(
                &s,
                &a,
                "active:rollout-test.jsonl",
                "test",
                next.snapshot_bytes,
                100,
                Some(&next.snapshot_id)
            )
            .unwrap()
            .messages[0]
                .text,
            "CHANGED 100"
        );
        let replacement = s.join("replacement.tmp");
        fs::write(&replacement, &original).unwrap();
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        fs::rename(&path, s.join("old.tmp")).unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(read_messages_versioned(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            next.snapshot_bytes,
            100,
            Some(&next.snapshot_id)
        )
        .is_err());
        let replaced = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{}\n").unwrap();
        drop(file);
        assert!(read_messages_versioned(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            replaced.snapshot_bytes,
            100,
            Some(&replaced.snapshot_id)
        )
        .is_err());
        fs::write(&path, b"{}\n").unwrap();
        assert!(read_messages(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            replaced.snapshot_bytes,
            100
        )
        .is_err());
    }

    #[test]
    #[ignore = "Synthetic large history benchmark; no user transcript access"]
    fn indexed_history_paging_benchmark() {
        let records=(0..10_000).map(|i| json!({"type":"event_msg","payload":{"type":"user_message","message":format!("message {i}: {}", "x".repeat(2000))}})).collect();
        let (_dir, s, a) = fixture(records);
        let key = "active:rollout-test.jsonl";
        let started = std::time::Instant::now();
        let detail = read_details(&s, &a, key, "test").unwrap();
        let build = started.elapsed();
        let started = std::time::Instant::now();
        for offset in (9000..10000).step_by(PAGE_SIZE) {
            let page = read_messages_versioned(
                &s,
                &a,
                key,
                "test",
                detail.snapshot_bytes,
                offset,
                Some(&detail.snapshot_id),
            )
            .unwrap();
            assert_eq!(page.messages.len(), PAGE_SIZE);
            assert!(page.messages[0]
                .text
                .starts_with(&format!("message {offset}:")));
        }
        let indexed = started.elapsed();
        let started = std::time::Instant::now();
        for _ in (9000..10000).step_by(PAGE_SIZE) {
            let (mut reader, _) =
                open_snapshot(&s.join("rollout-test.jsonl"), Some(detail.snapshot_bytes)).unwrap();
            let mut parser = MessageParser::default();
            visit(
                &mut reader,
                "test",
                &mut Diagnostics::default(),
                |v, _, _| {
                    parser.candidate(v);
                    true
                },
            )
            .unwrap();
        }
        println!("history benchmark: bytes={}, build_ms={}, twenty_late_pages_ms={}, twenty_full_rescans_ms={}",detail.snapshot_bytes,build.as_millis(),indexed.as_millis(),started.elapsed().as_millis());
    }
    #[test]
    fn snapshots_are_deduplicated_and_attributed_to_recorded_context() {
        let (dir, s, a) = fixture(vec![
            context("official"),
            token(100, 20, 40, 5),
            token(100, 20, 40, 5),
            context("relay::model"),
            token(300, 50, 130, 15),
        ]);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(
            detail.usage.unwrap(),
            Usage {
                input: 300,
                output: 50,
                cached: Some(130),
                reasoning: Some(15),
                total: 350
            }
        );
        assert_eq!(detail.by_model[0].usage.total, 120);
        assert_eq!(detail.by_model[1].usage.total, 230);
        assert_eq!(detail.by_date[0].usage.total, 350);
        drop(dir);
    }
    #[test]
    fn last_only_then_cumulative_does_not_double_count() {
        let mut last = token(100, 20, 40, 5);
        let info = last
            .pointer_mut("/payload/info")
            .unwrap()
            .as_object_mut()
            .unwrap();
        let usage = info.remove("total_token_usage").unwrap();
        info.insert("last_token_usage".into(), usage);
        let (_dir, s, a) = fixture(vec![last, token(100, 20, 40, 5), token(200, 40, 80, 10)]);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.usage.unwrap().total, 240);
        assert_eq!(detail.diagnostics.last_usage_only, 1);
    }
    #[test]
    fn unknown_breakdown_and_counter_resets_are_not_hidden() {
        let mut unknown = token(10, 2, 0, 0);
        unknown
            .pointer_mut("/payload/info/total_token_usage")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
        let (_dir, s, a) = fixture(vec![token(100, 20, 40, 5), unknown]);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.diagnostics.counter_resets, 1);
        assert_eq!(detail.usage.unwrap().cached, None);
    }
    #[test]
    fn pages_include_tools_and_summaries_but_never_ciphertext() {
        let mut records = vec![
            json!({"type":"response_item","payload":{"type":"reasoning","encrypted_content":"DO_NOT_EXPOSE","summary":[{"text":"readable summary"}]}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"pwd"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"ok"}}),
        ];
        for i in 0..55 {
            records.push(json!({"type":"event_msg","timestamp":format!("2026-09-30T01:00:{i:02}Z"),"payload":{"type":"agent_message","message":format!("message {i}")}}));
        }
        let (_dir, s, a) = fixture(records);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.tools[0].calls, 1);
        assert_eq!(detail.diagnostics.encrypted_records, 1);
        let first = read_messages(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            0,
        )
        .unwrap();
        assert_eq!(first.messages.len(), 50);
        assert_eq!(first.next_offset, Some(50));
        assert!(!serde_json::to_string(&first)
            .unwrap()
            .contains("DO_NOT_EXPOSE"));
        let second = read_messages(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            50,
        )
        .unwrap();
        assert_eq!(second.messages.len(), 8);
        assert_eq!(second.next_offset, None);
    }
    #[test]
    fn shared_message_parser_pairs_response_and_event_without_losing_repeated_turns() {
        let records = vec![
            json!({"type":"response_item","timestamp":"2026-09-30T01:00:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}),
            json!({"type":"event_msg","timestamp":"2026-09-30T01:00:01Z","payload":{"type":"user_message","message":"hello"}}),
            json!({"type":"event_msg","timestamp":"2026-09-30T01:00:20Z","payload":{"type":"user_message","message":"hello"}}),
        ];
        let (_dir, s, a) = fixture(records);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(
            read_messages(
                &s,
                &a,
                "active:rollout-test.jsonl",
                "test",
                detail.snapshot_bytes,
                0
            )
            .unwrap()
            .messages
            .len(),
            2
        );
    }
    #[test]
    fn identity_traversal_and_stale_snapshot_fail_closed() {
        let (_dir, s, a) = fixture(vec![]);
        for key in [
            "active:../rollout-test.jsonl",
            "active:C:/rollout-test.jsonl",
            "active:./rollout-test.jsonl",
            "active:/rollout-test.jsonl",
            "active:rollout-test.jsonl:stream",
            "catalog:test",
        ] {
            assert!(read_details(&s, &a, key, "test").is_err(), "{key}");
        }
        assert!(read_details(&s, &a, "active:rollout-test.jsonl", "wrong").is_err());
        assert!(read_messages(&s, &a, "active:rollout-test.jsonl", "test", u64::MAX, 0).is_err());
        assert!(read_messages(&s, &a, "active:rollout-test.jsonl", "test", 0, 0).is_err());
    }

    #[test]
    fn forked_history_is_visible_only_when_final_metadata_matches_even_past_page_end() {
        let mut records = vec![json!({"type":"session_meta","payload":{"id":"parent"}})];
        for i in 0..55 {
            records.push(json!({"type":"event_msg","payload":{"type":"user_message","message":format!("inherited {i}")}}));
        }
        records.push(json!({"type":"session_meta","payload":{"id":"test"}}));
        let (_dir, s, a) = fixture(records);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.diagnostics.inherited_sessions, 1);
        let page = read_messages(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            0,
        )
        .unwrap();
        assert_eq!(page.messages.len(), 50);
        assert_eq!(page.messages[0].text, "inherited 0");
        assert!(
            read_messages(
                &s,
                &a,
                "active:rollout-test.jsonl",
                "parent",
                detail.snapshot_bytes,
                0
            )
            .is_err(),
            "Must validate identity beyond the collected page"
        );
    }

    #[test]
    fn legacy_session_id_has_the_same_precedence_as_the_thread_list() {
        let (_dir, s, a) = fixture(vec![
            json!({"type":"session_meta","payload":{"id":"legacy-trace-id","session_id":"test"}}),
        ]);
        assert!(read_details(&s, &a, "active:rollout-test.jsonl", "test").is_ok());
        assert!(read_details(&s, &a, "active:rollout-test.jsonl", "legacy-trace-id").is_err());
    }

    #[test]
    fn absent_usage_is_normal_and_settings_changes_have_explicit_model_attribution() {
        let (_dir, s, a) = fixture(vec![
            json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
            json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"explicit-settings-model"}}}),
            token(100, 20, 40, 5),
        ]);
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.diagnostics.invalid_lines, 0);
        assert_eq!(detail.by_model[0].label, "explicit-settings-model");
    }
    #[test]
    fn oversized_corrupt_lines_and_long_text_are_bounded() {
        let (_dir, s, a) = fixture(vec![
            json!({"type":"event_msg","payload":{"type":"user_message","message":"中".repeat(MAX_TEXT_CHARS+10)}}),
        ]);
        let path = s.join("rollout-test.jsonl");
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        file.write_all(b"{invalid}\n").unwrap();
        file.write_all(&vec![b'x'; MAX_LINE_BYTES + 10]).unwrap();
        file.write_all(b"\n").unwrap();
        let detail = read_details(&s, &a, "active:rollout-test.jsonl", "test").unwrap();
        assert_eq!(detail.diagnostics.invalid_lines, 1);
        assert_eq!(detail.diagnostics.oversized_lines, 1);
        let page = read_messages(
            &s,
            &a,
            "active:rollout-test.jsonl",
            "test",
            detail.snapshot_bytes,
            0,
        )
        .unwrap();
        assert!(page.messages[0].truncated);
        assert_eq!(page.messages[0].text.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    #[ignore = "Explicit local read-only probe; reports only aggregate counts, never transcript bodies"]
    fn local_rollout_read_only_probe() {
        let home = std::env::var("AGENTHARBOR_DETAILS_PROBE_HOME").expect("explicit Codex home");
        let s = Path::new(&home).join("sessions");
        let a = Path::new(&home).join("archived_sessions");
        let scan = super::super::scan_threads_with_warnings(&s, &a).unwrap();
        let mut checked = 0;
        let cutoff = chrono::Utc::now().timestamp() - 300;
        for thread in scan.items.iter().filter(|t| t.updated_at < cutoff).take(3) {
            let path = Path::new(&thread.file_path);
            let before = fs::read(path).unwrap();
            let detail = read_details(&s, &a, &thread.key, &thread.id).unwrap();
            let page =
                read_messages(&s, &a, &thread.key, &thread.id, detail.snapshot_bytes, 0).unwrap();
            assert_eq!(
                before,
                fs::read(path).unwrap(),
                "probe must not modify rollout"
            );
            println!("local rollout: usage={}, models={}, toolKinds={}, messages={}, encrypted={}, compactions={}",detail.usage.is_some(),detail.by_model.len(),detail.tools.len(),page.messages.len(),detail.diagnostics.encrypted_records,detail.diagnostics.compactions);
            checked += 1;
        }
        assert!(checked > 0);
    }
}
