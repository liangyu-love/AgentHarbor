//! Acceptance regressions use real resolvers, store saves, config projection
//! and usage observers with isolated paths; no paid upstream requests.
use super::{
    capability_preview, model_metadata as md, model_policy as policy, models_dev as dev, newapi,
    relay_usage as usage, router, secret_file,
};
use crate::platform::paths::CodexPaths;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
};

fn snapshot(source: &str) -> Value {
    json!({"schemaVersion":1,"sourceUrl":source,"fetchedAt":1_700_000_000,"revision":"fixture","providers":[
        {"id":"openai","models":[{"id":"model","limit":{"context":400000,"input":272000,"output":8192},"cost":{"input":1,"output":2}}]},
        {"id":"billing","models":[{"id":"model","limit":{"context":100000,"output":8192},"cost":{"input":9,"output":18}}]}
    ]})
}

fn catalog(value: &Value) -> dev::Catalog {
    dev::Catalog::parse(value.to_string().as_bytes(), dev::Origin::Runtime).unwrap()
}

fn request<'a>(raw: &'a str, scope: &'a [String]) -> md::ResolutionRequest<'a> {
    md::ResolutionRequest {
        raw_model_id: raw,
        display_name: "",
        catalog_providers: scope,
        price_providers: scope,
        explicit_ref: None,
        price_ref: None,
        user: Default::default(),
        provider_context: None,
        official: None,
        upstream: None,
    }
}

fn paths() -> (tempfile::TempDir, CodexPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_roots(dir.path().join("codex"), dir.path().join("app"));
    paths.ensure_directories().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(&paths.config_path, "").unwrap();
    (dir, paths)
}

