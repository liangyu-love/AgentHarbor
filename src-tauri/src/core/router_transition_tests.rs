use super::*;
use rusqlite::Connection;
use sha2::Digest;
use std::cell::Cell;

const FIRST: &str = "01999999-1111-7222-8333-444444444444";
const SECOND: &str = "02999999-1111-7222-8333-444444444444";
const THIRD: &str = "03999999-1111-7222-8333-444444444444";
const ORIGINAL: &str = "model_provider = 'openai'\nmodel = 'official-fixture'\ntheme = 'light'\n";

struct FakeCodex {
    running: bool,
    stop_error: Option<String>,
    reopen_error: Option<String>,
    stops: usize,
    ensures: Cell<usize>,
    reopens: usize,
    on_stop: Option<Box<dyn FnOnce()>>,
}

impl Default for FakeCodex {
    fn default() -> Self {
        Self {
            running: true,
            stop_error: None,
            reopen_error: None,
            stops: 0,
            ensures: Cell::new(0),
            reopens: 0,
            on_stop: None,
        }
    }
}

impl CodexProcess for FakeCodex {
    fn running(&self) -> bool {
        self.running
    }
    fn stop(&mut self) -> Result<(), String> {
        self.stops += 1;
        if let Some(error) = &self.stop_error {
            return Err(error.clone());
        }
        if let Some(flush) = self.on_stop.take() {
            flush();
        }
        self.running = false;
        Ok(())
    }
    fn ensure_stopped(&self) -> Result<(), String> {
        self.ensures.set(self.ensures.get() + 1);
        if self.running {
            Err("synthetic writer is still running".into())
        } else {
            Ok(())
        }
    }
    fn reopen(&mut self, _paths: &CodexPaths) -> Result<(), String> {
        self.reopens += 1;
        if let Some(error) = &self.reopen_error {
            return Err(error.clone());
        }
        self.running = true;
        Ok(())
    }
    fn check_writers(&self) -> bool {
        false
    }
}

