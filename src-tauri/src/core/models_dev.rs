//! models.dev catalog: one typed, versioned snapshot for model capabilities
//! and pricing. The runtime cache takes priority; the committed snapshot keeps
//! the app useful offline. Nothing here performs network I/O except `refresh`,
//! which the command/background layer calls explicitly.
use super::models::ApiProxyConfigPayload;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_SOURCE_URL: &str = "https://models.dev/api.json";
pub const CACHE_FILE: &str = "models-dev.json";
/// Refuse unrealistically large downloads; the observed document is ~5 MB.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// After a failed check, wait this long before trying again.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const EMBEDDED: &str = include_str!("../../assets/models-dev.snapshot.json");
const MAX_PROVIDERS: usize = 5_000;
const MAX_MODELS_PER_PROVIDER: usize = 10_000;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    pub schema_version: u32,
    pub source_url: String,
    pub fetched_at: i64,
    pub revision: String,
    pub providers: Vec<Provider>,
    /// Only present in the runtime cache; the embedded file omits them.
    pub last_checked_at: Option<i64>,
    pub etag: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub npm: Option<String>,
    pub api: Option<String>,
    pub env: Vec<String>,
    pub doc: Option<String>,
    pub models: Vec<Model>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub family: Option<String>,
    /// `Some(false)` is an explicit "no reasoning" declaration.
    pub reasoning: Option<bool>,
    pub reasoning_options: Vec<ReasoningOption>,
    /// `None` means an older snapshot cannot distinguish absent from [].
    pub reasoning_options_declared: Option<bool>,
    /// Response-format hint only; never selects a protocol or endpoint.
    #[serde(deserialize_with = "deserialize_interleaved")]
    pub interleaved: Option<Interleaved>,
    pub limit: Limit,
    pub cost: Option<Cost>,
    pub modalities: Option<Modalities>,
    pub attachment: Option<bool>,
    pub tool_call: Option<bool>,
    pub structured_output: Option<bool>,
    pub temperature: Option<bool>,
    pub open_weights: Option<bool>,
    pub knowledge: Option<String>,
    pub release_date: Option<String>,
    pub last_updated: Option<String>,
    pub canonical_model_id: Option<String>,
    pub status: Option<String>,
    /// Diagnostics-only hints; never used for protocol selection.
    pub provider_hint: Option<Value>,
    pub experimental: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Interleaved {
    Enabled(bool),
    Field { field: String },
}

fn deserialize_interleaved<'de, D>(deserializer: D) -> Result<Option<Interleaved>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Bool(enabled) => Some(Interleaved::Enabled(enabled)),
        Value::Object(object) => object
            .get("field")
            .and_then(Value::as_str)
            .filter(|field| {
                !field.is_empty() && field.len() <= 128 && !field.chars().any(char::is_control)
            })
            .map(|field| Interleaved::Field {
                field: field.into(),
            }),
        _ => None,
    })
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limit {
    #[serde(deserialize_with = "optional_positive_u64", default)]
    pub context: Option<u64>,
    #[serde(deserialize_with = "optional_positive_u64", default)]
    pub input: Option<u64>,
    #[serde(deserialize_with = "optional_positive_u64", default)]
    pub output: Option<u64>,
}

