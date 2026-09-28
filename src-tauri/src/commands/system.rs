use crate::core::models::*;
use crate::core::repository::Repository;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, State};

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
pub async fn clean(app: AppHandle) -> Result<CoreEnvelope<CleanPayload>, String> {
    super::with_repo(app, |repo| repo.clean().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn rebuild_registry(
    app: AppHandle,
) -> Result<CoreEnvelope<RebuildRegistryPayload>, String> {
    super::with_repo(app, |repo| {
        repo.rebuild_registry().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub fn set_auto_switch(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<CoreEnvelope<AutoSwitchConfigPayload>, String> {
    // A scheduler entry from an older build would keep launching a removed entry point.
    crate::platform::daemon::remove_legacy_scheduler();
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
    repo: State<'_, Mutex<Repository>>,
    interval: String,
) -> Result<String, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    repo.set_usage_refresh_interval(&interval)
        .map_err(|e| e.to_string())
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
pub async fn diagnose(app: AppHandle) -> Result<CoreEnvelope<DiagnosePayload>, String> {
    super::with_repo(app, |repo| repo.diagnose().map_err(|e| e.to_string())).await
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

/// Extensions the OS shell would execute instead of opening for viewing.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "app", "bat", "cmd", "com", "cpl", "exe", "hta", "jar", "js", "jse", "lnk", "msc", "msi",
    "pif", "ps1", "reg", "scr", "sh", "url", "vbe", "vbs", "command", "wsf",
];

fn validate_open_path(path: &str) -> Result<std::path::PathBuf, String> {
    let target = std::path::Path::new(path.trim());
    if !target.is_absolute() {
        return Err("Path must be absolute".into());
    }
    let target = target
        .canonicalize()
        .map_err(|_| format!("Path does not exist: {path}"))?;
    let executable = target
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXECUTABLE_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()));
    if executable {
        return Err("Refusing to open an executable path".into());
    }
    Ok(target)
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let path = validate_open_path(&path)?;
    // canonicalize adds a verbatim `\\?\` prefix on Windows, which explorer does not accept.
    #[cfg(target_os = "windows")]
    let path = {
        let text = path.to_string_lossy();
        std::path::PathBuf::from(if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
        })
    };
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

#[cfg(test)]
mod tests {
    use super::validate_open_path;

    #[test]
    fn open_path_rejects_relative_missing_and_executable_targets() {
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_open_path(dir.path().to_str().unwrap()).is_ok());
        let doc = dir.path().join("notes.md");
        std::fs::write(&doc, "x").unwrap();
        assert!(validate_open_path(doc.to_str().unwrap()).is_ok());
        let exe = dir.path().join("tool.EXE");
        std::fs::write(&exe, "x").unwrap();
        assert!(validate_open_path(exe.to_str().unwrap()).is_err());
        assert!(validate_open_path("relative/dir").is_err());
        assert!(validate_open_path(dir.path().join("missing").to_str().unwrap()).is_err());
    }
}
