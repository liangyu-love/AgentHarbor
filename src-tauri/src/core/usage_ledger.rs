//! The usage ledger behind the usage page. Local Codex and Claude Code
//! transcripts and the router's request log are joined, so a request routed
//! through AgentHarbor counts once, with the client it came from, the channel
//! it went through and what it cost.
use super::local_usage::{self, TranscriptSource, UsageRecord};
use super::relay_usage::{self, PricingSnapshot, RelayUsageRecord, TokenCounts};
use super::usage_cache::CacheUsageTotals;
use super::{models_dev, router};
use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const LEDGER_FILE: &str = "token-usage-ledger.sqlite3";
const RECENT_LIMIT: usize = 200;
const RANK_LIMIT: usize = 12;
/// Series a chart stacks before the rest is folded into one.
const PART_LIMIT: usize = 6;
/// A transcript logs a routed call while it streams or shortly after the
/// router request that carried it ends.
const MATCH_LEAD_SECS: i64 = 30;
const MATCH_LAG_SECS: i64 = 120;
/// Requests are logged when they end, so a long one lands after shorter later
/// ones; new requests are taken from this far behind the newest one kept.
const INGEST_OVERLAP_SECS: i64 = 6 * 3600;
const KNOWN_CLIENTS: [&str; 8] = [
    "codex",
    "claudeCode",
    "opencode",
    "pi",
    "harness",
    "claudeDesktop",
    "cursor",
    "aggregate",
];

fn ledger_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Where the transcripts and the ledger live.
pub struct LedgerPaths<'a> {
    pub codex_home: &'a Path,
    pub claude_home: &'a Path,
    pub data_dir: &'a Path,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageRange {
    Today,
    #[default]
    Week,
    Month,
}

impl UsageRange {
    fn days(self) -> i64 {
        match self {
            Self::Today => 1,
            Self::Week => 7,
            Self::Month => 30,
        }
    }
}

/// How a call reached its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    /// Straight to the client's own account; never through a relay.
    Official,
    Relay,
    Aggregate,
}

impl Channel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Relay => "relay",
            Self::Aggregate => "aggregate",
        }
    }
}

/// One model call, from whichever records saw it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    pub timestamp: i64,
    pub client: String,
    pub channel: Channel,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    /// The model that served and billed the call.
    pub model: String,
    /// What the client asked for when the router mapped it to another model.
    pub requested_model: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    /// Input excludes cache reads and writes; output includes reasoning.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub usage_reported: bool,
    pub cache_reported: bool,
    pub cost_usd: Option<f64>,
    /// priced | partial | unpriced | legacy
    pub pricing_status: String,
    /// Priced afterwards at current rates, not recorded when the call ran.
    pub estimated: bool,
    pub pricing_source: Option<String>,
    pub multiplier: Option<f64>,
    pub cache_savings_usd: Option<f64>,
    /// What only the router saw; absent for calls that never went through it.
    pub request: Option<RequestFacts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFacts {
    pub status_code: u16,
    pub success: bool,
    pub duration_ms: u64,
    pub first_token_ms: Option<u64>,
    pub served_model: Option<String>,
    pub model_match_status: String,
    pub credential_status: String,
    pub credential_fingerprint: Option<String>,
    /// Joined with the transcript turn of the same call.
    pub matched: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    /// What the models processed anew: input, cache writes and output.
    pub new_tokens: u64,
    pub cost_micros: u64,
    pub priced_count: u64,
    pub unpriced_count: u64,
    pub partial_count: u64,
    pub coverage_unknown_count: u64,
    /// The part of the cost priced afterwards.
    pub estimated_micros: u64,
    pub estimated_count: u64,
    pub cache_savings_micros: u64,
    pub cache_usage: CacheUsageTotals,
}

