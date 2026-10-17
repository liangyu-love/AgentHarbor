use super::*;
use std::{fs, path::Path};

#[path = "router_aggregate_access_tests.rs"]
mod aggregate_access_tests;

#[test]
fn accountless_can_be_selected_before_injection_without_changing_client_files() {
    let (_directory, paths) = fixture();
    let before = file_state(&paths);
    let change = set_accountless(&paths, true).unwrap();
    assert!(change.state.router.accountless);
    assert!(!change.state.router.enabled);
    assert!(change.applied.is_none());
    assert_eq!(file_state(&paths)[1..], before[1..]);
    set_smart_routing(&paths, true).unwrap();
    let applied = config(&paths);
    assert_eq!(applied["preferred_auth_method"].as_str(), Some("apikey"));
    assert_eq!(applied["model_providers"][CATALOG_PROVIDER_KEY]["requires_openai_auth"].as_bool(), Some(false));
    set_smart_routing(&paths, false).unwrap();
    assert_eq!(config(&paths)["preferred_auth_method"].as_str(), Some("chatgpt"));
}

#[test]
fn accountless_toggle_replaces_official_selection_and_keeps_account_catalogs() {
    let (_directory, paths) = fixture();
    let auth = br#"{"auth_mode":"chatgpt","tokens":{"account_id":"fixture-plus","access_token":"synthetic-preserved-token"}}"#;
    fs::write(&paths.auth_path, auth).unwrap();
    let key = super::super::auth::make_auth_snapshot(
        &super::super::auth::load_auth_file(&paths.auth_path).unwrap(),
        &paths.auth_path,
    )
    .unwrap()
    .account_key;
    let store = load_store(&paths).unwrap();
    let mut official =
        catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
    official["slug"] = serde_json::json!("official-plus-model");
    let catalog = serde_json::json!({"models":[official]}).to_string();
    fs::write(paths.codex_home.join("models_cache.json"), &catalog).unwrap();
    super::super::model_catalog::store_account_catalog(
        &paths.app_data_dir,
        &key,
        catalog.as_bytes(),
    )
    .unwrap();
    let saved_path = paths
        .app_data_dir
        .join(format!("model-capabilities-{key}.json"));
    let saved_catalog = fs::read(&saved_path).unwrap();
    set_smart_routing(&paths, true).unwrap();

    for _ in 0..2 {
        let mut selected: toml_edit::DocumentMut = fs::read_to_string(&paths.config_path)
            .unwrap()
            .parse()
            .unwrap();
        selected["model"] = toml_edit::value("official-plus-model");
        fs::write(&paths.config_path, selected.to_string()).unwrap();
        let enabled = set_accountless(&paths, true).unwrap();
        assert!(enabled.applied.is_some());
        assert_eq!(
            config(&paths)["model"].as_str(),
            Some("access-mode-fixture::fixture-model")
        );
        let store = load_store(&paths).unwrap();
        let menu = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        assert_eq!(menu["models"].as_array().unwrap().len(), 1);
        assert_eq!(
            menu["models"][0]["slug"],
            "access-mode-fixture::fixture-model"
        );
        assert!(catalog_entry_for_slug(
            &paths.app_data_dir,
            &paths.codex_home,
            &store,
            "official-plus-model"
        )
        .is_none());
        assert!(catalog_entry_for_slug(
            &paths.app_data_dir,
            &paths.codex_home,
            &store,
            "access-mode-fixture::fixture-model"
        )
        .is_some());
        let injected: serde_json::Value =
            serde_json::from_slice(&fs::read(catalog_path(&paths)).unwrap())
                .unwrap();
        assert_eq!(injected, menu);
        assert!(super::subagents::load(&paths)
            .unwrap()
            .models
            .iter()
            .all(|model| model.id.contains("::")));
        assert_eq!(fs::read(&paths.auth_path).unwrap(), auth);
        assert_eq!(fs::read(&saved_path).unwrap(), saved_catalog);
        assert_eq!(
            fs::read_to_string(paths.codex_home.join("models_cache.json")).unwrap(),
            catalog
        );

        set_accountless(&paths, false).unwrap();
        let store = load_store(&paths).unwrap();
        assert!(catalog_entry_for_slug(
            &paths.app_data_dir,
            &paths.codex_home,
            &store,
            "official-plus-model"
        )
        .is_some());
        assert_eq!(
            catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            config(&paths)["model"].as_str(),
            Some("access-mode-fixture::fixture-model")
        );
    }
    super::subagents::save(
        &paths,
        Some(super::subagents::Settings {
            model: Some("official-plus-model".into()),
            reasoning_effort: None,
        }),
    )
    .unwrap();
    let before = file_state(&paths);
    assert_eq!(
        set_accountless(&paths, true)
            .err()
            .expect("fixed official child is incompatible"),
        "newapi.errors.subagentModel"
    );
    assert_eq!(
        file_state(&paths),
        before,
        "rejected mode switch must preserve the saved configuration"
    );
    super::subagents::save(&paths, Some(super::subagents::Settings::default())).unwrap();
    set_accountless(&paths, true).unwrap();
    assert_eq!(
        config(&paths)["model"].as_str(),
        Some("access-mode-fixture::fixture-model")
    );
}

