use crate::core::api_client::{configure_codex_desktop, configure_command_network};
use crate::core::models::{ApiProxyConfigPayload, CoreError};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::sync::Mutex;

#[cfg(target_os = "windows")]
static LAST_KNOWN_CODEX_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(target_os = "macos")]
pub fn kill_process(name: &str) -> Result<(), CoreError> {
    std::process::Command::new("killall")
        .args(["-9", name])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("killall failed: {e}")))?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn kill_process(name: &str) -> Result<(), CoreError> {
    if name.eq_ignore_ascii_case("Codex") {
        return signal_windows_codex_desktop(true);
    }
    crate::platform::windows::background_command("taskkill")
        .args(["/F", "/IM", &format!("{name}.exe")])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("taskkill failed: {e}")))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn kill_process(name: &str) -> Result<(), CoreError> {
    std::process::Command::new("pkill")
        .args(["-9", name])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("pkill failed: {e}")))?;
    Ok(())
}

fn wait_for_process_exit(name: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_process_running(name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(target_os = "macos")]
fn wait_for_process_start(name: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_process_running(name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    false
}

#[cfg(target_os = "macos")]
fn is_process_running(name: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn is_process_running(name: &str) -> bool {
    if name.eq_ignore_ascii_case("Codex") {
        !windows_codex_desktop_processes().is_empty()
    } else {
        !win32_process::find_processes_by_name(name).is_empty()
    }
}

#[cfg(target_os = "windows")]
mod win32_process {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    pub fn has_visible_window(pid: u32) -> bool {
        unsafe extern "system" fn check_window(
            window: windows_sys::Win32::Foundation::HWND,
            data: isize,
        ) -> i32 {
            let state = &mut *(data as *mut (u32, bool));
            let mut owner = 0;
            GetWindowThreadProcessId(window, &mut owner);
            if owner == state.0 && IsWindowVisible(window) != 0 {
                state.1 = true;
                return 0;
            }
            1
        }

        let mut state = (pid, false);
        unsafe {
            EnumWindows(Some(check_window), &mut state as *mut _ as isize);
        }
        state.1
    }

    pub struct Process {
        pub pid: u32,
        pub path: PathBuf,
    }

    pub fn find_processes_by_name(name: &str) -> Vec<Process> {
        let mut processes = Vec::new();
        let target_lower: Vec<u16> = format!("{name}.exe")
            .to_ascii_lowercase()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return processes;
            }

            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snapshot, &mut entry) == 0 {
                CloseHandle(snapshot);
                return processes;
            }

            loop {
                let exe_name_lower: Vec<u16> = entry
                    .szExeFile
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| {
                        if (b'A' as u16..=b'Z' as u16).contains(&c) {
                            c + 32
                        } else {
                            c
                        }
                    })
                    .chain(std::iter::once(0))
                    .collect();

                if exe_name_lower == target_lower {
                    let pid = entry.th32ProcessID;
                    if let Some(path) = get_process_exe_path(pid) {
                        processes.push(Process { pid, path });
                    }
                }

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }

            CloseHandle(snapshot);
        }
        processes
    }

    fn get_process_exe_path(pid: u32) -> Option<PathBuf> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }

            let mut buf = [0u16; 32768];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(handle);

            if ok == 0 || size == 0 {
                return None;
            }

            let path_str = String::from_utf16_lossy(&buf[..size as usize]);
            let p = PathBuf::from(path_str);
            if p.exists() {
                Some(p)
            } else {
                None
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn is_process_running(name: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn codex_process_name() -> &'static str {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        "Codex"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "codex"
    }
}

pub fn is_codex_app_running() -> bool {
    is_process_running(codex_process_name())
}

pub fn stop_codex_app_gracefully(timeout: Duration) -> Result<(), CoreError> {
    if !is_codex_app_running() {
        return Ok(());
    }

    // 优雅退出：忽略信号发送本身的错误（部分 Windows 版本/权限下即使成功
    // 发出信号也可能返回非零码），只用进程是否消失来判断结果。
    let _ = request_codex_app_quit();
    if wait_for_process_exit(codex_process_name(), timeout) {
        return Ok(());
    }

    // 优雅退出超时（Windows 上可能弹对话框或任务未完成）；
    // 降级到强制 kill，再等一小段时间，避免直接报错卡死切换流程。
    let _ = kill_process(codex_process_name());
    if wait_for_process_exit(codex_process_name(), Duration::from_secs(5)) {
        return Ok(());
    }

    Err(CoreError::OperationFailed(
        "CODEX_APP_QUIT_TIMEOUT: Codex did not quit in time; please quit Codex manually and try again".to_string(),
    ))
}