impl UsageTotals {
    fn add(&mut self, entry: &UsageEntry) {
        self.request_count += 1;
        self.input_tokens = self.input_tokens.saturating_add(entry.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(entry.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(entry.reasoning_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(entry.cache_write_tokens);
        self.total_tokens = self.total_tokens.saturating_add(entry.total_tokens);
        self.new_tokens = self.new_tokens.saturating_add(new_tokens(entry));
        if entry.usage_reported && entry.cache_reported {
            self.cache_usage.add(
                entry.input_tokens,
                entry.cache_read_tokens,
                entry.cache_write_tokens,
            );
        }
        match entry.cost_usd {
            Some(cost) if entry.pricing_status != "unpriced" => {
                let micros = relay_usage::to_micros(cost);
                self.cost_micros = self.cost_micros.saturating_add(micros);
                self.priced_count += 1;
                match entry.pricing_status.as_str() {
                    "priced" => {}
                    "partial" => self.partial_count += 1,
                    _ => self.coverage_unknown_count += 1,
                }
                if entry.estimated {
                    self.estimated_micros = self.estimated_micros.saturating_add(micros);
                    self.estimated_count += 1;
                }
            }
            _ => self.unpriced_count += 1,
        }
        if let Some(savings) = entry.cache_savings_usd {
            self.cache_savings_micros = self
                .cache_savings_micros
                .saturating_add(relay_usage::to_micros(savings));
        }
    }
}

fn new_tokens(entry: &UsageEntry) -> u64 {
    entry
        .input_tokens
        .saturating_add(entry.cache_write_tokens)
        .saturating_add(entry.output_tokens)
}

/// One row of a ranking.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub key: String,
    pub label: String,
    pub detail: Option<String>,
    pub client: Option<String>,
    pub channel: Option<Channel>,
    pub provider_id: Option<String>,
    pub totals: UsageTotals,
}

/// One series of a chart bucket.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePart {
    pub key: String,
    pub new_tokens: u64,
    pub cost_micros: u64,
}

/// An hour of today, or a day of a longer range.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub key: String,
    pub start: i64,
    pub totals: UsageTotals,
    pub by_client: Vec<UsagePart>,
    pub by_channel: Vec<UsagePart>,
    pub by_model: Vec<UsagePart>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSources {
    pub transcripts: Vec<TranscriptSource>,
    /// When the oldest router request the ledger keeps ran.
    pub relay_since: Option<i64>,
    pub relay_count: u64,
    pub matched_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePayload {
    pub generated_at: i64,
    pub range: UsageRange,
    /// The range runs from `start` to now; `previous` is the same span before it.
    pub start: i64,
    pub previous_start: i64,
    pub today: UsageTotals,
    pub current: UsageTotals,
    pub previous: UsageTotals,
    pub all_time: UsageTotals,
    pub buckets: Vec<UsageBucket>,
    pub by_client: Vec<UsageGroup>,
    pub by_channel: Vec<UsageGroup>,
    pub by_model: Vec<UsageGroup>,
    pub by_project: Vec<UsageGroup>,
    pub by_session: Vec<UsageGroup>,
    pub recent: Vec<UsageEntry>,
    pub sources: UsageSources,
    pub warning_count: u64,
    pub warnings: Vec<String>,
}

/// What the ledger keeps of a router request: enough to count, price and
/// explain it, without the full price snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RelaySample {
    timestamp: i64,
    client_source: Option<String>,
    requested_model: Option<String>,
    provider_id: String,
    provider_name: String,
    model: String,
    status_code: u16,
    success: bool,
    duration_ms: u64,
    first_token_ms: Option<u64>,
    usage_reported: bool,
    cache_reported: bool,
    #[serde(flatten)]
    tokens: TokenCounts,
    cost_usd: Option<f64>,
    pricing_status: String,
    pricing_source: String,
    multiplier: Option<f64>,
    cache_savings_usd: Option<f64>,
    served_model: Option<String>,
    model_match_status: String,
    credential_status: String,
    credential_fingerprint: Option<String>,
}

impl From<&RelayUsageRecord> for RelaySample {
    fn from(record: &RelayUsageRecord) -> Self {
        let pricing = record.pricing_snapshot.as_ref();
        Self {
            timestamp: record.timestamp,
            client_source: record.identity.client_source.clone(),
            requested_model: record.identity.requested_model.clone(),
            provider_id: record.provider_id.clone(),
            provider_name: record.provider_name.clone(),
            model: record.model.clone(),
            status_code: record.status_code,
            success: record.success,
            duration_ms: record.duration_ms,
            first_token_ms: record.first_token_ms,
            usage_reported: record.usage_reported,
            cache_reported: record.cache_reported,
            tokens: record.tokens,
            cost_usd: record.cost_usd,
            pricing_status: record.pricing_status.clone(),
            pricing_source: record.pricing_source.clone(),
            multiplier: pricing.and_then(|pricing| pricing.multiplier),
            cache_savings_usd: pricing
                .and_then(|pricing| relay_usage::cache_savings(&record.tokens, pricing)),
            served_model: record.model_observation.served_model.clone(),
            model_match_status: record.model_observation.model_match_status.clone(),
            credential_status: record.credential.status.clone(),
            credential_fingerprint: record.credential.version_fingerprint.clone(),
        }
    }
}

/// Every field that tells two log lines apart: a retried attempt shares its
/// request id, and older lines carry no attempt id at all.
fn sample_key(record: &RelayUsageRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        record.attempt_id,
        record.request_id,
        record.timestamp,
        record.provider_id,
        record.model,
        record.tokens.total_tokens,
        record.status_code,
        record.duration_ms
    )
}

