use super::*;
use rusqlite::Connection;

const OFFICIAL: &str = "01999999-1111-7222-8333-444444444444";
const LEGACY: &str = "02999999-1111-7222-8333-444444444444";
const FOREIGN: &str = "03999999-1111-7222-8333-444444444444";
const ORIGINAL: &str = "model = 'official-fixture'\ntheme = 'light'\n";

#[derive(Default)]
struct FakeCodex {
    running: bool,
    stop_error: Option<String>,
    reopen_error: Option<String>,
    stops: usize,
    reopens: usize,
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
        self.running = false;
        Ok(())
    }
    fn reopen(&mut self, _paths: &CodexPaths) -> Result<(), String> {
        self.reopens += 1;
        if let Some(error) = &self.reopen_error {
            return Err(error.clone());
        }
        self.running = true;
        Ok(())
    }
}

fn running() -> FakeCodex {
    FakeCodex {
        running: true,
        ..Default::default()
    }
}

fn fixture(accountless: bool) -> (tempfile::TempDir, CodexPaths) {
    let directory = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(directory.path().join("codex"));
    paths.ensure_directories().unwrap();
    std::fs::write(&paths.config_path, ORIGINAL).unwrap();
    let connection = Connection::open(&paths.codex_state_db_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, model TEXT);",
        )
        .unwrap();
    for (id, provider, model) in [
        (OFFICIAL, "openai", "transition-fixture::response-model"),
        (LEGACY, CATALOG_PROVIDER_KEY, "official-fixture"),
        (FOREIGN, "custom", "custom::model"),
    ] {
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3)",
                (id, provider, model),
            )
            .unwrap();
    }
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
            accountless,
            port: 18_765,
            bearer_token: "synthetic-transition-token".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let _guard = store_write_guard().unwrap();
    save_store_unlocked(&paths, &store).unwrap();
    (directory, paths)
}

