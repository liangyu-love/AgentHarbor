use crate::commands::accounts::refresh_where;
use crate::core::accounts;
use crate::core::auth::current_timestamp;
use crate::core::auto_switch::{self, Decision, Draining, LoopState, Restart};
use crate::core::repository::Repository;
use crate::core::targets::{TargetId, TargetRegistry};
use crate::platform::paths::CodexPaths;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// The accounts page and settings listen for this to reload after a background switch.
pub const ACCOUNTS_CHANGED_EVENT: &str = "codex-accounts-changed";

fn reopen_codex(paths: &CodexPaths) -> Result<(), String> {
    let registry = TargetRegistry::new(paths.clone());
    registry
        .process(TargetId::Codex)
        .ok_or_else(|| "Unsupported target process controller".to_owned())?
        .restart()
        .map_err(|error| error.to_string())
}

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
        state.draining = None;
        return Ok(());
    }
    if let Some(draining) = state.draining.clone() {
        drain(app, &repo, state, &draining).await?;
    }
    let now = current_timestamp();
    if state.in_cooldown(now) {
        return Ok(());
    }
    let active = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        accounts::list(&repo).map_err(|error| error.to_string())?.active_account_key
    };
    let Some(active) = active else {
        return Ok(());
    };
    // Only the active account is judged each round; the others are asked for
    // their quota only when it is low enough to need one of them.
    let mut refreshed = refresh_where(&repo, |key| key == active).await?;
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());
    if !auto_switch::active_is_low(&refreshed, &config) {
        state.clear_exhausted();
        return Ok(());
    }
    refreshed = refresh_where(&repo, |key| key != active).await?;
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());

    let (target, label) = match auto_switch::decide(&refreshed, &config) {
        Decision::Stay => {
            state.clear_exhausted();
            return Ok(());
        }
        Decision::NoCandidate => {
            if state.should_notify_exhausted(refreshed.active_account_key.as_deref()) {
                let (title, body) = text::no_candidate();
                notify(app, &title, &body);
            }
            return Ok(());
        }
        Decision::Switch { to, to_label, .. } => (to, to_label),
    };
    let used_up = refreshed
        .items
        .iter()
        .find(|item| item.is_active)
        .is_some_and(auto_switch::is_used_up);
    let plan = auto_switch::restart_plan(crate::platform::process::is_codex_app_running(), used_up);
    let paths = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        repo.paths().clone()
    };
    if plan == Restart::Now {
        stop_codex().await?;
    }
    let switched = (|| -> Result<bool, String> {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        // Re-check under the lock: the user may have switched or disabled it meanwhile.
        if !repo.auto_switch_config().enabled
            || accounts::list(&repo)
                .map_err(|error| error.to_string())?
                .active_account_key
                != refreshed.active_account_key
        {
            Ok(false)
        } else {
            accounts::switch(&repo, &target)
                .map(|_| true)
                .map_err(|error| error.to_string())
        }
    })();
    if !matches!(&switched, Ok(true)) {
        if plan == Restart::Now {
            let reopened = tauri::async_runtime::spawn_blocking(move || reopen_codex(&paths)).await;
            if let Err(error) = reopened.unwrap_or_else(|error| Err(error.to_string())) {
                log::warn!("Codex reopen after skipped account switch failed: {error}");
            }
        }
        return switched.map(|_| ());
    }
    state.record_switch(now);
    // Codex reads the injected catalog at startup; fetch the target's first.
    crate::commands::accounts::refresh_model_catalogs(
        &repo,
        Some(&target),
        crate::commands::accounts::CATALOG_REFRESH_INTERVAL,
    )
    .await;
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());

    let (title, body) = match plan {
        Restart::NextStart => text::switched_next_start(&label),
        Restart::WhenUsedUp => {
            state.draining = Some(Draining {
                from: active,
                to: target,
                to_label: label.clone(),
            });
            text::switched_draining(&label)
        }
        Restart::Now => {
            let restarted = tauri::async_runtime::spawn_blocking(move || reopen_codex(&paths))
                .await
                .map_err(|error| error.to_string())?;
            match restarted {
                Ok(()) => text::switched_restarted(&label),
                Err(error) => {
                    log::warn!("auto-switch restart failed: {error}");
                    text::restart_failed(&label)
                }
            }
        }
    };
    notify(app, &title, &body);
    Ok(())
}

