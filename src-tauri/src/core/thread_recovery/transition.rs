//! Migrate against the planned configuration while the current route stays live.
use super::*;

pub(crate) fn connection_threads(paths: &CodexPaths) -> Result<Vec<String>, CoreError> {
    Ok(
        batch::database_providers(paths.codex_state_db_path.as_path())?
            .into_iter()
            .filter(|(_, provider)| super::super::router::managed_provider_key(provider))
            .map(|(id, _)| id)
            .collect(),
    )
}

pub(crate) fn ensure_transition_ready(paths: &CodexPaths) -> Result<(), CoreError> {
    if !matches!(
        inspect_pending_journal(paths),
        PendingJournalInspection::None
    ) {
        return Err(CoreError::OperationFailed(
            "An unfinished thread recovery must be completed before migrating sessions".into(),
        ));
    }
    Ok(())
}

/// The caller holds the thread lock and has stopped every Codex writer. All
/// plans bind to the unchanged live config, rather than the projected config.
pub(crate) fn restore_connection_threads(
    paths: &CodexPaths,
    ids: &[String],
    final_config: &str,
    check_writers: bool,
) -> Result<ThreadBatchResult, CoreError> {
    ensure_transition_ready(paths)?;
    let current = resolve_provider_target(&paths.config_path)?;
    let target = resolve_provider_text(final_config, current.generation.clone())?;
    let defined = batch::defined_providers(&paths.config_path)?;
    if !target.valid
        || (!target.configured.eq_ignore_ascii_case("openai")
            && !defined.contains(&target.configured))
        || super::super::router::managed_provider_key(&target.configured)
            && target.configured != super::super::router::NATIVE_PROVIDER_KEY
    {
        return Err(CoreError::OperationFailed(
            "The restored provider is unavailable; the current route was retained".into(),
        ));
    }
    let plans = ids
        .iter()
        .map(|id| {
            let plan =
                build_recovery_plan_for_target(paths, id, RolloutLookup::Refresh, target.clone())?;
            if !plan.preview.can_apply && plan.preview.status != ThreadRecoveryStatus::Compatible {
                return Err(CoreError::OperationFailed(format!(
                    "{id}: {}",
                    plan.preview.reason
                )));
            }
            Ok((id, plan.preview.revision))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let mut result = ThreadBatchResult {
        changed: 0,
        unchanged: 0,
        failed: Vec::new(),
        reopen_error: None,
    };
    for (id, revision) in plans {
        if check_writers {
            process::ensure_no_codex_writer_processes()?;
        }
        // Refresh under the same target to catch a CLI/external history edit.
        let refreshed =
            build_recovery_plan_for_target(paths, id, RolloutLookup::Refresh, target.clone())?;
        if refreshed.preview.revision != revision {
            return Err(CoreError::OperationFailed(format!(
                "{id}: thread changed during migration"
            )));
        }
        match apply_recovery_plan_locked(paths, id, check_writers, refreshed) {
            Ok(applied) if applied.changed => result.changed += 1,
            Ok(_) => result.unchanged += 1,
            Err(error) => return Err(CoreError::OperationFailed(format!(
                "{id}: {error}; {} sessions already migrated; remaining sessions keep their route",
                result.changed
            ))),
        }
    }
    ensure_transition_ready(paths)?;
    ensure_config_generation(paths, &current.generation)?;
    // Re-enumeration prevents deleting a provider still used by an unexpected
    // catalog row. Native is the prepared destination during a mode change.
    let remaining = batch::database_providers(&paths.codex_state_db_path)?;
    if remaining.iter().any(|(_, provider)| {
        super::super::router::managed_provider_key(provider) && provider != &target.configured
    }) {
        return Err(CoreError::OperationFailed(
            "Additional routed sessions appeared during migration".into(),
        ));
    }
    Ok(result)
}
