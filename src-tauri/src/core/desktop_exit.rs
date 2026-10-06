//! Gateway exit preflight. All filesystem work finishes before process exit.
use super::{
    claude_desktop::{
        self, desktop_app,
        transaction::AppliedMode,
        types::{ActionRequest, DesktopAction, DesktopError, DesktopResult, PathsChoice},
        Manager,
    },
    router::Store,
};

/// `false` means Desktop is running and the caller must obtain a stop decision.
pub fn restore_gateway(
    manager: &Manager,
    store: &Store,
    stop_running: bool,
) -> DesktopResult<bool> {
    let mut process_check = None;
    restore_gateway_with(
        manager,
        store,
        stop_running,
        |paths| {
            process_check
                .get_or_insert_with(|| desktop_app::StopCheck::new(paths))
                .ensure_stopped()
        },
        |paths| desktop_app::stop(paths, std::time::Duration::from_secs(10)).map(|_| ()),
    )
}

fn restore_gateway_with(
    manager: &Manager,
    store: &Store,
    stop_running: bool,
    mut ensure_stopped: impl FnMut(&claude_desktop::paths::DesktopPaths) -> DesktopResult<()>,
    mut stop: impl FnMut(&claude_desktop::paths::DesktopPaths) -> DesktopResult<()>,
) -> DesktopResult<bool> {
    let state = manager.load_state()?;
    let pending = manager.load_pending()?;
    let is_gateway = |state: &claude_desktop::transaction::State| {
        !state.tombstone && state.data.applied_mode == AppliedMode::Router
    };
    let committed = state
        .as_ref()
        .zip(pending.as_ref())
        .is_some_and(|(state, pending)| {
            state.transaction_id == pending.transaction_id && state.generation == pending.generation
        });
    let gateway_involved = state.as_ref().is_some_and(is_gateway)
        || (!committed
            && pending.as_ref().is_some_and(|pending| {
                is_gateway(&pending.next_state)
                    || pending.previous_state.as_ref().is_some_and(is_gateway)
            }));
    if !gateway_involved {
        return Ok(true);
    }
    let reference = pending
        .as_ref()
        .map(|p| &p.next_state)
        .or(state.as_ref())
        .ok_or_else(|| Box::new(DesktopError::new("storage", "claudeDesktop.errors.storage")))?;
    let saved = claude_desktop::paths_from_state(reference);
    claude_desktop::paths::validate_session_paths(&saved)?;
    let choice = PathsChoice {
        normal_root: Some(claude_desktop::paths::display(&saved.normal_root)),
        third_party_root: Some(claude_desktop::paths::display(&saved.third_party_root)),
    };
    // Stored paths do not confer process-control permission. Validate that the
    // selected installation remains a current discovery candidate.
    let paths = manager.resolve_paths(&choice)?;
    if let Err(error) = ensure_stopped(&paths) {
        if error.message != "claudeDesktop.errors.desktopRunning" {
            return Err(error);
        }
        if !stop_running {
            return Ok(false);
        }
        stop(&paths)?;
        ensure_stopped(&paths)?;
    }
    // Interrupted gateway writes need the same stop confirmation as a healthy
    // gateway. Recover first, then restore the resulting committed generation.
    manager.recover_at_startup()?;
    if !manager.load_state()?.as_ref().is_some_and(is_gateway) {
        return Ok(true);
    }
    let request = ActionRequest {
        provider_id: None,
        models: Vec::new(),
        paths: choice,
        rebaseline: false,
        routes: Vec::new(),
        rotate_gateway_token: false,
    };
    let prepared = manager.prepare(store, DesktopAction::RestorePrevious, &request)?;
    let preview = manager.issue_ticket(&prepared)?;
    manager.commit(
        store,
        DesktopAction::RestorePrevious,
        &preview.revision,
        &request,
    )?;
    Ok(true)
}

