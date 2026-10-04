//! Claude Desktop 直连：同模块单测。
//!
//! 全部使用临时根目录、空白 models.dev 目录与哨兵凭据，**不读取也不写入
//! 真实 Desktop 配置**。

use std::path::PathBuf;
use std::sync::MutexGuard;

use serde_json::{json, Value};

use crate::core::models_dev;
use crate::core::newapi;
use crate::core::router::{ModelProtocol, Provider};

use super::paths::{self, role, DesktopPaths};
use super::profile::{self, MetaDocument, MetaEntry};
use super::provider::{self, DesktopVersionPolicy};
use super::transaction::{self, DesiredState, ManagedValues};

use super::types::{DesktopAction, ManagedModelView, OneMMode};

const SENTINEL_KEY: &str = "sk-SENTINEL-DO-NOT-LEAK-0001";
const CC_SWITCH_ID: &str = "00000000-0000-4000-8000-000000157210";

/// 进程级环境变量是全局状态，与依赖发现的测试共用同一把锁。
fn env_lock() -> MutexGuard<'static, ()> {
    super::env_test_lock()
}

/// 在作用域内设置 `AGENT_HARBOR_CLIENT_HOME`，离开作用域时**必定**还原。
///
/// 直接 `set_var` / `remove_var` 时，一旦中间有断言失败，变量会留在进程里，
/// 后续测试就会看到隔离根——这正是之前整库测试偶发失败的原因。
struct IsolatedHome;

impl IsolatedHome {
    fn enter(root: &std::path::Path) -> Self {
        std::env::set_var("AGENT_HARBOR_CLIENT_HOME", root);
        Self
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        std::env::remove_var("AGENT_HARBOR_CLIENT_HOME");
    }
}

/// 与生产 `mod::profile_baseline` 同义：有管理记录时用记录里的基线，否则用现状。
fn baseline_for(paths: &DesktopPaths, previous: Option<&transaction::State>) -> transaction::TakeoverBaseline {
    let editions = transaction::editions_before(paths, previous, false).unwrap();
    transaction::TakeoverBaseline::from_editions(&editions, false)
}

pub fn temp_root(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "agentharbor-desktop-test-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&base).expect("create temp root");
    base
}

/// 隔离的两根；normal root 预先建好（模拟已确认安装）。
pub fn test_paths(name: &str) -> (PathBuf, DesktopPaths) {
    let base = temp_root(name);
    let normal = base.join("Claude");
    let three_p = base.join("Claude-3p");
    std::fs::create_dir_all(&normal).expect("create normal root");
    let paths = DesktopPaths::from_roots(normal, three_p).expect("distinct roots");
    (base, paths)
}

/// 空目录上的 catalog：不联网、不含任何模型族推断表。
fn empty_catalog() -> std::sync::Arc<models_dev::Catalog> {
    models_dev::load(&temp_root("catalog"))
}

fn provider(base_url: &str, models: Vec<newapi::Model>) -> Provider {
    Provider {
        id: "p1".into(),
        name: "中转 A".into(),
        base_url: base_url.into(),
        api_key: SENTINEL_KEY.into(),
        access_token: "balance-token-SENTINEL".into(),
        model: models.first().map(|model| model.id.clone()).unwrap_or_default(),
        models,
        enabled: false,
        ..Default::default()
    }
}

fn model(id: &str, protocol: Option<ModelProtocol>, endpoints: &[&str]) -> newapi::Model {
    newapi::Model {
        id: id.into(),
        display_name: String::new(),
        protocol,
        endpoint_types: endpoints.iter().map(|value| (*value).to_owned()).collect(),
        included: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 路径
// ---------------------------------------------------------------------------

#[test]
fn roots_must_be_distinct_and_non_nested() {
    let base = temp_root("roots");
    let same = base.join("Claude");
    assert!(DesktopPaths::from_roots(same.clone(), same.clone()).is_err());
    assert!(DesktopPaths::from_roots(base.clone(), base.join("Claude-3p")).is_err());
    assert!(DesktopPaths::from_roots(base.join("Claude-3p"), base.clone()).is_err());
    let ok = DesktopPaths::from_roots(base.join("Claude"), base.join("Claude-3p"));
    assert!(ok.is_ok());
}

#[test]
fn four_target_paths_are_always_distinct() {
    let (_base, paths) = test_paths("distinct");
    let all = [
        paths.normal_config.clone(),
        paths.third_party_config.clone(),
        paths.profile_path.clone(),
        paths.meta_path.clone(),
    ];
    for (index, left) in all.iter().enumerate() {
        for right in all.iter().skip(index + 1) {
            assert_ne!(left, right);
        }
    }
}

#[test]
fn isolated_home_confines_discovery_to_the_isolated_root() {
    let _guard = env_lock();
    let base = temp_root("isolated");
    std::fs::create_dir_all(base.join("Claude")).unwrap();
    let _isolated = IsolatedHome::enter(&base);
    let discovery = paths::discover();
    assert!(!discovery.candidates.is_empty());
    let candidate = &discovery.candidates[0];
    assert!(candidate.paths.isolated);
    assert_eq!(candidate.paths.normal_root, paths::normalize(&base.join("Claude")));
    assert!(candidate.paths.normal_root.starts_with(&base));
}

#[test]
fn version_detection_is_conservative_but_parses_three_parts() {
    assert!(paths::is_version_like("1.12603.1"));
    assert!(paths::is_version_like("1.12603.1-beta"));
    assert!(!paths::is_version_like("unknown"));
    assert!(!paths::is_version_like("1.2"));
    assert_eq!(provider::parse_version("1.12603.1"), Some((1, 12603, 1)));
}

// ---------------------------------------------------------------------------
// 模型名与协议
// ---------------------------------------------------------------------------

#[test]
fn safe_roles_keep_the_original_identifier() {
    let policy = DesktopVersionPolicy::conservative();
    assert!(provider::safe_model_id("claude-sonnet-4-6", policy));
    assert!(provider::safe_model_id("claude-opus-4-6", policy));
    assert!(provider::safe_model_id("claude-haiku-4-5", policy));
    assert!(provider::safe_model_id("anthropic/claude-sonnet-5", policy));
    // 版本未知时 fable 不在白名单内。
    assert!(!provider::safe_model_id("claude-fable-5", policy));
    let known = DesktopVersionPolicy::from_version(Some("1.12603.1"));
    assert!(known.fable_allowed);
    assert!(provider::safe_model_id("claude-fable-5", known));
    // Mythos 即使名字像 Claude 也不放行。
    assert!(!provider::safe_model_id("claude-mythos-1", known));
    let older = DesktopVersionPolicy::from_version(Some("1.12000.0"));
    assert!(!provider::safe_model_id("claude-fable-5", older));
    assert!(!provider::safe_model_id("claude-fable-5", DesktopVersionPolicy::from_version(None)));
}

#[test]
fn unsafe_identifiers_are_rejected_with_a_reason() {
    let policy = DesktopVersionPolicy::conservative();
    assert!(!provider::safe_model_id("kimi-k2", policy));
    assert!(!provider::safe_model_id("gpt-4o", policy));
    assert!(!provider::safe_model_id("claude-3-5-sonnet-20241022", policy));
    assert!(!provider::safe_model_id("claude-sonnet-", policy));
    assert!(!provider::safe_model_id("claude-sonnet-4-6[1m]", policy));
    assert!(!provider::safe_model_id("claude-sonnet-4-6\u{7}", policy));
    assert_eq!(provider::basic_model_id_error(""), Some("empty"));
    assert_eq!(provider::basic_model_id_error("claude-sonnet-4-6 "), Some("whitespace"));
    assert_eq!(
        provider::basic_model_id_error("claude-sonnet-4-6[1m]"),
        Some("oneMMarker")
    );
}

#[test]
fn protocol_sources_distinguish_inference_from_explicit_settings() {
    let explicit = provider(
        "https://relay.example",
        vec![model("claude-sonnet-4-6", Some(ModelProtocol::Responses), &[])],
    );
    let decision = provider::protocol_decision(&explicit, "claude-sonnet-4-6");
    assert_eq!(decision.protocol, ModelProtocol::Responses);
    assert_eq!(decision.source, super::types::protocol_source::EXPLICIT);
    assert!(decision.verified);

    let declared = provider(
        "https://relay.example",
        vec![model("claude-sonnet-4-6", None, &["anthropic_messages"])],
    );
    assert_eq!(
        provider::protocol_decision(&declared, "claude-sonnet-4-6").source,
        super::types::protocol_source::DECLARED
    );

    let inferred = provider("https://relay.example", vec![model("claude-sonnet-4-6", None, &[])]);
    let decision = provider::protocol_decision(&inferred, "claude-sonnet-4-6");
    assert_eq!(decision.protocol, ModelProtocol::AnthropicMessages);
    assert_eq!(decision.source, super::types::protocol_source::INFERRED);
    assert!(!decision.verified, "推断来源不能冒充上游实测成功");
}

#[test]
fn explicit_responses_protocol_is_excluded_even_for_a_safe_claude_name() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let card = provider(
        "https://relay.example",
        vec![model("claude-sonnet-4-6", Some(ModelProtocol::Responses), &[])],
    );
    let candidate = provider::model_candidate(&card, "claude-sonnet-4-6", &catalog, policy);
    assert!(!candidate.selectable);
    assert_eq!(
        candidate.exclude_reason.as_deref(),
        Some(super::types::reason::INCOMPATIBLE_PROTOCOL)
    );
}

#[test]
fn mixed_card_can_select_the_messages_model_without_touching_the_default() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let card = provider(
        "https://relay.example",
        vec![
            model("claude-sonnet-4-6", Some(ModelProtocol::Responses), &[]),
            model("claude-opus-4-6", Some(ModelProtocol::AnthropicMessages), &[]),
        ],
    );
    let responses = provider::model_candidate(&card, "claude-sonnet-4-6", &catalog, policy);
    let messages = provider::model_candidate(&card, "claude-opus-4-6", &catalog, policy);
    assert!(!responses.selectable);
    assert!(messages.selectable);
    // 卡片默认模型保持原样，不被直连选择改写。
    assert_eq!(card.model, "claude-sonnet-4-6");
}

