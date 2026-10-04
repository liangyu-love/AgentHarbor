//! Claude Desktop 直连：固定契约常量。
//!
//! 这些取值属于本产品的磁盘契约，**一经发布不得变更**：profile 文件名、
//! `_meta.json` 条目 id、部署模式取值与鉴权方案都直接写进 Claude Desktop
//! 读取的外部文件。变更会让已应用的配置失去所有权、无法恢复。

/// AgentHarbor 在 CCD `configLibrary` 中使用的固定 profile UUID。
///
/// 与 CC Switch 的 `00000000-0000-4000-8000-000000157210` 不同，两个工具
/// 必须能同时存在于同一 `configLibrary` 而不互相覆盖。
pub const PROFILE_ID: &str = "d918233d-0a41-47a1-a29f-6d610a03722a";

/// `_meta.json` entries 中本产品条目的显示名。
pub const PROFILE_DISPLAY_NAME: &str = "AgentHarbor";

/// 应用数据目录下 Desktop 管理的子目录名。
pub const DATA_DIR_NAME: &str = "claude-desktop";

/// 写入 `claude_desktop_config.json` 的 3P 部署模式取值。
pub const DEPLOYMENT_MODE_THIRD_PARTY: &str = "3p";
/// 切回官方模式时写入的部署模式取值。
pub const DEPLOYMENT_MODE_OFFICIAL: &str = "1p";

/// 参考实现与真实主机观察到的推理供应商取值。
pub const INFERENCE_PROVIDER_GATEWAY: &str = "gateway";
/// 直连固定使用 Bearer 鉴权。
pub const INFERENCE_AUTH_SCHEME_BEARER: &str = "bearer";

/// 共享配置文件与 `_meta.json` 的文件名。
pub const CONFIG_FILE: &str = "claude_desktop_config.json";
pub const META_FILE: &str = "_meta.json";
pub const CONFIG_LIBRARY_DIR: &str = "configLibrary";

/// 受管共享配置字段：只有 mode 由本工具管理。
pub const MANAGED_CONFIG_KEYS: &[&str] = &["deploymentMode"];

/// 声明 1M 上下文所需的最小有效上下文窗口。
pub const ONE_M_CONTEXT: u64 = 1_000_000;

/// 预览 ticket 有效期（秒）。
pub const TICKET_TTL_SECONDS: i64 = 300;
/// 每个配置目标同时保留的未消费 ticket 上限。
pub const MAX_PENDING_TICKETS: usize = 8;
/// 人工恢复备份保留份数。
pub const MAX_BACKUPS: usize = 20;
/// 探测到的、其 fail-all 校验器已纳入 fable 角色的最低 Desktop 版本。
pub const FABLE_MIN_VERSION: (u64, u64, u64) = (1, 12603, 1);

/// 依据版本策略返回可用的 Desktop 角色白名单。
///
/// `mythos` 未纳入参考实现的可用菜单，不由 substring 自动放行。
pub fn roles_for(policy: super::provider::DesktopVersionPolicy) -> &'static [&'static str] {
    if policy.fable_allowed {
        &["sonnet", "opus", "haiku", "fable"]
    } else {
        &["sonnet", "opus", "haiku"]
    }
}