fn row(paths: &CodexPaths, id: &str) -> (String, Option<String>) {
    Connection::open(&paths.codex_state_db_path)
        .unwrap()
        .query_row(
            "SELECT model_provider, model FROM threads WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn document(paths: &CodexPaths) -> toml_edit::DocumentMut {
    std::fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap()
}

fn original(paths: &CodexPaths) -> String {
    std::fs::read_to_string(&paths.config_path).unwrap()
}

fn change(paths: &CodexPaths, target: CodexConnectionTarget) -> CodexConnectionChange {
    change_with_process(paths, target, &mut FakeCodex::default(), false).unwrap()
}

#[test]
fn enabling_points_the_builtin_provider_at_the_router_and_keeps_threads() {
    let (_directory, paths) = fixture(false);
    let change = change(&paths, CodexConnectionTarget::Enabled);
    assert!(!change.restart_required);
    let config = document(&paths);
    assert_eq!(
        config["openai_base_url"].as_str(),
        Some("http://127.0.0.1:18765/openai/synthetic-transition-token/v1")
    );
    assert!(
        config.get("model_provider").is_none(),
        "threads keep the built-in provider"
    );
    assert!(config
        .get("model_providers")
        .and_then(|providers| providers.get(CATALOG_PROVIDER_KEY))
        .is_none());
    assert_eq!(
        config["model_catalog_json"].as_str(),
        Some(catalog_path(&paths).to_string_lossy().as_ref())
    );
    assert!(routing_config_matches(&paths, &load_store(&paths).unwrap()).unwrap());
    assert!(routing_config_intact(&paths));
    // A relay model on an official thread is valid while routing is on; a row
    // left on an old router provider returns to the built-in provider.
    assert_eq!(
        row(&paths, OFFICIAL),
        (
            "openai".into(),
            Some("transition-fixture::response-model".into())
        )
    );
    assert_eq!(row(&paths, LEGACY).0, "openai");
    assert_eq!(
        row(&paths, FOREIGN),
        ("custom".into(), Some("custom::model".into()))
    );
}

#[test]
fn a_running_codex_is_left_alone_until_the_user_restarts_it() {
    let (_directory, paths) = fixture(false);
    let mut codex = running();
    let change =
        change_with_process(&paths, CodexConnectionTarget::Enabled, &mut codex, false).unwrap();
    assert!(change.restart_required);
    assert_eq!((codex.stops, codex.reopens), (0, 0));
    assert!(document(&paths).get("openai_base_url").is_some());
    // A live Codex may rewrite its catalog rows; they are normalized when it
    // next stops.
    assert_eq!(row(&paths, LEGACY).0, CATALOG_PROVIDER_KEY);
    normalize_threads(&paths).unwrap();
    assert_eq!(row(&paths, LEGACY).0, "openai");
}

#[test]
fn disabling_restores_the_original_config_and_relay_selections() {
    let (_directory, paths) = fixture(false);
    change(&paths, CodexConnectionTarget::Enabled);
    let change = change(&paths, CodexConnectionTarget::Disabled);
    assert!(!change.change.state.router.enabled);
    assert_eq!(original(&paths), ORIGINAL);
    assert!(!config_snapshot::path(&paths).exists());
    // The relay model cannot work without the router; Codex falls back to the
    // configured default. Other tools' threads are left alone.
    assert_eq!(row(&paths, OFFICIAL), ("openai".into(), None));
    assert_eq!(
        row(&paths, FOREIGN),
        ("custom".into(), Some("custom::model".into()))
    );
}

#[test]
fn exit_restarts_a_running_codex_once_and_never_fails_on_it() {
    for (stop_error, reopen_error) in [
        (None, None),
        (Some("synthetic quit timeout"), None),
        (None, Some("synthetic launch failure")),
    ] {
        let (_directory, paths) = fixture(false);
        change(&paths, CodexConnectionTarget::Enabled);
        let mut codex = FakeCodex {
            running: true,
            stop_error: stop_error.map(Into::into),
            reopen_error: reopen_error.map(Into::into),
            ..Default::default()
        };
        let change =
            change_with_process(&paths, CodexConnectionTarget::Disabled, &mut codex, true).unwrap();
        assert_eq!(original(&paths), ORIGINAL);
        assert_eq!(codex.stops, 1);
        assert_eq!(codex.reopens, usize::from(stop_error.is_none()));
        assert_eq!(change.restart_required, stop_error.is_some());
        // Rows are normalized only while Codex is actually stopped.
        let expected = stop_error.map(|_| "transition-fixture::response-model".to_owned());
        assert_eq!(row(&paths, OFFICIAL).1, expected);
    }
}

#[test]
fn accountless_routing_keeps_its_dedicated_provider_threads() {
    let (_directory, paths) = fixture(true);
    change(&paths, CodexConnectionTarget::Enabled);
    let config = document(&paths);
    assert_eq!(
        config["model_provider"].as_str(),
        Some(CATALOG_PROVIDER_KEY)
    );
    assert!(config.get("openai_base_url").is_none());
    assert_eq!(row(&paths, LEGACY).0, CATALOG_PROVIDER_KEY);
    change(&paths, CodexConnectionTarget::Disabled);
    assert_eq!(original(&paths), ORIGINAL);
    assert_eq!(row(&paths, LEGACY).0, "openai");
}

#[test]
fn exit_cleanup_releases_the_router_without_waiting_on_threads() {
    let (_directory, paths) = fixture(false);
    change(&paths, CodexConnectionTarget::Enabled);
    stop_smart_routing_on_exit(&paths).unwrap();
    assert_eq!(original(&paths), ORIGINAL);
    assert!(!load_store(&paths).unwrap().router.enabled);
}

#[test]
fn a_missing_sidecar_still_releases_only_our_endpoint() {
    let (_directory, paths) = fixture(false);
    change(&paths, CodexConnectionTarget::Enabled);
    std::fs::remove_file(config_snapshot::path(&paths)).unwrap();
    let mut config = document(&paths);
    config["model"] = toml_edit::value("transition-fixture::response-model");
    std::fs::write(&paths.config_path, config.to_string()).unwrap();
    stop_smart_routing_on_exit(&paths).unwrap();
    let config = document(&paths);
    assert!(config.get("openai_base_url").is_none());
    assert!(config.get("model").is_none());
    assert!(config.get("model_catalog_json").is_none());
    assert_eq!(config["theme"].as_str(), Some("light"));
}

#[test]
fn switching_to_native_removes_the_builtin_endpoint() {
    let (_directory, paths) = fixture(false);
    change(&paths, CodexConnectionTarget::Enabled);
    change(&paths, CodexConnectionTarget::Native);
    let config = document(&paths);
    assert!(config.get("openai_base_url").is_none());
    assert_eq!(config["model_provider"].as_str(), Some(NATIVE_PROVIDER_KEY));
    change(&paths, CodexConnectionTarget::Router);
    assert!(document(&paths).get("openai_base_url").is_some());
    change(&paths, CodexConnectionTarget::Disabled);
    assert_eq!(original(&paths), ORIGINAL);
}
