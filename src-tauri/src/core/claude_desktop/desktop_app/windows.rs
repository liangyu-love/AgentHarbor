//! Native process operations: no tasklist/taskkill subprocesses or PID-only kills.
use super::*;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, HANDLE, HWND,
    INVALID_HANDLE_VALUE, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindow, PostMessageW, WM_CLOSE,
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn win32_error(operation: &str, code: u32) -> Box<DesktopError> {
    Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.process")
        .param(format!("{operation}: Win32 {code}")).retryable())
}

pub(super) fn candidates() -> DesktopResult<Vec<u32>> {
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(win32_error("CreateToolhelp32Snapshot", GetLastError()));
        }
        let snapshot = Handle(raw);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut next = Process32FirstW(snapshot.0, &mut entry);
        // Capture immediately after the failed Win32 call; any later Win32
        // operation may overwrite the thread's last-error value.
        let mut enumeration_error = if next == 0 { GetLastError() } else { 0 };
        let mut result = Vec::new();
        while next != 0 {
            let name = String::from_utf16_lossy(
                &entry.szExeFile[..entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len())],
            );
            if entry.th32ProcessID != 0 && name.eq_ignore_ascii_case("Claude.exe") {
                result.push(entry.th32ProcessID);
            }
            next = Process32NextW(snapshot.0, &mut entry);
            // Keep GetLastError adjacent to the enumeration call above.
            if next == 0 { enumeration_error = GetLastError(); }
        }
        // A truncated/failed enumeration must never authorize a config write.
        if enumeration_error != ERROR_NO_MORE_FILES {
            return Err(win32_error("Process32First/Next", enumeration_error));
        }
        Ok(result)
    }
}

fn open(pid: u32, force: bool) -> DesktopResult<Option<Handle>> {
    if pid == 0 {
        return Err(process_error(error_code::PROCESS));
    }
    // SYNCHRONIZE lets us distinguish a terminated process from access failure.
    let access =
        PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000 | if force { PROCESS_TERMINATE } else { 0 };
    let raw = unsafe { OpenProcess(access, 0, pid) };
    if raw.is_null() {
        let error = unsafe { GetLastError() };
        return if error == ERROR_INVALID_PARAMETER {
            Ok(None)
        } else {
            Err(win32_error("OpenProcess", error))
        };
    }
    Ok(Some(Handle(raw)))
}

fn exited(handle: &Handle) -> DesktopResult<bool> {
    match unsafe { WaitForSingleObject(handle.0, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(win32_error("WaitForSingleObject", unsafe { GetLastError() })),
    }
}

fn image_for(handle: &Handle) -> ProcessImage {
    match exited(handle) {
        Ok(true) => return ProcessImage::Exited,
        Err(_) => return ProcessImage::Unknown,
        _ => {}
    }
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut length) };
    if ok != 0 && length > 0 {
        ProcessImage::Running(PathBuf::from(String::from_utf16_lossy(
            &buffer[..length as usize],
        )))
    } else if matches!(exited(handle), Ok(true)) {
        ProcessImage::Exited
    } else {
        ProcessImage::Unknown
    }
}

pub(super) fn image(pid: u32) -> ProcessImage {
    match open(pid, false) {
        Ok(Some(handle)) => image_for(&handle),
        Ok(None) => ProcessImage::Exited,
        Err(_) => ProcessImage::Unknown,
    }
}

struct CloseWindows {
    pid: u32,
    requested: bool,
    failed: Option<u32>,
}

unsafe extern "system" fn close_window(window: HWND, data: LPARAM) -> i32 {
    let state = &mut *(data as *mut CloseWindows);
    let mut pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    if pid == state.pid {
        // PostMessage is nonblocking even when the Desktop UI is hung.
        if PostMessageW(window, WM_CLOSE, 0, 0) != 0 {
            state.requested = true;
        } else {
            // IsWindow can change last-error; capture the posting failure first.
            let error = GetLastError();
            if IsWindow(window) != 0 { state.failed = Some(error); }
        }
    }
    1
}

