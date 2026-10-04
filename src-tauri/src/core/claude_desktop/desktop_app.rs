//! Claude Desktop 直连：桌面应用的进程发现、停止与启动。
//!
//! 与既有客户端一致的做法：**先停应用、再写配置、最后启动**。Desktop 在退出时
//! 会回写自己的状态（真实日志里能看到 onQuitCleanup 与 readyForQuit），所以
//! 「先写文件再杀进程」有被回写覆盖的风险。
//!
//! 这里只碰进程，不碰配置：调用方负责决定何时写文件。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::paths::DesktopPaths;
use super::types::{error_code, DesktopError, DesktopResult};

/// Desktop 主进程名（MSIX 包内是 `app` + 分隔符 + `Claude.exe`）。
///
/// **不要**匹配 `Claude-3p` 下的 `claude-code` CLI：那是被 Desktop 当作工具
/// 调用起来的 Claude Code，杀掉它会破坏正在运行的会话。
pub const PROCESS_STEM: &str = "Claude";

/// 已核实的 MSIX 包族名前缀。
pub const PACKAGE_FAMILY: &str = "Claude_pzs8sxrjxfjjc";

/// 启动方式；`None` 表示这个平台没有可确认的启动路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Windows MSIX 包激活（explorer 的 shell:AppsFolder 形式）。
    MsixActivation,
    /// Windows 非打包安装：直接启动可执行文件。
    Executable,
    /// macOS 的 `open -b` 包标识。
    MacBundle,
    /// Linux 桌面入口。
    LinuxDesktop,
}

impl LaunchMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MsixActivation => "msixActivation",
            Self::Executable => "executable",
            Self::MacBundle => "macBundle",
            Self::LinuxDesktop => "linuxDesktop",
        }
    }
}

/// Desktop 应用当前状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    pub running: bool,
    /// 找到的主进程数量。
    pub process_count: usize,
    /// 已确认的启动方式；`None` 时界面不提供重启。
    pub launch_method: Option<LaunchMethod>,
    /// 启动目标（包激活 id、可执行文件或包标识）。
    pub launch_target: Option<PathBuf>,
}

/// A launch entry and its executable belong to one confirmed installation.
/// All process operations keep this identity instead of matching a global name.
struct AppIdentity {
    method: LaunchMethod,
    target: PathBuf,
    executable: PathBuf,
}

fn process_error(code: &str) -> Box<DesktopError> {
    Box::new(DesktopError::new(code, "claudeDesktop.errors.process").retryable())
}

// ---------------------------------------------------------------------------
// 纯函数：进程名与包标识
// ---------------------------------------------------------------------------

/// 判断一个进程名是否是我们管理的 Desktop 主进程。
pub fn is_desktop_process_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(PROCESS_STEM)
}

/// 排除 Claude Code CLI：它位于 `claude-code` 目录下，不是桌面应用。
pub fn is_cli_path(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    normalized.split('\\').any(|part| part == "claude-code")
}

fn executable_matches(actual: &Path, expected: &Path) -> bool {
    if is_cli_path(&actual.to_string_lossy()) || is_cli_path(&expected.to_string_lossy()) {
        return false;
    }
    // /proc reports the resolved executable; a desktop entry may use a symlink.
    let actual = actual.canonicalize().unwrap_or_else(|_| actual.to_owned());
    let expected = expected
        .canonicalize()
        .unwrap_or_else(|_| expected.to_owned());
    if is_cli_path(&actual.to_string_lossy()) || is_cli_path(&expected.to_string_lossy()) {
        return false;
    }
    super::paths::paths_equal(&actual, &expected)
}

fn process_control_allowed(paths: &DesktopPaths) -> bool {
    !paths.isolated && !paths.overridden && super::paths::isolated_home().is_none()
}

/// 解析 `tasklist /FO CSV /NH` 的输出为 (pid, 镜像名)。
pub fn parse_tasklist_csv(output: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let fields: Vec<String> = line
            .split(",")
            .map(|field| field.trim_matches('"').trim().to_owned())
            .collect();
        if fields.len() < 2 {
            continue;
        }
        let Ok(pid) = fields[1].parse::<u32>() else {
            continue;
        };
        out.push((pid, fields[0].clone()));
    }
    out
}

