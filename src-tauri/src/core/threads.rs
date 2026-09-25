use crate::core::models::{CoreError, CoreWarning};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    /// Stable locator for UI selection and export. Session IDs are not guaranteed to be unique.
    pub key: String,
    pub id: String,
    pub cwd: String,
    pub folder_name: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub user_messages: usize,
    pub agent_messages: usize,
    pub file_size: u64,
    pub archived: bool,
    pub source: String,
    pub cli_version: String,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadExportResult {
    pub exported: usize,
    pub directory: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadExportFormat {
    Markdown,
    Jsonl,
}

impl ThreadExportFormat {
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        match value {
            "markdown" | "md" => Ok(Self::Markdown),
            "jsonl" => Ok(Self::Jsonl),
            other => Err(CoreError::InvalidData(format!(
                "unsupported export format: {other}"
            ))),
        }
    }
}

#[derive(Debug)]
struct ThreadFile {
    key: String,
    path: PathBuf,
    archived: bool,
}

#[derive(Debug)]
struct ChatMessage {
    role: &'static str,
    timestamp: String,
    text: String,
}

#[derive(Debug)]
struct ParsedThread {
    summary: ThreadSummary,
    messages: Vec<ChatMessage>,
}

pub struct ThreadScanResult {
    pub items: Vec<ThreadSummary>,
    pub warnings: Vec<CoreWarning>,
}

#[derive(Clone)]
struct CachedThread {
    fingerprint: (u64, SystemTime),
    summary: Option<ThreadSummary>,
}

const THREAD_CACHE_LIMIT: usize = 4096;
static THREAD_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedThread>>> = OnceLock::new();

fn file_fingerprint(path: &Path) -> Result<(u64, SystemTime), CoreError> {
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.len(), metadata.modified()?))
}

fn scan_warning(path: &Path, error: &CoreError) -> CoreWarning {
    CoreWarning {
        code: "thread_scan_skipped".into(),
        message: format!(
            "Skipped unreadable conversation {}: {error}",
            path.display()
        ),
    }
}

fn scan_error(
    warnings: &mut Option<&mut Vec<CoreWarning>>,
    path: &Path,
    error: CoreError,
) -> Result<(), CoreError> {
    match warnings {
        Some(warnings) => {
            warnings.push(scan_warning(path, &error));
            Ok(())
        }
        None => Err(error),
    }
}

