//! Model identity matching and per-field metadata provenance.
//!
//! One resolver answers "where does every displayed value come from": user
//! overrides, provider settings, real upstream declarations, the signed-in
//! account snapshot, the models.dev catalog (exact / normalized / generic
//! reference), and finally an explicit fallback. Routing never consumes these
//! values: the upstream request keeps the raw model ID.
use super::models_dev::{Catalog, Cost, ModelRef, ReasoningOption};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

/// One selectable reasoning level: a stable identifier plus display text.
/// Re-exported by the provider configuration, which persists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningLevel {
    pub effort: String,
    pub description: String,
}

impl ReasoningLevel {
    /// Bounds that keep stored declarations safe and renderable.
    pub(crate) fn valid(&self) -> bool {
        !self.effort.is_empty()
            && self.effort.len() <= 32
            && self
                .effort
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && self.description.len() <= 1024
    }
}

pub const FALLBACK_CONTEXT_WINDOW: u64 = 128_000;
pub const FALLBACK_MAX_OUTPUT_TOKENS: u64 = 8_192;
const MAX_CANDIDATES: usize = 16;
const MAX_CONFLICT_VALUES: usize = 6;

pub mod source {
    pub const MANUAL: &str = "manual";
    pub const PROVIDER_SETTING: &str = "providerSetting";
    pub const UPSTREAM: &str = "upstream";
    pub const OFFICIAL: &str = "official";
    /// Scoped models.dev match (exact or normalized within configured catalogs).
    pub const CATALOG: &str = "catalog";
    /// Unscoped models.dev reference where every filled field agrees.
    pub const CATALOG_GENERIC: &str = "catalogGeneric";
    pub const CATALOG_ALIAS: &str = "catalogAlias";
    pub const POLICY: &str = "policy";
    pub const FALLBACK: &str = "fallback";
    pub const UNKNOWN: &str = "unknown";
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchKind {
    Explicit,
    Exact,
    Normalized,
    Consensus,
    Ambiguous,
    Invalid,
    #[default]
    Unmatched,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Exact => "exact",
            Self::Normalized => "normalized",
            Self::Consensus => "consensus",
            Self::Ambiguous => "ambiguous",
            Self::Invalid => "invalid",
            Self::Unmatched => "unmatched",
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchStep {
    pub model_id: String,
    pub step: String,
    pub matched: Vec<ModelRef>,
    pub note: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictValue {
    pub model_ref: ModelRef,
    pub value: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldConflict {
    pub field: String,
    pub candidates: Vec<ConflictValue>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Field<T> {
    pub value: Option<T>,
    pub source: String,
    pub source_ref: Option<ModelRef>,
    pub updated_at: Option<String>,
}

impl<T> Field<T> {
    fn unknown() -> Self {
        Self {
            value: None,
            source: source::UNKNOWN.into(),
            source_ref: None,
            updated_at: None,
        }
    }

    fn new(
        value: T,
        source: &str,
        source_ref: Option<ModelRef>,
        updated_at: Option<String>,
    ) -> Self {
        Self {
            value: Some(value),
            source: source.into(),
            source_ref,
            updated_at,
        }
    }
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningOptionInfo {
    pub kind: String,
    pub values: Vec<String>,
    pub min: Option<u64>,
    pub max: Option<u64>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningInfo {
    /// Effort-style levels. `Some([])` is an explicit "no levels" declaration;
    /// `None` means unknown.
    pub levels: Field<Vec<ReasoningLevel>>,
    /// Declared reasoning support when the source says so explicitly.
    pub supports: Option<bool>,
    /// Raw catalog options (effort / toggle / budget tokens), for display only.
    pub options: Vec<ReasoningOptionInfo>,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PriceInfo {
    pub currency: String,
    pub unit: String,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub reasoning: Option<f64>,
    pub input_audio: Option<f64>,
    pub output_audio: Option<f64>,
    pub tiers: Vec<super::models_dev::CostTier>,
    pub context_over_200k: Option<super::models_dev::CostRates>,
    pub source: String,
    pub source_ref: Option<ModelRef>,
    pub updated_at: Option<String>,
    /// False when conditional pricing exists that static rates cannot fully
    /// represent (tiered / >200k contexts).
    pub complete: bool,
    pub note: Option<String>,
}

impl PriceInfo {
    fn unknown(note: Option<&str>) -> Self {
        Self {
            currency: "USD".into(),
            unit: "perMillionTokens".into(),
            source: source::UNKNOWN.into(),
            complete: false,
            note: note.map(str::to_owned),
            ..Default::default()
        }
    }
}

#[derive(Clone, Default)]
pub struct UserValues {
    pub context_window: Option<u64>,
    pub input_limit: Option<u64>,
    pub output_limit: Option<u64>,
    /// `Some(vec![..])` is an explicit user selection; `Some(vec![])` is an
    /// explicit "no levels"; `None` inherits.
    pub reasoning: Option<Vec<ReasoningLevel>>,
    pub default_effort: Option<String>,
    pub price: Option<CostRatesInput>,
}

#[derive(Clone, Copy, Default)]
pub struct CostRatesInput {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

#[derive(Clone, Default)]
pub struct OfficialValues {
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub reasoning: Option<Vec<ReasoningLevel>>,
    pub default_effort: Option<String>,
    pub updated_at: Option<i64>,
}

#[derive(Clone, Default)]
pub struct UpstreamValues {
    pub context_window: Option<u64>,
    pub input_limit: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub reasoning: Option<Vec<ReasoningLevel>>,
    pub default_effort: Option<String>,
    pub fetched_at: Option<i64>,
    /// Historical values that may have been auto-filled; never treated as a
    /// real vendor declaration.
    pub legacy: bool,
}

pub struct ResolutionRequest<'a> {
    pub raw_model_id: &'a str,
    /// Saved display alias; empty inherits from the catalog name.
    pub display_name: &'a str,
    /// models.dev provider IDs this configured provider maps to. Empty means
    /// only generic reference matching is available.
    pub catalog_providers: &'a [String],
    /// Independently bound billing providers. Capability mappings alone never
    /// establish what a relay charges.
    pub price_providers: &'a [String],
    /// Explicit `{provider, model}` capability reference.
    pub explicit_ref: Option<&'a ModelRef>,
    /// Independent price binding; never inferred from the capability match.
    pub price_ref: Option<&'a ModelRef>,
    pub user: UserValues,
    /// Provider-level context window setting (existing override semantics).
    pub provider_context: Option<u64>,
    pub official: Option<OfficialValues>,
    pub upstream: Option<UpstreamValues>,
}

impl ResolutionRequest<'_> {
    fn price_scope(&self) -> &[String] {
        self.price_providers
    }
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMetadata {
    pub raw_model_id: String,
    pub display_name: String,
    pub display_source: String,
    pub match_kind: MatchKind,
    pub matched: Option<ModelRef>,
    pub steps: Vec<MatchStep>,
    pub conflicts: Vec<FieldConflict>,
    pub context_window: Field<u64>,
    pub input_limit: Field<u64>,
    pub output_limit: Field<u64>,
    pub reasoning: ReasoningInfo,
    pub default_effort: Option<String>,
    pub default_effort_source: String,
    pub price: PriceInfo,
}

pub struct Candidate {
    pub value: String,
    pub step: &'static str,
    /// Safety of the entire reduction path, not just its final step.
    pub price_safe: bool,
}

const KNOWN_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
const KNOWN_TAGS: &[&str] = &[
    "free", "batch", "fast", "nitro", "online", "thinking", "latest", "stable",
];
const BEDROCK_REGIONS: &[&str] = &["us", "eu", "apac", "ap", "sa", "ca", "me", "af"];

fn is_known_effort(value: &str) -> bool {
    KNOWN_EFFORTS.contains(&value.to_ascii_lowercase().as_str())
}

fn is_known_tag(value: &str) -> bool {
    KNOWN_TAGS.contains(&value.to_ascii_lowercase().as_str())
}

fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn strip_suffix_ci<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    value
        .get(value.len().saturating_sub(suffix.len())..)
        .filter(|tail| tail.eq_ignore_ascii_case(suffix))
        .map(|_| &value[..value.len() - suffix.len()])
}

/// Ordered identity candidates. The raw ID is always first. Every reduction is
/// one explicit step so the preview can explain exactly what was attempted.
/// Version numbers, dates and `-preview` suffixes are never removed.
pub fn candidates(raw_model_id: &str) -> Vec<Candidate> {
    let mut queue: VecDeque<(&'static str, String, bool)> = VecDeque::new();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    queue.push_back(("exact", raw_model_id.trim().to_owned(), true));
    while let Some((step, current, safe)) = queue.pop_front() {
        if current.is_empty() || !seen.insert(current.clone()) {
            continue;
        }
        out.push(Candidate {
            value: current.clone(),
            step,
            price_safe: safe,
        });
        if out.len() >= MAX_CANDIDATES {
            break;
        }
        for (next_step, next) in reductions(&current) {
            if !seen.contains(&next) {
                queue.push_back((next_step, next, safe && price_safe(next_step)));
            }
        }
    }
    out
}

fn reductions(current: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut push = |step: &'static str, value: &str| {
        let value = value.trim();
        if value.is_empty() || value == current {
            return;
        }
        if out.iter().any(|(_, known)| known == value) {
            return;
        }
        out.push((step, value.to_owned()));
    };
    for (last_step, first_step, separator) in [
        ("channelMarker", "channelNamespace", "::"),
        ("pathPrefix", "pathNamespace", "/"),
        ("channelPrefix", "channelPrefixNamespace", "__"),
    ] {
        if let Some((_, rest)) = current.rsplit_once(separator) {
            push(last_step, rest);
        }
        if let Some((_, rest)) = current.split_once(separator) {
            push(first_step, rest);
        }
    }
    if let Some(rest) = strip_prefix_ci(current, "anthropic.") {
        push("providerPrefix", rest);
    }
    if let Some((region, rest)) = current.split_once('.') {
        if BEDROCK_REGIONS.contains(&region.to_ascii_lowercase().as_str()) && rest.contains('.') {
            push("regionPrefix", rest);
        }
    }
    if let Some(index) = current.find('[') {
        push("clientOption", &current[..index]);
    }
    if current.ends_with(')') {
        if let Some(index) = current.rfind('(') {
            if is_known_effort(&current[index + 1..current.len() - 1]) {
                push("effortSuffix", &current[..index]);
            }
        }
    }
    if let Some((base, tag)) = current.rsplit_once(':') {
        if is_known_tag(tag) || is_known_effort(tag) {
            push("tagSuffix", base);
        }
    }
    if let Some((base, tag)) = current.rsplit_once('@') {
        if is_known_tag(tag) || is_known_effort(tag) {
            push("atTag", base);
        }
    }
    if let Some(base) = strip_suffix_ci(current, "-kiro") {
        push("knownChannelSuffix", base);
    }
    // Relay IDs often encode a reasoning mode or effort as a suffix. Keep the
    // full identity first, and reduce only known mode names, never versions,
    // dates, preview/vision variants or arbitrary trailing words.
    for separator in ['-', '_'] {
        if let Some((base, mode)) = current.rsplit_once(separator) {
            if mode.eq_ignore_ascii_case("thinking")
                || (is_known_effort(mode) && !matches!(mode.to_ascii_lowercase().as_str(), "max" | "ultra")) {
                push("reasoningSuffix", base);
            }
        }
    }
    // More specific (longer) candidates are attempted first within one level.
    out.sort_by(|left, right| right.1.len().cmp(&left.1.len()));
    out
}

/// Whether a catalog price may be borrowed across this normalization step.
/// Tagged and reasoning-mode variants can have their own billing rates.
fn price_safe(step: &str) -> bool {
    !matches!(
        step,
        "tagSuffix" | "atTag" | "reasoningSuffix" | "effortSuffix"
    )
}

#[derive(Debug, Clone, PartialEq)]
enum Agreement<T: Clone + PartialEq> {
    Missing,
    Agreed(T),
    Conflict(Vec<(ModelRef, T)>),
}

impl<T: Clone + PartialEq> Agreement<T> {
    fn agreed(&self) -> Option<&T> {
        match self {
            Self::Agreed(value) => Some(value),
            _ => None,
        }
    }
}

fn agreement<T, F>(matches: &[CatalogMatch<'_>], pick: F) -> Agreement<T>
where
    T: Clone + PartialEq,
    F: Fn(&CatalogMatch<'_>) -> Option<T>,
{
    let mut values: Vec<(ModelRef, T)> = Vec::new();
    for entry in matches {
        let Some(value) = pick(entry) else {
            continue;
        };
        let reference = entry.model_ref();
        if let Some((_, existing)) = values.iter().find(|(known, _)| known == &reference) {
            if existing != &value {
                values.push((reference, value));
                return Agreement::Conflict(values);
            }
            continue;
        }
        values.push((reference, value));
    }
    let Some(first) = values.first() else {
        return Agreement::Missing;
    };
    if values.iter().any(|(_, value)| value != &first.1) {
        return Agreement::Conflict(values);
    }
    Agreement::Agreed(first.1.clone())
}

/// A catalog match with its provider/model identity, kept in match order.
#[derive(Clone, Copy)]
pub struct CatalogMatch<'a> {
    pub provider_id: &'a str,
    pub model: &'a super::models_dev::Model,
    pub model_id: &'a str,
}

impl CatalogMatch<'_> {
    pub fn model_ref(&self) -> ModelRef {
        ModelRef {
            provider_id: self.provider_id.to_owned(),
            model_id: self.model_id.to_owned(),
        }
    }
}

/// Whether at least one fillable field agrees across the matches.
fn has_agreement(matches: &[CatalogMatch<'_>]) -> bool {
    [
        matches!(
            agreement(matches, |entry| entry.model.limit.context),
            Agreement::Agreed(_)
        ),
        matches!(
            agreement(matches, |entry| entry.model.limit.input),
            Agreement::Agreed(_)
        ),
        matches!(
            agreement(matches, |entry| entry.model.limit.output),
            Agreement::Agreed(_)
        ),
        matches!(
            agreement(matches, |entry| effective_levels(entry.model)),
            Agreement::Agreed(_)
        ),
    ]
    .into_iter()
    .any(|agreed| agreed)
}

fn to_match(entry: super::models_dev::Match<'_>) -> CatalogMatch<'_> {
    CatalogMatch {
        provider_id: &entry.provider.id,
        model: entry.model,
        model_id: &entry.model.id,
    }
}

fn effective_levels(model: &super::models_dev::Model) -> Option<Vec<ReasoningLevel>> {
    if model.reasoning == Some(false) {
        return Some(Vec::new());
    }
    let mut values: Vec<String> = Vec::new();
    for option in &model.reasoning_options {
        for value in option.efforts() {
            if !values.iter().any(|known| known.eq_ignore_ascii_case(value)) {
                values.push(value.clone());
            }
        }
    }
    if !values.is_empty() {
        return Some(
            values
                .iter()
                .map(|value| ReasoningLevel {
                    effort: value.clone(),
                    description: effort_description(value),
                })
                .collect(),
        );
    }
    // Reasoning is declared (toggle / budget) but no discrete efforts exist.
    model.reasoning.map(|_| Vec::new())
}

pub fn effort_description(effort: &str) -> String {
    match effort.to_ascii_lowercase().as_str() {
        "none" => "No reasoning",
        "minimal" => "Minimal reasoning",
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balanced reasoning",
        "high" => "Deeper reasoning for complex tasks",
        "xhigh" => "Extra high reasoning for complex tasks",
        "max" => "Maximum reasoning depth",
        "ultra" => "Ultra reasoning depth",
        _ => "Model-declared reasoning level",
    }
    .to_owned()
}

pub(crate) fn option_infos(model: &super::models_dev::Model) -> Vec<ReasoningOptionInfo> {
    model
        .reasoning_options
        .iter()
        .map(|option| match option {
            ReasoningOption::Effort { values } => ReasoningOptionInfo {
                kind: "effort".into(),
                values: values.clone(),
                ..Default::default()
            },
            ReasoningOption::Toggle => ReasoningOptionInfo {
                kind: "toggle".into(),
                ..Default::default()
            },
            ReasoningOption::BudgetTokens { min, max } => ReasoningOptionInfo {
                kind: "budgetTokens".into(),
                min: *min,
                max: *max,
                ..Default::default()
            },
        })
        .collect()
}

fn format_timestamp(timestamp: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(timestamp, 0).map(|time| time.to_rfc3339())
}

fn field_provenance(
    matches: &[CatalogMatch<'_>],
    present: impl Fn(&super::models_dev::Model) -> bool,
) -> (Option<ModelRef>, Option<String>) {
    matches
        .iter()
        .find(|entry| present(entry.model))
        .map(|entry| (Some(entry.model_ref()), entry.model.last_updated.clone()))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn resolve_number(
    manual: Option<u64>,
    provider_setting: Option<u64>,
    upstream: Option<u64>,
    official: Option<u64>,
    catalog: &Agreement<u64>,
    catalog_source: &str,
    catalog_ref: Option<ModelRef>,
    catalog_updated: Option<String>,
    official_time: Option<i64>,
    upstream_time: Option<i64>,
) -> Field<u64> {
    if let Some(value) = manual {
        return Field::new(value, source::MANUAL, None, None);
    }
    if let Some(value) = provider_setting {
        return Field::new(value, source::PROVIDER_SETTING, None, None);
    }
    if let Some(value) = upstream {
        return Field::new(
            value,
            source::UPSTREAM,
            None,
            upstream_time.and_then(format_timestamp),
        );
    }
    if let Some(value) = official {
        return Field::new(
            value,
            source::OFFICIAL,
            None,
            official_time.and_then(format_timestamp),
        );
    }
    if let Agreement::Agreed(value) = catalog {
        return Field::new(*value, catalog_source, catalog_ref, catalog_updated);
    }
    Field::unknown()
}

fn number_conflict(field: &str, agreement: &Agreement<u64>) -> Option<FieldConflict> {
    let Agreement::Conflict(values) = agreement else {
        return None;
    };
    Some(FieldConflict {
        field: field.into(),
        candidates: values
            .iter()
            .take(MAX_CONFLICT_VALUES)
            .map(|(reference, value)| ConflictValue {
                model_ref: reference.clone(),
                value: value.to_string(),
            })
            .collect(),
    })
}

fn levels_text(levels: &[ReasoningLevel]) -> String {
    if levels.is_empty() {
        return "(none)".into();
    }
    levels
        .iter()
        .map(|level| level.effort.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn resolve(catalog: &Catalog, request: &ResolutionRequest<'_>) -> ResolvedMetadata {
    resolve_with_global_fallback(catalog, request, true, None)
}

/// Resolve metadata while optionally preventing a scoped catalog miss from
/// falling back to unrelated providers. Endpoint-derived scopes use strict
/// matching so a relay with the same model ID cannot impersonate the origin.
/// A global provider allowlist limits automatic references; explicit user
/// references and configured scopes retain their own metadata.
pub fn resolve_with_global_fallback(
    catalog: &Catalog,
    request: &ResolutionRequest<'_>,
    allow_global_fallback: bool,
    allowed_global_providers: Option<&[&str]>,
) -> ResolvedMetadata {
    let raw = request.raw_model_id.trim();
    let mut result = ResolvedMetadata {
        raw_model_id: raw.to_owned(),
        ..Default::default()
    };
    let candidates = candidates(raw);
    let mut matches: Vec<CatalogMatch<'_>> = Vec::new();
    let mut kind = MatchKind::Unmatched;
    let mut matched_scoped = false;

    if let Some(reference) = request.explicit_ref {
        match catalog.exact_in(&reference.provider_id, &reference.model_id) {
            Some(found) => {
                matches.push(to_match(found));
                kind = MatchKind::Explicit;
                matched_scoped = true;
                result.steps.push(MatchStep {
                    model_id: raw.to_owned(),
                    step: "explicit".into(),
                    matched: vec![reference.clone()],
                    note: "explicit capability reference".into(),
                });
            }
            None => {
                kind = MatchKind::Invalid;
                result.steps.push(MatchStep {
                    model_id: raw.to_owned(),
                    step: "explicit".into(),
                    matched: Vec::new(),
                    note: "explicit capability reference no longer exists".into(),
                });
            }
        }
    } else {
        for candidate in &candidates {
            let mut found = Vec::new();
            let mut canonical = false;
            if !request.catalog_providers.is_empty() {
                for entry in catalog.scoped(request.catalog_providers, &candidate.value) {
                    found.push(to_match(entry));
                }
                if found.is_empty() {
                    for entry in
                        catalog.canonical_scoped(request.catalog_providers, &candidate.value)
                    {
                        found.push(to_match(entry));
                    }
                    canonical = !found.is_empty();
                }
            }
            let scoped = !found.is_empty();
            if !scoped && (request.catalog_providers.is_empty() || allow_global_fallback) {
                for entry in catalog.global(&candidate.value) {
                    if allowed_global_providers.is_none_or(|providers| {
                        providers
                            .iter()
                            .any(|id| id.eq_ignore_ascii_case(&entry.provider.id))
                    }) {
                        found.push(to_match(entry));
                    }
                }
            }
            if found.is_empty() {
                result.steps.push(MatchStep {
                    model_id: candidate.value.clone(),
                    step: candidate.step.into(),
                    matched: Vec::new(),
                    note: "no catalog match".into(),
                });
                continue;
            }
            let refs: Vec<ModelRef> = found.iter().map(CatalogMatch::model_ref).collect();
            result.steps.push(MatchStep {
                model_id: candidate.value.clone(),
                step: if canonical {
                    "canonicalIdentity"
                } else {
                    candidate.step
                }
                .into(),
                matched: refs.clone(),
                note: if canonical {
                    format!(
                        "publisher-declared canonical identity; matched {} scoped catalog entries",
                        refs.len()
                    )
                } else if scoped {
                    format!(
                        "matched {} scoped catalog entr{}",
                        refs.len(),
                        if refs.len() == 1 { "y" } else { "ies" }
                    )
                } else {
                    format!(
                        "generic reference across {} catalog entr{}",
                        refs.len(),
                        if refs.len() == 1 { "y" } else { "ies" }
                    )
                },
            });
            matched_scoped = scoped;
            let distinct: HashSet<(&str, &str)> = found
                .iter()
                .map(|entry| (entry.provider_id, entry.model_id))
                .collect();
            kind = if !scoped {
                // A generic reference stays a consensus suggestion as long as
                // at least one field agrees; per-field conflicts are reported
                // separately and never injected.
                if has_agreement(&found) {
                    MatchKind::Consensus
                } else {
                    MatchKind::Ambiguous
                }
            } else if distinct.len() > 1 {
                MatchKind::Ambiguous
            } else if !canonical && candidate.step == "exact" && candidate.value == raw {
                MatchKind::Exact
            } else {
                MatchKind::Normalized
            };
            matches = found;
            break;
        }
        // Some relays omit Google's preview marker from Gemini names. Only
        // consult the same-version `-preview` entry after every exact/canonical
        // candidate has missed. This is a capability reference, not a declared
        // identity alias; never feed the added marker into price resolution.
        if matches.is_empty()
            && request
                .catalog_providers
                .iter()
                .any(|provider| provider.eq_ignore_ascii_case("google"))
        {
            let scope = ["google".to_owned()];
            for candidate in &candidates {
                let id = candidate.value.to_ascii_lowercase();
                if !id.starts_with("gemini-") || id.contains("-preview") {
                    continue;
                }
                let preview_id = format!("{}-preview", candidate.value);
                let found: Vec<_> = catalog
                    .scoped(&scope, &preview_id)
                    .into_iter()
                    .filter(|entry| {
                        !entry
                            .model
                            .status
                            .as_deref()
                            .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
                    })
                    .map(to_match)
                    .collect();
                if found.is_empty() {
                    continue;
                }
                result.steps.push(MatchStep {
                    model_id: candidate.value.clone(),
                    step: "previewReference".into(),
                    matched: found.iter().map(CatalogMatch::model_ref).collect(),
                    note: format!(
                        "Google same-version preview capability reference: {preview_id}; request ID and billing unchanged"
                    ),
                });
                let distinct: HashSet<_> = found
                    .iter()
                    .map(|entry| (entry.provider_id, entry.model_id))
                    .collect();
                kind = if distinct.len() > 1 {
                    MatchKind::Ambiguous
                } else {
                    MatchKind::Normalized
                };
                matched_scoped = true;
                matches = found;
                break;
            }
        }
    }

    if !matches.is_empty() {
        result.matched = Some(pick_match_ref(&matches));
    }
    result.match_kind = kind;

    // Name: explicit alias → catalog name → raw ID.
    let saved_alias = request.display_name.trim();
    if !saved_alias.is_empty() {
        result.display_name = saved_alias.to_owned();
        result.display_source = source::MANUAL.into();
    } else if let Some(name) = matches
        .iter()
        .find_map(|entry| (!entry.model.name.is_empty()).then_some(entry.model.name.as_str()))
    {
        result.display_name = name.to_owned();
        result.display_source = source::CATALOG.into();
    } else {
        result.display_name = raw.to_owned();
        result.display_source = source::UNKNOWN.into();
    }

    let catalog_source = if matched_scoped {
        source::CATALOG
    } else {
        source::CATALOG_GENERIC
    };

    // Numbers.
    let context = agreement(&matches, |entry| entry.model.limit.context);
    let input = agreement(&matches, |entry| entry.model.limit.input);
    let output = agreement(&matches, |entry| entry.model.limit.output);
    let (context_ref, context_updated) =
        field_provenance(&matches, |model| model.limit.context.is_some());
    let (input_ref, input_updated) =
        field_provenance(&matches, |model| model.limit.input.is_some());
    let (output_ref, output_updated) =
        field_provenance(&matches, |model| model.limit.output.is_some());
    let (reasoning_ref, reasoning_updated) =
        field_provenance(&matches, |model| effective_levels(model).is_some());
    let official_time = request
        .official
        .as_ref()
        .and_then(|values| values.updated_at);
    let upstream_time = request
        .upstream
        .as_ref()
        .and_then(|values| values.fetched_at);
    let live_upstream = request.upstream.as_ref().filter(|values| !values.legacy);

    let context_field = resolve_number(
        request.user.context_window,
        request.provider_context,
        live_upstream.and_then(|values| values.context_window),
        request
            .official
            .as_ref()
            .and_then(|values| values.context_window),
        &context,
        catalog_source,
        context_ref,
        context_updated,
        official_time,
        upstream_time,
    );
    let input_field = resolve_number(
        request.user.input_limit,
        None,
        live_upstream.and_then(|values| values.input_limit),
        None,
        &input,
        catalog_source,
        input_ref,
        input_updated,
        official_time,
        upstream_time,
    );
    let mut output_field = resolve_number(
        request.user.output_limit,
        None,
        live_upstream.and_then(|values| values.max_output_tokens),
        request
            .official
            .as_ref()
            .and_then(|values| values.max_output_tokens),
        &output,
        catalog_source,
        output_ref,
        output_updated,
        official_time,
        upstream_time,
    );
    // Provider context keeps its long-standing clamp over non-manual output.
    if let Some(limit) = request.provider_context {
        if output_field.source != source::MANUAL {
            if let Some(output) = output_field.value {
                if output > limit {
                    output_field.value = Some(limit);
                    output_field.source = source::PROVIDER_SETTING.into();
                    output_field.source_ref = None;
                    output_field.updated_at = None;
                }
            }
        }
    }
    result
        .conflicts
        .extend(number_conflict("contextWindow", &context));
    result
        .conflicts
        .extend(number_conflict("inputLimit", &input));
    result
        .conflicts
        .extend(number_conflict("outputLimit", &output));
    if matches!(context, Agreement::Conflict(_)) {
        // A conflict on the total context also makes the derived input budget
        // ambiguous unless the catalog supplied an explicit input limit.
        if input_field.source == source::CATALOG_GENERIC || input_field.source == source::CATALOG {
            result.input_limit = Field::unknown();
        }
    }
    result.context_window = context_field;
    result.input_limit = input_field;
    result.output_limit = output_field;

    // Reasoning.
    let levels = agreement(&matches, |entry| effective_levels(entry.model));
    let supports = agreement(&matches, |entry| entry.model.reasoning);
    let levels_field = if let Some(user) = request.user.reasoning.clone() {
        Field::new(user, source::MANUAL, None, None)
    } else if let Some(upstream) = live_upstream.and_then(|values| values.reasoning.clone()) {
        Field::new(
            upstream,
            source::UPSTREAM,
            None,
            upstream_time.and_then(format_timestamp),
        )
    } else if let Some(official) = request
        .official
        .as_ref()
        .and_then(|values| values.reasoning.clone())
    {
        Field::new(
            official,
            source::OFFICIAL,
            None,
            official_time.and_then(format_timestamp),
        )
    } else {
        match &levels {
            Agreement::Agreed(value) => Field::new(
                value.clone(),
                catalog_source,
                reasoning_ref,
                reasoning_updated,
            ),
            Agreement::Conflict(values) => {
                result.conflicts.push(FieldConflict {
                    field: "reasoning".into(),
                    candidates: values
                        .iter()
                        .take(MAX_CONFLICT_VALUES)
                        .map(|(reference, value)| ConflictValue {
                            model_ref: reference.clone(),
                            value: levels_text(value),
                        })
                        .collect(),
                });
                Field::unknown()
            }
            Agreement::Missing => Field::unknown(),
        }
    };
    result.reasoning = ReasoningInfo {
        levels: levels_field,
        supports: supports.agreed().copied(),
        options: matches
            .first()
            .map(|entry| option_infos(entry.model))
            .unwrap_or_default(),
    };

    // Default effort: user selection → declared default from the level source →
    // explicit application policy. Never invented for the vendor.
    let final_levels = result.reasoning.levels.value.clone().unwrap_or_default();
    let supports_effort = |effort: &str| {
        final_levels
            .iter()
            .any(|level| level.effort.eq_ignore_ascii_case(effort))
    };
    let declared_default: Option<(String, String)> = request
        .user
        .default_effort
        .as_deref()
        .filter(|effort| supports_effort(effort))
        .map(|effort| (effort.to_owned(), source::MANUAL.to_owned()))
        .or_else(|| {
            let declared = if result.reasoning.levels.source == source::UPSTREAM {
                live_upstream.and_then(|values| values.default_effort.clone())
            } else if result.reasoning.levels.source == source::OFFICIAL {
                request
                    .official
                    .as_ref()
                    .and_then(|values| values.default_effort.clone())
            } else {
                None
            };
            let declared_source = result.reasoning.levels.source.clone();
            declared
                .filter(|effort| supports_effort(effort))
                .map(|effort| (effort, declared_source.clone()))
        });
    let (default_effort, default_source) = declared_default.unwrap_or_else(|| {
        let policy = final_levels
            .iter()
            .find(|level| level.effort.eq_ignore_ascii_case("medium"))
            .or_else(|| final_levels.first())
            .map(|level| level.effort.clone());
        (
            policy.unwrap_or_default(),
            if final_levels.is_empty() {
                source::UNKNOWN.to_owned()
            } else {
                source::POLICY.to_owned()
            },
        )
    });
    result.default_effort = (!default_effort.is_empty()).then_some(default_effort);
    result.default_effort_source = default_source.into();

    // Price. Borrows a price only from price-safe candidates within the
    // configured (billing) provider scope or from an explicit binding.
    result.price = resolve_price(catalog, request, &candidates);
    result
}

fn pick_match_ref(matches: &[CatalogMatch<'_>]) -> ModelRef {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for entry in matches {
        match counts
            .iter_mut()
            .find(|(known, _)| known == entry.provider_id)
        {
            Some((_, count)) => *count += 1,
            None => counts.push((entry.provider_id.to_owned(), 1)),
        }
    }
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let provider_id = counts
        .first()
        .map(|(provider, _)| provider.as_str())
        .unwrap_or_default();
    matches
        .iter()
        .find(|entry| entry.provider_id == provider_id)
        .map(CatalogMatch::model_ref)
        .unwrap_or_default()
}

fn resolve_price(
    catalog: &Catalog,
    request: &ResolutionRequest<'_>,
    candidates: &[Candidate],
) -> PriceInfo {
    // 1. Explicit user price override.
    if let Some(rates) = request.user.price {
        if rates.input.is_some() || rates.output.is_some() {
            return PriceInfo {
                input: rates.input,
                output: rates.output,
                cache_read: rates.cache_read,
                cache_write: rates.cache_write,
                source: source::MANUAL.into(),
                complete: rates.input.is_some() && rates.output.is_some(),
                note: (rates.input.is_none() || rates.output.is_none())
                    .then(|| "some token categories have no rate".into()),
                ..PriceInfo::unknown(None)
            };
        }
    }
    // 2. Only an independent model price reference overrides the billing scope.
    if let Some(reference) = request.price_ref {
        return match catalog.exact_in(&reference.provider_id, &reference.model_id) {
            Some(entry) => price_from_catalog(
                entry.model.cost.as_ref(),
                entry.model.last_updated.clone(),
                Some(entry.model_ref()),
                false,
            ),
            None => PriceInfo::unknown(Some("price binding no longer exists")),
        };
    }
    let scope = request.price_scope();
    if scope.is_empty() {
        // Without a provider scope a generic price would cross vendor
        // boundaries; keep it unknown and say why.
        return PriceInfo::unknown(Some(
            "no billing catalog binding; capability mappings do not establish prices",
        ));
    }
    // A price found through a reduced candidate is an alias fallback; the full
    // identity keeps the plain catalog label.
    let alias_of = |value: &str| !value.eq_ignore_ascii_case(request.raw_model_id.trim());
    // 3. Resolve the actual request identity first, then price-safe reductions
    // in their original order. Capability selection cannot reorder billing.
    for candidate in candidates.iter().filter(|candidate| candidate.price_safe) {
        if let Some(price) =
            scoped_price(catalog, scope, &candidate.value, alias_of(&candidate.value))
        {
            return price;
        }
    }
    PriceInfo::unknown(Some("no price record for this provider and model"))
}

/// Price for one model ID inside the billing scope; `None` when nothing is
/// priced or when the scope disagrees, so a conflict is never papered over.
fn scoped_price(
    catalog: &Catalog,
    scope: &[String],
    model_id: &str,
    alias: bool,
) -> Option<PriceInfo> {
    let matches = catalog.scoped(scope, model_id);
    let mut priced = matches
        .iter()
        .filter_map(|entry| entry.model.cost.as_ref().map(|cost| (*entry, cost)));
    let (first_entry, first_cost) = priced.next()?;
    if priced.any(|(_, cost)| cost != first_cost) {
        // A conflicting complete ID must not fall through to a cheaper alias.
        return Some(PriceInfo::unknown(Some(
            "billing catalogs disagree on pricing",
        )));
    }
    Some(price_from_catalog(
        Some(first_cost),
        first_entry.model.last_updated.clone(),
        Some(first_entry.model_ref()),
        alias,
    ))
}

fn price_from_catalog(
    cost: Option<&Cost>,
    updated_at: Option<String>,
    source_ref: Option<ModelRef>,
    alias: bool,
) -> PriceInfo {
    let Some(cost) = cost else {
        return PriceInfo::unknown(Some("price record exists without rates"));
    };
    let conditional = !cost.tiers.is_empty() || cost.context_over_200k.is_some();
    let audio = cost.input_audio.is_some()
        || cost.output_audio.is_some()
        || cost
            .context_over_200k
            .as_ref()
            .is_some_and(|rates| rates.has_audio())
        || cost.tiers.iter().any(|tier| tier.rates.has_audio());
    PriceInfo {
        currency: "USD".into(),
        unit: "perMillionTokens".into(),
        input: cost.input,
        output: cost.output,
        cache_read: cost.cache_read,
        cache_write: cost.cache_write,
        reasoning: cost.reasoning,
        input_audio: cost.input_audio,
        output_audio: cost.output_audio,
        tiers: cost.tiers.clone(),
        context_over_200k: cost.context_over_200k,
        source: if alias {
            source::CATALOG_ALIAS.into()
        } else {
            source::CATALOG.into()
        },
        source_ref,
        updated_at,
        complete: !conditional && !audio && cost.input.is_some() && cost.output.is_some(),
        note: if audio {
            Some("audio token rates are preserved; audio usage is not observed, so estimates are partial".to_owned())
        } else if conditional {
            Some("conditional pricing varies with context size".to_owned())
        } else if cost.input.is_none() || cost.output.is_none() {
            Some("some token categories have no rate".to_owned())
        } else {
            None
        },
    }
}

/// Shared product fallback for clients that must inject a number. Never
/// presented as catalog data.
pub fn fallback_values() -> (u64, u64) {
    (FALLBACK_CONTEXT_WINDOW, FALLBACK_MAX_OUTPUT_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> Catalog {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sourceUrl": "https://models.dev/api.json",
            "fetchedAt": 1_700_000_000,
            "revision": "fixture",
            "providers": [
                {
                    "id": "anthropic",
                    "name": "Anthropic",
                    "models": [
                        {
                            "id": "claude-opus-4-5",
                            "name": "Claude Opus 4.5",
                            "reasoning": true,
                            "reasoningOptions": [
                                {"type": "effort", "values": ["low", "medium", "high"]},
                                {"type": "budget_tokens", "min": 1024}
                            ],
                            "limit": {"context": 200000, "input": 180000, "output": 64000},
                            "cost": {"input": 5, "output": 25, "cacheRead": 0.5},
                            "lastUpdated": "2025-11-24"
                        },
                        {
                            "id": "claude-sonnet-4-5",
                            "name": "Claude Sonnet 4.5",
                            "reasoning": true,
                            "reasoningOptions": [{"type": "toggle"}],
                            "limit": {"context": 1000000, "output": 64000},
                            "cost": {"input": 3, "output": 15},
                            "lastUpdated": "2025-09-29"
                        }
                    ]
                },
                {
                    "id": "openrouter",
                    "name": "OpenRouter",
                    "models": [
                        {"id": "anthropic/claude-opus-4-5", "name": "Claude via OR", "limit": {"context": 200000, "output": 32000}, "cost": {"input": 6, "output": 30}},
                        {"id": "meta/llama-4", "name": "Llama", "reasoning": false, "limit": {"context": 128000, "output": 8192}}
                    ]
                },
                {
                    "id": "relay",
                    "name": "Relay",
                    "models": [
                        {"id": "claude-opus-4-5", "name": "Claude Relay", "limit": {"context": 200000, "output": 32000}, "cost": {"input": 1, "output": 4}}
                    ]
                }
            ]
        }))
        .unwrap();
        Catalog::parse(&bytes, super::super::models_dev::Origin::Runtime).unwrap()
    }

    fn input_for<'a>(raw: &'a str) -> ResolutionRequest<'a> {
        ResolutionRequest {
            raw_model_id: raw,
            display_name: "",
            catalog_providers: &[],
            price_providers: &[],
            explicit_ref: None,
            price_ref: None,
            user: UserValues::default(),
            provider_context: None,
            official: None,
            upstream: None,
        }
    }

    #[test]
    fn candidate_identity_is_explicit_and_version_preserving() {
        let steps = candidates("openai/GPT-6.1-SOL:free");
        let values: Vec<&str> = steps.iter().map(|step| step.value.as_str()).collect();
        assert_eq!(values[0], "openai/GPT-6.1-SOL:free");
        let step = |needle: &str| {
            steps
                .iter()
                .find(|step| step.value == needle)
                .map(|step| step.step)
        };
        assert!(values.contains(&"openai/GPT-6.1-SOL"));
        assert!(values.contains(&"GPT-6.1-SOL"));
        assert!(values.contains(&"GPT-6.1-SOL:free"));
        assert_eq!(step("GPT-6.1-SOL"), Some("pathPrefix"));
        assert_eq!(step("openai/GPT-6.1-SOL"), Some("tagSuffix"));
        assert_eq!(step("GPT-6.1-SOL:free"), Some("pathPrefix"));

        // Versions, dates and preview suffixes are never removed.
        for raw in ["gpt-5.5", "model-preview", "model-20261001"] {
            let values: Vec<String> = candidates(raw).into_iter().map(|step| step.value).collect();
            assert_eq!(values, vec![raw.to_owned()], "{raw} must stay intact");
        }
        // Known markers are removed with a trace.
        let values: Vec<String> = candidates("channel__claude-opus-5-5-kiro@fast")
            .into_iter()
            .map(|step| step.value)
            .collect();
        assert!(values.contains(&"channel__claude-opus-5-5-kiro".to_owned()));
        assert!(values.contains(&"claude-opus-5-5-kiro".to_owned()));
        assert!(values.contains(&"claude-opus-5-5".to_owned()));
        // Bedrock region prefixes only with a provider namespace behind them.
        let values: Vec<String> = candidates("apac.anthropic.claude-opus-4-5")
            .into_iter()
            .map(|step| step.value)
            .collect();
        assert!(values.contains(&"claude-opus-4-5".to_owned()));
        let values: Vec<String> = candidates("us.gpt-5")
            .into_iter()
            .map(|step| step.value)
            .collect();
        assert!(!values.contains(&"gpt-5".to_owned()));
        // Effort suffixes are recognized, unknown parentheses are not.
        let values: Vec<String> = candidates("gpt-5.5(high)")
            .into_iter()
            .map(|step| step.value)
            .collect();
        assert!(values.contains(&"gpt-5.5".to_owned()));
        let values: Vec<String> = candidates("gpt-5.5(2025-11)")
            .into_iter()
            .map(|step| step.value)
            .collect();
        assert!(!values.contains(&"gpt-5.5".to_owned()));
    }

    #[test]
    fn reasoning_variants_keep_exact_records_and_do_not_borrow_base_prices() {
        let catalog = Catalog::parse(serde_json::json!({
            "schemaVersion": 1,
            "providers": [{"id": "google", "models": [
                {"id": "gemini-3.8-flash", "limit": {"context": 1000000}, "cost": {"input": 1, "output": 2}},
                {"id": "gemini-3.8-flash-high", "limit": {"context": 400000}, "cost": {"input": 9, "output": 10}}
            ]}]
        }).to_string().as_bytes(), super::super::models_dev::Origin::Embedded).unwrap();
        let scope = ["google".to_owned()];
        let mut exact = input_for("gemini-3.8-flash-high");
        exact.catalog_providers = &scope;
        exact.price_providers = &scope;
        let resolved = resolve(&catalog, &exact);
        assert_eq!(resolved.match_kind, MatchKind::Exact);
        assert_eq!(resolved.context_window.value, Some(400000));
        assert_eq!(resolved.price.input, Some(9.0));

        for raw in ["gemini-3.8-flash-thinking", "gemini-3.8-flash(high)"] {
            let mut request = input_for(raw);
            request.catalog_providers = &scope;
            request.price_providers = &scope;
            let resolved = resolve(&catalog, &request);
            assert_eq!(resolved.match_kind, MatchKind::Normalized);
            assert_eq!(resolved.raw_model_id, raw);
            assert_eq!(resolved.context_window.value, Some(1000000));
            assert_eq!(resolved.price.source, source::UNKNOWN);
        }
        for raw in [
            "gemini-3.9-flash-high",
            "gemini-3.8-flash-preview",
            "gemini-3.8-flash-20261002",
            "gemini-3.8-flash-vision",
        ] {
            let mut request = input_for(raw);
            request.catalog_providers = &scope;
            let resolved = resolve(&catalog, &request);
            assert!(
                resolved.matched.is_none(),
                "{raw} must not reduce to another version or variant"
            );
        }
    }

    #[test]
    fn gemini_preview_references_keep_versions_variants_and_prices_separate() {
        let catalog = Catalog::parse(json!({
            "schemaVersion": 1,
            "providers": [
                {"id": "google", "models": [
                    {"id": "gemini-3.1-pro-preview", "limit": {"context": 1048576}, "cost": {"input": 2, "output": 12}},
                    {"id": "gemini-3.1-pro-preview-customtools", "limit": {"context": 2000}},
                    {"id": "gemini-3.2-flash-preview", "status": "deprecated", "limit": {"context": 3000}},
                    {"id": "gemini-3.3-pro", "limit": {"context": 4000}},
                    {"id": "gemini-3.3-pro-preview", "limit": {"context": 5000}},
                    {"id": "gemini-3.3-pro-high", "limit": {"context": 6000}}
                ]},
                {"id": "302ai", "models": [
                    {"id": "gemini-3.1-pro", "limit": {"context": 9999}},
                    {"id": "gemini-3.4-pro-preview", "limit": {"context": 9999}}
                ]}
            ]
        }).to_string().as_bytes(), super::super::models_dev::Origin::Embedded).unwrap();
        let scope = ["google".to_owned()];
        for raw in [
            "gemini-3.1-pro",
            "gemini-3.1-pro-high",
            "gemini-3.1-pro-thinking-high",
            "channel/gemini-3.1-pro_high",
            "channel::gemini-3.1-pro@high",
            "gemini-3.1-pro(high)",
            "GEMINI-3.1-PRO-HIGH",
        ] {
            let mut request = input_for(raw);
            request.catalog_providers = &scope;
            request.price_providers = &scope;
            let resolved = resolve_with_global_fallback(&catalog, &request, false, None);
            assert_eq!(resolved.match_kind, MatchKind::Normalized, "{raw}");
            assert_eq!(resolved.raw_model_id, raw);
            let matched = resolved.matched.unwrap();
            assert_eq!(matched.provider_id, "google");
            assert_eq!(matched.model_id, "gemini-3.1-pro-preview");
            assert_eq!(resolved.context_window.value, Some(1048576));
            assert_eq!(resolved.price.source, source::UNKNOWN);
            assert_eq!(resolved.steps.last().unwrap().step, "previewReference");
        }
        for (raw, expected) in [
            ("gemini-3.3-pro-high", "gemini-3.3-pro-high"),
            ("gemini-3.3-pro-thinking", "gemini-3.3-pro"),
            ("gemini-3.1-pro-preview", "gemini-3.1-pro-preview"),
            (
                "gemini-3.1-pro-preview-customtools",
                "gemini-3.1-pro-preview-customtools",
            ),
        ] {
            let mut request = input_for(raw);
            request.catalog_providers = &scope;
            let resolved = resolve_with_global_fallback(&catalog, &request, false, None);
            assert_eq!(resolved.matched.unwrap().model_id, expected);
            assert!(!resolved
                .steps
                .iter()
                .any(|step| step.step == "previewReference"));
        }
        for raw in [
            "gemini-3.2-pro-high",
            "gemini-3.1-pro-vision",
            "gemini-3.1-pro-20261002",
            "gemini-3.1-pro-customtools",
            "gemini-3.1-pro-unknown",
            "gemini-3.2-flash",
            "gemini-3.4-pro",
            "claude-3.1-pro",
        ] {
            let mut request = input_for(raw);
            request.catalog_providers = &scope;
            let resolved = resolve_with_global_fallback(&catalog, &request, false, None);
            assert!(
                resolved.matched.is_none(),
                "{raw} must not guess another identity"
            );
        }
        // An explicit reference failure must not silently become a suggestion.
        let missing = ModelRef {
            provider_id: "google".into(),
            model_id: "missing".into(),
        };
        let mut request = input_for("gemini-3.1-pro-high");
        request.catalog_providers = &scope;
        request.explicit_ref = Some(&missing);
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.match_kind, MatchKind::Invalid);
        assert!(resolved.matched.is_none());
        request.explicit_ref = None;
        request.raw_model_id = "gemini-3.4-pro-high";
        let relay_scope = ["302ai".to_owned()];
        request.catalog_providers = &relay_scope;
        assert!(
            resolve_with_global_fallback(&catalog, &request, false, None)
                .matched
                .is_none()
        );
    }

    #[test]
    fn gemini_preview_reference_keeps_active_case_conflicts_visible() {
        let catalog = Catalog::parse(json!({
            "schemaVersion": 1,
            "providers": [{"id": "google", "models": [
                {"id": "gemini-3.1-pro-preview", "limit": {"context": 1000}},
                {"id": "GEMINI-3.1-PRO-PREVIEW", "limit": {"context": 2000}},
                {"id": "Gemini-3.1-pro-preview", "status": "deprecated", "limit": {"context": 3000}}
            ]}]
        }).to_string().as_bytes(), super::super::models_dev::Origin::Embedded).unwrap();
        let scope = ["google".to_owned()];
        let mut request = input_for("gemini-3.1-pro-high");
        request.catalog_providers = &scope;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.match_kind, MatchKind::Ambiguous);
        assert_eq!(resolved.context_window.value, None);
        assert_eq!(resolved.steps.last().unwrap().matched.len(), 2);
        assert!(resolved
            .conflicts
            .iter()
            .any(|conflict| conflict.field == "contextWindow"));
    }

    #[test]
    fn canonical_aliases_keep_conflicting_active_records_visible() {
        let catalog = Catalog::parse(serde_json::json!({
            "schemaVersion": 1,
            "providers": [{"id": "deepseek", "models": [
                {"id": "a", "canonicalModelId": "deepseek/deepseek-v4.1-flash", "limit": {"context": 1000}},
                {"id": "b", "canonicalModelId": "deepseek/deepseek-v4.1-flash", "limit": {"context": 2000}}
            ]}]
        }).to_string().as_bytes(), super::super::models_dev::Origin::Embedded).unwrap();
        let scope = ["deepseek".to_owned()];
        let mut request = input_for("deepseek-v4.1-flash");
        request.catalog_providers = &scope;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.match_kind, MatchKind::Ambiguous);
        assert_eq!(resolved.context_window.value, None);
        assert_eq!(resolved.steps[0].step, "canonicalIdentity");
        assert_eq!(resolved.steps[0].matched.len(), 2);
        assert!(resolved
            .conflicts
            .iter()
            .any(|conflict| conflict.field == "contextWindow"));
    }

    #[test]
    fn scoped_and_generic_matches_carry_provenance_and_conflicts() {
        let catalog = catalog();
        let scoped_ids = ["openrouter".to_owned()];
        let mut scoped = input_for("openrouter/anthropic/claude-opus-4-5");
        scoped.catalog_providers = &scoped_ids;
        scoped.price_providers = &scoped_ids;
        let resolved = resolve(&catalog, &scoped);
        assert_eq!(resolved.match_kind, MatchKind::Normalized);
        assert_eq!(resolved.context_window.value, Some(200_000));
        assert_eq!(resolved.context_window.source, source::CATALOG);
        assert_eq!(resolved.price.input, Some(6.0));
        assert_eq!(resolved.matched.as_ref().unwrap().provider_id, "openrouter");

        // Generic matching: no scope, values agree across providers → filled
        // with the generic source.
        let generic = resolve(&catalog, &input_for("claude-opus-4-5"));
        assert_eq!(generic.match_kind, MatchKind::Consensus);
        assert_eq!(generic.context_window.value, Some(200_000));
        assert_eq!(generic.context_window.source, source::CATALOG_GENERIC);
        // Prices never cross the provider boundary.
        assert_eq!(generic.price.source, source::UNKNOWN);
        assert!(generic.price.input.is_none());
    }

    #[test]
    fn conflicting_catalogs_keep_agreeing_fields_and_report_the_rest() {
        let catalog = catalog();
        let scoped_ids = ["anthropic".to_owned(), "relay".to_owned()];
        let mut request = input_for("claude-opus-4-5");
        request.catalog_providers = &scoped_ids;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.match_kind, MatchKind::Ambiguous);
        // Context agrees (200000 in both) so it is kept.
        assert_eq!(resolved.context_window.value, Some(200_000));
        // Output differs (64000 vs 32000) so it stays unknown with a conflict.
        assert!(resolved.output_limit.value.is_none());
        assert!(resolved
            .conflicts
            .iter()
            .any(|conflict| conflict.field == "outputLimit" && conflict.candidates.len() >= 2));
    }

    #[test]
    fn user_values_and_provider_settings_win_over_the_catalog() {
        let catalog = catalog();
        let scoped_ids = ["anthropic".to_owned()];
        let mut request = input_for("claude-opus-4-5-kiro");
        request.catalog_providers = &scoped_ids;
        request.user.context_window = Some(99_000);
        request.user.reasoning = Some(vec![ReasoningLevel {
            effort: "high".into(),
            description: "High".into(),
        }]);
        request.user.default_effort = Some("high".into());
        request.provider_context = Some(50_000);
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.context_window.value, Some(99_000));
        assert_eq!(resolved.context_window.source, source::MANUAL);
        // The provider clamp still bounds non-manual outputs.
        assert_eq!(resolved.output_limit.value, Some(50_000));
        assert_eq!(resolved.output_limit.source, source::PROVIDER_SETTING);
        assert_eq!(resolved.reasoning.levels.value.as_ref().unwrap().len(), 1);
        assert_eq!(resolved.reasoning.levels.source, source::MANUAL);
        assert_eq!(resolved.default_effort.as_deref(), Some("high"));
        assert_eq!(resolved.default_effort_source, source::MANUAL);
    }

    #[test]
    fn explicit_references_fail_closed_and_legacy_upstream_values_are_ignored() {
        let catalog = catalog();
        let missing = ModelRef {
            provider_id: "anthropic".into(),
            model_id: "claude-opus-9".into(),
        };
        let mut request = input_for("claude-opus-4-5");
        request.explicit_ref = Some(&missing);
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.match_kind, MatchKind::Invalid);
        assert!(resolved.context_window.value.is_none());

        let scoped_ids = ["anthropic".to_owned()];
        let mut legacy = input_for("claude-opus-4-5");
        legacy.catalog_providers = &scoped_ids;
        legacy.upstream = Some(UpstreamValues {
            context_window: Some(777),
            max_output_tokens: Some(555),
            legacy: true,
            ..Default::default()
        });
        let resolved = resolve(&catalog, &legacy);
        assert_eq!(resolved.context_window.value, Some(200_000));
        assert_eq!(resolved.context_window.source, source::CATALOG);
    }

    #[test]
    fn reasoning_sources_never_invent_levels_for_toggle_models() {
        let catalog = catalog();
        let scoped_ids = ["anthropic".to_owned()];
        let mut request = input_for("claude-sonnet-4-5");
        request.catalog_providers = &scoped_ids;
        let resolved = resolve(&catalog, &request);
        let levels = resolved.reasoning.levels.value.clone().unwrap();
        assert!(
            levels.is_empty(),
            "toggle-only models have no discrete efforts"
        );
        assert_eq!(resolved.reasoning.supports, Some(true));
        assert!(resolved
            .reasoning
            .options
            .iter()
            .any(|option| option.kind == "toggle"));
        assert!(resolved.default_effort.is_none());

        // An explicitly unsupported model reports supports=false and no levels.
        let mut unsupported = input_for("meta/llama-4");
        unsupported.catalog_providers = &[];
        let resolved = resolve(&catalog, &unsupported);
        assert_eq!(resolved.reasoning.supports, Some(false));
        assert!(resolved.reasoning.levels.value.as_ref().unwrap().is_empty());
    }

    #[test]
    fn tag_prices_are_not_borrowed_from_the_base_model() {
        let catalog = catalog();
        let scoped_ids = ["openrouter".to_owned()];
        let mut request = input_for("anthropic/claude-opus-4-5:free");
        request.catalog_providers = &scoped_ids;
        request.price_providers = &scoped_ids;
        let resolved = resolve(&catalog, &request);
        // Capability still resolves through the tag-stripped candidate.
        assert_eq!(resolved.context_window.value, Some(200_000));
        // Price does not silently fall back to the base model's rates.
        assert_eq!(resolved.price.source, source::UNKNOWN);
        assert!(resolved.price.input.is_none());
    }

    #[test]
    fn prices_stay_with_the_billing_provider_scope() {
        let catalog = catalog();
        // Capability from anthropic, price bound to the relay: the borrowed
        // capability must not drag anthropic's price along.
        let capability = ["anthropic".to_owned()];
        let billing = ["relay".to_owned()];
        let mut request = input_for("claude-opus-4-5");
        request.catalog_providers = &capability;
        request.price_providers = &billing;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.context_window.value, Some(200_000));
        assert_eq!(resolved.context_window.source, source::CATALOG);
        assert_eq!(
            resolved.price.input,
            Some(1.0),
            "relay price, not anthropic's 5"
        );
        // The full identity matched inside the billing scope: a plain catalog
        // price, not an alias fallback.
        assert_eq!(resolved.price.source, source::CATALOG);

        // A reduced candidate inside the same scope keeps the alias label.
        let mut request = input_for("claude-opus-4-5-kiro");
        request.catalog_providers = &capability;
        request.price_providers = &billing;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.price.input, Some(1.0));
        assert_eq!(resolved.price.source, source::CATALOG_ALIAS);

        // A capability binding alone never establishes billing prices.
        let mut request = input_for("claude-opus-4-5");
        request.catalog_providers = &capability;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.price.input, None);
        assert_eq!(resolved.price.source, source::UNKNOWN);
    }

    #[test]
    fn conflicting_prices_inside_one_scope_are_never_averaged() {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sourceUrl": "https://models.dev/api.json",
            "fetchedAt": 1_700_000_000,
            "revision": "fixture",
            "providers": [
                {"id": "relay-a", "name": "A", "models": [
                    {"id": "model-x", "name": "X", "limit": {"context": 1000, "output": 100}, "cost": {"input": 1.0, "output": 2.0}}
                ]},
                {"id": "relay-b", "name": "B", "models": [
                    {"id": "model-x", "name": "X", "limit": {"context": 1000, "output": 100}, "cost": {"input": 9.0, "output": 2.0}}
                ]}
            ]
        }))
        .unwrap();
        let catalog = Catalog::parse(&bytes, super::super::models_dev::Origin::Runtime).unwrap();
        let scope = ["relay-a".to_owned(), "relay-b".to_owned()];
        let mut request = input_for("model-x");
        request.catalog_providers = &scope;
        request.price_providers = &scope;
        let resolved = resolve(&catalog, &request);
        // Capability agrees, price does not: the price stays unknown.
        assert_eq!(resolved.context_window.value, Some(1000));
        assert_eq!(resolved.price.source, source::UNKNOWN);
        assert!(resolved.price.input.is_none());
    }

    #[test]
    fn zero_prices_stay_zero_instead_of_unknown() {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sourceUrl": "https://models.dev/api.json",
            "fetchedAt": 1_700_000_000,
            "revision": "fixture",
            "providers": [{
                "id": "relay-a",
                "name": "A",
                "models": [{
                    "id": "free-model",
                    "name": "Free",
                    "limit": {"context": 1000, "output": 100},
                    "cost": {"input": 0, "output": 0}
                }]
            }]
        }))
        .unwrap();
        let catalog = Catalog::parse(&bytes, super::super::models_dev::Origin::Runtime).unwrap();
        let scope = ["relay-a".to_owned()];
        let mut request = input_for("free-model");
        request.catalog_providers = &scope;
        request.price_providers = &scope;
        let resolved = resolve(&catalog, &request);
        // A declared zero rate is a price, not a missing one.
        assert_eq!(resolved.price.input, Some(0.0));
        assert_eq!(resolved.price.output, Some(0.0));
        assert_ne!(resolved.price.source, source::UNKNOWN);
        assert!(resolved.price.complete);

        // A user override of zero behaves the same way.
        let mut request = input_for("free-model");
        request.catalog_providers = &scope;
        request.user.price = Some(CostRatesInput {
            input: Some(0.0),
            output: Some(0.0),
            ..Default::default()
        });
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.price.source, source::MANUAL);
        assert_eq!(resolved.price.input, Some(0.0));
    }

    #[test]
    fn conditional_prices_are_marked_incomplete() {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sourceUrl": "https://models.dev/api.json",
            "fetchedAt": 1_700_000_000,
            "revision": "fixture",
            "providers": [{
                "id": "deepinfra",
                "name": "DeepInfra",
                "models": [{
                    "id": "qwen3-max",
                    "name": "Qwen3 Max",
                    "limit": {"context": 128000, "output": 32768},
                    "cost": {
                        "input": 2.4, "output": 12,
                        "tiers": [{"input": 3, "output": 15, "tier": {"type": "context", "size": 128000}}]
                    }
                }]
            }]
        }))
        .unwrap();
        let catalog = Catalog::parse(&bytes, super::super::models_dev::Origin::Runtime).unwrap();
        let scoped_ids = ["deepinfra".to_owned()];
        let mut request = input_for("qwen3-max");
        request.catalog_providers = &scoped_ids;
        request.price_providers = &scoped_ids;
        let resolved = resolve(&catalog, &request);
        assert_eq!(resolved.price.input, Some(2.4));
        assert!(!resolved.price.complete);
        assert_eq!(resolved.price.tiers.len(), 1);
    }
    #[test]
    fn product_max_and_ultra_are_not_effort_aliases() {
        for id in ["gpt-5.1-codex-max", "imagen-4-ultra", "model_MAX"] {
            assert_eq!(candidates(id).iter().map(|c| c.value.as_str()).collect::<Vec<_>>(), vec![id]);
        }
        for id in ["gpt-5(max)", "gpt-5:max", "gpt-5@ultra"] {
            assert!(candidates(id).iter().any(|c| c.value == "gpt-5"));
        }
    }

}
