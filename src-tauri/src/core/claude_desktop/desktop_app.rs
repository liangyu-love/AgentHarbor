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

mod command;
#[cfg(windows)]
mod windows;

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
    pub discovery_error: Option<String>,
    pub running: bool,
    /// 找到的主进程数量。
    pub process_count: usize,
    /// 已确认的启动方式；`None` 时界面不提供重启。
    pub launch_method: Option<LaunchMethod>,
    /// 启动目标（包激活 id、可执行文件或包标识）。
    pub launch_target: Option<PathBuf>,
}

impl AppState {
    pub fn restart_capable(&self) -> bool {
        self.launch_method.is_some() || self.discovery_error.is_some()
    }
}

/// A launch entry and its executable belong to one confirmed installation.
/// All process operations keep this identity instead of matching a global name.
struct AppIdentity {
    method: LaunchMethod,
    target: PathBuf,
    executable: PathBuf,
}

enum ProcessImage {
    Running(PathBuf),
    Exited,
    Unknown,
}

impl From<Option<PathBuf>> for ProcessImage {
    fn from(path: Option<PathBuf>) -> Self {
        path.map(Self::Running).unwrap_or(Self::Unknown)
    }
}

/// Reuse installation discovery within one transaction, but query live
/// processes at every write gate. This is not a cache of "already stopped".
pub struct StopCheck {
    allowed: bool,
    identity: Option<AppIdentity>,
    discovery_error: Option<Box<DesktopError>>,
}

impl StopCheck {
    pub fn new(paths: &DesktopPaths) -> Self {
        let allowed = process_control_allowed(paths);
        let (identity, discovery_error) = match if allowed { resolve_identity(paths) } else { Ok(None) } {
            Ok(identity) => (identity, None), Err(error) => (None, Some(error)),
        };
        Self {
            allowed,
            identity,
            discovery_error,
        }
    }

    pub fn ensure_stopped(&self) -> DesktopResult<()> {
        if !self.allowed {
            return Ok(());
        }
        if let Some(error) = &self.discovery_error { return Err(error.clone()); }
        stopped_result(checked_processes(
            self.identity.as_ref(),
            candidate_processes(self.identity.as_ref()),
            process_image,
        ))
    }