fn fixture() -> (tempfile::TempDir, CodexPaths) {
    let directory = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(directory.path().join("codex"));
    paths.ensure_directories().unwrap();
    std::fs::create_dir_all(&paths.sessions_dir).unwrap();
    std::fs::write(&paths.config_path, ORIGINAL).unwrap();
    Connection::open(&paths.codex_state_db_path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, rollout_path TEXT);",
        )
        .unwrap();
    let store = Store {
        providers: vec![Provider {
            id: "transition-fixture".into(),
            name: "Synthetic destination".into(),
            base_url: "https://transition.example.invalid/v1".into(),
            api_key: "synthetic-transition-secret".into(),
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
        active_provider_id: Some("transition-fixture".into()),
        router: RouterSettings {
            enabled: true,
            accountless: true,
            bearer_token: "synthetic-transition-token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let _guard = store_write_guard().unwrap();
    apply_routing_config(&paths, &store).unwrap();
    save_store_unlocked(&paths, &store).unwrap();
    (directory, paths)
}

fn add_thread(paths: &CodexPaths, id: &str, provider: &str) {
    let rollout = paths.sessions_dir.join(format!("rollout-{id}.jsonl"));
    let meta = serde_json::json!({"timestamp":"2026-10-08T00:00:00Z","type":"session_meta",
        "payload":{"id":id,"model_provider":provider,"cwd":"C:/synthetic"}});
    std::fs::write(&rollout, format!("{meta}\n")).unwrap();
    Connection::open(&paths.codex_state_db_path)
        .unwrap()
        .execute(
            "INSERT INTO threads (id, model_provider, rollout_path) VALUES (?1, ?2, ?3)",
            (id, provider, rollout.display().to_string()),
        )
        .unwrap();
}

fn provider_of(paths: &CodexPaths, id: &str) -> String {
    Connection::open(&paths.codex_state_db_path)
        .unwrap()
        .query_row(
            "SELECT model_provider FROM threads WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

fn document(paths: &CodexPaths) -> toml_edit::DocumentMut {
    std::fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap()
}

fn retained(paths: &CodexPaths, config: &[u8], snapshot: &[u8]) {
    assert_eq!(std::fs::read(&paths.config_path).unwrap(), config);
    assert_eq!(
        std::fs::read(config_snapshot::path(paths)).unwrap(),
        snapshot
    );
    assert!(load_store(paths).unwrap().router.enabled);
}

fn prepared_insertion_journal(paths: &CodexPaths) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version": 2,
        "threadId": FIRST,
        "backupPath": paths.app_data_dir.join("thread-recovery-backups/pending.json").display().to_string(),
        "configGeneration": format!("{:x}", sha2::Sha256::digest(std::fs::read(&paths.config_path).unwrap())),
        "stage": "prepared",
        "mutation": {
            "type": "insertRow",
            "schema_signature": "synthetic-schema",
            "columns": [],
            "row_fingerprint": "synthetic-row"
        }
    })).unwrap()
}

#[test]
#[cfg(target_os = "windows")]
fn disable_restores_catalog_and_rollout_before_removing_the_provider() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    let mut process = FakeCodex::default();
    let result =
        change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process).unwrap();
    assert!(!result.change.state.router.enabled);
    assert_eq!(result.thread_migration.unwrap().changed, 1);
    assert_eq!(provider_of(&paths, FIRST), "openai");
    let rollout: Value = serde_json::from_str(
        std::fs::read_to_string(paths.sessions_dir.join(format!("rollout-{FIRST}.jsonl")))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(rollout["payload"]["model_provider"], "openai");
    assert_eq!(document(&paths)["model_provider"].as_str(), Some("openai"));
    assert!(document(&paths)
        .get("model_providers")
        .and_then(|p| p.get(CATALOG_PROVIDER_KEY))
        .is_none());
    assert!(!config_snapshot::path(&paths).exists());
    assert!(
        std::fs::read_dir(paths.app_data_dir.join("thread-recovery-backups"))
            .unwrap()
            .count()
            >= 1
    );
    assert_eq!(
        (process.stops, process.ensures.get(), process.reopens),
        (1, 2, 1)
    );
}

#[test]
fn failed_stop_keeps_the_store_and_owned_configuration() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    let mut process = FakeCodex {
        stop_error: Some("synthetic stop failure".into()),
        ..Default::default()
    };
    let error = change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process)
        .err()
        .unwrap();
    assert!(
        error.starts_with("CODEX_ROUTING_RESTORE_FAILED:"),
        "{error}"
    );
    retained(&paths, &config, &snapshot);
    assert_eq!(provider_of(&paths, FIRST), CATALOG_PROVIDER_KEY);
    assert_eq!(
        (process.stops, process.ensures.get(), process.reopens),
        (1, 0, 0)
    );
}

#[test]
fn one_bad_rollout_blocks_the_entire_preflight_without_partial_migration() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    add_thread(&paths, SECOND, CATALOG_PROVIDER_KEY);
    std::fs::write(
        paths.sessions_dir.join(format!("rollout-{SECOND}.jsonl")),
        "{truncated",
    )
    .unwrap();
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    let mut process = FakeCodex::default();
    assert!(
        change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process).is_err()
    );
    retained(&paths, &config, &snapshot);
    assert_eq!(provider_of(&paths, FIRST), CATALOG_PROVIDER_KEY);
    assert_eq!(provider_of(&paths, SECOND), CATALOG_PROVIDER_KEY);
    assert_eq!(process.reopens, 1);
    assert!(!paths.app_data_dir.join("thread-recovery-backups").exists());
}

#[test]
fn pending_recovery_keeps_its_configuration_generation_and_skips_process_stop() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    let journal = paths.app_data_dir.join("thread-recovery-journal.json");
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    let mut process = FakeCodex::default();
    for pending in [
        b"unfinished synthetic journal".to_vec(),
        prepared_insertion_journal(&paths),
    ] {
        std::fs::write(&journal, &pending).unwrap();
        assert!(
            change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process)
                .is_err()
        );
        retained(&paths, &config, &snapshot);
        assert_eq!(std::fs::read(&journal).unwrap(), pending);
    }
    assert_eq!(process.stops, 0);
}

