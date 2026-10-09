//! Coordinate session migration before releasing the selected Codex provider.
use super::*;
use crate::{core::thread_recovery, platform::process};

#[derive(Clone, Copy)]
pub enum CodexConnectionTarget {
    Disabled,
    Native,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexConnectionChange {
    #[serde(flatten)]
    pub change: StateChange,
    pub thread_migration: Option<thread_recovery::ThreadBatchResult>,
}

pub(super) trait CodexProcess {
    fn running(&self) -> bool;
    fn stop(&mut self) -> Result<(), String>;
    fn ensure_stopped(&self) -> Result<(), String>;
    fn reopen(&mut self, paths: &CodexPaths) -> Result<(), String>;
    fn check_writers(&self) -> bool {
        true
    }
}
struct SystemCodex;
impl CodexProcess for SystemCodex {
    fn running(&self) -> bool {
        process::is_codex_app_running()
    }
    fn stop(&mut self) -> Result<(), String> {
        process::stop_codex_for_thread_adoption().map_err(|e| e.to_string())
    }
    fn ensure_stopped(&self) -> Result<(), String> {
        process::ensure_no_codex_writer_processes().map_err(|e| e.to_string())
    }
    fn reopen(&mut self, paths: &CodexPaths) -> Result<(), String> {
        process::launch_codex_app(&newapi::proxy_from_settings(&paths.settings_path))
            .map_err(|e| e.to_string())
    }
}

pub fn change_codex_connection(
    paths: &CodexPaths,
    target: CodexConnectionTarget,
    force: bool,
) -> Result<CodexConnectionChange, String> {
    change_with_process(paths, target, force, &mut SystemCodex)
}

fn blocked(error: impl std::fmt::Display) -> String {
    format!("CODEX_ROUTING_RESTORE_FAILED: {error}")
}

pub(super) fn change_with_process(
    paths: &CodexPaths,
    target: CodexConnectionTarget,
    force: bool,
    process: &mut impl CodexProcess,
) -> Result<CodexConnectionChange, String> {
    // Match the adoption/recovery lock order: thread -> store -> config.
    let _threads = thread_recovery::thread_mutation_guard().map_err(blocked)?;
    let _store = store_write_guard()?;
    let store = load_store_unlocked(paths)?;
    let commit = || match target {
        CodexConnectionTarget::Disabled => set_smart_routing_unlocked(paths, false),
        CodexConnectionTarget::Native => set_access_mode_unlocked(paths, CodexAccessMode::Native),
    };
    if force
        || matches!(target, CodexConnectionTarget::Native)
            && store.codex_access_mode == CodexAccessMode::Native
    {
        return commit().map(|change| CodexConnectionChange {
            change,
            thread_migration: None,
        });
    }
    let ids = thread_recovery::connection_threads(paths).map_err(blocked)?;
    // Even an unindexed pending insertion depends on the current config hash.
    if store.router.enabled || config_snapshot::path(paths).is_file() || !ids.is_empty() {
        thread_recovery::ensure_transition_ready(paths).map_err(blocked)?;
    }
    // A running desktop may flush its first session only during shutdown.
    // Empty SQLite is a fast path only when we own no live connection.
    if ids.is_empty() && !store.router.enabled && !config_snapshot::path(paths).is_file() {
        return commit().map(|change| CodexConnectionChange {
            change,
            thread_migration: None,
        });
    }
    let was_running = process.running();
    let outcome = (|| {
        process.stop().map_err(blocked)?;
        process.ensure_stopped().map_err(blocked)?;
        let final_config = match target {
            CodexConnectionTarget::Disabled => projected_removal(paths, &store)?,
            CodexConnectionTarget::Native => {
                let mut native = store.clone();
                if native.router.aggregate_model_id.is_some() {
                    return Err("aggregation.errors.routerOnly".into());
                }
                native.active_provider_id =
                    effective_active_for(&native, ModelProtocol::Responses).map(|p| p.id.clone());
                native.codex_access_mode = CodexAccessMode::Native;
                validate_native_selection(&native)?;
                native_config::prepare_provider(paths, &native).map_err(blocked)?;
                let (_, mut document) = open_config(paths)?;
                document["model_provider"] = toml_edit::value(NATIVE_PROVIDER_KEY);
                // A selected profile can override root provider settings.
                if let Some(profile) = document
                    .get("profile")
                    .and_then(toml_edit::Item::as_str)
                    .map(str::to_owned)
                {
                    if document
                        .get("profiles")
                        .and_then(|p| p.get(&profile))
                        .and_then(|p| p.get("model_provider"))
                        .is_some()
                    {
                        return Err(blocked("The selected Codex profile overrides the provider"));
                    }
                }
                document.to_string()
            }
        };
        // Re-enumerate after graceful shutdown flushes the desktop's catalog.
        let ids = thread_recovery::connection_threads(paths).map_err(blocked)?;
        let migration = thread_recovery::restore_connection_threads(
            paths,
            &ids,
            &final_config,
            process.check_writers(),
        )
        .map_err(blocked)?;
        process.ensure_stopped().map_err(blocked)?;
        let change = commit()?;
        Ok::<_, String>(CodexConnectionChange {
            change,
            thread_migration: Some(migration),
        })
    })();
    let reopen_error = if was_running
        && !process.running()
        && thread_recovery::ensure_transition_ready(paths).is_ok()
    {
        process.reopen(paths).err()
    } else {
        None
    };
    match outcome {
        Ok(mut change) => {
            if let Some(migration) = &mut change.thread_migration {
                migration.reopen_error = reopen_error;
            }
            Ok(change)
        }
        Err(error) => Err(match reopen_error {
            Some(reopen) => format!("{error}; CODEX_REOPEN_FAILED: {reopen}"),
            None => error,
        }),
    }
}

fn projected_removal(paths: &CodexPaths, store: &Store) -> Result<String, String> {
    let (original, mut document) = open_config(paths)?;
    let snapshot = match config_snapshot::load(paths, original.as_deref().unwrap_or(""))
        .map_err(config_update_error)?
    {
        Some(snapshot) => Some(snapshot),
        None if model_provider_is_ours(&document) => Some(
            config_snapshot::recover_legacy(paths, &document, store)
                .map_err(config_update_error)?,
        ),
        None => None,
    };
    if let Some(snapshot) = snapshot {
        remove_managed_provider_entries(&mut document);
        snapshot
            .restore(paths, &mut document)
            .map_err(config_update_error)?;
    }
    Ok(document.to_string())
}

#[cfg(test)]
#[path = "router_transition_tests.rs"]
mod tests;