/// Real catalog data uses `-1` for "unset" in numeric option fields; a
/// non-positive value must read as missing instead of failing the document.
fn optional_positive_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<i64>::deserialize(deserializer)?;
    Ok(value.filter(|value| *value > 0).map(|value| value as u64))
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cost {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub reasoning: Option<f64>,
    pub input_audio: Option<f64>,
    pub output_audio: Option<f64>,
    pub tiers: Vec<CostTier>,
    pub context_over_200k: Option<CostRates>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CostRates {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub reasoning: Option<f64>,
    pub input_audio: Option<f64>,
    pub output_audio: Option<f64>,
}

impl CostRates {
    pub fn has_audio(&self) -> bool {
        self.input_audio.is_some() || self.output_audio.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostTier {
    #[serde(flatten)]
    pub rates: CostRates,
    pub tier: TierCondition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierCondition {
    #[serde(rename = "type")]
    pub condition_type: String,
    #[serde(default, deserialize_with = "optional_positive_u64")]
    pub size: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningOption {
    Effort {
        values: Vec<String>,
    },
    Toggle,
    BudgetTokens {
        #[serde(default, deserialize_with = "optional_positive_u64")]
        min: Option<u64>,
        #[serde(default, deserialize_with = "optional_positive_u64")]
        max: Option<u64>,
    },
}

impl ReasoningOption {
    pub fn efforts(&self) -> &[String] {
        match self {
            Self::Effort { values } => values,
            Self::Toggle | Self::BudgetTokens { .. } => &[],
        }
    }
}

/// One catalog record located by the in-memory indices.
#[derive(Clone, Copy)]
pub struct Match<'a> {
    pub provider: &'a Provider,
    pub model: &'a Model,
}

impl Match<'_> {
    pub fn model_ref(&self) -> ModelRef {
        ModelRef {
            provider_id: self.provider.id.clone(),
            model_id: self.model.id.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Runtime,
    Embedded,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Embedded => "embedded",
        }
    }
}

pub struct Catalog {
    pub snapshot: Snapshot,
    pub origin: Origin,
    pub error: Option<String>,
    index: Index,
}

#[derive(Default)]
struct Index {
    /// provider id (lower) -> position in `snapshot.providers`
    providers: HashMap<String, usize>,
    /// provider id (lower) -> model id (lower) -> positions in that provider
    provider_models: HashMap<String, HashMap<String, Vec<usize>>>,
    /// Scoped canonical identities and their unqualified model names. Kept
    /// separate from exact IDs so aliases never masquerade as exact matches.
    provider_canonical: HashMap<String, HashMap<String, Vec<usize>>>,
    global: HashMap<String, Vec<Entry>>,
    api_hosts: HashMap<String, Vec<String>>,
}

#[derive(Clone, Copy)]
struct Entry {
    provider: usize,
    model: usize,
}

fn lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

impl Catalog {
    pub fn parse(bytes: &[u8], origin: Origin) -> Result<Self, String> {
        let snapshot: Snapshot =
            serde_json::from_slice(bytes).map_err(|error| format!("invalid_snapshot: {error}"))?;
        Self::from_snapshot(snapshot, origin)
    }

    /// The public API is a provider-keyed object with snake_case model fields;
    /// the cache and embedded assets use our versioned camelCase snapshot.
    fn from_api(bytes: &[u8], source_url: &str, fetched_at: i64) -> Result<Self, String> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|error| format!("invalid_api: {error}"))?;
        let Value::Object(providers) = value else {
            return Err("invalid_api_providers".into());
        };
        if providers.is_empty() || providers.len() > MAX_PROVIDERS {
            return Err("invalid_provider_count".into());
        }
        let mut transformed = Vec::new();
        for (_, provider) in providers {
            let Value::Object(mut provider) = provider else {
                return Err("invalid_api_provider".into());
            };
            let Some(Value::Object(models)) = provider.remove("models") else {
                return Err("invalid_api_models".into());
            };
            if models.len() > MAX_MODELS_PER_PROVIDER {
                return Err("invalid_model_count".into());
            }
            let mut entries = Vec::new();
            for (_, model) in models {
                let Value::Object(mut model) = model else {
                    return Err("invalid_api_model".into());
                };
                let reasoning_declared = model.contains_key("reasoning_options");
                rename_fields(
                    &mut model,
                    &[
                        ("reasoning_options", "reasoningOptions"),
                        ("tool_call", "toolCall"),
                        ("structured_output", "structuredOutput"),
                        ("open_weights", "openWeights"),
                        ("release_date", "releaseDate"),
                        ("last_updated", "lastUpdated"),
                        ("canonical_model_id", "canonicalModelId"),
                        ("provider", "providerHint"),
                    ],
                );
                model.insert(
                    "reasoningOptionsDeclared".into(),
                    Value::Bool(reasoning_declared),
                );
                if let Some(Value::Array(options)) = model.get_mut("reasoningOptions") {
                    // Unknown option variants are future diagnostics, not a
                    // reason to discard the whole catalog.
                    options.retain_mut(|option| match option["type"].as_str() {
                        Some("effort") => {
                            let Some(Value::Array(values)) = option.get_mut("values") else {
                                return false;
                            };
                            values.retain(|value| {
                                value.as_str().is_some_and(|value| !value.is_empty())
                            });
                            !values.is_empty()
                        }
                        Some("toggle" | "budget_tokens") => true,
                        _ => false,
                    });
                }
                if let Some(Value::Object(cost)) = model.get_mut("cost") {
                    rename_rates(cost);
                    rename_fields(cost, &[("context_over_200k", "contextOver200k")]);
                    if let Some(Value::Object(over)) = cost.get_mut("contextOver200k") {
                        rename_rates(over);
                    }
                    if let Some(Value::Array(tiers)) = cost.get_mut("tiers") {
                        for tier in tiers {
                            if let Value::Object(tier) = tier {
                                rename_rates(tier);
                            }
                        }
                    }
                }
                entries.push(Value::Object(model));
            }
            provider.insert("models".into(), Value::Array(entries));
            transformed.push(
                serde_json::from_value::<Provider>(Value::Object(provider))
                    .map_err(|error| format!("invalid_api_provider: {error}"))?,
            );
        }
        for provider in &mut transformed {
            provider.models.sort_by(|a, b| a.id.cmp(&b.id));
        }
        transformed.sort_by(|a, b| a.id.cmp(&b.id));
        Self::from_snapshot(
            Snapshot {
                schema_version: SCHEMA_VERSION,
                source_url: source_url.into(),
                fetched_at,
                // Both refresh paths hash the source bytes; timestamps do not
                // invent new revisions for identical catalog content.
                revision: format!("{:x}", Sha256::digest(bytes)),
                providers: transformed,
                ..Default::default()
            },
            Origin::Runtime,
        )
    }

    pub fn from_snapshot(mut snapshot: Snapshot, origin: Origin) -> Result<Self, String> {
        if snapshot.schema_version != SCHEMA_VERSION {
            return Err("unsupported_schema_version".into());
        }
        if snapshot.providers.is_empty() || snapshot.providers.len() > MAX_PROVIDERS {
            return Err("invalid_provider_count".into());
        }
        let mut valid = 0usize;
        let mut rejected = 0usize;
        snapshot.providers.retain_mut(|provider| {
            if provider.id.trim().is_empty() || provider.id.len() > 128 {
                return false;
            }
            if provider.models.len() > MAX_MODELS_PER_PROVIDER {
                return false;
            }
            provider.models.retain(|model| {
                let keep = !model.id.trim().is_empty()
                    && model.id.len() <= 256
                    && !model.id.chars().any(char::is_control)
                    && valid_model(model);
                if keep {
                    valid += 1;
                } else {
                    rejected += 1;
                }
                keep
            });
            !provider.models.is_empty()
        });
        if snapshot.providers.is_empty() {
            return Err("no_valid_providers".into());
        }
        if rejected > valid {
            return Err("too_many_invalid_models".into());
        }
        let index = Index::build(&snapshot.providers);
        Ok(Self {
            snapshot,
            origin,
            error: None,
            index,
        })
    }

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.index
            .providers
            .get(&lower(id))
            .and_then(|index| self.snapshot.providers.get(*index))
    }

    /// Endpoint hosts are parsed once per catalog revision, not per request.
    pub fn providers_for_host(&self, host: &str) -> &[String] {
        self.index.api_hosts.get(&lower(host)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Case-sensitive lookup inside one provider. Used for explicit references.
    pub fn exact_in(&self, provider_id: &str, model_id: &str) -> Option<Match<'_>> {
        let provider = self.provider(provider_id)?;
        provider
            .models
            .iter()
            .find(|model| model.id == model_id)
            .map(|model| Match { provider, model })
    }

    /// Case-insensitive matches for one model ID inside the given providers.
    /// Case-conflicting entries are all returned; callers must treat more than
    /// one distinct model as ambiguous. Indexed, so the request hot path never
    /// scans provider model lists.
    pub fn scoped(&self, provider_ids: &[String], model_id: &str) -> Vec<Match<'_>> {
        let model_id = lower(model_id);
        let mut matches = Vec::new();
        for provider_id in provider_ids {
            let provider_id = lower(provider_id);
            let Some(provider_index) = self.index.providers.get(&provider_id).copied() else {
                continue;
            };
            let Some(indices) = self
                .index
                .provider_models
                .get(&provider_id)
                .and_then(|by_model| by_model.get(&model_id))
            else {
                continue;
            };
            let Some(provider) = self.snapshot.providers.get(provider_index) else {
                continue;
            };
            for model_index in indices {
                if let Some(model) = provider.models.get(*model_index) {
                    matches.push(Match { provider, model });
                }
            }
        }
        matches
    }

    /// Match a publisher-declared canonical identity within the selected
    /// catalogs. Deprecated aliases only participate if that provider has no
    /// active record for the identity; distinct active records remain visible.
    pub fn canonical_scoped(
        &self,
        provider_ids: &[impl AsRef<str>],
        model_id: &str,
    ) -> Vec<Match<'_>> {
        let key = lower(model_id);
        let mut matches = Vec::new();
        for provider_id in provider_ids {
            let provider_id = lower(provider_id.as_ref());
            let Some(provider) = self.provider(&provider_id) else {
                continue;
            };
            let Some(indices) = self
                .index
                .provider_canonical
                .get(&provider_id)
                .and_then(|models| models.get(&key))
            else {
                continue;
            };
            let entries = indices
                .iter()
                .filter_map(|index| provider.models.get(*index))
                .collect::<Vec<_>>();
            let deprecated = |model: &Model| {
                model
                    .status
                    .as_deref()
                    .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
            };
            let has_active = entries.iter().any(|model| !deprecated(model));
            matches.extend(
                entries
                    .into_iter()
                    .filter(|model| !has_active || !deprecated(model))
                    .map(|model| Match { provider, model }),
            );
        }
        matches
    }

    /// Case-insensitive matches across the whole catalog, for generic
    /// (unscoped) reference resolution.
    pub fn global(&self, model_id: &str) -> Vec<Match<'_>> {
        let model_id = lower(model_id);
        let Some(entries) = self.index.global.get(&model_id) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| {
                let provider = self.snapshot.providers.get(entry.provider)?;
                let model = provider.models.get(entry.model)?;
                Some(Match { provider, model })
            })
            .collect()
    }

    pub fn model_count(&self) -> usize {
        self.index.global.values().map(Vec::len).sum()
    }

    pub fn is_stale(&self, now: i64) -> bool {
        self.snapshot.fetched_at <= 0
            || now.saturating_sub(self.snapshot.fetched_at) as u64 > TTL.as_secs()
    }

    pub fn status(&self, now: i64) -> CatalogStatus {
        CatalogStatus {
            schema_version: self.snapshot.schema_version,
            source_url: self.snapshot.source_url.clone(),
            fetched_at: (self.snapshot.fetched_at > 0).then_some(self.snapshot.fetched_at),
            last_checked_at: self.snapshot.last_checked_at,
            stale: self.is_stale(now),
            origin: self.origin.as_str().into(),
            revision: self.snapshot.revision.clone(),
            providers: self.snapshot.providers.len(),
            models: self.model_count(),
            error: self.error.clone(),
        }
    }

    fn with_error(self, error: Option<String>) -> Self {
        Self { error, ..self }
    }
}

