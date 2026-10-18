//! Starting a client from its panel once its configuration changed: a CLI in
//! a new terminal in the folder the user picks, Codex and Harness Desktop as
//! apps (restarted when they run), and a command to copy where nothing runs
//! interactively.
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::targets::TargetId;

const HARNESS_DESKTOP: &str = "DeepSeek Harness";
/// Harness profiles that answer one job or serve a program, not a person.
const HARNESS_SERVICE_PROFILES: [&str; 4] = ["headless", "sdk", "sdk-minimal", "acp"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Launch {
    /// A command a new terminal runs in the chosen folder.
    #[serde(rename_all = "camelCase")]
    Terminal { command: String, installed: bool },
    /// An app, started or (when it runs) restarted.
    #[serde(rename_all = "camelCase")]
    App { installed: bool, running: bool },
    /// Nothing to start by hand: the command to run it with.
    #[serde(rename_all = "camelCase")]
    Copy { command: String },
}

fn program(target: TargetId) -> &'static str {
    match target {
        TargetId::Codex => "codex",
        TargetId::ClaudeCode => "claude",
        TargetId::OpenCode => "opencode",
        TargetId::Pi => "pi",
        TargetId::Harness => "dsh",
    }
}

/// The command a terminal runs for `target`, a Harness one for its profile.
fn command(target: TargetId, profile: Option<&str>) -> String {
    match (target, profile) {
        (TargetId::Harness, Some("web")) => "dsh web".into(),
        (TargetId::Harness, Some(profile)) if HARNESS_SERVICE_PROFILES.contains(&profile) => {
            format!("dsh --profile {profile} \"…\"")
        }
        (TargetId::Harness, Some(profile)) => format!("dsh --profile {profile}"),
        (target, _) => program(target).into(),
    }
}

fn cli_installed(target: TargetId) -> bool {
    super::account_sources::cli::find(&[program(target)], &[], |_| true).is_some()
}

