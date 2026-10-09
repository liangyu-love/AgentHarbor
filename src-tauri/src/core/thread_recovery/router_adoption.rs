//! One-time migration of a direct thread into the unified Router. A separate
//! app-server cannot unload the desktop's in-memory thread. Use the existing
//! guarded recovery transaction after a graceful desktop shutdown instead of
//! starting another real-home app-server (which could run project hooks/MCP).
use super::*;
use crate::core::{newapi, router, router_proxy};

pub(super) fn validate_router_target(
    paths: &CodexPaths,
    preview: &ThreadRecoveryPreview,
) -> Result<u16, CoreError> {
    if preview.target_provider != router::CATALOG_PROVIDER_KEY {
        return Err(router_not_ready());
    }
    owned_router_port(paths)
}

pub(super) fn router_not_ready() -> CoreError {
    CoreError::OperationFailed(
        "THREAD_ROUTER_NOT_READY: enable the unified Router and refresh its configuration before adopting a thread; external provider settings were not changed".into(),
    )
}

/// The Router port when config.toml is AgentHarbor's current Router setup.
pub(super) fn owned_router_port(paths: &CodexPaths) -> Result<u16, CoreError> {
    let store = router::read_store(&paths.app_data_dir.join("newapi-providers.dat"))
        .map_err(CoreError::OperationFailed)?;
    let document: toml::Value = std::fs::read_to_string(&paths.config_path)?.parse()?;
    let profile = document
        .get("profile")
        .and_then(toml::Value::as_str)
        .and_then(|name| document.get("profiles")?.get(name));
    // Profiles may override the catalog as well as the selected provider.
    let profile_catalog_matches =
        match profile.and_then(|profile| profile.get("model_catalog_json")) {
            Some(catalog) => Some(catalog) == document.get("model_catalog_json"),
            None => true,
        };
    if !profile_catalog_matches
        || !router::routing_config_matches(paths, &store).map_err(CoreError::OperationFailed)?
    {
        return Err(router_not_ready());
    }
    Ok(store.router.port)
}

fn validate_preview(preview: &ThreadRecoveryPreview, revision: &str) -> Result<(), CoreError> {
    if revision.is_empty() || preview.revision != revision {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_REVISION_CHANGED: preview state changed; refresh and confirm again"
                .into(),
        ));
    }
    if !preview.can_apply
        || !matches!(
            preview.status,
            ThreadRecoveryStatus::ProviderMismatch | ThreadRecoveryStatus::MissingCatalog
        )
    {
        return Err(CoreError::OperationFailed(
            "THREAD_RECOVERY_NOT_APPLICABLE: resolve recovery conflicts before adopting this thread".into(),
        ));
    }
    Ok(())
}

