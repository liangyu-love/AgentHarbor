//! Claude Code project transcripts, independent from the Codex catalog.
use super::{
    client_config::ClientPaths,
    models::{CoreError, CoreWarning},
    threads::{
        details::{
            Diagnostics, Message, MessagePage, ThreadDetails, ToolCount, Usage, UsageBucket,
        },
        ThreadExportFormat, ThreadExportResult, ThreadScanResult, ThreadSummary,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IDENTITIES: usize = 100_000;
const MAX_FILES: usize = 10_000;
const MAX_WARNINGS: usize = 100;
const MAX_TEXT_CHARS: usize = 20_000;
const PAGE_SIZE: usize = 50;
const CACHE_LIMIT: usize = 4096;

mod deletion;
pub use deletion::delete_threads;

pub fn projects_root() -> PathBuf {
    ClientPaths::discover()
        .claude
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("projects")
}

#[derive(Clone)]
struct CachedSummary {
    bytes: u64,
    modified: SystemTime,
    summary: ThreadSummary,
    unreadable_lines: usize,
    oversized_lines: usize,
}

fn summary_cache() -> &'static Mutex<HashMap<PathBuf, CachedSummary>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedSummary>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn warning(warnings: &mut Vec<CoreWarning>, path: &Path, error: impl std::fmt::Display) {
    if warnings.len() < MAX_WARNINGS {
        warnings.push(CoreWarning {
            code: "CLAUDE_SESSION_UNREADABLE".into(),
            message: format!("{}: {error}", path.display()),
        });
    }
}

fn partial_warning(warnings: &mut Vec<CoreWarning>, path: &Path, invalid: usize, oversized: usize) {
    if invalid + oversized > 0 {
        warning(
            warnings,
            path,
            format!("skipped {invalid} invalid and {oversized} oversized transcript lines"),
        );
    }
}

fn session_id(path: &Path) -> Option<String> {
    if path.extension()?.to_str()? != "jsonl" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    uuid::Uuid::parse_str(stem).ok()?;
    Some(stem.to_owned())
}

fn locator(root: &Path, path: &Path) -> Result<String, CoreError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        CoreError::InvalidData("Claude session is outside its projects root".into())
    })?;
    let parts = relative
        .components()
        .map(|part| match part {
            Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| CoreError::InvalidData("invalid Claude session path".into()))?;
    if parts.len() != 2 {
        return Err(CoreError::InvalidData("invalid Claude session path".into()));
    }
    Ok(format!("claude:{}", parts.join("/")))
}

fn resolve(root: &Path, key: &str, id: Option<&str>) -> Result<PathBuf, CoreError> {
    let relative = key
        .strip_prefix("claude:")
        .ok_or_else(|| CoreError::InvalidData("invalid Claude session key".into()))?;
    if relative.contains(['\\', ':']) {
        return Err(CoreError::InvalidData(
            "invalid Claude session locator".into(),
        ));
    }
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(CoreError::InvalidData(
            "invalid Claude session locator".into(),
        ));
    }
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(CoreError::InvalidData(
            "linked Claude projects root is unsupported".into(),
        ));
    }
    let mut path = root.to_path_buf();
    for part in parts {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(CoreError::InvalidData(
                "linked Claude session is unsupported".into(),
            ));
        }
    }
    let actual = session_id(&path)
        .ok_or_else(|| CoreError::InvalidData("invalid Claude session filename".into()))?;
    if id.is_some_and(|id| id != actual) || !path.canonicalize()?.starts_with(root.canonicalize()?)
    {
        return Err(CoreError::InvalidData(
            "Claude session identity mismatch".into(),
        ));
    }
    Ok(path)
}

