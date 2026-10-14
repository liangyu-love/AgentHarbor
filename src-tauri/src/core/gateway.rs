//! The general gateway: one local URL and named keys through which any agent
//! reaches every enabled relay model, shared account source and aggregate,
//! each under its own id, with the router's protocol translation.
//!
//! Model ids are `<provider short name>/<model>` for providers and
//! `<aggregate prefix>/<public id>` for aggregates (`pool/gpt-5.5` beside a
//! relay's `relay-a/gpt-5.5`). The short name is assigned once from the
//! provider's name (or its host) and kept through renames; aggregates share
//! one prefix unless one names its own, and no provider may take a prefix.
use super::{aggregation, models_dev, router, router::Store};
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use serde_json::{json, Value};

const MAX_SLUG: usize = 40;
/// What aggregates' gateway ids start with until the user picks another.
pub const DEFAULT_AGGREGATE_PREFIX: &str = "pool";
const MAX_KEYS: usize = 32;
const MAX_KEY_NAME: usize = 40;

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches(|ch| ch == '-' || ch == '.' || ch == '_');
    slug.chars().take(MAX_SLUG).collect::<String>().trim_end_matches(['-', '.', '_']).to_owned()
}

/// The registrable part of a host: `api.siliconflow.cn` → `siliconflow`.
fn host_label(base_url: &str) -> String {
    let host = reqwest::Url::parse(base_url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" || bare.parse::<std::net::IpAddr>().is_ok() {
        return "local".into();
    }
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    let pick = match labels.as_slice() {
        [.., name, "com" | "net" | "org" | "cn" | "ai" | "io" | "dev" | "app" | "co", _] if labels.len() > 2 => *name,
        [.., name, _] => *name,
        [only] => *only,
        [] => "",
    };
    slugify(pick)
}

/// Whether `slug` is a valid short name.
pub fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    slug.len() <= MAX_SLUG
        && chars.next().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '.' | '_'))
}

/// The shared prefix of aggregates' gateway ids.
pub fn shared_prefix(store: &Store) -> &str {
    let prefix = store.router.gateway_aggregate_prefix.trim();
    if prefix.is_empty() { DEFAULT_AGGREGATE_PREFIX } else { prefix }
}

/// The prefix of one aggregate's gateway id: its own, else the shared one.
pub fn aggregate_prefix<'a>(store: &'a Store, aggregate: &'a aggregation::AggregateModel) -> &'a str {
    aggregate.gateway_prefix.as_deref().map(str::trim).filter(|prefix| !prefix.is_empty()).unwrap_or_else(|| shared_prefix(store))
}

/// An aggregate's gateway id.
pub fn aggregate_id(store: &Store, aggregate: &aggregation::AggregateModel) -> String {
    format!("{}/{}", aggregate_prefix(store, aggregate), aggregate.public_model_id)
}

/// Whether `slug` is an aggregate prefix (the shared one or an aggregate's own).
fn is_prefix(store: &Store, slug: &str) -> bool {
    shared_prefix(store) == slug || store.aggregates.iter().any(|aggregate| aggregate_prefix(store, aggregate) == slug)
}

/// Whether `prefix` may be an aggregate prefix: a valid short name no
/// provider uses.
pub(crate) fn check_prefix(store: &Store, prefix: &str) -> Result<(), String> {
    if !valid_slug(prefix) {
        return Err("gateway.errors.prefix".into());
    }
    if store.providers.iter().any(|provider| provider.gateway_slug.as_deref() == Some(prefix)) {
        return Err("gateway.errors.prefixTaken".into());
    }
    Ok(())
}

/// Set the shared aggregate prefix; empty restores the default.
pub fn set_shared_prefix(paths: &CodexPaths, prefix: &str) -> Result<(), String> {
    let prefix = prefix.trim();
    let prefix = if prefix.is_empty() { DEFAULT_AGGREGATE_PREFIX } else { prefix }.to_owned();
    router::mutate_store(paths, move |store| {
        check_prefix(store, &prefix)?;
        store.router.gateway_aggregate_prefix = prefix;
        Ok(())
    }).map(|_| ())
}