struct Collected {
    sources: Vec<TranscriptSource>,
    records: Vec<UsageRecord>,
    warnings: Vec<String>,
    samples: Vec<RelaySample>,
}

/// Counts what is on disk into the ledger, so transcripts about to be deleted
/// keep their usage.
pub fn record(paths: &LedgerPaths) -> Result<(), String> {
    collect_locked(paths).map(drop)
}

pub fn load(
    paths: &LedgerPaths,
    store: Option<&router::Store>,
    catalog: &models_dev::Catalog,
    range: UsageRange,
) -> Result<UsagePayload, String> {
    let collected = collect_locked(paths)?;
    let entries = join(&collected, store, catalog);
    Ok(summarize(&collected, entries, range, Local::now()))
}

fn collect_locked(paths: &LedgerPaths) -> Result<Collected, String> {
    let _guard = ledger_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::fs::create_dir_all(paths.data_dir).map_err(|error| error.to_string())?;
    let relay = relay_usage::itemized(paths.data_dir)?;
    let ledger = paths.data_dir.join(LEDGER_FILE);
    match collect(&ledger, paths, &relay) {
        Ok(collected) => Ok(collected),
        Err(first) => {
            set_aside(&ledger);
            collect(&ledger, paths, &relay).map_err(|second| {
                format!("usage ledger rebuild failed: {second}; original error: {first}")
            })
        }
    }
}

/// The ledger outlives deleted transcripts and compacted request logs, so a
/// broken one is kept aside (replacing the last one kept) instead of deleted.
fn set_aside(ledger: &Path) {
    let aside = ledger.with_extension("sqlite3.broken");
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let with_suffix = |path: &Path| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        };
        let target = with_suffix(&aside);
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::rename(with_suffix(ledger), target);
    }
}

fn collect(
    ledger: &Path,
    paths: &LedgerPaths,
    relay: &[RelayUsageRecord],
) -> Result<Collected, String> {
    let mut db = Connection::open(ledger).map_err(|error| error.to_string())?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         DROP TABLE IF EXISTS payload_cache;
         CREATE TABLE IF NOT EXISTS relay_requests(
           id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL, sample_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS relay_requests_time ON relay_requests(timestamp);",
    )
    .map_err(|error| error.to_string())?;
    let mut sources = local_usage::sync(&mut db, paths.codex_home, paths.claude_home)?;
    let transcripts = match local_usage::read(&db)? {
        Some(transcripts) => transcripts,
        None => {
            sources = local_usage::sync(&mut db, paths.codex_home, paths.claude_home)?;
            local_usage::read(&db)?.ok_or("usage ledger rows stay unreadable")?
        }
    };
    ingest(&mut db, relay)?;
    Ok(Collected {
        sources,
        records: transcripts.records,
        warnings: transcripts.warnings,
        samples: samples(&db)?,
    })
}

