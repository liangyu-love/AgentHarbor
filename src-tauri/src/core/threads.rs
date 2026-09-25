use crate::core::models::{CoreError, CoreWarning};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
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
    pub model_provider: String,
    pub model: String,
    pub tokens_used: i64,
    pub thread_source: String,
    pub rollout_status: String,
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

#[derive(Debug)]
struct CatalogThread {
    id: String,
    rollout_path: String,
    created_at: i64,
    created_at_ms: i64,
    updated_at: i64,
    updated_at_ms: i64,
    source: String,
    model_provider: String,
    model: String,
    cwd: String,
    title: String,
    first_user_message: String,
    tokens_used: i64,
    archived: bool,
    cli_version: String,
}

#[derive(Debug)]
struct CatalogRead {
    threads: Vec<CatalogThread>,
    child_thread_ids: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageFormat {
    ResponseItem,
    ItemCompleted,
    Direct,
}

#[derive(Debug)]
struct SeenMessage {
    role: &'static str,
    normalized_text: String,
    timestamp: String,
    timestamp_millis: Option<i64>,
    format: MessageFormat,
    paired: bool,
}

#[derive(Clone)]
struct CachedThread {
    fingerprint: (u64, SystemTime),
    summary: Option<ThreadSummary>,
}

const THREAD_CACHE_LIMIT: usize = 4096;
const ROLLOUT_ID_INDEX_LIMIT: usize = 32;
static THREAD_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedThread>>> = OnceLock::new();
static ROLLOUT_ID_INDEX: OnceLock<
    Mutex<HashMap<(PathBuf, PathBuf), HashMap<String, Vec<PathBuf>>>>,
> = OnceLock::new();

pub(super) fn indexed_rollout_paths(
    sessions_dir: &Path,
    archived_dir: &Path,
    thread_id: &str,
) -> Option<Vec<PathBuf>> {
    ROLLOUT_ID_INDEX
        .get()?
        .lock()
        .ok()?
        .get(&(sessions_dir.to_path_buf(), archived_dir.to_path_buf()))
        .and_then(|by_id| by_id.get(thread_id).cloned())
}

fn replace_rollout_id_index(sessions_dir: &Path, archived_dir: &Path, items: &[ThreadSummary]) {
    let mut by_id: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for item in items {
        if !item.id.is_empty() && !item.file_path.is_empty() {
            by_id
                .entry(item.id.clone())
                .or_default()
                .push(PathBuf::from(&item.file_path));
        }
    }
    for paths in by_id.values_mut() {
        paths.sort();
        paths.dedup();
    }
    let index = ROLLOUT_ID_INDEX.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut entries) = index.lock() {
        let key = (sessions_dir.to_path_buf(), archived_dir.to_path_buf());
        if entries.len() >= ROLLOUT_ID_INDEX_LIMIT && !entries.contains_key(&key) {
            if let Some(stale) = entries.keys().next().cloned() {
                entries.remove(&stale);
            }
        }
        entries.insert(key, by_id);
    }
}

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