fn rename_fields(object: &mut serde_json::Map<String, Value>, fields: &[(&str, &str)]) {
    for (from, to) in fields {
        if let Some(value) = object.remove(*from) {
            object.insert((*to).into(), value);
        }
    }
}

fn rename_rates(object: &mut serde_json::Map<String, Value>) {
    rename_fields(
        object,
        &[
            ("cache_read", "cacheRead"),
            ("cache_write", "cacheWrite"),
            ("input_audio", "inputAudio"),
            ("output_audio", "outputAudio"),
        ],
    );
}

fn valid_model(model: &Model) -> bool {
    // A record with no usable metadata at all is a shape change, not a model.
    if model.limit.context.is_none()
        && model.limit.input.is_none()
        && model.limit.output.is_none()
        && model.cost.is_none()
        && model.reasoning.is_none()
    {
        return false;
    }
    let valid_limit =
        |value: Option<u64>| value.is_none_or(|value| value > 0 && value <= 100_000_000);
    if !valid_limit(model.limit.context)
        || !valid_limit(model.limit.input)
        || !valid_limit(model.limit.output)
    {
        return false;
    }
    let valid_rate = |value: Option<f64>| {
        value.is_none_or(|value| value.is_finite() && value >= 0.0 && value < 1_000_000.0)
    };
    let valid_rates = |rates: &CostRates| {
        [
            rates.input,
            rates.output,
            rates.cache_read,
            rates.cache_write,
            rates.reasoning,
            rates.input_audio,
            rates.output_audio,
        ]
        .into_iter()
        .all(valid_rate)
    };
    if model.cost.as_ref().is_some_and(|cost| {
        !valid_rate(cost.input)
            || !valid_rate(cost.output)
            || !valid_rate(cost.cache_read)
            || !valid_rate(cost.cache_write)
            || !valid_rate(cost.reasoning)
            || !valid_rate(cost.input_audio)
            || !valid_rate(cost.output_audio)
            || cost
                .context_over_200k
                .as_ref()
                .is_some_and(|rates| !valid_rates(rates))
            || cost.tiers.iter().any(|tier| !valid_rates(&tier.rates))
    }) {
        return false;
    }
    model.reasoning_options.iter().all(|option| match option {
        ReasoningOption::Effort { values } => {
            !values.is_empty()
                && values.len() <= 16
                && values
                    .iter()
                    .all(|value| !value.is_empty() && value.len() <= 32)
        }
        ReasoningOption::Toggle => true,
        ReasoningOption::BudgetTokens { min, max } => {
            min.is_none_or(|min| min > 0) && max.is_none_or(|max| max > 0)
        }
    })
}