/// Harness Desktop's executable, wherever its installer put it.
fn harness_desktop() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let exe = format!("{HARNESS_DESKTOP}.exe");
        let recorded = crate::platform::windows::uninstall_location(HARNESS_DESKTOP)
            .map(|folder| folder.join(&exe));
        let per_user =
            dirs::data_local_dir().map(|dir| dir.join("Programs").join(HARNESS_DESKTOP).join(&exe));
        [recorded, per_user]
            .into_iter()
            .flatten()
            .find(|path| path.is_file())
    }
    #[cfg(target_os = "macos")]
    {
        Some(PathBuf::from(format!(
            "/Applications/{HARNESS_DESKTOP}.app"
        )))
        .filter(|path| path.is_dir())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// How `target` starts; a Harness one by the profile it was configured in.
pub fn plan(target: TargetId, profile: Option<&str>) -> Launch {
    match (target, profile) {
        (TargetId::Codex, _) => Launch::App {
            installed: true,
            running: crate::platform::process::is_codex_app_running(),
        },
        // The CLI refuses the profile Harness Desktop owns.
        (TargetId::Harness, Some("desktop")) => Launch::App {
            installed: harness_desktop().is_some(),
            running: crate::platform::process::is_process_running(HARNESS_DESKTOP),
        },
        (TargetId::Harness, Some(name)) if HARNESS_SERVICE_PROFILES.contains(&name) => {
            Launch::Copy {
                command: command(target, profile),
            }
        }
        _ => Launch::Terminal {
            command: command(target, profile),
            installed: cli_installed(target),
        },
    }
}

/// Starts `target` the way [`plan`] describes: a CLI in a terminal in `folder`,
/// `direct_hosts` (its direct cards' relays) kept off any proxy it is given.
pub fn launch(
    target: TargetId,
    profile: Option<&str>,
    folder: Option<&Path>,
    direct_hosts: &[String],
) -> Result<(), String> {
    match plan(target, profile) {
        Launch::App { .. } if target == TargetId::Harness => restart_harness_desktop(),
        Launch::App { .. } | Launch::Copy { .. } => Err("clients.errors.launch".into()),
        Launch::Terminal {
            installed: false, ..
        } => Err("clients.errors.cliMissing".into()),
        Launch::Terminal { command, .. } => {
            let folder = folder
                .filter(|folder| folder.is_dir())
                .ok_or("clients.errors.launchFolder")?;
            open_terminal(&format!("{}{command}", proxy_prelude(direct_hosts)), folder)
        }
    }
}

/// The relay hosts of the direct cards `target`'s own configuration reaches;
/// none when it goes through the router, which picks each card's exit.
pub fn direct_relay_hosts(paths: &crate::platform::paths::CodexPaths, target: TargetId) -> Vec<String> {
    use super::client_config::{ClientPaths, ConfigManager};
    let manager = ConfigManager::new(&paths.app_data_dir, ClientPaths::discover());
    let Ok(Some(request)) = manager.stored_request(target) else {
        return Vec::new();
    };
    if request.needs_router() {
        return Vec::new();
    }
    let Ok(store) = super::router::load_store(paths) else {
        return Vec::new();
    };
    let mut hosts = Vec::new();
    for provider in store.providers.iter().filter(|provider| {
        provider.network_mode() == "direct"
            && (provider.id == request.provider_id || request.options.provider_ids.contains(&provider.id))
    }) {
        if let Some(host) = provider.relay_host().filter(|host| !hosts.contains(host)) {
            hosts.push(host);
        }
    }
    hosts
}

/// PowerShell that gives a new terminal the proxy variables a terminal opened
/// now has: this app's environment is the one it started with, from before
/// the terminal proxy last changed. While a proxy applies, `direct_hosts`
/// join `NO_PROXY`.
#[cfg(windows)]
fn proxy_prelude(direct_hosts: &[String]) -> String {
    let mut values = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"]
        .map(|name| (name, crate::platform::windows::current_environment_text(name)));
    let proxied = values[..3]
        .iter()
        .any(|(_, value)| value.as_deref().is_some_and(|value| !value.trim().is_empty()));
    if proxied && !direct_hosts.is_empty() {
        values[3].1 = Some(with_hosts(values[3].1.as_deref(), direct_hosts));
    }
    values
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("$env:{name}='{}'; ", value.replace('\'', "''")),
            None => format!("Remove-Item Env:{name} -ErrorAction SilentlyContinue; "),
        })
        .collect()
}

/// Shell that adds `direct_hosts` to the terminal's bypass lists.
#[cfg(not(windows))]
fn proxy_prelude(direct_hosts: &[String]) -> String {
    if direct_hosts.is_empty() {
        return String::new();
    }
    let hosts = direct_hosts.join(",");
    format!("export NO_PROXY=\"${{NO_PROXY:+$NO_PROXY,}}{hosts}\" no_proxy=\"${{no_proxy:+$no_proxy,}}{hosts}\" && ")
}

/// A `NO_PROXY` list with `hosts` added; loopback stays off the proxy.
#[cfg(any(windows, test))]
fn with_hosts(list: Option<&str>, hosts: &[String]) -> String {
    let list = list.filter(|list| !list.trim().is_empty()).unwrap_or("localhost,127.0.0.1,::1");
    let mut entries: Vec<&str> = Vec::new();
    for entry in list.split(',').map(str::trim).chain(hosts.iter().map(String::as_str)) {
        if !entry.is_empty() && !entries.iter().any(|known| known.eq_ignore_ascii_case(entry)) {
            entries.push(entry);
        }
    }
    entries.join(",")
}