/// A free short name for `provider`, preferring `wanted`.
fn free_slug(store: &Store, provider_id: &str, wanted: &str) -> String {
    let base = if wanted.is_empty() { "relay".to_owned() } else { wanted.to_owned() };
    let taken = |candidate: &str| is_prefix(store, candidate) || store.providers.iter()
        .any(|other| other.id != provider_id && other.gateway_slug.as_deref() == Some(candidate));
    if !taken(&base) {
        return base;
    }
    (2..).map(|index| {
        let suffix = format!("-{index}");
        format!("{}{suffix}", base.chars().take(MAX_SLUG - suffix.len()).collect::<String>())
    }).find(|candidate| !taken(candidate)).expect("an unused suffix")
}

/// The short name a provider gets by default: its account source's fixed
/// name, else its name, else its host.
fn default_slug(provider: &router::Provider) -> String {
    if let Some(kind) = provider.account_source {
        return match kind {
            super::account_sources::SourceKind::ChatgptApi => "chatgpt",
            super::account_sources::SourceKind::Workbuddy => "workbuddy",
            super::account_sources::SourceKind::WorkbuddyAi => "workbuddy-ai",
            super::account_sources::SourceKind::Copilot => "copilot",
            super::account_sources::SourceKind::GeminiCli => "gemini-cli",
            super::account_sources::SourceKind::Antigravity => "antigravity",
            super::account_sources::SourceKind::Kiro => "kiro",
            super::account_sources::SourceKind::Grok => "grok",
            super::account_sources::SourceKind::Cursor => "cursor",
        }.to_owned();
    }
    let from_name = slugify(&provider.name);
    if from_name.is_empty() { host_label(&provider.base_url) } else { from_name }
}

/// Give every provider without a short name one. True when anything changed.
pub(crate) fn assign_slugs(store: &mut Store) -> bool {
    let mut changed = false;
    for index in 0..store.providers.len() {
        let current = store.providers[index].gateway_slug.clone();
        let keep = current.as_deref().is_some_and(|slug| valid_slug(slug) && !is_prefix(store, slug) && !store.providers.iter()
            .take(index).any(|earlier| earlier.gateway_slug.as_deref() == Some(slug)));
        if keep {
            continue;
        }
        let slug = free_slug(store, &store.providers[index].id.clone(), &default_slug(&store.providers[index]));
        store.providers[index].gateway_slug = Some(slug);
        changed = true;
    }
    changed
}

/// Whether a user-chosen short name may be given to `provider_id` (`None`
/// for a provider not saved yet). Empty means "keep".
pub(crate) fn check_slug(store: &Store, provider_id: Option<&str>, wanted: Option<&str>) -> Result<(), String> {
    let Some(wanted) = wanted.map(str::trim).filter(|wanted| !wanted.is_empty()) else { return Ok(()) };
    if !valid_slug(wanted) {
        return Err("newapi.errors.gatewaySlug".into());
    }
    if store.providers.iter().any(|other| Some(other.id.as_str()) != provider_id && other.gateway_slug.as_deref() == Some(wanted)) {
        return Err("newapi.errors.gatewaySlugTaken".into());
    }
    if is_prefix(store, wanted) {
        return Err("newapi.errors.gatewaySlugIsPrefix".into());
    }
    Ok(())
}

/// Apply a user-chosen short name: empty keeps the current one.
pub(crate) fn set_slug(store: &mut Store, provider_id: &str, wanted: Option<&str>) -> Result<(), String> {
    check_slug(store, Some(provider_id), wanted)?;
    let Some(wanted) = wanted.map(str::trim).filter(|wanted| !wanted.is_empty()) else { return Ok(()) };
    let provider = store.providers.iter_mut().find(|provider| provider.id == provider_id).ok_or("newapi.errors.notFound")?;
    provider.gateway_slug = Some(wanted.to_owned());
    Ok(())
}

