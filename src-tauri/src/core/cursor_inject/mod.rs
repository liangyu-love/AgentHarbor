//! Cursor injection: Cursor IDE and `cursor-agent` run their Agent, Tab and
//! commit messages on the models AgentHarbor routes.
//!
//! ```text
//! settings (cursor-injection.json) ── reconcile ──▶ engine (cursor-engine) on 127.0.0.1:<port>
//!   ide on  → Cursor's settings.json: http.proxy = engine        (ide::Lease)
//!   cli on  → user variable CURSOR_API_ENDPOINT = engine         (cli::Lease)
//!   models  → gateway models of the kinds turned on, via router /mix
//! ```
//! The engine runs while either is on; the router runs with it. On exit the
//! account, IDE and CLI leases are released.
pub mod account;
pub mod cli;
pub mod ide;
pub mod models;
pub mod search;

use std::{path::Path, sync::Arc};

use cursor_engine::{
    access::{ca, TrustState},
    CommitLocale, CommitSettings, Egress, Engine, EngineConfig, EngineSettings, Host, TabMode,
};
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        gateway::ModelKind,
        models::ApiProxyMode,
        models_dev, newapi,
        router::{self, ModelSources},
    },
    platform::paths::CodexPaths,
};

const SETTINGS_FILE: &str = "cursor-injection.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorSettings {
    /// Cursor IDE goes through the engine.
    pub ide: bool,
    /// `cursor-agent` goes through the engine.
    pub cli: bool,
    /// The kinds of model offered to Cursor.
    pub sources: ModelSources,
    /// The engine's port, kept so Cursor's settings stay valid across restarts.
    pub port: u16,
    /// The gateway id of the model serving Tab; none leaves Tab to Cursor.
    pub tab_model: Option<String>,
    /// The gateway id of the model writing commit messages; none leaves them to Cursor.
    pub commit_model: Option<String>,
    pub commit_locale: CommitLocale,
    pub(crate) ide_lease: Option<ide::Lease>,
    pub(crate) account_lease: Option<account::Lease>,
    pub(crate) cli_lease: Option<cli::Lease>,
}

impl CursorSettings {
    pub fn active(&self) -> bool {
        self.ide || self.cli
    }
}

fn settings_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(SETTINGS_FILE)
}