fn restart_harness_desktop() -> Result<(), String> {
    let executable = harness_desktop().ok_or("clients.errors.launch")?;
    #[cfg(windows)]
    {
        crate::platform::process::restart_windows_app(
            &format!("{HARNESS_DESKTOP}.exe"),
            &executable,
        )
        .map_err(|_| "clients.errors.launch".into())
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("quit app \"{HARNESS_DESKTOP}\"")])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        std::process::Command::new("open")
            .arg(executable)
            .spawn()
            .map(|_| ())
            .map_err(|_| "clients.errors.launch".into())
    }
}

/// A visible terminal in `folder` running `command`, left open once it exits:
/// Windows Terminal when installed, else a PowerShell console.
#[cfg(windows)]
fn open_terminal(command: &str, folder: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;
    use base64::Engine as _;
    // `;` separates Windows Terminal's own commands, so the script travels
    // encoded (UTF-16LE, as PowerShell takes it).
    let script = base64::engine::general_purpose::STANDARD
        .encode(command.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    // An npm CLI's PowerShell shim must run even under a restrictive policy.
    let shell = [
        "powershell.exe",
        "-NoLogo",
        "-NoExit",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &script,
    ];
    let terminal = dirs::data_local_dir()
        .map(|dir| dir.join(r"Microsoft\WindowsApps\wt.exe"))
        .filter(|path| path.exists());
    let started = match terminal {
        Some(terminal) => std::process::Command::new(terminal)
            .arg("-d")
            .arg(folder)
            .args(shell)
            .spawn(),
        None => std::process::Command::new(shell[0])
            .args(&shell[1..])
            .current_dir(folder)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn(),
    };
    started
        .map(|_| ())
        .map_err(|_| "clients.errors.launch".into())
}

#[cfg(not(windows))]
fn open_terminal(command: &str, folder: &Path) -> Result<(), String> {
    let folder = folder.to_string_lossy().replace('\'', r"'\''");
    crate::platform::process::open_terminal_with_command(&format!("cd '{folder}' && {command}"))
        .map_err(|_| "clients.errors.launch".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_profiles_choose_how_they_start() {
        assert_eq!(command(TargetId::Harness, Some("web")), "dsh web");
        assert_eq!(
            command(TargetId::Harness, Some("dsh-tui")),
            "dsh --profile dsh-tui"
        );
        assert_eq!(command(TargetId::ClaudeCode, None), "claude");
        assert!(
            matches!(plan(TargetId::Harness, Some("headless")), Launch::Copy { command } if command.starts_with("dsh --profile headless"))
        );
        assert!(matches!(
            plan(TargetId::Harness, Some("desktop")),
            Launch::App { .. }
        ));
        assert_eq!(
            launch(TargetId::Harness, Some("sdk"), None, &[]).unwrap_err(),
            "clients.errors.launch",
            "nothing interactive runs for a service profile"
        );
    }

    #[test]
    fn a_terminal_needs_an_existing_folder() {
        if !cli_installed(TargetId::Pi) {
            return;
        }
        let missing = std::env::temp_dir().join("agentharbor-missing-folder-for-launch");
        assert_eq!(
            launch(TargetId::Pi, None, Some(&missing), &[]).unwrap_err(),
            "clients.errors.launchFolder"
        );
        assert_eq!(
            launch(TargetId::Pi, None, None, &[]).unwrap_err(),
            "clients.errors.launchFolder"
        );
    }

    #[test]
    fn a_direct_relay_joins_the_bypass_list() {
        let hosts = ["relay.example".to_owned()];
        assert_eq!(with_hosts(None, &hosts), "localhost,127.0.0.1,::1,relay.example");
        assert_eq!(with_hosts(Some(" corp.internal, Relay.Example "), &hosts), "corp.internal,Relay.Example");
    }

    #[cfg(windows)]
    #[test]
    fn a_terminal_gets_the_proxy_variables_set_now() {
        let prelude = proxy_prelude(&[]);
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
            assert!(prelude.contains(&format!("$env:{name}=")) || prelude.contains(&format!("Env:{name} ")));
        }
    }
}