#[cfg(any(windows, test))]
fn tasklist_candidates(output: &str) -> DesktopResult<Vec<u32>> {
    let records = parse_tasklist_csv(output);
    let csv_rows = output
        .lines()
        .filter(|line| line.trim().starts_with('"'))
        .count();
    if records.len() != csv_rows
        || records
            .iter()
            .any(|(pid, image)| *pid == 0 || !image.eq_ignore_ascii_case("Claude.exe"))
    {
        return Err(process_error(error_code::PROCESS));
    }
    Ok(records.into_iter().map(|(pid, _)| pid).collect())
}

/// 只接受本项目已核实的包族名与 `!Claude` 应用 id。
pub fn is_msix_app_id(text: &str) -> bool {
    let Some((family, app)) = text.trim().split_once('!') else {
        return false;
    };
    family == PACKAGE_FAMILY && app == "Claude"
}

// ---------------------------------------------------------------------------
// 状态与操作
// ---------------------------------------------------------------------------

/// 读取 Desktop 应用状态。发现失败不报错：只是没有重启能力。
pub fn state(paths: &DesktopPaths) -> AppState {
    let identity = app_identity(paths);
    let processes = identity
        .as_ref()
        .and_then(|identity| list_processes(identity).ok())
        .unwrap_or_default();
    AppState {
        running: !processes.is_empty(),
        process_count: processes.len(),
        launch_method: identity.as_ref().map(|identity| identity.method),
        launch_target: identity.map(|identity| identity.target),
    }
}

/// The write gate must distinguish a failed process query from a stopped app.
/// An unknown Claude installation is never signalled, but blocks a production
/// write until the user closes it. Test overrides never inspect real processes.
pub fn ensure_stopped(paths: &DesktopPaths) -> DesktopResult<()> {
    if !process_control_allowed(paths) {
        return Ok(());
    }
    let identity = app_identity(paths);
    let processes = checked_processes(
        identity.as_ref(),
        candidate_processes(identity.as_ref()),
        process_executable,
    );
    stopped_result(processes)
}

fn stopped_result(processes: DesktopResult<Vec<u32>>) -> DesktopResult<()> {
    if processes?.is_empty() {
        return Ok(());
    }
    Err(Box::new(
        DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.desktopRunning")
            .retryable()
            .action("repreview"),
    ))
}

fn checked_processes(
    identity: Option<&AppIdentity>,
    candidates: DesktopResult<Vec<u32>>,
    mut executable: impl FnMut(u32) -> Option<PathBuf>,
) -> DesktopResult<Vec<u32>> {
    let mut matched = Vec::new();
    for pid in candidates? {
        let path = executable(pid).ok_or_else(|| process_error(error_code::PROCESS))?;
        if is_cli_path(&path.to_string_lossy()) {
            continue;
        }
        let identity = identity.ok_or_else(|| process_error(error_code::PROCESS))?;
        if executable_matches(&path, &identity.executable) {
            matched.push(pid);
        }
    }
    Ok(matched)
}

fn list_processes(identity: &AppIdentity) -> DesktopResult<Vec<u32>> {
    checked_processes(
        Some(identity),
        candidate_processes(Some(identity)),
        process_executable,
    )
}

/// 停止 Desktop，返回是否确实停到了进程。
pub fn stop(paths: &DesktopPaths, timeout: Duration) -> DesktopResult<bool> {
    let identity = app_identity(paths).ok_or_else(|| {
        Box::new(
            DesktopError::new(
                error_code::PROCESS,
                "claudeDesktop.errors.launchUnavailable",
            )
            .action("openManually"),
        )
    })?;
    let processes = list_processes(&identity)?;
    if processes.is_empty() {
        return Ok(false);
    }
    for pid in &processes {
        signal(*pid, false)?;
    }
    if wait_until_stopped(&identity, timeout)? {
        return Ok(true);
    }
    // 优雅退出没完成时强制结束，避免半退出状态下写文件被回写覆盖。
    // Re-enumerate by executable identity before forcing: PIDs can be reused.
    for pid in &list_processes(&identity)? {
        signal(*pid, true)?;
    }
    if wait_until_stopped(&identity, Duration::from_secs(5))? {
        return Ok(true);
    }
    Err(Box::new(
        DesktopError::new(
            error_code::PROCESS,
            "claudeDesktop.errors.processStopTimeout",
        )
        .retryable(),
    ))
}

