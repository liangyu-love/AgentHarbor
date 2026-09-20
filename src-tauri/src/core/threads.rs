use crate::core::models::CoreError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub cwd: String,
    pub folder_name: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub user_messages: i32,
    pub agent_messages: i32,
    pub file_size: u64,
    pub archived: bool,
    pub source: String,
    pub cli_version: String,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadExportResult {
    pub exported: i32,
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

struct ChatMessage {
    role: &'static str,
    timestamp: String,
    text: String,
}


/// 扫描 sessions 与 archived_sessions 目录，汇总每个 Codex 线程（rollout jsonl）。
pub fn scan_threads(sessions_dir: &Path, archived_dir: &Path) -> Vec<ThreadSummary> {
    let mut files = Vec::new();
    collect_rollout_files(sessions_dir, false, &mut files);
    collect_rollout_files(archived_dir, true, &mut files);
    let mut items: Vec<ThreadSummary> = files
        .iter()
        .filter_map(|(path, archived)| summarize_thread(path, *archived))
        .collect();
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    items
}

fn collect_rollout_files(root: &Path, archived: bool, out: &mut Vec<(PathBuf, bool)>) {
    if !root.is_dir() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map_or(true, |kind| kind.is_symlink()) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if is_rollout_file(&path) {
                out.push((path, archived));
            }
        }
    }
}

fn is_rollout_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

fn parse_timestamp_millis(value: &serde_json::Value) -> Option<i64> {
    parse_iso8601_millis(value.as_str()?)
}

/// 解析 ISO8601（如 2026-06-14T07:29:58.259Z，允许 +08:00 偏移），返回毫秒时间戳。
fn parse_iso8601_millis(raw: &str) -> Option<i64> {
    let bytes = raw.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = raw.get(0..4)?.parse().ok()?;
    let month: i64 = raw.get(5..7)?.parse().ok()?;
    let day: i64 = raw.get(8..10)?.parse().ok()?;
    let hour: i64 = raw.get(11..13)?.parse().ok()?;
    let minute: i64 = raw.get(14..16)?.parse().ok()?;
    let second: i64 = raw.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let rest = &raw[19..];
    let mut millis: i64 = 0;
    let mut offset_seconds: i64 = 0;
    if let Some(dot) = rest.strip_prefix('.') {
        let digits: String = dot.chars().take_while(|c| c.is_ascii_digit()).collect();
        let padded = format!("{:0<3}", digits);
        millis = padded.get(0..3)?.parse().ok()?;
    }
    if rest.ends_with('Z') {
        offset_seconds = 0;
    } else if let Some(pos) = rest.rfind(['+', '-']) {
        let sign = if rest.as_bytes()[pos] == b'-' { -1 } else { 1 };
        let oh: i64 = rest.get(pos + 1..pos + 3)?.parse().ok()?;
        let om: i64 = rest.get(pos + 4..pos + 6)?.parse().ok()?;
        offset_seconds = sign * (oh * 3600 + om * 60);
    }
    Some(
        days_from_civil(year, month, day) * 86_400_000
            + (hour * 3600 + minute * 60 + second - offset_seconds) * 1000
            + millis,
    )
}

/// Howards Hinnant 的 days_from_civil 算法（公历日期 → 距 1970-01-01 天数）
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// days_from_civil 的逆运算
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
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