pub fn scan_threads(root: &Path) -> Result<ThreadScanResult, CoreError> {
    let projects = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ThreadScanResult {
                items: vec![],
                warnings: vec![],
            });
        }
        Err(error) => return Err(error.into()),
    };
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(CoreError::InvalidData(
            "linked Claude projects root is unsupported".into(),
        ));
    }
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    // Main sessions are exactly projects/<encoded-project>/<UUID>.jsonl.
    // Sidecars and subagent transcripts must not appear as independent chats.
    for project in projects {
        let project = match project {
            Ok(entry) => entry,
            Err(error) => {
                warning(&mut warnings, root, error);
                continue;
            }
        };
        let kind = match project.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                warning(&mut warnings, &project.path(), error);
                continue;
            }
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let entries = match fs::read_dir(project.path()) {
            Ok(entries) => entries,
            Err(error) => {
                warning(&mut warnings, &project.path(), error);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warning(&mut warnings, &project.path(), error);
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    warning(&mut warnings, &path, error);
                    continue;
                }
            };
            if kind.is_file() && !kind.is_symlink() && session_id(&path).is_some() {
                if files.len() >= MAX_FILES {
                    warning(&mut warnings, root, "session scan limit reached");
                    break;
                }
                files.push(path);
            }
        }
        if files.len() >= MAX_FILES {
            break;
        }
    }
    files.sort();
    let mut items = Vec::new();
    for path in files {
        let result = (|| {
            let metadata = fs::metadata(&path)?;
            let modified = metadata.modified()?;
            let cached = summary_cache()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&path)
                .filter(|entry| entry.bytes == metadata.len() && entry.modified == modified)
                .cloned();
            if let Some(cached) = cached {
                partial_warning(
                    &mut warnings,
                    &path,
                    cached.unreadable_lines,
                    cached.oversized_lines,
                );
                return Ok(cached.summary);
            }
            let id = session_id(&path).unwrap();
            let key = locator(root, &path)?;
            let mut summary = SummaryBuilder::default();
            let result = visit(&path, &id, None, |value| summary.observe(value))?;
            if summary.users == 0 && summary.agents == 0 {
                return Err(CoreError::InvalidData(
                    "no readable conversation messages".into(),
                ));
            }
            let summary = summary.finish(&path, key, id, result.bytes);
            partial_warning(
                &mut warnings,
                &path,
                result.diagnostics.invalid_lines,
                result.diagnostics.oversized_lines,
            );
            let mut cache = summary_cache()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if cache.len() >= CACHE_LIMIT && !cache.contains_key(&path) {
                if let Some(stale) = cache.keys().next().cloned() {
                    cache.remove(&stale);
                }
            }
            cache.insert(
                path.clone(),
                CachedSummary {
                    bytes: metadata.len(),
                    modified,
                    summary: summary.clone(),
                    unreadable_lines: result.diagnostics.invalid_lines,
                    oversized_lines: result.diagnostics.oversized_lines,
                },
            );
            Ok::<_, CoreError>(summary)
        })();
        match result {
            Ok(summary) => items.push(summary),
            Err(error) => warning(&mut warnings, &path, error),
        }
    }
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.key.cmp(&b.key)));
    Ok(ThreadScanResult { items, warnings })
}

struct Snapshot {
    bytes: u64,
    token: String,
    diagnostics: Diagnostics,
}

// Hash every consumed byte, including invalid/oversized lines. Pages can use an
// unchanged prefix when Claude appends, but cannot mix a replaced transcript.
fn visit(
    path: &Path,
    id: &str,
    bytes: Option<u64>,
    mut callback: impl FnMut(&Value) -> Result<(), CoreError>,
) -> Result<Snapshot, CoreError> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let bytes = bytes.unwrap_or(length);
    if bytes == 0 || bytes > MAX_FILE_BYTES || length < bytes {
        return Err(CoreError::InvalidData(
            "Claude transcript is empty, too large, or truncated".into(),
        ));
    }
    let mut reader = BufReader::new(file.take(bytes));
    let mut hash = Sha256::new();
    let mut diagnostics = Diagnostics::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        let mut oversized = false;
        let mut read = false;
        loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            read = true;
            let end = buffer
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1);
            let count = end.unwrap_or(buffer.len());
            hash.update(&buffer[..count]);
            if !oversized && line.len() + count <= MAX_LINE_BYTES {
                line.extend_from_slice(&buffer[..count]);
            } else {
                oversized = true;
                line.clear();
            }
            reader.consume(count);
            if end.is_some() {
                break;
            }
        }
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
        let value = match serde_json::from_slice::<Value>(&line) {
            Ok(value) => value,
            Err(_) => {
                diagnostics.invalid_lines += 1;
                continue;
            }
        };
        if let Some(actual) = value.get("sessionId").and_then(Value::as_str) {
            if actual != id {
                return Err(CoreError::InvalidData(
                    "Claude transcript session ID mismatch".into(),
                ));
            }
        }
        if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") {
            diagnostics.compactions += 1;
        }
        callback(&value)?;
    }
    let remaining = reader.get_ref().limit();
    if remaining != 0 {
        return Err(CoreError::InvalidData(
            "Claude transcript changed while reading".into(),
        ));
    }
    Ok(Snapshot {
        bytes,
        token: format!("{:x}", hash.finalize()),
        diagnostics,
    })
}

