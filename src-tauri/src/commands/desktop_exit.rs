//! One preflight for tray Quit, updater restart and ExitRequested.
use crate::core::{
    claude_desktop::Manager as DesktopManager, desktop_exit, repository::Repository, router,
    routing_runtime,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExitIntent {
    Quit,
    Restart,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitRequest {
    pub intent: ExitIntent,
    pub error: Option<String>,
}

#[derive(Default)]
struct ExitState {
    pending: Option<ExitRequest>,
    busy: bool,
    approved: bool,
}

#[derive(Default)]
pub struct ExitCoordinator(Mutex<ExitState>);

impl ExitCoordinator {
    pub fn approved(&self) -> bool {
        self.0.lock().map(|s| s.approved).unwrap_or(false)
    }
    fn begin(&self, intent: ExitIntent) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.busy || state.pending.is_some() || state.approved {
            return false;
        }
        state.pending = Some(ExitRequest {
            intent,
            error: None,
        });
        state.busy = true;
        true
    }
    fn waiting(&self) -> Option<ExitRequest> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.busy {
            None
        } else {
            state.pending.clone()
        }
    }
    fn defer(&self, error: Option<String>) -> Option<ExitRequest> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.busy = false;
        if let Some(request) = &mut state.pending {
            request.error = error;
        }
        state.pending.clone()
    }
    fn approve(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.approved = true;
        state.pending = None;
        state.busy = false;
    }
}

pub fn request_exit(app: &AppHandle, intent: ExitIntent) {
    if !app.state::<ExitCoordinator>().begin(intent) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = attempt_exit(&app, intent, false, false).await;
    });
}

fn show_request(app: &AppHandle, error: Option<String>) {
    if let Some(request) = app.state::<ExitCoordinator>().defer(error) {
        crate::commands::hotspot::force_reveal_main_window(app);
        let _ = app.emit("claude-desktop-exit-required", request);
    }
}

async fn attempt_exit(
    app: &AppHandle,
    intent: ExitIntent,
    stop_running: bool,
    force_codex: bool,
) -> Result<(), String> {
    let _guard = routing_runtime::mutation_guard().await;
    let paths = app
        .state::<Mutex<Repository>>()
        .lock()
        .map_err(|_| "claudeDesktop.errors.storage".to_owned())
        .map(|repo| repo.paths().clone());
    let result = match paths {
        Ok(paths) => {
            let restore_paths = paths.clone();
            let result = super::blocking(move || {
                let store = router::load_store(&restore_paths)?;
                let restored = desktop_exit::restore_gateway(
                    &DesktopManager::new(&restore_paths.app_data_dir),
                    &store,
                    stop_running,
                )
                .map_err(desktop_exit::encode)?;
                if restored {
                    router::change_codex_connection(
                        &restore_paths,
                        router::CodexConnectionTarget::Disabled,
                        force_codex,
                    )?;
                }
                Ok(restored)
            })
            .await;
            if result == Ok(true) {
                routing_runtime::release_all_preview_listeners();
                // A restored Desktop no longer owns the listener. Existing CLI
                // clients are restored by the final Exit callback.
                let _ = routing_runtime::reconcile(&paths).await;
            }
            result
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(true) => {
            if let Err(error) = finish_exit(app, intent) {
                show_request(app, Some(error.clone()));
                return Err(error);
            }
            Ok(())
        }
        Ok(false) => {
            show_request(app, None);
            Ok(())
        }
        Err(error) => {
            show_request(app, Some(error.clone()));
            Err(error)
        }
    }
}

fn finish_exit(app: &AppHandle, intent: ExitIntent) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if intent == ExitIntent::Restart {
        let exe = std::env::current_exe().map_err(|_| "claudeDesktop.errors.process")?;
        let bundle = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .ok_or("claudeDesktop.errors.process")?;
        // This helper is scheduled only after the gateway restore has succeeded.
        std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 1 && exec open \"$1\"")
            .arg("agentharbor-relaunch")
            .arg(bundle)
            .spawn()
            .map_err(|_| "claudeDesktop.errors.process")?;
    }
    app.state::<ExitCoordinator>().approve();
    #[cfg(not(target_os = "macos"))]
    if intent == ExitIntent::Restart {
        app.request_restart();
        return Ok(());
    }
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn get_claude_desktop_exit_request(
    coordinator: State<'_, ExitCoordinator>,
) -> Option<ExitRequest> {
    coordinator.waiting()
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum ExitDecision {
    Background,
    StopAndExit,
    ForceStopAndExit,
}

#[tauri::command]
pub async fn resolve_claude_desktop_exit(
    app: AppHandle,
    decision: ExitDecision,
) -> Result<(), String> {
    if matches!(decision, ExitDecision::Background) {
        let request = {
            let coordinator = app.state::<ExitCoordinator>();
            let mut state = coordinator
                .0
                .lock()
                .map_err(|_| "claudeDesktop.errors.storage")?;
            if state.busy {
                return Err("claudeDesktop.errors.writerBusy".into());
            }
            state.pending.take()
        };
        if let Some(request) = request {
            let _ = app.emit("claude-desktop-exit-cancelled", request);
        }
        return Ok(());
    }
    let intent = {
        let coordinator = app.state::<ExitCoordinator>();
        let mut state = coordinator
            .0
            .lock()
            .map_err(|_| "claudeDesktop.errors.storage")?;
        if state.busy {
            return Err("claudeDesktop.errors.writerBusy".into());
        }
        match decision {
            ExitDecision::Background => unreachable!(),
            ExitDecision::StopAndExit | ExitDecision::ForceStopAndExit => {
                if matches!(decision, ExitDecision::ForceStopAndExit)
                    && !state
                        .pending
                        .as_ref()
                        .and_then(|request| request.error.as_deref())
                        .is_some_and(|error| error.starts_with("CODEX_ROUTING_RESTORE_FAILED:"))
                {
                    return Err("claudeDesktop.errors.previewExpired".into());
                }
                let intent = state
                    .pending
                    .as_ref()
                    .ok_or("claudeDesktop.errors.previewExpired")?
                    .intent;
                state.busy = true;
                intent
            }
        }
    };
    attempt_exit(
        &app,
        intent,
        true,
        matches!(decision, ExitDecision::ForceStopAndExit),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_exit_and_restart_requests_do_not_schedule_a_second_action() {
        let coordinator = ExitCoordinator::default();
        assert!(coordinator.begin(ExitIntent::Quit));
        assert!(!coordinator.begin(ExitIntent::Restart));
        assert!(coordinator.waiting().is_none());
        assert_eq!(coordinator.defer(None).unwrap().intent, ExitIntent::Quit);
        assert!(!coordinator.approved());
        assert!(!coordinator.begin(ExitIntent::Restart));
        coordinator.approve();
        assert!(coordinator.approved());
        assert!(coordinator.waiting().is_none());
    }
}