pub fn load(app_data_dir: &Path) -> CursorSettings {
    std::fs::read(settings_path(app_data_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(app_data_dir: &Path, settings: &CursorSettings) -> Result<(), String> {
    let path = settings_path(app_data_dir);
    let bytes = serde_json::to_vec_pretty(settings).map_err(|_| "cursor.errors.storage")?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).map_err(|_| "cursor.errors.storage")?;
    std::fs::rename(&temporary, &path).map_err(|_| "cursor.errors.storage".into())
}

/// Whether Cursor needs the engine and the router right now.
pub fn active(app_data_dir: &Path) -> bool {
    load(app_data_dir).active()
}

// ---- the engine ----------------------------------------------------------------

static ENGINE: tokio::sync::Mutex<Option<Engine>> = tokio::sync::Mutex::const_new(None);
/// The engine's port while it runs, readable without waiting on the engine.
static PORT: std::sync::Mutex<Option<u16>> = std::sync::Mutex::new(None);

fn running_port() -> Option<u16> {
    *PORT.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn engine_dir(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join("cursor")
}

/// The certificate authority if one was made already; on Windows looking
/// never makes one. Elsewhere the user installs it by hand, so it is made to
/// have a file and a command to show.
fn existing_authority(app_data_dir: &Path) -> Option<Arc<ca::Authority>> {
    let dir = engine_dir(app_data_dir);
    (!cfg!(windows) || ca::Authority::exists(&dir)).then(|| authority(app_data_dir).ok()).flatten()
}

/// The certificate authority, created on first use.
pub fn authority(app_data_dir: &Path) -> Result<Arc<ca::Authority>, String> {
    static AUTHORITY: std::sync::Mutex<Option<Arc<ca::Authority>>> = std::sync::Mutex::new(None);
    let mut held = AUTHORITY.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(authority) = held.as_ref() {
        return Ok(authority.clone());
    }
    let authority = Arc::new(ca::Authority::load_or_create(&engine_dir(app_data_dir)).map_err(|error| {
        log::warn!("Cursor certificate authority unavailable: {error}");
        "cursor.errors.certificate".to_owned()
    })?);
    *held = Some(authority.clone());
    Ok(authority)
}

/// Brings the engine, its host and the leases in line with the settings.
/// `apply_leases` rewrites Cursor's settings and the variable; the periodic
/// reconcile leaves them, so a user's own edit is not fought every 30 s.
pub async fn reconcile(paths: &CodexPaths, apply_leases: bool) -> Result<(), String> {
    let mut settings = load(&paths.app_data_dir);
    let before = settings.clone();
    let mut engine = ENGINE.lock().await;
    if !settings.active() {
        if let Some(running) = engine.take() {
            running.stop().await;
            *PORT.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        }
        return release_leases(&paths.app_data_dir, &mut settings);
    }
    if engine.is_none() {
        let host = Host::new();
        let config = EngineConfig { data_dir: engine_dir(&paths.app_data_dir), port: settings.port };
        let started = Engine::start(config, host).await.map_err(|error| {
            log::warn!("Cursor engine failed to start: {error}");
            "cursor.errors.engine".to_owned()
        })?;
        *PORT.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(started.port());
        *engine = Some(started);
    }
    let running = engine.as_ref().expect("started above");
    configure_host(paths, &settings, running.host());
    let url = running.url();
    let port_moved = settings.port != running.port();
    settings.port = running.port();
    let applied = if apply_leases || port_moved {
        apply_leases_now(&mut settings, &url)
    } else {
        Ok(())
    };
    // Earlier writes still need their recovery leases if a later step fails.
    if settings != before {
        save(&paths.app_data_dir, &settings)?;
    }
    applied
}

/// Points the host at AgentHarbor's models, settings, search and network.
fn configure_host(paths: &CodexPaths, settings: &CursorSettings, host: &Host) {
    let app_data_dir = paths.app_data_dir.clone();
    let sources = settings.sources;
    let codex_paths = paths.clone();
    host.set_model_source(Arc::new(move || {
        let Ok(store) = router::load_store(&codex_paths) else { return Vec::new() };
        let catalog = models_dev::load(&app_data_dir);
        models::engine_models(&store, &catalog, sources)
    }));
    let tab = settings.tab_model.as_deref().map_or(TabMode::Official, |id| TabMode::Model { model_id: models::cursor_id(id) });
    host.set_settings(EngineSettings {
        tab,
        commit: CommitSettings {
            model_id: settings.commit_model.as_deref().map(models::cursor_id),
            locale: settings.commit_locale,
        },
    });
    host.set_egress(egress(paths, settings));
    host.set_searcher(searcher(paths));
}

/// How the engine reaches Cursor and the web: through the proxy Cursor used
/// before, or else AgentHarbor's own network setting.
fn egress(paths: &CodexPaths, settings: &CursorSettings) -> Egress {
    if let Some(proxy) = settings.ide_lease.as_ref().and_then(ide::Lease::original_proxy) {
        return Egress::Proxy(proxy);
    }
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    match proxy.mode {
        ApiProxyMode::Direct => Egress::Direct,
        ApiProxyMode::Manual => proxy.url.filter(|url| !url.trim().is_empty()).map_or(Egress::System, Egress::Proxy),
        ApiProxyMode::System => crate::platform::proxy::detect_system_proxy_candidates()
            .into_iter()
            .next()
            .map_or(Egress::Direct, |url| if url.contains("://") { Egress::Proxy(url) } else { Egress::Proxy(format!("http://{url}")) }),
    }
}

/// WebSearch through the model chosen for it in routing, if one is.
fn searcher(paths: &CodexPaths) -> Option<Arc<dyn cursor_engine::Searcher>> {
    let store = router::load_store(paths).ok()?;
    let provider_id = store.router.web_search_provider_id.as_deref()?;
    let provider = store.providers.iter().find(|provider| provider.id == provider_id)?;
    let slug = provider.gateway_slug.as_deref()?;
    let model = store.router.web_search_model_id.clone().filter(|model| !model.is_empty()).unwrap_or_else(|| provider.model.clone());
    Some(Arc::new(search::RouterSearch {
        url: format!("http://127.0.0.1:{}/clients/cursor/mix/v1/messages", store.router.port),
        token: store.router.bearer_token.clone(),
        model: format!("{slug}/{model}"),
    }))
}

fn apply_ide_leases(settings: &mut CursorSettings, url: &str, user_dir: Option<&Path>) -> Result<(), String> {
    if settings.ide {
        let directory = user_dir.ok_or("cursor.errors.ideMissing")?;
        settings.ide_lease = Some(ide::apply(&directory.join("settings.json"), url, settings.ide_lease.as_ref())?);
        let storage = directory.join("globalStorage");
        std::fs::create_dir_all(&storage).map_err(|_| "cursor.errors.accountWrite")?;
        settings.account_lease = account::apply(&storage.join("state.vscdb"), settings.account_lease.as_ref())?;
    } else {
        // Keep the proxy while an account restoration still needs a retry.
        if let Some(lease) = settings.account_lease.as_ref() {
            account::release(lease)?;
            settings.account_lease = None;
        }
        if let Some(lease) = settings.ide_lease.as_ref() {
            ide::release(lease)?;
            settings.ide_lease = None;
        }
    }
    Ok(())
}

fn apply_leases_now(settings: &mut CursorSettings, url: &str) -> Result<(), String> {
    apply_ide_leases(settings, url, ide::user_dir().as_deref())?;
    if settings.cli && cli::supported() {
        match cli::apply(url, settings.cli_lease.as_ref()) {
            Ok(lease) => settings.cli_lease = Some(lease),
            Err(error) => log::warn!("{} was not set: {error}", cli::VARIABLE),
        }
    } else if let Some(lease) = settings.cli_lease.take() {
        if let Err(error) = cli::release(&lease) {
            log::warn!("{} was not restored: {error}", cli::VARIABLE);
            settings.cli_lease = Some(lease);
        }
    }
    Ok(())
}

fn release_leases(app_data_dir: &Path, settings: &mut CursorSettings) -> Result<(), String> {
    let mut changed = false;
    let mut failure = None;
    if let Some(lease) = settings.account_lease.take() {
        match account::release(&lease) {
            Ok(()) => changed = true,
            Err(error) => {
                log::warn!("Cursor local account was not restored: {error}");
                failure = Some(error);
                settings.account_lease = Some(lease);
            }
        }
    }
    if let Some(lease) = settings.ide_lease.take() {
        match ide::release(&lease) {
            Ok(()) => changed = true,
            Err(error) => {
                log::warn!("Cursor settings were not restored: {error}");
                failure.get_or_insert(error);
                settings.ide_lease = Some(lease);
            }
        }
    }
    if let Some(lease) = settings.cli_lease.take() {
        match cli::release(&lease) {
            Ok(()) => changed = true,
            Err(error) => {
                log::warn!("{} was not restored: {error}", cli::VARIABLE);
                failure.get_or_insert(error);
                settings.cli_lease = Some(lease);
            }
        }
    }
    if changed {
        save(app_data_dir, settings)?;
    }
    failure.map_or(Ok(()), Err)
}

/// On exit: Cursor and `cursor-agent` stop pointing at this process. The
/// switches stay on, so the next start points them back.
pub fn release_on_exit(app_data_dir: &Path) {
    let mut settings = load(app_data_dir);
    if let Err(error) = release_leases(app_data_dir, &mut settings) {
        log::warn!("Cursor injection could not be fully released: {error}");
    }
}

// ---- what the window shows and changes -------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorModelView {
    /// The gateway id, as the settings name the model.
    pub id: String,
    /// The id Cursor shows in its model list.
    pub cursor_id: String,
    pub label: String,
    pub source_name: String,
    pub kind: ModelKind,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorStatus {
    pub ide_installed: bool,
    pub cli_installed: bool,
    pub cli_supported: bool,
    pub settings_path: Option<String>,
    pub ide: bool,
    pub cli: bool,
    pub running: bool,
    pub port: Option<u16>,
    pub url: Option<String>,
    /// Cursor's settings point at the engine right now.
    pub ide_applied: bool,
    /// The user variable points at the engine right now.
    pub cli_applied: bool,
    pub cli_command: Option<String>,
    pub certificate: TrustState,
    pub certificate_path: Option<String>,
    pub trust_command: Option<String>,
    pub sources: ModelSources,
    pub models: Vec<CursorModelView>,
    pub tab_model: Option<String>,
    pub commit_model: Option<String>,
    pub commit_locale: CommitLocale,
    pub web_search_ready: bool,
    pub router_running: bool,
}

pub fn status(paths: &CodexPaths) -> Result<CursorStatus, String> {
    let settings = load(&paths.app_data_dir);
    let store = router::load_store(paths)?;
    let catalog = models_dev::load(&paths.app_data_dir);
    let models = models::offered(&store, &catalog, settings.sources)
        .into_iter()
        .map(|model| CursorModelView {
            cursor_id: models::cursor_id(&model.id),
            id: model.id,
            label: model.label,
            source_name: model.source_name,
            kind: model.kind,
        })
        .collect();
    let port = running_port();
    let url = port.map(|port| format!("http://127.0.0.1:{port}"));
    let authority = existing_authority(&paths.app_data_dir);
    let settings_path = ide::settings_path();
    Ok(CursorStatus {
        ide_installed: ide::installed(),
        cli_installed: cli::installed(),
        cli_supported: cli::supported(),
        settings_path: settings_path.as_ref().map(|path| path.display().to_string()),
        ide: settings.ide,
        cli: settings.cli,
        running: port.is_some(),
        port,
        ide_applied: match (&url, &settings_path) {
            (Some(url), Some(path)) => ide::applied(path, url),
            _ => false,
        },
        cli_applied: url.as_deref().is_some_and(|url| cli::current().as_deref() == Some(url)),
        cli_command: url.as_deref().map(cli::command),
        url,
        certificate: match &authority {
            Some(authority) => ca::trust_state(authority.certificate_der()),
            None if ca::trust_command(Path::new("")).is_some() => TrustState::Manual,
            None => TrustState::Untrusted,
        },
        certificate_path: authority.as_ref().map(|authority| authority.certificate_path().display().to_string()),
        trust_command: authority.as_ref().and_then(|authority| ca::trust_command(authority.certificate_path())),
        sources: settings.sources,
        models,
        tab_model: settings.tab_model,
        commit_model: settings.commit_model,
        commit_locale: settings.commit_locale,
        web_search_ready: searcher(paths).is_some(),
        router_running: crate::core::router_proxy::is_running(),
    })
}

/// Changes the settings, then brings everything in line with them.
pub async fn update(paths: &CodexPaths, change: impl FnOnce(&mut CursorSettings) -> Result<(), String>) -> Result<(), String> {
    let mut settings = load(&paths.app_data_dir);
    change(&mut settings)?;
    save(&paths.app_data_dir, &settings)?;
    super::routing_runtime::reconcile_with(paths, true).await
}

/// Turns the IDE's injection on or off. On needs the certificate trusted
/// first where the app can tell.
pub async fn set_ide(paths: &CodexPaths, on: bool) -> Result<(), String> {
    if on {
        if !ide::installed() {
            return Err("cursor.errors.ideMissing".into());
        }
        let authority = authority(&paths.app_data_dir)?;
        if ca::trust_state(authority.certificate_der()) == TrustState::Untrusted {
            return Err("cursor.errors.untrusted".into());
        }
    }
    update(paths, |settings| {
        settings.ide = on;
        Ok(())
    })
    .await
}

pub async fn set_cli(paths: &CodexPaths, on: bool) -> Result<(), String> {
    update(paths, |settings| {
        settings.cli = on;
        Ok(())
    })
    .await
}

pub async fn set_sources(paths: &CodexPaths, sources: ModelSources) -> Result<(), String> {
    update(paths, |settings| {
        settings.sources = sources;
        Ok(())
    })
    .await
}

pub async fn set_tab(paths: &CodexPaths, model: Option<String>) -> Result<(), String> {
    update(paths, |settings| {
        settings.tab_model = model.filter(|model| !model.trim().is_empty());
        Ok(())
    })
    .await
}

pub async fn set_commit(paths: &CodexPaths, model: Option<String>, locale: CommitLocale) -> Result<(), String> {
    update(paths, |settings| {
        settings.commit_model = model.filter(|model| !model.trim().is_empty());
        settings.commit_locale = locale;
        Ok(())
    })
    .await
}

/// Adds the certificate to the user's trusted roots; Windows asks first.
pub fn trust(paths: &CodexPaths) -> Result<(), String> {
    let authority = authority(&paths.app_data_dir)?;
    ca::trust(authority.certificate_der()).map_err(|error| {
        log::warn!("Cursor certificate was not trusted: {error}");
        "cursor.errors.trust".to_owned()
    })
}

/// Removes the certificate from the user's trusted roots. Cursor IDE stops
/// being injected first, since it could no longer connect.
pub async fn distrust(paths: &CodexPaths) -> Result<(), String> {
    if load(&paths.app_data_dir).ide {
        set_ide(paths, false).await?;
    }
    let authority = authority(&paths.app_data_dir)?;
    ca::distrust(authority.certificate_der()).map_err(|error| {
        log::warn!("Cursor certificate was not removed: {error}");
        "cursor.errors.distrust".to_owned()
    })
}

pub fn open_terminal() -> Result<(), String> {
    let port = running_port().ok_or("cursor.errors.notRunning")?;
    cli::open_terminal(&format!("http://127.0.0.1:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_default_to_everything_off() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = load(directory.path());
        assert!(!settings.active());
        assert_eq!(settings.sources, ModelSources::default());
        settings.ide = true;
        settings.tab_model = Some("relay-a/glm".into());
        save(directory.path(), &settings).unwrap();
        assert_eq!(load(directory.path()), settings);
        assert!(active(directory.path()));
    }

    #[test]
    fn exit_restores_the_local_identity_and_proxy_and_keeps_the_switch_on() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("globalStorage/state.vscdb");
        let cursor_settings = directory.path().join("settings.json");
        std::fs::write(&cursor_settings, r#"{"http.proxy":"http://previous:8080"}"#).unwrap();
        let mut settings = CursorSettings {
            ide: true,
            ..Default::default()
        };
        apply_ide_leases(&mut settings, "http://127.0.0.1:4567", Some(directory.path())).unwrap();
        assert!(settings.account_lease.is_some());
        assert!(ide::applied(&cursor_settings, "http://127.0.0.1:4567"));
        save(directory.path(), &settings).unwrap();
        assert_eq!(load(directory.path()), settings);
        release_on_exit(directory.path());
        let restored = load(directory.path());
        assert!(restored.ide);
        assert!(restored.account_lease.is_none());
        assert!(restored.ide_lease.is_none());
        let connection = rusqlite::Connection::open(database).unwrap();
        let count: i32 = connection.query_row("SELECT COUNT(*) FROM ItemTable WHERE key LIKE 'cursorAuth/%'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        let text = std::fs::read_to_string(cursor_settings).unwrap();
        assert!(text.contains("http://previous:8080"));
        assert!(!text.contains("disableHttp2"));
    }

    #[test]
    fn failed_proxy_settings_never_inject_a_local_account() {
        for malformed in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("settings.json");
            let original = if malformed { "{ broken json" } else { "{}" };
            std::fs::write(&path, original).unwrap();
            if !malformed {
                // Occupy the temporary write path without depending on OS permissions.
                std::fs::create_dir(path.with_extension("json.agentharbor-tmp")).unwrap();
            }
            let mut settings = CursorSettings { ide: true, ..Default::default() };
            let error = apply_ide_leases(&mut settings, "http://127.0.0.1:4567", Some(directory.path())).unwrap_err();
            assert_eq!(error, if malformed { "cursor.errors.settingsUnreadable" } else { "cursor.errors.settingsWrite" });
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert!(!directory.path().join("globalStorage").exists());
            assert!(settings.account_lease.is_none());
            assert!(settings.ide_lease.is_none());
        }
    }

    #[test]
    fn an_account_write_failure_keeps_the_proxy_lease_recoverable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"http.proxy":"http://previous:8080"}"#).unwrap();
        std::fs::write(directory.path().join("globalStorage"), "occupied").unwrap();
        let mut settings = CursorSettings { ide: true, ..Default::default() };
        assert_eq!(apply_ide_leases(&mut settings, "http://127.0.0.1:4567", Some(directory.path())).unwrap_err(), "cursor.errors.accountWrite");
        assert!(settings.account_lease.is_none());
        assert!(settings.ide_lease.is_some());
        save(directory.path(), &settings).unwrap();
        release_on_exit(directory.path());
        assert!(load(directory.path()).ide_lease.is_none());
        assert!(std::fs::read_to_string(path).unwrap().contains("http://previous:8080"));
    }
}