pub fn ensure_no_codex_writer_processes() -> Result<(), CoreError> {
    // Windows 上强制 kill Codex.exe 后，Electron 的 Helper / GPU / Renderer
    // 子进程需要几百毫秒才能完全退出；立即检查会误判"仍在运行"。
    // 这里最多重试 3 次（每次间隔 500ms），覆盖绝大多数情况。
    #[cfg(target_os = "windows")]
    {
        for attempt in 0..3u32 {
            let writers = list_codex_writer_processes()?;
            if writers.is_empty() {
                return Ok(());
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_millis(500));
            } else {
                return Err(CoreError::OperationFailed(format!(
                    "CODEX_WRITER_RUNNING: 检测到仍有 Codex 相关进程可能正在写入线程数据。请完全退出 Codex 和 codex CLI 后再试：{}",
                    writers.join(" | ")
                )));
            }
        }
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let writers = list_codex_writer_processes()?;
        if writers.is_empty() {
            return Ok(());
        }
        Err(CoreError::OperationFailed(format!(
            "CODEX_WRITER_RUNNING: 检测到仍有 Codex 相关进程可能正在写入线程数据。请完全退出 Codex 和 codex CLI 后再试：{}",
            writers.join(" | ")
        )))
    }
}

#[cfg(target_os = "macos")]
fn list_codex_writer_processes() -> Result<Vec<String>, CoreError> {
    let output = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,command="])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("ps failed: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::OperationFailed(format!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let current_pid = std::process::id();
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, command) = trimmed.split_once(char::is_whitespace)?;
            let pid = pid.parse::<u32>().ok()?;
            let command = command.trim_start();
            if pid == current_pid || !is_codex_writer_command(command) {
                return None;
            }
            Some(format!("{pid} {command}"))
        })
        .collect())
}

#[cfg(not(target_os = "windows"))]
fn is_codex_writer_command(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    // crashpad_handler 是 Electron 的崩溃报告子进程，主进程退出后可能残留数秒，
    // 不应被视为"Codex 仍在运行"。
    if command.contains("crashpad_handler") {
        return false;
    }
    command.contains("codex.app/")
        || command.contains("codex helper")
        || command.contains("chatgpt.app/")
        || command.contains("chatgpt helper")
        || command.contains("codex app-server")
        || (command.contains("/extensions/") && command.contains("codex"))
        || command == "codex"
        || command.starts_with("codex ")
        || command.ends_with("/codex")
        || command.contains("/codex ")
}

#[cfg(target_os = "windows")]
fn is_windows_codex_writer_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "codex.exe"
            | "codex-cli.exe"
            | "codex-app-server.exe"
            | "codexappserver.exe"
            | "chatgpt.exe"
            | "openai.chatgpt.exe"
    ) || (name.starts_with("codex-") && name.ends_with(".exe"))
        || (name.starts_with("chatgpt-") && name.ends_with(".exe"))
}

#[cfg(target_os = "windows")]
fn list_codex_writer_processes() -> Result<Vec<String>, CoreError> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let current_pid = std::process::id();
    let mut result = Vec::new();

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(CoreError::OperationFailed(format!(
                "CreateToolhelp32Snapshot failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) == 0 {
            let error = std::io::Error::last_os_error();
            CloseHandle(snapshot);
            return Err(CoreError::OperationFailed(format!(
                "Process32FirstW failed: {error}"
            )));
        }

        loop {
            let exe_name: String = entry
                .szExeFile
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8 as char)
                .collect();
            if entry.th32ProcessID != current_pid && is_windows_codex_writer_name(&exe_name) {
                result.push(format!("{} {}", entry.th32ProcessID, exe_name));
            }

            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }

        CloseHandle(snapshot);
    }
    Ok(result)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn list_codex_writer_processes() -> Result<Vec<String>, CoreError> {
    let output = std::process::Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("ps failed: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::OperationFailed(format!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let current_pid = std::process::id();
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, command) = trimmed.split_once(char::is_whitespace)?;
            let pid = pid.parse::<u32>().ok()?;
            let command = command.trim_start();
            if pid == current_pid || !is_codex_writer_command(command) {
                return None;
            }
            Some(format!("{pid} {command}"))
        })
        .collect())
}

