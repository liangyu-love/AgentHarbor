//! Apply a Codex connection change to config.toml.
//!
//! Account-mode routing only changes the endpoint of Codex's built-in `openai`
//! provider, so threads keep their provider and nothing is migrated. Codex
//! reads config.toml at startup: a running Codex keeps its old connection
//! until the user restarts it, and the router stays available meanwhile.
use super::*;
use crate::{core::thread_recovery, platform::process};
use rusqlite::{Connection, OpenFlags};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexConnectionTarget {
    Enabled,
    Disabled,
    Native,
    Router,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexConnectionChange {
    #[serde(flatten)]
    pub change: StateChange,
    /// Codex is running with the previous connection until it is restarted.
    pub restart_required: bool,
}

static RESTART_PENDING: AtomicBool = AtomicBool::new(false);

/// A running Codex still uses a connection written before its last start.
pub fn codex_restart_pending() -> bool {
    if !RESTART_PENDING.load(Ordering::Relaxed) {
        return false;
    }
    if process::is_codex_app_running() {
        return true;
    }
    RESTART_PENDING.store(false, Ordering::Relaxed);
    false
}

pub(super) fn clear_restart_pending() {
    RESTART_PENDING.store(false, Ordering::Relaxed);
}

pub(super) trait CodexProcess {
    fn running(&self) -> bool;
    fn stop(&mut self) -> Result<(), String>;
    fn reopen(&mut self, paths: &CodexPaths) -> Result<(), String>;
}

struct SystemCodex;
impl CodexProcess for SystemCodex {
    fn running(&self) -> bool {
        process::is_codex_app_running()
    }
    fn stop(&mut self) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        process::snapshot_codex_path_before_stop();
        process::stop_codex_app_gracefully(std::time::Duration::from_secs(8))
            .map_err(|e| e.to_string())
    }
    fn reopen(&mut self, paths: &CodexPaths) -> Result<(), String> {
        process::launch_codex_app(&newapi::proxy_from_settings(&paths.settings_path))
            .map_err(|e| e.to_string())
    }
}

/// Write the connection only; the user decides when to restart Codex.
pub fn change_codex_connection(
    paths: &CodexPaths,
    target: CodexConnectionTarget,
) -> Result<CodexConnectionChange, String> {
    let change = change_with_process(paths, target, &mut SystemCodex, false)?;
    if change.restart_required {
        RESTART_PENDING.store(true, Ordering::Relaxed);
    }
    Ok(change)
}

/// Exit cleanup: the router is going away, so a running Codex is restarted
/// once to reach its own provider directly. Failures never block exiting.
pub fn release_codex_connection_on_exit(paths: &CodexPaths) -> Result<(), String> {
    let change = change_with_process(
        paths,
        CodexConnectionTarget::Disabled,
        &mut SystemCodex,
        true,
    );
    clear_restart_pending();
    change.map(|_| ())
}

pub(super) fn change_with_process(
    paths: &CodexPaths,
    target: CodexConnectionTarget,
    process: &mut impl CodexProcess,
    restart: bool,
) -> Result<CodexConnectionChange, String> {
    let stopped = restart
        && process.running()
        && match process.stop() {
            Ok(()) => true,
            Err(error) => {
                log::warn!("Codex could not stop before the connection change: {error}");
                false
            }
        };
    let change = {
        // Same lock order as single-thread recovery: thread -> store -> config.
        let _threads = thread_recovery::thread_mutation_guard().map_err(|e| e.to_string())?;
        let _store = store_write_guard()?;
        let change = match target {
            CodexConnectionTarget::Enabled => set_smart_routing_unlocked(paths, true),
            CodexConnectionTarget::Disabled => set_smart_routing_unlocked(paths, false),
            CodexConnectionTarget::Native => {
                set_access_mode_unlocked(paths, CodexAccessMode::Native)
            }
            CodexConnectionTarget::Router => {
                set_access_mode_unlocked(paths, CodexAccessMode::Router)
            }
        };
        // A live Codex may still rewrite its catalog rows; normalize them
        // when it next stops (restart, startup or exit).
        if change.is_ok() && !process.running() {
            if let Err(error) = normalize_threads(paths) {
                log::warn!("Codex thread catalog normalization skipped: {error}");
            }
        }
        change
    };
    if stopped {
        if let Err(error) = process.reopen(paths) {
            log::warn!("Codex could not reopen after the connection change: {error}");
        }
    }
    Ok(CodexConnectionChange {
        change: change?,
        restart_required: !stopped && process.running(),
    })
}

/// Codex resumes a thread with the provider and model recorded in its catalog
/// row. Earlier builds moved threads onto router providers, and a relay model
/// only works while routing is on. Reset just those values so every thread
/// stays visible and usable; conversation files are never rewritten.
pub(crate) fn normalize_threads(paths: &CodexPaths) -> Result<usize, String> {
    let store = load_store_unlocked(paths)?;
    let routed = store.router.enabled && store.codex_access_mode == CodexAccessMode::Router;
    // Accountless routing still uses a dedicated provider for its threads.
    let keep_gateway = routed && store.router.accountless;
    if !paths.codex_state_db_path.is_file() {
        return Ok(0);
    }
    let connection = Connection::open_with_flags(
        &paths.codex_state_db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('threads')")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<_, _>>()
        })
        .map_err(|e| e.to_string())?;
    if !columns.iter().any(|column| column == "model_provider") {
        return Ok(0);
    }
    let mut changed = 0;
    if !keep_gateway {
        let providers: Vec<String> = connection
            .prepare("SELECT DISTINCT model_provider FROM threads WHERE model_provider IS NOT NULL")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<_, _>>()
            })
            .map_err(|e| e.to_string())?;
        for provider in providers
            .iter()
            .filter(|provider| is_gateway_provider_key(provider))
        {
            changed += connection
                .execute(
                    "UPDATE threads SET model_provider = 'openai' WHERE model_provider = ?1",
                    [provider],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    if !routed && columns.iter().any(|column| column == "model") {
        changed += connection
            .execute(
                "UPDATE threads SET model = NULL WHERE model_provider = 'openai' AND model LIKE '%::%'",
                [],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "router_transition_tests.rs"]
mod tests;
