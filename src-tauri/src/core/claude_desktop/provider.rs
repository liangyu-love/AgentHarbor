//! Claude Desktop 直连：供应商引用、模型资格与 URL/能力投影。
//!
//! 这里是**纯逻辑**：不读 Store、不写文件、不联网。所有判定都以传入的
//! `Provider` 为准，因此前端预览与后端 commit 可以共用同一份规则。

use crate::core::models_dev;
use crate::core::newapi;
use crate::core::router::{ModelProtocol, Provider};

use super::constants::ONE_M_CONTEXT;
use super::types::{
    error_code, protocol_source, reason, DesktopError, DesktopResult, EndpointView,
    ModelCandidateView, MAX_KEY_BYTES, MAX_LABEL_CHARS, MAX_MODEL_ID_CHARS,
};

/// 上下文来源中可视为“有依据”的取值；其余（fallback/unknown/policy）不足以
/// 支撑 1M 声明。
fn context_source_is_evidence(source: &str) -> bool {
    use crate::core::model_metadata::source;
    matches!(
        source,
        source::MANUAL
            | source::PROVIDER_SETTING
            | source::UPSTREAM
            | source::OFFICIAL
            | source::CATALOG
            | source::CATALOG_GENERIC
            | source::CATALOG_ALIAS
            | source::CATALOG_SPEED_REFERENCE
    )
}

// ---------------------------------------------------------------------------
// 推理密钥
// ---------------------------------------------------------------------------

/// 清理后的推理密钥；`None` 表示该卡片没有可用的推理凭据。
pub fn inference_key(provider: &Provider) -> Option<String> {
    let raw = provider.model_credential().api_key.trim();
    if raw.is_empty() {
        return None;
    }
    // 允许用户把 `Bearer ` 一起粘贴进卡片，但 profile 只存原始 token，
    // 否则 Desktop 会拼出双前缀。
    let stripped = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(raw);
    Some(stripped.to_owned())
}

/// 密钥是否可以安全写入 profile：非空、无空白、长度可控。
pub fn key_is_writable(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_BYTES
        && !key.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

// ---------------------------------------------------------------------------
// 模型名校验
// ---------------------------------------------------------------------------

/// Desktop 版本策略：只影响角色白名单的版本证据，不改变其它规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopVersionPolicy {
    /// 已核实 fail-all 校验器纳入 fable 的版本或更高。
    pub fable_allowed: bool,
}

impl DesktopVersionPolicy {
    /// 未知版本一律保守：不启用 fable。
    pub fn from_version(version: Option<&str>) -> Self {
        let parsed = version.and_then(parse_version);
        Self {
            fable_allowed: parsed.is_some_and(|value| value >= super::constants::FABLE_MIN_VERSION),
        }
    }

    pub fn conservative() -> Self {
        Self {
            fable_allowed: false,
        }
    }
}

/// 解析 `1.12603.1` 形式的三段版本号；带预发布后缀时忽略后缀。
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().split(['-', '+']).next().unwrap_or_default();
    let mut parts = core.split('.');
    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts.next()?.trim().parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").trim().parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// 基础 ID 校验：非空、长度、控制字符、空白与 `[1m]` 后缀。
pub fn basic_model_id_error(model_id: &str) -> Option<&'static str> {
    if model_id.trim().is_empty() {
        return Some("empty");
    }
    if model_id.chars().count() > MAX_MODEL_ID_CHARS {
        return Some("tooLong");
    }
    if model_id.chars().any(char::is_control) {
        return Some("controlChar");
    }
    if model_id.chars().any(char::is_whitespace) || model_id.trim() != model_id {
        return Some("whitespace");
    }
    // 本工具保留中转原始 ID；1M 菜单变体通过 supports1m 表达，不将标记当上游 ID。
    if model_id.to_ascii_lowercase().contains("[1m]") {
        return Some("oneMMarker");
    }
    None
}

/// Desktop 的 fail-all 校验器接受的安全模型名。
///
/// 保留原始 ID：本函数只做判定，不做归一化后写入。
pub fn safe_model_id(model_id: &str, policy: DesktopVersionPolicy) -> bool {
    if basic_model_id_error(model_id).is_some() {
        return false;
    }
    let normalized = model_id.trim().to_ascii_lowercase();
    let tail = match normalized.strip_prefix("anthropic/claude-") {
        Some(tail) => tail,
        None => match normalized.strip_prefix("claude-") {
            Some(tail) => tail,
            None => return false,
        },
    };
    super::constants::roles_for(policy).iter().any(|role| {
        tail.strip_prefix(role)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|rest| !rest.is_empty())
    })
}

// ---------------------------------------------------------------------------
// 展示名
// ---------------------------------------------------------------------------