/// Codex kept running on the account switched away from: restart it onto the
/// new one once the old one is used up. A Codex closed meanwhile, or another
/// account chosen by the user, ends the wait.
async fn drain(
    app: &AppHandle,
    repo: &Mutex<Repository>,
    state: &mut LoopState,
    draining: &Draining,
) -> Result<(), String> {
    if !crate::platform::process::is_codex_app_running() {
        state.draining = None;
        return Ok(());
    }
    let refreshed = refresh_where(repo, |key| key == draining.from).await?;
    if refreshed.active_account_key.as_deref() != Some(draining.to.as_str()) {
        state.draining = None;
        return Ok(());
    }
    if !refreshed
        .items
        .iter()
        .find(|item| item.account_key == draining.from)
        .is_some_and(auto_switch::is_used_up)
    {
        return Ok(());
    }
    state.draining = None;
    stop_codex().await?;
    let paths = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        // The running Codex may have written its own tokens back while it
        // kept working: put the new account in place again before it starts.
        if accounts::list(&repo)
            .map_err(|error| error.to_string())?
            .active_account_key
            .as_deref()
            != Some(draining.to.as_str())
        {
            accounts::switch(&repo, &draining.to).map_err(|error| error.to_string())?;
        }
        repo.paths().clone()
    };
    let _ = app.emit(ACCOUNTS_CHANGED_EVENT, ());
    let restarted = tauri::async_runtime::spawn_blocking(move || reopen_codex(&paths))
        .await
        .map_err(|error| error.to_string())?;
    let (title, body) = match restarted {
        Ok(()) => text::drained(&draining.to_label),
        Err(error) => {
            log::warn!("restart after the old account was used up failed: {error}");
            text::restart_failed(&draining.to_label)
        }
    };
    notify(app, &title, &body);
    Ok(())
}

async fn stop_codex() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    crate::platform::process::snapshot_codex_path_before_stop();
    tauri::async_runtime::spawn_blocking(|| {
        crate::platform::process::stop_codex_app_gracefully(Duration::from_secs(8))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

/// Notification text in the app's language.
mod text {
    fn pick(zh: String, en: String) -> String {
        if crate::commands::system::ui_language_is_english() {
            en
        } else {
            zh
        }
    }

    pub fn no_candidate() -> (String, String) {
        (
            pick("当前账号额度不足".into(), "The current account is running low".into()),
            pick(
                "没有其他额度充足的账号可切换，自动切换已跳过。".into(),
                "No other account has enough quota left, so auto-switch skipped it.".into(),
            ),
        )
    }

    fn switched() -> String {
        pick("已自动切换账号".into(), "Switched accounts".into())
    }

    pub fn switched_next_start(label: &str) -> (String, String) {
        (
            switched(),
            pick(
                format!("已切换到 {label}，下次启动 Codex 时生效。"),
                format!("Switched to {label}; Codex uses it from its next start."),
            ),
        )
    }

    pub fn switched_draining(label: &str) -> (String, String) {
        (
            switched(),
            pick(
                format!("已切换到 {label}。正在运行的 Codex 会在原账号额度用尽时自动重启，新开的 Codex 已使用新账号。"),
                format!("Switched to {label}. The running Codex restarts onto it once the previous account is used up; a Codex started now uses it already."),
            ),
        )
    }

    pub fn switched_restarted(label: &str) -> (String, String) {
        (
            switched(),
            pick(
                format!("已切换到 {label}，并重启了 Codex。"),
                format!("Switched to {label} and restarted Codex."),
            ),
        )
    }

    pub fn drained(label: &str) -> (String, String) {
        (
            pick("已重启 Codex".into(), "Restarted Codex".into()),
            pick(
                format!("原账号额度已用尽，Codex 已重启并使用 {label}。"),
                format!("The previous account is used up; Codex restarted on {label}."),
            ),
        )
    }

    pub fn restart_failed(label: &str) -> (String, String) {
        (
            switched(),
            pick(
                format!("已切换到 {label}，但重启 Codex 失败，请手动重启。"),
                format!("Switched to {label}, but restarting Codex failed; restart it yourself."),
            ),
        )
    }
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