/// Read the Codex SQLite thread catalog as the authoritative thread list.
/// Rollout files are parsed only to enrich catalog rows with message counts and
/// export locators. If the catalog cannot be read, the legacy file scan remains
/// available as a compatibility fallback.
pub fn scan_threads_from_catalog_with_warnings(
    state_database_path: &Path,
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<ThreadScanResult, CoreError> {
    match read_catalog_threads(state_database_path) {
        Ok(catalog) => merge_catalog_with_rollouts(catalog, sessions_dir, archived_dir),
        Err(error) => {
            let mut fallback = scan_threads_with_warnings(sessions_dir, archived_dir)?;
            fallback.warnings.insert(
                0,
                CoreWarning {
                    code: "thread_catalog_fallback".into(),
                    message: format!(
                        "Codex thread catalog {} could not be read; using rollout files: {error}",
                        state_database_path.display()
                    ),
                },
            );
            Ok(fallback)
        }
    }
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, CoreError> {
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|error| CoreError::OperationFailed(error.to_string()))
}

fn optional_text(columns: &HashSet<String>, name: &str) -> String {
    if columns.contains(name) {
        format!("COALESCE(CAST(\"{name}\" AS TEXT), '')")
    } else {
        "''".into()
    }
}

fn optional_integer(columns: &HashSet<String>, name: &str) -> String {
    if columns.contains(name) {
        format!("COALESCE(CAST(\"{name}\" AS INTEGER), 0)")
    } else {
        "0".into()
    }
}

fn read_catalog_threads(state_database_path: &Path) -> Result<CatalogRead, CoreError> {
    if !state_database_path.is_file() {
        return Err(CoreError::NotFound(format!(
            "thread catalog does not exist: {}",
            state_database_path.display()
        )));
    }
    let connection = Connection::open_with_flags(
        state_database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    let columns = table_columns(&connection, "threads")?;
    if !columns.contains("id") {
        return Err(CoreError::InvalidData(
            "Codex thread catalog has no threads.id column".into(),
        ));
    }
    let spawn_columns = table_columns(&connection, "thread_spawn_edges")?;
    let job_columns = table_columns(&connection, "agent_job_items")?;
    let mut child_queries = Vec::new();
    if spawn_columns.contains("child_thread_id") {
        child_queries.push(
            "SELECT \"child_thread_id\" FROM \"thread_spawn_edges\" WHERE \"child_thread_id\" IS NOT NULL"
                .to_owned(),
        );
    }
    if job_columns.contains("assigned_thread_id") {
        child_queries.push(
            "SELECT \"assigned_thread_id\" FROM \"agent_job_items\" WHERE \"assigned_thread_id\" IS NOT NULL"
                .to_owned(),
        );
    }
    let child_query = child_queries.join(" UNION ");
    let child_filter = if child_query.is_empty() {
        String::new()
    } else {
        format!(" WHERE \"id\" NOT IN ({child_query})")
    };
    let sql = format!(
        "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM \"threads\"{}",
        optional_text(&columns, "id"),
        optional_text(&columns, "rollout_path"),
        optional_integer(&columns, "created_at"),
        optional_integer(&columns, "created_at_ms"),
        optional_integer(&columns, "updated_at"),
        optional_integer(&columns, "updated_at_ms"),
        optional_text(&columns, "source"),
        optional_text(&columns, "model_provider"),
        optional_text(&columns, "model"),
        optional_text(&columns, "cwd"),
        optional_text(&columns, "title"),
        optional_text(&columns, "first_user_message"),
        optional_integer(&columns, "tokens_used"),
        optional_integer(&columns, "archived"),
        optional_text(&columns, "cli_version"),
        child_filter,
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CatalogThread {
                id: row.get(0)?,
                rollout_path: row.get(1)?,
                created_at: row.get(2)?,
                created_at_ms: row.get(3)?,
                updated_at: row.get(4)?,
                updated_at_ms: row.get(5)?,
                source: row.get(6)?,
                model_provider: row.get(7)?,
                model: row.get(8)?,
                cwd: row.get(9)?,
                title: row.get(10)?,
                first_user_message: row.get(11)?,
                tokens_used: row.get(12)?,
                archived: row.get::<_, i64>(13)? != 0,
                cli_version: row.get(14)?,
            })
        })
        .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    let threads = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
    let child_thread_ids = if child_query.is_empty() {
        HashSet::new()
    } else {
        let mut statement = connection
            .prepare(&child_query)
            .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| CoreError::OperationFailed(error.to_string()))?;
        rows.collect::<Result<HashSet<_>, _>>()
            .map_err(|error| CoreError::OperationFailed(error.to_string()))?
    };
    Ok(CatalogRead {
        threads,
        child_thread_ids,
    })
}

fn path_identity(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn catalog_timestamp(seconds: i64, millis: i64) -> i64 {
    if millis > 0 {
        millis / 1000
    } else {
        seconds
    }
}

fn folder_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| cwd.to_owned())
}