/// 启动 Desktop 并等待主进程出现。
pub fn launch(paths: &DesktopPaths, timeout: Duration) -> DesktopResult<()> {
    let Some(identity) = app_identity(paths) else {
        return Err(Box::new(
            DesktopError::new(
                error_code::PROCESS,
                "claudeDesktop.errors.launchUnavailable",
            )
            .action("openManually"),
        ));
    };
    let spawned = match identity.method {
        LaunchMethod::MsixActivation => msix_activate(&identity.target),
        LaunchMethod::Executable => cmd_spawn(&identity.target, &[]),
        LaunchMethod::MacBundle => cmd_spawn(
            Path::new("open"),
            &["-a", &identity.target.to_string_lossy()],
        ),
        LaunchMethod::LinuxDesktop => linux_launch_command(&identity.target)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string()),
    };
    if let Err(detail) = spawned {
        return Err(Box::new(
            DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.launchFailed")
                .param(detail)
                .retryable()
                .action("openManually"),
        ));
    }
    if wait_until_running(&identity, timeout)? {
        Ok(())
    } else {
        Err(Box::new(
            DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.launchTimeout")
                .retryable()
                .action("openManually"),
        ))
    }
}

// ---------------------------------------------------------------------------
// 平台实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn candidate_processes(_identity: Option<&AppIdentity>) -> DesktopResult<Vec<u32>> {
    let filter = format!("IMAGENAME eq {}.exe", PROCESS_STEM);
    let output = crate::platform::windows::background_command("tasklist")
        .args(["/FO", "CSV", "/NH", "/FI", &filter])
        .output()
        .map_err(|_| process_error(error_code::PROCESS))?;
    if !output.status.success() {
        return Err(process_error(error_code::PROCESS));
    }
    tasklist_candidates(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
fn process_executable(pid: u32) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(handle);
        (ok != 0 && length > 0)
            .then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
    }
}

