use super::*;
use crate::core::aggregation;

fn aggregate_fixture() -> (tempfile::TempDir, CodexPaths) {
    let (directory, paths) = fixture();
    let mut store = load_store(&paths).unwrap();
    store.aggregates.push(aggregation::AggregateModel {
        id: "pool".into(),
        public_model_id: "official-model".into(),
        display_name: "Official model".into(),
        members: vec![aggregation::AggregateMember {
            id: "member".into(),
            provider_id: store.providers[0].id.clone(),
            actual_model_id: "fixture-model".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    save_store_unlocked(&paths, &store).unwrap();
    (directory, paths)
}

#[test]
fn codex_aggregate_selection_saves_then_applies_the_public_model_and_restores_user_files() {
    let (_directory, paths) = aggregate_fixture();
    let before = file_state(&paths);
    let selection = select_aggregate(&paths, Some("pool".into())).unwrap();
    assert!(selection.applied.is_none());
    assert_eq!(file_state(&paths)[1..], before[1..]);
    assert_eq!(
        load_store(&paths)
            .unwrap()
            .router
            .aggregate_model_id
            .as_deref(),
        Some("pool")
    );
    set_smart_routing(&paths, true).unwrap();
    let installed = config(&paths);
    assert_eq!(installed["model"].as_str(), Some("official-model"));
    // The selected aggregate is reached through the built-in provider by its
    // public model ID; no dedicated provider is written.
    let store = load_store(&paths).unwrap();
    assert_eq!(
        installed["openai_base_url"].as_str(),
        Some(builtin_router_base(&store).as_str())
    );
    assert!(installed
        .get("model_providers")
        .and_then(|providers| providers.get(AGGREGATE_PROVIDER_KEY))
        .is_none());
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(catalog_path(&paths)).unwrap()).unwrap();
    assert_eq!(catalog["models"][0]["slug"], "official-model");
    assert!(!catalog.to_string().contains("synthetic-native-mode-key"));
    assert!(!catalog.to_string().contains("fixture-model"));
    assert_user_files_preserved(&paths);
    let config_before = fs::read(&paths.config_path).unwrap();
    let mut changed = store;
    changed.aggregates[0].members[0].weight = 8;
    changed.providers[0].api_key = "rotated-upstream-key".into();
    save_store_unlocked(&paths, &changed).unwrap();
    assert!(ensure_routing_config(&paths, &load_store(&paths).unwrap())
        .unwrap()
        .is_none());
    assert_eq!(fs::read(&paths.config_path).unwrap(), config_before);
    set_smart_routing(&paths, false).unwrap();
    assert_original_config(&paths);
    assert_user_files_preserved(&paths);
}

#[test]
fn explicit_aggregate_selection_still_changes_the_default_model() {
    let (_directory, paths) = aggregate_fixture();
    let mut store = load_store(&paths).unwrap();
    let mut second = store.aggregates[0].clone();
    second.id = "second-pool".into();
    second.public_model_id = "second-official-model".into();
    store.aggregates.push(second);
    save_store_unlocked(&paths, &store).unwrap();
    select_aggregate(&paths, Some("pool".into())).unwrap();
    set_smart_routing(&paths, true).unwrap();
    for (pool, model) in [("second-pool", "second-official-model"), ("pool", "official-model")] {
        select_aggregate(&paths, Some(pool.into())).unwrap();
        assert_eq!(config(&paths)["model"].as_str(), Some(model));
        assert!(routing_config_matches(&paths, &load_store(&paths).unwrap()).unwrap());
    }
}

#[test]
fn codex_aggregate_never_changes_native_mode_or_a_foreign_config_without_authorization() {
    let (_directory, paths) = aggregate_fixture();
    set_access_mode(&paths, CodexAccessMode::Native).unwrap();
    let before = file_state(&paths);
    assert_eq!(
        select_aggregate(&paths, Some("pool".into()))
            .err()
            .as_deref(),
        Some("aggregation.errors.routerOnly")
    );
    assert_eq!(file_state(&paths), before);
    set_access_mode(&paths, CodexAccessMode::Router).unwrap();
    select_aggregate(&paths, Some("pool".into())).unwrap();
    set_smart_routing(&paths, true).unwrap();
    let before = file_state(&paths);
    assert_eq!(
        set_access_mode(&paths, CodexAccessMode::Native)
            .err()
            .as_deref(),
        Some("aggregation.errors.routerOnly")
    );
    assert_eq!(file_state(&paths), before);
    let mut document: toml_edit::DocumentMut = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap();
    document["model_provider"] = toml_edit::value("external-manager");
    document["model"] = toml_edit::value("external-model");
    fs::write(&paths.config_path, document.to_string()).unwrap();
    let external = fs::read(&paths.config_path).unwrap();
    select_aggregate(&paths, None).unwrap();
    assert_eq!(fs::read(&paths.config_path).unwrap(), external);
}

fn menu_slugs(paths: &CodexPaths) -> Vec<String> {
    catalog_value(&paths.app_data_dir, &paths.codex_home, &load_store(paths).unwrap())["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["slug"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn codex_menu_offers_every_kind_turned_on_and_earlier_builds_keep_their_choice() {
    let (_directory, paths) = aggregate_fixture();
    let relay = format!("{}::fixture-model", load_store(&paths).unwrap().providers[0].id);
    // A store from before the switches: relays alone until an aggregate is chosen.
    assert!(menu_slugs(&paths).contains(&relay));
    assert!(!menu_slugs(&paths).contains(&"official-model".to_owned()));
    select_aggregate(&paths, Some("pool".into())).unwrap();
    assert_eq!(menu_slugs(&paths), ["official-model"]);
    // Every kind together; aggregates lead the menu.
    set_codex_sources(&paths, ModelSources::default()).unwrap();
    let slugs = menu_slugs(&paths);
    assert_eq!(slugs[0], "official-model");
    assert!(slugs.contains(&relay));
    assert_eq!(load_store(&paths).unwrap().router.aggregate_model_id.as_deref(), Some("pool"),
        "turning relays on keeps the default aggregate");
    // Turning aggregates off drops the default aggregate with them.
    set_codex_sources(&paths, ModelSources { aggregate: false, ..Default::default() }).unwrap();
    assert_eq!(menu_slugs(&paths), [relay.clone()]);
    assert!(load_store(&paths).unwrap().router.aggregate_model_id.is_none());
}

#[test]
fn codex_keeps_a_selection_its_menu_still_offers_and_refuses_an_empty_one() {
    let (_directory, paths) = aggregate_fixture();
    let relay = format!("{}::fixture-model", load_store(&paths).unwrap().providers[0].id);
    set_codex_sources(&paths, ModelSources::default()).unwrap();
    select_aggregate(&paths, Some("pool".into())).unwrap();
    set_smart_routing(&paths, true).unwrap();
    assert_eq!(config(&paths)["model"].as_str(), Some("official-model"));
    // The user picks a relay model in Codex: it is a valid menu choice.
    let mut document: toml_edit::DocumentMut = fs::read_to_string(&paths.config_path).unwrap().parse().unwrap();
    document["model"] = toml_edit::value(relay.as_str());
    fs::write(&paths.config_path, document.to_string()).unwrap();
    assert!(routing_config_matches(&paths, &load_store(&paths).unwrap()).unwrap());
    set_codex_sources(&paths, ModelSources { aggregate: false, ..Default::default() }).unwrap();
    assert_eq!(config(&paths)["model"].as_str(), Some(relay.as_str()), "a selection still offered stays");
    // Relays off: the selection leaves the menu and the first aggregate starts Codex.
    set_codex_sources(&paths, ModelSources { relay: false, account: false, aggregate: true }).unwrap();
    assert_eq!(config(&paths)["model"].as_str(), Some("official-model"));
    // Nothing left to start on is refused and changes nothing.
    let before = file_state(&paths);
    assert_eq!(
        set_codex_sources(&paths, ModelSources { relay: false, account: false, aggregate: false }).err().as_deref(),
        Some("newapi.errors.noActiveProvider")
    );
    assert_eq!(file_state(&paths), before);
    assert_eq!(codex_sources(&load_store(&paths).unwrap()), ModelSources { relay: false, account: false, aggregate: true });
}
