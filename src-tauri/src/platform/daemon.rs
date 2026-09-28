//! Auto-switch now runs inside the app. Older builds could register a system
//! scheduler entry that launched `daemon-run-once`; this only removes those leftovers.

/// Best effort; a missing entry is not an error.
#[cfg(target_os = "macos")]
pub fn remove_legacy_scheduler() {
    let Some(agents) = dirs::home_dir().map(|home| home.join("Library/LaunchAgents")) else {
        return;
    };
    for (label, file) in [
        ("dev.agentharbor.auto-switch", "dev.agentharbor.auto-switch.plist"),
        ("dev.codexharbor.auto-switch", "dev.codexharbor.auto-switch.plist"),
    ] {
        let path = agents.join(file);
        if path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", "-w"])
                .arg(&path)
                .output();
            let _ = std::fs::remove_file(&path);
        }
        let _ = std::process::Command::new("launchctl")
            .args(["remove", label])
            .output();
    }
}

#[cfg(target_os = "windows")]
pub fn remove_legacy_scheduler() {
    let _ = crate::platform::windows::background_command("schtasks")
        .args(["/Delete", "/TN", "AgentHarborAutoSwitch", "/F"])
        .output();
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn remove_legacy_scheduler() {}