#[cfg(target_os = "windows")]
fn signal(pid: u32, force: bool) -> DesktopResult<()> {
    let mut command = crate::platform::windows::background_command("taskkill");
    command.args(["/PID", &pid.to_string()]);
    if force {
        // /T would also terminate Claude Code tools descended from Desktop.
        command.arg("/F");
    }
    command
        .output()
        .map_err(|_| process_error(error_code::PROCESS))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn msix_activate(app_id: &Path) -> Result<(), String> {
    let target = format!("shell:AppsFolder\\{}", app_id.to_string_lossy());
    crate::platform::windows::background_command("explorer.exe")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn app_identity(paths: &DesktopPaths) -> Option<AppIdentity> {
    if !process_control_allowed(paths) {
        return None;
    }
    // MSIX is only known to own the standard local-data pair. An unrelated
    // selected channel must not silently restart the global Claude package.
    if let Some(base) = dirs::data_local_dir() {
        if super::paths::paths_equal(&paths.normal_root, &base.join("Claude"))
            && super::paths::paths_equal(&paths.third_party_root, &base.join("Claude-3p"))
        {
            if let Some(identity) = msix_identity() {
                return Some(identity);
            }
        }
    }
    // Nonpackaged installs must have their executable in the selected roots.
    for root in [&paths.normal_root, &paths.third_party_root] {
        for relative in [r"app\Claude.exe", "Claude.exe"] {
            let candidate = root.join(relative);
            if candidate.is_file() && !is_cli_path(&candidate.to_string_lossy()) {
                return Some(AppIdentity {
                    method: LaunchMethod::Executable,
                    target: candidate.clone(),
                    executable: candidate,
                });
            }
        }
    }
    None
}

/// Resolve both the exact package activation ID and its manifest executable.
#[cfg(target_os = "windows")]
fn msix_identity() -> Option<AppIdentity> {
    let script = "$p = @(Get-AppxPackage -Name 'Claude' -ErrorAction SilentlyContinue | Where-Object { $_.PackageFamilyName -eq 'Claude_pzs8sxrjxfjjc' }); if ($p.Count -eq 1) { $a = @((Get-AppxPackageManifest $p[0]).Package.Applications.Application | Where-Object { $_.Id -eq 'Claude' }); if ($a.Count -eq 1) { [pscustomobject]@{ appId = $p[0].PackageFamilyName + '!' + $a[0].Id; executable = Join-Path $p[0].InstallLocation $a[0].Executable } | ConvertTo-Json -Compress } }";
    let output = crate::platform::windows::background_command("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let record: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let app_id = record["appId"].as_str()?;
    let executable = PathBuf::from(record["executable"].as_str()?);
    if !is_msix_app_id(app_id)
        || !executable.is_absolute()
        || !executable.is_file()
        || is_cli_path(&executable.to_string_lossy())
    {
        return None;
    }
    Some(AppIdentity {
        method: LaunchMethod::MsixActivation,
        target: PathBuf::from(app_id),
        executable,
    })
}

#[cfg(unix)]
fn candidate_processes(identity: Option<&AppIdentity>) -> DesktopResult<Vec<u32>> {
    let names = match identity {
        Some(identity) => vec![identity
            .executable
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| process_error(error_code::PROCESS))?],
        None => vec![PROCESS_STEM, "claude-desktop"],
    };
    let mut candidates = Vec::new();
    for name in names {
        let output = std::process::Command::new("pgrep")
            .args(["-x", name])
            .output()
            .map_err(|_| process_error(error_code::PROCESS))?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(process_error(error_code::PROCESS));
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let pid = line
                .trim()
                .parse::<u32>()
                .map_err(|_| process_error(error_code::PROCESS))?;
            if !candidates.contains(&pid) {
                candidates.push(pid);
            }
        }
    }
    Ok(candidates)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn process_executable(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(target_os = "macos")]
fn process_executable(pid: u32) -> Option<PathBuf> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(text.trim());
    path.is_absolute().then_some(path)
}

#[cfg(unix)]
fn signal(pid: u32, force: bool) -> DesktopResult<()> {
    let flag = if force { "-KILL" } else { "-TERM" };
    std::process::Command::new("kill")
        .args([flag, &pid.to_string()])
        .output()
        .map_err(|_| process_error(error_code::PROCESS))?;
    Ok(())
}

/// macOS 的包标识。
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

#[cfg(target_os = "macos")]
fn app_identity(paths: &DesktopPaths) -> Option<AppIdentity> {
    // 与 Windows 同理：隔离根或显式覆盖下不得触碰用户真实安装。
    if !process_control_allowed(paths) {
        return None;
    }
    let base = super::paths::macos_app_support(&dirs::home_dir()?);
    if !super::paths::paths_equal(&paths.normal_root, &base.join("Claude"))
        || !super::paths::paths_equal(&paths.third_party_root, &base.join("Claude-3p"))
    {
        return None;
    }
    // 只有确实装了才提供重启：否则界面会出现一个点了必然失败的按钮。
    let bundle = macos_bundle_path(MACOS_BUNDLE_ID)?;
    let executable = bundle.join("Contents/MacOS/Claude");
    executable.is_file().then_some(AppIdentity {
        method: LaunchMethod::MacBundle,
        target: bundle,
        executable,
    })
}

/// 用 `mdfind` 查询包是否已安装；查询失败时保守地认为未确认。
#[cfg(target_os = "macos")]
fn macos_bundle_path(bundle_id: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == {bundle_id}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let bundle = PathBuf::from(entries.next()?);
    // Multiple bundles with one ID cannot be safely selected by global ID.
    (entries.next().is_none() && bundle.is_absolute()).then_some(bundle)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn app_identity(paths: &DesktopPaths) -> Option<AppIdentity> {
    if !process_control_allowed(paths) {
        return None;
    }
    let base = super::paths::config_dir_from_xdg(
        &dirs::home_dir()?,
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
    );
    if !super::paths::paths_equal(&paths.normal_root, &base.join("Claude"))
        || !super::paths::paths_equal(&paths.third_party_root, &base.join("Claude-3p"))
    {
        return None;
    }
    // 配置根下不会有可执行文件；只采用已确认主入口的桌面文件。
    let mut application_dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];
    if let Some(data_dir) = dirs::data_dir().filter(|path| path.is_absolute()) {
        application_dirs.insert(0, data_dir.join("applications"));
    }
    let mut selected: Option<AppIdentity> = None;
    for entry in [
        "claude-desktop.desktop",
        "com.anthropic.claudefordesktop.desktop",
    ] {
        for base in &application_dirs {
            let candidate = base.join(entry);
            if let Some(executable) = std::fs::read_to_string(&candidate)
                .ok()
                .and_then(|text| desktop_entry_executable(&text))
                .filter(|path| path.is_file())
            {
                if let Some(identity) = &selected {
                    // Two different applications cannot be assigned to one config pair.
                    if !executable_matches(&executable, &identity.executable) {
                        return None;
                    }
                } else {
                    selected = Some(AppIdentity {
                        method: LaunchMethod::LinuxDesktop,
                        target: candidate,
                        executable,
                    });
                }
            }
        }
    }
    selected
}

/// Only a desktop entry naming an absolute Desktop executable is confirmed.
/// A PATH `claude` command may be Claude Code and is never a launch fallback.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn desktop_entry_executable(text: &str) -> Option<PathBuf> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut command = None;
    let mut entry_type = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            if in_entry {
                if seen_entry {
                    return None;
                }
                seen_entry = true;
            }
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Type=") {
            if entry_type.replace(value).is_some() {
                return None;
            }
        } else if let Some(value) = line.strip_prefix("Exec=") {
            if command.replace(value.trim()).is_some() {
                return None;
            }
        } else if line
            .strip_prefix("DBusActivatable=")
            .is_some_and(|value| value != "false")
            || line
                .strip_prefix("Hidden=")
                .is_some_and(|value| value != "false")
        {
            // D-Bus activation can ignore Exec, so its process identity is unknown.
            return None;
        }
    }
    if entry_type != Some("Application") {
        return None;
    }
    let command = command?;
    let program = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"')?.0
    } else {
        command.split_whitespace().next()?
    };
    let path = PathBuf::from(program);
    if !path.is_absolute()
        || is_cli_path(program)
        || !matches!(path.file_name()?.to_str()?, "Claude" | "claude-desktop")
    {
        return None;
    }
    Some(path)
}