const ORIGINAL_CONFIG: &str = r#"# Existing user configuration
model = 'original-model'
model_provider = 'user-provider'
model_catalog_json = 'original-catalog.json'
model_context_window = 65536
model_auto_compact_token_limit = 60000
preferred_auth_method = 'chatgpt'
user_setting = 'preserved'

[model_providers.user-provider]
name = 'User provider'
base_url = 'https://original.example.invalid/v1'
wire_api = 'responses'
env_key = 'USER_PROVIDER_KEY'

[unrelated]
enabled = true
"#;
const ORIGINAL_ENV: &[u8] = b"# User-owned environment\r\nUSER_PROVIDER_KEY='synthetic-existing-key'\r\nUNCHANGED_SETTING=kept";
const ORIGINAL_AUTH: &[u8] = b"{\r\n  \"auth_mode\": \"chatgpt\", \"tokens\": {\"access_token\": \"synthetic-preserved-token\"}\r\n}\r\n";
const ROOT_FIELDS: [&str; 6] = [
    "model",
    "model_provider",
    "model_catalog_json",
    "model_context_window",
    "model_auto_compact_token_limit",
    "preferred_auth_method",
];

fn fixture() -> (tempfile::TempDir, CodexPaths) {
    let directory = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(directory.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.config_path, ORIGINAL_CONFIG).unwrap();
    fs::write(paths.codex_home.join(".env"), ORIGINAL_ENV).unwrap();
    fs::write(&paths.auth_path, ORIGINAL_AUTH).unwrap();
    let provider = Provider {
        id: "access-mode-fixture".into(),
        name: "Access mode fixture".into(),
        base_url: "https://fixture.example.invalid/".into(),
        api_key: "synthetic-native-mode-key".into(),
        enabled: true,
        protocol: ModelProtocol::Responses,
        model: "fixture-model".into(),
        context_window: Some(32768),
        auto_compact_limit: Some(24000),
        models: vec![newapi::Model {
            id: "fixture-model".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let store = Store {
        version: STORE_VERSION,
        active_provider_id: Some(provider.id.clone()),
        providers: vec![provider],
        router: RouterSettings {
            bearer_token: "agentharbor_synthetic_mode_token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    save_store_unlocked(&paths, &store).unwrap();
    load_store(&paths).unwrap();
    (directory, paths)
}

fn optional_bytes(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("failed to read isolated fixture: {error}"),
    }
}

fn file_state(paths: &CodexPaths) -> [Option<Vec<u8>>; 4] {
    [
        optional_bytes(&store_path(paths)),
        optional_bytes(&paths.config_path),
        optional_bytes(&paths.codex_home.join(".env")),
        optional_bytes(&paths.auth_path),
    ]
}

fn config(paths: &CodexPaths) -> toml::Value {
    fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap()
}

fn assert_user_files_preserved(paths: &CodexPaths) {
    assert_eq!(
        fs::read(paths.codex_home.join(".env")).unwrap(),
        ORIGINAL_ENV
    );
    assert_eq!(fs::read(&paths.auth_path).unwrap(), ORIGINAL_AUTH);
}

fn assert_original_config(paths: &CodexPaths) {
    let original: toml::Value = ORIGINAL_CONFIG.parse().unwrap();
    let restored = config(paths);
    for field in ROOT_FIELDS {
        assert_eq!(restored.get(field), original.get(field), "restore {field}");
    }
    assert_eq!(
        restored, original,
        "unrelated config must also be preserved"
    );
}

#[test]
fn changing_mode_while_disabled_only_updates_the_store() {
    for with_existing_files in [true, false] {
        let (_directory, paths) = fixture();
        if !with_existing_files {
            fs::remove_file(&paths.config_path).unwrap();
            fs::remove_file(paths.codex_home.join(".env")).unwrap();
        }
        let before = file_state(&paths);
        for mode in [CodexAccessMode::Native, CodexAccessMode::Router] {
            let changed = set_access_mode(&paths, mode).unwrap();
            assert!(changed.applied.is_none());
            let stored = load_store(&paths).unwrap();
            assert_eq!(stored.codex_access_mode, mode);
            assert!(!stored.router.enabled);
            let after = file_state(&paths);
            assert_eq!(
                after[1..],
                before[1..],
                "disabled mode changes must not write client files"
            );
            assert!(!config_snapshot::path(&paths).exists());
        }
    }
}

#[test]
fn loading_a_saved_toggle_and_exiting_without_injection_leave_codex_files_untouched() {
    let (_directory, paths) = fixture();
    let mut store = load_store(&paths).unwrap();
    store.router.enabled = true;
    save_store_unlocked(&paths, &store).unwrap();
    let before = file_state(&paths);
    let state = state_from_store_at(&load_store(&paths).unwrap(), false, Some(&paths));
    assert!(state.router.config_drifted);
    assert_eq!(file_state(&paths), before);

    stop_smart_routing_on_exit(&paths).unwrap();
    let after = file_state(&paths);
    assert_eq!(after[1..], before[1..]);
    assert!(!load_store(&paths).unwrap().router.enabled);
    assert!(!config_snapshot::path(&paths).exists());
}

#[test]
fn editing_a_provider_after_cc_switch_takes_over_does_not_reinject() {
    let (_directory, paths) = fixture();
    set_smart_routing(&paths, true).unwrap();
    let mut external: toml_edit::DocumentMut = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap();
    external["model_provider"] = toml_edit::value("cc-switch");
    external["model_providers"]["cc-switch"] = toml_edit::Item::Table(toml_edit::Table::new());
    external["model_providers"]["cc-switch"]["base_url"] = toml_edit::value("https://cc.invalid");
    fs::write(&paths.config_path, external.to_string()).unwrap();
    let before = fs::read(&paths.config_path).unwrap();
    let result = set_accountless(&paths, true).unwrap();
    assert!(result.applied.is_none());
    assert_eq!(fs::read(&paths.config_path).unwrap(), before);

    stop_smart_routing_on_exit(&paths).unwrap();
    let cleaned = config(&paths);
    assert_eq!(cleaned["model_provider"].as_str(), Some("cc-switch"));
    assert_eq!(
        cleaned["model_providers"]["cc-switch"]["base_url"].as_str(),
        Some("https://cc.invalid")
    );
    assert!(!load_store(&paths).unwrap().router.enabled);
}

#[test]
fn foreign_entry_in_our_reserved_namespace_blocks_destructive_enable() {
    for key in [
        "agentharbor_relay",
        "agentharbor_p_foreign",
        "codexharbor_relay",
        "codexharbor_p_foreign",
    ] {
        let (_directory, paths) = fixture();
        let original = fs::read_to_string(&paths.config_path).unwrap();
        let collision =
            format!("{original}\n[model_providers.{key}]\nbase_url = 'https://foreign.invalid'\n");
        fs::write(&paths.config_path, &collision).unwrap();
        assert_eq!(
            set_smart_routing(&paths, true).unwrap_err(),
            "newapi.errors.routerSnapshot"
        );
        assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), collision);
        assert!(!config_snapshot::path(&paths).exists());
    }
}

/// Account-mode routing keeps Codex's built-in provider and moves its endpoint.
fn assert_builtin_route(paths: &CodexPaths) {
    let installed = config(paths);
    assert!(installed
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .is_none_or(|provider| provider == "openai"));
    let store = load_store(paths).unwrap();
    assert_eq!(
        installed["openai_base_url"].as_str(),
        Some(builtin_router_base(&store).as_str())
    );
}

#[test]
fn router_native_router_then_disable_restores_six_fields_and_user_files() {
    let (_directory, paths) = fixture();
    set_smart_routing(&paths, true).unwrap();
    assert_builtin_route(&paths);
    assert_user_files_preserved(&paths);
    let native = set_access_mode(&paths, CodexAccessMode::Native).unwrap();
    assert!(native.applied.is_some());
    assert_eq!(native.state.router.access_mode, CodexAccessMode::Native);
    let installed = config(&paths);
    assert_eq!(
        installed["model_provider"].as_str(),
        Some(NATIVE_PROVIDER_KEY)
    );
    assert_eq!(installed["model"].as_str(), Some("fixture-model"));
    assert_eq!(installed["preferred_auth_method"].as_str(), Some("apikey"));
    let native_env = fs::read(paths.codex_home.join(".env")).unwrap();
    assert!(native_env.starts_with(ORIGINAL_ENV));
    assert!(String::from_utf8(native_env)
        .unwrap()
        .contains("synthetic-native-mode-key"));
    assert_eq!(fs::read(&paths.auth_path).unwrap(), ORIGINAL_AUTH);
    let routed = set_access_mode(&paths, CodexAccessMode::Router).unwrap();
    assert!(routed.applied.is_some());
    assert_builtin_route(&paths);
    assert_user_files_preserved(&paths);
    let disabled = set_smart_routing(&paths, false).unwrap();
    assert!(!disabled.state.router.enabled);
    assert_original_config(&paths);
    assert_user_files_preserved(&paths);
    assert!(!config_snapshot::path(&paths).exists());
}

#[test]
fn repeated_mode_selection_does_not_rewrite_files_or_rotate_native_credentials() {
    let (_directory, paths) = fixture();
    set_smart_routing(&paths, true).unwrap();
    for mode in [CodexAccessMode::Router, CodexAccessMode::Native] {
        set_access_mode(&paths, mode).unwrap();
        let before = file_state(&paths);
        let repeated = set_access_mode(&paths, mode).unwrap();
        assert!(repeated.applied.is_none());
        assert_eq!(file_state(&paths), before);
        if mode == CodexAccessMode::Native {
            let store = load_store(&paths).unwrap();
            assert!(ensure_routing_config(&paths, &store).unwrap().is_none());
            apply_routing_config(&paths, &store).unwrap();
            assert_eq!(
                file_state(&paths),
                before,
                "idempotent Native reconciliation must retain its env_key"
            );
        }
    }
}

#[test]
fn incompatible_native_selection_preserves_store_config_environment_and_auth() {
    for protocol in [
        ModelProtocol::ChatCompletions,
        ModelProtocol::AnthropicMessages,
    ] {
        let (_directory, paths) = fixture();
        let mut store = load_store(&paths).unwrap();
        let mut fallback = store.providers[0].clone();
        fallback.id = "responses-fallback".into();
        store.providers.push(fallback);
        store.providers[0].models[0].protocol = Some(protocol);
        save_store_unlocked(&paths, &store).unwrap();
        set_smart_routing(&paths, true).unwrap();
        let before = file_state(&paths);
        assert_eq!(
            set_access_mode(&paths, CodexAccessMode::Native)
                .err()
                .as_deref(),
            Some("newapi.errors.nativeResponsesRequired")
        );
        assert_eq!(
            file_state(&paths),
            before,
            "must neither fall back nor partially change modes"
        );
    }
}

#[test]
fn entering_native_pins_the_provider_actually_used_after_router_fallback() {
    let (_directory, paths) = fixture();
    let mut store = load_store(&paths).unwrap();
    let stale_id = store.providers[0].id.clone();
    let mut replacement = store.providers[0].clone();
    replacement.id = "replacement-provider".into();
    replacement.model = "replacement-model".into();
    replacement.models = vec![newapi::Model {
        id: replacement.model.clone(),
        ..Default::default()
    }];
    let replacement_id = replacement.id.clone();
    store.providers[0].enabled = false;
    store.providers.push(replacement);
    save_store_unlocked(&paths, &store).unwrap();
    assert_eq!(
        effective_active_for(&store, ModelProtocol::Responses)
            .unwrap()
            .id,
        replacement_id
    );
    let routed = set_smart_routing(&paths, true).unwrap();
    assert_eq!(
        routed.state.active_provider_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        load_store(&paths).unwrap().active_provider_id.as_deref(),
        Some(stale_id.as_str())
    );
    assert_eq!(
        config(&paths)["model"].as_str(),
        Some("replacement-provider::replacement-model")
    );

    let native = set_access_mode(&paths, CodexAccessMode::Native).unwrap();
    assert!(native.applied.is_some());
    assert_eq!(
        native.state.active_provider_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        load_store(&paths).unwrap().active_provider_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        config(&paths)["model_provider"].as_str(),
        Some(NATIVE_PROVIDER_KEY)
    );
    assert_eq!(config(&paths)["model"].as_str(), Some("replacement-model"));
    set_smart_routing(&paths, false).unwrap();
    assert_original_config(&paths);
    assert_user_files_preserved(&paths);
}

#[test]
fn a_native_provider_without_a_default_model_enables_without_taking_the_pin() {
    let (_directory, paths) = fixture();
    let mut store = load_store(&paths).unwrap();
    let mut unpinned = store.providers[0].clone();
    unpinned.id = "no-default-provider".into();
    unpinned.model.clear();
    unpinned.enabled = false;
    store.codex_access_mode = CodexAccessMode::Native;
    store.providers[0].enabled = false;
    store.providers.push(unpinned);
    save_store_unlocked(&paths, &store).unwrap();

    set_enabled(&paths, "no-default-provider", true).unwrap();
    let saved = load_store(&paths).unwrap();
    assert!(saved.providers[1].enabled);
    assert_ne!(saved.active_provider_id.as_deref(), Some("no-default-provider"));
}

#[test]
fn enabling_a_native_provider_replaces_a_disabled_pin_before_connecting() {
    let (_directory, paths) = fixture();
    let mut store = load_store(&paths).unwrap();
    let mut replacement = store.providers[0].clone();
    replacement.id = "new-native-provider".into();
    replacement.model = "new-native-model".into();
    replacement.models = vec![newapi::Model {
        id: replacement.model.clone(),
        ..Default::default()
    }];
    replacement.enabled = false;
    let replacement_id = replacement.id.clone();
    store.codex_access_mode = CodexAccessMode::Native;
    store.providers[0].enabled = false;
    store.providers.push(replacement);
    save_store_unlocked(&paths, &store).unwrap();
    let before = file_state(&paths);

    let enabled = set_enabled(&paths, &replacement_id, true).unwrap();
    assert!(enabled.applied.is_none());
    let saved = load_store(&paths).unwrap();
    assert_eq!(saved.codex_access_mode, CodexAccessMode::Native);
    assert!(!saved.router.enabled);
    assert!(!saved.providers[0].enabled);
    assert!(saved.providers[1].enabled);
    assert_eq!(
        saved.active_provider_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        file_state(&paths)[1..],
        before[1..],
        "enabling a provider while disconnected must not write client files"
    );

    let connected = set_smart_routing(&paths, true).unwrap();
    assert!(connected.applied.is_some());
    assert!(connected.state.router.enabled);
    assert_eq!(
        connected.state.active_provider_id.as_deref(),
        Some(replacement_id.as_str())
    );
    assert_eq!(
        config(&paths)["model_provider"].as_str(),
        Some(NATIVE_PROVIDER_KEY)
    );
    assert_eq!(config(&paths)["model"].as_str(), Some("new-native-model"));
    set_smart_routing(&paths, false).unwrap();
    assert_original_config(&paths);
    assert_user_files_preserved(&paths);
}

#[test]
fn invalid_toml_mode_switch_preserves_all_client_and_store_bytes() {
    for current in [CodexAccessMode::Router, CodexAccessMode::Native] {
        let (_directory, paths) = fixture();
        set_access_mode(&paths, current).unwrap();
        set_smart_routing(&paths, true).unwrap();
        fs::write(&paths.config_path, b"# user edit in progress\nmodel = [\n").unwrap();
        let before = file_state(&paths);
        let target = if current == CodexAccessMode::Router {
            CodexAccessMode::Native
        } else {
            CodexAccessMode::Router
        };
        assert!(set_access_mode(&paths, target).is_err());
        assert_eq!(
            file_state(&paths),
            before,
            "failed switch must not write config, store, env, or auth"
        );
    }
}

#[test]
fn store_write_failure_rolls_back_native_configuration() {
    for previously_enabled in [false, true] {
        let (_directory, paths) = fixture();
        if previously_enabled {
            set_smart_routing(&paths, true).unwrap();
        }
        let previous = load_store(&paths).unwrap();
        let previous_config = config(&paths);
        let previous_store_bytes = fs::read(store_path(&paths)).unwrap();
        let mut proposed = previous.clone();
        proposed.codex_access_mode = CodexAccessMode::Native;
        proposed.router.enabled = true;
        apply_routing_config(&paths, &proposed).unwrap();
        assert_eq!(
            config(&paths)["model_provider"].as_str(),
            Some(NATIVE_PROVIDER_KEY)
        );
        // Fail the final store commit after config application, without changing
        // filesystem permissions or affecting any path outside the temp fixture.
        let store_file = store_path(&paths);
        fs::remove_file(&store_file).unwrap();
        fs::create_dir(&store_file).unwrap();
        assert!(persist_connection_change(&paths, &previous, &proposed, true).is_err());
        assert!(store_file.is_dir());
        assert_eq!(config(&paths), previous_config);
        assert_user_files_preserved(&paths);
        fs::remove_dir(&store_file).unwrap();
        fs::write(&store_file, previous_store_bytes).unwrap();
        assert_eq!(
            load_store(&paths).unwrap().codex_access_mode,
            CodexAccessMode::Router
        );
        if previously_enabled {
            set_smart_routing(&paths, false).unwrap();
        }
        assert_original_config(&paths);
        assert_user_files_preserved(&paths);
    }
}