fn short(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
// IDs normally fit in a UUID-sized string. Hash unusually long IDs instead of
// truncating them, so the bounded dedupe keys cannot alias distinct records.
fn identity(value: &str) -> String {
    if value.len() <= 256 {
        value.to_owned()
    } else {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }
}
fn remember(seen: &mut HashSet<String>, value: &str, limit: usize) -> Result<bool, CoreError> {
    let value = identity(value);
    if seen.contains(&value) {
        return Ok(false);
    }
    if seen.len() >= limit {
        return Err(CoreError::InvalidData(
            "Claude transcript has too many message identities".into(),
        ));
    }
    seen.insert(value);
    Ok(true)
}
fn timestamp(value: &Value) -> &str {
    value.get("timestamp").and_then(Value::as_str).unwrap_or("")
}
fn seconds(value: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp(value))
        .ok()
        .map(|time| time.timestamp())
}
fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block.get("text").and_then(Value::as_str).map(str::to_owned),
                Some("image") => Some("[image]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn tool_result_text(value: &Value) -> String {
    let text = content_text(value);
    if !text.trim().is_empty() {
        text
    } else if !value.is_null() {
        value.to_string()
    } else {
        String::new()
    }
}

#[derive(Default)]
struct MessageParser {
    seen: HashSet<String>,
}
impl MessageParser {
    fn candidates(&mut self, value: &Value) -> Result<Vec<(String, String)>, CoreError> {
        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return Ok(vec![]);
        }
        let compact = value.get("subtype").and_then(Value::as_str) == Some("compact_boundary");
        let role = value
            .pointer("/message/role")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !compact && !matches!(role, "user" | "assistant") {
            return Ok(vec![]);
        }
        if let Some(uuid) = value.get("uuid").and_then(Value::as_str) {
            if !remember(&mut self.seen, uuid, MAX_IDENTITIES)? {
                return Ok(vec![]);
            }
        }
        if compact {
            let text = value
                .get("content")
                .map(content_text)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "Conversation compacted".into());
            return Ok(vec![("compaction".into(), text)]);
        }
        let Some(message) = value.get("message") else {
            return Ok(vec![]);
        };
        let Some(content) = message.get("content") else {
            return Ok(vec![]);
        };
        if let Some(text) = content.as_str() {
            return Ok(if text.trim().is_empty() {
                vec![]
            } else {
                vec![(role.into(), text.to_owned())]
            });
        }
        let Some(blocks) = content.as_array() else {
            return Ok(vec![]);
        };
        let mut candidates = Vec::new();
        for block in blocks {
            let candidate = match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| (role.to_owned(), text.to_owned())),
                Some("thinking") => block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|text| ("reasoning".into(), text.to_owned())),
                Some("redacted_thinking") => {
                    Some(("reasoning".into(), "[redacted thinking]".into()))
                }
                Some("tool_use" | "server_tool_use") => Some((
                    "toolCall".into(),
                    format!(
                        "{}\n{}",
                        block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        block.get("input").map(Value::to_string).unwrap_or_default()
                    ),
                )),
                Some(kind) if kind == "tool_result" || kind.ends_with("_tool_result") => Some((
                    "toolResult".into(),
                    block
                        .get("content")
                        .map(tool_result_text)
                        .unwrap_or_default(),
                )),
                Some("image") => Some((role.to_owned(), "[image]".into())),
                _ => None,
            };
            if let Some(candidate) = candidate {
                if !candidate.1.trim().is_empty() {
                    candidates.push(candidate);
                }
            }
        }
        Ok(candidates)
    }
}