fn merge_catalog_with_rollouts(
    catalog: CatalogRead,
    sessions_dir: &Path,
    archived_dir: &Path,
) -> Result<ThreadScanResult, CoreError> {
    let rollout_scan = scan_threads_with_warnings(sessions_dir, archived_dir)?;
    let mut warnings = rollout_scan.warnings;
    let rollout_items = rollout_scan.items;
    let mut by_path = HashMap::new();
    let mut by_id: HashMap<String, Vec<ThreadSummary>> = HashMap::new();
    for summary in &rollout_items {
        by_path.insert(
            path_identity(Path::new(&summary.file_path)),
            summary.clone(),
        );
        by_id
            .entry(summary.id.clone())
            .or_default()
            .push(summary.clone());
    }

    let mut catalog_ids = HashSet::new();
    let mut items = Vec::with_capacity(catalog.threads.len() + rollout_items.len());
    for row in catalog.threads {
        if row.id.is_empty() {
            warnings.push(CoreWarning {
                code: "thread_catalog_skipped".into(),
                message: "Skipped a Codex thread catalog row with an empty ID".into(),
            });
            continue;
        }
        catalog_ids.insert(row.id.clone());
        let catalog_path = PathBuf::from(&row.rollout_path);
        let exact_candidate = (!row.rollout_path.is_empty())
            .then(|| by_path.get(&path_identity(&catalog_path)).cloned())
            .flatten();
        let exact = exact_candidate.and_then(|summary| {
            if summary.id == row.id {
                Some(summary)
            } else {
                warnings.push(CoreWarning {
                    code: "thread_rollout_mismatch".into(),
                    message: format!(
                        "Catalog thread {} points to a rollout for {}; matched the rollout by thread ID instead",
                        row.id, summary.id
                    ),
                });
                None
            }
        });
        let discovered = exact.or_else(|| {
            by_id
                .get(&row.id)
                .and_then(|candidates| candidates.iter().max_by_key(|item| item.updated_at))
                .cloned()
        });
        let rollout_available = discovered.is_some();
        let mut summary = discovered.unwrap_or_else(|| ThreadSummary {
            key: format!("catalog:{}", row.id),
            id: row.id.clone(),
            cwd: String::new(),
            folder_name: String::new(),
            title: String::new(),
            created_at: 0,
            updated_at: 0,
            user_messages: 0,
            agent_messages: 0,
            file_size: 0,
            archived: row.archived,
            source: String::new(),
            cli_version: String::new(),
            file_path: row.rollout_path.clone(),
            model_provider: String::new(),
            model: String::new(),
            tokens_used: 0,
            thread_source: "sqlite".into(),
            rollout_status: "missing".into(),
        });
        summary.id = row.id;
        if !row.cwd.is_empty() {
            summary.cwd = row.cwd;
            summary.folder_name = folder_name(&summary.cwd);
        }
        let title = if row.title.is_empty() {
            row.first_user_message
        } else {
            row.title
        };
        if !title.is_empty() {
            summary.title = title;
        }
        let created_at = catalog_timestamp(row.created_at, row.created_at_ms);
        let updated_at = catalog_timestamp(row.updated_at, row.updated_at_ms);
        if created_at > 0 {
            summary.created_at = created_at;
        }
        if updated_at > 0 {
            summary.updated_at = updated_at;
        }
        summary.archived = row.archived;
        if !row.source.is_empty() {
            summary.source = row.source;
        }
        if !row.cli_version.is_empty() {
            summary.cli_version = row.cli_version;
        }
        summary.model_provider = row.model_provider;
        summary.model = row.model;
        summary.tokens_used = row.tokens_used;
        summary.thread_source = "sqlite".into();
        summary.rollout_status = if rollout_available {
            "available".into()
        } else {
            "missing".into()
        };
        items.push(summary);
    }
    for summary in rollout_items {
        if !catalog_ids.contains(&summary.id) && !catalog.child_thread_ids.contains(&summary.id) {
            items.push(summary);
        }
    }
    items.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(ThreadScanResult { items, warnings })
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
    replace_rollout_id_index(sessions_dir, archived_dir, &items);
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
    timestamp_millis(value.as_str()?)
}

fn timestamp_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
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

