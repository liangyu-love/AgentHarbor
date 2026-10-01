use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const USAGE_FILE: &str = "relay-usage.jsonl";
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const RECENT_LIMIT: usize = 200;
const DAILY_WINDOW_DAYS: i64 = 14;
/// Compact the ledger once it grows past this size.
const COMPACT_BYTES: u64 = 16 * 1024 * 1024;
/// Records newer than this stay itemized; the daily and weekly views need them.
const KEEP_DAYS: i64 = 35;

static NEXT_COMPACT_AT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(COMPACT_BYTES);

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
    /// Uncached input tokens when the protocol reports cache details.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    /// Cache hits and cache writes, tracked separately from input for pricing.
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenCounts {
    fn merge_max(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.max(other.reasoning_tokens);
        self.total_tokens = self.total_tokens.max(other.total_tokens);
        self.cache_read_tokens = self.cache_read_tokens.max(other.cache_read_tokens);
        self.cache_write_tokens = self.cache_write_tokens.max(other.cache_write_tokens);
    }

    fn normalize_total(&mut self) {
        self.total_tokens = self.total_tokens.max(
            self.input_tokens
                .saturating_add(self.cache_read_tokens)
                .saturating_add(self.cache_write_tokens)
                .saturating_add(self.output_tokens),
        );
    }

    fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }
}

/// Rates fixed when the request starts, so a later catalog refresh or a price
/// edit never rewrites what the ledger already recorded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PricingSnapshot {
    /// Base rates, per million tokens in USD.
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub reasoning: Option<f64>,
    /// Conditional rates that depend on the request's prompt size.
    pub context_over_200k: Option<super::models_dev::CostRates>,
    pub tiers: Vec<super::models_dev::CostTier>,
    /// manual | catalog | catalogAlias | unknown | ...
    pub source: String,
    pub revision: String,
    /// False when conditional pricing cannot be fully evaluated.
    pub complete: bool,
    pub currency: String,
}

impl PricingSnapshot {
    pub fn unknown(source: &str, revision: &str) -> Self {
        Self {
            source: source.into(),
            revision: revision.into(),
            complete: false,
            currency: "USD".into(),
            ..Default::default()
        }
    }

    pub fn is_known(&self) -> bool {
        self.input.is_some() || self.output.is_some()
    }