#[derive(Default)]
struct UsageTracker {
    messages: HashMap<String, RecordedUsage>,
    anonymous: Vec<RecordedUsage>,
}
struct RecordedUsage {
    usage: Usage,
    model: String,
    date: String,
}
fn add_usage(sum: &mut Usage, usage: &Usage) {
    sum.input = sum.input.saturating_add(usage.input);
    sum.output = sum.output.saturating_add(usage.output);
    sum.cached = sum
        .cached
        .zip(usage.cached)
        .map(|(a, b)| a.saturating_add(b));
    sum.total = sum.total.saturating_add(usage.total);
}
impl UsageTracker {
    fn observe(&mut self, value: &Value) -> Result<(), CoreError> {
        if value.pointer("/message/role").and_then(Value::as_str) != Some("assistant") {
            return Ok(());
        }
        let Some(usage) = value.pointer("/message/usage") else {
            return Ok(());
        };
        let (Some(input), Some(output)) = (
            usage.get("input_tokens").and_then(Value::as_u64),
            usage.get("output_tokens").and_then(Value::as_u64),
        ) else {
            return Ok(());
        };
        // Anthropic separates cache read/creation from input_tokens. Include both
        // in prompt total, while cached reports the read subset only.
        let cached = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
        let input = input.saturating_add(cached.unwrap_or(0)).saturating_add(
            usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        let current = Usage {
            input,
            output,
            cached,
            reasoning: None,
            total: input.saturating_add(output),
        };
        let id = value
            .pointer("/message/id")
            .and_then(Value::as_str)
            .or_else(|| value.get("uuid").and_then(Value::as_str));
        let model = short(
            value
                .pointer("/message/model")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            256,
        );
        let date = chrono::DateTime::parse_from_rfc3339(timestamp(value))
            .ok()
            .map(|time| time.date_naive().to_string())
            .unwrap_or_else(|| "unknown".into());
        if let Some(id) = id {
            let id = identity(id);
            if let Some(old) = self.messages.get_mut(&id) {
                old.usage.input = old.usage.input.max(current.input);
                old.usage.output = old.usage.output.max(current.output);
                old.usage.cached = match (old.usage.cached, current.cached) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                old.usage.total = old.usage.input.saturating_add(old.usage.output);
                if old.model == "unknown" {
                    old.model = model;
                }
                if old.date == "unknown" {
                    old.date = date;
                }
                return Ok(());
            }
            if self.messages.len() + self.anonymous.len() >= MAX_IDENTITIES {
                return Err(CoreError::InvalidData(
                    "Claude transcript has too many usage identities".into(),
                ));
            }
            self.messages.insert(
                id,
                RecordedUsage {
                    usage: current,
                    model,
                    date,
                },
            );
        } else {
            if self.messages.len() + self.anonymous.len() >= MAX_IDENTITIES {
                return Err(CoreError::InvalidData(
                    "Claude transcript has too many usage identities".into(),
                ));
            }
            self.anonymous.push(RecordedUsage {
                usage: current,
                model,
                date,
            });
        }
        Ok(())
    }
    fn records(&self) -> impl Iterator<Item = &RecordedUsage> {
        self.messages
            .values()
            .chain(self.anonymous.iter())
            .filter(|record| record.usage.total > 0)
    }
    fn total(&self) -> Option<Usage> {
        self.records()
            .map(|record| record.usage.clone())
            .reduce(|mut sum, usage| {
                add_usage(&mut sum, &usage);
                sum
            })
    }
}

#[derive(Default)]
struct SummaryBuilder {
    cwd: String,
    title: String,
    custom_title: String,
    cli_version: String,
    model: String,
    created: Option<i64>,
    updated: Option<i64>,
    users: usize,
    agents: usize,
    user_ids: HashSet<String>,
    assistant_ids: HashSet<String>,
    usage: UsageTracker,
}
impl SummaryBuilder {
    fn observe(&mut self, value: &Value) -> Result<(), CoreError> {
        if let Some(time) = seconds(value) {
            self.created = Some(self.created.map_or(time, |old| old.min(time)));
            self.updated = Some(self.updated.map_or(time, |old| old.max(time)));
        }
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
            self.cwd = short(cwd, 4096);
        }
        if let Some(version) = value.get("version").and_then(Value::as_str) {
            self.cli_version = short(version, 128);
        }
        if value.get("type").and_then(Value::as_str) == Some("custom-title") {
            if let Some(title) = value
                .get("customTitle")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
            {
                self.custom_title = short(title.trim(), 120);
            }
        }
        self.usage.observe(value)?;
        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let Some(message) = value.get("message") else {
            return Ok(());
        };
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content");
        let text = content.map(content_text).unwrap_or_default();
        if role == "user" {
            let only_tools = content.and_then(Value::as_array).is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks.iter().all(|block| {
                        block.get("type").and_then(Value::as_str) == Some("tool_result")
                    })
            });
            if !only_tools && !text.trim().is_empty() {
                let identity = value.get("uuid").and_then(Value::as_str);
                if match identity {
                    Some(id) => remember(&mut self.user_ids, id, MAX_IDENTITIES)?,
                    None => true,
                } {
                    self.users += 1;
                }
                if self.title.is_empty()
                    && !text.contains("<local-command-caveat>")
                    && !text.trim().starts_with("<command-name>")
                {
                    self.title = short(text.trim(), 120);
                }
            }
        } else if role == "assistant" {
            let identity = message
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("uuid").and_then(Value::as_str));
            if match identity {
                Some(id) => remember(&mut self.assistant_ids, id, MAX_IDENTITIES)?,
                None => true,
            } {
                self.agents += 1;
            }
            if let Some(model) = message.get("model").and_then(Value::as_str) {
                self.model = short(model, 256);
            }
        }
        Ok(())
    }
    fn finish(self, path: &Path, key: String, id: String, bytes: u64) -> ThreadSummary {
        let folder = self
            .cwd
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_owned();
        let timestamp = fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|time| time.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let usage = self.usage.total();
        let count = |value: u64| value.min(i64::MAX as u64) as i64;
        let title = if !self.custom_title.is_empty() {
            self.custom_title
        } else if !self.title.is_empty() {
            self.title
        } else if !folder.is_empty() {
            folder.clone()
        } else {
            id.clone()
        };
        ThreadSummary {
            key,
            id: id.clone(),
            cwd: self.cwd,
            folder_name: folder,
            title,
            created_at: self.created.unwrap_or(timestamp),
            updated_at: self.updated.unwrap_or(timestamp),
            user_messages: self.users,
            agent_messages: self.agents,
            file_size: bytes,
            archived: false,
            source: "claudeCode".into(),
            cli_version: self.cli_version,
            file_path: path.to_string_lossy().into_owned(),
            model_provider: String::new(),
            model: self.model,
            tokens_used: usage.as_ref().map_or(0, |usage| count(usage.total)),
            input_tokens: usage.as_ref().map(|usage| count(usage.input)),
            cached_tokens: usage.as_ref().and_then(|usage| usage.cached.map(count)),
            output_tokens: usage.as_ref().map(|usage| count(usage.output)),
            thread_source: "claudeJsonl".into(),
            rollout_status: "available".into(),
        }
    }
}

