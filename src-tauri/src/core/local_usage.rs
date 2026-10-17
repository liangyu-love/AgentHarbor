//! Usage records read from local Codex and Claude Code transcripts. They live
//! in the usage ledger (see `usage_ledger`) and outlive the transcripts: a
//! deleted or archived session keeps its usage.
use chrono::DateTime;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// How transcripts are read into records; transcripts still on disk are parsed
/// again under a new reading (2: Claude turns without a message id go by request
/// id; 3: records carry the directory the session ran in).
const PARSE_VERSION: i64 = 3;
/// The fingerprint left on a transcript that is gone. Its records stay in the
/// ledger, so deleting or archiving a session never takes its usage away.
const GONE: (i64, u64) = (-2, 0);
const LINEAGE_GAP_MS: i64 = 500;

/// One model call read from a transcript. Input excludes cache reads and
/// writes; output excludes reasoning.
// Records of gone transcripts cannot be parsed again, so a field added later
// must not make the stored ones unreadable.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct UsageRecord {
    pub request_id: String,
    pub timestamp: i64,
    pub source_id: String,
    pub model: String,
    pub session_id: String,
    /// Empty in records read before version 3.
    pub project: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

/// A transcript source as this sync found it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSource {
    pub source_id: &'static str,
    pub available: bool,
    pub file_count: u64,
    pub updated_file_count: u64,
}

pub(crate) struct Transcripts {
    pub records: Vec<UsageRecord>,
    pub warnings: Vec<String>,
}

struct SourceFile {
    source_id: &'static str,
    path: PathBuf,
}

#[derive(Clone, Copy, Default)]
struct RawCounts {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
    total: u64,
}

impl RawCounts {
    fn diff(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            cached: self.cached.saturating_sub(previous.cached),
            cache_write: self.cache_write.saturating_sub(previous.cache_write),
            output: self.output.saturating_sub(previous.output),
            reasoning: self.reasoning.saturating_sub(previous.reasoning),
            total: self.total.saturating_sub(previous.total),
        }
    }
}

/// Brings the ledger's transcript records up to date with the files on disk.
pub(crate) fn sync(
    db: &mut Connection,
    codex_home: &Path,
    claude_home: &Path,
) -> Result<Vec<TranscriptSource>, String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS source_files(
           path TEXT PRIMARY KEY, source TEXT NOT NULL, mtime_ms INTEGER NOT NULL,
           size INTEGER NOT NULL, records_json TEXT NOT NULL, warning TEXT
         );",
    )
    .map_err(|error| error.to_string())?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != PARSE_VERSION {
        db.execute_batch(&format!(
            "DELETE FROM source_files WHERE mtime_ms<>{}; PRAGMA user_version={PARSE_VERSION};",
            GONE.0
        ))
        .map_err(|error| error.to_string())?;
    }

    let fingerprints = {
        let mut query = db
            .prepare("SELECT path,mtime_ms,size FROM source_files")
            .map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut fingerprints = HashMap::new();
        for row in rows {
            let (path, mtime, size) = row.map_err(|error| error.to_string())?;
            fingerprints.insert(path, (mtime, size));
        }
        fingerprints
    };

    let codex_roots = [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ];
    let claude_root = claude_home.join("projects");
    let mut sources = vec![
        TranscriptSource {
            source_id: "codex",
            available: codex_roots.iter().any(|path| path.exists()),
            file_count: 0,
            updated_file_count: 0,
        },
        TranscriptSource {
            source_id: "claude",
            available: claude_root.exists(),
            file_count: 0,
            updated_file_count: 0,
        },
    ];
    let mut files = Vec::new();
    for root in &codex_roots {
        discover(root, "codex", true, &mut files);
    }
    discover(&claude_root, "claude", false, &mut files);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let seen_paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().to_string())
        .collect::<HashSet<_>>();

    let transaction = db.transaction().map_err(|error| error.to_string())?;
    // A deleted or archived transcript keeps its records; request ids keep an
    // archived copy from counting twice.
    for (stale_path, _) in fingerprints
        .iter()
        .filter(|(path, fingerprint)| **fingerprint != GONE && !seen_paths.contains(*path))
    {
        transaction
            .execute(
                "UPDATE source_files SET mtime_ms=?2,size=?3,warning=NULL WHERE path=?1",
                params![stale_path, GONE.0, GONE.1],
            )
            .map_err(|error| error.to_string())?;
    }
    for source_file in files {
        let Some(metadata) = std::fs::metadata(&source_file.path).ok() else {
            continue;
        };
        let path = source_file.path.to_string_lossy().to_string();
        let mtime = modified_ns(&metadata);
        let size = metadata.len();
        let unchanged = fingerprints.get(&path).copied() == Some((mtime, size));
        if let Some(source) = sources
            .iter_mut()
            .find(|source| source.source_id == source_file.source_id)
        {
            source.file_count += 1;
            source.updated_file_count += u64::from(!unchanged);
        }
        if unchanged {
            continue;
        }
        let parsed = if source_file.source_id == "codex" {
            parse_codex(&source_file.path, mtime / 1_000_000_000)
        } else {
            parse_claude(&source_file.path, mtime / 1_000_000_000)
        };
        let (records, warning) = match parsed {
            Ok(result) => result,
            Err(error) => {
                if fingerprints.contains_key(&path) {
                    transaction
                        .execute(
                            "UPDATE source_files SET warning=?2 WHERE path=?1",
                            params![path, error],
                        )
                        .map_err(|update_error| update_error.to_string())?;
                    continue;
                }
                transaction
                    .execute(
                        "INSERT INTO source_files(path,source,mtime_ms,size,records_json,warning)
                         VALUES(?1,?2,-1,0,'[]',?3)",
                        params![path, source_file.source_id, error],
                    )
                    .map_err(|insert_error| insert_error.to_string())?;
                continue;
            }
        };
        let json = serde_json::to_string(&records).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO source_files(path,source,mtime_ms,size,records_json,warning)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(path) DO UPDATE SET source=excluded.source,mtime_ms=excluded.mtime_ms,
               size=excluded.size,records_json=excluded.records_json,warning=excluded.warning",
                params![path, source_file.source_id, mtime, size, json, warning],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(sources)
}