/// Keeps router requests past the log's own compaction.
fn ingest(db: &mut Connection, relay: &[RelayUsageRecord]) -> Result<(), String> {
    let newest: Option<i64> = db
        .query_row("SELECT MAX(timestamp) FROM relay_requests", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    let since = newest.map_or(i64::MIN, |newest| newest - INGEST_OVERLAP_SECS);
    let transaction = db.transaction().map_err(|error| error.to_string())?;
    {
        let mut insert = transaction
            .prepare(
                "INSERT OR IGNORE INTO relay_requests(id,timestamp,sample_json) VALUES(?1,?2,?3)",
            )
            .map_err(|error| error.to_string())?;
        for record in relay.iter().filter(|record| record.timestamp >= since) {
            let json = serde_json::to_string(&RelaySample::from(record))
                .map_err(|error| error.to_string())?;
            insert
                .execute(params![sample_key(record), record.timestamp, json])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn samples(db: &Connection) -> Result<Vec<RelaySample>, String> {
    let mut query = db
        .prepare("SELECT sample_json FROM relay_requests ORDER BY timestamp")
        .map_err(|error| error.to_string())?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut samples = Vec::new();
    for row in rows {
        // A row this build cannot read is skipped; the log it came from may be gone.
        if let Ok(sample) = serde_json::from_str(&row.map_err(|error| error.to_string())?) {
            samples.push(sample);
        }
    }
    Ok(samples)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    Codex,
    Claude,
}

fn record_family(record: &UsageRecord) -> Family {
    if record.source_id == "codex" {
        Family::Codex
    } else {
        Family::Claude
    }
}

/// The transcripts a request can belong to: a tagged client's own, or either
/// when the request came in untagged.
fn sample_families(sample: &RelaySample) -> &'static [Family] {
    match sample.client_source.as_deref() {
        Some("codex") => &[Family::Codex],
        Some("claudeCode") => &[Family::Claude],
        None | Some("aggregate") => &[Family::Codex, Family::Claude],
        _ => &[],
    }
}

fn record_print(record: &UsageRecord) -> (u64, u64) {
    (
        record.input_tokens + record.cache_read_tokens + record.cache_write_tokens,
        record.output_tokens + record.reasoning_tokens,
    )
}

fn sample_print(sample: &RelaySample) -> (u64, u64) {
    let tokens = &sample.tokens;
    (
        tokens.input_tokens + tokens.cache_read_tokens + tokens.cache_write_tokens,
        tokens.output_tokens,
    )
}

/// How far a transcript turn logged at `timestamp` is from the request's end,
/// or `None` when it falls outside the window the request could have logged it in.
fn distance(sample: &RelaySample, timestamp: i64) -> Option<i64> {
    let end = sample.timestamp + (sample.duration_ms / 1000) as i64;
    (timestamp >= sample.timestamp - MATCH_LEAD_SECS && timestamp <= end + MATCH_LAG_SECS)
        .then(|| (timestamp - end).abs())
}

fn same_model(record: &UsageRecord, sample: &RelaySample) -> bool {
    [
        Some(&sample.model),
        sample.requested_model.as_ref(),
        sample.served_model.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|model| model.eq_ignore_ascii_case(&record.model))
}

/// For each transcript record, the router request that carried it.
fn pair(records: &[UsageRecord], samples: &[RelaySample]) -> Vec<Option<usize>> {
    let mut pairs = vec![None; records.len()];
    let mut taken = vec![false; samples.len()];
    let mut order = (0..records.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| records[*index].timestamp);

    // Exact token counts first: both sides report the usage the upstream sent.
    let mut by_print = HashMap::<(Family, u64, u64), Vec<usize>>::new();
    for (index, sample) in samples.iter().enumerate() {
        let print = sample_print(sample);
        if !sample.usage_reported || print == (0, 0) {
            continue;
        }
        for family in sample_families(sample) {
            by_print
                .entry((*family, print.0, print.1))
                .or_default()
                .push(index);
        }
    }
    for &index in &order {
        let record = &records[index];
        let (prompt, output) = record_print(record);
        let Some(candidates) = by_print.get(&(record_family(record), prompt, output)) else {
            continue;
        };
        // A Codex relay turn names its provider; nobody else's request is it.
        let provider = record.model.split_once("::").map(|(provider, _)| provider);
        let best = candidates
            .iter()
            .filter(|candidate| !taken[**candidate])
            .filter(|candidate| {
                provider.is_none_or(|provider| samples[**candidate].provider_id == provider)
            })
            .filter_map(|candidate| {
                distance(&samples[*candidate], record.timestamp).map(|gap| (gap, *candidate))
            })
            .min();
        if let Some((_, candidate)) = best {
            taken[candidate] = true;
            pairs[index] = Some(candidate);
        }
    }

    // Then routed turns whose usage the two sides counted differently, by time
    // alone: Codex names a relay turn's provider, and a Claude Code turn only
    // pairs with a request Claude Code sent for the same model.
    let mut by_time = (0..samples.len()).collect::<Vec<_>>();
    by_time.sort_by_key(|index| samples[*index].timestamp);
    let longest = samples
        .iter()
        .map(|sample| (sample.duration_ms / 1000) as i64)
        .max()
        .unwrap_or(0);
    for &index in &order {
        if pairs[index].is_some() {
            continue;
        }
        let record = &records[index];
        let family = record_family(record);
        let provider = record.model.split_once("::").map(|(provider, _)| provider);
        if family == Family::Codex && provider.is_none() {
            continue;
        }
        let low = record.timestamp - MATCH_LAG_SECS - longest;
        let high = record.timestamp + MATCH_LEAD_SECS;
        let from = by_time.partition_point(|candidate| samples[*candidate].timestamp < low);
        let best = by_time[from..]
            .iter()
            .take_while(|candidate| samples[**candidate].timestamp <= high)
            .filter(|candidate| !taken[**candidate])
            .filter(|candidate| {
                let sample = &samples[**candidate];
                match (family, provider) {
                    (Family::Codex, Some(provider)) => {
                        sample_families(sample).contains(&Family::Codex)
                            && sample.provider_id == provider
                    }
                    _ => {
                        sample.client_source.as_deref() == Some("claudeCode")
                            && same_model(record, sample)
                    }
                }
            })
            .filter_map(|candidate| {
                distance(&samples[*candidate], record.timestamp).map(|gap| (gap, *candidate))
            })
            .min();
        if let Some((_, candidate)) = best {
            taken[candidate] = true;
            pairs[index] = Some(candidate);
        }
    }
    pairs
}

/// Prices calls the router did not: official account calls and routed turns
/// the request log no longer has.
struct Pricer<'a> {
    catalog: &'a models_dev::Catalog,
    providers: HashMap<&'a str, &'a router::Provider>,
    known: HashMap<(Option<String>, String), Option<PricingSnapshot>>,
}

impl Pricer<'_> {
    fn price(
        &mut self,
        provider: Option<&str>,
        family: Family,
        model: &str,
    ) -> Option<PricingSnapshot> {
        let key = (provider.map(str::to_owned), model.to_owned());
        if let Some(known) = self.known.get(&key) {
            return known.clone();
        }
        let vendor = match family {
            Family::Codex => "openai",
            Family::Claude => "anthropic",
        };
        let price = provider
            .and_then(|id| self.providers.get(id))
            .map(|provider| router::pricing_for(provider, model, self.catalog))
            .filter(PricingSnapshot::is_known)
            .or_else(|| catalog_price(self.catalog, (provider.is_none()).then_some(vendor), model));
        self.known.insert(key, price.clone());
        price
    }
}

/// A catalog reference price; dated model ids also try their undated name.
fn catalog_price(
    catalog: &models_dev::Catalog,
    vendor: Option<&str>,
    model: &str,
) -> Option<PricingSnapshot> {
    let undated = model
        .rsplit_once('-')
        .filter(|(_, date)| date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|(base, _)| base);
    for id in std::iter::once(model).chain(undated) {
        let found = vendor
            .and_then(|vendor| catalog.scoped(&[vendor.to_owned()], id).into_iter().next())
            .or_else(|| catalog.global(id).into_iter().next());
        let Some(found) = found else { continue };
        let Some(cost) = found
            .model
            .cost
            .as_ref()
            .filter(|cost| cost.input.is_some() || cost.output.is_some())
        else {
            continue;
        };
        let complete = cost.input_audio.is_none()
            && cost.output_audio.is_none()
            && cost
                .context_over_200k
                .as_ref()
                .is_none_or(|rates| !rates.has_audio())
            && cost.tiers.iter().all(|tier| {
                !tier.rates.has_audio()
                    && tier.tier.condition_type == "context"
                    && tier.tier.size.is_some()
            });
        return Some(PricingSnapshot {
            input: cost.input,
            output: cost.output,
            cache_read: cost.cache_read,
            cache_write: cost.cache_write,
            reasoning: cost.reasoning,
            input_audio: cost.input_audio,
            output_audio: cost.output_audio,
            context_over_200k: cost.context_over_200k,
            tiers: cost.tiers.clone(),
            source: "catalogReference".into(),
            multiplier: None,
            revision: catalog.snapshot.revision.clone(),
            match_kind: "exact".into(),
            complete,
            currency: "USD".into(),
            source_ref: Some(found.model_ref()),
        });
    }
    None
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn normalize_client(source: Option<&str>) -> String {
    match source {
        Some(source) if KNOWN_CLIENTS.contains(&source) || source.starts_with("gateway:") => {
            source.to_owned()
        }
        _ => "unknown".into(),
    }
}

fn relay_entry(sample: &RelaySample, aggregates: &HashSet<&str>) -> UsageEntry {
    let aggregate = sample.client_source.as_deref() == Some("aggregate")
        || sample
            .requested_model
            .as_deref()
            .is_some_and(|model| aggregates.contains(model));
    let tokens = &sample.tokens;
    UsageEntry {
        timestamp: sample.timestamp,
        client: normalize_client(sample.client_source.as_deref()),
        channel: if aggregate {
            Channel::Aggregate
        } else {
            Channel::Relay
        },
        provider_id: non_empty(&sample.provider_id),
        provider_name: non_empty(&sample.provider_name),
        model: sample.model.clone(),
        requested_model: sample.requested_model.clone(),
        project: None,
        session_id: None,
        input_tokens: tokens.input_tokens,
        output_tokens: tokens.output_tokens,
        reasoning_tokens: tokens.reasoning_tokens,
        cache_read_tokens: tokens.cache_read_tokens,
        cache_write_tokens: tokens.cache_write_tokens,
        total_tokens: tokens.total_tokens,
        usage_reported: sample.usage_reported,
        cache_reported: sample.cache_reported,
        cost_usd: sample.cost_usd,
        pricing_status: if sample.pricing_status.is_empty() {
            "legacy".into()
        } else {
            sample.pricing_status.clone()
        },
        estimated: false,
        pricing_source: non_empty(&sample.pricing_source),
        multiplier: sample.multiplier,
        cache_savings_usd: sample.cache_savings_usd,
        request: Some(RequestFacts {
            status_code: sample.status_code,
            success: sample.success,
            duration_ms: sample.duration_ms,
            first_token_ms: sample.first_token_ms,
            served_model: sample.served_model.clone(),
            model_match_status: sample.model_match_status.clone(),
            credential_status: sample.credential_status.clone(),
            credential_fingerprint: sample.credential_fingerprint.clone(),
            matched: false,
        }),
    }
}

fn record_client(record: &UsageRecord) -> &'static str {
    match record_family(record) {
        Family::Codex => "codex",
        Family::Claude => "claudeCode",
    }
}

fn local_entry(
    record: &UsageRecord,
    pricer: &mut Pricer,
    aggregates: &HashSet<&str>,
    provider_names: &HashMap<String, String>,
) -> UsageEntry {
    // Codex names a relay model `provider::model`; anything else it ran is
    // either an aggregate or its own account's model.
    let (channel, provider_id, model) = match record.model.split_once("::") {
        Some((provider, model)) => (Channel::Relay, Some(provider.to_owned()), model.to_owned()),
        None if aggregates.contains(record.model.as_str()) => {
            (Channel::Aggregate, None, record.model.clone())
        }
        None => (Channel::Official, None, record.model.clone()),
    };
    let tokens = TokenCounts {
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens + record.reasoning_tokens,
        reasoning_tokens: record.reasoning_tokens,
        total_tokens: record.total_tokens,
        cache_read_tokens: record.cache_read_tokens,
        cache_write_tokens: record.cache_write_tokens,
    };
    let pricing = (channel != Channel::Aggregate)
        .then(|| pricer.price(provider_id.as_deref(), record_family(record), &model))
        .flatten();
    let priced = pricing
        .as_ref()
        .and_then(|pricing| relay_usage::estimate(&tokens, pricing));
    let (cost_usd, pricing_status) = match priced {
        Some((cost, true)) => (Some(cost), "priced"),
        Some((cost, false)) => (Some(cost), "partial"),
        None => (None, "unpriced"),
    };
    UsageEntry {
        timestamp: record.timestamp,
        client: record_client(record).into(),
        channel,
        provider_name: provider_id.as_ref().map(|id| {
            provider_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.clone())
        }),
        provider_id,
        model,
        requested_model: None,
        project: non_empty(&record.project),
        session_id: non_empty(&record.session_id),
        input_tokens: tokens.input_tokens,
        output_tokens: tokens.output_tokens,
        reasoning_tokens: tokens.reasoning_tokens,
        cache_read_tokens: tokens.cache_read_tokens,
        cache_write_tokens: tokens.cache_write_tokens,
        total_tokens: tokens.total_tokens,
        usage_reported: true,
        cache_reported: true,
        cost_usd,
        pricing_status: pricing_status.into(),
        estimated: cost_usd.is_some(),
        pricing_source: pricing.as_ref().map(|pricing| pricing.source.clone()),
        multiplier: pricing.as_ref().and_then(|pricing| pricing.multiplier),
        cache_savings_usd: pricing
            .as_ref()
            .and_then(|pricing| relay_usage::cache_savings(&tokens, pricing)),
        request: None,
    }
}