impl Index {
    fn build(providers: &[Provider]) -> Self {
        let mut index = Index::default();
        for (provider_index, provider) in providers.iter().enumerate() {
            if let Some(host) = provider.api.as_deref()
                .and_then(|api| reqwest::Url::parse(api).ok())
                .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            {
                index.api_hosts.entry(host).or_default().push(provider.id.clone());
            }
            let provider_key = lower(&provider.id);
            index.providers.insert(provider_key.clone(), provider_index);
            let by_model = index.provider_models.entry(provider_key).or_default();
            for (model_index, model) in provider.models.iter().enumerate() {
                by_model
                    .entry(lower(&model.id))
                    .or_default()
                    .push(model_index);
                index
                    .global
                    .entry(lower(&model.id))
                    .or_default()
                    .push(Entry {
                        provider: provider_index,
                        model: model_index,
                    });
                if let Some(canonical) = model
                    .canonical_model_id
                    .as_deref()
                    .filter(|id| id.len() <= 512 && !id.chars().any(char::is_control))
                {
                    if let Some((owner, name)) = canonical
                        .split_once('/')
                        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty())
                    {
                        let canonical_models = index
                            .provider_canonical
                            .entry(lower(&provider.id))
                            .or_default();
                        for key in [format!("{owner}/{name}"), name.to_owned()] {
                            canonical_models
                                .entry(lower(&key))
                                .or_default()
                                .push(model_index);
                        }
                    }
                }
            }
        }
        index
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub schema_version: u32,
    pub source_url: String,
    pub fetched_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub stale: bool,
    pub origin: String,
    pub revision: String,
    pub providers: usize,
    pub models: usize,
    pub error: Option<String>,
}

