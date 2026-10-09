//! Batch provider migration for many threads with one Codex shutdown/restart.
//! Every thread still goes through the single-thread guarded transaction, with
//! its own backup and journal; one failure never rolls back or blocks others.
use super::*;
use crate::core::{newapi, router, router_proxy};

/// Where an existing thread's requests go with the current configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadRouteState {
    /// Uses the unified Router; official and relay models can be mixed.
    Routed,
    /// A defined provider that bypasses the Router; only its own models work.
    Direct,
    /// The provider is no longer defined in config.toml; resuming fails.
    Broken,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRouteOverview {
    /// Whether config.toml is AgentHarbor's current Router setup.
    pub router_active: bool,
    /// Catalog rows still bound to the Router provider, even after a mode
    /// change has made that provider unusable.
    pub router_provider_threads: usize,
    /// Thread ID to route state, for threads in the Codex SQLite catalog.
    pub states: BTreeMap<String, ThreadRouteState>,
}

/// What a batch migration would do. Only `ready` threads are applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadBatchPreview {
    pub target_provider: String,
    pub ready: Vec<ThreadBatchCandidate>,
    pub skipped: Vec<ThreadBatchSkip>,
    /// Ready threads grouped by their current provider.
    pub providers: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadBatchCandidate {
    pub thread_id: String,
    pub revision: String,
    pub from_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadBatchSkip {
    pub thread_id: String,
    pub status: ThreadRecoveryStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadBatchResult {
    pub changed: usize,
    pub unchanged: usize,
    pub failed: Vec<ThreadBatchSkip>,
    /// Codex was running but could not be reopened; the migration itself stands.
    pub reopen_error: Option<String>,
}

/// Which provider a batch moves threads to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadBatchTarget {
    /// The active config.toml provider, which must be the unified Router.
    Router,
    /// The active non-Router provider (normally built-in `openai`), used to keep
    /// routed threads usable after the Router configuration is removed.
    Restore,
}

pub fn route_overview(paths: &CodexPaths) -> Result<ThreadRouteOverview, CoreError> {
    let router_active = owned_router_port(paths).is_ok();
    let defined = defined_providers(&paths.config_path)?;
    let mut states = BTreeMap::new();
    let providers = database_providers(&paths.codex_state_db_path)?;
    let router_provider_threads = providers
        .iter()
        .filter(|(_, provider)| router::managed_provider_key(provider))
        .count();
    for (thread_id, provider) in providers {
        let state = if router::is_gateway_provider_key(&provider) && router_active {
            ThreadRouteState::Routed
        } else if router::is_gateway_provider_key(&provider) {
            ThreadRouteState::Broken
        } else if provider.eq_ignore_ascii_case("openai") || defined.contains(&provider) {
            ThreadRouteState::Direct
        } else {
            ThreadRouteState::Broken
        };
        states.insert(thread_id, state);
    }
    Ok(ThreadRouteOverview {
        router_active,
        router_provider_threads,
        states,
    })
}

/// Threads whose catalog provider equals `provider`.
pub fn threads_with_provider(paths: &CodexPaths, provider: &str) -> Result<Vec<String>, CoreError> {
    Ok(database_providers(&paths.codex_state_db_path)?
        .into_iter()
        .filter(|(_, stored)| stored == provider)
        .map(|(thread_id, _)| thread_id)
        .collect())
}

pub fn preview_batch(
    paths: &CodexPaths,
    thread_ids: &[String],
    target: ThreadBatchTarget,
) -> Result<ThreadBatchPreview, CoreError> {
    let target_provider = resolve_provider_target(&paths.config_path)?.configured;
    validate_batch_target(paths, &target_provider, target)?;
    let mut preview = ThreadBatchPreview {
        target_provider,
        ready: Vec::new(),
        skipped: Vec::new(),
        providers: BTreeMap::new(),
    };
    let mut seen = HashSet::new();
    for thread_id in thread_ids {
        if !seen.insert(thread_id.as_str()) {
            continue;
        }
        match build_recovery_plan(paths, thread_id) {
            Ok(plan) if plan.preview.can_apply && plan.pending_journal.is_none() => {
                let from_provider = plan.preview.database_provider.clone();
                *preview
                    .providers
                    .entry(from_provider.clone().unwrap_or_default())
                    .or_default() += 1;
                preview.ready.push(ThreadBatchCandidate {
                    thread_id: thread_id.clone(),
                    revision: plan.preview.revision,
                    from_provider,
                });
            }
            Ok(plan) => preview.skipped.push(ThreadBatchSkip {
                thread_id: thread_id.clone(),
                status: plan.preview.status,
                reason: plan.preview.reason,
            }),
            Err(error) => preview.skipped.push(ThreadBatchSkip {
                thread_id: thread_id.clone(),
                status: ThreadRecoveryStatus::InvalidRollout,
                reason: error.to_string(),
            }),
        }
    }
    Ok(preview)
}

/// Stop Codex once, apply each confirmed thread, then reopen Codex if it was
/// running. A thread whose revision changed after preview is reported as
/// failed instead of being refreshed silently.
pub fn apply_batch(
    paths: &CodexPaths,
    candidates: &[ThreadBatchCandidate],
    target: ThreadBatchTarget,
) -> Result<ThreadBatchResult, CoreError> {
    let _guard = thread_mutation_guard()?;
    let target_provider = resolve_provider_target(&paths.config_path)?.configured;
    validate_batch_target(paths, &target_provider, target)?;
    let was_running = process::is_codex_app_running();
    process::stop_codex_for_thread_adoption()?;
    let outcome = apply_batch_stopped(paths, candidates, target, true);
    // A failed preflight after shutdown (for example, a CLI writer still
    // running) must not leave a previously open desktop closed.
    let reopen_error = if !was_running {
        None
    } else if safe_to_reopen(paths) {
        process::launch_codex_app(&newapi::proxy_from_settings(&paths.settings_path))
            .err()
            .map(|error| error.to_string())
    } else {
        Some("Codex stayed closed because an unfinished thread-recovery journal needs repair".into())
    };
    match outcome {
        Ok(result) => Ok(ThreadBatchResult {
            reopen_error,
            ..result
        }),
        Err(error) => match reopen_error {
            Some(reopen) => Err(CoreError::OperationFailed(format!(
                "{error}; CODEX_REOPEN_FAILED: {reopen}"
            ))),
            None => Err(error),
        },
    }
}

fn apply_batch_stopped(
    paths: &CodexPaths,
    candidates: &[ThreadBatchCandidate],
    target: ThreadBatchTarget,
    check_writer_processes: bool,
) -> Result<ThreadBatchResult, CoreError> {
    if check_writer_processes {
        process::ensure_no_codex_writer_processes()?;
    }
    let mut result = ThreadBatchResult {
        changed: 0,
        unchanged: 0,
        failed: Vec::new(),
        reopen_error: None,
    };
    for (index, candidate) in candidates.iter().enumerate() {
        // Shutdown may flush history, and an earlier failure may leave the
        // Router unavailable; re-check before every thread.
        let applied = resolve_provider_target(&paths.config_path)
            .and_then(|provider| validate_batch_target(paths, &provider.configured, target))
            .and_then(|()| {
            apply_thread_recovery_locked(
                paths,
                &candidate.thread_id,
                &candidate.revision,
                check_writer_processes,
            )
            });
        match applied {
            Ok(applied) if applied.changed => result.changed += 1,
            Ok(_) => result.unchanged += 1,
            Err(error) => {
                let status = build_recovery_plan(paths, &candidate.thread_id)
                    .map_or(ThreadRecoveryStatus::InvalidRollout, |plan| plan.preview.status);
                result.failed.push(ThreadBatchSkip {
                    thread_id: candidate.thread_id.clone(),
                    status,
                    reason: error.to_string(),
                });
                // An unfinished journal blocks every later thread; stop here
                // and leave it for the single-thread recovery flow.
                if inspect_pending_journal_blocks(paths) {
                    result.failed.extend(candidates[index + 1..].iter().map(|remaining| {
                        ThreadBatchSkip {
                            thread_id: remaining.thread_id.clone(),
                            status: ThreadRecoveryStatus::JournalBlocked,
                            reason: "An unfinished recovery journal blocked the remaining batch"
                                .into(),
                        }
                    }));
                    break;
                }
            }
        }
    }
    Ok(result)
}

fn inspect_pending_journal_blocks(paths: &CodexPaths) -> bool {
    !matches!(inspect_pending_journal(paths), PendingJournalInspection::None)
}

fn safe_to_reopen(paths: &CodexPaths) -> bool {
    !inspect_pending_journal_blocks(paths)
}

fn validate_batch_target(
    paths: &CodexPaths,
    configured: &str,
    target: ThreadBatchTarget,
) -> Result<(), CoreError> {
    match target {
        ThreadBatchTarget::Router => {
            if configured != router::CATALOG_PROVIDER_KEY {
                return Err(router_not_ready());
            }
            let port = owned_router_port(paths)?;
            if router_proxy::running_port() != Some(port) {
                return Err(CoreError::OperationFailed(
                    "THREAD_ROUTER_NOT_READY: the local Router is not listening; enable it before adopting threads".into(),
                ));
            }
            Ok(())
        }
        ThreadBatchTarget::Restore => {
            let active = resolve_provider_target(&paths.config_path)?;
            if configured == router::CATALOG_PROVIDER_KEY || !active.valid {
                return Err(CoreError::OperationFailed(format!(
                    "THREAD_RESTORE_TARGET_INVALID: Codex currently uses {configured}; disable the unified Router before restoring threads"
                )));
            }
            Ok(())
        }
    }
}

pub(super) fn defined_providers(config_path: &Path) -> Result<HashSet<String>, CoreError> {
    let text = match std::fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(error.into()),
    };
    let document: toml::Value = text.parse()?;
    Ok(document
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .map(|providers| {
            providers
                .iter()
                .filter(|(_, entry)| entry.is_table())
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default())
}

pub(super) fn database_providers(database_path: &Path) -> Result<Vec<(String, String)>, CoreError> {
    match std::fs::metadata(database_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(CoreError::InvalidData(
                "Codex thread catalog is not a regular file".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    }
    let connection = open_database(database_path, true)?;
    let columns = crate::core::threads::table_columns(&connection, "threads")?;
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    if !columns.contains("id") || !columns.contains("model_provider") {
        return Err(CoreError::InvalidData(
            "Codex thread catalog has an unsupported provider schema".into(),
        ));
    }
    let mut statement = connection
        .prepare("SELECT id, COALESCE(model_provider, '') FROM threads")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{fixture, THREAD_ID};

    const SECOND: &str = "02999999-1111-7222-8333-444444444444";

    fn add_thread(paths: &CodexPaths, thread_id: &str, provider: &str) {
        let rollout = paths.sessions_dir.join(format!("rollout-{thread_id}.jsonl"));
        let meta = serde_json::json!({"timestamp":"2026-09-25T00:00:00Z","type":"session_meta",
            "payload":{"id":thread_id,"model_provider":provider,"cwd":"C:/fixture"}});
        std::fs::write(&rollout, format!("{meta}\n")).unwrap();
        Connection::open(&paths.codex_state_db_path)
            .unwrap()
            .execute(
                "INSERT INTO threads (id, model_provider, rollout_path) VALUES (?1, ?2, ?3)",
                (thread_id, provider, rollout.display().to_string()),
            )
            .unwrap();
    }

    fn provider_of(paths: &CodexPaths, thread_id: &str) -> String {
        load_database_thread(&paths.codex_state_db_path, thread_id)
            .unwrap()
            .unwrap()
            .provider
    }

    /// Router on → batch adopt → Router off → batch restore, with a real owned
    /// Router configuration. Only the listener check is outside this test.
    #[test]
    fn adopted_threads_survive_turning_routing_off() {
        let (_temp, paths) = fixture("openai", "openai");
        add_thread(&paths, SECOND, "aimai1");
        let saved = router::save_provider(
            &paths,
            serde_json::from_value(serde_json::json!({
                "name": "Fixture", "baseUrl": "http://127.0.0.1:12345", "accessToken": "",
                "apiKey": "synthetic-key", "model": "gpt-6-astra", "networkMode": "direct"
            }))
            .unwrap(),
        )
        .unwrap();
        router::set_enabled(&paths, &saved.state.providers[0].id, true).unwrap();
        router::set_smart_routing(&paths, true).unwrap();

        let overview = route_overview(&paths).unwrap();
        assert!(overview.router_active);
        assert_eq!(overview.router_provider_threads, 0);
        assert_eq!(overview.states[THREAD_ID], ThreadRouteState::Direct);
        assert_eq!(overview.states[SECOND], ThreadRouteState::Broken);
        let ids = [THREAD_ID.to_owned(), SECOND.to_owned()];
        // The Router batch also requires a live listener; preview the same
        // threads against the Router target without it.
        let target = resolve_provider_target(&paths.config_path).unwrap().configured;
        assert_eq!(target, router::CATALOG_PROVIDER_KEY);
        assert!(preview_batch(&paths, &ids, ThreadBatchTarget::Router).is_err());
        let ready: Vec<_> = ids
            .iter()
            .map(|id| {
                let plan = build_recovery_plan(&paths, id).unwrap();
                assert!(plan.preview.can_apply, "{:?}", plan.preview.status);
                ThreadBatchCandidate {
                    thread_id: id.clone(),
                    revision: plan.preview.revision,
                    from_provider: plan.preview.database_provider,
                }
            })
            .collect();
        if !cfg!(target_os = "windows") {
            return;
        }
        for candidate in &ready {
            apply_thread_recovery_locked(&paths, &candidate.thread_id, &candidate.revision, false)
                .unwrap();
        }
        let overview = route_overview(&paths).unwrap();
        assert_eq!(overview.states[THREAD_ID], ThreadRouteState::Routed);
        assert_eq!(overview.states[SECOND], ThreadRouteState::Routed);

        // Turning routing off removes the Router provider from config.toml;
        // without a restore the adopted threads would no longer resume.
        router::set_smart_routing(&paths, false).unwrap();
        let overview = route_overview(&paths).unwrap();
        assert!(!overview.router_active);
        assert_eq!(overview.router_provider_threads, 2);
        assert_eq!(overview.states[THREAD_ID], ThreadRouteState::Broken);
        let routed = threads_with_provider(&paths, router::CATALOG_PROVIDER_KEY).unwrap();
        let preview = preview_batch(&paths, &routed, ThreadBatchTarget::Restore).unwrap();
        assert_eq!(preview.target_provider, "openai");
        assert_eq!(preview.ready.len(), 2);
        let result =
            apply_batch_stopped(&paths, &preview.ready, ThreadBatchTarget::Restore, false).unwrap();
        assert_eq!((result.changed, result.failed.len()), (2, 0));
        for thread_id in [THREAD_ID, SECOND] {
            assert_eq!(provider_of(&paths, thread_id), "openai");
            assert_eq!(
                route_overview(&paths).unwrap().states[thread_id],
                ThreadRouteState::Direct
            );
        }
        // Every migration step left its own backup.
        assert!(
            std::fs::read_dir(backup_directory(&paths)).unwrap().count() >= 4,
            "one backup per thread per move"
        );
    }

    #[test]
    fn route_overview_separates_direct_and_broken_threads() {
        let (_temp, paths) = fixture("custom", "openai");
        add_thread(&paths, SECOND, "aimai1");
        let overview = route_overview(&paths).unwrap();
        assert!(!overview.router_active);
        assert_eq!(overview.states[THREAD_ID], ThreadRouteState::Direct);
        assert_eq!(overview.states[SECOND], ThreadRouteState::Broken);
        // Without an owned Router config, a stale Router thread is broken too.
        let (_temp, paths) = fixture("custom", router::CATALOG_PROVIDER_KEY);
        assert_eq!(
            route_overview(&paths).unwrap().states[THREAD_ID],
            ThreadRouteState::Broken
        );
    }

    #[test]
    fn router_batch_requires_the_owned_router_configuration() {
        let (_temp, paths) = fixture("custom", "openai");
        let error = preview_batch(&paths, &[THREAD_ID.into()], ThreadBatchTarget::Router)
            .unwrap_err()
            .to_string();
        assert!(error.contains("THREAD_ROUTER_NOT_READY"), "{error}");
        let (_temp, paths) = fixture(router::CATALOG_PROVIDER_KEY, "openai");
        let error = preview_batch(&paths, &[THREAD_ID.into()], ThreadBatchTarget::Restore)
            .unwrap_err()
            .to_string();
        assert!(error.contains("THREAD_RESTORE_TARGET_INVALID"), "{error}");
    }

    #[test]
    fn restore_batch_moves_routed_threads_and_reports_each_outcome() {
        let (_temp, paths) = fixture("openai", router::CATALOG_PROVIDER_KEY);
        add_thread(&paths, SECOND, router::CATALOG_PROVIDER_KEY);
        assert_eq!(
            threads_with_provider(&paths, router::CATALOG_PROVIDER_KEY).unwrap(),
            [THREAD_ID, SECOND]
        );
        let ids = [THREAD_ID.to_owned(), SECOND.to_owned(), THREAD_ID.to_owned()];
        let preview = preview_batch(&paths, &ids, ThreadBatchTarget::Restore).unwrap();
        assert_eq!(preview.target_provider, "openai");
        assert_eq!(preview.ready.len(), 2, "duplicates are applied once");
        assert_eq!(preview.providers[router::CATALOG_PROVIDER_KEY], 2);

        // A thread that changed after preview fails alone; the other applies.
        let mut stale = preview.ready.clone();
        stale[1].revision = "changed".into();
        let result =
            apply_batch_stopped(&paths, &stale, ThreadBatchTarget::Restore, false).unwrap();
        if cfg!(target_os = "windows") {
            assert_eq!(result.changed, 1);
            assert_eq!(result.failed.len(), 1);
            assert_eq!(result.failed[0].thread_id, SECOND);
            assert!(result.failed[0].reason.contains("REVISION_CHANGED"));
            for (thread_id, provider) in [(THREAD_ID, "openai"), (SECOND, router::CATALOG_PROVIDER_KEY)] {
                assert_eq!(
                    load_database_thread(&paths.codex_state_db_path, thread_id)
                        .unwrap()
                        .unwrap()
                        .provider,
                    provider
                );
            }
            assert_eq!(
                route_overview(&paths).unwrap().states[THREAD_ID],
                ThreadRouteState::Direct
            );
        } else {
            assert_eq!(result.changed, 0);
        }
    }

    #[test]
    fn batch_reports_each_candidate_when_config_becomes_unreadable() {
        let (_temp, paths) = fixture("openai", router::CATALOG_PROVIDER_KEY);
        add_thread(&paths, SECOND, router::CATALOG_PROVIDER_KEY);
        let ids = [THREAD_ID.to_owned(), SECOND.to_owned()];
        let ready = preview_batch(&paths, &ids, ThreadBatchTarget::Restore)
            .unwrap()
            .ready;
        std::fs::write(&paths.config_path, "not valid = [").unwrap();

        let result = apply_batch_stopped(&paths, &ready, ThreadBatchTarget::Restore, false).unwrap();
        assert_eq!(result.changed, 0);
        assert_eq!(result.failed.len(), 2);
        assert_eq!(result.failed[0].thread_id, THREAD_ID);
        assert_eq!(result.failed[1].thread_id, SECOND);
    }

    #[test]
    fn journal_block_reports_unprocessed_candidates() {
        let (_temp, paths) = fixture("openai", router::CATALOG_PROVIDER_KEY);
        add_thread(&paths, SECOND, router::CATALOG_PROVIDER_KEY);
        let ids = [THREAD_ID.to_owned(), SECOND.to_owned()];
        let ready = preview_batch(&paths, &ids, ThreadBatchTarget::Restore)
            .unwrap()
            .ready;
        std::fs::write(journal_path(&paths), "invalid journal").unwrap();

        let result = apply_batch_stopped(&paths, &ready, ThreadBatchTarget::Restore, false).unwrap();
        assert_eq!(result.changed, 0);
        assert_eq!(result.failed.len(), 2);
        assert_eq!(result.failed[1].thread_id, SECOND);
        assert_eq!(result.failed[1].status, ThreadRecoveryStatus::JournalBlocked);
    }
}
