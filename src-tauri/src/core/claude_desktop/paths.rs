//! Claude Desktop 直连：安装目录发现与四文件路径。
//!
//! 目录名优先精确匹配，但仍要求安装/配置证据；多渠道时返回候选列表让用户
//! 明确选择，**不按字典序挑选 `Claude*`**，避免误选 Claude Code 或另一安装
//! 渠道。`AGENT_HARBOR_CLIENT_HOME` 生效时全部查找与读写落入隔离根，不再
//! 扫描生产 `%LOCALAPPDATA%` / 用户目录。

use std::path::{Component, Path, PathBuf};

use super::constants::{CONFIG_FILE, CONFIG_LIBRARY_DIR, META_FILE, PROFILE_ID};
use super::types::{error_code, DesktopError, DesktopResult, PathCandidateView, PathsSummary};

/// 一个安装渠道的两根配对及其导出的四文件路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPaths {
    pub normal_root: PathBuf,
    pub third_party_root: PathBuf,
    pub normal_config: PathBuf,
    pub third_party_config: PathBuf,
    pub profile_path: PathBuf,
    pub meta_path: PathBuf,
    /// 是否来自 `AGENT_HARBOR_CLIENT_HOME` 隔离根。
    pub isolated: bool,
    /// 是否由调用方显式覆盖（隔离测试）。覆盖时不得触碰本机真实安装。
    pub overridden: bool,
}

/// 四文件角色标识，用于错误定位与预览分组。
pub mod role {
    pub const NORMAL_CONFIG: &str = "normalConfig";
    pub const THIRD_PARTY_CONFIG: &str = "thirdPartyConfig";
    pub const PROFILE: &str = "profile";
    pub const META: &str = "meta";
}

pub const FILE_ROLES: [&str; 4] = [
    role::PROFILE,
    role::META,
    role::THIRD_PARTY_CONFIG,
    role::NORMAL_CONFIG,
];