pub(super) fn signal(identity: &AppIdentity, pid: u32, force: bool) -> DesktopResult<bool> {
    let Some(handle) = open(pid, force)? else {
        return Ok(false);
    };
    match image_for(&handle) {
        ProcessImage::Exited => return Ok(false),
        ProcessImage::Running(path) if executable_matches(&path, &identity.executable) => {}
        _ => return Err(process_error(error_code::PROCESS)),
    }
    // Keep this exact process handle alive through the action. Never terminate
    // descendants (Desktop can have Claude Code tools running beneath it).
    if force {
        if unsafe { TerminateProcess(handle.0, 1) } != 0 {
            return Ok(true);
        }
        let error = unsafe { GetLastError() };
        if exited(&handle)? {
            return Ok(false);
        }
        return Err(win32_error("TerminateProcess", error));
    }
    let mut state = CloseWindows {
        pid,
        requested: false,
        failed: None,
    };
    let ok = unsafe { EnumWindows(Some(close_window), &mut state as *mut _ as LPARAM) };
    if ok == 0 {
        return Err(win32_error("EnumWindows", unsafe { GetLastError() }));
    }
    if let Some(error) = state.failed {
        return Err(win32_error("PostMessageW", error));
    }
    Ok(state.requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Stdio};

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn fixture(window: bool) -> (tempfile::TempDir, ChildGuard, AppIdentity) {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let exe = std::env::current_exe().unwrap();
        let mut command = crate::platform::windows::background_command_path(&exe);
        command
            .args([
                "--exact",
                "core::claude_desktop::desktop_app::windows::tests::child_process_fixture",
                "--ignored",
            ])
            .env("DESKTOP_TEST_READY", &ready)
            .env("DESKTOP_TEST_WINDOW", if window { "1" } else { "0" })
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ChildGuard(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "fixture exited before ready"
            );
            assert!(Instant::now() < deadline, "fixture startup timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let identity = AppIdentity {
            method: LaunchMethod::Executable,
            target: exe.clone(),
            executable: exe,
        };
        (temp, child, identity)
    }

    #[test]
    #[ignore = "Child process fixture; only parent tests launch it, no real Desktop/config access"]
    fn child_process_fixture() {
        let ready = std::env::var_os("DESKTOP_TEST_READY").expect("parent fixture path");
        if std::env::var("DESKTOP_TEST_WINDOW").as_deref() == Ok("1") {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetMessageW, MSG,
            };
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            unsafe {
                // Hidden, test-owned top-level window; never show native UI.
                let window = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    10,
                    10,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!window.is_null());
                std::fs::write(ready, "ready").unwrap();
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    if message.message == WM_CLOSE {
                        break;
                    }
                }
                DestroyWindow(window);
            }
        } else {
            std::fs::write(ready, "ready").unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn graceful_close_targets_only_the_test_window_and_does_not_block() {
        let (_temp, mut child, identity) = fixture(true);
        let started = Instant::now();
        assert!(signal(&identity, child.0.id(), false).unwrap());
        assert!(started.elapsed() < Duration::from_secs(2));
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "WM_CLOSE was not delivered");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(image(child.0.id()), ProcessImage::Exited));
    }

    #[test]
    fn graceful_stop_waits_for_the_test_owned_window_process_to_exit() {
        let (_temp, child, identity) = fixture(true);
        let pid = child.0.id();
        let started = Instant::now();
        assert!(stop_processes(Duration::from_secs(2), || {
            checked_processes(Some(&identity), Ok(vec![pid]), image)
        }, |pid, force| {
            assert!(!force, "the test window should complete its graceful exit");
            signal(&identity, pid, force)
        }).unwrap());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(image(pid), ProcessImage::Exited));
    }

    #[test]
    fn headless_signal_reports_no_window_and_force_checks_executable_identity() {
        let (temp, mut child, identity) = fixture(false);
        assert!(!signal(&identity, child.0.id(), false).unwrap());
        let unrelated = AppIdentity {
            method: LaunchMethod::Executable,
            target: temp.path().join("unrelated.exe"),
            executable: temp.path().join("unrelated.exe"),
        };
        assert!(signal(&unrelated, child.0.id(), true).is_err());
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "wrong identity must not kill"
        );
        assert!(signal(&identity, child.0.id(), true).unwrap());
        child.0.wait().unwrap();
        assert!(matches!(image(child.0.id()), ProcessImage::Exited));
        assert!(!signal(&identity, child.0.id(), true).unwrap());
        assert!(matches!(image(0), ProcessImage::Unknown));
    }

    #[test]
    fn native_enumeration_and_cached_stop_gate_are_fast_and_read_only() {
        let _env = super::super::super::env_test_lock();
        let root = dirs::data_local_dir().unwrap();
        let paths = DesktopPaths::from_roots(root.join("Claude"), root.join("Claude-3p")).unwrap();
        let resolve = Instant::now();
        let gate = StopCheck::new(&paths);
        let discovery_ms = resolve.elapsed().as_millis();
        let started = Instant::now();
        for _ in 0..10 {
            candidates().unwrap();
            match gate.ensure_stopped() {
                Ok(()) => {}
                Err(error) => assert_eq!(error.message, "claudeDesktop.errors.desktopRunning"),
            }
        }
        println!(
            "Desktop timing: discovery={discovery_ms}ms; ten native enumerations+write gates={}ms",
            started.elapsed().as_millis()
        );
    }
}