pub fn read_details(root: &Path, key: &str, id: &str) -> Result<ThreadDetails, CoreError> {
    let path = resolve(root, key, Some(id))?;
    let mut usage = UsageTracker::default();
    let mut models = BTreeMap::<String, Usage>::new();
    let mut dates = BTreeMap::<String, Usage>::new();
    let mut tools = BTreeMap::<String, usize>::new();
    let mut seen_tools = HashSet::new();
    let mut undated = 0;
    let snapshot = visit(&path, id, None, |value| {
        usage.observe(value)?;
        if let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) {
            for block in blocks {
                if matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("tool_use" | "server_tool_use")
                ) {
                    let identity = block.get("id").and_then(Value::as_str);
                    if let Some(id) = identity {
                        if !remember(&mut seen_tools, id, MAX_IDENTITIES)? {
                            continue;
                        }
                    }
                    let name = short(
                        block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        256,
                    );
                    if tools.len() < 1000 || tools.contains_key(&name) {
                        *tools.entry(name).or_default() += 1;
                    }
                }
            }
        }
        Ok(())
    })?;
    // Stream records can fill in usage fields over time. Aggregate each final
    // message maximum once, rather than poisoning totals with an early unknown
    // cache counter or spreading one message across duplicate stream records.
    for record in usage.records() {
        let model = if models.len() < 1000 || models.contains_key(&record.model) {
            record.model.as_str()
        } else {
            "other"
        };
        models
            .entry(model.to_owned())
            .and_modify(|sum| add_usage(sum, &record.usage))
            .or_insert_with(|| record.usage.clone());
        if record.date == "unknown" {
            undated += 1;
        }
        let date = if dates.len() < 5000 || dates.contains_key(&record.date) {
            record.date.as_str()
        } else {
            "other"
        };
        dates
            .entry(date.to_owned())
            .and_modify(|sum| add_usage(sum, &record.usage))
            .or_insert_with(|| record.usage.clone());
    }
    let buckets = |map: BTreeMap<String, Usage>| {
        map.into_iter()
            .map(|(label, usage)| UsageBucket { label, usage })
            .collect()
    };
    let mut diagnostics = snapshot.diagnostics;
    diagnostics.undated_usage = undated;
    Ok(ThreadDetails {
        snapshot_bytes: snapshot.bytes,
        snapshot_id: snapshot.token,
        usage: usage.total(),
        by_model: buckets(models),
        by_date: buckets(dates),
        tools: tools
            .into_iter()
            .map(|(name, calls)| ToolCount { name, calls })
            .collect(),
        diagnostics,
    })
}