pub fn isolated_home() -> Option<PathBuf> {
    std::env::var_os("AGENT_HARBOR_CLIENT_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// 把路径规范化成可以比较身份的形式：解析 `..`、去掉多余的 `.` 与末尾分隔符。
///
/// 不解析符号链接：链接目标在预览后变化必须被当作 revision 变化处理，而不是
/// 在这里被折叠掉。
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn roots_identity_conflict(normal: &Path, third_party: &Path) -> Option<String> {
    if paths_equal(normal, third_party) {
        return Some("sameRoot".into());
    }
    // 一个根不能包含另一个根，否则四文件会互相覆盖。
    if path_is_within(normal, third_party) {
        return Some("normalInsideThirdParty".into());
    }
    if path_is_within(third_party, normal) {
        return Some("thirdPartyInsideNormal".into());
    }
    None
}

impl DesktopPaths {
    /// 显式两根构造。四条目标路径必须互不相同，异常时调用方零写入。
    pub fn from_roots(normal: PathBuf, third_party: PathBuf) -> DesktopResult<Self> {
        let normal_root = normalize(&normal);
        let third_party_root = normalize(&third_party);
        if let Some(reason) = roots_identity_conflict(&normal_root, &third_party_root) {
            return Err(Box::new(
                DesktopError::new(
                    error_code::PATHS_COLLISION,
                    "claudeDesktop.errors.pathsCollision",
                )
                .param(reason),
            ));
        }
        let config_library = third_party_root.join(CONFIG_LIBRARY_DIR);
        let paths = Self {
            normal_config: normal_root.join(CONFIG_FILE),
            third_party_config: third_party_root.join(CONFIG_FILE),
            profile_path: config_library.join(format!("{PROFILE_ID}.json")),
            meta_path: config_library.join(META_FILE),
            normal_root,
            third_party_root,
            isolated: false,
            overridden: false,
        };
        paths.ensure_distinct()?;
        Ok(paths)
    }

    fn ensure_distinct(&self) -> DesktopResult<()> {
        let targets = [
            (&self.normal_config, role::NORMAL_CONFIG),
            (&self.third_party_config, role::THIRD_PARTY_CONFIG),
            (&self.profile_path, role::PROFILE),
            (&self.meta_path, role::META),
        ];
        for (index, (path, file_role)) in targets.iter().enumerate() {
            for (other, other_role) in targets.iter().skip(index + 1) {
                if paths_equal(path, other) {
                    return Err(Box::new(
                        DesktopError::new(
                            error_code::PATHS_COLLISION,
                            "claudeDesktop.errors.pathCollision",
                        )
                        .file(file_role)
                        .param(*other_role),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn summary(&self) -> PathsSummary {
        PathsSummary {
            normal_root: display(&self.normal_root),
            third_party_root: display(&self.third_party_root),
            normal_config: display(&self.normal_config),
            third_party_config: display(&self.third_party_config),
            profile_path: display(&self.profile_path),
            meta_path: display(&self.meta_path),
            isolated: self.isolated,
        }
    }

    /// 路径身份指纹：预览 ticket 用它检测预览后重定向/换目录。
    pub fn identity(&self) -> String {
        format!(
            "{}|{}|{}",
            display(&self.normal_root),
            display(&self.third_party_root),
            if self.isolated {
                "isolated"
            } else {
                "production"
            }
        )
    }

    pub fn mutating_roots(&self) -> [&Path; 2] {
        [&self.normal_root, &self.third_party_root]
    }

    /// normal root 是否具备安装证据：目录存在，或已存在配置文件。
    pub fn normal_has_install_evidence(&self) -> bool {
        self.normal_root.is_dir() || self.normal_config.is_file()
    }
}

pub fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 三段数字版本（可带预发布后缀），用于判断版本探测是否真的成功。
pub fn is_version_like(text: &str) -> bool {
    let core = text.trim().split(['-', '+']).next().unwrap_or_default();
    if core.split('.').count() < 3 {
        return false;
    }
    core.split('.').all(|part| {
        !part.is_empty() && part.len() <= 6 && part.chars().all(|ch| ch.is_ascii_digit())
    })
}

/// 一个可选的安装渠道。
#[derive(Debug, Clone)]
pub struct PathCandidate {
    pub paths: DesktopPaths,
    pub install_evidence: bool,
}

impl PathCandidate {
    pub fn id(&self) -> String {
        format!(
            "{}|{}",
            display(&self.paths.normal_root),
            display(&self.paths.third_party_root)
        )
    }

    pub fn view(&self) -> PathCandidateView {
        PathCandidateView {
            id: self.id(),
            normal_root: display(&self.paths.normal_root),
            third_party_root: display(&self.paths.third_party_root),
            normal_exists: self.paths.normal_root.is_dir(),
            third_party_exists: self.paths.third_party_root.is_dir(),
            has_install_evidence: self.install_evidence,
        }
    }
}

/// 发现结果：候选列表 + 无歧义时的默认项。
#[derive(Debug, Clone)]
pub struct Discovery {
    pub candidates: Vec<PathCandidate>,
    pub default: Option<DesktopPaths>,
    pub notes: Vec<String>,
}

fn with_isolation(mut paths: DesktopPaths, isolated: bool) -> DesktopPaths {
    paths.isolated = isolated;
    paths.overridden = isolated;
    paths
}

/// 隔离根：直接使用 `<home>/Claude` 与 `<home>/Claude-3p`，不再扫描生产目录。
fn isolated_candidates(home: &Path) -> Vec<DesktopPaths> {
    let normal = home.join("Claude");
    let third_party = home.join("Claude-3p");
    DesktopPaths::from_roots(normal, third_party)
        .map(|paths| with_isolation(paths, true))
        .into_iter()
        .collect()
}

/// 精确目录名匹配的候选。
#[cfg(windows)]
fn platform_bases() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let local = std::env::var("LOCALAPPDATA").ok();
    let base = windows_local_app_data(local.as_deref(), &home);
    // 环境变量不可信时优先使用平台 API 的取值。
    let base = if local
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        base
    } else {
        dirs::data_local_dir().unwrap_or(base)
    };
    vec![base]
}

#[cfg(target_os = "macos")]
fn platform_bases() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| macos_app_support(&home))
        .into_iter()
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_bases() -> Vec<PathBuf> {
    // Flatpak 的沙箱内 XDG_CONFIG_HOME 是私有路径，宿主自定义取值不可靠，
    // 因此非绝对取值一律回落到宿主默认的 ~/.config。
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    vec![config_dir_from_xdg(&home, xdg.as_deref())]
}

#[cfg(not(any(windows, unix)))]
fn platform_bases() -> Vec<PathBuf> {
    Vec::new()
}

/// 精确目录名优先，其次才是同渠道的其它命名。
pub fn channel_dir_names(names: &[String], third_party: bool) -> Vec<String> {
    let exact = if third_party { "Claude-3p" } else { "Claude" };
    let mut found: Vec<String> = Vec::new();
    if names.iter().any(|name| name == exact) {
        found.push(exact.to_owned());
    }
    let mut scanned: Vec<String> = names
        .iter()
        .filter(|name| is_channel_name(name, third_party))
        .cloned()
        .collect();
    // 排序只让同一次发现的输出稳定，**不**决定选哪一个。
    scanned.sort();
    for name in scanned {
        if !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// 目录名是否可能是 Desktop 的配置根。
///
/// 只做**名字**筛选：`Claude` 或 `Claude-<非空后缀>`。`Claude Code`
/// （空格分隔，是 Claude Code 的数据目录）因此被排除。
///
/// 具体属于哪个渠道、以及是否真的是一个安装，由 `channel_has_evidence`
/// 另行判定——名字相近但为空的目录（真实机器上存在 `Claude-Data`）不能
/// 仅凭名字成为候选，否则会把本来无歧义的安装变成「必须让用户选」。
pub fn is_channel_name(name: &str, third_party: bool) -> bool {
    let is_third = name == "Claude-3p"
        || name
            .strip_prefix("Claude-3p-")
            .is_some_and(|rest| !rest.trim().is_empty())
        || name
            .strip_prefix("Claude-")
            .and_then(|rest| rest.strip_suffix("-3p"))
            .is_some_and(|rest| !rest.trim().is_empty());
    if third_party {
        is_third
    } else {
        name == "Claude"
            || name
                .strip_prefix("Claude-")
                .is_some_and(|rest| !rest.trim().is_empty() && !is_third)
    }
}

/// 精确名优先，其次才是同渠道的其它命名。
///
/// 除渠道名之外还要求**安装痕迹**：只有名字是不够的。真实机器上存在
/// `Claude-Data` 这类同前缀的空数据目录，按名字收下会让发现结果凭空多出
/// 一个候选，并把本来无歧义的安装变成「必须让用户选」。
fn channel_candidates(base: &Path, third_party: bool) -> Vec<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(base)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    channel_dir_names(&names, third_party)
        .into_iter()
        .map(|name| base.join(name))
        .filter(|path| channel_has_evidence(path, third_party))
        .collect()
}

/// 某个候选根是否有该渠道的安装痕迹。
///
/// - 常规渠道：存在 `claude_desktop_config.json` 或 `logs` 目录。
/// - 3P 渠道：存在 `configLibrary`（3P 部署才会创建它）。
pub fn channel_has_evidence(root: &Path, third_party: bool) -> bool {
    if third_party {
        return root.join(CONFIG_LIBRARY_DIR).is_dir();
    }
    root.join(CONFIG_FILE).is_file() || root.join("logs").is_dir()
}

/// Linux/XDG：只有非空且绝对的 `XDG_CONFIG_HOME` 才被采用，否则回落 `~/.config`。
pub fn config_dir_from_xdg(home: &Path, xdg: Option<&str>) -> PathBuf {
    xdg.filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

/// macOS 的 Application Support 根。
pub fn macos_app_support(home: &Path) -> PathBuf {
    home.join("Library").join("Application Support")
}

/// Windows 的 LOCALAPPDATA 根；环境变量缺失或为空时回落 `~/AppData/Local`。
pub fn windows_local_app_data(local_app_data: Option<&str>, home: &Path) -> PathBuf {
    local_app_data
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"))
}

fn pair_candidates(normal: &[PathBuf], third_party: &[PathBuf]) -> Vec<PathCandidate> {
    let mut out: Vec<PathCandidate> = Vec::new();
    // 同渠道按名字配对，不按两个扫描列表的索引交叉配对。首次官方安装
    // 尚没有 3P 根时，保留已确认 normal 根对应的待创建路径。
    for normal_root in normal {
        let Some(name) = normal_root.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix("Claude") else {
            continue;
        };
        let expected = normal_root.with_file_name(format!("{name}-3p"));
        let alternate = normal_root.with_file_name(format!("Claude-3p{suffix}"));
        let matches = third_party
            .iter()
            .filter(|candidate| {
                paths_equal(candidate, &expected) || paths_equal(candidate, &alternate)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            push_pair(&mut out, normal_root.clone(), expected);
        } else {
            for third in matches {
                push_pair(&mut out, normal_root.clone(), third.clone());
            }
        }
    }
    out
}

fn push_pair(out: &mut Vec<PathCandidate>, normal: PathBuf, third_party: PathBuf) {
    let Ok(paths) = DesktopPaths::from_roots(normal, third_party) else {
        return;
    };
    let install_evidence = paths.normal_has_install_evidence();
    if out.iter().any(|candidate| candidate.paths == paths) {
        return;
    }
    out.push(PathCandidate {
        paths,
        install_evidence,
    });
}

/// 执行路径发现。
pub fn discover() -> Discovery {
    if let Some(home) = isolated_home() {
        let candidates = isolated_candidates(&home)
            .into_iter()
            .map(|paths| PathCandidate {
                install_evidence: paths.normal_has_install_evidence(),
                paths,
            })
            .collect::<Vec<_>>();
        let default = candidates.first().map(|candidate| candidate.paths.clone());
        return Discovery {
            candidates,
            default,
            notes: vec!["claudeDesktop.notes.isolated".into()],
        };
    }

    discover_in_bases(&platform_bases())
}

fn discover_in_bases(bases: &[PathBuf]) -> Discovery {
    let mut candidates: Vec<PathCandidate> = Vec::new();
    let mut notes = Vec::new();
    for base in bases {
        let normal = channel_candidates(&base, false);
        let third_party = channel_candidates(&base, true);
        if normal.is_empty() && third_party.is_empty() {
            continue;
        }
        candidates.extend(pair_candidates(&normal, &third_party));
    }

    // 多候选且互相冲突时应由用户选择；只有一个候选时才给出默认值。
    let with_evidence: Vec<&PathCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.install_evidence)
        .collect();
    let default = match with_evidence.len() {
        0 => None,
        1 => Some(with_evidence[0].paths.clone()),
        _ => {
            notes.push("claudeDesktop.notes.multipleChannels".into());
            None
        }
    };
    if candidates.is_empty() {
        notes.push("claudeDesktop.notes.notDetected".into());
    }
    Discovery {
        candidates,
        default,
        notes,
    }
}

/// 依据前端选择解析出确定路径。
pub fn resolve(choice: &super::types::PathsChoice) -> DesktopResult<DesktopPaths> {
    resolve_in(&discover(), choice)
}

/// 与 [`resolve`] 相同，但复用一次已经完成的发现结果。
pub fn resolve_in(
    discovery: &Discovery,
    choice: &super::types::PathsChoice,
) -> DesktopResult<DesktopPaths> {
    match (&choice.normal_root, &choice.third_party_root) {
        (Some(normal), Some(third_party)) => {
            if normal.trim().is_empty() || third_party.trim().is_empty() {
                return Err(Box::new(DesktopError::new(
                    error_code::PATHS_NOT_DETECTED,
                    "claudeDesktop.errors.pathsNotDetected",
                )));
            }
            let requested =
                DesktopPaths::from_roots(PathBuf::from(normal), PathBuf::from(third_party))?;
            if !requested.normal_root.is_absolute() || !requested.third_party_root.is_absolute() {
                return Err(paths_not_detected());
            }
            // 两根 DTO 只是对后端候选的选择，绝不能成为任意文件写入接口。
            let paths = discovery
                .candidates
                .iter()
                .find(|candidate| {
                    paths_equal(&candidate.paths.normal_root, &requested.normal_root)
                        && paths_equal(
                            &candidate.paths.third_party_root,
                            &requested.third_party_root,
                        )
                })
                .map(|candidate| candidate.paths.clone())
                .ok_or_else(paths_not_detected)?;
            validate_selection(discovery, &paths)?;
            Ok(paths)
        }
        (None, None) => {
            let paths = discovery.default.clone().ok_or_else(|| {
                Box::new(DesktopError::new(
                    error_code::PATHS_NOT_DETECTED,
                    "claudeDesktop.errors.pathsNotDetected",
                ))
            })?;
            validate_selection(discovery, &paths)?;
            Ok(paths)
        }
        _ => Err(Box::new(
            DesktopError::new(
                error_code::PATHS_NOT_DETECTED,
                "claudeDesktop.errors.pathsIncomplete",
            )
            .action("selectPaths"),
        )),
    }
}

fn paths_not_detected() -> Box<DesktopError> {
    Box::new(
        DesktopError::new(
            error_code::PATHS_NOT_DETECTED,
            "claudeDesktop.errors.pathsNotDetected",
        )
        .action("selectPaths"),
    )
}

fn path_identity(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let normalized = normalize(path).to_string_lossy().to_ascii_lowercase();
        // canonicalize uses extended Windows paths; user-selected roots may not.
        let normalized = if let Some(unc) = normalized.strip_prefix(r"\\?\unc\") {
            format!(r"\\{unc}")
        } else {
            normalized.trim_start_matches(r"\\?\").to_owned()
        };
        PathBuf::from(normalized)
    }
    #[cfg(not(windows))]
    {
        normalize(path)
    }
}

pub(super) fn paths_equal(left: &Path, right: &Path) -> bool {
    path_identity(left) == path_identity(right)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    path_identity(path).starts_with(path_identity(root))
}

/// Resolve existing ancestors too: the 3P/configLibrary directories may not have
/// been created yet, but a junction/symlink must not redirect isolated writes.
fn resolved_location(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut tail = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(ancestor).ok()?;
                for name in tail.iter().rev() {
                    resolved.push(name);
                }
                return Some(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        tail.push(ancestor.file_name()?.to_owned());
        ancestor = ancestor.parent()?;
    }
}

/// A saved session cannot be reused across production/isolated client homes.
/// Explicit Manager test overrides must be checked by that Manager separately.
pub fn validate_session_paths(paths: &DesktopPaths) -> DesktopResult<()> {
    if !paths.normal_root.is_absolute() || !paths.third_party_root.is_absolute() {
        return Err(paths_not_detected());
    }
    let normal_location = resolved_location(&paths.normal_root).ok_or_else(paths_not_detected)?;
    let third_party_location =
        resolved_location(&paths.third_party_root).ok_or_else(paths_not_detected)?;
    if let Some(reason) = roots_identity_conflict(&normal_location, &third_party_location) {
        return Err(Box::new(
            DesktopError::new(
                error_code::PATHS_COLLISION,
                "claudeDesktop.errors.pathsCollision",
            )
            .param(reason),
        ));
    }
    // A configuration file or configLibrary link must stay in its selected root.
    // Otherwise two nominally distinct roles can write the same physical target.
    for (target, root) in [
        (&paths.normal_config, &normal_location),
        (&paths.third_party_config, &third_party_location),
        (&paths.profile_path, &third_party_location),
        (&paths.meta_path, &third_party_location),
    ] {
        if !resolved_location(target).is_some_and(|target| path_is_within(&target, root)) {
            return Err(paths_not_detected());
        }
    }
    if let Some(home) = isolated_home() {
        let expected = isolated_candidates(&home)
            .into_iter()
            .next()
            .ok_or_else(paths_not_detected)?;
        if !paths.isolated
            || !paths_equal(&paths.normal_root, &expected.normal_root)
            || !paths_equal(&paths.third_party_root, &expected.third_party_root)
        {
            return Err(paths_not_detected());
        }
        let canonical_home = resolved_location(&home).ok_or_else(paths_not_detected)?;
        for target in [
            &paths.normal_root,
            &paths.third_party_root,
            &paths.normal_config,
            &paths.third_party_config,
            &paths.profile_path,
            &paths.meta_path,
        ] {
            if !resolved_location(target)
                .is_some_and(|target| path_is_within(&target, &canonical_home))
            {
                return Err(paths_not_detected());
            }
        }
    } else if paths.isolated {
        return Err(paths_not_detected());
    }
    Ok(())
}

fn validate_selection(discovery: &Discovery, paths: &DesktopPaths) -> DesktopResult<()> {
    validate_session_paths(paths)?;
    if !discovery
        .candidates
        .iter()
        .any(|candidate| candidate.paths == *paths)
    {
        return Err(paths_not_detected());
    }
    validate_detected(paths)
}

/// 3P 目录缺失但 normal root 已确认时允许创建；normal root 未确认时返回
/// `notDetected`，不凭空创建“已配置”的假安装。
pub fn validate_detected(paths: &DesktopPaths) -> DesktopResult<()> {
    if !paths.normal_has_install_evidence() {
        return Err(Box::new(
            DesktopError::new(
                error_code::PATHS_NOT_DETECTED,
                "claudeDesktop.errors.pathsNotDetected",
            )
            .action("selectPaths"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::PathsChoice;
    use super::*;

    struct HomeGuard(Option<std::ffi::OsString>);
    impl HomeGuard {
        fn set(home: Option<&Path>) -> Self {
            let previous = std::env::var_os("AGENT_HARBOR_CLIENT_HOME");
            match home {
                Some(home) => std::env::set_var("AGENT_HARBOR_CLIENT_HOME", home),
                None => std::env::remove_var("AGENT_HARBOR_CLIENT_HOME"),
            }
            Self(previous)
        }
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(home) => std::env::set_var("AGENT_HARBOR_CLIENT_HOME", home),
                None => std::env::remove_var("AGENT_HARBOR_CLIENT_HOME"),
            }
        }
    }

    fn temp_base() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "agentharbor-path-selection-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn choice(paths: &DesktopPaths) -> PathsChoice {
        PathsChoice {
            normal_root: Some(display(&paths.normal_root)),
            third_party_root: Some(display(&paths.third_party_root)),
        }
    }

    #[cfg(any(windows, unix))]
    fn directory_link(target: &Path, link: &Path) {
        #[cfg(windows)]
        {
            let output = crate::platform::windows::background_command("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(output.status.success(), "create temporary test junction");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_roots_cannot_hide_collisions_with_case_or_extended_prefixes() {
        assert!(DesktopPaths::from_roots(
            PathBuf::from(r"C:\temp\Claude"),
            PathBuf::from(r"c:\TEMP\claude"),
        )
        .is_err());
        assert!(DesktopPaths::from_roots(
            PathBuf::from(r"C:\temp\Claude"),
            PathBuf::from(r"\\?\c:\TEMP\claude\child"),
        )
        .is_err());
        assert!(paths_equal(
            Path::new(r"\\?\UNC\server\share\Claude"),
            Path::new(r"\\server\share\claude"),
        ));
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn directory_aliases_cannot_make_two_roots_share_configuration() {
        let _lock = super::super::env_test_lock();
        let _home = HomeGuard::set(None);
        let base = tempfile::tempdir().unwrap();
        let normal = base.path().join("Claude");
        let third = base.path().join("Claude-3p");
        std::fs::create_dir_all(normal.join("logs")).unwrap();
        std::fs::create_dir_all(normal.join(CONFIG_LIBRARY_DIR)).unwrap();
        directory_link(&normal, &third);
        let paths = DesktopPaths::from_roots(normal, third).unwrap();
        assert_eq!(
            validate_session_paths(&paths).unwrap_err().code,
            error_code::PATHS_COLLISION
        );
        assert!(resolve_in(
            &discover_in_bases(&[base.path().to_owned()]),
            &PathsChoice::default()
        )
        .is_err());
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn a_config_library_link_cannot_redirect_an_isolated_session() {
        let _lock = super::super::env_test_lock();
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join("Claude")).unwrap();
        std::fs::create_dir_all(base.path().join("Claude-3p")).unwrap();
        directory_link(
            outside.path(),
            &base.path().join("Claude-3p").join(CONFIG_LIBRARY_DIR),
        );
        let _home = HomeGuard::set(Some(base.path()));
        assert!(resolve_in(&discover(), &PathsChoice::default()).is_err());
    }

    #[test]
    fn first_official_installation_derives_a_missing_third_party_root() {
        let _lock = super::super::env_test_lock();
        let _home = HomeGuard::set(None);
        let base = temp_base();
        std::fs::create_dir_all(base.join("Claude/logs")).unwrap();
        let discovery = discover_in_bases(&[base.clone()]);
        assert_eq!(discovery.candidates.len(), 1);
        let paths = resolve_in(&discovery, &PathsChoice::default()).unwrap();
        assert_eq!(paths.normal_root, base.join("Claude"));
        assert_eq!(paths.third_party_root, base.join("Claude-3p"));
        assert!(
            !paths.third_party_root.exists(),
            "discovery must not create configuration"
        );
    }

    #[test]
    fn channels_pair_by_identity_without_cross_pairing_or_treating_3p_as_normal() {
        let base = temp_base();
        for root in ["Claude", "Claude-preview"] {
            std::fs::create_dir_all(base.join(root).join("logs")).unwrap();
        }
        std::fs::create_dir_all(base.join("Claude-3p/logs")).unwrap();
        std::fs::create_dir_all(base.join("Claude-3p/configLibrary")).unwrap();
        std::fs::create_dir_all(base.join("Claude-preview-3p/configLibrary")).unwrap();
        let discovery = discover_in_bases(&[base.clone()]);
        assert_eq!(discovery.candidates.len(), 2);
        assert!(discovery.default.is_none());
        for candidate in &discovery.candidates {
            let name = candidate
                .paths
                .normal_root
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(
                candidate.paths.third_party_root,
                base.join(format!("{name}-3p"))
            );
        }
    }

    #[test]
    fn explicit_paths_must_select_a_backend_candidate() {
        let _lock = super::super::env_test_lock();
        let _home = HomeGuard::set(None);
        let base = temp_base();
        std::fs::create_dir_all(base.join("Claude/logs")).unwrap();
        let discovery = discover_in_bases(&[base.clone()]);
        let allowed = discovery.default.as_ref().unwrap();
        assert_eq!(resolve_in(&discovery, &choice(allowed)).unwrap(), *allowed);
        let foreign =
            DesktopPaths::from_roots(base.join("outside/Claude"), base.join("outside/Claude-3p"))
                .unwrap();
        assert!(resolve_in(&discovery, &choice(&foreign)).is_err());
        let relative = PathsChoice {
            normal_root: Some("Claude".into()),
            third_party_root: Some("Claude-3p".into()),
        };
        assert!(resolve_in(&discovery, &relative).is_err());
    }

    #[test]
    fn explicit_paths_keep_the_backend_isolation_provenance_and_reject_escape() {
        let _lock = super::super::env_test_lock();
        let base = temp_base();
        std::fs::create_dir_all(base.join("Claude")).unwrap();
        let _home = HomeGuard::set(Some(&base));
        let discovery = discover();
        let allowed = discovery.default.as_ref().unwrap();
        let selected = resolve_in(&discovery, &choice(allowed)).unwrap();
        assert!(selected.isolated && selected.overridden);
        let foreign =
            DesktopPaths::from_roots(base.join("outside/Claude"), base.join("outside/Claude-3p"))
                .unwrap();
        assert!(resolve_in(&discovery, &choice(&foreign)).is_err());
        assert!(validate_session_paths(&foreign).is_err());
    }

    #[test]
    fn saved_isolated_sessions_cannot_be_reused_in_production() {
        let _lock = super::super::env_test_lock();
        let _home = HomeGuard::set(None);
        let base = temp_base();
        let mut paths =
            DesktopPaths::from_roots(base.join("Claude"), base.join("Claude-3p")).unwrap();
        paths.isolated = true;
        paths.overridden = true;
        assert!(validate_session_paths(&paths).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn isolated_symlink_roots_cannot_redirect_writes_outside_the_home() {
        let _lock = super::super::env_test_lock();
        let base = temp_base();
        let outside = temp_base();
        std::os::unix::fs::symlink(&outside, base.join("Claude")).unwrap();
        let _home = HomeGuard::set(Some(&base));
        assert!(resolve_in(&discover(), &PathsChoice::default()).is_err());
    }
}