    /// Rates that apply to a prompt of this size. Tiers are inclusive lower
    /// bounds, so the largest matching bound wins; unknown tier conditions
    /// (anything but a context size) keep the base rates and mark the price
    /// incomplete rather than guessing.
    fn rates_for(&self, prompt_tokens: u64) -> (Rates, bool) {
        let mut rates = Rates {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            reasoning: self.reasoning,
        };
        let mut complete = self.complete;
        if let Some(over) = &self.context_over_200k {
            if prompt_tokens > 200_000 {
                rates = Rates::from(over);
            }
        }
        let mut selected: Option<Rates> = None;
        for tier in &self.tiers {
            if tier.tier.condition_type != "context" {
                complete = false;
                continue;
            }
            let Some(size) = tier.tier.size else {
                continue;
            };
            if prompt_tokens >= size {
                selected = Some(Rates::from(&tier.rates));
            }
        }
        if let Some(selected) = selected {
            rates = selected;
        }
        (rates, complete)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Rates {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    reasoning: Option<f64>,
}

impl From<&super::models_dev::CostRates> for Rates {
    fn from(rates: &super::models_dev::CostRates) -> Self {
        Self {
            input: rates.input,
            output: rates.output,
            cache_read: rates.cache_read,
            cache_write: rates.cache_write,
            reasoning: rates.reasoning,
        }
    }
}

/// `Some(cost)` when any rate applies. Missing cache rates fall back to the
/// input rate instead of pretending cached tokens are free; the caller marks
/// that request as partially priced.
fn request_cost(tokens: &TokenCounts, pricing: &PricingSnapshot) -> Option<(f64, bool)> {
    if !pricing.is_known() {
        return None;
    }
    let prompt = tokens.prompt_tokens();
    let (rates, complete) = pricing.rates_for(prompt);
    let input_rate = rates.input?;
    let output_rate = rates.output.unwrap_or(0.0);
    let cache_read_rate = rates.cache_read.unwrap_or(input_rate);
    let cache_write_rate = rates.cache_write.unwrap_or(input_rate);
    let reasoning_rate = rates.reasoning;
    let per_million = |tokens: u64, rate: f64| (tokens as f64) * rate / 1_000_000.0;
    let mut cost = per_million(tokens.input_tokens, input_rate)
        + per_million(tokens.cache_read_tokens, cache_read_rate)
        + per_million(tokens.cache_write_tokens, cache_write_rate);
    // Reasoning tokens are a subset of the output; bill them separately only
    // when the pricing declares a distinct rate, and never add them twice.
    let reasoning = tokens.reasoning_tokens.min(tokens.output_tokens);
    match reasoning_rate {
        Some(rate) if reasoning > 0 => {
            cost += per_million(reasoning, rate);
            cost += per_million(tokens.output_tokens.saturating_sub(reasoning), output_rate);
        }
        _ => cost += per_million(tokens.output_tokens, output_rate),
    }
    let missing_cache_rates = tokens.cache_read_tokens + tokens.cache_write_tokens > 0
        && (rates.cache_read.is_none() || rates.cache_write.is_none());
    Some((cost, complete && !missing_cache_rates))
}

fn to_micros(cost: f64) -> u64 {
    if !cost.is_finite() || cost <= 0.0 {
        return 0;
    }
    (cost * 1_000_000.0).round().min(u64::MAX as f64) as u64
}

pub fn micros_to_usd(micros: u64) -> f64 {
    (micros as f64) / 1_000_000.0
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
    /// Estimated cost in USD at the rates fixed when the request started.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// priced | partial | unpriced | legacy
    #[serde(default)]
    pub pricing_status: String,
    #[serde(default)]
    pub pricing_source: String,
    #[serde(default)]
    pub catalog_revision: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RelayUsageTotals {
    pub request_count: u64,
    pub successful_count: u64,
    pub reported_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Estimated cost in micro-USD, so small amounts survive accumulation.
    pub cost_micros: u64,
    /// Requests whose cost was recorded (fully or partially priced).
    pub priced_count: u64,
    /// Requests with no usable price (unknown price, legacy records).
    pub unpriced_count: u64,
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
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(record.tokens.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(record.tokens.cache_write_tokens);
        match (record.cost_usd, record.pricing_status.as_str()) {
            (Some(cost), _) if record.pricing_status != "unpriced" => {
                self.cost_micros = self.cost_micros.saturating_add(to_micros(cost));
                self.priced_count = self.priced_count.saturating_add(1);
            }
            _ => {
                self.unpriced_count = self.unpriced_count.saturating_add(1);
            }
        }
    }

    fn merge(&mut self, other: &Self) {
        self.request_count = self.request_count.saturating_add(other.request_count);
        self.successful_count = self.successful_count.saturating_add(other.successful_count);
        self.reported_count = self.reported_count.saturating_add(other.reported_count);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
        self.priced_count = self.priced_count.saturating_add(other.priced_count);
        self.unpriced_count = self.unpriced_count.saturating_add(other.unpriced_count);
    }
}

/// Totals of records folded out of the ledger by compaction, per provider and model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchivedModel {
    provider_id: String,
    provider_name: String,
    model: String,
    totals: RelayUsageTotals,
}

/// First line of a compacted ledger. Records never carry an `archive` field, so
/// the two line shapes cannot be confused.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ArchiveLine {
    archive: Vec<ArchivedModel>,
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
    pricing: PricingSnapshot,
}

impl RelayUsageRequest {
    pub fn new(
        app_data_dir: PathBuf,
        provider_id: String,
        provider_name: String,
        model: String,
        protocol: String,
        pricing: PricingSnapshot,
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
            pricing,
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
        // A request can carry reliable usage even when it failed or was cut
        // off, so the cost follows the tokens, not the HTTP status.
        let priced = request_cost(&tokens, &self.pricing);
        let (cost_usd, pricing_status) = match priced {
            Some((cost, true)) => (Some(cost), "priced"),
            Some((cost, false)) => (Some(cost), "partial"),
            None => (None, "unpriced"),
        };
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
            cost_usd,
            pricing_status: pricing_status.into(),
            pricing_source: self.pricing.source.clone(),
            catalog_revision: self.pricing.revision.clone(),
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

    // Cache details: Anthropic reports cache read/write beside an already
    // uncached input count; OpenAI-family payloads report a cached subset
    // inside the input total, which must be subtracted to stay non-overlapping.
    let detail_cached = number(value.pointer("/input_tokens_details/cached_tokens"))
        .max(number(value.pointer("/prompt_tokens_details/cached_tokens")));
    let anthropic_read = number(object.get("cache_read_input_tokens"));
    let cache_read_tokens = anthropic_read.max(detail_cached);
    let cache_write_tokens = number(object.get("cache_creation_input_tokens"));
    let reported_input = number(object.get("input_tokens")).max(number(object.get("prompt_tokens")));
    let input_tokens = if detail_cached > 0 {
        reported_input.saturating_sub(detail_cached)
    } else {
        reported_input
    };
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
        cache_read_tokens,
        cache_write_tokens,
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
    file.flush().map_err(|error| error.to_string())?;
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    drop(file);
    if size >= NEXT_COMPACT_AT.load(std::sync::atomic::Ordering::Relaxed) {
        if let Err(error) = compact_locked(app_data_dir) {
            log::warn!("relay usage compaction failed: {error}");
        }
        // Even when little can be folded, do not retry on every request.
        let size = std::fs::metadata(usage_path(app_data_dir)).map(|meta| meta.len()).unwrap_or(0);
        NEXT_COMPACT_AT.store(size.max(COMPACT_BYTES) + COMPACT_BYTES / 2, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// Parsed ledger: archived totals from earlier compactions plus itemized records.
#[derive(Default)]
struct Ledger {
    archive: Vec<ArchivedModel>,
    records: Vec<RelayUsageRecord>,
}

fn parse_ledger(bytes: &[u8]) -> Ledger {
    let mut ledger = Ledger::default();
    // Split on raw bytes: a crash can leave a line cut mid UTF-8 sequence, and that one
    // record must be skipped without dropping every record after it.
    for line in bytes.split(|byte| *byte == b'\n') {
        if let Ok(record) = serde_json::from_slice::<RelayUsageRecord>(line) {
            ledger.records.push(record);
        } else if let Ok(archive) = serde_json::from_slice::<ArchiveLine>(line) {
            ledger.archive.extend(archive.archive);
        }
    }
    ledger
}

fn read_ledger(app_data_dir: &Path) -> Result<Ledger, String> {
    // Hold the lock only for the read; parsing and aggregation run without it so
    // the proxy can keep appending usage while the dashboard loads.
    let bytes = {
        let _guard = file_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match std::fs::read(usage_path(app_data_dir)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(error) => return Err(error.to_string()),
        }
    };
    Ok(parse_ledger(&bytes))
}

/// Fold records older than `KEEP_DAYS` into per-model archived totals, so the
/// ledger stays bounded while all-time and per-provider/model totals stay exact.
/// Caller holds the file lock.
fn compact_locked(app_data_dir: &Path) -> Result<(), String> {
    let path = usage_path(app_data_dir);
    let ledger = parse_ledger(&std::fs::read(&path).map_err(|error| error.to_string())?);
    let cutoff = (Local::now().date_naive() - ChronoDuration::days(KEEP_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    let mut archive: HashMap<(String, String), ArchivedModel> = HashMap::new();
    for entry in ledger.archive {
        archive
            .entry((entry.provider_id.clone(), entry.model.clone()))
            .or_insert_with(|| ArchivedModel { totals: RelayUsageTotals::default(), ..entry.clone() })
            .totals
            .merge(&entry.totals);
    }
    let mut kept = Vec::new();
    for record in ledger.records {
        if record.date >= cutoff {
            kept.push(record);
            continue;
        }
        let entry = archive
            .entry((record.provider_id.clone(), record.model.clone()))
            .or_insert_with(|| ArchivedModel {
                provider_id: record.provider_id.clone(),
                provider_name: record.provider_name.clone(),
                model: record.model.clone(),
                totals: RelayUsageTotals::default(),
            });
        entry.provider_name.clone_from(&record.provider_name);
        entry.totals.add(&record);
    }
    let mut output = Vec::new();
    if !archive.is_empty() {
        let mut archive: Vec<_> = archive.into_values().collect();
        archive.sort_by(|left, right| (&left.provider_id, &left.model).cmp(&(&right.provider_id, &right.model)));
        serde_json::to_writer(&mut output, &ArchiveLine { archive }).map_err(|error| error.to_string())?;
        output.push(b'\n');
    }
    for record in &kept {
        serde_json::to_writer(&mut output, record).map_err(|error| error.to_string())?;
        output.push(b'\n');
    }
    super::storage::write_atomic(&path, &output).map_err(|error| error.to_string())
}

fn sort_totals(left: &RelayUsageTotals, right: &RelayUsageTotals) -> std::cmp::Ordering {
    right
        .total_tokens
        .cmp(&left.total_tokens)
        .then_with(|| right.request_count.cmp(&left.request_count))
}

pub fn load(app_data_dir: &Path) -> Result<RelayUsagePayload, String> {
    let Ledger { archive, records } = read_ledger(app_data_dir)?;
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

    // Compacted records are older than every time window, so they only count
    // toward all-time and per-provider/model totals.
    for entry in &archive {
        all_time.merge(&entry.totals);
        provider_totals
            .entry(entry.provider_id.clone())
            .or_insert_with(|| (entry.provider_name.clone(), RelayUsageTotals::default()))
            .1
            .merge(&entry.totals);
        model_totals
            .entry((entry.provider_id.clone(), entry.model.clone()))
            .or_insert_with(|| (entry.provider_name.clone(), RelayUsageTotals::default()))
            .1
            .merge(&entry.totals);
    }

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

    fn record_on(date: &str, provider: &str, model: &str, tokens: u64) -> RelayUsageRecord {
        RelayUsageRecord {
            timestamp: 0,
            date: date.into(),
            provider_id: provider.into(),
            provider_name: provider.to_uppercase(),
            model: model.into(),
            protocol: "responses".into(),
            status_code: 200,
            duration_ms: 1,
            first_token_ms: None,
            success: true,
            usage_reported: true,
            tokens: TokenCounts { input_tokens: tokens, output_tokens: 0, reasoning_tokens: 0, total_tokens: tokens, cache_read_tokens: 0, cache_write_tokens: 0 },
            cost_usd: None,
            pricing_status: "unpriced".into(),
            pricing_source: String::new(),
            catalog_revision: String::new(),
        }
    }

    fn pricing() -> PricingSnapshot {
        PricingSnapshot {
            input: Some(4.0),
            output: Some(20.0),
            cache_read: Some(0.4),
            cache_write: Some(5.0),
            source: "catalog".into(),
            revision: "abc".into(),
            complete: true,
            currency: "USD".into(),
            ..Default::default()
        }
    }

    #[test]
    fn costs_use_non_overlapping_token_categories() {
        let tokens = TokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            reasoning_tokens: 20_000,
            cache_read_tokens: 2_000_000,
            cache_write_tokens: 100_000,
            ..Default::default()
        };
        let (cost, complete) = request_cost(&tokens, &pricing()).unwrap();
        assert!(complete);
        // 1M*4 + 0.1M*20 + 2M*0.4 + 0.1M*5 = 4 + 2 + 0.8 + 0.5
        assert!((cost - 7.3).abs() < 1e-9, "{cost}");
        // Reasoning is inside the output count and has no separate rate.
        assert!((cost - 7.3).abs() < 1e-9);
    }

    #[test]
    fn a_distinct_reasoning_rate_never_double_bills_the_output() {
        let mut price = pricing();
        price.reasoning = Some(30.0);
        let tokens = TokenCounts {
            input_tokens: 0,
            output_tokens: 1_000_000,
            reasoning_tokens: 400_000,
            ..Default::default()
        };
        let (cost, _) = request_cost(&tokens, &price).unwrap();
        // 0.6M at the output rate + 0.4M at the reasoning rate.
        assert!((cost - (0.6 * 20.0 + 0.4 * 30.0)).abs() < 1e-9, "{cost}");
    }

    #[test]
    fn missing_cache_rates_are_estimated_at_the_input_rate_and_flagged_partial() {
        let mut price = pricing();
        price.cache_read = None;
        let tokens = TokenCounts {
            input_tokens: 0,
            cache_read_tokens: 1_000_000,
            ..Default::default()
        };
        let (cost, complete) = request_cost(&tokens, &price).unwrap();
        assert!(!complete, "missing cache rate must be visible");
        assert!((cost - 4.0).abs() < 1e-9, "cached tokens are not free: {cost}");
    }

    #[test]
    fn conditional_pricing_selects_the_matching_tier() {
        let mut price = pricing();
        price.tiers = vec![super::super::models_dev::CostTier {
            rates: super::super::models_dev::CostRates {
                input: Some(6.0),
                output: Some(30.0),
                cache_read: Some(0.6),
                ..Default::default()
            },
            tier: super::super::models_dev::TierCondition {
                condition_type: "context".into(),
                size: Some(200_000),
            },
        }];
        price.complete = false;
        let small = TokenCounts { input_tokens: 100_000, ..Default::default() };
        let (cost, _) = request_cost(&small, &price).unwrap();
        assert!((cost - 0.4).abs() < 1e-9, "base rate below the tier: {cost}");
        let large = TokenCounts { input_tokens: 400_000, ..Default::default() };
        let (cost, _) = request_cost(&large, &price).unwrap();
        assert!((cost - 2.4).abs() < 1e-9, "tier rate above the bound: {cost}");
    }

    #[test]
    fn usage_with_a_price_is_recorded_and_legacy_records_stay_unpriced() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut observer = RelayUsageRequest::new(
                dir.path().to_owned(),
                "provider-1".into(),
                "Provider".into(),
                "model-1".into(),
                "responses".into(),
                pricing(),
            )
            .observe(200, false);
            let payload = serde_json::json!({"usage": {"input_tokens": 1_000_000, "output_tokens": 0}});
            observer.observe(payload.to_string().as_bytes());
        }
        // A legacy line without pricing fields is read as unpriced, not as 0.
        let legacy = r#"{"timestamp":1,"date":"2020-01-01","providerId":"p","providerName":"P","model":"m","protocol":"responses","statusCode":200,"durationMs":1,"success":true,"usageReported":true,"inputTokens":10,"outputTokens":1,"reasoningTokens":0,"totalTokens":11}"#;
        std::fs::OpenOptions::new()
            .append(true)
            .open(usage_path(dir.path()))
            .unwrap()
            .write_all(format!("{legacy}\n").as_bytes())
            .unwrap();

        let payload = load(dir.path()).unwrap();
        assert_eq!(payload.all_time.request_count, 2);
        assert_eq!(payload.all_time.priced_count, 1);
        assert_eq!(payload.all_time.unpriced_count, 1);
        assert!((micros_to_usd(payload.all_time.cost_micros) - 4.0).abs() < 1e-9);
        let provider = payload
            .by_provider
            .iter()
            .find(|entry| entry.provider_id == "provider-1")
            .unwrap();
        assert!((micros_to_usd(provider.totals.cost_micros) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn compaction_folds_old_records_without_changing_totals() {
        let dir = tempfile::tempdir().unwrap();
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        let old = (Local::now().date_naive() - ChronoDuration::days(KEEP_DAYS + 10))
            .format("%Y-%m-%d")
            .to_string();
        for record in [
            record_on(&old, "p", "m", 5),
            record_on(&old, "p", "m", 7),
            record_on(&old, "q", "n", 3),
            record_on(&today, "p", "m", 11),
        ] {
            append_record(dir.path(), &record).unwrap();
        }
        // A crash-truncated line must not break the compacted file either.
        std::fs::OpenOptions::new().append(true).open(usage_path(dir.path())).unwrap()
            .write_all(b"{\"timestamp\":1,\"da\n").unwrap();
        let before = load(dir.path()).unwrap();

        compact_locked(dir.path()).unwrap();
        let text = std::fs::read_to_string(usage_path(dir.path())).unwrap();
        assert_eq!(text.lines().count(), 2, "one archive line plus the recent record: {text}");
        let after = load(dir.path()).unwrap();
        assert_eq!(after.all_time.total_tokens, before.all_time.total_tokens);
        assert_eq!(after.all_time.request_count, 4);
        assert_eq!(after.today.total_tokens, 11);
        let p = after.by_provider.iter().find(|entry| entry.provider_id == "p").unwrap();
        assert_eq!(p.totals.total_tokens, 23);

        // Compacting again merges into the existing archive instead of duplicating it.
        compact_locked(dir.path()).unwrap();
        assert_eq!(load(dir.path()).unwrap().all_time.total_tokens, 26);
    }

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
        // Anthropic keeps cache categories separate from the uncached input.
        assert_eq!(counts.input_tokens, 40);
        assert_eq!(counts.cache_read_tokens, 10);
        assert_eq!(counts.cache_write_tokens, 5);
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
            PricingSnapshot::unknown("unknown", "fixture"),
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
            PricingSnapshot::unknown("unknown", "fixture"),
        )
        .failed(502);
        let payload = load(dir.path()).unwrap();
        assert_eq!(payload.all_time.request_count, 1);
        assert_eq!(payload.all_time.successful_count, 0);
        assert_eq!(payload.all_time.reported_count, 0);
        assert_eq!(payload.all_time.total_tokens, 0);
    }
}