pub fn read_messages_versioned(
    root: &Path,
    key: &str,
    id: &str,
    snapshot_bytes: u64,
    offset: usize,
    snapshot_id: Option<&str>,
) -> Result<MessagePage, CoreError> {
    let path = resolve(root, key, Some(id))?;
    let mut parser = MessageParser::default();
    let mut count = 0usize;
    let mut messages = Vec::new();
    let snapshot = visit(&path, id, Some(snapshot_bytes), |value| {
        for (role, text) in parser.candidates(value)? {
            if count >= offset && messages.len() < PAGE_SIZE {
                let truncated = text.chars().count() > MAX_TEXT_CHARS;
                messages.push(Message {
                    role,
                    timestamp: short(timestamp(value), 128),
                    text: short(&text, MAX_TEXT_CHARS),
                    truncated,
                });
            }
            count = count.saturating_add(1);
        }
        Ok(())
    })?;
    if snapshot_id.is_some_and(|token| token != snapshot.token) {
        return Err(CoreError::InvalidData(
            "Claude transcript changed; refresh the thread details".into(),
        ));
    }
    Ok(MessagePage {
        next_offset: (count > offset.saturating_add(messages.len()))
            .then_some(offset.saturating_add(messages.len())),
        messages,
        diagnostics: snapshot.diagnostics,
    })
}

pub fn export_threads(
    root: &Path,
    keys: &[String],
    target: &Path,
    format: ThreadExportFormat,
) -> Result<ThreadExportResult, CoreError> {
    fs::create_dir_all(target)?;
    let mut selected = HashSet::new();
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| {
        for key in keys {
            if !selected.insert(key) {
                continue;
            }
            let path = resolve(root, key, None)?;
            let id = session_id(&path).unwrap();
            let extension = if format == ThreadExportFormat::Markdown {
                "md"
            } else {
                "jsonl"
            };
            let base = format!("claude-{id}");
            let mut name = format!("{base}.{extension}");
            let mut index = 2;
            while target.join(&name).exists()
                || staged
                    .iter()
                    .any(|(_, destination)| destination == &target.join(&name))
            {
                name = format!("{base}-{index}.{extension}");
                index += 1;
            }
            let temporary =
                target.join(format!(".agentharbor-export-{}.tmp", uuid::Uuid::new_v4()));
            let output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            staged.push((temporary, target.join(name)));
            let mut output = std::io::BufWriter::new(output);
            if format == ThreadExportFormat::Markdown {
                writeln!(output, "# Claude Code · {id}\n")?;
                let mut parser = MessageParser::default();
                visit(&path, &id, None, |value| {
                    for (role, text) in parser.candidates(value)? {
                        writeln!(output, "## {role} {}\n\n{text}\n", timestamp(value))?;
                    }
                    Ok(())
                })?;
            } else {
                let snapshot = visit(&path, &id, None, |_| Ok(()))?;
                let mut input = File::open(&path)?.take(snapshot.bytes);
                let copied = std::io::copy(&mut input, &mut output)?;
                if copied != snapshot.bytes {
                    return Err(CoreError::InvalidData(
                        "Claude transcript changed during export".into(),
                    ));
                }
                output.flush()?;
                let exported = visit(&staged.last().unwrap().0, &id, Some(snapshot.bytes), |_| {
                    Ok(())
                })?;
                if exported.token != snapshot.token {
                    return Err(CoreError::InvalidData(
                        "Claude transcript changed during export".into(),
                    ));
                }
            }
            output.flush()?;
            output.get_ref().sync_all()?;
        }
        let mut committed = Vec::new();
        for (temporary, destination) in &staged {
            // create_new also works on export volumes without hard-link support
            // and cannot overwrite an existing file if another export wins.
            let publish = (|| {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)?;
                committed.push(destination.clone());
                let mut input = File::open(temporary)?;
                std::io::copy(&mut input, &mut output)?;
                output.sync_all()?;
                Ok::<_, std::io::Error>(())
            })();
            if let Err(error) = publish {
                for path in committed {
                    let _ = fs::remove_file(path);
                }
                return Err(error.into());
            }
        }
        Ok::<_, CoreError>(ThreadExportResult {
            exported: staged.len(),
            directory: target.to_string_lossy().into_owned(),
            files: staged
                .iter()
                .map(|(_, path)| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect(),
        })
    })();
    for (temporary, _) in staged {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests;