#[cfg(test)]
mod writer_detection_tests {
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_writer_names_include_desktop_cli_app_server_and_chatgpt() {
        for name in [
            "Codex.exe",
            "codex-cli.exe",
            "codex-app-server.exe",
            "ChatGPT.exe",
        ] {
            assert!(super::is_windows_codex_writer_name(name), "{name}");
        }
        assert!(!super::is_windows_codex_writer_name("AgentHarbor.exe"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unix_writer_commands_include_ide_app_server_and_chatgpt() {
        assert!(super::is_codex_writer_command(
            "/home/u/.vscode/extensions/openai.chatgpt/bin/codex app-server"
        ));
        assert!(super::is_codex_writer_command(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
        assert!(!super::is_codex_writer_command("crashpad_handler codex"));
    }
}

#[cfg(target_os = "macos")]
fn request_codex_app_quit() -> Result<(), CoreError> {
    let output = std::process::Command::new("osascript")
        .args(["-e", r#"tell application "Codex" to quit"#])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("osascript quit Codex failed: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::OperationFailed(format!(
            "osascript quit Codex failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn request_codex_app_quit() -> Result<(), CoreError> {
    signal_windows_codex_desktop(false)
}

#[cfg(target_os = "windows")]
fn signal_windows_codex_desktop(force: bool) -> Result<(), CoreError> {
    // Only signal verified desktop PIDs: MSIX uses ChatGPT.exe, while older
    // unpackaged builds use Codex.exe (also used by the CLI).
    for process in windows_codex_desktop_processes() {
        let mut command = crate::platform::windows::background_command("taskkill");
        command.args(["/PID", &process.pid.to_string()]);
        if force {
            command.args(["/F", "/T"]);
        }
        command.output().map_err(|error| {
            CoreError::OperationFailed(format!("taskkill Codex desktop failed: {error}"))
        })?;
    }
    // Callers wait for the selected desktop processes, so an already-exited
    // child or a nonzero taskkill status does not incorrectly fail the stop.
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn request_codex_app_quit() -> Result<(), CoreError> {
    let output = std::process::Command::new("pkill")
        .args(["-TERM", "codex"])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("pkill Codex failed: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::OperationFailed(format!(
            "pkill Codex failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn open_terminal_with_command(command: &str) -> Result<(), CoreError> {
    let script = format!(
        r#"tell application "Terminal"
    activate
    do script "{}"
end tell"#,
        command.replace('\\', "\\\\").replace('"', "\\\"")
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("osascript failed: {e}")))?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn open_terminal_with_command(command: &str) -> Result<(), CoreError> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "cmd", "/k", command])
        .output()
        .map_err(|e| CoreError::OperationFailed(format!("cmd start failed: {e}")))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn open_terminal_with_command(command: &str) -> Result<(), CoreError> {
    for terminal in &["gnome-terminal", "xterm", "konsole"] {
        if std::process::Command::new("which")
            .arg(terminal)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let _ = std::process::Command::new(terminal)
                .args(["--", "sh", "-c", command])
                .spawn();
            return Ok(());
        }
    }
    Err(CoreError::OperationFailed(
        "No terminal emulator found".to_string(),
    ))
}

#[cfg(target_os = "macos")]
pub fn launch_codex_app(proxy: &ApiProxyConfigPayload) -> Result<(), CoreError> {
    let launch_timeout = Duration::from_secs(3);

    let bundle_ids = ["com.openai.codex", "com.openai.Codex"];
    for _ in 0..2 {
        for bid in &bundle_ids {
            let mut command = std::process::Command::new("open");
            command.args(["-b", bid]);
            configure_macos_open(&mut command, proxy)?;
            let result = command.output();
            if let Ok(o) = result {
                if o.status.success() && wait_for_process_start("Codex", launch_timeout) {
                    return Ok(());
                }
            }
        }

        let app_paths = [
            "/Applications/Codex.app",
            &format!(
                "{}/Applications/Codex.app",
                dirs::home_dir().unwrap_or_default().display()
            ),
        ];
        for path in &app_paths {
            if std::path::Path::new(path).exists() {
                let mut command = std::process::Command::new("open");
                command.arg(path);
                configure_macos_open(&mut command, proxy)?;
                command
                    .output()
                    .map_err(|e| CoreError::OperationFailed(format!("open app failed: {e}")))?;
                if wait_for_process_start("Codex", launch_timeout) {
                    return Ok(());
                }
            }
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    Err(CoreError::OperationFailed(
        "Codex launch timed out".to_string(),
    ))
}

#[cfg(target_os = "macos")]
pub fn restart_codex_app(
    proxy: &ApiProxyConfigPayload,
    paths: &crate::platform::paths::CodexPaths,
) -> Result<(), CoreError> {
    stop_codex_app_gracefully(Duration::from_secs(8))?;
    crate::core::codex_config::prepare_network_config(paths, proxy)?;
    launch_codex_app(proxy)
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    fn fixture_executable(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture only; never execute").unwrap();
        path
    }

    #[test]
    fn windows_desktop_candidates_reject_npm_and_bundled_cli() {
        let temp = tempfile::tempdir().unwrap();
        let npm = fixture_executable(temp.path(), "node_modules/codex/bin/codex.exe");
        let bundled = fixture_executable(temp.path(), "Codex/resources/codex.exe");
        let desktop = fixture_executable(temp.path(), "Codex/Codex.exe");
        fixture_executable(temp.path(), "Codex/resources/app.asar");

        assert!(!is_windows_codex_desktop_path(&npm));
        assert!(!is_windows_codex_desktop_path(&bundled));
        assert!(is_windows_codex_desktop_path(&desktop));
        let candidates = [npm, bundled, desktop.clone()];
        assert_eq!(
            candidates
                .into_iter()
                .find(|path| is_windows_codex_desktop_path(path)),
            Some(desktop)
        );
    }

    #[test]
    fn windows_desktop_candidate_requires_both_executable_and_asar() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("Codex.exe");
        fixture_executable(temp.path(), "resources/app.asar");
        assert!(!is_windows_codex_desktop_path(&executable));
        fixture_executable(temp.path(), "Codex.exe");
        assert!(is_windows_codex_desktop_path(&executable));
        let other = fixture_executable(temp.path(), "Other.exe");
        assert!(!is_windows_codex_desktop_path(&other));
        std::fs::remove_file(temp.path().join("resources/app.asar")).unwrap();
        assert!(!is_windows_codex_desktop_path(&executable));
    }

    #[test]
    fn windows_msix_main_is_chatgpt_but_unrelated_chatgpt_is_not_codex() {
        let temp = tempfile::tempdir().unwrap();
        let packaged = temp.path().join("OpenAI.Codex_26.917.9434.0_x64__test");
        let main = fixture_executable(&packaged, "app/ChatGPT.exe");
        fixture_executable(&packaged, "app/resources/app.asar");
        assert!(!is_windows_codex_desktop_path(&main));
        fixture_executable(&packaged, "AppxManifest.xml");
        assert!(is_windows_codex_desktop_path(&main));
        let unrelated = fixture_executable(temp.path(), "ChatGPT/app/ChatGPT.exe");
        fixture_executable(temp.path(), "ChatGPT/app/resources/app.asar");
        fixture_executable(temp.path(), "ChatGPT/AppxManifest.xml");
        assert!(!is_windows_codex_desktop_path(&unrelated));
    }

    #[test]
    fn windows_appx_candidates_preserve_package_order_and_spaces() {
        let output = "\u{feff}C:\\Program Files\\WindowsApps\\OpenAI.Codex_new\r\n\r\n C:\\Program Files\\WindowsApps\\OpenAI.Codex_old \r\n";
        let candidates = windows_appx_codex_candidates(output);
        let latest = PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_new");
        let older = PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_old");
        assert_eq!(
            candidates,
            vec![
                latest.join("app").join("ChatGPT.exe"),
                latest.join("app").join("Codex.exe"),
                latest.join("Codex.exe"),
                older.join("app").join("ChatGPT.exe"),
                older.join("app").join("Codex.exe"),
                older.join("Codex.exe")
            ]
        );
        assert!(windows_appx_codex_candidates("\r\n  \n").is_empty());
    }

    #[test]
    fn windows_executable_comparison_handles_case_slashes_and_extended_prefix() {
        assert!(same_windows_executable(
            Path::new(r"C:\Codex Fixture\Codex.exe"),
            Path::new(r"\\?\c:\codex fixture\CODEX.EXE")
        ));
        assert!(same_windows_executable(
            Path::new("C:/Codex Fixture/Codex.exe"),
            Path::new(r"c:\codex fixture\codex.exe")
        ));
        assert!(!same_windows_executable(
            Path::new(r"C:\Codex Fixture\Codex.exe"),
            Path::new(r"C:\Codex Fixture\resources\codex.exe")
        ));
    }
}

#[cfg(target_os = "macos")]
fn configure_macos_open(
    command: &mut std::process::Command,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    let mut child = std::process::Command::new("Codex");
    configure_codex_desktop(&mut child, proxy)?;
    // LaunchServices does not inherit `open`'s environment automatically.
    for (key, value) in child.get_envs() {
        command.arg("--env").arg(format!(
            "{}={}",
            key.to_string_lossy(),
            value.unwrap_or_default().to_string_lossy()
        ));
    }
    command.arg("--args").args(child.get_args());
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn launch_codex_app(proxy: &ApiProxyConfigPayload) -> Result<(), CoreError> {
    launch_windows_codex_app(proxy)
}

#[cfg(target_os = "windows")]
pub fn restart_codex_app(
    proxy: &ApiProxyConfigPayload,
    paths: &crate::platform::paths::CodexPaths,
) -> Result<(), CoreError> {
    snapshot_running_codex_path();
    stop_codex_app_gracefully(Duration::from_secs(8))?;
    crate::core::codex_config::prepare_network_config(paths, proxy)?;
    launch_windows_codex_app(proxy)
}

#[cfg(target_os = "windows")]
pub fn snapshot_codex_path_before_stop() {
    snapshot_running_codex_path();
}

/// Windows 路由 toggle / config 重写前的快路径：
/// - 桌面端与 CLI 均未运行时直接返回，避免 PowerShell writer probe 的开销
///   （AgentHarbor 用户经常在打开 Codex 之前就先在面板里切换路由）
/// - Codex 在跑时按完整流程：snapshot 路径 + graceful stop +
///   ensure 写 ~/.codex 的进程都退出
///
/// 调用方（manager.rs::set_codex_router_enabled）已经把
/// snapshot_codex_path_before_stop 与 stop_codex_app_gracefully +
/// ensure_no_codex_writer_processes 三步在 Windows 上合并到这一个入口，
/// 避免每次 toggle 都付几百 ms 启动开销。
#[cfg(target_os = "windows")]
pub fn stop_codex_for_config_edit(timeout: Duration) -> Result<(), CoreError> {
    if !is_codex_app_running() {
        // A standalone CLI must never be stopped as though it were Electron,
        // but it can still be a writer that makes a config edit unsafe.
        return if win32_process::find_processes_by_name("Codex").is_empty() {
            Ok(())
        } else {
            ensure_no_codex_writer_processes()
        };
    }
    snapshot_running_codex_path();
    // Codex is running (checked above); skip the redundant is_codex_app_running()
    // inside stop_codex_app_gracefully by inlining the stop logic directly.
    let _ = request_codex_app_quit();
    if !wait_for_process_exit(codex_process_name(), timeout) {
        let _ = kill_process(codex_process_name());
        if !wait_for_process_exit(codex_process_name(), Duration::from_secs(5)) {
            return Err(CoreError::OperationFailed(
                "CODEX_APP_QUIT_TIMEOUT: Codex did not quit in time; please quit Codex manually and try again".to_string(),
            ));
        }
    }
    ensure_no_codex_writer_processes()
}

#[cfg(target_os = "windows")]
fn snapshot_running_codex_path() {
    let path = windows_codex_desktop_processes()
        .into_iter()
        .next()
        .map(|process| process.path);
    if let Some(ref p) = path {
        log::info!("[AgentHarbor] captured running Codex path: {}", p.display());
    }
    if let Ok(mut cached) = LAST_KNOWN_CODEX_PATH.lock() {
        *cached = path;
    }
}

#[cfg(target_os = "windows")]
fn windows_codex_desktop_processes() -> Vec<win32_process::Process> {
    win32_process::find_processes_by_name("ChatGPT")
        .into_iter()
        .chain(win32_process::find_processes_by_name("Codex"))
        .filter(|process| is_windows_codex_desktop_path(&process.path))
        .collect()
}

#[cfg(target_os = "windows")]
fn is_windows_codex_desktop_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let packaged_main = name.eq_ignore_ascii_case("ChatGPT.exe")
        && path.parent().is_some_and(|parent| {
            parent
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("app"))
        })
        && path.parent().and_then(Path::parent).is_some_and(|root| {
            root.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("OpenAI.Codex_"))
                && root.join("AppxManifest.xml").is_file()
        });
    let unpackaged_main = name.eq_ignore_ascii_case("Codex.exe");
    (packaged_main || unpackaged_main)
        && path.is_file()
        && path
            .parent()
            .is_some_and(|parent| parent.join("resources").join("app.asar").is_file())
}

#[cfg(target_os = "windows")]
fn is_windows_packaged_codex_main(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ChatGPT.exe"))
}

#[cfg(target_os = "windows")]
fn same_windows_executable(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
    };
    normalize(left) == normalize(right)
}

#[cfg(target_os = "windows")]
fn wait_for_windows_codex_start(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if windows_codex_desktop_processes().iter().any(|process| {
            same_windows_executable(&process.path, path)
                && (!is_windows_packaged_codex_main(path)
                    || win32_process::has_visible_window(process.pid))
        }) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    false
}

#[cfg(target_os = "windows")]
pub(crate) fn launch_windows_codex_app(proxy: &ApiProxyConfigPayload) -> Result<(), CoreError> {
    let cached = LAST_KNOWN_CODEX_PATH
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|path| is_windows_codex_desktop_path(path));
    let path = cached
        .or_else(find_windows_codex_exe)
        .ok_or_else(|| CoreError::NotFound("Codex.exe not found".to_string()))?;
    launch_windows_codex_from_path(&path, proxy)
}

#[cfg(target_os = "windows")]
fn launch_windows_codex_from_path(
    path: &Path,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    if !is_windows_codex_desktop_path(path) {
        return Err(CoreError::InvalidData(
            "Codex desktop executable is required; refusing to launch a CLI as Electron".into(),
        ));
    }
    let launch_timeout = if is_windows_packaged_codex_main(path) {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(5)
    };

    let mut direct = crate::platform::windows::background_command_path(path);
    configure_codex_desktop(&mut direct, proxy)?;
    if let Some(parent) = path.parent() {
        direct.current_dir(parent);
    }
    if direct.spawn().is_ok() {
        if wait_for_windows_codex_start(path, launch_timeout) {
            return Ok(());
        }
        if windows_codex_desktop_processes()
            .iter()
            .any(|process| same_windows_executable(&process.path, path))
        {
            return Err(CoreError::OperationFailed(
                "Codex desktop process started but no window appeared".into(),
            ));
        }
    }

    let path_literal = windows_powershell_literal(path.to_string_lossy().as_ref());
    let arguments = direct
        .get_args()
        .map(|arg| format!("'{}'", windows_powershell_literal(&arg.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(",");
    let argument_list = if arguments.is_empty() {
        String::new()
    } else {
        format!(" -ArgumentList @({arguments})")
    };
    let mut fallback = crate::platform::windows::background_command("powershell");
    configure_command_network(&mut fallback, proxy)?;
    fallback
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &format!("Start-Process -FilePath '{path_literal}'{argument_list}"),
        ])
        .spawn()
        .map_err(|e| CoreError::OperationFailed(format!("launch failed: {e}")))?;

    if wait_for_windows_codex_start(path, launch_timeout) {
        return Ok(());
    }

    Err(CoreError::OperationFailed(format!(
        "Codex did not start after launch: {}",
        path.display()
    )))
}

#[cfg(target_os = "windows")]
fn find_windows_codex_exe() -> Option<PathBuf> {
    for candidate in windows_codex_candidates() {
        if is_windows_codex_desktop_path(&candidate) {
            return Some(candidate);
        }
    }

    if let Some(p) = find_codex_from_registry() {
        return Some(p);
    }
    if let Some(path) = find_codex_from_appx() {
        return Some(path);
    }

    for name in ["Codex.exe", "Codex"] {
        let output = crate::platform::windows::background_command("where")
            .arg(name)
            .output()
            .ok();
        if let Some(output) = output.filter(|output| output.status.success()) {
            if let Some(path) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .find(|path| is_windows_codex_desktop_path(path))
            {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_codex_from_appx() -> Option<PathBuf> {
    let output = crate::platform::windows::background_command("powershell")
        .args(["-NoProfile", "-Command",
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Get-AppxPackage -Name OpenAI.Codex -ErrorAction SilentlyContinue | Sort-Object Version -Descending | Select-Object -ExpandProperty InstallLocation"])
        .output().ok()?;
    if !output.status.success() {
        return None;
    }
    windows_appx_codex_candidates(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .find(|path| is_windows_codex_desktop_path(path))
}

#[cfg(target_os = "windows")]
fn windows_appx_codex_candidates(output: &str) -> Vec<PathBuf> {
    output
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .flat_map(|location| {
            let root = PathBuf::from(location);
            [
                root.join("app").join("ChatGPT.exe"),
                root.join("app").join("Codex.exe"),
                root.join("Codex.exe"),
            ]
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_codex_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let env_candidates = [
        ("LOCALAPPDATA", "Programs\\Codex\\Codex.exe"),
        ("LOCALAPPDATA", "Codex\\Codex.exe"),
        ("LOCALAPPDATA", "Programs\\OpenAI Codex\\Codex.exe"),
        ("LOCALAPPDATA", "Programs\\OpenAI\\Codex\\Codex.exe"),
        ("PROGRAMFILES", "Codex\\Codex.exe"),
        ("PROGRAMFILES", "OpenAI Codex\\Codex.exe"),
        ("PROGRAMFILES", "OpenAI\\Codex\\Codex.exe"),
        ("PROGRAMFILES(X86)", "Codex\\Codex.exe"),
        ("PROGRAMFILES(X86)", "OpenAI Codex\\Codex.exe"),
        ("PROGRAMFILES(X86)", "OpenAI\\Codex\\Codex.exe"),
    ];

    for (env_key, suffix) in env_candidates {
        if let Ok(prefix) = std::env::var(env_key) {
            if !prefix.trim().is_empty() {
                candidates.push(PathBuf::from(prefix).join(suffix));
            }
        }
    }

    candidates
}

#[cfg(target_os = "windows")]
fn find_codex_from_registry() -> Option<PathBuf> {
    let reg_paths = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Codex.exe",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Codex.exe",
    ];
    for reg_path in reg_paths {
        if let Some(p) = query_reg_default_value(reg_path) {
            let pb = PathBuf::from(&p);
            if is_windows_codex_desktop_path(&pb) {
                return Some(pb);
            }
        }
    }

    let uninstall_roots = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    for root in uninstall_roots {
        if let Some(p) = find_codex_in_uninstall_entries(root) {
            return Some(p);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn query_reg_default_value(key_path: &str) -> Option<String> {
    let output = crate::platform::windows::background_command("reg")
        .args(["query", key_path, "/ve"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.contains("REG_SZ") || trimmed.contains("REG_EXPAND_SZ") {
            let parts: Vec<&str> = trimmed.splitn(3, "    ").collect();
            if parts.len() == 3 {
                let value = parts[2].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_codex_in_uninstall_entries(root: &str) -> Option<PathBuf> {
    let output = crate::platform::windows::background_command("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-ChildItem 'Registry::{}' -ErrorAction SilentlyContinue | ForEach-Object {{ \
                    $dn = (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName; \
                    $il = (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).InstallLocation; \
                    if ($dn -like '*Codex*' -and $il) {{ Write-Output $il }} \
                }}",
                root
            ),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let dir = line.trim();
        if dir.is_empty() {
            continue;
        }
        let exe = PathBuf::from(dir).join("Codex.exe");
        if is_windows_codex_desktop_path(&exe) {
            return Some(exe);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn windows_powershell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn launch_codex_app(proxy: &ApiProxyConfigPayload) -> Result<(), CoreError> {
    let mut command = std::process::Command::new("codex");
    configure_command_network(&mut command, proxy)?;
    command
        .spawn()
        .map_err(|e| CoreError::OperationFailed(format!("launch failed: {e}")))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn restart_codex_app(
    proxy: &ApiProxyConfigPayload,
    paths: &crate::platform::paths::CodexPaths,
) -> Result<(), CoreError> {
    stop_codex_app_gracefully(Duration::from_secs(8))?;
    crate::core::codex_config::prepare_network_config(paths, proxy)?;
    launch_codex_app(proxy)
}
