//! Starting AgentHarbor when the user signs in, straight into the tray.
use serde::Serialize;
use std::path::Path;

/// What the system passes at sign-in: the window stays in the tray.
pub const ARG: &str = "--autostart";

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub supported: bool,
    pub enabled: bool,
    /// Turned off outside the app: Task Manager's Startup apps on Windows.
    pub blocked: bool,
}

/// Whether this process was started by the system at sign-in.
pub fn launched_at_sign_in() -> bool {
    std::env::args().skip(1).any(|arg| arg == ARG)
}

pub fn status() -> AutostartStatus {
    imp::status()
}

pub fn set(enabled: bool) -> Result<AutostartStatus, String> {
    let exe = std::env::current_exe().map_err(|_| "settings.autostart.errors.write")?;
    imp::set(enabled, &exe)?;
    Ok(status())
}

/// An installation moved or reinstalled elsewhere starts from where it is now.
pub fn refresh() {
    if let Ok(exe) = std::env::current_exe() {
        imp::refresh(&exe);
    }
}

/// The command line the system runs, quoted for paths with spaces.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn command(exe: &Path) -> String {
    format!("\"{}\" {ARG}", exe.display())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::platform::terminal_environment::{
        windows::UserEnvironment, Environment, EnvironmentValue,
    };

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// Where Task Manager keeps a Startup app switched off: an odd first byte.
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const NAME: &str = "AgentHarbor";

    pub fn status() -> AutostartStatus {
        status_at(RUN, APPROVED)
    }

    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        set_at(RUN, APPROVED, enabled, exe)
    }

    pub fn refresh(exe: &Path) {
        refresh_at(RUN, exe);
    }

    fn read(path: &str) -> Option<String> {
        UserEnvironment::open_at(path, false)
            .ok()?
            .read(NAME)
            .ok()
            .flatten()
            .and_then(|value| value.text())
    }

    pub(super) fn status_at(run: &str, approved: &str) -> AutostartStatus {
        let enabled = read(run).is_some_and(|value| !value.trim().is_empty());
        let blocked = enabled
            && UserEnvironment::open_at(approved, false)
                .ok()
                .and_then(|mut key| key.read(NAME).ok().flatten())
                .is_some_and(|value| value.bytes.first().is_some_and(|byte| byte & 1 == 1));
        AutostartStatus {
            supported: true,
            enabled,
            blocked,
        }
    }

    pub(super) fn set_at(run: &str, approved: &str, enabled: bool, exe: &Path) -> Result<(), String> {
        let error = |_| "settings.autostart.errors.write".to_owned();
        let mut key = UserEnvironment::open_at(run, true).map_err(error)?;
        if !enabled {
            return key.write(NAME, None).map_err(error);
        }
        key.write(NAME, Some(&EnvironmentValue::string(&command(exe))))
            .map_err(error)?;
        // Turned on here after Task Manager turned it off: it is on again there.
        if let Ok(mut approved) = UserEnvironment::open_at(approved, true) {
            let _ = approved.write(NAME, None);
        }
        Ok(())
    }

    pub(super) fn refresh_at(run: &str, exe: &Path) {
        let wanted = command(exe);
        if read(run).is_some_and(|value| value != wanted) {
            if let Ok(mut key) = UserEnvironment::open_at(run, true) {
                let _ = key.write(NAME, Some(&EnvironmentValue::string(&wanted)));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::{Foundation::ERROR_SUCCESS, System::Registry::*};

        fn delete(path: &str) {
            let name: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            assert_eq!(unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()) }, ERROR_SUCCESS);
        }

        #[test]
        fn the_run_value_turns_autostart_on_and_task_manager_can_block_it() {
            let root = format!(r"Software\AgentHarbor-tests\{}", uuid::Uuid::new_v4());
            let (run, approved) = (format!(r"{root}\Run"), format!(r"{root}\Approved"));
            let exe = Path::new(r"C:\Program Files\AgentHarbor\AgentHarbor.exe");
            assert_eq!(status_at(&run, &approved), AutostartStatus { supported: true, enabled: false, blocked: false });
            set_at(&run, &approved, true, exe).unwrap();
            assert_eq!(read(&run).unwrap(), r#""C:\Program Files\AgentHarbor\AgentHarbor.exe" --autostart"#);
            assert!(status_at(&run, &approved).enabled);

            // Task Manager switched it off: odd first byte.
            let off = EnvironmentValue { kind: REG_BINARY, bytes: vec![3, 0, 0, 0] };
            UserEnvironment::open_at(&approved, true).unwrap().write(NAME, Some(&off)).unwrap();
            assert!(status_at(&run, &approved).blocked);
            set_at(&run, &approved, true, exe).unwrap();
            assert!(!status_at(&run, &approved).blocked, "turning it on here turns it on there");

            let moved = Path::new(r"D:\Apps\AgentHarbor\AgentHarbor.exe");
            refresh_at(&run, moved);
            assert_eq!(read(&run).unwrap(), command(moved));
            set_at(&run, &approved, false, exe).unwrap();
            assert!(!status_at(&run, &approved).enabled);
            refresh_at(&run, exe);
            assert!(read(&run).is_none(), "a refresh never turns it on");
            delete(&root);
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    const LABEL: &str = "dev.agentharbor.desktop.autostart";

    fn plist() -> Option<PathBuf> {
        Some(dirs::home_dir()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn contents(exe: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>{ARG}</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n",
            escape(&exe.display().to_string())
        )
    }

    pub fn status() -> AutostartStatus {
        AutostartStatus {
            supported: true,
            enabled: plist().is_some_and(|path| path.is_file()),
            blocked: false,
        }
    }

    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        let path = plist().ok_or("settings.autostart.errors.write")?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err("settings.autostart.errors.write".into())
                }
                _ => Ok(()),
            };
        }
        crate::core::storage::write_atomic(&path, contents(exe).as_bytes())
            .map_err(|_| "settings.autostart.errors.write".into())
    }

    pub fn refresh(exe: &Path) {
        if let Some(path) = plist().filter(|path| path.is_file()) {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(contents(exe).as_str()) {
                let _ = crate::core::storage::write_atomic(&path, contents(exe).as_bytes());
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn entry() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| Some(dirs::home_dir()?.join(".config")))?;
        Some(config.join("autostart/agentharbor.desktop"))
    }

    fn contents(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=AgentHarbor\nExec={}\nX-GNOME-Autostart-enabled=true\n",
            command(exe)
        )
    }

    pub fn status() -> AutostartStatus {
        AutostartStatus {
            supported: true,
            enabled: entry().is_some_and(|path| path.is_file()),
            blocked: false,
        }
    }

    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        let path = entry().ok_or("settings.autostart.errors.write")?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err("settings.autostart.errors.write".into())
                }
                _ => Ok(()),
            };
        }
        crate::core::storage::write_atomic(&path, contents(exe).as_bytes())
            .map_err(|_| "settings.autostart.errors.write".into())
    }

    pub fn refresh(exe: &Path) {
        if let Some(path) = entry().filter(|path| path.is_file()) {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(contents(exe).as_str()) {
                let _ = crate::core::storage::write_atomic(&path, contents(exe).as_bytes());
            }
        }
    }
}