struct State {
    catalog: Arc<Catalog>,
    checked: Instant,
}

fn states() -> &'static Mutex<HashMap<PathBuf, State>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, State>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn evict_oldest(states: &mut HashMap<PathBuf, State>) {
    // Preserve fresh refresh results and retry status when another directory
    // is loaded. HashMap iteration order must not choose the victim.
    let oldest = states
        .iter()
        .min_by_key(|(_, state)| state.checked)
        .map(|(path, _)| path.clone());
    if let Some(path) = oldest {
        states.remove(&path);
    }
}

fn refresh_guard() -> &'static Mutex<()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(()))
}

fn cache_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(CACHE_FILE)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn bounded_read(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

fn embedded() -> Arc<Catalog> {
    static EMBEDDED_CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    EMBEDDED_CATALOG
        .get_or_init(
            || match Catalog::parse(EMBEDDED.as_bytes(), Origin::Embedded) {
                Ok(catalog) => Arc::new(catalog),
                Err(error) => {
                    log::error!("embedded models.dev snapshot rejected: {error}");
                    Arc::new(Catalog {
                        snapshot: Snapshot {
                            schema_version: SCHEMA_VERSION,
                            source_url: DEFAULT_SOURCE_URL.into(),
                            ..Default::default()
                        },
                        origin: Origin::Embedded,
                        error: Some(error),
                        index: Index::default(),
                    })
                }
            },
        )
        .clone()
}

/// Read the catalog for `app_data_dir` without network access. The runtime
/// cache wins when it is whole and valid; otherwise the committed snapshot is
/// used for the lifetime of the process. Never merges the two.
pub fn load(app_data_dir: &Path) -> Arc<Catalog> {
    let mut states = states().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(state) = states.get(app_data_dir) {
        return state.catalog.clone();
    }
    let loaded = match bounded_read(&cache_path(app_data_dir)) {
        Some(bytes) => Catalog::parse(&bytes, Origin::Runtime),
        None => Err("cache_unreadable".to_owned()),
    };
    let catalog = match loaded {
        Ok(catalog) => Arc::new(catalog),
        Err(reason) => {
            log::debug!("models.dev runtime cache not used: {reason}");
            embedded()
        }
    };
    if states.len() >= 32 && !states.contains_key(app_data_dir) {
        evict_oldest(&mut states);
    }
    states.insert(
        app_data_dir.to_path_buf(),
        State {
            catalog: catalog.clone(),
            checked: Instant::now(),
        },
    );
    catalog
}

/// Whether a background check is due: stale data, or a previous failure whose
/// retry interval has elapsed. A fresh success is never re-checked early.
pub fn should_refresh(app_data_dir: &Path) -> bool {
    let catalog = load(app_data_dir);
    if catalog.error.is_some() {
        let states = states().lock().unwrap_or_else(|error| error.into_inner());
        return states
            .get(app_data_dir)
            .is_some_and(|state| state.checked.elapsed() >= RETRY_INTERVAL);
    }
    catalog.is_stale(now())
}

fn store(app_data_dir: &Path, catalog: Catalog) {
    let mut states = states().lock().unwrap_or_else(|error| error.into_inner());
    states.insert(
        app_data_dir.to_path_buf(),
        State {
            catalog: Arc::new(catalog),
            checked: Instant::now(),
        },
    );
}

fn record_failure(app_data_dir: &Path, message: &str) {
    let current = load(app_data_dir);
    let mut snapshot = current.snapshot.clone();
    snapshot.last_checked_at = Some(now());
    if let Ok(catalog) = Catalog::from_snapshot(snapshot, current.origin) {
        store(app_data_dir, catalog.with_error(Some(message.to_owned())));
    }
}

fn write_cache(app_data_dir: &Path, snapshot: &Snapshot) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;
    super::storage::write_atomic(&cache_path(app_data_dir), &bytes)
        .map_err(|error| error.to_string())
}