/// Scan active and archived Codex rollout files.
pub fn scan_threads(
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<Vec<ThreadSummary>, CoreError> {
    Ok(scan_threads_with_warnings(sessions_dir, archived_dir)?.items)
}

pub fn scan_threads_with_warnings(
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<ThreadScanResult, CoreError> {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    collect_rollout_files(sessions_dir, false, &mut files, Some(&mut warnings))?;
    collect_rollout_files(archived_dir, true, &mut files, Some(&mut warnings))?;
    let cache = THREAD_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let seen: HashSet<_> = files.iter().map(|file| file.path.clone()).collect();
    for file in files {
        let parsed = (|| -> Result<Option<ThreadSummary>, CoreError> {
            let fingerprint = file_fingerprint(&file.path)?;
            let cached = cache
                .lock()
                .ok()
                .and_then(|entries| entries.get(&file.path).cloned())
                .filter(|entry| entry.fingerprint == fingerprint);
            if let Some(cached) = cached {
                return Ok(cached.summary.map(|mut summary| {
                    summary.key = file.key.clone();
                    summary.archived = file.archived;
                    summary
                }));
            }
            let summary = parse_thread(&file, false)?.map(|parsed| parsed.summary);
            // Do not cache a log being appended to or replaced during parsing.
            if file_fingerprint(&file.path).ok() == Some(fingerprint) {
                if let Ok(mut entries) = cache.lock() {
                    if entries.len() >= THREAD_CACHE_LIMIT && !entries.contains_key(&file.path) {
                        if let Some(key) = entries.keys().next().cloned() {
                            entries.remove(&key);
                        }
                    }
                    entries.insert(
                        file.path.clone(),
                        CachedThread {
                            fingerprint,
                            summary: summary.clone(),
                        },
                    );
                }
            }
            Ok(summary)
        })();
        match parsed {
            Ok(Some(summary)) => items.push(summary),
            Ok(None) => {}
            Err(error) => warnings.push(scan_warning(&file.path, &error)),
        }
    }
    if let Ok(mut entries) = cache.lock() {
        entries.retain(|path, _| {
            (!path.starts_with(sessions_dir) && !path.starts_with(archived_dir))
                || seen.contains(path)
        });
    }
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(ThreadScanResult { items, warnings })
}

fn collect_thread_files(
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<Vec<ThreadFile>, CoreError> {
    let mut files = Vec::new();
    collect_rollout_files(sessions_dir, false, &mut files, None)?;
    collect_rollout_files(archived_dir, true, &mut files, None)?;
    Ok(files)
}

fn collect_rollout_files(
    root: &Path,
    archived: bool,
    out: &mut Vec<ThreadFile>,
    mut warnings: Option<&mut Vec<CoreWarning>>,
) -> Result<(), CoreError> {
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return scan_error(
            &mut warnings,
            root,
            CoreError::InvalidData(format!(
                "thread source is not a directory: {}",
                root.display()
            )),
        );
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                scan_error(&mut warnings, &dir, error.into())?;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan_error(&mut warnings, &dir, error.into())?;
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan_error(&mut warnings, &entry.path(), error.into())?;
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && is_rollout_file(&path) {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                let scope = if archived { "archived" } else { "active" };
                out.push(ThreadFile {
                    key: format!("{scope}:{}", normalize_relative_path(relative)),
                    path,
                    archived,
                });
            }
        }
    }
    Ok(())
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_rollout_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

fn parse_timestamp_millis(value: &serde_json::Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn extract_item_text(item: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
        for block in content {
            let kind = block
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(kind.as_str(), "text" | "output_text" | "input_text") {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    parts.join("\n\n")
}

fn set_title_once(title: &mut Option<String>, text: &str) {
    if title.is_some() {
        return;
    }
    let first_line = text.lines().next().unwrap_or("").trim();
    if !first_line.is_empty() {
        *title = Some(first_line.chars().take(80).collect());
    }
}

fn parse_thread(
    file: &ThreadFile,
    include_messages: bool,
) -> Result<Option<ParsedThread>, CoreError> {
    let source_file = File::open(&file.path)?;
    let mut reader = BufReader::new(source_file);
    let mut line = String::new();
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut created_at: Option<i64> = None;
    let mut updated_at: Option<i64> = None;
    let mut title: Option<String> = None;
    let mut user_messages = 0usize;
    let mut agent_messages = 0usize;
    let mut source = String::new();
    let mut cli_version = String::new();
    let mut messages = Vec::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let timestamp = value
            .get("timestamp")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if let Some(timestamp_millis) = value.get("timestamp").and_then(parse_timestamp_millis) {
            updated_at =
                Some(updated_at.map_or(timestamp_millis, |prev| prev.max(timestamp_millis)));
            created_at =
                Some(created_at.map_or(timestamp_millis, |prev| prev.min(timestamp_millis)));
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };

        let record_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let payload_type = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if record_type == "session_meta" || payload_type == "session_meta" {
            session_id = payload
                .get("session_id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            source = payload
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            cli_version = payload
                .get("cli_version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            continue;
        }

        match payload_type {
            "item_completed" => {
                let Some(item) = payload.get("item") else {
                    continue;
                };
                let (role, text) = match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "UserMessage" => {
                        user_messages += 1;
                        let text = extract_item_text(item);
                        set_title_once(&mut title, &text);
                        (Some("user"), text)
                    }
                    "AgentMessage" => {
                        agent_messages += 1;
                        (Some("assistant"), extract_item_text(item))
                    }
                    _ => (None, String::new()),
                };
                if include_messages && !text.is_empty() {
                    if let Some(role) = role {
                        messages.push(ChatMessage {
                            role,
                            timestamp: timestamp.to_string(),
                            text,
                        });
                    }
                }
            }
            "user_message" => {
                user_messages += 1;
                if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                    let text = text.trim();
                    set_title_once(&mut title, text);
                    if include_messages && !text.is_empty() {
                        messages.push(ChatMessage {
                            role: "user",
                            timestamp: timestamp.to_string(),
                            text: text.to_string(),
                        });
                    }
                }
            }
            "agent_message" => {
                agent_messages += 1;
                if include_messages {
                    if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                        let text = text.trim();
                        if !text.is_empty() {
                            messages.push(ChatMessage {
                                role: "assistant",
                                timestamp: timestamp.to_string(),
                                text: text.to_string(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if session_id.is_empty() && cwd.is_empty() && user_messages == 0 && agent_messages == 0 {
        return Ok(None);
    }
    if session_id.is_empty() {
        session_id = file
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
    }
    let file_size = std::fs::metadata(&file.path)?.len();
    let folder_name = Path::new(&cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| cwd.clone());
    let created = created_at.or(updated_at).unwrap_or(0);

    Ok(Some(ParsedThread {
        summary: ThreadSummary {
            key: file.key.clone(),
            id: session_id,
            cwd,
            folder_name,
            title: title.unwrap_or_default(),
            created_at: created / 1000,
            updated_at: updated_at.unwrap_or(created) / 1000,
            user_messages,
            agent_messages,
            file_size,
            archived: file.archived,
            source,
            cli_version,
            file_path: file.path.display().to_string(),
        },
        messages,
    }))
}

fn sanitize_file_name(name: &str) -> String {
    let out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let out = out.trim_matches([' ', '.']);
    let mut out: String = if out.is_empty() {
        "thread".to_string()
    } else {
        out.chars().take(60).collect()
    };
    let device_name = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let numbered_device = device_name
        .strip_prefix("COM")
        .or_else(|| device_name.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        });
    if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered_device {
        out.insert(0, '_');
    }
    out
}

fn export_file_stem(summary: &ThreadSummary) -> String {
    let date = DateTime::from_timestamp(summary.created_at, 0)
        .map(|value| value.format("%Y%m%d").to_string())
        .unwrap_or_else(|| "undated".to_string());
    let base_title = if summary.title.is_empty() {
        &summary.folder_name
    } else {
        &summary.title
    };
    format!("{date}-{}", sanitize_file_name(base_title))
}

fn validate_target_dir(
    sessions_dir: &Path,
    archived_dir: &Path,
    target_dir: &Path,
) -> Result<PathBuf, CoreError> {
    if target_dir.as_os_str().is_empty() {
        return Err(CoreError::InvalidData(
            "export directory is required".into(),
        ));
    }
    if target_dir.exists() && !target_dir.is_dir() {
        return Err(CoreError::InvalidData(format!(
            "export target is not a directory: {}",
            target_dir.display()
        )));
    }
    std::fs::create_dir_all(target_dir)?;
    let target = target_dir.canonicalize()?;
    for source in [sessions_dir, archived_dir] {
        if source.exists() {
            let source = source.canonicalize()?;
            if target.starts_with(&source) {
                return Err(CoreError::InvalidData(
                    "export directory cannot be inside a thread source directory".into(),
                ));
            }
        }
    }
    Ok(target)
}

fn next_export_name(
    summary: &ThreadSummary,
    format: ThreadExportFormat,
    target_dir: &Path,
    used_names: &mut HashSet<String>,
) -> String {
    let ext = match format {
        ThreadExportFormat::Markdown => "md",
        ThreadExportFormat::Jsonl => "jsonl",
    };
    let stem = export_file_stem(summary);
    let mut file_name = format!("{stem}.{ext}");
    let mut counter = 2;
    while used_names.contains(&file_name.to_ascii_lowercase())
        || target_dir.join(&file_name).exists()
    {
        file_name = format!("{stem}-{counter}.{ext}");
        counter += 1;
    }
    used_names.insert(file_name.to_ascii_lowercase());
    file_name
}

#[derive(Debug)]
struct StagedExport {
    temp_path: PathBuf,
    final_path: PathBuf,
    file_name: String,
}

fn create_staged_file(target_dir: &Path) -> Result<(PathBuf, File), CoreError> {
    for _ in 0..10 {
        let path = target_dir.join(format!(".agentharbor-export-{}.tmp", uuid::Uuid::new_v4()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(CoreError::OperationFailed(
        "could not allocate a temporary export file".into(),
    ))
}

fn cleanup_staged(staged: &[StagedExport]) {
    for item in staged {
        let _ = std::fs::remove_file(&item.temp_path);
    }
}

fn stage_export(
    file: &ThreadFile,
    target_dir: &Path,
    format: ThreadExportFormat,
    used_names: &mut HashSet<String>,
) -> Result<StagedExport, CoreError> {
    let include_messages = format == ThreadExportFormat::Markdown;
    let parsed = parse_thread(file, include_messages)?.ok_or_else(|| {
        CoreError::InvalidData(format!(
            "selected file is not a readable conversation log: {}",
            file.path.display()
        ))
    })?;
    let mut jsonl_source = match format {
        ThreadExportFormat::Jsonl => Some(File::open(&file.path)?),
        ThreadExportFormat::Markdown => None,
    };
    let file_name = next_export_name(&parsed.summary, format, target_dir, used_names);
    let final_path = target_dir.join(&file_name);
    let (temp_path, temp_file) = create_staged_file(target_dir)?;
    let write_result = match format {
        ThreadExportFormat::Jsonl => {
            let mut target = BufWriter::new(temp_file);
            match jsonl_source.as_mut() {
                Some(source) => std::io::copy(source, &mut target)
                    .and_then(|_| target.flush())
                    .map(|_| ()),
                None => Err(std::io::Error::other("missing JSONL source")),
            }
        }
        ThreadExportFormat::Markdown => {
            let mut target = BufWriter::new(temp_file);
            target
                .write_all(render_markdown(&parsed.summary, &parsed.messages).as_bytes())
                .and_then(|_| target.flush())
        }
    };
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(StagedExport {
        temp_path,
        final_path,
        file_name,
    })
}

/// Export selected rollout files as Markdown transcripts or raw JSONL.
pub fn export_threads(
    sessions_dir: &Path,
    archived_dir: &Path,
    keys: &[String],
    target_dir: &Path,
    format: ThreadExportFormat,
) -> Result<ThreadExportResult, CoreError> {
    let requested: HashSet<&str> = keys.iter().map(String::as_str).collect();
    if requested.is_empty() {
        return Err(CoreError::InvalidData("no threads selected".into()));
    }

    let target_dir = validate_target_dir(sessions_dir, archived_dir, target_dir)?;
    let mut selected: Vec<ThreadFile> = collect_thread_files(sessions_dir, archived_dir)?
        .into_iter()
        .filter(|file| requested.contains(file.key.as_str()))
        .collect();
    if selected.len() != requested.len() {
        return Err(CoreError::NotFound(
            "one or more selected threads no longer exist; refresh and try again".into(),
        ));
    }
    selected.sort_by(|a, b| a.key.cmp(&b.key));

    let mut staged = Vec::with_capacity(selected.len());
    let mut used_names = HashSet::new();
    for file in selected {
        match stage_export(&file, &target_dir, format, &mut used_names) {
            Ok(item) => staged.push(item),
            Err(error) => {
                cleanup_staged(&staged);
                return Err(error);
            }
        }
    }

    let mut committed = Vec::new();
    for (index, item) in staged.iter().enumerate() {
        if let Err(error) = std::fs::rename(&item.temp_path, &item.final_path) {
            for path in committed {
                let _ = std::fs::remove_file(path);
            }
            cleanup_staged(&staged[index..]);
            return Err(error.into());
        }
        committed.push(item.final_path.clone());
    }

    let files: Vec<String> = staged.into_iter().map(|item| item.file_name).collect();
    Ok(ThreadExportResult {
        exported: files.len(),
        directory: target_dir.display().to_string(),
        files,
    })
}

fn escape_heading(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(['\r', '\n'], " ")
        .replace('#', "\\#")
}

fn html_code(value: &str) -> String {
    format!(
        "<code>{}</code>",
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    )
}

fn render_markdown(summary: &ThreadSummary, messages: &[ChatMessage]) -> String {
    let heading = if summary.title.is_empty() {
        &summary.id
    } else {
        &summary.title
    };
    let mut out = format!(
        "# {}\n\n- 工作目录: {}\n- 线程 ID: {}\n- 来源: {}\n- 消息数: 用户 {} / 助手 {}\n\n---\n\n",
        escape_heading(heading),
        html_code(&summary.cwd),
        html_code(&summary.id),
        html_code(&summary.source),
        summary.user_messages,
        summary.agent_messages
    );
    for message in messages {
        out.push_str(if message.role == "user" {
            "## 👤 用户"
        } else {
            "## 🤖 助手"
        });
        if !message.timestamp.is_empty() {
            out.push_str("  ");
            out.push_str(&html_code(&message.timestamp));
        }
        out.push_str("\n\n");
        out.push_str(&message.text);
        out.push_str("\n\n---\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn rollout(id: &str, cwd: &str, title: &str) -> String {
        format!(
            "{{\"timestamp\":\"2026-09-20T12:00:00+08:00\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"source\":\"cli\"}}}}\n\
             {{\"timestamp\":\"2026-09-20T12:01:00+08:00\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{title}\"}}}}\n\
             {{\"timestamp\":\"2026-09-20T12:02:00+08:00\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"done\"}}}}\n"
        )
    }

    #[test]
    fn scan_skips_unreadable_logs_and_reports_a_warning() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("rollout-good.jsonl"),
            rollout("good", "C:/work", "Good"),
        )
        .unwrap();
        fs::write(sessions.join("rollout-bad.jsonl"), [0xff]).unwrap();
        let scan = scan_threads_with_warnings(&sessions, &archived).unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].id, "good");
        assert_eq!(scan.warnings.len(), 1);
        assert_eq!(scan.warnings[0].code, "thread_scan_skipped");
        assert!(scan.warnings[0].message.contains("rollout-bad.jsonl"));
        assert!(export_threads(
            &sessions,
            &archived,
            &["active:rollout-bad.jsonl".into()],
            &root.path().join("output"),
            ThreadExportFormat::Markdown
        )
        .is_err());
    }

    #[test]
    fn scan_keeps_valid_source_when_other_source_is_not_a_directory() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("rollout-good.jsonl"),
            rollout("good", "C:/work", "Good"),
        )
        .unwrap();
        fs::write(&archived, "not a directory").unwrap();
        let scan = scan_threads_with_warnings(&sessions, &archived).unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn cached_scan_observes_modified_removed_and_repaired_files() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived");
        fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("rollout-a.jsonl");
        fs::write(&path, rollout("id", "C:/work", "Original")).unwrap();
        let first = scan_threads(&sessions, &archived).unwrap();
        assert_eq!(scan_threads(&sessions, &archived).unwrap(), first);
        fs::write(
            &path,
            rollout("id", "C:/work", "Changed title with a different length"),
        )
        .unwrap();
        assert_eq!(
            scan_threads(&sessions, &archived).unwrap()[0].title,
            "Changed title with a different length"
        );
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            scan_threads_with_warnings(&sessions, &archived)
                .unwrap()
                .warnings
                .len(),
            1
        );
        fs::write(&path, rollout("id", "C:/work", "Repaired")).unwrap();
        let repaired = scan_threads_with_warnings(&sessions, &archived).unwrap();
        assert!(repaired.warnings.is_empty());
        assert_eq!(repaired.items[0].title, "Repaired");
        fs::remove_file(path).unwrap();
        assert!(scan_threads(&sessions, &archived).unwrap().is_empty());
    }

    #[test]
    fn scan_streams_threads_and_assigns_unique_keys() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(sessions.join("2026/09/20")).unwrap();
        fs::create_dir_all(&archived).unwrap();
        fs::write(
            sessions.join("2026/09/20/rollout-a.jsonl"),
            rollout("same-id", "C:/work/one", "First"),
        )
        .unwrap();
        fs::write(
            archived.join("rollout-b.jsonl"),
            rollout("same-id", "C:/work/two", "Second"),
        )
        .unwrap();

        let items = scan_threads(&sessions, &archived).unwrap();
        assert_eq!(items.len(), 2);
        assert_ne!(items[0].key, items[1].key);
        assert_eq!(items[0].user_messages, 1);
        assert_eq!(items[0].agent_messages, 1);
    }

    #[test]
    fn export_rejects_source_directory_and_stale_keys() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("rollout-a.jsonl"),
            rollout("id", "C:/work", "Title"),
        )
        .unwrap();
        let key = scan_threads(&sessions, &archived).unwrap()[0].key.clone();

        assert!(export_threads(
            &sessions,
            &archived,
            std::slice::from_ref(&key),
            &sessions.join("exports"),
            ThreadExportFormat::Markdown,
        )
        .is_err());
        assert!(export_threads(
            &sessions,
            &archived,
            &["active:missing.jsonl".into()],
            &root.path().join("exports"),
            ThreadExportFormat::Markdown,
        )
        .is_err());
    }

    #[test]
    fn export_is_collision_safe_and_escapes_metadata() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        let output = root.path().join("output");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("rollout-a.jsonl"),
            rollout("id`x", "C:/work/`demo`", "Title"),
        )
        .unwrap();
        let key = scan_threads(&sessions, &archived).unwrap()[0].key.clone();

        let first = export_threads(
            &sessions,
            &archived,
            std::slice::from_ref(&key),
            &output,
            ThreadExportFormat::Markdown,
        )
        .unwrap();
        let second = export_threads(
            &sessions,
            &archived,
            &[key],
            &output,
            ThreadExportFormat::Markdown,
        )
        .unwrap();
        assert_ne!(first.files[0], second.files[0]);
        let markdown = fs::read_to_string(output.join(&first.files[0])).unwrap();
        assert!(markdown.contains("<code>C:/work/`demo`</code>"));
        assert!(markdown.contains("<code>id`x</code>"));
    }

    #[test]
    fn timestamp_parser_rejects_invalid_dates() {
        assert!(DateTime::parse_from_rfc3339("2026-02-30T12:00:00Z").is_err());
        assert!(DateTime::parse_from_rfc3339("2026-09-20T12:00:00+25:00").is_err());
    }
}
