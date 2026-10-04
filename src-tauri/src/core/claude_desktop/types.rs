//! Claude Desktop 直连：请求、视图与错误类型。
//!
//! 这里的类型只描述 Desktop 独立配置能力，**不复用** `ClientConfigRequest`
//! 或 `TargetId`：Desktop 是四文件事务，且不需要进程重启能力。

use serde::{Deserialize, Serialize};

/// 展示名长度上限（字符），足够表达“中转 · Opus”而不至于撑破菜单。
pub const MAX_LABEL_CHARS: usize = 64;
/// 模型 ID 长度上限（字符）。
pub const MAX_MODEL_ID_CHARS: usize = 200;
/// 推理密钥长度上限（字节）。
pub const MAX_KEY_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopAction {
    ApplyDirect,
    Reapply,
    RestorePrevious,
    SwitchOfficial,
    Release,
    RecoverRollback,
}

impl DesktopAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApplyDirect => "applyDirect",
            Self::Reapply => "reapply",
            Self::RestorePrevious => "restorePrevious",
            Self::SwitchOfficial => "switchOfficial",
            Self::Release => "release",
            Self::RecoverRollback => "recoverRollback",
        }
    }

    /// 该操作是否会改动 Desktop 外部文件。
    pub fn writes_files(self) -> bool {
        !matches!(self, Self::Release)
    }
}

/// 候选路径选择：`None` 表示使用默认发现结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PathsChoice {
    pub normal_root: Option<String>,
    pub third_party_root: Option<String>,
}