/// Download and validate a fresh snapshot, then replace the runtime cache
/// atomically. Any failure keeps the previously valid catalog active.
pub fn refresh(
    app_data_dir: &Path,
    proxy: &ApiProxyConfigPayload,
) -> Result<CatalogStatus, String> {
    let _guard = refresh_guard()
        .lock()
        .map_err(|_| "refresh_busy".to_owned())?;
    let current = load(app_data_dir);
    match download(&current, proxy) {
        Ok(Download::NotModified) => {
            let mut snapshot = current.snapshot.clone();
            snapshot.last_checked_at = Some(now());
            let _ = write_cache(app_data_dir, &snapshot);
            match Catalog::from_snapshot(snapshot, current.origin) {
                Ok(catalog) => store(app_data_dir, catalog),
                Err(error) => record_failure(app_data_dir, &error),
            }
            Ok(load(app_data_dir).status(now()))
        }
        Ok(Download::Fresh(snapshot)) => {
            let mut snapshot = snapshot;
            snapshot.last_checked_at = Some(now());
            let catalog = Catalog::from_snapshot(snapshot, Origin::Runtime)?;
            write_cache(app_data_dir, &catalog.snapshot)?;
            let status = catalog.status(now());
            store(app_data_dir, catalog);
            Ok(status)
        }
        Err(error) => {
            record_failure(app_data_dir, &error);
            Err(error)
        }
    }
}

enum Download {
    Fresh(Snapshot),
    NotModified,
}

