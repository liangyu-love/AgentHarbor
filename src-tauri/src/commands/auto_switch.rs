use crate::commands::accounts::refresh_all;
use crate::core::accounts;
use crate::core::auth::current_timestamp;
use crate::core::auto_switch::{self, Decision, LoopState};
use crate::core::repository::Repository;
use crate::core::targets::{TargetId, TargetRegistry};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// The accounts page and settings listen for this to reload after a background switch.
pub const ACCOUNTS_CHANGED_EVENT: &str = "codex-accounts-changed";

fn interval_secs(interval: &str) -> u64 {
    let secs = match interval {
        "30s" => 30,
        "3m" => 180,
        "5m" => 300,
        _ => 60,
    };
    secs.max(auto_switch::MIN_CHECK_INTERVAL_SECS)
}

/// Runs for the lifetime of the app. Each round reads the setting afresh, so turning
/// auto-switch on or off takes effect by the next round without restarting the loop.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut state = LoopState::default();
        loop {
            let wait = {
                let repo = app.state::<Mutex<Repository>>();
                let interval = repo
                    .lock()
                    .map(|repo| repo.get_usage_refresh_interval())
                    .unwrap_or_default();
                interval_secs(&interval)
            };
            tokio::time::sleep(Duration::from_secs(wait)).await;
            if let Err(error) = run_once(&app, &mut state).await {
                log::warn!("auto-switch round failed: {error}");
            }
        }
    });
}

async fn run_once(app: &AppHandle, state: &mut LoopState) -> Result<(), String> {
    let repo = app.state::<Mutex<Repository>>();
    let config = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        repo.auto_switch_config()
    };
    if !config.enabled {
        state.clear_exhausted();
        return Ok(());
    }
    let now = current_timestamp();
    if state.in_cooldown(now) {
        return Ok(());
    }
    let refreshed = refresh_all(&repo).await?;
    // The refresh stored new quotas; let open pages pick them up either way.
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());

    let (target, label) = match auto_switch::decide(&refreshed, &config) {
        Decision::Stay => {
            state.clear_exhausted();
            return Ok(());
        }
        Decision::NoCandidate => {
            if state.should_notify_exhausted(refreshed.active_account_key.as_deref()) {
                notify(
                    app,
                    "当前账号额度不足",
                    "没有其他额度充足的账号可切换，自动切换已跳过。",
                );
            }
            return Ok(());
        }
        Decision::Switch { to, to_label, .. } => (to, to_label),
    };

    let paths = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        // Re-check under the lock: the user may have switched or disabled it meanwhile.
        if !repo.auto_switch_config().enabled
            || accounts::list(&repo)
                .map_err(|error| error.to_string())?
                .active_account_key
                != refreshed.active_account_key
        {
            return Ok(());
        }
        accounts::switch(&repo, &target).map_err(|error| error.to_string())?;
        repo.paths().clone()
    };
    state.record_switch(now);
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());

    // Codex only reads auth.json on start. Leave it closed if it was not running.
    let restarted = tauri::async_runtime::spawn_blocking(move || {
        if !crate::platform::process::is_codex_app_running() {
            return Ok(false);
        }
        let registry = TargetRegistry::new(paths);
        registry
            .process(TargetId::Codex)
            .ok_or_else(|| "Unsupported target process controller".to_owned())?
            .restart()
            .map(|()| true)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?;

    let body = match restarted {
        Ok(true) => format!("已切换到 {label}，并重启了 Codex。"),
        Ok(false) => format!("已切换到 {label}，下次启动 Codex 时生效。"),
        Err(error) => {
            log::warn!("auto-switch restart failed: {error}");
            format!("已切换到 {label}，但重启 Codex 失败，请手动重启。")
        }
    };
    notify(app, "已自动切换账号", &body);
    Ok(())
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        log::warn!("auto-switch notification failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_interval_follows_the_refresh_setting_with_a_floor() {
        assert_eq!(interval_secs("30s"), 60);
        assert_eq!(interval_secs("1m"), 60);
        assert_eq!(interval_secs("3m"), 180);
        assert_eq!(interval_secs("5m"), 300);
        assert_eq!(interval_secs("bogus"), 60);
    }
}