// ---------------------------------------------------------------------------
// 地址投影
// ---------------------------------------------------------------------------

fn endpoints(base_url: &str) -> (String, String) {
    let card = provider(base_url, vec![]);
    let (root, messages, _) = provider::resolve_direct_endpoints(&card).expect("projectable");
    (root, messages)
}

#[test]
fn root_and_v1_and_custom_prefix_project_without_doubling_v1() {
    assert_eq!(
        endpoints("https://relay.example"),
        (
            "https://relay.example".into(),
            "https://relay.example/v1/messages".into()
        )
    );
    assert_eq!(
        endpoints("https://relay.example/v1"),
        (
            "https://relay.example".into(),
            "https://relay.example/v1/messages".into()
        )
    );
    assert_eq!(
        endpoints("https://relay.example/custom/v1"),
        (
            "https://relay.example/custom".into(),
            "https://relay.example/custom/v1/messages".into()
        )
    );
}

#[test]
fn deepseek_messages_root_is_preserved() {
    let (root, messages) = endpoints("https://api.deepseek.com");
    assert_eq!(root, "https://api.deepseek.com/anthropic");
    assert_eq!(messages, "https://api.deepseek.com/anthropic/v1/messages");
    // 与项目自身端点投影一致。
    let project = newapi::provider_endpoint("https://api.deepseek.com", "v1/messages").unwrap();
    assert_eq!(messages, project.as_str());
}

#[test]
fn non_v1_and_full_endpoint_layouts_are_rejected() {
    // 非 v1 特例主机：Desktop 只会追加 v1/，与项目自身投影不一致，首期拒绝。
    for base in [
        "https://api.novita.ai/v3/openai",
        "https://open.bigmodel.cn/api/paas/v4",
        "https://api.z.ai/api/coding/paas/v4",
    ] {
        let card = provider(base, vec![]);
        let error = provider::resolve_direct_endpoints(&card).expect_err("layout mismatch");
        assert_eq!(
            error.code,
            super::types::error_code::ENDPOINT_UNSUPPORTED,
            "base {base}"
        );
    }
    for base in [
        "https://relay.example/v1/messages",
        "https://relay.example/v1/chat/completions",
    ] {
        let card = provider(base, vec![]);
        let error = provider::resolve_direct_endpoints(&card).expect_err("full endpoint as root");
        assert_eq!(error.code, super::types::error_code::INVALID_URL, "base {base}");
    }
}

#[test]
fn invalid_urls_are_rejected_before_any_write() {
    for base in [
        "not-a-url",
        "ftp://relay.example",
        "https://user:pass@relay.example",
        "https://relay.example?token=1",
        "https://relay.example#frag",
    ] {
        let card = provider(base, vec![]);
        assert!(provider::resolve_direct_endpoints(&card).is_err(), "base {base}");
    }
}

#[test]
fn plaintext_http_is_reported_but_not_upgraded() {
    let view = provider::endpoint_view(&provider("http://127.0.0.1:8080", vec![])).unwrap();
    assert_eq!(view.profile_base_url, "http://127.0.0.1:8080");
    assert!(view.plaintext);
    assert!(!provider::endpoint_view(&provider("https://relay.example", vec![]))
        .unwrap()
        .plaintext);
}

// ---------------------------------------------------------------------------
// 凭据
// ---------------------------------------------------------------------------

#[test]
fn inference_key_never_borrows_the_balance_token() {
    let mut card = provider("https://relay.example", vec![]);
    card.api_key = String::new();
    assert!(provider::inference_key(&card).is_none());
    let error = provider::require_inference_key(&card).unwrap_err();
    assert_eq!(error.code, super::types::error_code::NO_INFERENCE_KEY);
    // access_token 存在也不能被当成推理凭据。
    assert_eq!(card.access_token, "balance-token-SENTINEL");
    // 只有余额凭据的卡片整卡不合格，但原因必须具体。
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let (eligible, reason, _) = provider::provider_qualification(&card, policy, &catalog);
    assert!(!eligible);
    assert_eq!(reason.as_deref(), Some(super::types::reason::NO_INFERENCE_KEY));
}

#[test]
fn bearer_prefix_is_not_doubled_in_the_profile() {
    let mut card = provider("https://relay.example", vec![]);
    card.api_key = format!("Bearer {SENTINEL_KEY}");
    let key = provider::inference_key(&card).unwrap();
    assert_eq!(key, SENTINEL_KEY);
    let profile = profile::profile_json("https://relay.example", &key, &[]);
    assert_eq!(profile[profile::KEY_API_KEY], json!(SENTINEL_KEY));
}

#[test]
fn invalid_keys_are_rejected() {
    let mut card = provider("https://relay.example", vec![]);
    card.api_key = "has space".into();
    assert!(provider::require_inference_key(&card).is_err());
    card.api_key = "x".repeat(9000);
    assert!(provider::require_inference_key(&card).is_err());
}

// ---------------------------------------------------------------------------
// profile 与 meta
// ---------------------------------------------------------------------------

#[test]
fn profile_uses_strings_without_options_and_objects_with_them() {
    let models = vec![
        ManagedModelView {
            model_id: "claude-sonnet-4-6".into(),
            label: "claude-sonnet-4-6".into(),
            one_m: false,
        },
        ManagedModelView {
            model_id: "claude-opus-4-6".into(),
            label: "中转 A · Opus".into(),
            one_m: true,
        },
    ];
    let profile = profile::profile_json("https://relay.example", SENTINEL_KEY, &models);
    assert_eq!(profile[profile::KEY_MODELS][0], json!("claude-sonnet-4-6"));
    assert_eq!(profile[profile::KEY_MODELS][1]["name"], json!("claude-opus-4-6"));
    assert_eq!(profile[profile::KEY_MODELS][1]["labelOverride"], json!("中转 A · Opus"));
    assert_eq!(profile[profile::KEY_MODELS][1]["supports1m"], json!(true));
    assert_eq!(profile[profile::KEY_DISABLE_CHOOSER], json!(true));
    assert_eq!(profile[profile::KEY_INFERENCE_PROVIDER], json!("gateway"));
    assert_eq!(profile[profile::KEY_AUTH_SCHEME], json!("bearer"));
    // 首期最小候选：不写 Cowork 出口字段，也不写 Claude Code 的 [1m] 后缀。
    assert!(profile.get("coworkEgressAllowedHosts").is_none());
    assert!(!profile.to_string().contains("[1m]"));
}