fn download(catalog: &Catalog, proxy: &ApiProxyConfigPayload) -> Result<Download, String> {
    let client = super::api_client::configure_blocking_client(
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60)),
        proxy,
    )
    .map_err(|error| error.to_string())?
    .build()
    .map_err(|error| error.to_string())?;
    let source = if catalog.snapshot.source_url.is_empty() {
        DEFAULT_SOURCE_URL
    } else {
        catalog.snapshot.source_url.as_str()
    };
    let mut request = client.get(source);
    if let Some(etag) = catalog.snapshot.etag.as_deref() {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .map_err(|error| error.without_url().to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Download::NotModified);
    }
    if !response.status().is_success() {
        return Err(format!("http_{}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES)
    {
        return Err("snapshot_too_large".into());
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("snapshot_too_large".into());
    }
    // Validate before anything touches the cache file; keep the ETag only for
    // the document we actually accepted.
    let mut parsed = Catalog::from_api(&bytes, source, now())?;
    parsed.snapshot.etag = etag;
    Ok(Download::Fresh(parsed.snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_bytes(providers: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "sourceUrl": DEFAULT_SOURCE_URL,
            "fetchedAt": 1_700_000_000,
            "revision": "fixture",
            "providers": providers,
        }))
        .unwrap()
    }

    fn fixture() -> Vec<u8> {
        snapshot_bytes(serde_json::json!([
            {
                "id": "anthropic",
                "name": "Anthropic",
                "npm": "@ai-sdk/anthropic",
                "models": [
                    {
                        "id": "claude-opus-4-5",
                        "name": "Claude Opus 4.5",
                        "family": "claude-opus",
                        "reasoning": true,
                        "reasoningOptions": [{"type": "effort", "values": ["low", "high"]}],
                        "limit": {"context": 200000, "output": 64000},
                        "cost": {"input": 5, "output": 25, "cacheRead": 0.5},
                        "lastUpdated": "2025-11-24"
                    },
                    {"id": "CLAUDE-OPUS-4-5", "name": "Case variant", "limit": {"context": 1000, "output": 100}}
                ]
            },
            {
                "id": "openrouter",
                "name": "OpenRouter",
                "models": [
                    {"id": "anthropic/claude-opus-4-5", "name": "Claude via OR", "limit": {"context": 200000, "output": 32000}},
                    {"id": "meta/llama-4", "name": "Llama", "reasoning": false, "limit": {"context": 128000, "output": 8192}}
                ]
            }
        ]))
    }

    #[test]
    fn parses_and_indexes_with_case_folding_and_scoping() {
        let catalog = Catalog::parse(&fixture(), Origin::Runtime).unwrap();
        assert_eq!(catalog.snapshot.providers.len(), 2);
        assert_eq!(catalog.provider("ANTHROPIC").unwrap().name, "Anthropic");
        assert!(catalog.exact_in("anthropic", "claude-opus-4-5").is_some());
        assert!(catalog.exact_in("anthropic", "CLAUDE-OPUS-4-5").is_some());
        // Case-insensitive lookup returns both case variants; ambiguity is the
        // caller's decision.
        let matches = catalog.scoped(&["anthropic".into()], "Claude-Opus-4-5");
        assert_eq!(matches.len(), 2);
        let global = catalog.global("anthropic/claude-opus-4-5");
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].provider.id, "openrouter");
        // Scoped lookups never leak into other providers.
        assert!(catalog
            .scoped(&["openrouter".into()], "claude-opus-4-5")
            .is_empty());
    }

    #[test]
    fn canonical_aliases_stay_scoped_and_prefer_active_publisher_records() {
        let catalog = Catalog::parse(&snapshot_bytes(serde_json::json!([
            {"id": "deepseek", "models": [
                {"id": "deepseek-flash", "canonicalModelId": "deepseek/deepseek-v4.1-flash", "limit": {"context": 1000000}},
                {"id": "deepseek-v4-flash", "canonicalModelId": "deepseek/deepseek-v4.1-flash", "status": "deprecated", "limit": {"context": 128000}}
            ]},
            {"id": "302ai", "models": [
                {"id": "deepseek-flash", "canonicalModelId": "deepseek/deepseek-v4.1-flash", "limit": {"context": 4000}}
            ]}
        ])), Origin::Embedded).unwrap();
        for id in ["deepseek-v4.1-flash", "DEEPSEEK/deepseek-v4.1-flash"] {
            let matches = catalog.canonical_scoped(&["deepseek"], id);
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].provider.id, "deepseek");
            assert_eq!(matches[0].model.id, "deepseek-flash");
        }
        assert!(catalog
            .canonical_scoped(&["deepseek"], "deepseek-v4.2-flash")
            .is_empty());
        // Direct requests for a deprecated ID still resolve that exact entry.
        assert_eq!(
            catalog
                .exact_in("deepseek", "deepseek-v4-flash")
                .unwrap()
                .model
                .limit
                .context,
            Some(128000)
        );
    }

    #[test]
    fn rejects_malformed_documents_instead_of_publishing_them() {
        for bytes in [
            b"{}".to_vec(),
            b"not json".to_vec(),
            snapshot_bytes(serde_json::json!([])),
            snapshot_bytes(serde_json::json!([{"id": "p", "models": []}])),
            snapshot_bytes(
                serde_json::json!([{"id": "p", "models": [{"id": "m", "limit": {"context": 0}}]}]),
            ),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 99,
                "providers": [{"id": "p", "models": [{"id": "m", "limit": {"context": 1}}]}]
            }))
            .unwrap(),
        ] {
            assert!(Catalog::parse(&bytes, Origin::Runtime).is_err());
        }
    }

    #[test]
    fn malformed_interleaved_hints_never_drop_models_or_the_catalog() {
        for hint in [
            serde_json::json!({"field": 5}),
            serde_json::json!([]),
            serde_json::json!("x"),
            serde_json::json!({"field": ""}),
            serde_json::json!({"field": "x".repeat(129)}),
            serde_json::json!({"field": "bad\nfield"}),
        ] {
            let mut snapshot: Value = serde_json::from_slice(&fixture()).unwrap();
            snapshot["providers"][0]["models"][0]["interleaved"] = hint.clone();
            let catalog =
                Catalog::parse(&serde_json::to_vec(&snapshot).unwrap(), Origin::Runtime).unwrap();
            let model = catalog
                .exact_in("anthropic", "claude-opus-4-5")
                .unwrap()
                .model;
            assert!(model.interleaved.is_none(), "snapshot hint: {hint}");
            assert_eq!(catalog.model_count(), 4);

            let api = serde_json::json!({"p": {"id": "p", "name": "P", "models": {
                "m": {"id": "m", "name": "M", "limit": {"context": 100}, "interleaved": hint}
            }}});
            let catalog =
                Catalog::from_api(api.to_string().as_bytes(), DEFAULT_SOURCE_URL, 1).unwrap();
            let model = catalog.exact_in("p", "m").unwrap().model;
            assert!(model.interleaved.is_none(), "API hint: {hint}");
            assert_eq!(catalog.model_count(), 1);
        }
    }

    #[test]
    fn runtime_cache_wins_over_the_embedded_snapshot_and_falls_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cache_path(dir.path()), fixture()).unwrap();
        let catalog = load(dir.path());
        assert_eq!(catalog.origin, Origin::Runtime);
        assert_eq!(catalog.provider("anthropic").unwrap().name, "Anthropic");

        // A corrupt cache falls back to the committed snapshot, never to a
        // partial merge of the two.
        states().lock().unwrap().remove(dir.path());
        std::fs::write(cache_path(dir.path()), b"{ truncated").unwrap();
        let catalog = load(dir.path());
        assert_eq!(catalog.origin, Origin::Embedded);
        assert!(catalog.snapshot.providers.len() > 10);
        states().lock().unwrap().remove(dir.path());
    }

    #[test]
    fn stale_detection_uses_the_twenty_four_hour_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut value: Value = serde_json::from_slice(&fixture()).unwrap();
        value["fetchedAt"] = serde_json::json!(1_700_000_000);
        std::fs::write(cache_path(dir.path()), serde_json::to_vec(&value).unwrap()).unwrap();
        let catalog = load(dir.path());
        assert!(catalog.is_stale(1_700_000_000 + 86_401));
        assert!(!catalog.is_stale(1_700_000_000 + 86_399));
        states().lock().unwrap().remove(dir.path());
    }

    #[test]
    fn refresh_failure_never_replaces_valid_data() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cache_path(dir.path()), fixture()).unwrap();
        let before = load(dir.path());
        record_failure(dir.path(), "network_error");
        let after = load(dir.path());
        assert_eq!(after.snapshot.revision, before.snapshot.revision);
        assert_eq!(
            after.snapshot.providers.len(),
            before.snapshot.providers.len()
        );
        assert_eq!(after.error.as_deref(), Some("network_error"));
        assert!(after.provider("anthropic").is_some());
        states().lock().unwrap().remove(dir.path());
    }

    #[test]
    fn path_cache_eviction_keeps_fresh_failure_status() {
        let catalog = Arc::new(Catalog::parse(&fixture(), Origin::Runtime).unwrap());
        let checked = Instant::now();
        let mut entries = HashMap::from([
            (PathBuf::from("old"), State {
                catalog: catalog.clone(),
                checked: checked - Duration::from_secs(10),
            }),
            (PathBuf::from("recent"), State {
                catalog,
                checked: checked - Duration::from_secs(1),
            }),
            (PathBuf::from("failed"), State {
                catalog: Arc::new(Catalog::parse(&fixture(), Origin::Runtime).unwrap()
                    .with_error(Some("network_error".into()))),
                checked,
            }),
        ]);
        evict_oldest(&mut entries);
        assert!(!entries.contains_key(Path::new("old")));
        assert!(entries.contains_key(Path::new("recent")));
        assert_eq!(entries[Path::new("failed")].catalog.error.as_deref(), Some("network_error"));
    }

    #[test]
    fn embedded_snapshot_ships_models_and_prices() {
        let parsed = Catalog::parse(EMBEDDED.as_bytes(), Origin::Embedded);
        assert!(parsed.is_ok(), "embedded parse failed: {:?}", parsed.err());
        let catalog = embedded();
        assert!(
            catalog.snapshot.providers.len() > 100,
            "embedded snapshot is too small"
        );
        let anthropic = catalog.provider("anthropic").unwrap();
        let opus = anthropic
            .models
            .iter()
            .find(|model| model.id == "claude-opus-4-5")
            .unwrap();
        assert_eq!(opus.limit.context, Some(200_000));
        assert_eq!(opus.limit.output, Some(64_000));
        assert!(opus.cost.as_ref().unwrap().input.is_some());
        assert!(opus
            .reasoning_options
            .iter()
            .any(|option| !option.efforts().is_empty()));
    }
}
