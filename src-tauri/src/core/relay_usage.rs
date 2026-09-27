use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const USAGE_FILE: &str = "relay-usage.jsonl";
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const RECENT_LIMIT: usize = 200;
const DAILY_WINDOW_DAYS: i64 = 14;

fn file_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn usage_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(USAGE_FILE)
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenCounts {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

impl TokenCounts {
    fn merge_max(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.max(other.reasoning_tokens);
        self.total_tokens = self.total_tokens.max(other.total_tokens);
    }

    fn normalize_total(&mut self) {
        self.total_tokens = self
            .total_tokens
            .max(self.input_tokens.saturating_add(self.output_tokens));
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsageRecord {
    pub timestamp: i64,
    pub date: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub protocol: String,
    pub status_code: u16,
    pub duration_ms: u64,
    #[serde(default)]
    pub first_token_ms: Option<u64>,
    pub success: bool,
    pub usage_reported: bool,
    #[serde(flatten)]
    pub tokens: TokenCounts,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsageTotals {
    pub request_count: u64,
    pub successful_count: u64,
    pub reported_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

impl RelayUsageTotals {
    fn add(&mut self, record: &RelayUsageRecord) {
        self.request_count = self.request_count.saturating_add(1);
        if record.success {
            self.successful_count = self.successful_count.saturating_add(1);
        }
        if record.usage_reported {
            self.reported_count = self.reported_count.saturating_add(1);
        }
        self.input_tokens = self.input_tokens.saturating_add(record.tokens.input_tokens);
        self.output_tokens = self
            .output_tokens
            .saturating_add(record.tokens.output_tokens);
        self.reasoning_tokens = self
            .reasoning_tokens
            .saturating_add(record.tokens.reasoning_tokens);
        self.total_tokens = self.total_tokens.saturating_add(record.tokens.total_tokens);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProviderUsage {
    pub provider_id: String,
    pub provider_name: String,
    pub totals: RelayUsageTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayModelUsage {
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub totals: RelayUsageTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsageDay {
    pub date: String,
    pub totals: RelayUsageTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsagePayload {
    pub generated_at: i64,
    pub today: RelayUsageTotals,
    pub week: RelayUsageTotals,
    pub all_time: RelayUsageTotals,
    pub by_provider: Vec<RelayProviderUsage>,
    pub by_model: Vec<RelayModelUsage>,
    pub daily: Vec<RelayUsageDay>,
    pub recent: Vec<RelayUsageRecord>,
}

pub struct RelayUsageRequest {
    app_data_dir: PathBuf,
    provider_id: String,
    provider_name: String,
    model: String,
    protocol: String,
    timestamp: i64,
    date: String,
    started: Instant,
}

impl RelayUsageRequest {
    pub fn new(
        app_data_dir: PathBuf,
        provider_id: String,
        provider_name: String,
        model: String,
        protocol: String,
    ) -> Self {
        Self {
            app_data_dir,
            provider_id,
            provider_name,
            model,
            protocol,
            timestamp: Utc::now().timestamp(),
            date: Local::now().format("%Y-%m-%d").to_string(),
            started: Instant::now(),
        }
    }

    pub fn failed(self, status_code: u16) {
        self.persist(status_code, false, TokenCounts::default(), None);
    }

    pub fn observe(self, status_code: u16, is_event_stream: bool) -> RelayUsageObserver {
        RelayUsageObserver {
            request: Some(self),
            status_code,
            is_event_stream,
            capture: Vec::new(),
            usage_reported: false,
            tokens: TokenCounts::default(),
            capture_truncated: false,
            first_token_ms: None,
        }
    }

    fn persist(self, status_code: u16, usage_reported: bool, mut tokens: TokenCounts, first_token_ms: Option<u64>) {
        tokens.normalize_total();
        let record = RelayUsageRecord {
            timestamp: self.timestamp,
            date: self.date,
            provider_id: self.provider_id,
            provider_name: self.provider_name,
            model: self.model,
            protocol: self.protocol,
            status_code,
            duration_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            first_token_ms,
            success: (200..300).contains(&status_code),
            usage_reported,
            tokens,
        };
        if let Err(error) = append_record(&self.app_data_dir, &record) {
            eprintln!("[AgentHarbor] relay usage write failed: {error}");
        }
    }
}

/// Observes upstream bytes while the router forwards them unchanged. It stores only
/// aggregate counters and request metadata; prompts, completions and credentials are
/// never persisted. Dropping the response body finalizes the record even on a client
/// disconnect.
pub struct RelayUsageObserver {
    request: Option<RelayUsageRequest>,
    status_code: u16,
    is_event_stream: bool,
    capture: Vec<u8>,
    usage_reported: bool,
    tokens: TokenCounts,
    capture_truncated: bool,
    first_token_ms: Option<u64>,
}

impl RelayUsageObserver {
    pub fn observe(&mut self, chunk: &[u8]) {
        if self.capture_truncated {
            return;
        }
        if self.capture.len().saturating_add(chunk.len()) > MAX_CAPTURE_BYTES {
            self.capture.clear();
            self.capture_truncated = true;
            return;
        }
        self.capture.extend_from_slice(chunk);
        if self.is_event_stream {
            self.drain_sse(false);
        }
    }

    fn drain_sse(&mut self, include_tail: bool) {
        while let Some((index, separator_len)) = event_boundary(&self.capture) {
            let event = self.capture[..index].to_vec();
            self.capture.drain(..index + separator_len);
            self.observe_sse_event(&event);
        }
        if include_tail && !self.capture.is_empty() {
            let event = std::mem::take(&mut self.capture);
            self.observe_sse_event(&event);
        }
    }

    fn observe_sse_event(&mut self, event: &[u8]) {
        let text = String::from_utf8_lossy(event);
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            if self.first_token_ms.is_none() && contains_output_delta(&value) {
                self.first_token_ms = self.request.as_ref().map(|request|
                    request.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            }
            self.merge_value(&value);
        }
    }

    fn merge_value(&mut self, value: &Value) {
        let mut found = false;
        let mut counts = TokenCounts::default();
        visit_usage_values(value, &mut found, &mut counts);
        if found {
            self.usage_reported = true;
            self.tokens.merge_max(counts);
        }
    }
}

impl Drop for RelayUsageObserver {
    fn drop(&mut self) {
        if !self.capture_truncated {
            if self.is_event_stream {
                self.drain_sse(true);
            } else if let Ok(value) = serde_json::from_slice::<Value>(&self.capture) {
                self.merge_value(&value);
            }
        }
        if let Some(request) = self.request.take() {
            request.persist(self.status_code, self.usage_reported, self.tokens, self.first_token_ms);
        }
    }
}

fn contains_output_delta(value: &Value) -> bool {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or_default();
    if matches!(event_type, "response.output_text.delta" | "content_block_delta") {
        return value.pointer("/delta/text").or_else(|| value.pointer("/delta/partial_json")).or_else(|| value.get("delta"))
            .and_then(Value::as_str).is_some_and(|text| !text.is_empty());
    }
    value.pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}

fn event_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes
        .windows(2)
        .position(|part| part == b"\n\n")
        .map(|i| (i, 2));
    let crlf = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn number(value: Option<&Value>) -> u64 {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
        })
        .unwrap_or(0)
}

fn counts_from_usage(value: &Value) -> Option<TokenCounts> {
    let object = value.as_object()?;
    let has_known_field = [
        "input_tokens",
        "output_tokens",
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "reasoning_tokens",
    ]
    .iter()
    .any(|key| object.contains_key(*key));
    if !has_known_field {
        return None;
    }

    let input_tokens = number(object.get("input_tokens"))
        .max(number(object.get("prompt_tokens")))
        .saturating_add(number(object.get("cache_creation_input_tokens")))
        .saturating_add(number(object.get("cache_read_input_tokens")));
    let output_tokens =
        number(object.get("output_tokens")).max(number(object.get("completion_tokens")));
    let reasoning_tokens = number(object.get("reasoning_tokens"))
        .max(number(
            object
                .get("output_tokens_details")
                .and_then(|details| details.get("reasoning_tokens")),
        ))
        .max(number(
            object
                .get("completion_tokens_details")
                .and_then(|details| details.get("reasoning_tokens")),
        ));
    let mut counts = TokenCounts {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens: number(object.get("total_tokens")),
    };
    counts.normalize_total();
    Some(counts)
}

fn visit_usage_values(value: &Value, found: &mut bool, counts: &mut TokenCounts) {
    match value {
        Value::Object(object) => {
            if let Some(usage) = object.get("usage") {
                if let Some(next) = counts_from_usage(usage) {
                    *found = true;
                    counts.merge_max(next);
                }
            }
            for child in object.values() {
                visit_usage_values(child, found, counts);
            }
        }
        Value::Array(items) => {
            for child in items {
                visit_usage_values(child, found, counts);
            }
        }
        _ => {}
    }
}

fn append_record(app_data_dir: &Path, record: &RelayUsageRecord) -> Result<(), String> {
    let _guard = file_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(usage_path(app_data_dir))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut file, record).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn read_records(app_data_dir: &Path) -> Result<Vec<RelayUsageRecord>, String> {
    let _guard = file_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = usage_path(app_data_dir);
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    Ok(BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<RelayUsageRecord>(&line).ok())
        .collect())
}

fn sort_totals(left: &RelayUsageTotals, right: &RelayUsageTotals) -> std::cmp::Ordering {
    right
        .total_tokens
        .cmp(&left.total_tokens)
        .then_with(|| right.request_count.cmp(&left.request_count))
}

pub fn load(app_data_dir: &Path) -> Result<RelayUsagePayload, String> {
    let records = read_records(app_data_dir)?;
    let today = Local::now().date_naive();
    let week_start = today - ChronoDuration::days(today.weekday().num_days_from_monday() as i64);
    let first_day = today - ChronoDuration::days(DAILY_WINDOW_DAYS - 1);
    let today_text = today.format("%Y-%m-%d").to_string();
    let mut today_totals = RelayUsageTotals::default();
    let mut week_totals = RelayUsageTotals::default();
    let mut all_time = RelayUsageTotals::default();
    let mut provider_totals: HashMap<String, (String, RelayUsageTotals)> = HashMap::new();
    let mut model_totals: HashMap<(String, String), (String, RelayUsageTotals)> = HashMap::new();
    let mut daily_totals: HashMap<String, RelayUsageTotals> = HashMap::new();

    for record in &records {
        all_time.add(record);
        if record.date == today_text {
            today_totals.add(record);
        }
        if NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .is_ok_and(|date| date >= week_start && date <= today)
        {
            week_totals.add(record);
        }
        let provider = provider_totals
            .entry(record.provider_id.clone())
            .or_insert_with(|| (record.provider_name.clone(), RelayUsageTotals::default()));
        provider.0.clone_from(&record.provider_name);
        provider.1.add(record);
        let model = model_totals
            .entry((record.provider_id.clone(), record.model.clone()))
            .or_insert_with(|| (record.provider_name.clone(), RelayUsageTotals::default()));
        model.0.clone_from(&record.provider_name);
        model.1.add(record);
        if NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .is_ok_and(|date| date >= first_day && date <= today)
        {
            daily_totals
                .entry(record.date.clone())
                .or_default()
                .add(record);
        }
    }

    let mut by_provider: Vec<_> = provider_totals
        .into_iter()
        .map(
            |(provider_id, (provider_name, totals))| RelayProviderUsage {
                provider_id,
                provider_name,
                totals,
            },
        )
        .collect();
    by_provider.sort_by(|left, right| sort_totals(&left.totals, &right.totals));

    let mut by_model: Vec<_> = model_totals
        .into_iter()
        .map(
            |((provider_id, model), (provider_name, totals))| RelayModelUsage {
                provider_id,
                provider_name,
                model,
                totals,
            },
        )
        .collect();
    by_model.sort_by(|left, right| sort_totals(&left.totals, &right.totals));

    let daily = (0..DAILY_WINDOW_DAYS)
        .map(|offset| first_day + ChronoDuration::days(offset))
        .map(|date| {
            let date = date.format("%Y-%m-%d").to_string();
            RelayUsageDay {
                totals: daily_totals.remove(&date).unwrap_or_default(),
                date,
            }
        })
        .collect();
    let recent = records.into_iter().rev().take(RECENT_LIMIT).collect();

    Ok(RelayUsagePayload {
        generated_at: Utc::now().timestamp(),
        today: today_totals,
        week: week_totals,
        all_time,
        by_provider,
        by_model,
        daily,
        recent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_first_content_event_without_treating_usage_as_text() {
        assert!(contains_output_delta(&serde_json::json!({"type":"response.output_text.delta","delta":"hello"})));
        assert!(contains_output_delta(&serde_json::json!({"type":"content_block_delta","delta":{"text":"hello"}})));
        assert!(contains_output_delta(&serde_json::json!({"choices":[{"delta":{"content":"hello"}}]})));
        assert!(!contains_output_delta(&serde_json::json!({"type":"response.completed","response":{"usage":{"input_tokens":1}}})));
    }

    #[test]
    fn parses_openai_and_anthropic_usage_without_counting_reasoning_twice() {
        let openai = serde_json::json!({
            "type": "response.completed",
            "response": {"usage": {
                "input_tokens": 120,
                "output_tokens": 30,
                "total_tokens": 150,
                "output_tokens_details": {"reasoning_tokens": 11}
            }}
        });
        let mut found = false;
        let mut counts = TokenCounts::default();
        visit_usage_values(&openai, &mut found, &mut counts);
        assert!(found);
        assert_eq!(counts.input_tokens, 120);
        assert_eq!(counts.output_tokens, 30);
        assert_eq!(counts.reasoning_tokens, 11);
        assert_eq!(counts.total_tokens, 150);

        let anthropic = serde_json::json!({"usage": {
            "input_tokens": 40,
            "cache_creation_input_tokens": 5,
            "cache_read_input_tokens": 10,
            "output_tokens": 9
        }});
        let mut found = false;
        let mut counts = TokenCounts::default();
        visit_usage_values(&anthropic, &mut found, &mut counts);
        assert!(found);
        assert_eq!(counts.input_tokens, 55);
        assert_eq!(counts.total_tokens, 64);
    }

    #[test]
    fn fragmented_sse_is_aggregated_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let request = RelayUsageRequest::new(
            dir.path().to_owned(),
            "provider-1".into(),
            "Provider".into(),
            "model-1".into(),
            "responses".into(),
        );
        {
            let mut observer = request.observe(200, true);
            observer.observe(b"event: response.completed\r\ndata: {\"type\":\"response.completed\",\"response\":{\"usa");
            observer.observe(
                b"ge\":{\"input_tokens\":10,\"output_tokens\":4,\"total_tokens\":14}}}\r\n\r\n",
            );
        }
        let payload = load(dir.path()).unwrap();
        assert_eq!(payload.all_time.request_count, 1);
        assert_eq!(payload.all_time.reported_count, 1);
        assert_eq!(payload.all_time.total_tokens, 14);
        assert_eq!(payload.by_provider[0].provider_name, "Provider");
    }

    #[test]
    fn unreported_failures_still_count_as_requests() {
        let dir = tempfile::tempdir().unwrap();
        RelayUsageRequest::new(
            dir.path().to_owned(),
            "provider-1".into(),
            "Provider".into(),
            "model-1".into(),
            "chatCompletions".into(),
        )
        .failed(502);
        let payload = load(dir.path()).unwrap();
        assert_eq!(payload.all_time.request_count, 1);
        assert_eq!(payload.all_time.successful_count, 0);
        assert_eq!(payload.all_time.reported_count, 0);
        assert_eq!(payload.all_time.total_tokens, 0);
    }
}