/// 清理后的 label：`None` 表示应回退到原 ID。
pub fn normalize_label(label: &str) -> Option<String> {
    let cleaned: String = label
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.chars().take(MAX_LABEL_CHARS).collect())
}

// ---------------------------------------------------------------------------
// 协议判定
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDecision {
    pub protocol: ModelProtocol,
    pub source: &'static str,
    /// `false` 表示协议来自名称推断，不能当作上游实测成功。
    pub verified: bool,
}

/// 逐模型解析协议及其来源。
///
/// 顺序与 `Provider::protocol_for_model` 完全一致，只是额外记录证据强度：
/// 显式设置 > 端点声明 > 名称推断 > 卡片默认。
pub fn protocol_decision(provider: &Provider, model_id: &str) -> ProtocolDecision {
    let claude_name = model_id.to_ascii_lowercase().contains("claude");
    let entry = provider.models.iter().find(|entry| entry.id == model_id);
    if let Some(entry) = entry {
        if let Some(explicit) = entry.protocol {
            return ProtocolDecision {
                protocol: explicit,
                source: protocol_source::EXPLICIT,
                verified: true,
            };
        }
        if claude_name {
            if let Some(declared) = crate::core::protocol::declared_protocol(&entry.endpoint_types)
            {
                return ProtocolDecision {
                    protocol: declared,
                    source: protocol_source::DECLARED,
                    verified: true,
                };
            }
        }
    }
    if claude_name {
        return ProtocolDecision {
            protocol: ModelProtocol::AnthropicMessages,
            source: protocol_source::INFERRED,
            verified: false,
        };
    }
    ProtocolDecision {
        protocol: provider.protocol,
        source: protocol_source::PROVIDER_DEFAULT,
        verified: false,
    }
}

pub fn protocol_label(protocol: ModelProtocol) -> &'static str {
    match protocol {
        ModelProtocol::Responses => "responses",
        ModelProtocol::ChatCompletions => "chatCompletions",
        ModelProtocol::AnthropicMessages => "anthropicMessages",
    }
}

// ---------------------------------------------------------------------------
// 地址投影
// ---------------------------------------------------------------------------

fn unsupported_layout(detail: &str) -> Box<DesktopError> {
    Box::new(
        DesktopError::new(
            error_code::ENDPOINT_UNSUPPORTED,
            "claudeDesktop.errors.endpointLayoutUnsupported",
        )
        .param(detail)
        .action("useGateway"),
    )
}

/// 完整接口地址不能当 root：不把已有错误地址偷偷改成另一种含义。
fn looks_like_full_endpoint(path: &str) -> bool {
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    path.ends_with("/v1/messages")
        || path.ends_with("/v1/responses")
        || path.ends_with("/v1/chat/completions")
        || path.ends_with("/v1/models")
        || path.ends_with("/messages")
        || path.ends_with("/chat/completions")
}

/// 解析 Desktop 将实际访问的地址。
///
/// 返回 `(profile 中保存的根地址, Messages URL, Models URL)`。
pub fn resolve_direct_endpoints(provider: &Provider) -> DesktopResult<(String, String, String)> {
    let base = provider.base_url.trim();
    let parsed = newapi::base_url(base).map_err(|_| {
        Box::new(
            DesktopError::new(error_code::INVALID_URL, "claudeDesktop.errors.invalidUrl")
                .retryable(),
        )
    })?;
    if looks_like_full_endpoint(parsed.path()) {
        return Err(Box::new(
            DesktopError::new(
                error_code::INVALID_URL,
                "claudeDesktop.errors.fullEndpointAsRoot",
            )
            .action("editProvider"),
        ));
    }
    let protocol_root =
        newapi::protocol_base_url(base, ModelProtocol::AnthropicMessages).map_err(|_| {
            Box::new(
                DesktopError::new(error_code::INVALID_URL, "claudeDesktop.errors.invalidUrl")
                    .retryable(),
            )
        })?;
    // Check the actual Messages root, after mapping standard official roots.
    // Desktop appends v1/... and cannot use a remaining non-v1 layout.
    let host = protocol_root.host_str().unwrap_or_default();
    let path = protocol_root.path();
    let non_v1 = matches!(
        (host, path),
        ("api.novita.ai", "/v3/openai/")
            | (
                "open.bigmodel.cn" | "api.z.ai",
                "/api/coding/paas/v4/" | "/api/paas/v4/"
            )
    );
    if non_v1 {
        return Err(unsupported_layout("nonV1"));
    }

    let root = desktop_root(protocol_root.as_str());
    let messages = join_desktop(&root, "v1/messages")?;
    let models = join_desktop(&root, "v1/models")?;

    // 与项目自身的端点投影比对：不一致说明 Desktop 无法访问同一接口。
    let project = newapi::provider_endpoint(base, "v1/messages").map_err(|_| {
        Box::new(
            DesktopError::new(error_code::INVALID_URL, "claudeDesktop.errors.invalidUrl")
                .retryable(),
        )
    })?;
    if !same_url(&messages, project.as_str()) {
        return Err(unsupported_layout("layoutMismatch"));
    }
    Ok((root, messages, models))
}

