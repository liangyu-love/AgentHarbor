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
    assert_eq!(
        installed["model_provider"].as_str(),
        Some(AGGREGATE_PROVIDER_KEY)
    );
    assert_eq!(
        installed["model_providers"][AGGREGATE_PROVIDER_KEY]["base_url"].as_str(),
        Some("http://127.0.0.1:43127/aggregate/v1")
    );
    let store = load_store(&paths).unwrap();
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(selected_catalog_path(&paths, &store)).unwrap()).unwrap();
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