/// Every record once: a resumed session repeats earlier turns, and the fullest
/// copy wins. `None` when unreadable rows were dropped instead; the transcripts
/// still on disk come back with the next sync.
pub(crate) fn read(db: &Connection) -> Result<Option<Transcripts>, String> {
    let mut records = HashMap::<String, UsageRecord>::new();
    let mut warnings = Vec::new();
    let mut unreadable = Vec::new();
    {
        let mut query = db
            .prepare("SELECT path,records_json,warning FROM source_files")
            .map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (path, json, warning) = row.map_err(|error| error.to_string())?;
            let Ok(cached) = serde_json::from_str::<Vec<UsageRecord>>(&json) else {
                unreadable.push(path);
                continue;
            };
            for record in cached {
                merge_record(&mut records, record);
            }
            if let Some(warning) = warning.filter(|value| !value.is_empty()) {
                let name = Path::new(&path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&path);
                warnings.push(format!("{name}: {warning}"));
            }
        }
    }
    if !unreadable.is_empty() {
        // Only the unreadable rows go, not the whole ledger.
        for path in &unreadable {
            db.execute("DELETE FROM source_files WHERE path=?1", params![path])
                .map_err(|error| error.to_string())?;
        }
        return Ok(None);
    }
    warnings.sort();
    Ok(Some(Transcripts {
        records: records.into_values().collect(),
        warnings,
    }))
}

fn merge_record(records: &mut HashMap<String, UsageRecord>, record: UsageRecord) {
    let replace = records
        .get(&record.request_id)
        .map(|old| record.total_tokens > old.total_tokens)
        .unwrap_or(true);
    if replace {
        records.insert(record.request_id.clone(), record);
    }
}

fn discover(root: &Path, source_id: &'static str, codex_only: bool, output: &mut Vec<SourceFile>) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let jsonl = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("jsonl"))
                .unwrap_or(false);
            let rollout = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.starts_with("rollout-"))
                .unwrap_or(false);
            if jsonl && (!codex_only || rollout) {
                output.push(SourceFile { source_id, path });
            }
        }
    }
}