/// Desktop 在 profile 根地址后追加 `v1/...`；去掉末尾 `/` 与末尾 `/v1`，
/// 避免拼出 `/v1/v1`。
fn desktop_root(text: &str) -> String {
    let trimmed = text.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn join_desktop(root: &str, path: &str) -> DesktopResult<String> {
    reqwest::Url::parse(&format!("{root}/{path}"))
        .map(|url| url.as_str().to_owned())
        .map_err(|_| unsupported_layout("joinFailed"))
}

/// 只比较调用语义：scheme、host、有效端口与去掉末尾 `/` 的 path。
/// 查询串与 fragment 已在 `newapi::base_url` 阶段被拒绝。
fn same_url(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (reqwest::Url::parse(left), reqwest::Url::parse(right)) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
        && left.path().trim_end_matches('/') == right.path().trim_end_matches('/')
}

pub fn endpoint_view(provider: &Provider) -> DesktopResult<EndpointView> {
    let (root, messages, models) = resolve_direct_endpoints(provider)?;
    Ok(EndpointView {
        profile_base_url: root.clone(),
        messages_url: messages,
        models_url: models,
        plaintext: root.starts_with("http://"),
    })
}

// ---------------------------------------------------------------------------
// 能力投影
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextProjection {
    pub context_window: Option<u64>,
    pub source: String,
    pub supports1m: bool,
}

/// 由有效能力推导 1M 声明；未知、兜底或纯名称推断时默认不声明。
pub fn project_context(
    provider: &Provider,
    model_id: &str,
    catalog: &models_dev::Catalog,
) -> ContextProjection {
    let resolved = crate::core::router::model_metadata_for(provider, model_id, catalog);
    let source = resolved.context_window.source.clone();
    let supports1m = resolved
        .context_window
        .value
        .is_some_and(|value| value >= ONE_M_CONTEXT)
        && context_source_is_evidence(&source);
    ContextProjection {
        context_window: resolved.context_window.value,
        source,
        supports1m,
    }
}

// ---------------------------------------------------------------------------
// 候选模型
// ---------------------------------------------------------------------------

fn candidate_ids(provider: &Provider) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let default_id = provider.model.trim();
    let default_entry = provider.models.iter().find(|entry| entry.id == default_id);
    // 卡片默认模型在目录中没有条目时，可作为已有手填模型候选；
    // 但已有条目明确 included=false 时不因其同时是默认模型而绕过排除设置。
    if !default_id.is_empty() && default_entry.is_none() {
        out.push((default_id.to_owned(), true));
    }
    // 排除项也返回原因，只有 selectable 项可提交。
    for entry in &provider.models {
        if entry.id.trim().is_empty() {
            continue;
        }
        if !out.iter().any(|(id, _)| id == &entry.id) {
            out.push((entry.id.clone(), false));
        }
    }
    if !default_id.is_empty()
        && default_entry.is_some_and(|entry| entry.included)
        && !out.iter().any(|(id, _)| id == default_id)
    {
        out.push((default_id.to_owned(), false));
    }
    out
}