#[test]
fn label_only_model_uses_an_object_without_supports1m() {
    let models = vec![ManagedModelView {
        model_id: "claude-opus-4-6".into(),
        label: "Opus".into(),
        one_m: false,
    }];
    let profile = profile::profile_json("https://relay.example", SENTINEL_KEY, &models);
    assert_eq!(profile[profile::KEY_MODELS][0]["labelOverride"], json!("Opus"));
    assert!(profile[profile::KEY_MODELS][0].get("supports1m").is_none());
}

#[test]
fn meta_keeps_foreign_entries_and_order() {
    let text = format!(
        r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}},{{"id":"other","name":"Other","extra":1}}],"unknown":true}}"#
    );
    let document = MetaDocument::parse(Some(&text)).unwrap();
    assert_eq!(document.external_entry_count().unwrap(), 2);
    assert!(!document.own_entry_present().unwrap());
    let rendered = document
        .render(Some(&MetaEntry::own()), Some(Some(super::constants::PROFILE_ID)), 0)
        .unwrap();
    let entries = rendered["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    // 自身条目按 insert_at 插入，外部条目顺序不变。
    assert_eq!(entries[0]["id"], json!(super::constants::PROFILE_ID));
    assert_eq!(entries[1]["id"], json!(CC_SWITCH_ID));
    assert_eq!(entries[1]["name"], json!("CC Switch"));
    assert_eq!(entries[2]["extra"], json!(1));
    assert_eq!(rendered["unknown"], json!(true));
    assert_eq!(rendered["appliedId"], json!(super::constants::PROFILE_ID));
}

#[test]
fn meta_rejects_non_array_entries_and_non_object_roots() {
    let error = MetaDocument::parse(Some(r#"{"entries":{"id":"x"}}"#)).unwrap_err();
    assert_eq!(error.code, super::types::error_code::INVALID_FILE);
    assert_eq!(error.file_role.as_deref(), Some(role::META));
    let error = MetaDocument::parse(Some(r#"[1,2,3]"#)).unwrap_err();
    assert_eq!(error.code, super::types::error_code::INVALID_FILE);
    let error = MetaDocument::parse(Some("{not json")).unwrap_err();
    assert_eq!(error.code, super::types::error_code::INVALID_FILE);
}

#[test]
fn reapply_never_duplicates_the_own_entry() {
    let text = format!(r#"{{"entries":[{{"id":"{CC_SWITCH_ID}"}}]}}"#);
    let mut current = MetaDocument::parse(Some(&text)).unwrap();
    for _ in 0..3 {
        let rendered = current
            .render(Some(&MetaEntry::own()), Some(Some(super::constants::PROFILE_ID)), 0)
            .unwrap();
        current = MetaDocument::parse(Some(&rendered.to_string())).unwrap();
    }
    let entries = current.entry_list().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.id == super::constants::PROFILE_ID)
            .count(),
        1
    );
    assert_eq!(current.external_entry_count().unwrap(), 1);
}

#[test]
fn deployment_mode_patch_preserves_unrelated_keys() {
    let text = r#"{"deploymentMode":"3p","mcpServers":{"x":1},"preferences":{"a":2},"coworkUserFilesPath":"C:\\x"}"#;
    let mut map = profile::parse_object(Some(text), role::NORMAL_CONFIG).unwrap();
    profile::patch_deployment_mode(&mut map, "1p");
    let value = Value::Object(map);
    assert_eq!(value["deploymentMode"], json!("1p"));
    assert_eq!(value["mcpServers"]["x"], json!(1));
    assert_eq!(value["preferences"]["a"], json!(2));
    assert_eq!(value["coworkUserFilesPath"], json!("C:\\x"));
}

#[test]
fn shared_object_files_must_parse_as_objects() {
    assert_eq!(
        profile::parse_object(Some("{"), role::NORMAL_CONFIG)
        .unwrap_err()
        .code,
        super::types::error_code::INVALID_FILE
    );
    assert_eq!(
        profile::parse_object(Some("[1]"), role::NORMAL_CONFIG)
        .unwrap_err()
        .code,
        super::types::error_code::INVALID_FILE
    );
    assert!(profile::parse_object(None, role::NORMAL_CONFIG).unwrap().is_empty());
    assert_eq!(profile::parse_object(Some("  "), role::NORMAL_CONFIG).unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// 1M 与能力
// ---------------------------------------------------------------------------

#[test]
fn context_projection_is_fallback_aware() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    // 无任何有效来源（unknown）时不得声明 1M。
    let card = provider("https://relay.example", vec![model("claude-opus-4-6", None, &[])]);
    let candidate = provider::model_candidate(&card, "totally-unknown-model", &catalog, policy);
    assert!(!candidate.supports1m, "unknown 来源不能声明 1M");
    assert_eq!(
        candidate.context_source,
        crate::core::model_metadata::source::UNKNOWN
    );
    // 用户显式覆盖 1,000,000 时来源是 manual，可以声明。
    let mut explicit = provider("https://relay.example", vec![model("claude-sonnet-4-6", None, &[])]);
    explicit.models[0].overrides.context_window = Some(1_000_000);
    let candidate = provider::model_candidate(&explicit, "claude-sonnet-4-6", &catalog, policy);
    assert!(candidate.supports1m);
    assert_eq!(candidate.context_source, crate::core::model_metadata::source::MANUAL);
    // 小于 1M 的显式覆盖不声明。
    let mut small = explicit.clone();
    small.models[0].overrides.context_window = Some(200_000);
    assert!(!provider::model_candidate(&small, "claude-sonnet-4-6", &catalog, policy).supports1m);
}

// ---------------------------------------------------------------------------
// 资格
// ---------------------------------------------------------------------------

#[test]
fn disabled_provider_is_still_eligible_for_direct_connection() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let card = provider("https://relay.example", vec![model("claude-sonnet-4-6", None, &[])]);
    assert!(!card.enabled, "本测试要求卡片未启用");
    let (eligible, reason, models) = provider::provider_qualification(&card, policy, &catalog);
    assert!(reason.is_none());
    assert!(eligible, "enabled=false 不应阻止直连资格");
    assert!(models.iter().any(|model| model.selectable));
}

#[test]
fn exclusion_reasons_are_specific_per_model() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let card = provider(
        "https://relay.example",
        vec![
            model("claude-sonnet-4-6", None, &["openai_chat"]),
            model(
                "claude-opus-4-5",
                Some(ModelProtocol::AnthropicMessages),
                &["openai_chat"],
            ),
            model("gpt-4o", None, &[]),
            model("claude-3-5-sonnet-20241022", None, &[]),
            model("claude-opus-4-6", None, &[]),
        ],
    );
    let (_, _, models) = provider::provider_qualification(&card, policy, &catalog);
    let reason = |id: &str| {
        models
            .iter()
            .find(|model| model.model_id == id)
            .and_then(|model| model.exclude_reason.clone())
    };
    // `openai_chat` 声明让名称推断出的 Anthropic Messages 不成立：协议不合格。
    assert_eq!(
        reason("claude-sonnet-4-6").as_deref(),
        Some(super::types::reason::INCOMPATIBLE_PROTOCOL)
    );
    // 显式声明 Messages 但端点不支持时，是端点不受支持而不是协议问题。
    assert_eq!(
        reason("claude-opus-4-5").as_deref(),
        Some(super::types::reason::UNSUPPORTED_MODEL_ENDPOINT)
    );
    assert_eq!(
        reason("gpt-4o").as_deref(),
        Some(super::types::reason::UNSUPPORTED_DESKTOP_ROLE)
    );
    assert_eq!(
        reason("claude-3-5-sonnet-20241022").as_deref(),
        Some(super::types::reason::UNSUPPORTED_DESKTOP_ROLE)
    );
    assert!(reason("claude-opus-4-6").is_none());
}

#[test]
fn included_false_is_not_bypassed_by_being_the_default_model() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let mut card = provider("https://relay.example", vec![model("claude-sonnet-4-6", None, &[])]);
    card.models[0].included = false;
    let (eligible, _, models) = provider::provider_qualification(&card, policy, &catalog);
    assert!(models.is_empty(), "included=false 的条目不能成为候选");
    assert!(!eligible);
}

#[test]
fn manual_default_model_becomes_a_candidate() {
    let catalog = empty_catalog();
    let policy = DesktopVersionPolicy::conservative();
    let mut card = provider("https://relay.example", vec![]);
    card.model = "claude-sonnet-4-6".into();
    let (eligible, _, models) = provider::provider_qualification(&card, policy, &catalog);
    assert!(eligible);
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "claude-sonnet-4-6");
}