fn parse_codex(path: &Path, fallback: i64) -> Result<(Vec<UsageRecord>, Option<String>), String> {
    let mut start = 0;
    let mut tracking_lineage = false;
    let mut previous = None;
    visit_values(path, |index, value| {
        if is_session_meta(&value) {
            start = index + 1;
            previous = timestamp_ms(&value);
            tracking_lineage = true;
        } else if tracking_lineage {
            let current = timestamp_ms(&value);
            match (previous, current) {
                (Some(left), Some(right)) if right.saturating_sub(left) <= LINEAGE_GAP_MS => {
                    previous = Some(right);
                    start = index + 1;
                }
                _ => tracking_lineage = false,
            }
        }
    })?;
    let mut context = CodexContext {
        session: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_string(),
        model: "unknown".to_string(),
        project: String::new(),
    };
    let mut previous_total = RawCounts::default();
    let mut ordinal = 0_u64;
    let mut records = Vec::new();
    let invalid = visit_values(path, |index, value| {
        context.update(&value);
        if index < start {
            if value.pointer("/payload/type").and_then(Value::as_str) == Some("token_count") {
                if let Some(total) = value.pointer("/payload/info/total_token_usage") {
                    previous_total = raw_counts(total);
                }
            }
            return;
        }
        if value.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
            return;
        }
        let total = value
            .pointer("/payload/info/total_token_usage")
            .map(raw_counts)
            .unwrap_or_default();
        let raw = value
            .pointer("/payload/info/last_token_usage")
            .map(raw_counts)
            .unwrap_or_else(|| total.diff(previous_total));
        if total.total > 0 || total.input > 0 || total.output > 0 {
            previous_total = total
        }
        let total_tokens = raw.total.max(raw.input.saturating_add(raw.output));
        if total_tokens == 0 {
            return;
        }
        ordinal += 1;
        records.push(UsageRecord {
            request_id: format!("codex:{}:{ordinal}", context.session),
            timestamp: timestamp_seconds(&value).unwrap_or(fallback),
            source_id: "codex".into(),
            model: context.model.clone(),
            session_id: context.session.clone(),
            project: context.project.clone(),
            input_tokens: raw
                .input
                .saturating_sub(raw.cached)
                .saturating_sub(raw.cache_write),
            output_tokens: raw.output.saturating_sub(raw.reasoning),
            reasoning_tokens: raw.reasoning,
            cache_read_tokens: raw.cached,
            cache_write_tokens: raw.cache_write,
            total_tokens,
        });
    })?;
    Ok((
        records,
        (invalid > 0).then(|| format!("skipped {invalid} unreadable lines")),
    ))
}

fn parse_claude(path: &Path, fallback: i64) -> Result<(Vec<UsageRecord>, Option<String>), String> {
    let fallback_session = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    let mut records = Vec::new();
    let invalid = visit_values(path, |line, value| {
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(usage) = value.pointer("/message/usage") else {
            return;
        };
        let message_id = value
            .pointer("/message/id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if message_id.starts_with("<synthetic>") {
            return;
        }
        let input = number(usage.get("input_tokens"));
        let output = number(usage.get("output_tokens"));
        let cache_read = number(usage.get("cache_read_input_tokens"));
        let cache_write = cache_creation(usage.get("cache_creation_input_tokens"))
            .max(cache_creation(usage.get("cache_creation")));
        let total = input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write);
        if total == 0 {
            return;
        }
        // A resumed session copies earlier turns into its own file; the API
        // request id still names the turn once when the message has no id.
        let api_request = value
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        records.push(UsageRecord {
            request_id: match (message_id.is_empty(), api_request) {
                (false, _) => format!("claude:{message_id}"),
                (true, Some(request)) => format!("claude-request:{request}"),
                (true, None) => format!("claude-noid:{}:{line}", path.display()),
            },
            timestamp: timestamp_seconds(&value).unwrap_or(fallback),
            source_id: "claude".into(),
            model: value
                .pointer("/message/model")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            session_id: text("sessionId").unwrap_or(fallback_session).to_string(),
            project: text("cwd").unwrap_or_default().to_string(),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: 0,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            total_tokens: total,
        });
    })?;
    Ok((
        records,
        (invalid > 0).then(|| format!("skipped {invalid} unreadable lines")),
    ))
}

fn visit_values(path: &Path, mut visit: impl FnMut(usize, Value)) -> Result<u64, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut index = 0_usize;
    let mut invalid = 0_u64;
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(value) => {
                visit(index, value);
                index += 1;
            }
            Err(_) => invalid += 1,
        }
    }
    Ok(invalid)
}