pub fn encode(error: Box<DesktopError>) -> String {
    serde_json::to_string(&*error).unwrap_or_else(|_| "claudeDesktop.errors.storage".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::claude_desktop::{
        paths::{role, DesktopPaths},
        transaction,
        types::{DesktopRouteChoice, ListenerLease, OneMMode},
    };
    use crate::core::router::{ModelProtocol, Provider};
    use std::cell::Cell;

    fn fixture() -> (
        tempfile::TempDir,
        DesktopPaths,
        Manager,
        Store,
        ActionRequest,
    ) {
        let root = tempfile::tempdir().unwrap();
        let paths =
            DesktopPaths::from_roots(root.path().join("Claude"), root.path().join("Claude-3p"))
                .unwrap();
        std::fs::create_dir_all(&paths.third_party_root).unwrap();
        std::fs::create_dir_all(&paths.normal_root).unwrap();
        std::fs::write(
            &paths.normal_config,
            r#"{"deploymentMode":"1p","external":true}"#,
        )
        .unwrap();
        let store = Store {
            providers: vec![Provider {
                id: "exit-fixture".into(),
                name: "Fixture".into(),
                enabled: true,
                base_url: "https://relay.example".into(),
                api_key: "synthetic-key".into(),
                model: "gpt-fixture".into(),
                protocol: ModelProtocol::Responses,
                ..Default::default()
            }],
            ..Default::default()
        };
        let manager = Manager::new(root.path())
            .with_paths(paths.clone())
            .with_listener_lease(ListenerLease {
                bound_port: store.router.port,
                generation: 1,
                process_epoch: "isolated-exit".into(),
            });
        let request = ActionRequest {
            provider_id: None,
            models: Vec::new(),
            paths: Default::default(),
            rebaseline: false,
            routes: vec![DesktopRouteChoice {
                target_ref: None,
                role: "sonnet".into(),
                role_alias: "claude-sonnet-4-6".into(),
                provider_id: "exit-fixture".into(),
                model_id: "gpt-fixture".into(),
                label_override: None,
                one_m_mode: OneMMode::Off,
            }],
            rotate_gateway_token: false,
        };
        let prepared = manager
            .prepare(&store, DesktopAction::ApplyRouter, &request)
            .unwrap();
        let preview = manager.issue_ticket(&prepared).unwrap();
        manager
            .commit(
                &store,
                DesktopAction::ApplyRouter,
                &preview.revision,
                &request,
            )
            .unwrap();
        (root, paths, manager, store, request)
    }

    fn running() -> Box<DesktopError> {
        Box::new(DesktopError::new(
            "process",
            "claudeDesktop.errors.desktopRunning",
        ))
    }

    #[test]
    fn running_desktop_keeps_gateway_until_stop_and_restore_both_succeed() {
        let _env = claude_desktop::env_test_lock();
        let (_root, paths, manager, store, _) = fixture();
        let before = transaction::snapshot(&paths).unwrap();
        let token = manager
            .load_state()
            .unwrap()
            .unwrap()
            .router
            .unwrap()
            .gateway_token;
        assert!(!restore_gateway_with(
            &manager,
            &store,
            false,
            |_| Err(running()),
            |_| panic!("no stop authorization")
        )
        .unwrap());
        let failed = restore_gateway_with(
            &manager,
            &store,
            true,
            |_| Err(running()),
            |_| {
                Err(Box::new(DesktopError::new(
                    "process",
                    "claudeDesktop.errors.processStopTimeout",
                )))
            },
        )
        .unwrap_err();
        assert_eq!(failed.message, "claudeDesktop.errors.processStopTimeout");
        for file in &before {
            assert_eq!(
                std::fs::read(&file.path).ok(),
                file.text.as_ref().map(|t| t.as_bytes().to_vec())
            );
        }
        assert_eq!(
            manager
                .load_state()
                .unwrap()
                .unwrap()
                .router
                .unwrap()
                .gateway_token,
            token
        );
        let stopped = Cell::new(false);
        assert!(restore_gateway_with(
            &manager,
            &store,
            true,
            |_| {
                if stopped.get() {
                    Ok(())
                } else {
                    Err(running())
                }
            },
            |_| {
                stopped.set(true);
                Ok(())
            }
        )
        .unwrap());
        assert!(manager.load_state().unwrap().unwrap().tombstone);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(&paths.normal_config).unwrap()
            )
            .unwrap(),
            serde_json::json!({"deploymentMode": "1p", "external": true})
        );
        assert!(!paths.profile_path.exists());
        assert!(manager.load_pending().unwrap().is_none());
    }

    #[test]
    fn exit_recovers_an_interrupted_mode_switch_before_restoring_first_baseline() {
        let _env = claude_desktop::env_test_lock();
        let (_root, paths, manager, store, _) = fixture();
        let request = ActionRequest {
            provider_id: None,
            models: Vec::new(),
            paths: Default::default(),
            rebaseline: false,
            routes: Vec::new(),
            rotate_gateway_token: false,
        };
        let prepared = manager
            .prepare(&store, DesktopAction::SwitchOfficial, &request)
            .unwrap();
        let pending = transaction::Pending {
            schema: transaction::STATE_SCHEMA,
            transaction_id: prepared.next_state.transaction_id.clone(),
            generation: prepared.next_state.generation,
            action: "switchOfficial".into(),
            created_at: chrono::Utc::now().timestamp(),
            paths_identity: prepared.paths.identity(),
            files: prepared.plans.clone(),
            before: prepared.before.clone(),
            previous_state: prepared.previous_state.clone(),
            next_state: prepared.next_state.clone(),
            backup_id: None,
        };
        manager
            .write_pending_for_test(&serde_json::to_vec(&pending).unwrap())
            .unwrap();
        transaction::apply_plan(
            prepared
                .plans
                .iter()
                .find(|p| p.role == role::PROFILE)
                .unwrap(),
        )
        .unwrap();
        assert!(!restore_gateway_with(
            &manager,
            &store,
            false,
            |_| Err(running()),
            |_| panic!("no stop authorization")
        )
        .unwrap());
        assert!(manager.load_pending().unwrap().is_some());
        assert!(restore_gateway_with(
            &manager,
            &store,
            true,
            |_| Ok(()),
            |_| panic!("already stopped")
        )
        .unwrap());
        assert!(manager.load_pending().unwrap().is_none());
        assert!(manager.load_state().unwrap().unwrap().tombstone);
        assert!(!paths.profile_path.exists());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(&paths.normal_config).unwrap()
            )
            .unwrap(),
            serde_json::json!({"deploymentMode": "1p", "external": true})
        );
    }

    #[test]
    fn direct_and_official_states_do_not_gain_a_gateway_exit_prompt() {
        let _env = claude_desktop::env_test_lock();
        let (_root, _, manager, store, _) = fixture();
        for mode in [AppliedMode::Direct, AppliedMode::Official] {
            let mut state = manager.load_state().unwrap().unwrap();
            state.data.applied_mode = mode;
            state.router = None;
            manager.write_state_for_test(&state).unwrap();
            assert!(restore_gateway_with(
                &manager,
                &store,
                false,
                |_| panic!("no gateway process dependency"),
                |_| panic!("no stop")
            )
            .unwrap());
        }
    }

    #[test]
    fn a_committed_restore_with_pending_cleanup_does_not_stop_desktop_again() {
        let _env = claude_desktop::env_test_lock();
        let (_root, _, manager, store, _) = fixture();
        let request = ActionRequest {
            provider_id: None,
            models: Vec::new(),
            paths: Default::default(),
            rebaseline: false,
            routes: Vec::new(),
            rotate_gateway_token: false,
        };
        let prepared = manager
            .prepare(&store, DesktopAction::RestorePrevious, &request)
            .unwrap();
        let pending = transaction::Pending {
            schema: transaction::STATE_SCHEMA,
            transaction_id: prepared.next_state.transaction_id.clone(),
            generation: prepared.next_state.generation,
            action: "restorePrevious".into(),
            created_at: chrono::Utc::now().timestamp(),
            paths_identity: prepared.paths.identity(),
            files: prepared.plans.clone(),
            before: prepared.before.clone(),
            previous_state: prepared.previous_state.clone(),
            next_state: prepared.next_state.clone(),
            backup_id: None,
        };
        for plan in &prepared.plans {
            transaction::apply_plan(plan).unwrap();
        }
        manager
            .write_pending_for_test(&serde_json::to_vec(&pending).unwrap())
            .unwrap();
        manager.write_state_for_test(&prepared.next_state).unwrap();
        assert!(restore_gateway_with(
            &manager,
            &store,
            false,
            |_| panic!("committed restore removed gateway dependency"),
            |_| panic!("no stop")
        )
        .unwrap());
        assert!(manager.load_state().unwrap().unwrap().tombstone);
    }
}
