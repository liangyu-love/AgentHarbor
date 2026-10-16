use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

const LEDGER_FILE: &str = "token-usage-ledger.sqlite3";
/// How transcripts are read into records; a ledger kept by another reading
/// is parsed again (2: Claude turns without a message id go by request id).
const PARSE_VERSION: i64 = 2;
const DAILY_DAYS: i64 = 14;
const RECENT_LIMIT: usize = 24;
const LINEAGE_GAP_MS: i64 = 500;

fn ledger_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenUsageTotals {
    pub turn_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsageTotals {
    fn add(&mut self, record: &UsageRecord) {
        self.turn_count = self.turn_count.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(record.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(record.output_tokens);
        self.reasoning_tokens = self
            .reasoning_tokens
            .saturating_add(record.reasoning_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(record.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(record.cache_write_tokens);
        self.total_tokens = self.total_tokens.saturating_add(record.total_tokens);
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSource {
    pub source_id: String,
    pub source_name: String,
    pub available: bool,
    pub file_count: u64,
    pub updated_file_count: u64,
    pub totals: TokenUsageTotals,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenModelUsage {
    pub source_id: String,
    pub source_name: String,
    pub model: String,
    pub totals: TokenUsageTotals,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageDay {
    pub date: String,
    pub totals: TokenUsageTotals,
    #[serde(default)]
    pub by_model: Vec<TokenModelUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTokenUsage {
    pub timestamp: i64,
    pub date: String,
    pub source_id: String,
    pub source_name: String,
    pub model: String,
    pub session_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTokenUsagePayload {
    pub generated_at: i64,
    pub today: TokenUsageTotals,
    pub week: TokenUsageTotals,
    pub all_time: TokenUsageTotals,
    pub by_source: Vec<TokenUsageSource>,
    pub by_model: Vec<TokenModelUsage>,
    pub daily: Vec<TokenUsageDay>,
    pub recent: Vec<RecentTokenUsage>,
    pub warning_count: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageRecord {
    request_id: String,
    timestamp: i64,
    date: String,
    source_id: String,
    source_name: String,
    model: String,
    session_id: String,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_tokens: u64,
}

struct SourceFile {
    source_id: &'static str,
    path: PathBuf,
}

#[derive(Default)]
struct SourceScan {
    available: bool,
    files: u64,
    updated: u64,
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

pub fn load(
    codex_home: &Path,
    claude_home: &Path,
    data_dir: &Path,
) -> Result<LocalTokenUsagePayload, String> {
    let _guard = ledger_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let ledger = data_dir.join(LEDGER_FILE);
    match sync(&ledger, codex_home, claude_home) {
        Ok(payload) => Ok(payload),
        Err(first) => {
            remove_ledger_files(&ledger);
            sync(&ledger, codex_home, claude_home).map_err(|second| {
                format!("usage ledger rebuild failed: {second}; original error: {first}")
            })
        }
    }
}

fn remove_ledger_files(ledger: &Path) {
    let mut candidates = vec![ledger.to_path_buf()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut path = ledger.as_os_str().to_os_string();
        path.push(suffix);
        candidates.push(PathBuf::from(path));
    }
    for path in candidates {
        let _ = std::fs::remove_file(path);
    }
}

fn sync(
    ledger: &Path,
    codex_home: &Path,
    claude_home: &Path,
) -> Result<LocalTokenUsagePayload, String> {
    let mut db = Connection::open(ledger).map_err(|error| error.to_string())?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS source_files(
           path TEXT PRIMARY KEY, source TEXT NOT NULL, mtime_ms INTEGER NOT NULL,
           size INTEGER NOT NULL, records_json TEXT NOT NULL, warning TEXT
         );
         CREATE TABLE IF NOT EXISTS payload_cache(
           id INTEGER PRIMARY KEY CHECK(id=1), local_date TEXT NOT NULL,
           payload_json TEXT NOT NULL
         );",
    )
    .map_err(|error| error.to_string())?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != PARSE_VERSION {
        db.execute_batch(&format!(
            "DELETE FROM source_files; DELETE FROM payload_cache; PRAGMA user_version={PARSE_VERSION};"
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
    let mut scans = HashMap::from([
        (
            "codex",
            SourceScan {
                available: codex_roots.iter().any(|path| path.exists()),
                ..Default::default()
            },
        ),
        (
            "claude",
            SourceScan {
                available: claude_root.exists(),
                ..Default::default()
            },
        ),
    ]);
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
    let mut dirty = false;
    for stale_path in fingerprints
        .keys()
        .filter(|path| !seen_paths.contains(*path))
    {
        transaction
            .execute(
                "DELETE FROM source_files WHERE path=?1",
                params![stale_path],
            )
            .map_err(|error| error.to_string())?;
        dirty = true;
    }
    for source_file in files {
        let Some(metadata) = std::fs::metadata(&source_file.path).ok() else {
            continue;
        };
        let path = source_file.path.to_string_lossy().to_string();
        let mtime = modified_ns(&metadata);
        let size = metadata.len();
        if let Some(scan) = scans.get_mut(source_file.source_id) {
            scan.files += 1;
        }
        if fingerprints.get(&path).copied() == Some((mtime, size)) {
            continue;
        }
        let parsed = if source_file.source_id == "codex" {
            parse_codex(&source_file.path, mtime / 1000)
        } else {
            parse_claude(&source_file.path, mtime / 1000)
        };
        let (records, warning) = match parsed {
            Ok(result) => result,
            Err(error) => {
                dirty = true;
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
                if let Some(scan) = scans.get_mut(source_file.source_id) {
                    scan.updated += 1;
                }
                continue;
            }
        };
        dirty = true;
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
        if let Some(scan) = scans.get_mut(source_file.source_id) {
            scan.updated += 1;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;

    let today_key = Local::now().date_naive().format("%Y-%m-%d").to_string();
    if !dirty {
        let cached = db
            .query_row(
                "SELECT payload_json FROM payload_cache WHERE id=1 AND local_date=?1",
                params![today_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(json) = cached {
            if let Ok(mut payload) = serde_json::from_str::<LocalTokenUsagePayload>(&json) {
                payload.generated_at = Utc::now().timestamp();
                apply_scan_metadata(&mut payload, &scans);
                return Ok(payload);
            }
        }
    }

    let mut records = HashMap::<String, UsageRecord>::new();
    let mut warnings = Vec::new();
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
        let cached = serde_json::from_str::<Vec<UsageRecord>>(&json)
            .map_err(|error| format!("invalid cached usage for {path}: {error}"))?;
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
    let payload = aggregate(records, scans, warnings);
    let json = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
    if db
        .execute(
            "INSERT INTO payload_cache(id,local_date,payload_json) VALUES(1,?1,?2)
             ON CONFLICT(id) DO UPDATE SET local_date=excluded.local_date,
               payload_json=excluded.payload_json",
            params![today_key, json],
        )
        .is_err()
    {
        let _ = db.execute("DELETE FROM payload_cache WHERE id=1", []);
    }
    Ok(payload)
}

fn apply_scan_metadata(
    payload: &mut LocalTokenUsagePayload,
    scans: &HashMap<&'static str, SourceScan>,
) {
    for source in &mut payload.by_source {
        if let Some(scan) = scans.get(source.source_id.as_str()) {
            source.available = scan.available;
            source.file_count = scan.files;
            source.updated_file_count = scan.updated;
        }
    }
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
    let mut session = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut model = "unknown".to_string();
    let mut previous_total = RawCounts::default();
    let mut ordinal = 0_u64;
    let mut records = Vec::new();
    let invalid = visit_values(path, |index, value| {
        update_context(&value, &mut session, &mut model);
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
        let counts = exclusive(raw);
        if counts.total_tokens == 0 {
            return;
        }
        ordinal += 1;
        let timestamp = timestamp_seconds(&value).unwrap_or(fallback);
        records.push(UsageRecord {
            request_id: format!("codex:{session}:{ordinal}"),
            timestamp,
            date: local_date(timestamp),
            source_id: "codex".into(),
            source_name: "Codex".into(),
            model: model.clone(),
            session_id: session.clone(),
            input_tokens: counts.input_tokens,
            output_tokens: counts.output_tokens,
            reasoning_tokens: counts.reasoning_tokens,
            cache_read_tokens: counts.cache_read_tokens,
            cache_write_tokens: counts.cache_write_tokens,
            total_tokens: counts.total_tokens,
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
        let timestamp = timestamp_seconds(&value).unwrap_or(fallback);
        let session = value
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or(fallback_session)
            .to_string();
        // A resumed session copies earlier turns into its own file; the API
        // request id still names the turn once when the message has no id.
        let api_request = value
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        records.push(UsageRecord {
            request_id: match (message_id.is_empty(), api_request) {
                (false, _) => format!("claude:{message_id}"),
                (true, Some(request)) => format!("claude-request:{request}"),
                (true, None) => format!("claude-noid:{}:{line}", path.display()),
            },
            timestamp,
            date: local_date(timestamp),
            source_id: "claude".into(),
            source_name: "Claude Code".into(),
            model: value
                .pointer("/message/model")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            session_id: session,
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

fn aggregate(
    records: HashMap<String, UsageRecord>,
    scans: HashMap<&'static str, SourceScan>,
    mut warnings: Vec<String>,
) -> LocalTokenUsagePayload {
    let mut records = records.into_values().collect::<Vec<_>>();
    records.sort_by_key(|record| record.timestamp);
    let today = Local::now().date_naive();
    let week_start = today - ChronoDuration::days(today.weekday().num_days_from_monday() as i64);
    let first_day = today - ChronoDuration::days(DAILY_DAYS - 1);
    let mut today_total = TokenUsageTotals::default();
    let mut week_total = TokenUsageTotals::default();
    let mut all_time = TokenUsageTotals::default();
    let mut sources = HashMap::<String, TokenUsageTotals>::new();
    let mut models = HashMap::<(String, String), (String, TokenUsageTotals)>::new();
    let mut days = HashMap::<String, TokenUsageTotals>::new();
    let mut daily_models = HashMap::<(String, String, String), (String, TokenUsageTotals)>::new();
    for record in &records {
        all_time.add(record);
        if record.date == today.format("%Y-%m-%d").to_string() {
            today_total.add(record)
        }
        if NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .map(|date| date >= week_start && date <= today)
            .unwrap_or(false)
        {
            week_total.add(record);
        }
        sources
            .entry(record.source_id.clone())
            .or_default()
            .add(record);
        models
            .entry((record.source_id.clone(), record.model.clone()))
            .or_insert_with(|| (record.source_name.clone(), TokenUsageTotals::default()))
            .1
            .add(record);
        if NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .map(|date| date >= first_day && date <= today)
            .unwrap_or(false)
        {
            days.entry(record.date.clone()).or_default().add(record);
            daily_models
                .entry((
                    record.date.clone(),
                    record.source_id.clone(),
                    record.model.clone(),
                ))
                .or_insert_with(|| (record.source_name.clone(), TokenUsageTotals::default()))
                .1
                .add(record);
        }
    }
    let mut by_source = [("codex", "Codex"), ("claude", "Claude Code")]
        .into_iter()
        .map(|(id, name)| {
            let scan = scans.get(id);
            TokenUsageSource {
                source_id: id.into(),
                source_name: name.into(),
                available: scan.map(|value| value.available).unwrap_or(false),
                file_count: scan.map(|value| value.files).unwrap_or(0),
                updated_file_count: scan.map(|value| value.updated).unwrap_or(0),
                totals: sources.remove(id).unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    by_source.sort_by_key(|item| std::cmp::Reverse(item.totals.total_tokens));
    let mut by_model = models
        .into_iter()
        .map(
            |((source_id, model), (source_name, totals))| TokenModelUsage {
                source_id,
                source_name,
                model,
                totals,
            },
        )
        .collect::<Vec<_>>();
    by_model.sort_by_key(|item| std::cmp::Reverse(item.totals.total_tokens));
    let daily = (0..DAILY_DAYS)
        .map(|offset| first_day + ChronoDuration::days(offset))
        .map(|date| {
            let date = date.format("%Y-%m-%d").to_string();
            let mut by_model = daily_models
                .iter()
                .filter(|((day, _, _), _)| day == &date)
                .map(
                    |((_, source_id, model), (source_name, totals))| TokenModelUsage {
                        source_id: source_id.clone(),
                        source_name: source_name.clone(),
                        model: model.clone(),
                        totals: *totals,
                    },
                )
                .collect::<Vec<_>>();
            by_model.sort_by_key(|item| std::cmp::Reverse(item.totals.total_tokens));
            TokenUsageDay {
                totals: days.remove(&date).unwrap_or_default(),
                date,
                by_model,
            }
        })
        .collect();
    let recent = records
        .into_iter()
        .rev()
        .take(RECENT_LIMIT)
        .map(|record| RecentTokenUsage {
            timestamp: record.timestamp,
            date: record.date,
            source_id: record.source_id,
            source_name: record.source_name,
            model: record.model,
            session_id: record.session_id,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            reasoning_tokens: record.reasoning_tokens,
            cache_read_tokens: record.cache_read_tokens,
            cache_write_tokens: record.cache_write_tokens,
            total_tokens: record.total_tokens,
        })
        .collect();
    warnings.sort();
    warnings.dedup();
    let warning_count = warnings.len() as u64;
    warnings.truncate(8);
    LocalTokenUsagePayload {
        generated_at: Utc::now().timestamp(),
        today: today_total,
        week: week_total,
        all_time,
        by_source,
        by_model,
        daily,
        recent,
        warning_count,
        warnings,
    }
}

fn is_session_meta(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("session_meta")
        || value.pointer("/payload/type").and_then(Value::as_str) == Some("session_meta")
}

fn update_context(value: &Value, session: &mut String, model: &mut String) {
    if is_session_meta(value) {
        if let Some(id) = value
            .pointer("/payload/id")
            .or_else(|| value.pointer("/payload/session_id"))
            .and_then(Value::as_str)
        {
            *session = id.to_string();
        }
    }
    if let Some(next) = value
        .pointer("/payload/model")
        .or_else(|| value.pointer("/payload/info/model"))
        .and_then(Value::as_str)
    {
        *model = next.to_string();
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

fn exclusive(raw: RawCounts) -> TokenUsageTotals {
    TokenUsageTotals {
        turn_count: 1,
        input_tokens: raw
            .input
            .saturating_sub(raw.cached)
            .saturating_sub(raw.cache_write),
        output_tokens: raw.output.saturating_sub(raw.reasoning),
        reasoning_tokens: raw.reasoning,
        cache_read_tokens: raw.cached,
        cache_write_tokens: raw.cache_write,
        total_tokens: raw.total.max(raw.input.saturating_add(raw.output)),
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
fn local_date(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|value| value.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string())
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

    #[test]
    fn daily_totals_include_models_for_the_hovered_day() {
        let date = Local::now().format("%Y-%m-%d").to_string();
        let record = UsageRecord {
            request_id: "turn-1".into(),
            timestamp: Utc::now().timestamp(),
            date: date.clone(),
            source_id: "codex".into(),
            source_name: "Codex".into(),
            model: "gpt-test".into(),
            session_id: "session-1".into(),
            input_tokens: 11,
            output_tokens: 3,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 14,
        };
        let payload = aggregate(
            HashMap::from([(record.request_id.clone(), record)]),
            HashMap::new(),
            Vec::new(),
        );
        let today = payload.daily.iter().find(|day| day.date == date).unwrap();
        assert_eq!(today.by_model.len(), 1);
        assert_eq!(today.by_model[0].model, "gpt-test");
        assert_eq!(today.by_model[0].totals.total_tokens, 14);
    }

    #[test]
    fn parses_codex_exclusive_dimensions_and_filters_lineage_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-test.jsonl");
        fs::write(&path, concat!(
            "{\"timestamp\":\"2026-09-25T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\"}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:00.100Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":99,\"output_tokens\":99,\"total_tokens\":198}}}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:01.000Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-test\"}}\n",
            "{\"timestamp\":\"2026-09-25T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"cache_write_input_tokens\":20,\"output_tokens\":30,\"reasoning_output_tokens\":10,\"total_tokens\":130}}}}\n"
        )).unwrap();
        let (records, _) = parse_codex(&path, 0).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            (
                records[0].input_tokens,
                records[0].cache_read_tokens,
                records[0].cache_write_tokens
            ),
            (20, 60, 20)
        );
        assert_eq!(
            (
                records[0].output_tokens,
                records[0].reasoning_tokens,
                records[0].total_tokens
            ),
            (20, 10, 130)
        );
        assert_eq!(records[0].model, "gpt-test");
    }

    #[test]
    fn ledger_recurses_claude_subagents_and_deduplicates_snapshots() {
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
        let payload = load(
            &dir.path().join("codex"),
            &dir.path().join("claude"),
            dir.path(),
        )
        .unwrap();
        let source = payload
            .by_source
            .iter()
            .find(|source| source.source_id == "claude")
            .unwrap();
        assert_eq!(source.file_count, 1);
        assert_eq!(source.totals.turn_count, 1);
        assert_eq!(source.totals.total_tokens, 21);
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
        fs::write(project.join("resumed.jsonl"), format!("{}\n{}\n", line("req_1"), line("req_2"))).unwrap();
        let totals = || {
            let payload = load(&dir.path().join("codex"), &dir.path().join("claude"), dir.path()).unwrap();
            let source = payload.by_source.iter().find(|source| source.source_id == "claude").unwrap().clone();
            (source.totals.turn_count, source.totals.total_tokens)
        };
        assert_eq!(totals(), (2, 30));
        // A ledger kept by an earlier reading is parsed again, not trusted.
        let db = Connection::open(dir.path().join(LEDGER_FILE)).unwrap();
        db.execute_batch("UPDATE source_files SET records_json='[]'; DELETE FROM payload_cache; PRAGMA user_version=1;").unwrap();
        drop(db);
        assert_eq!(totals(), (2, 30));
    }

    #[test]
    fn payload_cache_skips_history_deserialization_and_deleted_files_invalidate_it() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude/projects/p");
        fs::create_dir_all(&claude).unwrap();
        let source_path = claude.join("session.jsonl");
        fs::write(
            &source_path,
            concat!(
                "{\"timestamp\":\"2026-09-25T01:00:00Z\",\"type\":\"assistant\",",
                "\"sessionId\":\"s\",\"message\":{\"id\":\"m1\",",
                "\"model\":\"claude-test\",\"usage\":{\"input_tokens\":10,",
                "\"output_tokens\":5}}}\n"
            ),
        )
        .unwrap();

        let codex = dir.path().join("codex");
        let claude_home = dir.path().join("claude");
        let first = load(&codex, &claude_home, dir.path()).unwrap();
        assert_eq!(first.all_time.total_tokens, 15);

        let ledger = dir.path().join(LEDGER_FILE);
        let db = Connection::open(&ledger).unwrap();
        db.execute("UPDATE source_files SET records_json='not-json'", [])
            .unwrap();
        drop(db);

        let cached = load(&codex, &claude_home, dir.path()).unwrap();
        assert_eq!(cached.all_time.total_tokens, 15);
        assert_eq!(cached.by_source[0].updated_file_count, 0);

        fs::remove_file(source_path).unwrap();
        let after_delete = load(&codex, &claude_home, dir.path()).unwrap();
        assert_eq!(after_delete.all_time.total_tokens, 0);
        assert_eq!(
            after_delete
                .by_source
                .iter()
                .map(|item| item.file_count)
                .sum::<u64>(),
            0
        );
        let db = Connection::open(ledger).unwrap();
        let remaining: u64 = db
            .query_row("SELECT COUNT(*) FROM source_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