#[test]
fn explicit_force_closes_without_migrating_or_deleting_session_recovery_data() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    let journal = paths.app_data_dir.join("thread-recovery-journal.json");
    std::fs::write(&journal, "unfinished synthetic journal").unwrap();
    let mut process = FakeCodex::default();
    let result =
        change_with_process(&paths, CodexConnectionTarget::Disabled, true, &mut process).unwrap();
    assert!(!result.change.state.router.enabled);
    assert!(result.thread_migration.is_none());
    assert_eq!(provider_of(&paths, FIRST), CATALOG_PROVIDER_KEY);
    assert!(paths
        .sessions_dir
        .join(format!("rollout-{FIRST}.jsonl"))
        .is_file());
    assert_eq!(
        std::fs::read_to_string(journal).unwrap(),
        "unfinished synthetic journal"
    );
    assert_eq!(document(&paths)["model_provider"].as_str(), Some("openai"));
    assert_eq!((process.stops, process.reopens), (0, 0));
}

#[test]
#[cfg(target_os = "windows")]
fn closing_includes_native_aggregate_and_legacy_provider_threads() {
    let (_directory, paths) = fixture();
    for (id, provider) in [
        (FIRST, NATIVE_PROVIDER_KEY),
        (SECOND, AGGREGATE_PROVIDER_KEY),
        (THIRD, PREVIOUS_CATALOG_PROVIDER_KEY),
    ] {
        add_thread(&paths, id, provider);
    }
    let mut process = FakeCodex::default();
    let result =
        change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process).unwrap();
    assert_eq!(result.thread_migration.unwrap().changed, 3);
    for id in [FIRST, SECOND, THIRD] {
        assert_eq!(provider_of(&paths, id), "openai");
    }
}

#[test]
#[cfg(target_os = "windows")]
fn native_switch_migrates_router_and_aggregate_rows_to_the_prepared_provider() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    add_thread(&paths, SECOND, AGGREGATE_PROVIDER_KEY);
    let mut process = FakeCodex::default();
    let result =
        change_with_process(&paths, CodexConnectionTarget::Native, false, &mut process).unwrap();
    assert_eq!(result.thread_migration.unwrap().changed, 2);
    assert_eq!(
        result.change.state.router.access_mode,
        CodexAccessMode::Native
    );
    for id in [FIRST, SECOND] {
        assert_eq!(provider_of(&paths, id), NATIVE_PROVIDER_KEY);
    }
    let config = document(&paths);
    assert_eq!(config["model_provider"].as_str(), Some(NATIVE_PROVIDER_KEY));
    assert!(config["model_providers"].get(NATIVE_PROVIDER_KEY).is_some());
    assert!(config["model_providers"]
        .get(CATALOG_PROVIDER_KEY)
        .is_none());
}

#[test]
#[cfg(target_os = "windows")]
fn successful_migration_returns_reopen_failure_without_undoing_the_close() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
    let mut process = FakeCodex {
        reopen_error: Some("synthetic launch failure".into()),
        ..Default::default()
    };
    let result =
        change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process).unwrap();
    let migration = result.thread_migration.unwrap();
    assert_eq!(migration.changed, 1);
    assert_eq!(
        migration.reopen_error.as_deref(),
        Some("synthetic launch failure")
    );
    assert!(!result.change.state.router.enabled);
    assert_eq!(provider_of(&paths, FIRST), "openai");
    assert_eq!(process.reopens, 1);
}

