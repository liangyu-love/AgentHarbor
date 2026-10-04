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
    let marker = "claude-code\\";
    let trailing = "claude-code";
    // 匹配 `...claude-code...` 与以 `...claude-code` 结尾的路径。
    normalized.contains(marker)
        || normalized.ends_with(trailing)
        || normalized.contains(&format!("{marker}"))
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

/// 只接受本项目已核实的包族名与 `!Claude` 应用 id。
pub fn is_msix_app_id(text: &str) -> bool {
    let Some((family, app)) = text.trim().split_once('!') else {
        return false;
    };
    family.starts_with(PACKAGE_FAMILY) && app == "Claude"
}

// ---------------------------------------------------------------------------
// 状态与操作
// ---------------------------------------------------------------------------

/// 读取 Desktop 应用状态。发现失败不报错：只是没有重启能力。
pub fn state(paths: &DesktopPaths) -> AppState {
    let (launch_method, launch_target) = launch_plan(paths);
    let processes = list_processes();
    AppState {
        running: !processes.is_empty(),
        process_count: processes.len(),
        launch_method,
        launch_target,
    }
}

/// 停止 Desktop，返回是否确实停到了进程。
pub fn stop(timeout: Duration) -> DesktopResult<bool> {
    let processes = list_processes();
    if processes.is_empty() {
        return Ok(false);
    }
    for pid in &processes {
        signal(*pid, false)?;
    }
    if wait_until_stopped(timeout) {
        return Ok(true);
    }
    // 优雅退出没完成时强制结束，避免半退出状态下写文件被回写覆盖。
    for pid in &processes {
        let _ = signal(*pid, true);
    }
    if wait_until_stopped(Duration::from_secs(5)) {
        return Ok(true);
    }
    Err(Box::new(
        DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.processStopTimeout")
            .retryable(),
    ))
}