fn provider(id: &str, context: u64) -> router::Provider {
    router::Provider {
        id: id.into(),
        name: id.into(),
        base_url: "https://relay.example/v1".into(),
        api_key: "synthetic-key".into(),
        enabled: true,
        model: "model".into(),
        models: vec![newapi::Model {
            id: "model".into(),
            overrides: newapi::ModelOverrides {
                context_window: Some(context),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn input(id: Option<String>, model: newapi::Model) -> router::ProviderInput {
    router::ProviderInput {
        id,
        name: "Relay".into(),
        base_url: "https://relay.example/v1".into(),
        user_id: None,
        access_token: String::new(),
        api_key: "synthetic-key".into(),
        protocol: router::ModelProtocol::Responses,
        model: model.id.clone(),
        models: vec![model],
        context_window: None,
        auto_compact_limit: None,
        catalog_providers: Some(vec![]),
        price_catalog_provider: None,
        compression: Some(policy::CompressionPolicy::Inherit),
        network_mode: "direct".into(),
    }
}

#[test]
fn public_api_refresh_transforms_and_preserves_last_good_cache_on_failure_and_304() {
    let (_dir, paths) = paths();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let source = format!("http://{}/api.json", listener.local_addr().unwrap());
    std::fs::write(
        paths.app_data_dir.join(dev::CACHE_FILE),
        snapshot(&source).to_string(),
    )
    .unwrap();
    let public = json!({"openai":{"id":"openai","models":{"model":{"id":"model",
        "limit":{"context":200000,"input":180000,"output":64000}, "reasoning":true,
        "reasoning_options":[{"type":"effort","values":["low",null,"high"]},{"type":"budget_tokens","min":-1},{"type":"future-option"}],
        "cost":{"input":5,"output":25,"cache_read":0.5,"cache_write":6,"reasoning":30,
            "context_over_200k":{"input":10,"cache_read":1},
            "tiers":[{"tier":{"type":"context","size":100000},"input":7,"output":35,"cache_write":8}]},
        "canonical_model_id":"model-v1","interleaved":{"field":"reasoning_content"},"tool_call":true,"last_updated":"2026-10-01"
    }}}}).to_string();
    let server = std::thread::spawn(move || {
        for (status, body) in [(200, public), (200, "{}".into()), (304, String::new())] {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 8192];
            let count = socket.read(&mut request).unwrap();
            if status == 304 {
                assert!(String::from_utf8_lossy(&request[..count])
                    .to_lowercase()
                    .contains("if-none-match: fixture-etag"));
            }
            write!(socket, "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nETag: fixture-etag\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let proxy = super::models::ApiProxyConfigPayload {
        mode: super::models::ApiProxyMode::Direct,
        url: None,
    };
    let refreshed = dev::refresh(&paths.app_data_dir, &proxy).unwrap();
    let loaded = dev::load(&paths.app_data_dir);
    let model = loaded.exact_in("openai", "model").unwrap().model;
    assert_eq!(model.limit.input, Some(180000));
    assert_eq!(model.reasoning_options.len(), 2);
    assert_eq!(model.reasoning_options_declared, Some(true));
    assert!(matches!(model.interleaved, Some(dev::Interleaved::Field { ref field }) if field == "reasoning_content"));
    assert_eq!(model.cost.as_ref().unwrap().cache_read, Some(0.5));
    assert_eq!(
        model.cost.as_ref().unwrap().tiers[0].rates.cache_write,
        Some(8.0)
    );
    assert_eq!(
        model
            .cost
            .as_ref()
            .unwrap()
            .context_over_200k
            .unwrap()
            .cache_read,
        Some(1.0)
    );
    assert_eq!(model.canonical_model_id.as_deref(), Some("model-v1"));
    assert_eq!(model.tool_call, Some(true));
    let cache = std::fs::read(paths.app_data_dir.join(dev::CACHE_FILE)).unwrap();
    assert!(dev::refresh(&paths.app_data_dir, &proxy).is_err());
    assert_eq!(
        std::fs::read(paths.app_data_dir.join(dev::CACHE_FILE)).unwrap(),
        cache
    );
    assert_eq!(
        dev::load(&paths.app_data_dir).snapshot.revision,
        refreshed.revision
    );
    let checked = dev::refresh(&paths.app_data_dir, &proxy).unwrap();
    assert_eq!(checked.revision, refreshed.revision);
    assert_eq!(checked.fetched_at, refreshed.fetched_at);
    assert!(checked.error.is_none());
    server.join().unwrap();
}

#[test]
fn single_field_override_preserves_upstream_and_clears_across_save_and_reload() {
    let (_dir, paths) = paths();
    let mut model = newapi::Model {
        id: "upstream-only-fixture".into(),
        metadata_source: "upstream".into(),
        context_window: Some(200000),
        max_output_tokens: Some(4096),
        supported_reasoning_levels: Some(vec![newapi::ReasoningLevel {
            effort: "high".into(),
            description: "High".into(),
        }]),
        ..Default::default()
    };
    let saved = router::save_provider(&paths, input(None, model.clone())).unwrap();
    let id = saved.state.providers[0].id.clone();
    model.overrides.context_window = Some(100000);
    router::save_provider(&paths, input(Some(id.clone()), model.clone())).unwrap();
    let store = router::load_store(&paths).unwrap();
    let view = capability_preview::preview(&paths, &store);
    assert_eq!(view.models[0].output.value, Some(4096));
    assert_eq!(view.models[0].reasoning.source, "upstream");
    model.overrides.context_window = None;
    router::save_provider(&paths, input(Some(id), model)).unwrap();
    let store = router::load_store(&paths).unwrap();
    assert!(store.providers[0].models[0].overrides.is_empty());
    assert_eq!(
        capability_preview::preview(&paths, &store).models[0]
            .context
            .value,
        Some(200000)
    );
    // Even historical manual provenance cannot resurrect cleared overrides.
    let mut model = store.providers[0].models[0].clone();
    model.metadata_source = "manual".into();
    router::save_provider(&paths, input(Some(store.providers[0].id.clone()), model)).unwrap();
    let store = router::load_store(&paths).unwrap();
    assert!(store.providers[0].models[0].overrides.is_empty());
    assert_ne!(
        capability_preview::preview(&paths, &store).models[0]
            .context
            .source,
        "manual"
    );
}

#[test]
fn metadata_migration_backs_up_original_encrypted_store_once() {
    let (_dir, paths) = paths();
    let mut old = router::Store {
        providers: vec![provider("relay", 100000)],
        ..Default::default()
    };
    old.version = 7;
    old.providers[0].models[0].overrides = Default::default();
    old.providers[0].models[0].metadata_source = "manual".into();
    old.providers[0].models[0].context_window = Some(99999);
    let bytes = serde_json::to_vec(&old).unwrap();
    secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &bytes).unwrap();
    let migrated = router::load_store(&paths).unwrap();
    assert_eq!(
        migrated.providers[0].models[0].overrides.context_window,
        Some(99999)
    );
    let backup = paths.app_data_dir.join("newapi-providers.pre-v8.dat");
    assert_eq!(secret_file::read(&backup).unwrap().unwrap(), bytes);
    router::load_store(&paths).unwrap();
    assert_eq!(secret_file::read(&backup).unwrap().unwrap(), bytes);
}

#[test]
fn compression_respects_model_and_provider_context_without_rewriting_raw_input() {
    let cat = catalog(&snapshot(dev::DEFAULT_SOURCE_URL));
    let scope = vec!["openai".into()];
    for manual in [true, false] {
        let mut req = request("model", &scope);
        if manual {
            req.user.context_window = Some(128000);
        } else {
            req.provider_context = Some(128000);
        }
        let metadata = md::resolve(&cat, &req);
        let projected = policy::project(
            &metadata,
            (
                &policy::CompressionPolicy::percent(90),
                policy::PolicySource::Global,
            ),
        );
        assert_eq!(metadata.input_limit.value, Some(272000));
        assert_eq!(projected.budget, 128000);
        assert_eq!(projected.compression.threshold, Some(115200));
        let fixed = policy::project(
            &metadata,
            (
                &policy::CompressionPolicy::Tokens { tokens: 128000 },
                policy::PolicySource::Model,
            ),
        );
        assert_eq!(fixed.compression.state, "invalid");
    }
}

#[test]
fn failover_budgets_share_eligibility_and_only_limit_the_corresponding_menu_entry() {
    let (_dir, paths) = paths();
    let mut store = router::Store {
        providers: vec![provider("large", 400000), provider("small", 100000)],
        ..Default::default()
    };
    store.router.same_model_failover = true;
    let view = capability_preview::preview(&paths, &store);
    assert_eq!(view.models[0].projection.compression.threshold, Some(90000));
    assert_eq!(view.models[0].projection.budget_source, "failover");
    let menu = router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
    assert_eq!(menu["models"][0]["auto_compact_token_limit"], 90000);
    assert_eq!(
        router::catalog_entry_for_slug(
            &paths.app_data_dir,
            &paths.codex_home,
            &store,
            "large::model"
        )
        .unwrap()["auto_compact_token_limit"],
        90000
    );
    // The proxy's predicate still excludes protocol/identity/credential mismatches.
    for mismatch in ["protocol", "identity", "disabled", "excluded"] {
        let mut changed = store.clone();
        match mismatch {
            "protocol" => changed.providers[1].protocol = router::ModelProtocol::ChatCompletions,
            "identity" => {
                changed.providers[1].model = "other".into();
                changed.providers[1].models[0].id = "other".into();
            }
            "disabled" => changed.providers[1].enabled = false,
            _ => {
                changed.providers[1].model = "other".into();
                changed.providers[1].models[0].included = false;
            }
        }
        assert_eq!(
            capability_preview::preview(&paths, &changed).models[0]
                .projection
                .compression
                .threshold,
            Some(360000),
            "{mismatch}"
        );
    }
    store.providers[1].models[0].overrides.context_window = None;
    assert!(capability_preview::preview(&paths, &store).models[0]
        .projection
        .compression
        .note
        .as_ref()
        .unwrap()
        .contains("unknown"));
    store.router.same_model_failover = false;
    assert_eq!(
        capability_preview::preview(&paths, &store).models[0]
            .projection
            .compression
            .threshold,
        Some(360000)
    );
}

#[test]
fn prices_obey_independent_bindings_and_manual_overrides() {
    let cat = catalog(&snapshot(dev::DEFAULT_SOURCE_URL));
    let scope = vec!["openai".into()];
    let billing = vec!["billing".into()];
    let capability = dev::ModelRef {
        provider_id: "openai".into(),
        model_id: "model".into(),
    };
    let mut req = request("model", &scope);
    req.explicit_ref = Some(&capability);
    req.price_providers = &billing;
    assert_eq!(md::resolve(&cat, &req).price.input, Some(9.0));
    let missing = dev::ModelRef {
        provider_id: "billing".into(),
        model_id: "missing".into(),
    };
    req.price_ref = Some(&missing);
    assert_eq!(md::resolve(&cat, &req).price.source, "unknown");
    req.user.price = Some(md::CostRatesInput {
        input: Some(0.0),
        output: Some(0.0),
        ..Default::default()
    });
    assert_eq!(md::resolve(&cat, &req).price.input, Some(0.0));
    req.user.price = None;
    req.price_ref = None;
    req.price_providers = &[];
    assert_eq!(md::resolve(&cat, &req).price.source, "unknown");
    req.explicit_ref = None;
    assert_eq!(md::resolve(&cat, &req).price.source, "unknown");
}

#[test]
fn capability_reference_does_not_replace_the_full_billing_identity() {
    let mut data = snapshot(dev::DEFAULT_SOURCE_URL);
    data["providers"][1]["models"].as_array_mut().unwrap()
        .push(json!({"id":"custom","cost":{"input":100,"output":200}}));
    let cat = catalog(&data);
    let capability = dev::ModelRef { provider_id: "openai".into(), model_id: "model".into() };
    let billing = vec!["billing".into()];
    let mut req = request("custom", &billing);
    req.explicit_ref = Some(&capability);
    let result = md::resolve(&cat, &req);
    assert_eq!(result.context_window.value, Some(400000));
    assert_eq!(result.price.input, Some(100.0));
    assert_eq!(result.price.source_ref.unwrap().model_id, "custom");
}

#[test]
fn each_field_points_to_a_record_that_actually_declares_it() {
    let mut data = snapshot(dev::DEFAULT_SOURCE_URL);
    data["providers"][0]["models"][0]["limit"] = json!({"output":8192});
    data["providers"][0]["models"][0]["lastUpdated"] = json!("2020-01-01");
    data["providers"][1]["models"][0]["lastUpdated"] = json!("2026-10-01");
    let result = md::resolve(&catalog(&data), &request("model", &[]));
    assert_eq!(result.context_window.value, Some(100000));
    assert_eq!(result.context_window.source_ref.unwrap().provider_id, "billing");
    assert_eq!(result.context_window.updated_at.as_deref(), Some("2026-10-01"));
    assert_eq!(result.output_limit.source_ref.unwrap().provider_id, "openai");
    assert_eq!(result.output_limit.updated_at.as_deref(), Some("2020-01-01"));
}

#[test]
fn audio_and_conditional_rates_survive_resolution_and_pricing_snapshots() {
    let mut data = snapshot(dev::DEFAULT_SOURCE_URL);
    data["providers"][0]["models"][0]["cost"] = json!({"input":1,"output":2,"inputAudio":10,"outputAudio":20,
        "contextOver200k":{"input":3,"output":4,"inputAudio":30},
        "tiers":[{"tier":{"type":"context","size":300000},"input":5,"output":6,"outputAudio":40}]});
    let cat = catalog(&data);
    let mut p = provider("p", 400000);
    p.price_catalog_provider = Some("openai".into());
    let result = router::resolve_model_metadata(&p, "model", None, &cat);
    assert_eq!(result.price.input_audio, Some(10.0));
    assert_eq!(result.price.output_audio, Some(20.0));
    assert!(!result.price.complete);
    let pricing = router::pricing_for(&p, "model", &cat);
    let serialized = serde_json::to_value(&pricing).unwrap();
    assert_eq!(serialized["inputAudio"].as_f64(), Some(10.0));
    assert_eq!(serialized["contextOver200k"]["inputAudio"].as_f64(), Some(30.0));
    assert_eq!(serialized["tiers"][0]["outputAudio"].as_f64(), Some(40.0));
    assert!(!pricing.complete);
}

#[test]
fn compound_price_tags_never_borrow_base_prices_but_full_records_and_explicit_prices_work() {
    let scope = vec!["openai".into()];
    let cat = catalog(&snapshot(dev::DEFAULT_SOURCE_URL));
    for raw in [
        "model:free",
        "openai/model:free",
        "channel::model:free",
        "openai/model@fast",
        "channel__openai/model:batch",
    ] {
        assert_eq!(
            md::resolve(&cat, &request(raw, &scope)).price.source,
            "unknown",
            "{raw}"
        );
    }
    let mut data = snapshot(dev::DEFAULT_SOURCE_URL);
    data["providers"][0]["models"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"model:free","cost":{"input":0,"output":0}}));
    assert_eq!(
        md::resolve(&catalog(&data), &request("openai/model:free", &scope))
            .price
            .input,
        Some(0.0)
    );
    let reference = dev::ModelRef {
        provider_id: "openai".into(),
        model_id: "model".into(),
    };
    let mut req = request("openai/model:free", &scope);
    req.price_ref = Some(&reference);
    assert_eq!(md::resolve(&cat, &req).price.input, Some(1.0));
}

#[test]
fn price_agreement_compares_conditional_and_reasoning_rates_and_reports_conflicts() {
    let scope = vec!["openai".into(), "billing".into()];
    for field in ["tiers", "contextOver200k", "reasoning", "inputAudio"] {
        let mut data = snapshot(dev::DEFAULT_SOURCE_URL);
        for (index, rate) in [(0, 11), (1, 21)] {
            let mut cost = json!({"input":1,"output":2});
            cost[field] = match field {
                "tiers" => {
                    json!([{"tier":{"type":"context","size":200000},"input":rate,"output":rate*2}])
                }
                "contextOver200k" => json!({"input":rate,"output":rate*2}),
                _ => json!(rate),
            };
            data["providers"][index]["models"][0]["cost"] = cost;
        }
        let price = md::resolve(&catalog(&data), &request("model", &scope)).price;
        assert_eq!(price.source, "unknown", "{field}");
        assert!(price.note.unwrap().contains("disagree"));
    }
}

#[test]
fn a_missing_scoped_match_labels_actual_generic_provenance() {
    let scope = vec!["missing".into()];
    let result = md::resolve(
        &catalog(&snapshot(dev::DEFAULT_SOURCE_URL)),
        &request("model", &scope),
    );
    assert_eq!(result.match_kind, md::MatchKind::Consensus);
    assert_eq!(result.output_limit.source, "catalogGeneric");
}

#[test]
fn native_preview_separates_model_suggestions_shared_threshold_and_inactive_models() {
    let (_dir, paths) = paths();
    let mut active = provider("active", 100000);
    let mut larger = active.models[0].clone();
    larger.id = "larger".into();
    larger.overrides.context_window = Some(200000);
    active.models.push(larger);
    let store = router::Store {
        providers: vec![active, provider("inactive", 300000)],
        active_provider_id: Some("active".into()),
        codex_access_mode: router::CodexAccessMode::Native,
        ..Default::default()
    };
    router::apply_routing_config(&paths, &store).unwrap();
    let view = capability_preview::preview(&paths, &store);
    assert_eq!(view.models[0].compression_applied, Some(true));
    assert_eq!(
        view.models[1].projection.compression.threshold,
        Some(180000)
    );
    assert_eq!(view.models[1].compression_applied, Some(false));
    assert_eq!(view.models[1].compression_shared_threshold, Some(90000));
    assert_eq!(view.models[1].compression_applied_threshold, Some(90000));
    assert_eq!(view.models[1].compression_shared_applied, Some(true));
    assert_eq!(view.models[1].compression_limiting_models, vec!["model"]);
    assert_eq!(view.models[2].compression_scope, "inactive");
    assert_eq!(view.models[2].compression_applied, Some(false));
    assert!(view.models[2].compression_applied_threshold.is_none());
    let mut changed = store.clone();
    changed.providers[0].compression = policy::CompressionPolicy::percent(80);
    let view = capability_preview::preview(&paths, &changed);
    assert_eq!(view.models[0].compression_shared_threshold, Some(80000));
    assert_eq!(view.models[0].compression_applied_threshold, Some(90000));
    assert_eq!(view.models[0].compression_shared_applied, Some(false));
}

#[test]
fn native_shared_compression_checks_every_selected_model() {
    for state in ["invalid", "clientDefault", "unknown"] {
        let (_dir, paths) = paths();
        let mut p = provider("p", 10000);
        p.models[0].compression = match state {
            "invalid" => policy::CompressionPolicy::Tokens { tokens: 20000 },
            "clientDefault" => policy::CompressionPolicy::ClientDefault,
            _ => { p.models[0].overrides.context_window = None; policy::CompressionPolicy::Inherit }
        };
        let mut large = p.models[0].clone();
        large.id = "large".into();
        large.overrides.context_window = Some(1000000);
        large.compression = policy::CompressionPolicy::Inherit;
        p.models.push(large);
        let store = router::Store { providers: vec![p], active_provider_id: Some("p".into()),
            codex_access_mode: router::CodexAccessMode::Native, ..Default::default() };
        router::apply_routing_config(&paths, &store).unwrap();
        let config: toml_edit::DocumentMut = std::fs::read_to_string(&paths.config_path).unwrap().parse().unwrap();
        assert!(config.get("model_auto_compact_token_limit").is_none(), "{state}");
        let view = capability_preview::preview(&paths, &store);
        let shared = view.models[1].compression_shared_plan.as_ref().unwrap();
        assert_eq!(shared.state, state);
        assert_eq!(shared.blocking_models, vec!["model"]);
        assert!(shared.threshold.is_none());
        let native_catalog: Value = serde_json::from_slice(&std::fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(native_catalog["models"].as_array().unwrap().len(), 2);
        if state != "clientDefault" { assert_eq!(view.models[1].compression_shared_applied, Some(false)); }
    }
}

#[test]
fn native_and_opencode_use_the_same_applicable_official_snapshot_as_preview() {
    use super::{client_config, targets::TargetId};
    let (_dir, paths) = paths();
    std::fs::write(paths.app_data_dir.join(dev::CACHE_FILE), snapshot(dev::DEFAULT_SOURCE_URL).to_string()).unwrap();
    std::fs::write(paths.codex_home.join("models_cache.json"),
        json!({"models":[{"slug":"model","context_window":100000,"max_output_tokens":8192}]}).to_string()).unwrap();
    let mut p = provider("p", 400000);
    p.models[0].overrides = Default::default();
    p.base_url = "https://api.openai.com/v1".into();
    p.catalog_providers = vec!["openai".into()];
    let store = router::Store { providers: vec![p], active_provider_id: Some("p".into()),
        codex_access_mode: router::CodexAccessMode::Native, ..Default::default() };
    router::apply_routing_config(&paths, &store).unwrap();
    let view = capability_preview::preview(&paths, &store);
    assert_eq!(view.models[0].projection.context_window, 100000);
    assert_eq!(view.models[0].compression_shared_threshold, Some(90000));
    let config: toml_edit::DocumentMut = std::fs::read_to_string(&paths.config_path).unwrap().parse().unwrap();
    assert_eq!(config["model_auto_compact_token_limit"].as_integer(), Some(90000));
    let native_catalog: Value = serde_json::from_slice(&std::fs::read(config["model_catalog_json"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(native_catalog["models"][0]["context_window"], 100000);
    let manager = client_config::ConfigManager::new(&paths.app_data_dir,
        client_config::ClientPaths::from_home(&paths.app_data_dir.join("clients"))).with_official_home(&paths.codex_home);
    let request = client_config::ConfigRequest { target_id: TargetId::OpenCode, provider_id: "p".into(), model: "model".into(),
        context_window: None, max_output_tokens: None, options: Default::default() };
    let preview: Value = serde_json::from_str(&manager.preview(&store, &request).unwrap().content).unwrap();
    assert_eq!(preview["provider"]["agentharbor_p"]["models"]["model"]["limit"]["context"], 100000);
}

#[test]
fn a_stale_catalog_does_not_imply_an_active_router_configuration() {
    let (_dir, paths) = paths();
    let mut store = router::Store { providers: vec![provider("p", 10000)], ..Default::default() };
    router::write_catalog(&paths, &store).unwrap();
    std::fs::write(&paths.config_path, "model_provider = 'foreign'\n").unwrap();
    let row = &capability_preview::preview(&paths, &store).models[0];
    assert_eq!(row.compression_applied, Some(false));
    assert_eq!(row.compression_scope, "inactive");
    assert_eq!(row.compression_written_threshold, Some(9000));
    assert!(row.compression_applied_threshold.is_none());
    store.router.enabled = true;
    store.router.bearer_token = "synthetic-bearer".into();
    router::apply_routing_config(&paths, &store).unwrap();
    assert_eq!(capability_preview::preview(&paths, &store).models[0].compression_applied, Some(true));
    let config = std::fs::read_to_string(&paths.config_path).unwrap();
    let mut doc: toml_edit::DocumentMut = config.parse().unwrap();
    doc["model_catalog_json"] = toml_edit::value("foreign-catalog.json");
    std::fs::write(&paths.config_path, doc.to_string()).unwrap();
    assert_eq!(capability_preview::preview(&paths, &store).models[0].compression_applied, Some(false));
}

#[test]
fn usage_unknown_partial_zero_failure_and_price_snapshots_stay_distinct() {
    let (_dir, paths) = paths();
    let price = usage::PricingSnapshot {
        input: Some(1.0),
        output: Some(2.0),
        source: "manual".into(),
        revision: "original".into(),
        complete: true,
        currency: "USD".into(),
        ..Default::default()
    };
    for (body, rate, status) in [
        (json!({"choices":[]}), price.clone(), 200),
        (
            json!({"usage":{"input_tokens":0,"output_tokens":0}}),
            price.clone(),
            200,
        ),
        (
            json!({"usage":{"input_tokens":1000000,"output_tokens":1000000}}),
            usage::PricingSnapshot {
                output: None,
                ..price.clone()
            },
            200,
        ),
        (
            json!({"usage":{"input_tokens":1000000}}),
            price.clone(),
            200,
        ),
        (
            json!({"usage":{"input_tokens":1000000,"output_tokens":1000000}}),
            price.clone(),
            500,
        ),
        (
            json!({"usage":{"total_tokens":2000000}}),
            price.clone(),
            200,
        ),
    ] {
        let mut observer = usage::RelayUsageRequest::new(
            paths.app_data_dir.clone(),
            "p".into(),
            "P".into(),
            "model".into(),
            "responses".into(),
            rate,
        )
        .with_request_id("shared-request")
        .observe(status, false);
        observer.observe(body.to_string().as_bytes());
    }
    usage::RelayUsageRequest::new(
        paths.app_data_dir.clone(),
        "p".into(),
        "P".into(),
        "model".into(),
        "responses".into(),
        price.clone(),
    )
    .failed(502);
    let ledger = std::fs::read_to_string(usage::usage_path(&paths.app_data_dir)).unwrap();
    let records: Vec<usage::RelayUsageRecord> = ledger
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[0].cost_usd, None);
    assert_eq!(records[0].pricing_status, "unpriced");
    assert_eq!(records[1].cost_usd, Some(0.0));
    assert_eq!(records[1].pricing_status, "priced");
    assert_eq!(records[2].cost_usd, Some(1.0));
    assert_eq!(records[2].pricing_status, "partial");
    assert_eq!(records[3].pricing_status, "partial");
    assert!(!records[3].usage_complete);
    assert_eq!(records[4].cost_usd, Some(3.0));
    assert_eq!(records[4].pricing_status, "priced");
    assert_eq!(records[5].cost_usd, None);
    assert_eq!(records[6].cost_usd, None);
    assert_eq!(
        records[4].pricing_snapshot.as_ref().unwrap().input,
        Some(1.0)
    );
    assert!(records[4].applied_rates.is_some());
    assert_eq!(records[4].schema_version, 2);
    assert_eq!(records[0].request_id, records[1].request_id);
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
    assert_eq!(
        usage::load(&paths.app_data_dir)
            .unwrap()
            .all_time
            .cost_micros,
        5000000
    );
    // Price edits and reads do not reprice earlier records.
    let _changed_price = usage::PricingSnapshot {
        input: Some(9.0),
        ..price
    };
    assert_eq!(
        usage::load(&paths.app_data_dir)
            .unwrap()
            .all_time
            .cost_micros,
        5000000
    );
}