fn summarize_thread(path: &Path, archived: bool) -> Option<ThreadSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut created_at: Option<i64> = None;
    let mut updated_at: Option<i64> = None;
    let mut title: Option<String> = None;
    let mut user_messages = 0i32;
    let mut agent_messages = 0i32;
    let mut source = String::new();
    let mut cli_version = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_millis) {
            updated_at = Some(updated_at.map_or(ts, |prev| prev.max(ts)));
            if created_at.is_none() {
                created_at = Some(ts);
            }
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session_meta" => {
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
            }
            "item_completed" => {
                let Some(item) = payload.get("item") else {
                    continue;
                };
                match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "UserMessage" => {
                        user_messages += 1;
                        if title.is_none() {
                            let text = extract_item_text(item);
                            let first_line =
                                text.lines().next().unwrap_or("").trim().to_string();
                            if !first_line.is_empty() {
                                title = Some(first_line.chars().take(80).collect());
                            }
                        }
                    }
                    "AgentMessage" => agent_messages += 1,
                    _ => {}
                }
            }
            "user_message" => {
                user_messages += 1;
                if title.is_none() {
                    if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                        let first_line = text.lines().next().unwrap_or("").trim().to_string();
                        if !first_line.is_empty() {
                            title = Some(first_line.chars().take(80).collect());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if session_id.is_empty() && cwd.is_empty() && user_messages == 0 && agent_messages == 0 {
        return None;
    }
    if session_id.is_empty() {
        session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
    }
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let folder_name = Path::new(&cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| cwd.clone());
    let created = created_at.or(updated_at).unwrap_or(0);
    Some(ThreadSummary {
        id: session_id,
        cwd,
        folder_name,
        title: title.unwrap_or_default(),
        created_at: created / 1000,
        updated_at: updated_at.unwrap_or(created) / 1000,
        user_messages,
        agent_messages,
        file_size,
        archived,
        source,
        cli_version,
        file_path: path.display().to_string(),
    })
}

fn read_chat_messages(path: &Path) -> Vec<ChatMessage> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let timestamp = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let Some(payload) = value.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "item_completed" => {
                let Some(item) = payload.get("item") else {
                    continue;
                };
                let role = match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "UserMessage" => "user",
                    "AgentMessage" => "assistant",
                    _ => continue,
                };
                let text = extract_item_text(item);
                if !text.is_empty() {
                    messages.push(ChatMessage { role, timestamp, text });
                }
            }
            "user_message" => {
                if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                    let text = text.trim();
                    if !text.is_empty() {
                        messages.push(ChatMessage {
                            role: "user",
                            timestamp,
                            text: text.to_string(),
                        });
                    }
                }
            }
            "agent_message" => {
                if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                    let text = text.trim();
                    if !text.is_empty() {
                        messages.push(ChatMessage {
                            role: "assistant",
                            timestamp,
                            text: text.to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    messages
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
    if out.is_empty() {
        "thread".to_string()
    } else {
        out.chars().take(60).collect()
    }
}

fn export_file_stem(summary: &ThreadSummary) -> String {
    let (y, m, d) = civil_from_days(summary.created_at / 86_400);
    let base_title = if summary.title.is_empty() {
        summary.folder_name.clone()
    } else {
        summary.title.clone()
    };
    format!("{:04}{:02}{:02}-{}", y, m, d, sanitize_file_name(&base_title))
}

/// 将选中线程导出到指定目录：Markdown 会话记录或原始 jsonl 拷贝。
pub fn export_threads(
    sessions_dir: &Path,
    archived_dir: &Path,
    ids: &[String],
    target_dir: &Path,
    format: ThreadExportFormat,
) -> Result<ThreadExportResult, CoreError> {
    if ids.is_empty() {
        return Err(CoreError::InvalidData("no threads selected".into()));
    }
    let summaries = scan_threads(sessions_dir, archived_dir);
    std::fs::create_dir_all(target_dir)?;
    let mut files = Vec::new();
    let mut used_names: Vec<String> = Vec::new();
    for summary in summaries.iter().filter(|s| ids.contains(&s.id)) {
        let source_path = PathBuf::from(&summary.file_path);
        if !source_path.is_file() {
            continue;
        }
        let ext = match format {
            ThreadExportFormat::Markdown => "md",
            ThreadExportFormat::Jsonl => "jsonl",
        };
        let stem = export_file_stem(summary);
        let mut file_name = format!("{stem}.{ext}");
        let mut counter = 2;
        while used_names.iter().any(|n| n.eq_ignore_ascii_case(&file_name))
            || target_dir.join(&file_name).exists()
        {
            file_name = format!("{stem}-{counter}.{ext}");
            counter += 1;
        }
        used_names.push(file_name.clone());
        let target = target_dir.join(&file_name);
        match format {
            ThreadExportFormat::Jsonl => {
                std::fs::copy(&source_path, &target)?;
            }
            ThreadExportFormat::Markdown => {
                let messages = read_chat_messages(&source_path);
                std::fs::write(&target, render_markdown(summary, &messages))?;
            }
        }
        files.push(file_name);
    }
    Ok(ThreadExportResult {
        exported: files.len() as i32,
        directory: target_dir.display().to_string(),
        files,
    })
}

fn render_markdown(summary: &ThreadSummary, messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let heading = if summary.title.is_empty() {
        &summary.id
    } else {
        &summary.title
    };
    out.push_str(&format!(
        "# {heading}\n\n- 工作目录: `{}`\n- 线程 ID: `{}`\n- 来源: {}\n- 消息数: 用户 {} / 助手 {}\n\n---\n\n",
        summary.cwd, summary.id, summary.source, summary.user_messages, summary.agent_messages
    ));
    for message in messages {
        out.push_str(if message.role == "user" {
            "## 👤 用户"
        } else {
            "## 🤖 Codex"
        });
        if !message.timestamp.is_empty() {
            out.push_str(&format!("  `{}`", message.timestamp));
        }
        out.push_str("\n\n");
        out.push_str(&message.text);
        out.push_str("\n\n---\n\n");
    }
    out
}