// ---- keys --------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub id: String,
    pub name: String,
    /// The key with its middle hidden.
    pub masked: String,
    pub created_at: i64,
}

fn mask(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("sk-ah-…{tail}")
}

fn new_key() -> String {
    format!("sk-ah-{}{}", uuid::Uuid::new_v4().simple(), &uuid::Uuid::new_v4().simple().to_string()[..16])
}

fn key_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_KEY_NAME {
        return Err("gateway.errors.keyName".into());
    }
    Ok(name.to_owned())
}

pub fn key_views(store: &Store) -> Vec<KeyView> {
    store.router.gateway_keys.iter().map(|key| KeyView {
        id: key.id.clone(), name: key.name.clone(), masked: mask(&key.key), created_at: key.created_at,
    }).collect()
}

/// Add a named key. Returns its id.
pub fn create_key(paths: &CodexPaths, name: &str) -> Result<String, String> {
    let name = key_name(name)?;
    let id = uuid::Uuid::new_v4().to_string();
    let created = id.clone();
    router::mutate_store(paths, move |store| {
        if store.router.gateway_keys.len() >= MAX_KEYS {
            return Err("gateway.errors.tooManyKeys".into());
        }
        if store.router.gateway_keys.iter().any(|key| key.name == name) {
            return Err("gateway.errors.keyNameTaken".into());
        }
        store.router.gateway_keys.push(router::GatewayKey {
            id: created, name, key: new_key(), created_at: chrono::Utc::now().timestamp(),
        });
        Ok(())
    })?;
    Ok(id)
}

pub fn rename_key(paths: &CodexPaths, id: &str, name: &str) -> Result<(), String> {
    let name = key_name(name)?;
    router::mutate_store(paths, |store| {
        if store.router.gateway_keys.iter().any(|key| key.id != id && key.name == name) {
            return Err("gateway.errors.keyNameTaken".into());
        }
        store.router.gateway_keys.iter_mut().find(|key| key.id == id).ok_or("gateway.errors.keyNotFound")?.name = name;
        Ok(())
    }).map(|_| ())
}

/// Replace a key's secret; agents holding the old one are refused from now on.
pub fn regenerate_key(paths: &CodexPaths, id: &str) -> Result<(), String> {
    router::mutate_store(paths, |store| {
        store.router.gateway_keys.iter_mut().find(|key| key.id == id).ok_or("gateway.errors.keyNotFound")?.key = new_key();
        Ok(())
    }).map(|_| ())
}

pub fn delete_key(paths: &CodexPaths, id: &str) -> Result<(), String> {
    router::mutate_store(paths, |store| {
        let before = store.router.gateway_keys.len();
        store.router.gateway_keys.retain(|key| key.id != id);
        if store.router.gateway_keys.len() == before {
            return Err("gateway.errors.keyNotFound".into());
        }
        Ok(())
    }).map(|_| ())
}

pub fn reveal_key(store: &Store, id: &str) -> Result<String, String> {
    store.router.gateway_keys.iter().find(|key| key.id == id).map(|key| key.key.clone())
        .ok_or_else(|| "gateway.errors.keyNotFound".into())
}

fn constant_eq(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    left.len() == right.len() && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

/// The key a request presents (Bearer or `x-api-key`), when it is one of ours.
pub fn authenticate<'a>(store: &'a Store, bearer: Option<&str>, api_key: Option<&str>) -> Option<&'a router::GatewayKey> {
    let presented = bearer.or(api_key).map(str::trim).filter(|value| !value.is_empty())?;
    store.router.gateway_keys.iter().find(|key| constant_eq(&key.key, presented))
}

// ---- models ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    Relay,
    Account,
    Aggregate,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayModel {
    pub id: String,
    pub kind: ModelKind,
    pub source_name: String,
    /// The provider, or the aggregate's id.
    pub source_id: String,
    pub model_id: String,
    pub label: String,
    pub protocol: router::ModelProtocol,
    pub context_window: Option<u64>,
    /// The thinking levels the model offers, in order; none when it has none.
    #[serde(skip)]
    pub reasoning: Vec<String>,
    #[serde(skip)]
    pub default_reasoning: Option<String>,
    #[serde(skip)]
    pub output_limit: Option<u64>,
    /// The account source behind the model, if it is one.
    #[serde(skip)]
    pub account_source: Option<super::account_sources::SourceKind>,
}