/// 启动 Desktop 并等待主进程出现。
pub fn launch(paths: &DesktopPaths, timeout: Duration) -> DesktopResult<()> {
    let (method, target) = launch_plan(paths);
    let Some(method) = method else {
        return Err(Box::new(
            DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.launchUnavailable")
                .action("openManually"),
        ));
    };
    let target = target.ok_or_else(|| process_error(error_code::PROCESS))?;
    let spawned = match method {
        LaunchMethod::MsixActivation => msix_activate(&target),
        LaunchMethod::Executable => cmd_spawn(&target, &[]),
        LaunchMethod::MacBundle => cmd_spawn(Path::new("open"), &["-b", &target.to_string_lossy()]),
        LaunchMethod::LinuxDesktop => cmd_spawn(Path::new("gtk-launch"), &[&target.to_string_lossy()]),
    };
    if let Err(detail) = spawned {
        return Err(Box::new(
            DesktopError::new(error_code::PROCESS, "claudeDesktop.errors.launchFailed")
                .param(detail)
                .retryable()
                .action("openManually"),
        ));
    }
    if wait_until_running(timeout) {
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
fn list_processes() -> Vec<u32> {
    let filter = format!("IMAGENAME eq {}.exe", PROCESS_STEM);
    let Ok(output) = crate::platform::windows::background_command("tasklist")
        .args(["/FO", "CSV", "/NH", "/FI", &filter])
        .output()
    else {
        return Vec::new();
    };
    parse_tasklist_csv(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .filter(|(_, name)| is_desktop_process_name(name.trim_end_matches(".exe")))
        .map(|(pid, _)| pid)
        .collect()
}

#[cfg(target_os = "windows")]
fn signal(pid: u32, force: bool) -> DesktopResult<()> {
    let mut command = crate::platform::windows::background_command("taskkill");
    command.args(["/PID", &pid.to_string()]);
    if force {
        command.args(["/F", "/T"]);
    }
    command.output().map_err(|_| process_error(error_code::PROCESS))?;
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
fn launch_plan(paths: &DesktopPaths) -> (Option<LaunchMethod>, Option<PathBuf>) {
    // 打包安装优先：直接启动包内的可执行文件会丢失包身份并启动失败。
    //
    // 但**只在管理真实安装时**才查询 MSIX：隔离根或显式覆盖（测试、隔离运行）
    // 下必须报「不可重启」，否则会去启动用户真正的 Claude Desktop。
    if !paths.isolated && !paths.overridden {
        if let Some(app_id) = msix_app_user_model_id() {
            return (Some(LaunchMethod::MsixActivation), Some(PathBuf::from(app_id)));
        }
    }
    for root in [&paths.normal_root, &paths.third_party_root] {
        for relative in [format!("app\\Claude.exe"), "Claude.exe".to_owned()] {
            let candidate = root.join(relative);
            if candidate.is_file() {
                return (Some(LaunchMethod::Executable), Some(candidate));
            }
        }
    }
    (None, None)
}

/// 查询已安装的 Desktop MSIX 包并返回其包激活 id（AUMID）。
#[cfg(target_os = "windows")]
fn msix_app_user_model_id() -> Option<String> {
    let script = "$p = Get-AppxPackage -Name 'Claude' -ErrorAction SilentlyContinue | Sort-Object Version -Descending | Select-Object -First 1; if ($p) { $p.InstallLocation; (Get-AppxPackageManifest $p).Package.Applications.Application | Where-Object { $_.Id -eq 'Claude' } | ForEach-Object { $p.PackageFamilyName + '!' + $_.Id } }";
    let output = crate::platform::windows::background_command("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .find(|line| is_msix_app_id(line))
        .map(|line| (*line).to_owned())
}


#[cfg(unix)]
fn list_processes() -> Vec<u32> {
    let Ok(output) = std::process::Command::new("pgrep")
        .args(["-x", PROCESS_STEM])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
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
fn launch_plan(paths: &DesktopPaths) -> (Option<LaunchMethod>, Option<PathBuf>) {
    // 与 Windows 同理：隔离根或显式覆盖下不得触碰用户真实安装。
    if paths.isolated || paths.overridden {
        return (None, None);
    }
    // 只有确实装了才提供重启：否则界面会出现一个点了必然失败的按钮。
    if !macos_bundle_installed(MACOS_BUNDLE_ID) {
        return (None, None);
    }
    (
        Some(LaunchMethod::MacBundle),
        Some(PathBuf::from(MACOS_BUNDLE_ID)),
    )
}

/// 用 `mdfind` 查询包是否已安装；查询失败时保守地认为未确认。
#[cfg(target_os = "macos")]
fn macos_bundle_installed(bundle_id: &str) -> bool {
    std::process::Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == {bundle_id}"))
        .output()
        .map(|output| {
            output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        })
        .unwrap_or(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch_plan(paths: &DesktopPaths) -> (Option<LaunchMethod>, Option<PathBuf>) {
    if paths.isolated || paths.overridden {
        return (None, None);
    }
    // 配置根下不会有可执行文件；真正的启动入口是桌面文件或 PATH 上的可执行文件。
    for entry in [
        "claude-desktop.desktop",
        "com.anthropic.claudefordesktop.desktop",
    ] {
        for base in [
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
            dirs::data_dir().unwrap_or_default().join("applications"),
        ] {
            let candidate = base.join(entry);
            if candidate.is_file() {
                return (Some(LaunchMethod::LinuxDesktop), Some(candidate));
            }
        }
    }
    for name in ["claude-desktop", "claude"] {
        if which(name).is_some() {
            return (Some(LaunchMethod::Executable), Some(PathBuf::from(name)));
        }
    }
    (None, None)
}

/// 在 `PATH` 上查找可执行文件。
#[cfg(all(unix, not(target_os = "macos")))]
fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(not(any(windows, unix)))]
fn list_processes() -> Vec<u32> {
    Vec::new()
}

#[cfg(not(any(windows, unix)))]
fn signal(_pid: u32, _force: bool) -> DesktopResult<()> {
    Err(process_error(error_code::PROCESS))
}

#[cfg(not(any(windows, unix)))]
fn launch_plan(_paths: &DesktopPaths) -> (Option<LaunchMethod>, Option<PathBuf>) {
    (None, None)
}

fn cmd_spawn(program: &Path, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn wait_until_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if list_processes().is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    list_processes().is_empty()
}

fn wait_until_running(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !list_processes().is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    !list_processes().is_empty()
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
            vec![(1234, "Claude.exe".to_owned()), (5678, "Claude.exe".to_owned())]
        );
    }

    #[test]
    fn only_the_verified_package_family_and_app_id_are_accepted() {
        assert!(is_msix_app_id("Claude_pzs8sxrjxfjjc!Claude"));
        // 版本化的完整包名不是 AUMID。
        assert!(!is_msix_app_id("Claude_2.19675.0.0_x64__pzs8sxrjxfjjc!Claude"));
        // 同一包里的其它应用不是桌面主程序。
        assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjc!SshProxy"));
        assert!(!is_msix_app_id("Other_pzs8sxrjxfjjc!Claude"));
        assert!(!is_msix_app_id("Claude_pzs8sxrjxfjjc"));
    }

    /// 隔离根不能拿到启动能力：否则会把用户真实的 Desktop 拉起来。
    #[test]
    fn an_isolated_root_never_launches_the_production_app() {
        // `launch_plan` 会调用包查询；与设置隔离根的测试共用锁，避免互相干扰。
        let _guard = crate::core::claude_desktop::env_test_lock();
        let base = std::env::temp_dir().join(format!("agentharbor-iso-{}", uuid::Uuid::new_v4()));
        let mut paths =
            DesktopPaths::from_roots(base.join("Claude"), base.join("Claude-3p")).unwrap();
        assert!(!paths.isolated);
        // 隔离根必须一律拒绝，即使本机确实装了 Desktop。
        paths.isolated = true;
        assert_eq!(launch_plan(&paths), (None, None));
    }
}