/// 单个模型的资格判定。返回 `Ok` 表示可直连。
pub fn model_candidate(
    provider: &Provider,
    model_id: &str,
    catalog: &models_dev::Catalog,
    policy: DesktopVersionPolicy,
) -> ModelCandidateView {
    let decision = protocol_decision(provider, model_id);
    let projection = project_context(provider, model_id, catalog);
    let saved = provider.models.iter().find(|entry| entry.id == model_id);
    let display_name = saved
        .map(|entry| entry.display_name.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(model_id)
        .to_owned();
    let label = normalize_label(&display_name).unwrap_or_else(|| model_id.to_owned());

    let exclude_reason = model_exclusion(provider, model_id, decision, policy);
    ModelCandidateView {
        model_id: model_id.to_owned(),
        label,
        display_name,
        protocol: protocol_label(decision.protocol).to_owned(),
        protocol_source: decision.source.to_owned(),
        protocol_verified: decision.verified,
        included: saved.map(|entry| entry.included).unwrap_or(true),
        supports1m: projection.supports1m,
        context_window: projection.context_window,
        context_source: projection.source,
        selectable: exclude_reason.is_none(),
        exclude_reason,
    }
}

fn model_exclusion(
    provider: &Provider,
    model_id: &str,
    decision: ProtocolDecision,
    policy: DesktopVersionPolicy,
) -> Option<String> {
    if let Some(detail) = basic_model_id_error(model_id) {
        return Some(format!("{}:{detail}", reason::UNSAFE_MODEL_NAME));
    }
    if !safe_model_id(model_id, policy) {
        return Some(reason::UNSUPPORTED_DESKTOP_ROLE.to_owned());
    }
    match provider.models.iter().find(|entry| entry.id == model_id) {
        Some(entry) if !entry.included => return Some(reason::MODEL_NOT_INCLUDED.to_owned()),
        None if model_id != provider.model.trim() => {
            return Some(reason::MODEL_REMOVED.to_owned());
        }
        _ => {}
    }
    // 显式协议为 Responses/Chat 时，即使名字符合白名单也拒绝直连。
    if decision.protocol != ModelProtocol::AnthropicMessages {
        return Some(reason::INCOMPATIBLE_PROTOCOL.to_owned());
    }
    if !provider.supports_model(model_id) {
        return Some(reason::UNSUPPORTED_MODEL_ENDPOINT.to_owned());
    }
    None
}

/// 卡片级资格：与路由池 `enabled` 无关。
pub fn provider_qualification(
    provider: &Provider,
    policy: DesktopVersionPolicy,
    catalog: &models_dev::Catalog,
) -> (bool, Option<String>, Vec<ModelCandidateView>) {
    let key = inference_key(provider);
    let reason = match &key {
        None => Some(reason::NO_INFERENCE_KEY.to_owned()),
        Some(value) if !key_is_writable(value) => Some(reason::INVALID_INFERENCE_KEY.to_owned()),
        Some(_) => match resolve_direct_endpoints(provider) {
            Err(error) => Some(format!("{}:{}", error.code, error.params.join(","))),
            Ok(_) => None,
        },
    };
    let models = candidate_ids(provider)
        .into_iter()
        .map(|(model_id, _)| model_candidate(provider, &model_id, catalog, policy))
        .collect::<Vec<_>>();
    let eligible = reason.is_none() && models.iter().any(|model| model.selectable);
    (eligible, reason, models)
}

/// 推理密钥的指纹（SHA-256 十六进制）。
///
/// 用途只有一个：判断「卡片换了密钥，而 Desktop 还在用旧的」。因此**只存指纹**，
/// 不存密钥本身；指纹不足以还原密钥。没有可用密钥时返回 `None`。
pub fn credential_fingerprint(provider: &Provider) -> Option<String> {
    let key = inference_key(provider)?;
    if !key_is_writable(&key) {
        return None;
    }
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    Some(format!("{:x}", hasher.finalize()))
}

pub fn require_inference_key(provider: &Provider) -> DesktopResult<String> {
    let key = inference_key(provider).ok_or_else(|| {
        Box::new(
            DesktopError::new(
                error_code::NO_INFERENCE_KEY,
                "claudeDesktop.errors.noInferenceKey",
            )
            .action("editProvider"),
        )
    })?;
    if !key_is_writable(&key) {
        return Err(Box::new(
            DesktopError::new(
                error_code::INVALID_INFERENCE_KEY,
                "claudeDesktop.errors.invalidInferenceKey",
            )
            .action("editProvider"),
        ));
    }
    Ok(key)
}

#[cfg(test)]
mod candidate_membership_tests {
    use super::*;

    #[test]
    fn only_saved_included_models_or_the_exact_manual_default_are_selectable() {
        let root =
            std::env::temp_dir().join(format!("desktop-candidates-{}", uuid::Uuid::new_v4()));
        let catalog = models_dev::load(&root);
        let mut card = Provider {
            model: "claude-sonnet-4-6".into(),
            ..Default::default()
        };
        let policy = DesktopVersionPolicy::conservative();
        assert!(model_candidate(&card, &card.model, &catalog, policy).selectable);
        let missing = model_candidate(&card, "claude-haiku-4-5-DELETED", &catalog, policy);
        assert_eq!(
            missing.exclude_reason.as_deref(),
            Some(reason::MODEL_REMOVED)
        );
        card.models.push(newapi::Model {
            id: card.model.clone(),
            included: false,
            ..Default::default()
        });
        let excluded = model_candidate(&card, &card.model, &catalog, policy);
        assert_eq!(
            excluded.exclude_reason.as_deref(),
            Some(reason::MODEL_NOT_INCLUDED)
        );
        let (_, _, views) = provider_qualification(&card, policy, &catalog);
        assert_eq!(views.len(), 1, "排除模型保留可解释的原因");
        assert!(!views[0].selectable);
    }
}