/// The optional warning means adoption succeeded but the desktop did not open.
/// Never report a committed migration as failed just because launching failed.
pub fn adopt_thread_into_router(
    paths: &CodexPaths,
    thread_id: &str,
    expected_revision: &str,
) -> Result<(ThreadRecoveryResult, Option<String>), CoreError> {
    let _guard = thread_mutation_guard()?;
    let preview =
        build_recovery_plan_with_lookup(paths, thread_id, RolloutLookup::Refresh)?.preview;
    validate_preview(&preview, expected_revision)?;
    let port = validate_router_target(paths, &preview)?;
    if router_proxy::running_port() != Some(port) {
        return Err(CoreError::OperationFailed(
            "THREAD_ROUTER_NOT_READY: the local Router is not listening; enable it before adopting this thread".into(),
        ));
    }
    let was_running = process::is_codex_app_running();
    process::stop_codex_for_thread_adoption()?;
    let outcome = (|| {
        process::ensure_no_codex_writer_processes()?;
        // Shutdown may flush history or another tool may replace config. Never
        // silently refresh the user's confirmed revision after stopping.
        let current =
            build_recovery_plan_with_lookup(paths, thread_id, RolloutLookup::Refresh)?.preview;
        validate_preview(&current, expected_revision)?;
        let current_port = validate_router_target(paths, &current)?;
        if router_proxy::running_port() != Some(current_port) {
            return Err(CoreError::OperationFailed(
                "THREAD_ROUTER_NOT_READY: the local Router stopped while Codex was closing".into(),
            ));
        }
        apply_thread_recovery_locked(paths, thread_id, expected_revision, true)
    })();
    let safe_to_launch = match &outcome {
        Ok(_) => true,
        Err(_) => {
            was_running
                && build_recovery_plan(paths, thread_id).is_ok_and(|plan| {
                    !matches!(
                        plan.preview.status,
                        ThreadRecoveryStatus::JournalBlocked
                            | ThreadRecoveryStatus::RecoveryPending
                    )
                })
        }
    };
    let launch_error = if safe_to_launch {
        process::launch_codex_app(&newapi::proxy_from_settings(&paths.settings_path))
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    match outcome {
        Ok(result) => Ok((result, launch_error)),
        Err(error) => match launch_error {
            Some(launch) => Err(CoreError::OperationFailed(format!(
                "{error}; CODEX_REOPEN_FAILED: {launch}"
            ))),
            None => Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_or_conflicting_preview_is_rejected_before_stopping_any_process() {
        let (_temp, paths) = super::super::tests::fixture("agentharbor_relay", "openai");
        let mut preview = preview_thread_recovery(&paths, super::super::tests::THREAD_ID).unwrap();
        assert!(validate_preview(&preview, "stale").is_err());
        assert!(validate_preview(&preview, "").is_err());
        assert!(validate_preview(&preview, &preview.revision).is_ok());
        preview.status = ThreadRecoveryStatus::RecoveryPending;
        assert!(validate_preview(&preview, &preview.revision).is_err());
    }

    #[test]
    fn a_provider_named_router_is_not_enough_and_validation_is_read_only() {
        let (_temp, paths) = super::super::tests::fixture("agentharbor_relay", "openai");
        let preview = preview_thread_recovery(&paths, super::super::tests::THREAD_ID).unwrap();
        let before = std::fs::read(&paths.config_path).unwrap();
        assert!(validate_router_target(&paths, &preview).is_err());
        assert_eq!(before, std::fs::read(&paths.config_path).unwrap());
        assert!(!paths.app_data_dir.join("newapi-providers.dat").exists());
    }

    #[test]
    fn adoption_requires_owned_endpoint_credentials_catalog_and_effective_profile() {
        let (_temp, paths) = super::super::tests::fixture("openai", "openai");
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
        // Account-mode routing keeps the built-in provider: nothing to adopt.
        router::set_smart_routing(&paths, true).unwrap();
        let preview = preview_thread_recovery(&paths, super::super::tests::THREAD_ID).unwrap();
        assert_eq!(preview.target_provider, "openai");
        assert!(validate_router_target(&paths, &preview).is_err());
        // Accountless routing still uses a dedicated provider for its threads.
        router::set_accountless(&paths, true).unwrap();
        let store = router::load_store(&paths).unwrap();
        router::apply_routing_config(&paths, &store).unwrap();
        let config = std::fs::read_to_string(&paths.config_path).unwrap();
        let preview = preview_thread_recovery(&paths, super::super::tests::THREAD_ID).unwrap();
        assert_eq!(
            validate_router_target(&paths, &preview).unwrap(),
            store.router.port
        );
        assert_eq!(config, std::fs::read_to_string(&paths.config_path).unwrap());

        for bad in [
            config.replace(&format!("127.0.0.1:{}", store.router.port), "127.0.0.1:1"),
            config.replace(&store.router.bearer_token, "wrong-local-token"),
            format!("profile = 'external'\n{config}\n[profiles.external]\nmodel_provider = 'openai'\n"),
            format!("profile = 'external'\n{config}\n[profiles.external]\nmodel_catalog_json = 'other.json'\n"),
        ] {
            std::fs::write(&paths.config_path, &bad).unwrap();
            let preview = preview_thread_recovery(&paths, super::super::tests::THREAD_ID).unwrap();
            assert!(validate_router_target(&paths, &preview).is_err());
            assert_eq!(bad, std::fs::read_to_string(&paths.config_path).unwrap());
        }
        std::fs::write(&paths.config_path, config).unwrap();
        std::fs::write(router::catalog_path(&paths), "{}").unwrap();
        assert!(validate_router_target(&paths, &preview).is_err());
    }
}