    /// A fresh scan immediately after shutdown can still observe a disappearing
    /// Electron helper whose image is unreadable. Retry only inconclusive scans;
    /// a confirmed running instance must block the repair without delay.
    pub fn ensure_stopped_after_exit(&self) -> DesktopResult<()> {
        if !self.allowed { return Ok(()); }
        if let Some(error) = &self.discovery_error { return Err(error.clone()); }
        stopped_after_exit(Duration::from_millis(600), Duration::from_millis(100), || {
            checked_processes(
                self.identity.as_ref(),
                candidate_processes(self.identity.as_ref()),
                process_image,
            )
        })
    }
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

/// 读取 Desktop 应用状态；发现失败保留可重试原因，不当成「未安装」。
pub fn state(paths: &DesktopPaths) -> AppState {
    let (identity, mut discovery_error) = match resolve_identity(paths) {
        Ok(identity) => (identity, None), Err(error) => (None, Some(error.message.clone())),
    };
    let processes = if let Some(identity) = &identity {
        match list_processes(identity) {
            Ok(processes) => processes,
            Err(error) => { discovery_error = Some(error.message.clone()); Vec::new() }
        }
    } else { Vec::new() };
    AppState {
        discovery_error,
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
    StopCheck::new(paths).ensure_stopped()
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

fn stopped_after_exit(timeout: Duration, poll_interval: Duration, mut list: impl FnMut() -> DesktopResult<Vec<u32>>) -> DesktopResult<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match list() {
            Ok(processes) => return stopped_result(Ok(processes)),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        std::thread::sleep(poll_interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn checked_processes<T: Into<ProcessImage>>(
    identity: Option<&AppIdentity>,
    candidates: DesktopResult<Vec<u32>>,
    mut executable: impl FnMut(u32) -> T,
) -> DesktopResult<Vec<u32>> {
    let mut matched = Vec::new();
    for pid in candidates? {
        let path = match executable(pid).into() {
            ProcessImage::Running(path) => path,
            ProcessImage::Exited => continue,
            ProcessImage::Unknown => return Err(process_error(error_code::PROCESS)),
        };
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
        process_image,
    )
}

/// 停止 Desktop，返回是否确实停到了进程。
pub fn stop(paths: &DesktopPaths, timeout: Duration) -> DesktopResult<bool> {
    let identity = resolve_identity(paths)?.ok_or_else(|| {
        Box::new(
            DesktopError::new(
                error_code::PROCESS,
                "claudeDesktop.errors.launchUnavailable",
            )
            .action("openManually"),
        )
    })?;
    stop_identity(&identity, timeout)
}

fn stop_identity(identity: &AppIdentity, timeout: Duration) -> DesktopResult<bool> {
    stop_processes(timeout, || list_processes(identity), |pid, force| signal(identity, pid, force))
}

fn stop_processes(timeout: Duration, mut list: impl FnMut() -> DesktopResult<Vec<u32>>, mut send: impl FnMut(u32, bool) -> DesktopResult<bool>) -> DesktopResult<bool> {
    stop_processes_with_grace(timeout, Duration::from_secs(2), &mut list, &mut send)
}

fn stop_processes_with_grace(timeout: Duration, headless_grace: Duration, mut list: impl FnMut() -> DesktopResult<Vec<u32>>, mut send: impl FnMut(u32, bool) -> DesktopResult<bool>) -> DesktopResult<bool> {
    stop_processes_with_timeouts(timeout, headless_grace, Duration::from_secs(5), &mut list, &mut send)
}

fn stop_processes_with_timeouts(timeout: Duration, headless_grace: Duration, force_timeout: Duration, mut list: impl FnMut() -> DesktopResult<Vec<u32>>, mut send: impl FnMut(u32, bool) -> DesktopResult<bool>) -> DesktopResult<bool> {
    let processes = list()?;
    if processes.is_empty() {
        return Ok(false);
    }
    let deadline = Instant::now() + timeout;
    let headless_deadline = Instant::now() + headless_grace;
    let mut requested = std::collections::HashSet::new();
    let mut headless = std::collections::HashSet::new();
    let mut failures = std::collections::HashSet::new();
    let mut current = processes;
    loop {
        for pid in &current {
            // Individual posting failures never suppress cleanup of other
            // verified processes or the forced-stop fallback.
            if requested.contains(pid) { continue; }
            match send(*pid, false) {
                Ok(true) => { requested.insert(*pid); headless.remove(pid); }
                Ok(false) => { headless.insert(*pid); }
                Err(error) => {
                    headless.remove(pid);
                    if failures.insert(*pid) { log::warn!("Desktop stop signal failed: pid={pid}, force=false, code={}, reason={}, details={:?}", error.code, error.message, error.params); }
                }
            }
        }
        // A late Electron window gets a short chance to appear. Persistent
        // headless residuals do not need the full main-window shutdown budget.
        if !current.is_empty() && current.iter().all(|pid| headless.contains(pid)) && Instant::now() >= headless_deadline { break; }
        if Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(200));
        match list() {
            Ok(processes) if processes.is_empty() => return Ok(true),
            Ok(processes) => current = processes,
            Err(_) => {
                // Electron helpers can disappear between enumeration and image
                // inspection. An unreadable snapshot is not proof of shutdown,
                // but also must not abort before a later scan can confirm it.
                current.clear();
            }
        }
    }
    // 优雅退出没完成时强制结束，避免半退出状态下写文件被回写覆盖。
    // Re-enumerate by executable identity before forcing: PIDs can be reused.
    let deadline = Instant::now() + force_timeout;
    failures.clear();
    let mut scan_error;
    loop {
        match list() {
            Ok(processes) if processes.is_empty() => return Ok(true),
            Ok(processes) => {
                scan_error = None;
                for pid in processes {
                    // Only a complete, identity-checked scan authorizes force.
                    // signal rechecks the exact process handle to avoid PID reuse.
                    if let Err(error) = send(pid, true) {
                        if failures.insert(pid) {
                            log::warn!("Desktop stop signal failed: pid={pid}, force=true, code={}, reason={}, details={:?}", error.code, error.message, error.params);
                        }
                    }
                }
            }
            Err(error) => scan_error = Some(error),
        }
        if Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(200));
    }
    if let Some(error) = scan_error { return Err(stop_verification_error(error)); }
    Err(Box::new(
        DesktopError::new(
            error_code::PROCESS,
            "claudeDesktop.errors.processStopTimeout",
        )
        .retryable(),
    ))
}

fn stop_verification_error(error: Box<DesktopError>) -> Box<DesktopError> {
    Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.processStopTimeout")
        .param(format!("{}: {}; {:?}", error.code, error.message, error.params)).retryable().action("openManually"))
}

/// 启动 Desktop 并等待主进程出现。
pub fn launch(paths: &DesktopPaths, timeout: Duration) -> DesktopResult<()> {
    let Some(identity) = resolve_identity(paths)? else {
        return Err(Box::new(
            DesktopError::new(
                error_code::PROCESS,
                "claudeDesktop.errors.launchUnavailable",
            )
            .action("openManually"),
        ));
    };
    launch_identity(&identity, timeout)
}

/// Resolve again after shutdown: an automatic Desktop update may change its executable.
pub fn restart_after_stop(paths: &DesktopPaths, after_stop: impl FnOnce() -> DesktopResult<()>) -> DesktopResult<String> {
    restart_with_after_stop(|| resolve_identity(paths), stop_identity, after_stop, launch_identity)
}

#[cfg(test)]
fn restart_with(mut resolve: impl FnMut() -> DesktopResult<Option<AppIdentity>>, mut stop: impl FnMut(&AppIdentity, Duration) -> DesktopResult<bool>, mut launch: impl FnMut(&AppIdentity, Duration) -> DesktopResult<()>) -> DesktopResult<String> {
    restart_with_after_stop(&mut resolve, &mut stop, || Ok(()), &mut launch)
}

fn restart_with_after_stop(mut resolve: impl FnMut() -> DesktopResult<Option<AppIdentity>>, mut stop: impl FnMut(&AppIdentity, Duration) -> DesktopResult<bool>, after_stop: impl FnOnce() -> DesktopResult<()>, mut launch: impl FnMut(&AppIdentity, Duration) -> DesktopResult<()>) -> DesktopResult<String> {
    let identity = resolve()?.ok_or_else(|| {
        Box::new(
            DesktopError::new(
                error_code::PROCESS,
                "claudeDesktop.errors.launchUnavailable",
            )
            .action("openManually"),
        )
    })?;
    stop(&identity, Duration::from_secs(20))?;
    after_stop()?;
    let identity = resolve()?.ok_or_else(|| process_error(error_code::PROCESS))?;
    launch(&identity, Duration::from_secs(25))?;
    Ok(identity.method.as_str().to_owned())
}

fn launch_identity(identity: &AppIdentity, timeout: Duration) -> DesktopResult<()> {
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
    if wait_until_running(identity, timeout)? {
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
    windows::candidates()
}

#[cfg(all(windows, test))]
fn process_executable(pid: u32) -> Option<PathBuf> {
    match process_image(pid) {
        ProcessImage::Running(path) => Some(path),
        _ => None,
    }
}

#[cfg(windows)]
fn process_image(pid: u32) -> ProcessImage {
    windows::image(pid)
}

#[cfg(not(windows))]
fn process_image(pid: u32) -> ProcessImage {
    if let Some(path) = process_executable(pid) { return ProcessImage::Running(path); }
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::fs::symlink_metadata(format!("/proc/{pid}")).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) { return ProcessImage::Exited; }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("ps");
        cmd.args(["-p", &pid.to_string(), "-o", "pid="]);
        if command::output(&mut cmd, Duration::from_secs(2)).is_ok_and(|out| out.status.code() == Some(1) && out.stdout.iter().all(u8::is_ascii_whitespace)) { return ProcessImage::Exited; }
    }
    ProcessImage::Unknown
}

#[cfg(target_os = "windows")]
fn signal(identity: &AppIdentity, pid: u32, force: bool) -> DesktopResult<bool> {
    windows::signal(identity, pid, force)
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

#[cfg(all(any(windows, target_os = "macos"), test))]
fn app_identity(paths: &DesktopPaths) -> Option<AppIdentity> {
    resolve_identity(paths).ok().flatten()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn resolve_identity(paths: &DesktopPaths) -> DesktopResult<Option<AppIdentity>> { Ok(app_identity(paths)) }

#[cfg(windows)]
fn resolve_identity(paths: &DesktopPaths) -> DesktopResult<Option<AppIdentity>> {
    if !process_control_allowed(paths) {
        return Ok(None);
    }
    // MSIX is only known to own the standard local-data pair. An unrelated
    // selected channel must not silently restart the global Claude package.
    if let Some(base) = dirs::data_local_dir() {
        if super::paths::paths_equal(&paths.normal_root, &base.join("Claude"))
            && super::paths::paths_equal(&paths.third_party_root, &base.join("Claude-3p"))
        {
            if let Some(identity) = msix_identity()? {
                return Ok(Some(identity));
            }
        }
    }
    // Nonpackaged installs must have their executable in the selected roots.
    for root in [&paths.normal_root, &paths.third_party_root] {
        for relative in [r"app\Claude.exe", "Claude.exe"] {
            let candidate = root.join(relative);
            if candidate.is_file() && !is_cli_path(&candidate.to_string_lossy()) {
                return Ok(Some(AppIdentity {
                    method: LaunchMethod::Executable,
                    target: candidate.clone(),
                    executable: candidate,
                }));
            }
        }
    }
    Ok(None)
}

/// Resolve both the exact package activation ID and its manifest executable.
#[cfg(target_os = "windows")]
fn msix_identity() -> DesktopResult<Option<AppIdentity>> {
    let script = "$ErrorActionPreference = 'Stop'; $p = @(Get-AppxPackage -Name 'Claude' -ErrorAction Stop | Where-Object { $_.PackageFamilyName -eq 'Claude_pzs8sxrjxfjjc' }); if ($p.Count -gt 1) { throw 'Ambiguous Desktop package' }; if ($p.Count -eq 1) { $a = @((Get-AppxPackageManifest $p[0] -ErrorAction Stop).Package.Applications.Application | Where-Object { $_.Id -eq 'Claude' }); if ($a.Count -ne 1) { throw 'Desktop entry missing or ambiguous' }; [pscustomobject]@{ appId = $p[0].PackageFamilyName + '!' + $a[0].Id; executable = Join-Path $p[0].InstallLocation $a[0].Executable } | ConvertTo-Json -Compress }";
    let mut command = crate::platform::windows::background_command("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let output = command::output(&mut command, Duration::from_secs(10)).map_err(|_| {
        Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.installationDiscoveryFailed").retryable())
    })?;
    if !output.status.success() {
        return Err(Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.installationDiscoveryFailed").retryable()));
    }
    if output.stdout.iter().all(|b| b.is_ascii_whitespace()) { return Ok(None); }
    let record: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| process_error(error_code::PROCESS))?;
    let app_id = record["appId"].as_str().ok_or_else(|| process_error(error_code::PROCESS))?;
    let executable = PathBuf::from(record["executable"].as_str().ok_or_else(|| process_error(error_code::PROCESS))?);
    if !is_msix_app_id(app_id)
        || !executable.is_absolute()
        || !executable.is_file()
        || is_cli_path(&executable.to_string_lossy())
    {
        return Err(process_error(error_code::PROCESS));
    }
    Ok(Some(AppIdentity {
        method: LaunchMethod::MsixActivation,
        target: PathBuf::from(app_id),
        executable,
    }))
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
        let mut cmd = std::process::Command::new("pgrep");
        cmd.args(["-x", name]);
        let output = command::output(&mut cmd, Duration::from_secs(2))
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
    let mut cmd = std::process::Command::new("ps");
    cmd.args(["-p", &pid.to_string(), "-o", "comm="]);
    let output = command::output(&mut cmd, Duration::from_secs(2)).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(text.trim());
    path.is_absolute().then_some(path)
}

#[cfg(unix)]
fn signal(identity: &AppIdentity, pid: u32, force: bool) -> DesktopResult<bool> {
    if pid == 0 { return Err(process_error(error_code::PROCESS)); }
    match process_image(pid) {
        ProcessImage::Exited => return Ok(false),
        ProcessImage::Running(path) if executable_matches(&path, &identity.executable) => {},
        _ => return Err(process_error(error_code::PROCESS)),
    }
    let flag = if force { "-KILL" } else { "-TERM" };
    let mut cmd = std::process::Command::new("kill");
    cmd.args([flag, &pid.to_string()]);
    let output = command::output(&mut cmd, Duration::from_secs(2))
        .map_err(|_| process_error(error_code::PROCESS))?;
    if !output.status.success() {
        if matches!(process_image(pid), ProcessImage::Exited) { return Ok(false); }
        return Err(process_error(error_code::PROCESS));
    }
    Ok(true)
}

/// macOS 的包标识。
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

#[cfg(target_os = "macos")]
fn resolve_identity(paths: &DesktopPaths) -> DesktopResult<Option<AppIdentity>> {
    // 与 Windows 同理：隔离根或显式覆盖下不得触碰用户真实安装。
    if !process_control_allowed(paths) {
        return Ok(None);
    }
    let home = dirs::home_dir().ok_or_else(|| process_error(error_code::PROCESS))?;
    let base = super::paths::macos_app_support(&home);
    if !super::paths::paths_equal(&paths.normal_root, &base.join("Claude"))
        || !super::paths::paths_equal(&paths.third_party_root, &base.join("Claude-3p"))
    {
        return Ok(None);
    }
    // 只有确实装了才提供重启：否则界面会出现一个点了必然失败的按钮。
    let Some(bundle) = macos_bundle_path(MACOS_BUNDLE_ID)? else { return Ok(None); };
    let executable = bundle.join("Contents/MacOS/Claude");
    Ok(executable.is_file().then_some(AppIdentity {
        method: LaunchMethod::MacBundle,
        target: bundle,
        executable,
    }))
}

/// 用 `mdfind` 查询包是否已安装；查询失败与未安装必须区分。
#[cfg(target_os = "macos")]
fn macos_bundle_path(bundle_id: &str) -> DesktopResult<Option<PathBuf>> {
    let mut cmd = std::process::Command::new("mdfind");
    cmd.arg(format!("kMDItemCFBundleIdentifier == {bundle_id}"));
    let output = command::output(&mut cmd, Duration::from_secs(5)).map_err(|_| {
        Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.installationDiscoveryFailed").retryable())
    })?;
    if !output.status.success() {
        return Err(process_error(error_code::PROCESS));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = entries.next() else { return Ok(None); };
    let bundle = PathBuf::from(first);
    // Multiple bundles with one ID cannot be safely selected by global ID.
    if entries.next().is_some() || !bundle.is_absolute() { return Err(process_error(error_code::PROCESS)); }
    Ok(Some(bundle))
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
fn signal(_identity: &AppIdentity, _pid: u32, _force: bool) -> DesktopResult<bool> {
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
    fn signal_failures_do_not_skip_other_processes_or_force_fallback() {
        use std::cell::RefCell;
        let live = RefCell::new(vec![10, 11]);
        let mut sent = Vec::new();
        assert!(stop_processes(Duration::ZERO, || Ok(live.borrow().clone()), |pid, force| {
            sent.push((pid, force));
            if force { live.borrow_mut().retain(|p| *p != pid); }
            // Even a force error can race with a process's successful exit.
            if pid == 10 { Err(process_error(error_code::PROCESS)) } else { Ok(true) }
        }).unwrap());
        assert_eq!(sent, [(10, false), (11, false), (10, true), (11, true)]);
    }

    #[test]
    fn a_window_appearing_during_grace_is_retried_before_force() {
        use std::cell::Cell;
        let alive = Cell::new(true);
        let attempts = Cell::new(0);
        assert!(stop_processes(Duration::from_secs(1), || Ok(if alive.get() { vec![10] } else { vec![] }), |_, force| {
            assert!(!force, "late window should get a graceful request");
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 { Ok(false) } else { alive.set(false); Ok(true) }
        }).unwrap());
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn persistent_headless_residual_uses_a_short_grace_instead_of_twenty_seconds() {
        use std::cell::Cell;
        let live = Cell::new(true);
        let mut signals = Vec::new();
        assert!(stop_processes_with_grace(Duration::from_secs(20), Duration::ZERO,
            || Ok(if live.get() { vec![10] } else { vec![] }), |pid, force| {
                signals.push((pid, force));
                if force { live.set(false); }
                Ok(force)
            }).unwrap());
        assert_eq!(signals, [(10, false), (10, true)]);
    }

    #[test]
    fn a_transient_shutdown_scan_failure_is_rechecked_before_aborting() {
        let mut scans = 0;
        let mut signals = Vec::new();
        assert!(stop_processes(Duration::from_secs(1), || {
            scans += 1;
            match scans {
                1 => Ok(vec![10]),
                2 => Err(process_error(error_code::PROCESS)),
                _ => Ok(vec![]),
            }
        }, |pid, force| { signals.push((pid, force)); Ok(true) }).unwrap());
        assert_eq!(scans, 3);
        assert_eq!(signals, [(10, false)], "an unreadable exit race must not trigger a force kill");
    }

    #[test]
    fn an_after_exit_check_retries_only_inconclusive_scans() {
        let mut scans = 0;
        stopped_after_exit(Duration::from_millis(600), Duration::ZERO, || {
            scans += 1;
            if scans == 1 { Err(process_error(error_code::PROCESS)) } else { Ok(vec![]) }
        }).unwrap();
        assert_eq!(scans, 2);

        let mut scans = 0;
        let error = stopped_after_exit(Duration::from_secs(60), Duration::from_secs(60), || {
            scans += 1;
            Ok(vec![10])
        }).unwrap_err();
        assert_eq!(scans, 1, "a confirmed new instance must block immediately");
        assert_eq!(error.message, "claudeDesktop.errors.desktopRunning");
    }

    #[test]
    fn an_after_exit_check_has_a_bounded_unknown_wait_and_preserves_discovery_errors() {
        let mut scans = 0;
        let error = stopped_after_exit(Duration::from_millis(10), Duration::from_millis(1), || {
            scans += 1;
            Err(Box::new(DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.process").param("unknown image")))
        }).unwrap_err();
        assert!(scans >= 2);
        assert_eq!(error.params, ["unknown image"]);

        let check = StopCheck { identity: None, discovery_error: Some(Box::new(
            DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.installationDiscoveryFailed")
        )), allowed: true };
        assert_eq!(check.ensure_stopped_after_exit().unwrap_err().message,
            "claudeDesktop.errors.installationDiscoveryFailed");
    }

    #[test]
    fn force_wait_rechecks_unknown_processes_then_only_signals_verified_pids() {
        use std::cell::Cell;
        let scans = Cell::new(0);
        let live = Cell::new(true);
        let mut signals = Vec::new();
        assert!(stop_processes_with_timeouts(Duration::ZERO, Duration::ZERO, Duration::from_secs(1), || {
            scans.set(scans.get() + 1);
            if scans.get() == 2 { return Err(process_error(error_code::PROCESS)); }
            Ok(if live.get() { vec![10] } else { vec![] })
        }, |pid, force| {
            signals.push((pid, force));
            if force { live.set(false); }
            Ok(force)
        }).unwrap());
        assert_eq!(scans.get(), 4);
        assert_eq!(signals, [(10, false), (10, true)]);
    }

    #[test]
    fn posting_error_does_not_use_headless_shortcut_and_force_query_errors_have_guidance() {
        let attempts = std::cell::Cell::new(0);
        let live = std::cell::Cell::new(true);
        stop_processes_with_grace(Duration::from_secs(1), Duration::ZERO,
            || Ok(if live.get() { vec![10] } else { vec![] }), |_, force| {
                assert!(!force);
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 { Err(process_error(error_code::PROCESS)) }
                else { live.set(false); Ok(true) }
            }).unwrap();
        assert_eq!(attempts.get(), 2);
        let mut scans = 0;
        let result = stop_processes_with_timeouts(Duration::ZERO, Duration::ZERO, Duration::ZERO, || {
            scans += 1;
            if scans == 1 { Ok(vec![10]) } else { Err(process_error(error_code::PROCESS)) }
        }, |_, _| Ok(false)).unwrap_err();
        assert_eq!(result.message, "claudeDesktop.errors.processStopTimeout");
        assert!(!result.params.is_empty());
    }

    #[test]
    fn discovery_failure_preserves_restart_retry_but_absence_does_not() {
        let mut state = AppState { discovery_error: Some("claudeDesktop.errors.installationDiscoveryFailed".into()), running: false, process_count: 0, launch_method: None, launch_target: None };
        assert!(state.restart_capable());
        state.discovery_error = None;
        assert!(!state.restart_capable());
    }

    #[test]
    fn failed_enumeration_blocks_forcing_and_a_discovery_error_blocks_writes() {
        let mut scans = 0;
        let result = stop_processes_with_timeouts(Duration::ZERO, Duration::ZERO, Duration::from_millis(400), || {
            scans += 1;
            if scans == 1 { Ok(vec![10]) } else { Err(process_error(error_code::PROCESS)) }
        }, |_, force| { assert!(!force); Ok(false) });
        assert!(result.is_err());
        assert!(scans >= 3, "a persistent unknown state must be rechecked without authorizing force");
        let check = StopCheck { identity: None, discovery_error: Some(process_error(error_code::PROCESS)), allowed: true };
        assert!(check.ensure_stopped().is_err());
    }

    #[test]
    fn restart_rediscovers_the_updated_executable_after_shutdown() {
        let old = std::env::temp_dir().join("old-install/Claude.exe");
        let new = std::env::temp_dir().join("new-install/Claude.exe");
        let mut scans = 0;
        let result = restart_with(|| {
            scans += 1;
            let executable = if scans == 1 { old.clone() } else { new.clone() };
            Ok(Some(AppIdentity { method: LaunchMethod::Executable, target: executable.clone(), executable }))
        }, |identity, _| { assert_eq!(identity.executable, old); Ok(true) }, |identity, _| { assert_eq!(identity.executable, new); Ok(()) });
        assert!(result.is_ok());
        assert_eq!(scans, 2);
    }

    #[test]
    fn restart_repairs_after_stop_and_never_launches_after_a_failed_step() {
        use std::cell::RefCell;
        let path = std::env::temp_dir().join("restart-test/Claude.exe");
        for failure in [None, Some("stop"), Some("repair")] {
            let order = RefCell::new(Vec::new());
            let result = restart_with_after_stop(
                || Ok(Some(AppIdentity { method: LaunchMethod::Executable, target: path.clone(), executable: path.clone() })),
                |_, _| { order.borrow_mut().push("stop"); if failure == Some("stop") { Err(process_error(error_code::PROCESS)) } else { Ok(true) } },
                || { order.borrow_mut().push("repair"); if failure == Some("repair") { Err(process_error(error_code::STORAGE)) } else { Ok(()) } },
                |_, _| { order.borrow_mut().push("launch"); Ok(()) },
            );
            assert_eq!(result.is_ok(), failure.is_none());
            assert_eq!(order.into_inner(), match failure { Some("stop") => vec!["stop"], Some("repair") => vec!["stop", "repair"], _ => vec!["stop", "repair", "launch"] });
        }
    }

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
        let result = checked_processes(
            None,
            Err(process_error(error_code::PROCESS)),
            |_| -> Option<PathBuf> { panic!("a failed enumeration must not query any PID") },
        );
        assert!(stopped_result(result).is_err());
        let result = checked_processes(None, Ok(vec![7]), |_| None);
        assert!(stopped_result(result).is_err());
    }

    #[test]
    fn an_exited_candidate_is_ignored_but_unknown_state_still_blocks_writes() {
        assert!(stopped_result(checked_processes(None, Ok(vec![7]), |_| {
            ProcessImage::Exited
        }))
        .is_ok());
        assert!(stopped_result(checked_processes(None, Ok(vec![7]), |_| {
            ProcessImage::Unknown
        }))
        .is_err());
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
        assert!(stopped_result(checked_processes(
            None,
            Ok(vec![]),
            |_| -> Option<PathBuf> { panic!("an empty enumeration must not query any PID") }
        ))
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