#[cfg(not(any(windows, unix)))]
fn candidate_processes(_identity: Option<&AppIdentity>) -> DesktopResult<Vec<u32>> {
    Err(process_error(error_code::PROCESS))
}

#[cfg(not(any(windows, unix)))]
fn process_executable(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(not(any(windows, unix)))]
fn signal(_pid: u32, _force: bool) -> DesktopResult<()> {
    Err(process_error(error_code::PROCESS))
}

#[cfg(not(any(windows, unix)))]
fn app_identity(_paths: &DesktopPaths) -> Option<AppIdentity> {
    None
}

fn cmd_spawn(program: &Path, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn linux_launch_command(target: &Path) -> std::process::Command {
    // GIO launch accepts an exact file path; GTK launch performs desktop-ID lookup.
    let mut command = std::process::Command::new("gio");
    command.arg("launch").arg(target);
    command
}

fn wait_until_stopped(identity: &AppIdentity, timeout: Duration) -> DesktopResult<bool> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if list_processes(identity)?.is_empty() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(list_processes(identity)?.is_empty())
}

fn wait_until_running(identity: &AppIdentity, timeout: Duration) -> DesktopResult<bool> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !list_processes(identity)?.is_empty() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(!list_processes(identity)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_desktop_process_name_matches() {
        assert!(is_desktop_process_name("Claude"));
        assert!(is_desktop_process_name("claude"));
        assert!(is_desktop_process_name(" CLAUDE "));
        // 其它客户端与相近名字都必须排除。
        assert!(!is_desktop_process_name("claude-code"));
        assert!(!is_desktop_process_name("ClaudeHelper"));
        assert!(!is_desktop_process_name("Codex"));
        assert!(!is_desktop_process_name(""));
    }
    #[test]
    fn the_claude_code_cli_path_is_recognised() {
        assert!(is_cli_path(
            r"C:\Users\u\AppData\Local\Claude-3p\claude-code\2.1.286\claude.exe"
        ));
        assert!(is_cli_path("/home/u/.local/share/claude-code/claude"));
        assert!(!is_cli_path(
            r"C:\Program Files\WindowsApps\Claude_2.0_x64__p\app\Claude.exe"
        ));
    }

    #[test]
    fn tasklist_csv_parsing_ignores_noise() {
        // 无匹配进程时 tasklist 打印一条信息行，而不是 CSV。
        let none = "INFO: No tasks are running which match the specified criteria.";
        assert!(parse_tasklist_csv(none).is_empty());
        assert!(parse_tasklist_csv("").is_empty());
        let some = "\"Claude.exe\",\"1234\",\"Console\",\"1\",\"123,456 K\"\r\n\"Claude.exe\",\"5678\",\"Console\",\"1\",\"12,345 K\"\r\n";
        assert_eq!(
            parse_tasklist_csv(some),
            vec![
                (1234, "Claude.exe".to_owned()),
                (5678, "Claude.exe".to_owned())
            ]
        );
        assert_eq!(tasklist_candidates(some).unwrap(), vec![1234, 5678]);
        assert!(tasklist_candidates("\"Claude.exe\",\"not-a-pid\"").is_err());
        assert!(tasklist_candidates("\"unrelated.exe\",\"1234\"").is_err());
    }

    #[test]
    fn only_the_verified_package_family_and_app_id_are_accepted() {
        assert!(is_msix_app_id("Claude_pzs8sxrjxfjjc!Claude"));
        // 版本化的完整包名不是 AUMID。
        assert!(!is_msix_app_id(
            "Claude_2.19675.0.0_x64__pzs8sxrjxfjjc!Claude"
        ));
        // 同一包里的其它应用不是桌面主程序。
        assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjc!SshProxy"));
        assert!(!is_msix_app_id("Other_pzs8sxrjxfjjc!Claude"));
        assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjc"));
        assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjcOther!Claude"));
    }

    #[test]
    fn process_matching_requires_the_selected_executable_and_excludes_cli() {
        let base = std::env::temp_dir().join("desktop-process-identity");
        let desktop = base.join("Claude/app/Claude.exe");
        let cli = base.join("Claude-3p/claude-code/2.1.286/claude.exe");
        let unrelated = base.join("other-install/app/Claude.exe");
        assert!(executable_matches(&desktop, &desktop));
        assert!(!executable_matches(&cli, &desktop));
        assert!(!executable_matches(&unrelated, &desktop));
        assert!(!executable_matches(&cli, &cli));
        #[cfg(windows)]
        assert!(executable_matches(
            &PathBuf::from(desktop.to_string_lossy().to_ascii_lowercase()),
            &desktop
        ));
    }

    #[test]
    fn process_query_failures_never_authorize_a_write() {
        let result = checked_processes(None, Err(process_error(error_code::PROCESS)), |_| {
            panic!("a failed enumeration must not query any PID")
        });
        assert!(stopped_result(result).is_err());
        let result = checked_processes(None, Ok(vec![7]), |_| None);
        assert!(stopped_result(result).is_err());
    }

    #[test]
    fn an_unknown_installation_blocks_writes_but_a_confirmed_cli_does_not() {
        let base = std::env::temp_dir().join("desktop-write-gate");
        let unknown = base.join("other-install/app/Claude.exe");
        assert!(stopped_result(checked_processes(None, Ok(vec![7]), |_| {
            Some(unknown.clone())
        }))
        .is_err());
        let cli = base.join("claude-code/2.1.286/claude.exe");
        assert!(stopped_result(checked_processes(None, Ok(vec![7]), |_| {
            Some(cli.clone())
        }))
        .is_ok());
        assert!(stopped_result(checked_processes(None, Ok(vec![]), |_| {
            panic!("an empty enumeration must not query any PID")
        }))
        .is_ok());
    }

    #[test]
    fn the_write_gate_matches_only_the_confirmed_installation() {
        let base = std::env::temp_dir().join("desktop-write-gate");
        let executable = base.join("Claude/app/Claude.exe");
        let identity = AppIdentity {
            method: LaunchMethod::Executable,
            target: executable.clone(),
            executable: executable.clone(),
        };
        let result = checked_processes(Some(&identity), Ok(vec![7]), |_| Some(executable.clone()));
        assert_eq!(
            stopped_result(result).unwrap_err().message,
            "claudeDesktop.errors.desktopRunning"
        );
        let unrelated = base.join("unrelated/Claude.exe");
        assert!(
            stopped_result(checked_processes(Some(&identity), Ok(vec![7]), |_| {
                Some(unrelated.clone())
            }))
            .is_ok()
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_process_query_identifies_the_current_test_process() {
        let actual = process_executable(std::process::id()).expect("read our own executable");
        assert!(executable_matches(
            &actual,
            &std::env::current_exe().unwrap()
        ));
    }

    #[test]
    fn desktop_entries_cannot_launch_the_path_claude_cli() {
        assert!(
            desktop_entry_executable("[Desktop Entry]\nType=Application\nExec=claude").is_none()
        );
        assert!(
            desktop_entry_executable("[Desktop Entry]\nType=Application\nExec=claude-desktop")
                .is_none()
        );
        let base = std::env::temp_dir().join("desktop-entry-identity");
        let desktop = base.join("Claude");
        let cli = base.join("claude-code/Claude");
        assert_eq!(
            desktop_entry_executable(&format!(
                "[Desktop Entry]\nType=Application\nExec=\"{}\" %U",
                desktop.display()
            )),
            Some(desktop)
        );
        assert!(desktop_entry_executable(&format!(
            "[Desktop Entry]\nType=Application\nExec=\"{}\"",
            cli.display()
        ))
        .is_none());
    }

    #[test]
    fn desktop_entry_identity_comes_only_from_the_main_application_entry() {
        let base = std::env::temp_dir().join("desktop-main-entry");
        let desktop = base.join("Claude");
        let action = base.join("actions/Claude");
        let entry = format!("[Desktop Action Auxiliary]\nExec=\"{}\"\n[Desktop Entry]\nType=Application\nExec=\"{}\" %U", action.display(), desktop.display());
        assert_eq!(desktop_entry_executable(&entry), Some(desktop));
        assert!(desktop_entry_executable(&format!("{entry}\nDBusActivatable=true")).is_none());
        assert!(desktop_entry_executable(&format!("{entry}\nHidden=true")).is_none());
        assert!(
            desktop_entry_executable(&format!("{entry}\nExec=\"{}\"", action.display())).is_none()
        );
        assert!(desktop_entry_executable(&entry.replace("Type=Application\n", "")).is_none());
    }

    #[test]
    fn linux_launch_uses_the_confirmed_desktop_file_path() {
        let entry = std::env::temp_dir().join("selected install/claude-desktop.desktop");
        let command = linux_launch_command(&entry);
        assert_eq!(command.get_program(), std::ffi::OsStr::new("gio"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("launch"), entry.as_os_str(),]
        );
    }

    /// 隔离根不能拿到启动能力：否则会把用户真实的 Desktop 拉起来。
    #[test]
    fn an_isolated_root_never_launches_the_production_app() {
        // 与设置隔离根的测试共用锁，避免互相干扰。
        let _guard = crate::core::claude_desktop::env_test_lock();
        let base = std::env::temp_dir().join(format!("agentharbor-iso-{}", uuid::Uuid::new_v4()));
        let mut paths =
            DesktopPaths::from_roots(base.join("Claude"), base.join("Claude-3p")).unwrap();
        assert!(!paths.isolated);
        // 隔离根必须一律拒绝，即使本机确实装了 Desktop。
        paths.isolated = true;
        assert!(app_identity(&paths).is_none());
        assert!(ensure_stopped(&paths).is_ok());
    }

    #[test]
    fn explicit_overrides_disable_process_control_even_with_an_executable() {
        let _guard = crate::core::claude_desktop::env_test_lock();
        let base =
            std::env::temp_dir().join(format!("agentharbor-override-{}", uuid::Uuid::new_v4()));
        let mut paths =
            DesktopPaths::from_roots(base.join("Claude"), base.join("Claude-3p")).unwrap();
        std::fs::create_dir_all(paths.normal_root.join("app")).unwrap();
        std::fs::write(
            paths.normal_root.join("app/Claude.exe"),
            b"synthetic executable",
        )
        .unwrap();
        paths.overridden = true;
        assert!(!process_control_allowed(&paths));
        assert!(app_identity(&paths).is_none());
        assert!(ensure_stopped(&paths).is_ok());
    }
}