#[test]
#[cfg(target_os = "windows")]
fn first_thread_flushed_during_stop_is_migrated_before_closing() {
    let (_directory, paths) = fixture();
    let codex_home = paths.codex_home.clone();
    let mut process = FakeCodex {
        on_stop: Some(Box::new(move || {
            let paths = CodexPaths::from_home(codex_home);
            assert_eq!(
                document(&paths)["model_provider"].as_str(),
                Some(CATALOG_PROVIDER_KEY)
            );
            add_thread(&paths, FIRST, CATALOG_PROVIDER_KEY);
        })),
        ..Default::default()
    };
    let result =
        change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process).unwrap();
    assert_eq!(result.thread_migration.unwrap().changed, 1);
    assert_eq!(provider_of(&paths, FIRST), "openai");
    assert!(!load_store(&paths).unwrap().router.enabled);
}

#[test]
fn abrupt_exit_preserves_even_an_unindexed_pending_recovery() {
    let (_directory, paths) = fixture();
    let journal = paths.app_data_dir.join("thread-recovery-journal.json");
    std::fs::write(&journal, prepared_insertion_journal(&paths)).unwrap();
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    assert!(stop_smart_routing_on_exit(&paths).is_err());
    retained(&paths, &config, &snapshot);
    assert!(journal.is_file());
}

#[test]
fn abrupt_exit_cannot_remove_an_empty_catalogs_provider_while_a_writer_is_live() {
    let (_directory, paths) = fixture();
    let config = std::fs::read(&paths.config_path).unwrap();
    let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
    assert!(stop_smart_routing_on_exit_with_writer_check(&paths, || {
        Err("synthetic writer is still running".into())
    })
    .is_err());
    retained(&paths, &config, &snapshot);
}

#[test]
fn router_refresh_keeps_the_staged_native_provider_after_failed_migration() {
    let (_directory, paths) = fixture();
    add_thread(&paths, FIRST, NATIVE_PROVIDER_KEY);
    add_thread(&paths, SECOND, CATALOG_PROVIDER_KEY);
    std::fs::write(
        paths.sessions_dir.join(format!("rollout-{SECOND}.jsonl")),
        "{truncated",
    )
    .unwrap();
    let mut process = FakeCodex::default();
    assert!(
        change_with_process(&paths, CodexConnectionTarget::Native, false, &mut process).is_err()
    );
    assert_eq!(
        document(&paths)["model_provider"].as_str(),
        Some(CATALOG_PROVIDER_KEY)
    );
    assert!(document(&paths)["model_providers"]
        .get(NATIVE_PROVIDER_KEY)
        .is_some());
    let store = load_store(&paths).unwrap();
    apply_routing_config(&paths, &store).unwrap();
    assert!(document(&paths)["model_providers"]
        .get(NATIVE_PROVIDER_KEY)
        .is_some());
    assert_eq!(provider_of(&paths, FIRST), NATIVE_PROVIDER_KEY);
}

#[test]
fn unavailable_database_or_unknown_schema_keeps_routing_until_force_is_explicit() {
    for directory_instead_of_database in [true, false] {
        let (_directory, paths) = fixture();
        if directory_instead_of_database {
            std::fs::remove_file(&paths.codex_state_db_path).unwrap();
            std::fs::create_dir(&paths.codex_state_db_path).unwrap();
        } else {
            Connection::open(&paths.codex_state_db_path)
                .unwrap()
                .execute_batch("DROP TABLE threads; CREATE TABLE threads (id TEXT PRIMARY KEY);")
                .unwrap();
        }
        let config = std::fs::read(&paths.config_path).unwrap();
        let snapshot = std::fs::read(config_snapshot::path(&paths)).unwrap();
        let mut process = FakeCodex::default();
        let error =
            change_with_process(&paths, CodexConnectionTarget::Disabled, false, &mut process)
                .err()
                .unwrap();
        assert!(
            error.starts_with("CODEX_ROUTING_RESTORE_FAILED:"),
            "{error}"
        );
        retained(&paths, &config, &snapshot);
        assert_eq!(process.stops, 0);
        let result =
            change_with_process(&paths, CodexConnectionTarget::Disabled, true, &mut process)
                .unwrap();
        assert!(!result.change.state.router.enabled);
        assert!(result.thread_migration.is_none());
        assert_eq!(document(&paths)["model_provider"].as_str(), Some("openai"));
    }
}