fn record_message(
    role: &'static str,
    timestamp: &str,
    text: String,
    format: MessageFormat,
    include_messages: bool,
    seen: &mut Vec<SeenMessage>,
    title: &mut Option<String>,
    user_messages: &mut usize,
    agent_messages: &mut usize,
    messages: &mut Vec<ChatMessage>,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let normalized_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let timestamp_millis = timestamp_millis(timestamp);
    if seen.iter().any(|previous| {
        previous.role == role
            && previous.normalized_text == normalized_text
            && previous.format == format
            && previous.timestamp == timestamp
    }) {
        return;
    }

    let mut nearest_pair = None;
    let mut nearest_delta = u64::MAX;
    if let Some(current) = timestamp_millis {
        for (index, previous) in seen.iter().enumerate() {
            if previous.paired
                || previous.role != role
                || previous.normalized_text != normalized_text
                || previous.format == format
            {
                continue;
            }
            let Some(previous_timestamp) = previous.timestamp_millis else {
                continue;
            };
            let delta = previous_timestamp.abs_diff(current);
            if delta <= 10_000 && delta <= nearest_delta {
                nearest_pair = Some(index);
                nearest_delta = delta;
            }
        }
    }
    if let Some(index) = nearest_pair {
        seen[index].paired = true;
        seen.push(SeenMessage {
            role,
            normalized_text,
            timestamp: timestamp.to_owned(),
            timestamp_millis,
            format,
            paired: true,
        });
        return;
    }
    seen.push(SeenMessage {
        role,
        normalized_text,
        timestamp: timestamp.to_owned(),
        timestamp_millis,
        format,
        paired: false,
    });
    if role == "user" {
        *user_messages += 1;
        set_title_once(title, text);
    } else {
        *agent_messages += 1;
    }
    if include_messages {
        messages.push(ChatMessage {
            role,
            timestamp: timestamp.to_owned(),
            text: text.to_owned(),
        });
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
    let mut model_provider = String::new();
    let mut model = String::new();
    let mut messages = Vec::new();
    let mut seen_messages = Vec::new();

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
            model_provider = payload
                .get("model_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            model = payload
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            continue;
        }

        if record_type == "response_item" && payload_type == "message" {
            let role = match payload.get("role").and_then(|value| value.as_str()) {
                Some("user") => Some("user"),
                Some("assistant") => Some("assistant"),
                _ => None,
            };
            if let Some(role) = role {
                record_message(
                    role,
                    timestamp,
                    extract_item_text(payload),
                    MessageFormat::ResponseItem,
                    include_messages,
                    &mut seen_messages,
                    &mut title,
                    &mut user_messages,
                    &mut agent_messages,
                    &mut messages,
                );
            }
            continue;
        }

        match payload_type {
            "item_completed" => {
                let Some(item) = payload.get("item") else {
                    continue;
                };
                let role = match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "UserMessage" => Some("user"),
                    "AgentMessage" => Some("assistant"),
                    _ => None,
                };
                if let Some(role) = role {
                    record_message(
                        role,
                        timestamp,
                        extract_item_text(item),
                        MessageFormat::ItemCompleted,
                        include_messages,
                        &mut seen_messages,
                        &mut title,
                        &mut user_messages,
                        &mut agent_messages,
                        &mut messages,
                    );
                }
            }
            "user_message" => {
                if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                    record_message(
                        "user",
                        timestamp,
                        text.to_owned(),
                        MessageFormat::Direct,
                        include_messages,
                        &mut seen_messages,
                        &mut title,
                        &mut user_messages,
                        &mut agent_messages,
                        &mut messages,
                    );
                }
            }
            "agent_message" => {
                if let Some(text) = payload.get("message").and_then(|m| m.as_str()) {
                    record_message(
                        "assistant",
                        timestamp,
                        text.to_owned(),
                        MessageFormat::Direct,
                        include_messages,
                        &mut seen_messages,
                        &mut title,
                        &mut user_messages,
                        &mut agent_messages,
                        &mut messages,
                    );
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
            model_provider,
            model,
            tokens_used: 0,
            thread_source: "rollout".into(),
            rollout_status: "available".into(),
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
    fn rollout_id_index_tracks_duplicates_and_removals() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let archived = temp.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&archived).unwrap();
        let active = sessions.join("rollout-active.jsonl");
        let archived_path = archived.join("rollout-archived.jsonl");
        fs::write(&active, rollout("same-id", "C:/one", "Active")).unwrap();

        scan_threads_with_warnings(&sessions, &archived).unwrap();
        assert_eq!(
            indexed_rollout_paths(&sessions, &archived, "same-id"),
            Some(vec![active.clone()])
        );

        fs::write(&archived_path, rollout("same-id", "C:/two", "Archived")).unwrap();
        scan_threads_with_warnings(&sessions, &archived).unwrap();
        let mut indexed = indexed_rollout_paths(&sessions, &archived, "same-id").unwrap();
        indexed.sort();
        let mut expected = vec![active.clone(), archived_path.clone()];
        expected.sort();
        assert_eq!(indexed, expected);

        fs::remove_file(active).unwrap();
        fs::remove_file(archived_path).unwrap();
        scan_threads_with_warnings(&sessions, &archived).unwrap();
        assert!(indexed_rollout_paths(&sessions, &archived, "same-id").is_none());
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
    fn catalog_is_authoritative_schema_adaptive_and_filters_child_threads() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        let parent_path = sessions.join("rollout-parent.jsonl");
        let child_path = sessions.join("rollout-child.jsonl");
        let assigned_path = sessions.join("rollout-assigned.jsonl");
        fs::write(&parent_path, rollout("parent", "C:/file", "File title")).unwrap();
        fs::write(&child_path, rollout("child", "C:/child", "Child")).unwrap();
        fs::write(
            &assigned_path,
            rollout("assigned", "C:/assigned", "Assigned"),
        )
        .unwrap();
        fs::write(
            sessions.join("rollout-orphan.jsonl"),
            rollout("orphan", "C:/orphan", "Orphan"),
        )
        .unwrap();

        let database = root.path().join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    updated_at INTEGER,
                    cwd TEXT,
                    title TEXT,
                    model_provider TEXT,
                    model TEXT,
                    tokens_used INTEGER,
                    archived INTEGER
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT,
                    child_thread_id TEXT PRIMARY KEY,
                    status TEXT
                );
                CREATE TABLE agent_job_items (
                    id TEXT PRIMARY KEY,
                    assigned_thread_id TEXT
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 100, 'C:/catalog', 'Catalog title', 'custom', 'gpt-test', 42, 0)",
                rusqlite::params!["parent", parent_path.display().to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 200, 'C:/child', 'Child', 'custom', 'gpt-test', 7, 0)",
                rusqlite::params!["child", child_path.display().to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES ('missing', ?1, 300, 'C:/missing', 'Missing', 'openai', 'gpt-official', 9, 1)",
                [sessions.join("rollout-missing.jsonl").display().to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 250, 'C:/assigned', 'Assigned', 'custom', 'gpt-test', 5, 0)",
                rusqlite::params!["assigned", assigned_path.display().to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_spawn_edges VALUES ('parent', 'child', 'completed')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_job_items VALUES ('job-1', 'assigned')",
                [],
            )
            .unwrap();
        drop(connection);

        let scan =
            scan_threads_from_catalog_with_warnings(&database, &sessions, &archived).unwrap();
        assert_eq!(scan.items.len(), 3);
        assert!(!scan.items.iter().any(|item| item.id == "child"));
        assert!(!scan.items.iter().any(|item| item.id == "assigned"));
        let orphan = scan.items.iter().find(|item| item.id == "orphan").unwrap();
        assert_eq!(orphan.thread_source, "rollout");
        let parent = scan.items.iter().find(|item| item.id == "parent").unwrap();
        assert_eq!(parent.title, "Catalog title");
        assert_eq!(parent.cwd, "C:/catalog");
        assert_eq!(parent.model_provider, "custom");
        assert_eq!(parent.model, "gpt-test");
        assert_eq!(parent.tokens_used, 42);
        assert_eq!(parent.thread_source, "sqlite");
        assert_eq!(parent.rollout_status, "available");
        assert_eq!(parent.user_messages, 1);
        let missing = scan.items.iter().find(|item| item.id == "missing").unwrap();
        assert_eq!(missing.rollout_status, "missing");
        assert!(missing.archived);
        assert_eq!(missing.key, "catalog:missing");
    }

    #[test]
    fn missing_catalog_falls_back_to_rollout_scan() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("rollout-only.jsonl"),
            rollout("only", "C:/work", "Only"),
        )
        .unwrap();

        let scan = scan_threads_from_catalog_with_warnings(
            &root.path().join("missing.sqlite"),
            &sessions,
            &archived,
        )
        .unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].thread_source, "rollout");
        assert_eq!(scan.items[0].rollout_status, "available");
        assert_eq!(scan.warnings[0].code, "thread_catalog_fallback");
    }

    #[test]
    fn catalog_path_with_wrong_thread_id_falls_back_to_id_match_and_warns() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        let wrong_path = sessions.join("rollout-wrong.jsonl");
        let expected_path = sessions.join("rollout-expected.jsonl");
        fs::write(&wrong_path, rollout("wrong", "C:/wrong", "Wrong")).unwrap();
        fs::write(
            &expected_path,
            rollout("expected", "C:/expected", "Expected"),
        )
        .unwrap();

        let database = root.path().join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    title TEXT
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES ('expected', ?1, 'Catalog expected')",
                [wrong_path.display().to_string()],
            )
            .unwrap();
        drop(connection);

        let scan =
            scan_threads_from_catalog_with_warnings(&database, &sessions, &archived).unwrap();
        let expected = scan
            .items
            .iter()
            .find(|item| item.id == "expected")
            .unwrap();
        assert_eq!(
            path_identity(Path::new(&expected.file_path)),
            path_identity(&expected_path)
        );
        assert!(scan
            .warnings
            .iter()
            .any(|warning| warning.code == "thread_rollout_mismatch"));
    }

    #[test]
    fn parses_response_item_completed_and_direct_message_formats_without_exact_duplicates() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        let response_user = r#"{"timestamp":"2026-09-20T12:01:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Hello   world"}]}}"#;
        let contents = format!(
            "{{\"timestamp\":\"2026-09-20T12:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"mixed\",\"cwd\":\"C:/mixed\",\"model_provider\":\"custom\",\"model\":\"gpt-mixed\"}}}}\n{response_user}\n{response_user}\n{{\"timestamp\":\"2026-09-20T12:01:05Z\",\"payload\":{{\"type\":\"item_completed\",\"item\":{{\"type\":\"UserMessage\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Hello world\"}}]}}}}}}\n{{\"timestamp\":\"2026-09-20T12:01:08Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Hello world\"}}]}}}}\n{{\"timestamp\":\"2026-09-20T12:02:00Z\",\"payload\":{{\"type\":\"item_completed\",\"item\":{{\"type\":\"AgentMessage\",\"content\":[{{\"type\":\"output_text\",\"text\":\"Completed\"}}]}}}}}}\n{{\"timestamp\":\"2026-09-20T12:03:00Z\",\"payload\":{{\"type\":\"user_message\",\"message\":\"Follow-up\"}}}}\n{{\"timestamp\":\"2026-09-20T12:04:00Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Response\"}}]}}}}\n"
        );
        fs::write(sessions.join("rollout-mixed.jsonl"), contents).unwrap();

        let items = scan_threads(&sessions, &archived).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Hello   world");
        assert_eq!(items[0].user_messages, 3);
        assert_eq!(items[0].agent_messages, 2);
        assert_eq!(items[0].model_provider, "custom");
        assert_eq!(items[0].model, "gpt-mixed");
    }

    #[test]
    fn pairs_each_cross_format_duplicate_once_across_repeated_rounds() {
        let root = tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let archived = root.path().join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        let contents = "{\"timestamp\":\"2026-09-20T12:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"paired\",\"cwd\":\"C:/paired\"}}\n\
{\"timestamp\":\"2026-09-20T12:01:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Repeat me\"}]}}\n\
{\"timestamp\":\"2026-09-20T12:01:03Z\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"UserMessage\",\"content\":[{\"type\":\"input_text\",\"text\":\"Repeat me\"}]}}}\n\
{\"timestamp\":\"2026-09-20T12:02:00Z\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"UserMessage\",\"content\":[{\"type\":\"input_text\",\"text\":\"Repeat me\"}]}}}\n\
{\"timestamp\":\"2026-09-20T12:02:04Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Repeat me\"}]}}\n";
        fs::write(sessions.join("rollout-paired.jsonl"), contents).unwrap();

        let items = scan_threads(&sessions, &archived).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].user_messages, 2);
        assert_eq!(items[0].title, "Repeat me");
    }

    #[test]
    fn timestamp_parser_rejects_invalid_dates() {
        assert!(DateTime::parse_from_rfc3339("2026-02-30T12:00:00Z").is_err());
        assert!(DateTime::parse_from_rfc3339("2026-09-20T12:00:00+25:00").is_err());
    }
}