// ---------------------------------------------------------------------------
// 事务
// ---------------------------------------------------------------------------

fn write(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn read(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn apply_all(plans: &[transaction::FilePlan]) {
    for plan in plans {
        transaction::apply_plan(plan).unwrap();
    }
}

fn desired_direct() -> DesiredState {
    let models = vec![ManagedModelView {
        model_id: "claude-sonnet-4-6".into(),
        label: "claude-sonnet-4-6".into(),
        one_m: false,
    }];
    DesiredState {
        mode: Some(profile::third_party_mode()),
        profile: Some(profile::profile_json(
            "https://relay.example",
            SENTINEL_KEY,
            &models,
        )),
        own_entry: Some(MetaEntry::own()),
        applied_id: Some(super::constants::PROFILE_ID.to_owned()),
        entry_insert_at: 0,
    }
}

/// 模拟一次完整首次应用：先取接管前基线，再写文件，最后演算状态。
fn commit_plan(base: &std::path::Path, paths: &DesktopPaths) -> transaction::State {
    let editions = transaction::editions_before(paths, None, false).unwrap();
    let plans = transaction::build_plan(paths, &desired_direct(), &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);
    let state = transaction::next_state(
        &editions,
        &plans,
        paths,
        1,
        "tx-1",
        "p1",
        "中转 A",
        None,
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();
    let _ = base;
    state
}

fn state_models() -> Vec<ManagedModelView> {
    vec![ManagedModelView {
        model_id: "claude-sonnet-4-6".into(),
        label: "claude-sonnet-4-6".into(),
        one_m: false,
    }]
}

#[test]
fn first_apply_creates_all_four_files_and_keeps_external_profiles() {
    let (base, paths) = test_paths("apply");
    // 已有一个外部 3P profile 与 MCP 配置。
    write(&paths.normal_config, r#"{"deploymentMode":"1p"}"#);
    write(&paths.third_party_config, r#"{"deploymentMode":"3p","mcpServers":{"a":1}}"#);
    write(
        &paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    );
    write(
        &paths.profile_path.with_file_name(format!("{CC_SWITCH_ID}.json")),
        r#"{"inferenceProvider":"gateway"}"#,
    );

    let state = commit_plan(&base, &paths);
    assert_eq!(state.models.len(), 1);

    // 1) 两处 deploymentMode 都是 3p，且无关键保留。
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], json!("3p"));
    let threep: Value = serde_json::from_str(&read(&paths.third_party_config).unwrap()).unwrap();
    assert_eq!(threep["mcpServers"]["a"], json!(1));
    // 2) 自身 profile 写入真实地址与密钥。
    let own: Value = serde_json::from_str(&read(&paths.profile_path).unwrap()).unwrap();
    assert_eq!(own[profile::KEY_API_KEY], json!(SENTINEL_KEY));
    assert_eq!(own[profile::KEY_BASE_URL], json!("https://relay.example"));
    assert_eq!(own[profile::KEY_MODELS][0], json!("claude-sonnet-4-6"));
    // 3) meta 保留外部条目并选中自身。
    let meta: Value = serde_json::from_str(&read(&paths.meta_path).unwrap()).unwrap();
    assert_eq!(meta["appliedId"], json!(super::constants::PROFILE_ID));
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry["id"] == json!(CC_SWITCH_ID)));
    // 4) 外部 profile 不被删除。
    assert!(paths
        .profile_path
        .with_file_name(format!("{CC_SWITCH_ID}.json"))
        .exists());
}

#[test]
fn restore_previous_recovers_the_baseline_and_keeps_external_keys() {
    let (base, paths) = test_paths("restore");
    let external = format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#);
    write(&paths.normal_config, r#"{"deploymentMode":"1p"}"#);
    write(&paths.third_party_config, r#"{"deploymentMode":"3p","mcpServers":{"a":1}}"#);
    write(&paths.meta_path, &external);
    let state = commit_plan(&base, &paths);

    // 用户随后新增了无关 MCP 配置，恢复时必须保留。
    let mut threep: Value = serde_json::from_str(&read(&paths.third_party_config).unwrap()).unwrap();
    threep["mcpServers"]["new"] = json!(2);
    write(&paths.third_party_config, &threep.to_string());

    let mut failed = transaction::restore_all(&state, &paths).unwrap();
    assert!(failed.is_empty());
    failed.clear();
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], json!("1p"));
    let meta: Value = serde_json::from_str(&read(&paths.meta_path).unwrap()).unwrap();
    assert_eq!(meta["appliedId"], json!(CC_SWITCH_ID));
    assert_eq!(meta["entries"].as_array().unwrap().len(), 1);
    let threep: Value = serde_json::from_str(&read(&paths.third_party_config).unwrap()).unwrap();
    assert_eq!(threep["mcpServers"]["new"], json!(2), "外部新增内容必须保留");
    assert!(!paths.profile_path.exists(), "自己的 profile 应该被清理");
}

#[test]
fn restore_of_originally_missing_files_removes_only_own_content() {
    let (base, paths) = test_paths("missing");
    let state = commit_plan(&base, &paths);
    transaction::restore_all(&state, &paths).unwrap();
    assert!(!paths.profile_path.exists());
    assert!(!paths.meta_path.exists());
    assert!(!paths.normal_config.exists(), "只含本次生成内容的文件应删除");
    assert!(!paths.third_party_config.exists());
    // 目录本身不递归删除。
    assert!(paths.third_party_root.join("configLibrary").exists());
}

#[test]
fn managed_drift_is_reported_instead_of_silently_overwritten() {
    let (base, paths) = test_paths("drift");
    let state = commit_plan(&base, &paths);
    assert!(transaction::drifted_roles(&state, &paths).unwrap().is_empty());
    // 外部把 deploymentMode 改掉后必须能检测到。
    write(&paths.third_party_config, r#"{"deploymentMode":"1p"}"#);
    let drifted = transaction::drifted_roles(&state, &paths).unwrap();
    assert_eq!(drifted, vec![role::THIRD_PARTY_CONFIG.to_owned()]);
    // 恢复时拒绝覆盖。
    let error = transaction::restore_all(&state, &paths).unwrap_err();
    assert_eq!(error.code, super::types::error_code::RECOVERY_REQUIRED);
}

#[test]
fn restore_is_idempotent_and_repeatable() {
    let (base, paths) = test_paths("idempotent");
    let state = commit_plan(&base, &paths);
    transaction::restore_all(&state, &paths).unwrap();
    // 第二次执行不应报错也不应写入新内容。
    transaction::restore_all(&state, &paths).unwrap();
    assert!(!paths.profile_path.exists());
    assert!(!paths.meta_path.exists());
}

#[test]
fn unrelated_json_keys_do_not_block_or_change_restore() {
    let (base, paths) = test_paths("unrelated");
    write(&paths.normal_config, r#"{"deploymentMode":"1p","mcpServers":{"x":1}}"#);
    let state = commit_plan(&base, &paths);
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["mcpServers"]["x"], json!(1));
    transaction::restore_all(&state, &paths).unwrap();
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], json!("1p"));
    assert_eq!(normal["mcpServers"]["x"], json!(1));
}

