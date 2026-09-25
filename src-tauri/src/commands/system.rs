use crate::core::models::*;
use crate::core::repository::{usage_refresh_interval_seconds, Repository};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn load_snapshot(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<CoreSnapshotPayload>, String> {
    repo.lock()
        .map_err(|e| e.to_string())?
        .load_snapshot_local()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_updater_configured(app: AppHandle) -> bool {
    updater_configured(app.config())
}

pub(crate) fn updater_configured(config: &tauri::Config) -> bool {
    config.plugins.0.get("updater").map_or(false, |config| {
        config
            .get("pubkey")
            .and_then(|v| v.as_str())
            .map_or(false, |v| !v.is_empty())
            && config
                .get("endpoints")
                .and_then(|v| v.as_array())
                .map_or(false, |v| !v.is_empty())
    })
}

#[tauri::command]
pub fn clean(repo: State<'_, Mutex<Repository>>) -> Result<CoreEnvelope<CleanPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.clean().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rebuild_registry(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<RebuildRegistryPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.rebuild_registry().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_auto_switch(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<CoreEnvelope<AutoSwitchConfigPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.set_auto_switch(enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn configure_auto_switch(
    repo: State<'_, Mutex<Repository>>,
    threshold_5h_percent: Option<i32>,
    threshold_weekly_percent: Option<i32>,
) -> Result<CoreEnvelope<AutoSwitchConfigPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.configure_auto_switch(threshold_5h_percent, threshold_weekly_percent)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_api_proxy_config(
    repo: State<'_, Mutex<Repository>>,
    mode: ApiProxyMode,
    url: Option<String>,
) -> Result<CoreEnvelope<ApiModePayload>, String> {
    let result = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        repo.set_api_proxy_config(mode, url)
            .map_err(|e| e.to_string())?
    };
    crate::core::router_proxy::update_proxy(&result.data.api.proxy)?;
    Ok(result)
}

#[tauri::command]
pub fn get_usage_refresh_interval(repo: State<'_, Mutex<Repository>>) -> Result<String, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    Ok(repo.get_usage_refresh_interval())
}

#[tauri::command]
pub fn set_usage_refresh_interval(
    app: AppHandle,
    repo: State<'_, Mutex<Repository>>,
    interval: String,
) -> Result<String, String> {
    let normalized = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        repo.set_usage_refresh_interval(&interval)
            .map_err(|e| e.to_string())?
    };

    let repo_state = app.state::<Mutex<Repository>>();
    let _interval_seconds = {
        let repo = repo_state.lock().map_err(|e| e.to_string())?;
        usage_refresh_interval_seconds(&repo.get_usage_refresh_interval())
    };
    Ok(normalized)
}

#[tauri::command]
pub async fn test_api_proxy_config(
    mode: ApiProxyMode,
    url: Option<String>,
) -> Result<CoreEnvelope<ApiProxyTestPayload>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let payload =
            crate::core::api_client::test_api_connectivity(&ApiProxyConfigPayload { mode, url });
        Ok(CoreEnvelope::ok(payload))
    })
    .await
    .map_err(|e| format!("Blocking command task failed: {e}"))?
}

#[tauri::command]
pub async fn detect_api_proxy_config() -> Result<CoreEnvelope<ApiProxyDetectPayload>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let payload = crate::core::api_client::detect_api_proxy_config();
        Ok(CoreEnvelope::ok(payload))
    })
    .await
    .map_err(|e| format!("Blocking command task failed: {e}"))?
}

#[tauri::command]
pub fn run_daemon_once() -> Result<CoreEnvelope<DaemonRunPayload>, String> {
    Err("Automatic account switching is not available in this source build".into())
}

#[tauri::command]
pub fn diagnose(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<DiagnosePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.diagnose().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restart_codex(repo: State<'_, Mutex<Repository>>) -> Result<(), String> {
    let (proxy, paths) = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        (
            crate::core::newapi::proxy_from_settings(&repo.paths().settings_path),
            repo.paths().clone(),
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::platform::process::restart_codex_app(&proxy, &paths).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn load_bootstrap_state(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<crate::core::bootstrap_cache::BootstrapStatePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(repo.load_bootstrap_state()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub hostname: String,
}

#[tauri::command]
pub fn get_system_info() -> Result<SystemInfo, String> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let os_version = get_os_version();

    Ok(SystemInfo {
        os,
        os_version,
        arch,
        hostname,
    })
}

fn get_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        get_windows_os_version().unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "unknown".to_string()
    }
}

#[cfg(target_os = "windows")]
fn get_windows_os_version() -> Option<String> {
    let query_value = |name: &str| -> Option<String> {
        let output = crate::platform::windows::background_command("reg")
            .args([
                "query",
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "/v",
                name,
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        stdout.lines().find_map(|line| {
            if !line.contains(name) {
                return None;
            }
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let _kind = parts.next()?;
            let value = parts.collect::<Vec<_>>().join(" ");
            if key.eq_ignore_ascii_case(name) && !value.trim().is_empty() {
                Some(value.trim().to_string())
            } else {
                None
            }
        })
    };

    let product_name = query_value("ProductName");
    let display_version = query_value("DisplayVersion").or_else(|| query_value("ReleaseId"));
    let current_build = query_value("CurrentBuild");

    let mut parts = Vec::new();
    if let Some(name) = product_name {
        parts.push(name);
    }
    if let Some(version) = display_version {
        parts.push(version);
    }
    if let Some(build) = current_build {
        parts.push(format!("build {build}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

#[tauri::command]
pub fn graceful_restart_for_update(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let bundle = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .ok_or_else(|| "cannot resolve app bundle path".to_string())?;
        let bundle_str = bundle.to_string_lossy().to_string();

        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 1 && open \"{}\"", bundle_str))
            .spawn()
            .map_err(|e| e.to_string())?;

        app.exit(0);
    }

    #[cfg(not(target_os = "macos"))]
    {
        app.restart();
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[tauri::command]
pub fn check_update_installability() -> Result<UpdateInstallabilityPayload, String> {
    Ok(crate::platform::update::check_update_installability())
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        crate::platform::windows::background_command("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