/// The levels of a client projection, by name.
fn levels(projection: &super::model_policy::ClientProjection) -> Vec<String> {
    projection.reasoning.iter().filter_map(|level| level["effort"].as_str().map(str::to_owned)).collect()
}

/// What a gateway model id names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Provider { provider_id: String, model_id: String },
    Aggregate { public_id: String },
}

fn served_models(provider: &router::Provider) -> Vec<&str> {
    let mut ids: Vec<&str> = Vec::new();
    for id in std::iter::once(provider.model.as_str())
        .chain(provider.models.iter().filter(|model| model.included).map(|model| model.id.as_str()))
    {
        if !id.is_empty() && !ids.contains(&id) && provider.can_route_model(id) {
            ids.push(id);
        }
    }
    ids
}

fn admits(sources: router::ModelSources, kind: ModelKind) -> bool {
    match kind {
        ModelKind::Relay => sources.relay,
        ModelKind::Account => sources.account,
        ModelKind::Aggregate => sources.aggregate,
    }
}

/// The models the gateway serves: those of the kinds it has turned on.
pub fn models(store: &Store, catalog: &models_dev::Catalog) -> Vec<GatewayModel> {
    let sources = store.router.gateway_sources;
    all_models(store, catalog).into_iter().filter(|model| admits(sources, model.kind)).collect()
}

/// Turn the kinds of model the gateway serves on or off.
pub fn set_sources(paths: &CodexPaths, sources: router::ModelSources) -> Result<(), String> {
    router::mutate_store(paths, move |store| {
        store.router.gateway_sources = sources;
        Ok(())
    }).map(|_| ())
}

/// Every model the gateway could serve, whatever kinds are turned on:
/// providers in their saved order, then aggregates.
pub fn all_models(store: &Store, catalog: &models_dev::Catalog) -> Vec<GatewayModel> {
    let mut out = Vec::new();
    for provider in &store.providers {
        let Some(slug) = provider.gateway_slug.as_deref() else { continue };
        if !provider.has_route_credentials() {
            continue;
        }
        for model_id in served_models(provider) {
            let (_, projection) = router::model_projection(provider, model_id, None, catalog, &store.global_compression());
            let label = provider.models.iter().find(|model| model.id == model_id)
                .map(|model| model.display_name.trim()).filter(|name| !name.is_empty()).unwrap_or(model_id);
            out.push(GatewayModel {
                id: format!("{slug}/{model_id}"),
                kind: if provider.router_only() { ModelKind::Account } else { ModelKind::Relay },
                source_name: provider.name.clone(),
                source_id: provider.id.clone(),
                model_id: model_id.to_owned(),
                label: label.to_owned(),
                protocol: provider.protocol_for_model(model_id),
                context_window: (!projection.context_is_fallback).then_some(projection.context_window),
                reasoning: levels(&projection),
                default_reasoning: projection.default_reasoning.clone(),
                output_limit: projection.output_limit,
                account_source: provider.account_source,
            });
        }
    }
    for aggregate in store.aggregates.iter().filter(|aggregate| aggregate.enabled) {
        let view = aggregation::project(store, aggregate, catalog);
        if !view.available {
            continue;
        }
        out.push(GatewayModel {
            id: aggregate_id(store, aggregate),
            kind: ModelKind::Aggregate,
            source_name: aggregate.display_name.clone(),
            source_id: aggregate.id.clone(),
            model_id: aggregate.public_model_id.clone(),
            label: aggregate.display_name.clone(),
            protocol: router::ModelProtocol::Responses,
            context_window: view.projection.as_ref().map(|projection| projection.context_window),
            reasoning: view.projection.as_ref().map(levels).unwrap_or_default(),
            default_reasoning: view.projection.as_ref().and_then(|projection| projection.default_reasoning.clone()),
            output_limit: view.projection.as_ref().and_then(|projection| projection.output_limit),
            account_source: None,
        });
    }
    out
}