fn join(
    collected: &Collected,
    store: Option<&router::Store>,
    catalog: &models_dev::Catalog,
) -> Vec<UsageEntry> {
    let aggregates = store
        .map(|store| {
            store
                .aggregates
                .iter()
                .map(|aggregate| aggregate.public_model_id.as_str())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut provider_names = HashMap::new();
    for sample in &collected.samples {
        provider_names.insert(sample.provider_id.clone(), sample.provider_name.clone());
    }
    for provider in store
        .map(|store| store.providers.as_slice())
        .unwrap_or_default()
    {
        provider_names.insert(provider.id.clone(), provider.name.clone());
    }
    let mut pricer = Pricer {
        catalog,
        providers: store
            .map(|store| {
                store
                    .providers
                    .iter()
                    .map(|provider| (provider.id.as_str(), provider))
                    .collect()
            })
            .unwrap_or_default(),
        known: HashMap::new(),
    };
    let pairs = pair(&collected.records, &collected.samples);
    let mut taken = vec![false; collected.samples.len()];
    let mut entries = Vec::with_capacity(collected.records.len() + collected.samples.len());
    for (record, pair) in collected.records.iter().zip(pairs) {
        entries.push(match pair {
            Some(index) => {
                taken[index] = true;
                // The router's record bills the call; the transcript knows
                // which client, session and project it belonged to.
                let mut entry = relay_entry(&collected.samples[index], &aggregates);
                entry.client = record_client(record).into();
                entry.project = non_empty(&record.project);
                entry.session_id = non_empty(&record.session_id);
                if let Some(request) = entry.request.as_mut() {
                    request.matched = true;
                }
                entry
            }
            None => local_entry(record, &mut pricer, &aggregates, &provider_names),
        });
    }
    for (sample, taken) in collected.samples.iter().zip(taken) {
        if !taken {
            entries.push(relay_entry(sample, &aggregates));
        }
    }
    entries
}

fn local_midnight(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map(|midnight| midnight.timestamp())
        .unwrap_or_default()
}

fn local_time(timestamp: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(timestamp, 0).map(|time| time.with_timezone(&Local))
}

/// Rankings and chart series are built by key while the entries stream past.
#[derive(Default)]
struct Grouping {
    groups: HashMap<String, UsageGroup>,
}

impl Grouping {
    fn add(&mut self, key: String, entry: &UsageEntry, make: impl FnOnce() -> UsageGroup) {
        self.groups
            .entry(key)
            .or_insert_with(make)
            .totals
            .add(entry);
    }

    fn ranked(self, limit: usize) -> Vec<UsageGroup> {
        let mut groups = self.groups.into_values().collect::<Vec<_>>();
        groups.sort_by_key(|group| {
            (
                Reverse(group.totals.new_tokens),
                Reverse(group.totals.cost_micros),
                group.key.clone(),
            )
        });
        groups.truncate(limit);
        groups
    }
}

fn group(key: &str, label: String) -> UsageGroup {
    UsageGroup {
        key: key.to_owned(),
        label,
        detail: None,
        client: None,
        channel: None,
        provider_id: None,
        totals: UsageTotals::default(),
    }
}

fn channel_key(entry: &UsageEntry) -> String {
    match entry.channel {
        Channel::Official => "official".into(),
        Channel::Relay => format!("relay:{}", entry.provider_id.as_deref().unwrap_or("")),
        Channel::Aggregate => format!(
            "aggregate:{}",
            entry.requested_model.as_deref().unwrap_or(&entry.model)
        ),
    }
}

fn project_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// The keys a chart stacks: the largest few, the rest folded into `other`.
fn top_keys(groups: &[UsageGroup]) -> HashSet<String> {
    groups
        .iter()
        .take(PART_LIMIT)
        .map(|group| group.key.clone())
        .collect()
}

fn add_part(parts: &mut Vec<UsagePart>, key: &str, entry: &UsageEntry) {
    let index = match parts.iter().position(|part| part.key == key) {
        Some(index) => index,
        None => {
            parts.push(UsagePart {
                key: key.to_owned(),
                new_tokens: 0,
                cost_micros: 0,
            });
            parts.len() - 1
        }
    };
    let part = &mut parts[index];
    part.new_tokens = part.new_tokens.saturating_add(new_tokens(entry));
    if entry.pricing_status != "unpriced" {
        part.cost_micros = part
            .cost_micros
            .saturating_add(entry.cost_usd.map(relay_usage::to_micros).unwrap_or(0));
    }
}

fn summarize(
    collected: &Collected,
    entries: Vec<UsageEntry>,
    range: UsageRange,
    now: DateTime<Local>,
) -> UsagePayload {
    let today = now.date_naive();
    let days = range.days();
    let first_day = today - ChronoDuration::days(days - 1);
    let start = local_midnight(first_day);
    let span = now.timestamp() - start;
    let previous_start = local_midnight(first_day - ChronoDuration::days(days));
    let previous_end = previous_start + span;
    let today_start = local_midnight(today);

    let mut buckets = if range == UsageRange::Today {
        (0..24)
            .map(|hour| UsageBucket {
                key: format!("{:02}", hour),
                start: start + hour * 3600,
                totals: UsageTotals::default(),
                by_client: Vec::new(),
                by_channel: Vec::new(),
                by_model: Vec::new(),
            })
            .collect::<Vec<_>>()
    } else {
        (0..days)
            .map(|offset| {
                let date = first_day + ChronoDuration::days(offset);
                UsageBucket {
                    key: date.format("%Y-%m-%d").to_string(),
                    start: local_midnight(date),
                    totals: UsageTotals::default(),
                    by_client: Vec::new(),
                    by_channel: Vec::new(),
                    by_model: Vec::new(),
                }
            })
            .collect()
    };

    let mut today_totals = UsageTotals::default();
    let mut current = UsageTotals::default();
    let mut previous = UsageTotals::default();
    let mut all_time = UsageTotals::default();
    let mut clients = Grouping::default();
    let mut channels = Grouping::default();
    let mut models = Grouping::default();
    let mut projects = Grouping::default();
    let mut sessions = Grouping::default();
    let mut in_range = Vec::new();
    let mut matched_count = 0;
    for entry in entries {
        all_time.add(&entry);
        if entry
            .request
            .as_ref()
            .is_some_and(|request| request.matched)
        {
            matched_count += 1;
        }
        if entry.timestamp >= today_start {
            today_totals.add(&entry);
        }
        if entry.timestamp >= previous_start && entry.timestamp < previous_end {
            previous.add(&entry);
        }
        if entry.timestamp < start {
            continue;
        }
        current.add(&entry);
        clients.add(entry.client.clone(), &entry, || {
            let mut row = group(&entry.client, entry.client.clone());
            row.client = Some(entry.client.clone());
            row
        });
        let channel = channel_key(&entry);
        channels.add(channel.clone(), &entry, || {
            let label = match entry.channel {
                Channel::Official => String::new(),
                Channel::Relay => entry
                    .provider_name
                    .clone()
                    .or_else(|| entry.provider_id.clone())
                    .unwrap_or_default(),
                Channel::Aggregate => entry
                    .requested_model
                    .clone()
                    .unwrap_or_else(|| entry.model.clone()),
            };
            let mut row = group(&channel, label);
            row.channel = Some(entry.channel);
            row.provider_id = entry
                .provider_id
                .clone()
                .filter(|_| entry.channel == Channel::Relay);
            row
        });
        let model = entry.model.to_lowercase();
        models.add(model.clone(), &entry, || group(&model, entry.model.clone()));
        if let Some(project) = &entry.project {
            projects.add(project.clone(), &entry, || {
                let mut row = group(project, project_name(project));
                row.detail = Some(project.clone());
                row
            });
        }
        if let Some(session) = &entry.session_id {
            let key = format!("{}:{session}", entry.client);
            sessions.add(key.clone(), &entry, || {
                let mut row = group(&key, session.clone());
                row.detail = entry.project.as_deref().map(project_name);
                row.client = Some(entry.client.clone());
                row
            });
        }
        in_range.push(entry);
    }

    let by_client = clients.ranked(usize::MAX);
    let by_channel = channels.ranked(usize::MAX);
    let mut by_model = models.ranked(usize::MAX);
    // The chart stacks the same few series the rankings lead with.
    let client_keys = top_keys(&by_client);
    let model_keys = top_keys(&by_model);
    by_model.truncate(RANK_LIMIT);
    for entry in &in_range {
        let index = if range == UsageRange::Today {
            local_time(entry.timestamp).map(|time| time.hour() as usize)
        } else {
            local_time(entry.timestamp)
                .and_then(|time| usize::try_from((time.date_naive() - first_day).num_days()).ok())
        };
        let Some(bucket) = index.and_then(|index| buckets.get_mut(index)) else {
            continue;
        };
        bucket.totals.add(entry);
        let client = if client_keys.contains(&entry.client) {
            entry.client.as_str()
        } else {
            "other"
        };
        add_part(&mut bucket.by_client, client, entry);
        add_part(&mut bucket.by_channel, entry.channel.as_str(), entry);
        let model = entry.model.to_lowercase();
        add_part(
            &mut bucket.by_model,
            if model_keys.contains(&model) {
                &model
            } else {
                "other"
            },
            entry,
        );
    }

    // Each model's channels, by provider name, so a merged row says where it ran.
    let mut model_channels = HashMap::<String, Vec<String>>::new();
    for entry in &in_range {
        let label = match entry.channel {
            Channel::Official => "official".to_owned(),
            _ => entry
                .provider_name
                .clone()
                .unwrap_or_else(|| channel_key(entry)),
        };
        let channels = model_channels
            .entry(entry.model.to_lowercase())
            .or_default();
        if !channels.contains(&label) {
            channels.push(label);
        }
    }
    for row in &mut by_model {
        row.detail = model_channels
            .remove(&row.key)
            .map(|channels| channels.join(" · "));
    }

    let mut recent = in_range;
    recent.sort_by_key(|entry| Reverse(entry.timestamp));
    recent.truncate(RECENT_LIMIT);
    let mut warnings = collected.warnings.clone();
    warnings.dedup();
    let warning_count = warnings.len() as u64;
    warnings.truncate(8);
    UsagePayload {
        generated_at: now.timestamp(),
        range,
        start,
        previous_start,
        today: today_totals,
        current,
        previous,
        all_time,
        buckets,
        by_client,
        by_channel,
        by_model,
        by_project: projects.ranked(RANK_LIMIT),
        by_session: sessions.ranked(RANK_LIMIT),
        recent,
        sources: UsageSources {
            transcripts: collected.sources.clone(),
            relay_since: collected.samples.first().map(|sample| sample.timestamp),
            relay_count: collected.samples.len() as u64,
            matched_count,
        },
        warning_count,
        warnings,
    }
}

#[cfg(test)]
#[path = "usage_ledger_tests.rs"]
mod tests;
