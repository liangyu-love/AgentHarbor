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

pub const FILE_ROLES: [&str; 4] = [role::PROFILE, role::META, role::THIRD_PARTY_CONFIG, role::NORMAL_CONFIG];

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
    if normal == third_party {
        return Some("sameRoot".into());
    }
    // 一个根不能包含另一个根，否则四文件会互相覆盖。
    if normal.starts_with(third_party) {
        return Some("normalInsideThirdParty".into());
    }
    if third_party.starts_with(normal) {
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
                DesktopError::new(error_code::PATHS_COLLISION, "claudeDesktop.errors.pathsCollision")
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
                if path == other {
                    return Err(Box::new(
                        DesktopError::new(error_code::PATHS_COLLISION, "claudeDesktop.errors.pathCollision")
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
            if self.isolated { "isolated" } else { "production" }
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
    let base = if local.as_deref().is_some_and(|value| !value.trim().is_empty()) {
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
pub fn is_channel_name(name: &str, _third_party: bool) -> bool {
    name == "Claude"
        || name
            .strip_prefix("Claude-")
            .is_some_and(|rest| !rest.trim().is_empty())
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
    // 同索引优先（约定命名），再补齐精确目录的交叉配对。
    for (index, normal_root) in normal.iter().enumerate() {
        let third = third_party
            .get(index)
            .or_else(|| third_party.first())
            .or_else(|| Some(&normal_root));
        let Some(third) = third else { continue };
        push_pair(&mut out, normal_root.clone(), third.clone());
    }
    for normal_root in normal.iter().skip(1) {
        for third in third_party.iter().skip(1) {
            push_pair(&mut out, normal_root.clone(), third.clone());
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

    let mut candidates: Vec<PathCandidate> = Vec::new();
    let mut notes = Vec::new();
    for base in platform_bases() {
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
            let paths = DesktopPaths::from_roots(PathBuf::from(normal), PathBuf::from(third_party))?;
            Ok(with_isolation(paths, discovery_paths_isolated(discovery)))
        }
        (None, None) => {
            let paths = discovery.default.clone().ok_or_else(|| {
                Box::new(DesktopError::new(
                    error_code::PATHS_NOT_DETECTED,
                    "claudeDesktop.errors.pathsNotDetected",
                ))
            })?;
            validate_detected(&paths)?;
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

fn discovery_paths_isolated(discovery: &Discovery) -> bool {
    discovery
        .candidates
        .first()
        .map(|candidate| candidate.paths.isolated)
        .unwrap_or(false)
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