fn is_session_meta(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("session_meta")
        || value.pointer("/payload/type").and_then(Value::as_str) == Some("session_meta")
}

/// The session, model and working directory a Codex transcript is at.
struct CodexContext {
    session: String,
    model: String,
    project: String,
}

impl CodexContext {
    fn update(&mut self, value: &Value) {
        if is_session_meta(value) {
            if let Some(id) = value
                .pointer("/payload/id")
                .or_else(|| value.pointer("/payload/session_id"))
                .and_then(Value::as_str)
            {
                self.session = id.to_string();
            }
        }
        if let Some(next) = value
            .pointer("/payload/model")
            .or_else(|| value.pointer("/payload/info/model"))
            .and_then(Value::as_str)
        {
            self.model = next.to_string();
        }
        if let Some(cwd) = value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.is_empty())
        {
            self.project = cwd.to_string();
        }
    }
}

fn raw_counts(value: &Value) -> RawCounts {
    RawCounts {
        input: number(value.get("input_tokens")),
        cached: number(value.get("cached_input_tokens")),
        cache_write: number(value.get("cache_write_input_tokens")),
        output: number(value.get("output_tokens")),
        reasoning: number(value.get("reasoning_output_tokens")),
        total: number(value.get("total_tokens")),
    }
}

fn number(value: Option<&Value>) -> u64 {
    value
        .and_then(|value| {
            value.as_u64().or_else(|| {
                value
                    .as_i64()
                    .filter(|value| *value >= 0)
                    .map(|value| value as u64)
            })
        })
        .unwrap_or(0)
}