/// Resolve a gateway model id: an enabled aggregate's `<prefix>/<public id>`
/// first, then a provider's `<short name>/<model>` (the model itself possibly
/// containing `/`), then an aggregate's bare public id, which clients set up
/// before aggregates had a prefix still send.
pub fn resolve(store: &Store, model: &str) -> Option<Target> {
    resolve_with(store, model, store.router.gateway_sources)
}

/// [`resolve`] against the kinds `sources` turns on rather than the gateway's.
pub fn resolve_with(store: &Store, model: &str, sources: router::ModelSources) -> Option<Target> {
    let model = model.trim();
    let enabled = || store.aggregates.iter().filter(|aggregate| sources.aggregate && aggregate.enabled);
    if let Some(aggregate) = enabled().find(|aggregate| aggregate_id(store, aggregate) == model) {
        return Some(Target::Aggregate { public_id: aggregate.public_model_id.clone() });
    }
    let provider = model.split_once('/').and_then(|(slug, model_id)| {
        let provider = store.providers.iter().find(|provider| provider.gateway_slug.as_deref() == Some(slug))?;
        (sources.admits_provider(provider) && provider.has_route_credentials() && served_models(provider).contains(&model_id))
            .then(|| Target::Provider { provider_id: provider.id.clone(), model_id: model_id.to_owned() })
    });
    provider.or_else(|| enabled().find(|aggregate| aggregate.public_model_id == model)
        .map(|aggregate| Target::Aggregate { public_id: aggregate.public_model_id.clone() }))
}

