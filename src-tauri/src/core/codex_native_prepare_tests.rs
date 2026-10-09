use super::*;
use crate::core::router::{self, CodexAccessMode, RouterSettings};

const ORIGINAL: &str = "model = 'original-model'\nmodel_provider = 'custom'\npreferred_auth_method = 'chatgpt'\ntheme = 'light'\n[model_providers.custom]\nbase_url = 'https://custom.example.invalid/v1'\n[agents]\nmax_threads = 4\n";

fn fixture() -> (tempfile::TempDir, CodexPaths, Store) {
    let directory = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(directory.path().join("codex"));
    paths.ensure_directories().unwrap();
    std::fs::write(&paths.config_path, ORIGINAL).unwrap();
    std::fs::write(&paths.auth_path, b"synthetic-auth-preserved").unwrap();
    std::fs::write(env_path(&paths), "USER_KEY='untouched'\n").unwrap();
    let store = Store {
        providers: vec![Provider {
            id: "prepare-fixture".into(),
            name: "Preparation fixture".into(),
            base_url: "https://native.example.invalid/v1".into(),
            api_key: "synthetic-prepared-key".into(),
            model: "response-model".into(),
            enabled: true,
            models: vec![newapi::Model {
                id: "response-model".into(),
                protocol: Some(ModelProtocol::Responses),
                context_window: Some(100_000),
                metadata_source: "upstream".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        active_provider_id: Some("prepare-fixture".into()),
        codex_access_mode: CodexAccessMode::Native,
        router: RouterSettings {
            enabled: true,
            accountless: true,
            bearer_token: "synthetic-local-preparation-token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut routed = store.clone();
    routed.codex_access_mode = CodexAccessMode::Router;
    router::apply_routing_config(&paths, &routed).unwrap();
    (directory, paths, store)
}

fn read_config(paths: &CodexPaths) -> DocumentMut {
    std::fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn preparation_preserves_router_ownership_and_reuses_the_destination() {
    let (_directory, paths, store) = fixture();
    let before = read_config(&paths);
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    prepare_provider(&paths, &store).unwrap();
    let mut after = read_config(&paths);
    let native = after["model_providers"][PROVIDER_KEY].clone();
    assert_eq!(
        native["base_url"].as_str(),
        Some("https://native.example.invalid/v1")
    );
    assert_eq!(native["requires_openai_auth"].as_bool(), Some(false));
    after["model_providers"]
        .as_table_like_mut()
        .unwrap()
        .remove(PROVIDER_KEY);
    assert_eq!(
        after.to_string().parse::<toml::Value>().unwrap(),
        before.to_string().parse::<toml::Value>().unwrap()
    );
    assert_eq!(
        std::fs::read(config_snapshot::path(&paths)).unwrap(),
        snapshot
    );
    let env = std::fs::read(env_path(&paths)).unwrap();
    let credentials = EnvFile::load(&paths).unwrap();
    assert_eq!(credentials.owned.len(), 1);
    assert_eq!(credentials.owned[0].key, store.providers[0].api_key);
    assert_eq!(
        native["env_key"].as_str(),
        Some(credentials.owned[0].name.as_str())
    );
    assert!(catalog_path(&paths, credentials.owned[0].id).is_file());
    let prepared = std::fs::read(&paths.config_path).unwrap();
    assert!(prepare_provider(&paths, &store)
        .unwrap()
        .backup_path
        .is_none());
    assert_eq!(std::fs::read(&paths.config_path).unwrap(), prepared);
    assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
    assert_eq!(
        std::fs::read(&paths.auth_path).unwrap(),
        b"synthetic-auth-preserved"
    );
}

#[test]
fn final_selection_keeps_the_prepared_credential_and_the_original_baseline() {
    let (_directory, paths, store) = fixture();
    prepare_provider(&paths, &store).unwrap();
    let prepared_name = read_config(&paths)["model_providers"][PROVIDER_KEY]["env_key"]
        .as_str()
        .unwrap()
        .to_owned();
    apply(&paths, &store).unwrap();
    let native = read_config(&paths);
    assert_eq!(native["model_provider"].as_str(), Some(PROVIDER_KEY));
    assert!(native["model_providers"]
        .get(router::CATALOG_PROVIDER_KEY)
        .is_none());
    assert_eq!(
        native["model_providers"][PROVIDER_KEY]["env_key"].as_str(),
        Some(prepared_name.as_str())
    );
    assert!(intact(&paths, &store).unwrap());
    router::remove_routing_config(&paths, &store).unwrap();
    assert_eq!(
        read_config(&paths)
            .to_string()
            .parse::<toml::Value>()
            .unwrap(),
        ORIGINAL.parse::<toml::Value>().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(env_path(&paths)).unwrap(),
        "USER_KEY='untouched'\n"
    );
}

#[test]
fn invalid_destination_or_external_takeover_does_not_remove_the_router() {
    for external in [false, true] {
        let (_directory, paths, mut store) = fixture();
        if external {
            let mut document = read_config(&paths);
            document["model_provider"] = toml_edit::value("custom");
            std::fs::write(&paths.config_path, document.to_string()).unwrap();
        } else {
            store.providers[0].model = "chat-only".into();
            store.providers[0].models[0].id = "chat-only".into();
            store.providers[0].models[0].protocol = Some(ModelProtocol::ChatCompletions);
        }
        let config = std::fs::read(&paths.config_path).unwrap();
        let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
        let env = std::fs::read(env_path(&paths)).unwrap();
        assert!(prepare_provider(&paths, &store).is_err());
        assert_eq!(std::fs::read(&paths.config_path).unwrap(), config);
        assert_eq!(
            std::fs::read(config_snapshot::path(&paths)).unwrap(),
            snapshot
        );
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
    }
}

#[test]
fn preparation_settles_a_committed_snapshot_before_changing_the_config_hash() {
    let (_directory, paths, store) = fixture();
    let current = std::fs::read_to_string(&paths.config_path).unwrap();
    let mut document = current.parse::<DocumentMut>().unwrap();
    let mut snapshot = config_snapshot::load(&paths, &current).unwrap().unwrap();
    snapshot.apply_auth(&mut document, true).unwrap();
    snapshot
        .prepare(&paths, ORIGINAL, &document, config_snapshot::Action::Apply)
        .unwrap();
    prepare_provider(&paths, &store).unwrap();
    let prepared = std::fs::read_to_string(&paths.config_path).unwrap();
    assert!(config_snapshot::load(&paths, &prepared).unwrap().is_some());
    apply(&paths, &store).unwrap();
    router::remove_routing_config(&paths, &store).unwrap();
    assert_eq!(
        read_config(&paths)
            .to_string()
            .parse::<toml::Value>()
            .unwrap(),
        ORIGINAL.parse::<toml::Value>().unwrap()
    );
}

#[test]
fn failed_preparation_backup_keeps_the_router_and_cleans_only_the_staged_key() {
    let (_directory, paths, store) = fixture();
    let backup_directory = paths.app_data_dir.join("config-backups");
    for entry in std::fs::read_dir(&backup_directory).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(&backup_directory).unwrap();
    std::fs::write(&backup_directory, "synthetic backup write blocker").unwrap();
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    let env = std::fs::read(env_path(&paths)).unwrap();
    assert!(prepare_provider(&paths, &store).is_err());
    assert_eq!(std::fs::read(&paths.config_path).unwrap(), config);
    assert_eq!(
        std::fs::read(config_snapshot::path(&paths)).unwrap(),
        snapshot
    );
    assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
}