fn cache_creation(value: Option<&Value>) -> u64 {
    let Some(value) = value else { return 0 };
    if value.is_number() {
        return number(Some(value));
    }
    number(value.get("ephemeral_5m_input_tokens"))
        .saturating_add(number(value.get("ephemeral_1h_input_tokens")))
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.get("timestamp")?.as_str()?)
        .ok()
        .map(|value| value.timestamp_millis())
}
fn timestamp_seconds(value: &Value) -> Option<i64> {
    timestamp_ms(value).map(|value| value / 1000)
}
fn modified_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Syncs the transcripts under `root` into a ledger there and reads them back.
    fn synced(root: &Path) -> (Vec<TranscriptSource>, Transcripts) {
        let mut db = Connection::open(root.join("ledger.sqlite3")).unwrap();
        let sources = sync(&mut db, &root.join("codex"), &root.join("claude")).unwrap();
        let transcripts = match read(&db).unwrap() {
            Some(transcripts) => transcripts,
            None => {
                sync(&mut db, &root.join("codex"), &root.join("claude")).unwrap();
                read(&db).unwrap().unwrap()
            }
        };
        (sources, transcripts)
    }

    fn total(transcripts: &Transcripts) -> (usize, u64) {
        (
            transcripts.records.len(),
            transcripts
                .records
                .iter()
                .map(|record| record.total_tokens)
                .sum(),
        )
    }

    #[test]
    fn parses_codex_exclusive_dimensions_project_and_filters_lineage_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-test.jsonl");
        fs::write(&path, concat!(
            "{\"timestamp\":\"2026-09-25T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"cwd\":\"D:\\\\work\\\\harbor\"}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:00.100Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":99,\"output_tokens\":99,\"total_tokens\":198}}}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:01.000Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-test\"}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"cache_write_input_tokens\":20,\"output_tokens\":30,\"reasoning_output_tokens\":10,\"total_tokens\":130}}}}\n"
        )).unwrap();
        let (records, _) = parse_codex(&path, 0).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(
            (
                record.input_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens
            ),
            (20, 60, 20)
        );
        assert_eq!(
            (
                record.output_tokens,
                record.reasoning_tokens,
                record.total_tokens
            ),
            (20, 10, 130)
        );
        assert_eq!(record.model, "gpt-test");
        assert_eq!(record.session_id, "s1");
        assert_eq!(record.project, "D:\\work\\harbor");
    }

    #[test]
    fn recurses_claude_subagents_and_deduplicates_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir
            .path()
            .join("claude/projects/p/s/subagents/workflows/wf");
        fs::create_dir_all(&claude).unwrap();
        let message = |output: u64| {
            serde_json::json!({
                "timestamp": "2026-09-25T01:00:00Z",
                "type": "assistant",
                "sessionId": "s",
                "cwd": "/home/me/app",
                "message": {
                    "id": "m1",
                    "model": "claude-test",
                    "usage": {
                        "input_tokens": 10,
                        "cache_read_input_tokens": 3,
                        "cache_creation_input_tokens": {
                            "ephemeral_5m_input_tokens": 2,
                            "ephemeral_1h_input_tokens": 1
                        },
                        "output_tokens": output
                    }
                }
            })
            .to_string()
        };
        fs::write(
            claude.join("agent.jsonl"),
            format!("{}\n{}\n", message(2), message(5)),
        )
        .unwrap();
        let (sources, transcripts) = synced(dir.path());
        let claude = sources
            .iter()
            .find(|source| source.source_id == "claude")
            .unwrap();
        assert_eq!(claude.file_count, 1);
        assert_eq!(total(&transcripts), (1, 21));
        assert_eq!(transcripts.records[0].project, "/home/me/app");
    }

    #[test]
    fn a_resumed_copy_without_message_ids_counts_its_turns_once() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("claude/projects/p");
        fs::create_dir_all(&project).unwrap();
        let line = |request: &str| {
            serde_json::json!({
                "timestamp": "2026-09-25T01:00:00Z", "type": "assistant", "sessionId": "s", "requestId": request,
                "message": {"model": "claude-test", "usage": {"input_tokens": 10, "output_tokens": 5}}
            })
            .to_string()
        };
        fs::write(project.join("first.jsonl"), format!("{}\n", line("req_1"))).unwrap();
        fs::write(
            project.join("resumed.jsonl"),
            format!("{}\n{}\n", line("req_1"), line("req_2")),
        )
        .unwrap();
        assert_eq!(total(&synced(dir.path()).1), (2, 30));
        // A ledger kept by an earlier reading is parsed again, not trusted.
        let db = Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
        db.execute_batch("UPDATE source_files SET records_json='[]'; PRAGMA user_version=1;")
            .unwrap();
        drop(db);
        assert_eq!(total(&synced(dir.path()).1), (2, 30));
    }

    #[test]
    fn unreadable_rows_heal_from_the_transcripts_still_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("claude/projects/p");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("session.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-25T01:00:00Z\",\"type\":\"assistant\",",
                "\"sessionId\":\"s\",\"message\":{\"id\":\"m1\",",
                "\"model\":\"claude-test\",\"usage\":{\"input_tokens\":10,",
                "\"output_tokens\":5}}}\n"
            ),
        )
        .unwrap();
        assert_eq!(total(&synced(dir.path()).1), (1, 15));
        let db = Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
        db.execute("UPDATE source_files SET records_json='not-json'", [])
            .unwrap();
        drop(db);
        let (_, healed) = synced(dir.path());
        assert_eq!(total(&healed), (1, 15));
        assert!(healed.warnings.is_empty());
    }

    #[test]
    fn deleted_and_archived_transcripts_keep_their_records() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("codex/sessions/2026/09/25");
        let archived = dir.path().join("codex/archived_sessions");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&archived).unwrap();
        let rollout = live.join("rollout-a.jsonl");
        fs::write(&rollout, concat!(
            "{\"timestamp\":\"2026-09-25T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\"}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"output_tokens\":30,\"total_tokens\":130}}}}\n"
        )).unwrap();
        let codex = || {
            let (sources, transcripts) = synced(dir.path());
            let codex = sources
                .iter()
                .find(|source| source.source_id == "codex")
                .unwrap();
            (
                total(&transcripts).0,
                total(&transcripts).1,
                codex.file_count,
            )
        };
        assert_eq!(codex(), (1, 130, 1));

        let moved = archived.join("rollout-a.jsonl");
        fs::rename(&rollout, &moved).unwrap();
        assert_eq!(codex(), (1, 130, 1));

        fs::remove_file(&moved).unwrap();
        assert_eq!(codex(), (1, 130, 0));

        // A new reading parses the files on disk again but keeps the gone ones.
        let db = Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
        db.execute_batch("PRAGMA user_version=1;").unwrap();
        drop(db);
        assert_eq!(codex(), (1, 130, 0));
    }
}