/// `/gateway/v1/models` in OpenAI's list shape, or Anthropic's.
pub fn models_document(models: &[GatewayModel], anthropic: bool) -> Value {
    if anthropic {
        let data: Vec<Value> = models.iter().map(|model| json!({
            "type": "model", "id": model.id, "display_name": model.label, "created_at": "2026-10-11T00:00:00Z",
        })).collect();
        return json!({"data": data, "has_more": false,
            "first_id": models.first().map(|model| &model.id), "last_id": models.last().map(|model| &model.id)});
    }
    let data: Vec<Value> = models.iter().map(|model| {
        let mut entry = json!({"id": model.id, "object": "model", "created": 0, "owned_by": model.source_name,
            "display_name": model.label});
        if let Some(context) = model.context_window {
            entry["context_window"] = json!(context);
        }
        entry
    }).collect();
    json!({"object": "list", "data": data})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, name: &str, base: &str) -> router::Provider {
        router::Provider {
            id: id.into(), name: name.into(), base_url: base.into(), api_key: "sk".into(), enabled: true,
            model: "m1".into(), models: vec![
                super::super::newapi::Model { id: "m1".into(), ..Default::default() },
                super::super::newapi::Model { id: "vendor/m2".into(), ..Default::default() },
                super::super::newapi::Model { id: "hidden".into(), included: false, ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn short_names_come_from_the_name_or_host_and_never_collide() {
        assert_eq!(slugify("  DeepSeek 官方 / Pro "), "deepseek-pro");
        assert_eq!(slugify("Open Router"), "open-router");
        assert_eq!(host_label("https://api.siliconflow.cn/v1"), "siliconflow");
        assert_eq!(host_label("https://openrouter.ai/api/v1"), "openrouter");
        assert_eq!(host_label("http://127.0.0.1:8080/v1"), "local");
        let mut store = Store { providers: vec![
            provider("a", "DeepSeek", "https://api.deepseek.com"),
            provider("b", "deepseek", "https://other.example"),
            provider("c", "硅基流动", "https://api.siliconflow.cn/v1"),
            router::Provider { gateway_slug: Some("Bad Slug".into()), ..provider("d", "Kept?", "https://x.example") },
        ], ..Default::default() };
        assert!(assign_slugs(&mut store));
        let slugs: Vec<_> = store.providers.iter().map(|p| p.gateway_slug.clone().unwrap()).collect();
        assert_eq!(slugs, ["deepseek", "deepseek-2", "siliconflow", "kept"]);
        assert!(!assign_slugs(&mut store), "assigned names stay");
        store.providers[0].name = "Renamed".into();
        assert!(!assign_slugs(&mut store));
        assert_eq!(store.providers[0].gateway_slug.as_deref(), Some("deepseek"), "a rename keeps the short name");
        assert_eq!(set_slug(&mut store, "a", Some("deepseek-2")).unwrap_err(), "newapi.errors.gatewaySlugTaken");
        assert_eq!(set_slug(&mut store, "a", Some("Not Valid")).unwrap_err(), "newapi.errors.gatewaySlug");
        set_slug(&mut store, "a", Some("ds")).unwrap();
        set_slug(&mut store, "a", Some("  ")).unwrap();
        assert_eq!(store.providers[0].gateway_slug.as_deref(), Some("ds"));
    }

    #[test]
    fn models_list_routable_models_and_resolve_back() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = models_dev::load(dir.path());
        let mut store = Store { providers: vec![
            provider("a", "Relay A", "https://a.example/v1"),
            router::Provider { enabled: false, ..provider("b", "Off", "https://b.example/v1") },
            router::Provider { api_key: String::new(), account_source: Some(super::super::account_sources::SourceKind::Workbuddy),
                ..provider("c", "WorkBuddy", "https://copilot.tencent.com/v2") },
        ], ..Default::default() };
        assign_slugs(&mut store);
        let ids: Vec<_> = models(&store, &catalog).into_iter().map(|model| (model.id, model.kind)).collect();
        assert_eq!(ids, [
            ("relay-a/m1".to_owned(), ModelKind::Relay), ("relay-a/vendor/m2".to_owned(), ModelKind::Relay),
            ("workbuddy/m1".to_owned(), ModelKind::Account), ("workbuddy/vendor/m2".to_owned(), ModelKind::Account),
        ]);
        assert_eq!(resolve(&store, "relay-a/vendor/m2"), Some(Target::Provider { provider_id: "a".into(), model_id: "vendor/m2".into() }));
        assert_eq!(resolve(&store, "relay-a/hidden"), None, "an excluded model is not served");
        assert_eq!(resolve(&store, "off/m1"), None);
        assert_eq!(resolve(&store, "m1"), None);
        store.aggregates.push(aggregation::AggregateModel { id: "agg".into(), public_model_id: "relay-a/m1".into(), ..Default::default() });
        assert_eq!(resolve(&store, "pool/relay-a/m1"), Some(Target::Aggregate { public_id: "relay-a/m1".into() }));
        assert_eq!(resolve(&store, "relay-a/m1"), Some(Target::Provider { provider_id: "a".into(), model_id: "m1".into() }),
            "a relay's own id is never taken by an aggregate's bare public id");
    }

    #[test]
    fn kinds_turned_off_are_neither_listed_nor_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = models_dev::load(dir.path());
        let mut store = Store { providers: vec![
            provider("a", "Relay A", "https://a.example/v1"),
            router::Provider { api_key: String::new(), account_source: Some(super::super::account_sources::SourceKind::Workbuddy),
                ..provider("c", "WorkBuddy", "https://copilot.tencent.com/v2") },
        ], ..Default::default() };
        store.aggregates.push(aggregation::AggregateModel { id: "agg".into(), public_model_id: "mix".into(), ..Default::default() });
        assign_slugs(&mut store);
        let kinds = |store: &Store| {
            let mut kinds: Vec<_> = models(store, &catalog).into_iter().map(|model| model.kind).collect();
            kinds.dedup();
            kinds
        };
        assert_eq!(kinds(&store), [ModelKind::Relay, ModelKind::Account]);
        store.router.gateway_sources.account = false;
        assert_eq!(kinds(&store), [ModelKind::Relay]);
        assert_eq!(resolve(&store, "workbuddy/m1"), None, "an account source turned off is refused");
        assert!(resolve(&store, "relay-a/m1").is_some());
        assert_eq!(all_models(&store, &catalog).len(), 4, "every model is still counted");
        store.router.gateway_sources.relay = false;
        assert_eq!(resolve(&store, "relay-a/m1"), None);
        assert_eq!(resolve(&store, "pool/mix"), Some(Target::Aggregate { public_id: "mix".into() }));
        store.router.gateway_sources.aggregate = false;
        assert_eq!(resolve(&store, "pool/mix"), None);
        assert_eq!(resolve(&store, "mix"), None, "nor by its bare id");
        assert!(resolve_with(&store, "workbuddy/m1", router::ModelSources::default()).is_some(),
            "other consumers resolve with their own kinds");
    }

    #[test]
    fn aggregates_carry_a_prefix_no_relay_may_take() {
        let mut store = Store { providers: vec![provider("a", "Pool", "https://a.example/v1")], ..Default::default() };
        store.aggregates.push(aggregation::AggregateModel { id: "x".into(), public_model_id: "gpt-5.5".into(), ..Default::default() });
        store.aggregates.push(aggregation::AggregateModel { id: "y".into(), public_model_id: "glm-5".into(),
            gateway_prefix: Some("cn".into()), ..Default::default() });
        assign_slugs(&mut store);
        assert_eq!(store.providers[0].gateway_slug.as_deref(), Some("pool-2"), "a relay named like the prefix steps aside");
        assert_eq!(aggregate_id(&store, &store.aggregates[0]), "pool/gpt-5.5");
        assert_eq!(aggregate_id(&store, &store.aggregates[1]), "cn/glm-5", "an aggregate's own prefix wins");
        assert_eq!(resolve(&store, "pool/gpt-5.5"), Some(Target::Aggregate { public_id: "gpt-5.5".into() }));
        assert_eq!(resolve(&store, "cn/glm-5"), Some(Target::Aggregate { public_id: "glm-5".into() }));
        assert_eq!(resolve(&store, "pool/glm-5"), None, "only the aggregate's own prefix names it");
        assert_eq!(resolve(&store, "gpt-5.5"), Some(Target::Aggregate { public_id: "gpt-5.5".into() }), "bare ids still resolve");
        assert_eq!(check_slug(&store, Some("a"), Some("cn")).unwrap_err(), "newapi.errors.gatewaySlugIsPrefix");
        assert_eq!(check_prefix(&store, "pool-2").unwrap_err(), "gateway.errors.prefixTaken");
        assert_eq!(check_prefix(&store, "Pool!").unwrap_err(), "gateway.errors.prefix");
        store.router.gateway_aggregate_prefix = "mix".into();
        assert_eq!(resolve(&store, "mix/gpt-5.5"), Some(Target::Aggregate { public_id: "gpt-5.5".into() }));
        store.router.gateway_aggregate_prefix = String::new();
        assert_eq!(shared_prefix(&store), DEFAULT_AGGREGATE_PREFIX, "an empty prefix falls back to the default");
    }

    #[test]
    fn keys_authenticate_by_value_only() {
        let mut store = Store::default();
        store.router.bearer_token = "internal".into();
        store.router.gateway_keys.push(router::GatewayKey { id: "1".into(), name: "Hermes".into(), key: new_key(), created_at: 0 });
        let key = store.router.gateway_keys[0].key.clone();
        assert!(key.starts_with("sk-ah-") && key.len() > 40);
        assert_eq!(authenticate(&store, Some(&key), None).map(|key| key.name.as_str()), Some("Hermes"));
        assert_eq!(authenticate(&store, None, Some(&key)).map(|key| key.id.as_str()), Some("1"));
        assert!(authenticate(&store, Some("internal"), None).is_none(), "the router's own token is not a gateway key");
        assert!(authenticate(&store, Some(""), None).is_none());
        assert_eq!(mask(&key), format!("sk-ah-…{}", &key[key.len() - 4..]));
        assert!(models_document(&[], true)["data"].as_array().unwrap().is_empty());
    }
}