/// 1M 声明模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OneMMode {
    /// 由有效上下文来源推导，未知不声明。
    Auto,
    /// 用户显式关闭。
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopModelChoice {
    pub model_id: String,
    #[serde(default)]
    pub label_override: Option<String>,
    pub one_m_mode: OneMMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyDirectRequest {
    pub provider_id: String,
    #[serde(default)]
    pub models: Vec<DesktopModelChoice>,
    #[serde(default)]
    pub paths: PathsChoice,
    /// 以当前现状重新接管（覆盖受管冲突）。
    #[serde(default)]
    pub rebaseline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub models: Vec<DesktopModelChoice>,
    #[serde(default)]
    pub paths: PathsChoice,
    #[serde(default)]
    pub rebaseline: bool,
}

impl ActionRequest {
    pub fn into_direct(self) -> ApplyDirectRequest {
        ApplyDirectRequest {
            provider_id: self.provider_id.unwrap_or_default(),
            models: self.models,
            paths: self.paths,
            rebaseline: self.rebaseline,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub provider_id: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub paths: PathsChoice,
    /// 请求 Messages 兼容性核对（会向上游发一次 `max_tokens: 1` 的请求）。
    /// 默认关闭：不开就不会产生任何计费请求。
    #[serde(default)]
    pub check_messages: bool,
}

// ---------------------------------------------------------------------------
// 资格与检测原因码
// ---------------------------------------------------------------------------

/// 结构化资格原因。前端按码显示本地化文案，不猜测含义。
pub mod reason {
    pub const PROVIDER_MISSING: &str = "provider_missing";
    pub const NO_INFERENCE_KEY: &str = "no_inference_key";
    pub const INVALID_INFERENCE_KEY: &str = "invalid_inference_key";
    pub const INCOMPATIBLE_PROTOCOL: &str = "incompatible_protocol";
    pub const UNSUPPORTED_MODEL_ENDPOINT: &str = "unsupported_model_endpoint";
    pub const MODEL_NOT_INCLUDED: &str = "model_not_included";
    pub const UNSAFE_MODEL_NAME: &str = "unsafe_model_name";
    pub const UNSUPPORTED_DESKTOP_ROLE: &str = "unsupported_desktop_role";
    pub const ENDPOINT_LAYOUT_UNSUPPORTED: &str = "endpoint_layout_unsupported";
    // 注意：Bearer 检测结果不在这里。检测是用户点击触发的单次只读检查，
    // 结果由 `probe::BearerStatus` 表达，不写回候选列表，因此本模块不声明
    // 尚未接入的 bearer_* 原因码。
}

/// 协议来源：区分显式设置、端点声明、名称推断与卡片默认。
pub mod protocol_source {
    pub const EXPLICIT: &str = "explicit";
    pub const DECLARED: &str = "declared";
    pub const INFERRED: &str = "inferred";
    pub const PROVIDER_DEFAULT: &str = "providerDefault";
}

// ---------------------------------------------------------------------------
// 视图
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathCandidateView {
    pub id: String,
    pub normal_root: String,
    pub third_party_root: String,
    pub normal_exists: bool,
    pub third_party_exists: bool,
    /// 该候选是否具备安装证据（normal root 存在或含配置文件）。
    pub has_install_evidence: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PathsSummary {
    pub normal_root: String,
    pub third_party_root: String,
    pub normal_config: String,
    pub third_party_config: String,
    pub profile_path: String,
    pub meta_path: String,
    pub isolated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopDetection {
    pub platform: String,
    /// 探测到的 Desktop 版本；失败为 `unknown`，不编造。
    pub version: String,
    pub version_source: String,
    pub installed: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCandidateView {
    pub model_id: String,
    pub label: String,
    pub display_name: String,
    pub protocol: String,
    pub protocol_source: String,
    pub protocol_verified: bool,
    pub included: bool,
    pub supports1m: bool,
    pub context_window: Option<u64>,
    pub context_source: String,
    pub selectable: bool,
    pub exclude_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EndpointView {
    pub profile_base_url: String,
    pub messages_url: String,
    pub models_url: String,
    pub plaintext: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCandidateView {
    pub provider_id: String,
    pub name: String,
    pub enabled: bool,
    pub has_inference_key: bool,
    pub eligible: bool,
    pub reason: Option<String>,
    pub endpoint: Option<EndpointView>,
    pub models: Vec<ModelCandidateView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopCandidates {
    pub paths: PathsSummary,
    /// 多渠道时的候选；由用户明确选择，不按字典序自动挑。
    pub path_candidates: Vec<PathCandidateView>,
    /// 与 `paths` 对应的候选 id。
    pub selected_path_id: Option<String>,
    pub detector: DesktopDetection,
    pub providers: Vec<ProviderCandidateView>,
    pub selected_provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedModelView {
    pub model_id: String,
    pub label: String,
    pub one_m: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopActionView {
    pub action: DesktopAction,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStatus {
    pub detection: DesktopDetection,
    pub paths: Option<PathsSummary>,
    /// `unknown / official / foreignThirdParty / direct`
    pub mode: String,
    /// 卡片主显示状态：recoveryRequired > conflict > orphaned > stale > managed > unmanaged
    pub management_state: String,
    pub is_managed: bool,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub provider_missing: bool,
    pub models: Vec<ManagedModelView>,
    pub stale_reasons: Vec<String>,
    pub conflicts: Vec<String>,
    pub recovery_state: Option<String>,
    pub pending_recovery: bool,
    pub warnings: Vec<String>,
    pub last_applied_at: Option<i64>,
    pub restart_required: bool,
    /// Desktop 应用当前是否在运行（由进程探测得出，不是从文件推断）。
    pub app_running: bool,
    /// 是否已确认可自动启动；`false` 时界面不提供重启按钮。
    pub restart_capable: bool,
    /// 已确认的启动方式，便于用户理解为什么可以/不可以重启。
    pub launch_method: Option<String>,
    pub actions: Vec<DesktopActionView>,
    pub backup_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedChangeView {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeView {
    pub role: String,
    pub path: String,
    /// create / update / delete / noop
    pub kind: String,
    pub managed_changes: Vec<ManagedChangeView>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPreview {
    pub revision: String,
    pub action: DesktopAction,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub models: Vec<ManagedModelView>,
    pub messages_url: Option<String>,
    pub models_url: Option<String>,
    pub files: Vec<FileChangeView>,
    pub warnings: Vec<String>,
    pub conflicts: Vec<String>,
    pub requires_restart: bool,
    pub external_profile_count: usize,
    pub managed_entry_present: bool,
    pub current_applied_id: Option<String>,
    pub applies_to: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopRestartResult {
    /// 实际使用的启动方式。
    pub launch_method: String,
    /// 启动后探测到的运行状态。
    pub running: bool,
    pub restart_capable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopApplied {
    pub transaction_id: String,
    pub changed_paths: Vec<String>,
    pub backup_id: Option<String>,
    pub management_state: String,
    pub requires_restart: bool,
    pub warnings: Vec<String>,
}

/// 稳定的错误码：前端只按码与安全参数渲染，不解析原文。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopError {
    pub code: String,
    pub message: String,
    pub file_role: Option<String>,
    pub model_id: Option<String>,
    pub retryable: bool,
    pub suggested_action: Option<String>,
    pub params: Vec<String>,
}

impl DesktopError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            file_role: None,
            model_id: None,
            retryable: false,
            suggested_action: None,
            params: Vec::new(),
        }
    }

    pub fn file(mut self, role: &str) -> Self {
        self.file_role = Some(role.into());
        self
    }

    pub fn model(mut self, model_id: &str) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn action(mut self, action: &str) -> Self {
        self.suggested_action = Some(action.into());
        self
    }

    pub fn param(mut self, param: impl Into<String>) -> Self {
        self.params.push(param.into());
        self
    }
}

pub type DesktopResult<T> = Result<T, Box<DesktopError>>;

/// 前端可见的错误码。
pub mod error_code {
    pub const STORAGE: &str = "storage";
    pub const INVALID_URL: &str = "invalid_url";
    pub const PATHS_NOT_DETECTED: &str = "paths_not_detected";
    pub const PATHS_COLLISION: &str = "paths_collision";
    pub const INVALID_FILE: &str = "invalid_file";
    pub const PROVIDER_MISSING: &str = "provider_missing";
    pub const NO_INFERENCE_KEY: &str = "no_inference_key";
    pub const INVALID_INFERENCE_KEY: &str = "invalid_inference_key";
    pub const MODEL_REJECTED: &str = "model_rejected";
    pub const NO_MODELS_SELECTED: &str = "no_models_selected";
    pub const ENDPOINT_UNSUPPORTED: &str = "endpoint_unsupported";
    pub const PREVIEW_EXPIRED: &str = "preview_expired";
    pub const CHANGED: &str = "changed";
    pub const CONFLICT: &str = "conflict";
    pub const NOT_MANAGED: &str = "not_managed";
    pub const RECOVERY_REQUIRED: &str = "recovery_required";
    pub const PENDING_CLEANUP: &str = "pending_cleanup";
    pub const TOO_MANY_PREVIEWS: &str = "too_many_previews";
    pub const PROBE_FAILED: &str = "probe_failed";
    pub const WRITER_BUSY: &str = "writer_busy";
    pub const PROCESS: &str = "process";
}