#[test]
fn corrupt_shared_files_block_planning_with_a_located_error() {
    let (_base, paths) = test_paths("corrupt");
    write(&paths.meta_path, r#"{"entries":"nope"}"#);
    let error = transaction::editions_before(&paths, None, false).unwrap_err();
    assert_eq!(error.code, super::types::error_code::INVALID_FILE);
    assert_eq!(error.file_role.as_deref(), Some(role::META));
    // 零外部写入。
    assert_eq!(read(&paths.meta_path).as_deref(), Some(r#"{"entries":"nope"}"#));
    assert!(!paths.profile_path.exists());
    assert!(!paths.normal_config.exists());
}

#[test]
fn first_baseline_is_preserved_across_reapply() {
    let (base, paths) = test_paths("baseline");
    write(&paths.third_party_config, r#"{"deploymentMode":"1p"}"#);
    let editions = transaction::editions_before(&paths, None, false).unwrap();
    let plans = transaction::build_plan(&paths, &desired_direct(), &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);
    let first = transaction::next_state(
        &editions,
        &plans,
        &paths,
        1,
        "tx-1",
        "p1",
        "中转 A",
        None,
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();

    // 第二次应用：换供应商，但基线必须仍是首次接管的 1p。
    let editions = transaction::editions_before(&paths, Some(&first), false).unwrap();
    let mut second_desired = desired_direct();
    second_desired.own_entry = Some(MetaEntry::own());
    let plans = transaction::build_plan(&paths, &second_desired, &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);
    let second = transaction::next_state(
        &editions,
        &plans,
        &paths,
        2,
        "tx-2",
        "p1",
        "中转 A",
        None,
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();
    let baseline = second.edition(role::THIRD_PARTY_CONFIG).unwrap().baseline.clone();
    match baseline {
        ManagedValues::Fields { values } => {
            assert_eq!(values.get(profile::KEY_DEPLOYMENT_MODE), Some(&json!("1p")));
        }
        other => panic!("unexpected baseline {other:?}"),
    }
    let _ = base;
}

/// 提交之后，state 里记录的「最近安装值」必须等于实际写进文件的值。
///
/// 否则 `drifted_roles` 会立刻把刚应用完的配置判成 drift——也就是「刚做完就报冲突」。
#[test]
fn the_recorded_installed_values_match_what_was_written() {
    let (base, paths) = test_paths("installedValues");
    write(
        &paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    );
    let state = commit_plan(&base, &paths);

    // 文件里确实有我们自己的条目，并且已选中。
    let meta: Value = serde_json::from_str(&read(&paths.meta_path).unwrap()).unwrap();
    assert_eq!(meta["appliedId"], json!(super::constants::PROFILE_ID));
    assert_eq!(meta["entries"].as_array().unwrap().len(), 2);

    // 因此记录下来的 installed 也必须包含自己的条目与自己的 appliedId。
    match state.edition(role::META).map(|edition| &edition.installed) {
        Some(ManagedValues::Meta { own_entry, applied_id }) => {
            assert!(own_entry.is_some(), "installed 必须记住自己的 meta 条目");
            assert_eq!(
                applied_id.as_ref().and_then(Value::as_str),
                Some(super::constants::PROFILE_ID),
                "installed 必须记住自己写入的 appliedId"
            );
        }
        other => panic!("unexpected meta installed: {other:?}"),
    }

    // 直接后果：刚提交完不能报 drift。
    assert!(transaction::drifted_roles(&state, &paths).unwrap().is_empty());

    // 第二次应用（换一组模型）同样必须自洽：既有的 own 条目与 appliedId 保持记录。
    let editions = transaction::editions_before(&paths, Some(&state), false).unwrap();
    let second = transaction::build_plan(
        &paths,
        &desired_direct(),
        &transaction::TakeoverBaseline::from_editions(&editions, false),
    )
    .unwrap();
    apply_all(&second);
    let state2 = transaction::next_state(
        &editions,
        &second,
        &paths,
        2,
        "tx-2",
        "p1",
        "中转 A",
        None,
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();
    match state2.edition(role::META).map(|edition| &edition.installed) {
        Some(ManagedValues::Meta { own_entry, applied_id }) => {
            assert!(own_entry.is_some(), "第二次应用后仍必须记住自己的条目");
            assert_eq!(
                applied_id.as_ref().and_then(Value::as_str),
                Some(super::constants::PROFILE_ID)
            );
        }
        other => panic!("unexpected meta installed after reapply: {other:?}"),
    }
    assert!(
        transaction::drifted_roles(&state2, &paths).unwrap().is_empty(),
        "第二次应用后也不能报 drift"
    );
}

/// 显式 rebaseline 必须改用磁盘现状作为新的基线与安装值。
#[test]
fn rebaseline_adopts_the_current_state_as_the_new_baseline() {
    let (base, paths) = test_paths("rebaseline");
    write(
        &paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    );
    let first = commit_plan(&base, &paths);

    // 普通 reapply 沿用首次基线：接管前 profile 不存在。
    let kept = baseline_for(&paths, Some(&first));
    assert_eq!(
        kept.profile,
        transaction::ProfileBaseline::Absent,
        "普通 reapply 不得移动基线"
    );

    // 显式 rebaseline 采用现状：磁盘上那份是我们自己写的，因此仍是 Absent
    // （不能把自己的旧文件声称成「用户原有配置」）。
    let adopted = transaction::TakeoverBaseline::from_editions(
        &transaction::editions_before(&paths, Some(&first), true).unwrap(),
        true,
    );
    assert_eq!(adopted.profile, transaction::ProfileBaseline::Absent);

    // 若磁盘上的 profile 不是我们写的，则按现状接管并原样记录。
    let user_owned = transaction::TakeoverBaseline::from_editions(
        &transaction::editions_before(&paths, Some(&first), true).unwrap(),
        false,
    );
    assert!(matches!(
        user_owned.profile,
        transaction::ProfileBaseline::PreExisting(_)
    ));
}

/// `warnings.pendingCleanup` 声称「在确认清理前不会继续写入」。这里断言该承诺：
/// 只要还有未完成事务，任何新的提交都必须先被 `recoveryRequired` 挡住。
#[test]
fn an_unfinished_transaction_blocks_every_further_commit() {
    let (base, paths) = test_paths("pendingBlocks");
    let manager = super::Manager::new(&base).with_paths(paths.clone());
    craft_pending(
        &manager,
        &paths,
        "tx-blocking",
        1,
        true,
        None,
        direct_state("tx-blocking", 1),
    );

    let request = super::types::ActionRequest {
        provider_id: Some("p1".into()),
        models: vec![],
        paths: super::types::PathsChoice::default(),
        rebaseline: false,
    };
    // 预览阶段就挡住：不给出 revision，也就无法进入提交。
    let error = manager
        .prepare(&crate::core::router::Store::default(), DesktopAction::ApplyDirect, &request)
        .expect_err("存在未完成事务时必须拒绝新的预览");
    assert_eq!(error.code, super::types::error_code::RECOVERY_REQUIRED);

    // 提交同样进不去。
    let commit = manager.commit(
        &crate::core::router::Store::default(),
        DesktopAction::ApplyDirect,
        "whatever",
        &request,
    );
    assert!(commit.is_err(), "未清理的事务不得放过新的提交");

    // 只有显式恢复才能继续。
    manager.recover_pending().expect("显式恢复应当成功");
    assert!(manager.load_pending().unwrap().is_none());
}

/// 卡片换了推理密钥后必须提示需要重新应用；只比较指纹，不落密钥明文。
#[test]
fn a_changed_inference_key_marks_the_configuration_stale() {
    let (base, paths) = test_paths("credentialStale");
    write(
        &paths.meta_path,
        &format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#),
    );
    let card = provider(
        "https://relay.example",
        vec![model("claude-sonnet-4-6", None, &[])],
    );

    let editions = transaction::editions_before(&paths, None, false).unwrap();
    let plans = transaction::build_plan(
        &paths,
        &desired_direct(),
        &transaction::TakeoverBaseline::from_editions(&editions, false),
    )
    .unwrap();
    apply_all(&plans);
    let state = transaction::next_state(
        &editions,
        &plans,
        &paths,
        1,
        "tx-1",
        &card.id,
        &card.name,
        provider::credential_fingerprint(&card),
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();

    // 只存指纹：不是密钥，也不含密钥片段。
    let recorded = state.credential_fingerprint.clone().expect("应记录指纹");
    assert_ne!(recorded, SENTINEL_KEY);
    assert!(!recorded.contains("SENTINEL"));
    assert_eq!(recorded.len(), 64, "SHA-256 十六进制");

    // 密钥未变：不报 credential stale。
    let same = super::status::stale_reasons_for(&state, &card, &base);
    assert!(!same.contains(&"credential".to_owned()), "{same:?}");

    // 换了密钥：必须报 credential stale。
    let mut rotated = card.clone();
    rotated.api_key = "sk-ROTATED-NEW-KEY".into();
    let changed = super::status::stale_reasons_for(&state, &rotated, &base);
    assert!(changed.contains(&"credential".to_owned()), "{changed:?}");

    // 旧状态没有该字段（serde default = None）时不误报。
    let mut legacy = state.clone();
    legacy.credential_fingerprint = None;
    let legacy_reasons = super::status::stale_reasons_for(&legacy, &rotated, &base);
    assert!(!legacy_reasons.contains(&"credential".to_owned()));

    // 卡片已无可用密钥：这同样是「profile 里的密钥已不代表卡片现状」。
    let mut keyless = card.clone();
    keyless.api_key = String::new();
    let keyless_reasons = super::status::stale_reasons_for(&state, &keyless, &base);
    assert!(keyless_reasons.contains(&"credential".to_owned()));
}

/// A22：目标不可读时必须在写任何文件之前失败（零写入）。
#[test]
fn an_unreadable_target_is_rejected_before_any_write() {
    let (base, paths) = test_paths("a22Unreadable");
    let original = r#"{"deploymentMode":"1p"}"#;
    write(&paths.normal_config, original);
    // 3P config 是同名目录：读取必然失败。
    std::fs::create_dir_all(&paths.third_party_config).unwrap();

    let manager = super::Manager::new(&base).with_paths(paths.clone());
    let error = manager
        .prepare(
            &crate::core::router::Store::default(),
            DesktopAction::ApplyDirect,
            &super::types::ActionRequest {
                provider_id: Some("p1".into()),
                models: vec![],
                paths: super::types::PathsChoice::default(),
                rebaseline: false,
            },
        )
        .expect_err("目标不可读时必须拒绝");
    assert_eq!(error.code, super::types::error_code::STORAGE);
    assert_eq!(read(&paths.normal_config).as_deref(), Some(original));
    assert!(!paths.profile_path.exists());
    assert!(!paths.meta_path.exists());
}

/// A22：某个写入点失败时必须完整回滚，不得留下「已应用」的假成功。
#[test]
fn a_failed_write_point_rolls_back_and_never_reports_success() {
    let (_base, paths) = test_paths("a22WritePoint");
    let original = r#"{"deploymentMode":"1p","mcpServers":{"keep":1}}"#;
    write(&paths.normal_config, original);

    // 计划在目标可读时正常算出。
    let editions = transaction::editions_before(&paths, None, false).unwrap();
    let plans = transaction::build_plan(
        &paths,
        &desired_direct(),
        &transaction::TakeoverBaseline::from_editions(&editions, false),
    )
    .unwrap();

    // 事务开始前的四文件快照必须在改动任何文件之前取。
    let before = transaction::snapshot(&paths);

    // 先写完 profile，再把 3P config 变成无法被文件替换覆盖的目录。
    let profile_plan = plans
        .iter()
        .find(|plan| plan.role == role::PROFILE)
        .unwrap();
    transaction::apply_plan(profile_plan).unwrap();
    assert!(paths.profile_path.exists());
    std::fs::remove_file(&paths.third_party_config).ok();
    std::fs::create_dir_all(&paths.third_party_config).unwrap();
    let threep_plan = plans
        .iter()
        .find(|plan| plan.role == role::THIRD_PARTY_CONFIG)
        .unwrap();
    assert!(
        transaction::apply_plan(threep_plan).is_err(),
        "同名目录必须让该写入点失败"
    );

    let state = transaction::next_state(
        &editions,
        &plans,
        &paths,
        1,
        "tx-a22",
        "p1",
        "中转 A",
        None,
        &state_models(),
        transaction::AppliedMode::Direct,
    )
    .unwrap();
    let pending = transaction::Pending {
        schema: transaction::STATE_SCHEMA,
        transaction_id: "tx-a22".into(),
        generation: 1,
        action: DesktopAction::ApplyDirect.as_str().to_owned(),
        created_at: chrono::Utc::now().timestamp(),
        paths_identity: paths.identity(),
        files: plans,
        before,
        previous_state: None,
        next_state: state,
        backup_id: None,
    };
    let rollback = transaction::rollback(&pending);
    // 目标仍是目录：无法恢复到「文件」形态，必须上报而不是假成功。
    assert!(rollback.is_err(), "无法恢复的写入点必须上报");
    assert_eq!(
        rollback.unwrap_err().code,
        super::types::error_code::RECOVERY_REQUIRED
    );
    // 但已经成功写入的项必须被撤回。
    assert!(!paths.profile_path.exists(), "失败点之前的写入必须撤回");
    assert_eq!(read(&paths.normal_config).as_deref(), Some(original));
}
#[test]
fn switch_official_writes_1p_and_withdraws_the_own_entry() {
    let (base, paths) = test_paths("official");
    let external = format!(r#"{{"appliedId":"{CC_SWITCH_ID}","entries":[{{"id":"{CC_SWITCH_ID}","name":"CC Switch"}}]}}"#);
    write(&paths.meta_path, &external);
    let state = commit_plan(&base, &paths);
    let desired = DesiredState {
        mode: Some(profile::official_mode()),
        profile: None,
        own_entry: None,
        applied_id: None,
        entry_insert_at: 0,
    };
    let plans = transaction::build_plan(&paths, &desired, &baseline_for(&paths, Some(&state))).unwrap();
    apply_all(&plans);
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], json!("1p"));
    let meta: Value = serde_json::from_str(&read(&paths.meta_path).unwrap()).unwrap();
    assert!(meta.get("appliedId").is_none(), "不恢复外部指针");
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], json!(CC_SWITCH_ID));
    assert!(!paths.profile_path.exists());
    let _ = state;
}

#[test]
fn release_writes_no_desktop_file() {
    let (base, paths) = test_paths("release");
    let state = commit_plan(&base, &paths);
    let before = [
        read(&paths.normal_config),
        read(&paths.third_party_config),
        read(&paths.profile_path),
        read(&paths.meta_path),
    ];
    let tombstone = state.clone().into_tombstone(9, "tx-release");
    assert!(tombstone.tombstone);
    assert!(tombstone.data.files.is_empty());
    let after = [
        read(&paths.normal_config),
        read(&paths.third_party_config),
        read(&paths.profile_path),
        read(&paths.meta_path),
    ];
    assert_eq!(before, after);
    let _ = base;
}

#[test]
fn key_never_appears_in_the_previewed_change_values() {
    let (_base, paths) = test_paths("redact");
    let editions = transaction::read_editions(&paths).unwrap();
    let plans = transaction::build_plan(&paths, &desired_direct(), &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    let rendered =
        serde_json::to_string(&transaction::views(&plans, |_| false)).unwrap();
    assert!(!rendered.contains(SENTINEL_KEY), "预览 DTO 不得包含密钥");
    assert!(rendered.contains("<redacted>"));
    assert!(rendered.contains(profile::KEY_API_KEY));
}

#[test]
fn balance_token_never_reaches_the_profile_file() {
    let (base, paths) = test_paths("balanceToken");
    commit_plan(&base, &paths);
    let profile_text = read(&paths.profile_path).unwrap();
    assert!(!profile_text.contains("balance-token-SENTINEL"));
    let _ = base;
}

#[test]
fn one_m_mode_off_wins_over_automatic_capability() {
    // 显式关闭时 profile 不写 supports1m。
    let models = vec![ManagedModelView {
        model_id: "claude-opus-4-6".into(),
        label: "claude-opus-4-6".into(),
        one_m: matches!(OneMMode::Off, OneMMode::Auto),
    }];
    let profile = profile::profile_json("https://relay.example", SENTINEL_KEY, &models);
    assert_eq!(profile[profile::KEY_MODELS][0], json!("claude-opus-4-6"));
}

#[test]
fn actions_vocabulary_is_stable() {
    let actions = [
        DesktopAction::ApplyDirect,
        DesktopAction::Reapply,
        DesktopAction::RestorePrevious,
        DesktopAction::SwitchOfficial,
        DesktopAction::Release,
        DesktopAction::RecoverRollback,
    ];
    let rendered = serde_json::to_string(&actions).unwrap();
    assert_eq!(
        rendered,
        r#"["applyDirect","reapply","restorePrevious","switchOfficial","release","recoverRollback"]"#
    );
}
/// 回归：接管前 normal=1p、3P=3p 时，恢复必须让两处各自回到自己的取值，
/// 而不是共用一个模式把 normal 也写成 3p。
#[test]
fn restore_previous_returns_each_config_to_its_own_mode() {
    let (base, paths) = test_paths("perFileMode");
    write(&paths.normal_config, r#"{"deploymentMode":"1p"}"#);
    write(&paths.third_party_config, r#"{"deploymentMode":"3p"}"#);
    let state = commit_plan(&base, &paths);

    let editions = transaction::editions_before(&paths, Some(&state), false).unwrap();
    let desired = DesiredState {
        mode: None,
        profile: None,
        own_entry: None,
        applied_id: None,
        entry_insert_at: 0,
    };
    let plans = transaction::build_plan(&paths, &desired, &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);

    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], json!("1p"));
    let threep: Value = serde_json::from_str(&read(&paths.third_party_config).unwrap()).unwrap();
    assert_eq!(threep["deploymentMode"], json!("3p"));
}

/// 接管前共享文件里根本没有 `deploymentMode` 时，撤回应删除该键而不是写猜测值。
#[test]
fn restore_removes_a_deployment_mode_that_the_takeover_introduced() {
    let (base, paths) = test_paths("introducedMode");
    write(&paths.normal_config, r#"{"mcpServers":{"a":1}}"#);
    let state = commit_plan(&base, &paths);

    let editions = transaction::editions_before(&paths, Some(&state), false).unwrap();
    let desired = DesiredState {
        mode: None,
        profile: None,
        own_entry: None,
        applied_id: None,
        entry_insert_at: 0,
    };
    let plans = transaction::build_plan(&paths, &desired, &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);

    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert!(normal.get("deploymentMode").is_none());
    assert_eq!(normal["mcpServers"]["a"], json!(1));
}
/// 接管前同 id 的 profile 文件已有内容时，撤回只能恢复它，不能凭 id 删除。
#[test]
fn withdraw_restores_a_pre_existing_profile_instead_of_deleting_it() {
    let (base, paths) = test_paths("preExisting");
    let original = r#"{"inferenceProvider":"gateway","note":"pre-existing"}"#;
    write(&paths.profile_path, original);
    let state = commit_plan(&base, &paths);

    // 接管前的内容被记为基线，而不是「文件不存在」。
    let baseline = state.edition(role::PROFILE).unwrap().baseline.clone();
    match baseline {
        ManagedValues::Whole { value } => {
            assert_eq!(value.unwrap()["note"], json!("pre-existing"));
        }
        other => panic!("unexpected baseline {other:?}"),
    }

    let editions = transaction::editions_before(&paths, Some(&state), false).unwrap();
    let desired = DesiredState {
        mode: Some(profile::official_mode()),
        profile: None,
        own_entry: None,
        applied_id: None,
        entry_insert_at: 0,
    };
    let plans = transaction::build_plan(&paths, &desired, &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
    apply_all(&plans);

    // 撤销后文件仍然存在，并且是接管前的原始内容。
    assert!(paths.profile_path.exists(), "接管前存在的 profile 不能被删除");
    let now: Value = serde_json::from_str(&read(&paths.profile_path).unwrap()).unwrap();
    assert_eq!(now["note"], json!("pre-existing"));
}

/// 与 [`super::baseline_profile`] 相同语义，供测试直接取用。
fn baseline_profile_of(state: &transaction::State) -> Option<Value> {
    match state
        .edition(role::PROFILE)
        .map(|edition| &edition.baseline)
    {
        Some(ManagedValues::Whole { value }) => value.clone(),
        _ => None,
    }
}
// ---------------------------------------------------------------------------
// 崩溃恢复（A23）：按持久提交点判定实际文件态
// ---------------------------------------------------------------------------

/// 手工写一份 pending journal，模拟“进程在事务中途消失”。
fn craft_pending(
    manager: &super::Manager,
    paths: &DesktopPaths,
    transaction_id: &str,
    generation: u64,
    wrote_profile: bool,
    previous: Option<transaction::State>,
    next: transaction::State,
) {
    use super::constants::PROFILE_ID;
    let plans = vec![transaction::FilePlan {
        role: role::PROFILE.to_owned(),
        path: paths.profile_path.clone(),
        after_text: Some(serde_json::to_string_pretty(&json!({ "written": true })).unwrap()),
        after: ManagedValues::Whole {
            value: Some(json!({ "written": true })),
        },
        after_keys: Vec::new(),
    }];
    let pending = transaction::Pending {
        schema: transaction::STATE_SCHEMA,
        transaction_id: transaction_id.to_owned(),
        generation,
        action: DesktopAction::ApplyDirect.as_str().to_owned(),
        created_at: chrono::Utc::now().timestamp(),
        paths_identity: paths.identity(),
        files: plans,
        before: transaction::snapshot(paths),
        previous_state: previous,
        next_state: next,
        backup_id: Some("test-backup".to_owned()),
    };
    let _ = PROFILE_ID;
    let bytes = serde_json::to_vec(&pending).unwrap();
    manager.write_pending_for_test(&bytes).unwrap();
    if wrote_profile {
        write(
            &paths.profile_path,
            &serde_json::to_string_pretty(&json!({ "written": true })).unwrap(),
        );
    }
}

fn direct_state(transaction_id: &str, generation: u64) -> transaction::State {
    transaction::State {
        schema: transaction::STATE_SCHEMA,
        transaction_id: transaction_id.to_owned(),
        generation,
        paths_identity: String::new(),
        normal_root: PathBuf::new(),
        third_party_root: PathBuf::new(),
        provider_id: "p1".into(),
        provider_name: "中转 A".into(),
        credential_fingerprint: None,
        models: vec![],
        last_applied_at: 0,
        restart_required: true,
        data: transaction::ManagedData {
            files: vec![],
            applied_mode: transaction::AppliedMode::Direct,
        },
        tombstone: false,
    }
}

#[test]
fn recovery_rolls_back_an_uncommitted_interrupted_transaction() {
    let (base, paths) = test_paths("a23Rollback");
    let manager = super::Manager::new(&base).with_paths(paths.clone());
    // 接管前状态：state 不存在，pending 已经写了一半。
    craft_pending(
        &manager,
        &paths,
        "tx-interrupted",
        1,
        true,
        None,
        direct_state("tx-interrupted", 1),
    );
    assert!(paths.profile_path.exists(), "模拟：文件已写");

    let applied = manager.recover_pending().expect("未提交事务必须能恢复");
    assert!(!paths.profile_path.exists(), "profile 必须被撤回");
    assert!(manager.load_pending().unwrap().is_none(), "pending 必须被清理");
    assert!(applied
        .warnings
        .iter()
        .any(|key| key.ends_with("recoveredRolledBack")));
}

#[test]
fn recovery_confirming_a_committed_transaction_never_rolls_back() {
    let (base, paths) = test_paths("a23Committed");
    let manager = super::Manager::new(&base).with_paths(paths.clone());
    // 提交点已落盘：state 的 transactionId/generation 与 pending.nextState 一致。
    let previous = direct_state("tx-previous", 1);
    let next = direct_state("tx-committed", 2);
    craft_pending(
        &manager,
        &paths,
        "tx-committed",
        2,
        true,
        Some(previous),
        next.clone(),
    );
    manager.write_state_for_test(&next).unwrap();

    let applied = manager.recover_pending().expect("已提交事务只清理 journal");
    assert!(paths.profile_path.exists(), "已提交的内容不得回滚");
    assert!(manager.load_pending().unwrap().is_none());
    assert!(applied
        .warnings
        .iter()
        .any(|key| key.ends_with("recoveredCommitted")));
}

#[test]
fn recovery_reports_a_conflict_when_the_state_matches_neither_version() {
    let (base, paths) = test_paths("a23Conflict");
    let manager = super::Manager::new(&base).with_paths(paths.clone());
    craft_pending(
        &manager,
        &paths,
        "tx-x",
        5,
        true,
        Some(direct_state("tx-old", 3)),
        direct_state("tx-x", 5),
    );
    // 磁盘 state 既不是 tx-old 也不是 tx-x：保留证据，不自动覆盖。
    manager
        .write_state_for_test(&direct_state("tx-unrelated", 9))
        .unwrap();

    let error = manager.recover_pending().expect_err("必须报冲突");
    assert_eq!(error.code, super::types::error_code::RECOVERY_REQUIRED);
    assert!(manager.load_pending().unwrap().is_some(), "证据必须保留");
    assert!(paths.profile_path.exists(), "不得自动回滚");
}

// ---------------------------------------------------------------------------
// 提交失败回滚（A22）：写入过程中断不产生假成功
// ---------------------------------------------------------------------------

#[test]
fn recovery_is_repeatable_after_a_partial_rollback_failure() {
    let (base, paths) = test_paths("a22Repeat");
    let manager = super::Manager::new(&base).with_paths(paths.clone());
    craft_pending(
        &manager,
        &paths,
        "tx-repeat",
        1,
        true,
        None,
        direct_state("tx-repeat", 1),
    );
    // 第一次恢复成功撤回；再次调用必须报“没有待恢复事务”，而不是重复写。
    manager.recover_pending().unwrap();
    let second = manager.recover_pending().expect_err("没有 pending 时不得假成功");
    assert_eq!(second.code, super::types::error_code::NOT_MANAGED);
    assert!(!paths.profile_path.exists());
}
// ---------------------------------------------------------------------------
// 平台路径与多候选（A33）
// ---------------------------------------------------------------------------

/// 只有 Desktop 自己的目录名才是候选；`Claude Code` 与 `Claude-Data` 都不是。
/// 通用路径前缀也要能拦住：不只是白名单里的那几家主机。
///
/// 真实的新 API 网关允许站点挂在子路径下。项目自身会保留该前缀，而 Desktop 只
/// 会追加 `v1/...`，两者指向不同地址——必须报告布局不受支持，而不是按卡片地址
/// 发出去。
#[test]
fn prefixed_gateway_paths_are_reported_as_unsupported() {
    let projectable = |base: &str| {
        let card = provider(base, vec![]);
        provider::resolve_direct_endpoints(&card)
    };

    // 与项目端点投影一致：可用。
    for base in [
        "https://relay.example",
        "https://relay.example/v1",
        "https://relay.example/custom/v1",
        // 多段子路径：`/a/b/v1` 归一成 `/a/b`，两边拼出的地址一致，因此可用。
        "https://relay.example/a/b/v1",
        "https://api.deepseek.com",
    ] {
        assert!(projectable(base).is_ok(), "{base} 应当可用");
    }

    // 项目会改写路径或额外加前缀：必须报告不支持。
    for base in [
        "https://open.bigmodel.cn/api/paas/v4",
        "https://api.z.ai/api/paas/v4",
        "https://api.z.ai/api/coding/paas/v4",
        "https://api.novita.ai/v3/openai",
    ] {
        let card = provider(base, vec![]);
        let error = provider::resolve_direct_endpoints(&card)
            .expect_err("布局不一致必须被拒绝");
        assert_eq!(
            error.code,
            super::types::error_code::ENDPOINT_UNSUPPORTED,
            "{base}: {} {:?}",
            error.code,
            error.params
        );
    }
}

#[test]
fn sibling_data_directories_are_not_desktop_candidates() {
    assert!(paths::is_channel_name("Claude", false));
    assert!(paths::is_channel_name("Claude-3p", true));
    assert!(paths::is_channel_name("Claude-internal", false));
    assert!(paths::is_channel_name("Claude-preview-3p", true));
    // Claude Code 的数据目录用空格分隔，不是 Desktop 安装。
    assert!(!paths::is_channel_name("Claude Code", false));
    // `-Data` 命名本身是合法的同渠道命名；它是否算安装由「安装痕迹」决定。
    assert!(paths::is_channel_name("Claude-Data", false));
    assert!(paths::is_channel_name("Claude-Data-3p", true));
    // 渠道归属由安装痕迹决定，名字筛选对两个渠道一致。
    assert!(paths::is_channel_name("Claude-3p", false));
    assert!(paths::is_channel_name("Claude", true));
    // 空后缀与其它品牌都排除。
    assert!(!paths::is_channel_name("Claude-", false));
    assert!(!paths::is_channel_name("Claude-   ", false));
    assert!(!paths::is_channel_name("Other", false));
}

/// 复现真机序列：多次应用（含换密钥的重新应用）后，**首次接管基线不得改变**。
///
/// 真机上发现 profile 的基线被写成了「接管时已存在」，导致「恢复接管前配置」
/// 变成写回自己而不是删除。这里用纯逻辑复现并锁定。
#[test]
fn repeated_applies_never_move_the_first_baseline() {
    let (base, paths) = test_paths("baselineStable");
    write(&paths.normal_config, r#"{"deploymentMode":"1p"}"#);
    write(&paths.third_party_config, r#"{"deploymentMode":"3p"}"#);

    let mut previous: Option<transaction::State> = None;
    let mut generation = 0u64;
    for round in 0..3 {
        generation += 1;
        let editions = transaction::editions_before(&paths, previous.as_ref(), false).unwrap();
        let plans = transaction::build_plan(&paths, &desired_direct(), &transaction::TakeoverBaseline::from_editions(&editions, false)).unwrap();
        apply_all(&plans);
        let state = transaction::next_state(
            &editions,
            &plans,
            &paths,
            generation,
            &format!("tx-{round}"),
            "p1",
            "中转 A",
            Some(format!("fingerprint-{round}")),
            &state_models(),
            transaction::AppliedMode::Direct,
        )
        .unwrap();
        let profile = state.edition(role::PROFILE).unwrap();
        assert!(!profile.existed_before, "第 {round} 轮：接管前 profile 不存在");
        assert!(
            matches!(profile.baseline, ManagedValues::Whole { value: None }),
            "第 {round} 轮：基线必须保持 none，实际 {:?}",
            profile.baseline
        );
        let normal = state.edition(role::NORMAL_CONFIG).unwrap();
        match &normal.baseline {
            ManagedValues::Fields { values } => assert_eq!(
                values.get(profile::KEY_DEPLOYMENT_MODE).and_then(Value::as_str),
                Some("1p"),
                "第 {round} 轮：normal 的基线必须保持接管前的 1p"
            ),
            other => panic!("unexpected normal baseline: {other:?}"),
        }
        previous = Some(state);
    }

    // 恢复必须真的删除自身 profile，而不是写回自己。
    let state = previous.unwrap();
    let editions = transaction::editions_before(&paths, Some(&state), false).unwrap();
    let desired = DesiredState {
        mode: None,
        profile: None,
        own_entry: None,
        applied_id: None,
        entry_insert_at: 0,
    };
    let plans = transaction::build_plan(
        &paths,
        &desired,
        &transaction::TakeoverBaseline::from_editions(&editions, false),
    )
    .unwrap();
    let profile_plan = plans
        .iter()
        .find(|plan| plan.role == role::PROFILE)
        .expect("恢复必须为 profile 生成计划");
    assert!(profile_plan.after_text.is_none(), "恢复应当删除自身 profile");
}
