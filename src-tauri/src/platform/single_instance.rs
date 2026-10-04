//! Cross-platform process-level single instance guard.
//!
//! This guard is intentionally acquired before Tauri setup starts so a second
//! AgentHarbor process cannot rewrite Codex config.

use crate::platform::paths::CodexPaths;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct ActivationWatcherGuard {
    shutdown: Arc<AtomicBool>,
}

impl Drop for ActivationWatcherGuard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const ERROR_ALREADY_EXISTS: u32 = 183;

    type Handle = *mut c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(
            lp_mutex_attributes: *mut c_void,
            b_initial_owner: i32,
            lp_name: *const u16,
        ) -> Handle;
        fn GetLastError() -> u32;
        fn CloseHandle(h_object: Handle) -> i32;
    }

    pub struct SingleInstanceGuard {
        handle: Handle,
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    let _ = CloseHandle(self.handle);
                }
            }
        }
    }

    pub fn acquire(scope: &str) -> Result<SingleInstanceGuard, String> {
        // Local\\ is per interactive user session and avoids the extra
        // privilege requirements that Global\\ can trigger on locked-down
        // Windows machines.
        let name: Vec<u16> = std::ffi::OsStr::new(&format!("Local\\{scope}.single-instance"))
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe { CreateMutexW(ptr::null_mut(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!("create single-instance mutex failed: {}", unsafe {
                GetLastError()
            }));
        }
        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err("AgentHarbor is already running".into());
        }
        Ok(SingleInstanceGuard { handle })
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }

    pub struct SingleInstanceGuard {
        _file: File,
    }

    pub fn acquire(scope: &str) -> Result<SingleInstanceGuard, String> {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(scope);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("prepare single-instance lock dir failed: {e}"))?;
        let path = dir.join("agentharbor-single-instance.lock");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("open single-instance lock failed ({}): {e}", path.display()))?;
        let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
        if rc != 0 {
            return Err("AgentHarbor is already running".into());
        }
        let _ = file.set_len(0);
        let _ = writeln!(file, "pid={}", std::process::id());
        Ok(SingleInstanceGuard { _file: file })
    }
}

#[cfg(windows)]
pub use imp::SingleInstanceGuard;

#[cfg(unix)]
pub use imp::SingleInstanceGuard;

#[cfg(windows)]
pub fn acquire(_paths: &CodexPaths) -> Result<SingleInstanceGuard, String> {
    imp::acquire(&instance_scope())
}

#[cfg(unix)]
pub fn acquire(_paths: &CodexPaths) -> Result<SingleInstanceGuard, String> {
    imp::acquire(&instance_scope())
}

pub fn activation_baseline() -> Option<Uuid> {
    read_activation_request(&activation_request_path())
}

fn read_activation_request(path: &Path) -> Option<Uuid> {
    let text = std::fs::read_to_string(path).ok()?;
    // Only accept the complete canonical marker, never a partial write.
    let text = text.trim();
    if text.len() != 36 {
        return None;
    }
    Uuid::parse_str(text).ok()
}

pub fn start_activation_watcher<F>(
    baseline: Option<Uuid>,
    on_activate: F,
) -> Result<ActivationWatcherGuard, String>
where
    F: Fn() + Send + 'static,
{
    let request_path = activation_request_path();
    prepare_activation_dir(&request_path)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    std::thread::spawn(move || {
        let mut last_seen = baseline;
        while !thread_shutdown.load(Ordering::Relaxed) {
            if let Some(current) = read_activation_request(&request_path) {
                if last_seen != Some(current) {
                    last_seen = Some(current);
                    on_activate();
                }
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    });

    Ok(ActivationWatcherGuard { shutdown })
}

pub fn request_existing_instance_activation() -> bool {
    write_activation_request(&activation_request_path()).is_ok()
}

fn activation_request_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(instance_scope())
        .join("agentharbor-activate.request")
}

fn instance_scope() -> String {
    if std::env::var_os("AGENT_HARBOR_CODEX_HOME").is_some_and(|value| !value.is_empty()) {
        use sha2::{Digest, Sha256};
        let paths = CodexPaths::new();
        let absolute = std::fs::canonicalize(&paths.codex_home).unwrap_or(paths.codex_home);
        let hash = Sha256::digest(absolute.to_string_lossy().as_bytes());
        format!("dev.agentharbor.desktop.{hash:x}")
    } else {
        "dev.agentharbor.desktop".into()
    }
}

fn prepare_activation_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("prepare activation request dir failed: {e}"))?;
    }
    Ok(())
}

fn write_activation_request(path: &Path) -> Result<(), String> {
    prepare_activation_dir(path)?;
    crate::core::storage::write_atomic(path, Uuid::new_v4().to_string().as_bytes())
        .map_err(|e| format!("write activation request failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_markers_are_complete_and_replace_existing_requests() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("activate.request");
        std::fs::write(&path, "partial").unwrap();
        assert_eq!(read_activation_request(&path), None);
        write_activation_request(&path).unwrap();
        let first = read_activation_request(&path).unwrap();
        write_activation_request(&path).unwrap();
        assert_ne!(read_activation_request(&path).unwrap(), first);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
