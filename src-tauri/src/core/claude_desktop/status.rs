//! Claude Desktop 直连：只读状态投影。
//!
//! 这里把「磁盘上的四文件 + 内部记录 + 供应商列表」整理成卡片要显示的状态。
//! 它**不做任何写入**，也不规划事务——那是 `transaction` 与 `mod::Manager` 的职责。
//! 单独成模块是为了让读路径与写路径分开：排查「状态显示不对」时不必读事务代码。

use super::desktop_app;
use super::paths::{self, DesktopPaths};
use super::transaction;
use super::types::{DesktopAction, DesktopActionView, DesktopDetection, DesktopStatus, OneMMode};
use crate::core::models_dev;
use crate::core::router::{Provider, Store};

/// 只读状态投影的全部输入。
pub struct StatusInput<'a> {
    pub detection: DesktopDetection,
    pub state: Option<&'a transaction::State>,
    pub pending: Option<&'a transaction::Pending>,
    /// 是否存在未消费的预览 ticket。
    pub open_previews: bool,
    pub store: &'a Store,
    pub app_data_dir: &'a std::path::Path,
    /// 没有管理记录时用于展示的路径（来自调用方的发现结果）。
    pub fallback_paths: Option<DesktopPaths>,
    /// 路径是否来自隔离根（用于提示）。
    pub isolated: bool,
    pub backup_count: usize,
}

/// 依据输入算出一份完整状态。
pub fn project(input: StatusInput<'_>) -> DesktopStatus {
    let StatusInput {
        detection,
        state,
        pending,
        open_previews,
        store,
        app_data_dir,
        fallback_paths,
        isolated,
        backup_count,
    } = input;

    let mut warnings: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    let mut paths = state.map(state_paths).or(fallback_paths);
    // 切换隔离根后，旧生产会话的状态查询也不能读取真实 Desktop 文件。
    if paths::isolated_home().is_some()
        && paths
            .as_ref()
            .is_some_and(|paths| paths::validate_session_paths(paths).is_err())
    {
        conflicts.push("paths".to_owned());
        paths = None;
    }
    let mode = paths
        .as_ref()
        .map(read_mode)
        .unwrap_or_else(|| "unknown".to_owned());

    let active_state = state.filter(|state| !state.tombstone);
    let provider_id = active_state.map(|state| state.provider_id.clone());
    let provider = provider_id
        .as_ref()
        .and_then(|id| store.providers.iter().find(|entry| &entry.id == id));
    let provider_missing = provider_id
        .as_ref()
        .is_some_and(|id| !id.is_empty() && provider.is_none());

    let stale_reasons = match (active_state, provider) {
        (Some(state), Some(provider)) => stale_reasons_for(state, provider, app_data_dir),
        _ => Vec::new(),
    };

    if let (Some(state), Some(paths)) = (active_state, &paths) {
        match transaction::drifted_roles(state, paths) {
            Ok(roles) => conflicts.extend(roles.iter().map(|name| format!("drift:{name}"))),
            Err(error) => warnings.push(error.message.clone()),
        }
    }

    let recovery_state =
        pending.map(|pending| format!("{}:{}", pending.action, pending.transaction_id));
    if pending.is_some() {
        warnings.push("claudeDesktop.warnings.pendingRecovery".to_owned());
    }
    if state.is_some_and(|state| state.tombstone) {
        warnings.push("claudeDesktop.warnings.released".to_owned());
    }
    if open_previews {
        warnings.push("claudeDesktop.warnings.openPreviews".to_owned());
    }
    if isolated || paths.as_ref().is_some_and(|paths| paths.isolated) {
        warnings.push("claudeDesktop.warnings.isolated".to_owned());
    }

    let is_managed = state.is_some_and(|state| !state.tombstone);
    // 主显示状态按优先级取第一个成立项；其余事实另设字段，不做成互斥枚举。
    let management_state = if pending.is_some() {
        "recoveryRequired"
    } else if !conflicts.is_empty() {
        "conflict"
    } else if provider_missing {
        "orphaned"
    } else if !stale_reasons.is_empty() {
        "stale"
    } else if is_managed {
        "managed"
    } else {
        "unmanaged"
    }
    .to_owned();

    let actions = action_views(state, pending.is_some(), provider_missing, &conflicts);
    // 应用进程状态单独探测：写文件成功与「应用正在用它」是两件事。
    let app = paths
        .as_ref()
        .map(desktop_app::state)
        .unwrap_or(desktop_app::AppState {
            running: false,
            process_count: 0,
            launch_method: None,
            launch_target: None,
        });

    DesktopStatus {
        detection,
        paths: paths.as_ref().map(DesktopPaths::summary),
        mode,
        management_state,
        is_managed,
        provider_id,
        provider_name: provider
            .map(|provider| provider.name.clone())
            .or_else(|| active_state.map(|state| state.provider_name.clone())),
        provider_missing,
        models: active_state
            .map(|state| state.models.clone())
            .unwrap_or_default(),
        model_choices: active_state
            .map(|state| state.model_choices.clone())
            .unwrap_or_default(),
        stale_reasons,
        conflicts,
        recovery_state,
        pending_recovery: pending.is_some(),
        warnings,
        last_applied_at: state.map(|state| state.last_applied_at),
        restart_required: state.is_some_and(|state| state.restart_required),
        app_running: app.running,
        restart_capable: app.launch_method.is_some(),
        launch_method: app.launch_method.map(|method| method.as_str().to_owned()),
        actions,
        backup_count,
    }
}

/// 由已保存状态重建四文件路径；重启时用它，避免重新发现到另一个安装。
pub fn state_paths(state: &transaction::State) -> DesktopPaths {
    DesktopPaths {
        normal_root: state.normal_root.clone(),
        third_party_root: state.third_party_root.clone(),
        normal_config: state.normal_root.join(super::constants::CONFIG_FILE),
        third_party_config: state.third_party_root.join(super::constants::CONFIG_FILE),
        profile_path: state
            .third_party_root
            .join(super::constants::CONFIG_LIBRARY_DIR)
            .join(format!("{}.json", super::constants::PROFILE_ID)),
        meta_path: state
            .third_party_root
            .join(super::constants::CONFIG_LIBRARY_DIR)
            .join(super::constants::META_FILE),
        // 旧记录的 identity 已含隔离来源，不能因新增字段缺失变成生产路径。
        isolated: state.paths_isolated || state.paths_identity.ends_with("|isolated"),
        overridden: state.paths_overridden,
    }
}

/// 两处 config 里记录的部署模式，用于把「官方 / 第三方 / 本工具直连 / 未知」区分开。
pub fn read_mode(paths: &DesktopPaths) -> String {
    let Some(text) = transaction::read_text(&paths.third_party_config)
        .ok()
        .flatten()
    else {
        return "unknown".to_owned();
    };
    let Ok(map) = super::profile::parse_object(Some(&text), super::paths::role::THIRD_PARTY_CONFIG)
    else {
        return "unknown".to_owned();
    };
    match super::profile::deployment_mode(&map) {
        Some(value) if value == super::profile::official_mode() => "official".to_owned(),
        Some(_) => {
            let applied = transaction::read_text(&paths.meta_path)
                .ok()
                .flatten()
                .and_then(|text| super::profile::MetaDocument::parse(Some(&text)).ok())
                .and_then(|meta| meta.applied_id_str().map(str::to_owned));
            match applied.as_deref() {
                Some(id) if id == super::constants::PROFILE_ID => "direct".to_owned(),
                _ => "foreignThirdParty".to_owned(),
            }
        }
        None => "unknown".to_owned(),
    }
}

/// 与磁盘现状相比，哪些受管目标需要重新应用。
pub fn stale_reasons_for(
    state: &transaction::State,
    provider: &Provider,
    app_data_dir: &std::path::Path,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if state.provider_name != provider.name {
        reasons.push("name".to_owned());
    }
    // 推理密钥变了：Desktop 的 profile 里仍是旧快照，必须提示重新应用。
    // 只比较指纹，不比较密钥本身。旧状态没有指纹时不报（宁可漏报也不误报）。
    if let Some(recorded) = state.credential_fingerprint.as_deref() {
        if super::provider::credential_fingerprint(provider).as_deref() != Some(recorded) {
            reasons.push("credential".to_owned());
        }
    }
    // 旧记录也可从受保护的安装投影中读取地址，避免无声遗漏上游变更。
    let recorded_endpoint = state.profile_base_url.as_deref().or_else(|| {
        state
            .edition(paths::role::PROFILE)
            .and_then(|edition| match &edition.installed {
                transaction::ManagedValues::Whole { value: Some(value) } => value
                    .get(super::profile::KEY_BASE_URL)
                    .and_then(serde_json::Value::as_str),
                _ => None,
            })
    });
    match super::provider::endpoint_view(provider) {
        Ok(endpoint)
            if recorded_endpoint.is_some_and(|value| value != endpoint.profile_base_url) =>
        {
            reasons.push("endpoint".to_owned());
        }
        Err(_) => reasons.push("endpoint".to_owned()),
        _ => {}
    }
    let catalog = models_dev::load(app_data_dir);
    // 已安装的模型通过过版本门禁；这里只检查供应商变化，不再次用未知版本
    // 将合法 Fable 排除。新应用的版本门禁仍由 prepare 执行。
    let policy = super::provider::DesktopVersionPolicy {
        fable_allowed: true,
    };
    for model in &state.models {
        let candidate =
            super::provider::model_candidate(provider, &model.model_id, &catalog, policy);
        if candidate.exclude_reason.is_some() {
            reasons.push(format!("model:{}", model.model_id));
            continue;
        }
        let choice = state
            .model_choices
            .iter()
            .find(|choice| choice.model_id == model.model_id);
        // 新记录按原始选择判定；旧记录保留已固定的展示名，升级不改用户配置。
        let pinned = choice
            .map(|choice| choice.label_override.is_some())
            .unwrap_or_else(|| model.label != candidate.label && model.label != model.model_id);
        if !pinned && candidate.label != model.label {
            reasons.push(format!("label:{}", model.model_id));
        }
        let context_changed = match choice.map(|choice| choice.one_m_mode) {
            Some(OneMMode::Auto) => model.one_m != candidate.supports1m,
            Some(OneMMode::Off) => model.one_m,
            None => model.one_m && !candidate.supports1m,
        };
        if context_changed {
            reasons.push(format!("context:{}", model.model_id));
        }
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

/// 每个操作是否可用，以及不可用的原因。由真实条件计算，不按主状态猜。
fn action_views(
    state: Option<&transaction::State>,
    pending: bool,
    provider_missing: bool,
    conflicts: &[String],
) -> Vec<DesktopActionView> {
    let unmanaged = state.is_none();
    let released = state.is_some_and(|state| state.tombstone);
    let conflict_block = (!conflicts.is_empty()).then_some("claudeDesktop.actionReasons.conflict");
    let pending_block = pending.then_some("claudeDesktop.actionReasons.pendingRecovery");
    let not_managed = (unmanaged || released).then_some("claudeDesktop.actionReasons.notManaged");
    let view = |action: DesktopAction, disabled_reason: Option<&'static str>| DesktopActionView {
        action,
        enabled: disabled_reason.is_none(),
        disabled_reason: disabled_reason.map(str::to_owned),
    };
    vec![
        view(DesktopAction::ApplyDirect, pending_block),
        view(
            DesktopAction::Reapply,
            pending_block.or(not_managed).or(if provider_missing {
                Some("claudeDesktop.actionReasons.providerMissing")
            } else {
                conflict_block
            }),
        ),
        view(
            DesktopAction::RestorePrevious,
            pending_block.or(not_managed),
        ),
        view(
            DesktopAction::SwitchOfficial,
            pending_block.or(not_managed).or(conflict_block),
        ),
        view(DesktopAction::Release, pending_block.or(not_managed)),
        view(
            DesktopAction::RecoverRollback,
            if pending {
                None
            } else {
                Some("claudeDesktop.actionReasons.noPending")
            },
        ),
    ]
}

/// 供 Manager 复用的「没有管理记录时」的路径提示。
pub fn describe_isolated(paths: &DesktopPaths) -> bool {
    paths.isolated || paths::isolated_home().is_some()
}

#[cfg(test)]
mod projection_tests {
    use super::super::types::{DesktopModelChoice, ManagedModelView};
    use super::*;
    use crate::core::newapi;

    fn fixture(model_id: &str) -> (std::path::PathBuf, Provider, transaction::State) {
        let root = std::env::temp_dir().join(format!("desktop-status-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Claude")).unwrap();
        let mut paths =
            DesktopPaths::from_roots(root.join("Claude"), root.join("Claude-3p")).unwrap();
        paths.overridden = true;
        let provider = Provider {
            id: "status-fixture".into(),
            name: "Fixture".into(),
            base_url: "https://relay.example".into(),
            api_key: "synthetic-status-key".into(),
            model: model_id.into(),
            models: vec![newapi::Model {
                id: model_id.into(),
                overrides: newapi::ModelOverrides {
                    context_window: Some(100_000),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut state = transaction::next_state(
            &[],
            &[],
            &paths,
            1,
            "fixture-tx",
            &provider.id,
            &provider.name,
            super::super::provider::credential_fingerprint(&provider),
            &[ManagedModelView {
                model_id: model_id.into(),
                label: model_id.into(),
                one_m: false,
            }],
            transaction::AppliedMode::Direct,
        )
        .unwrap();
        state.model_choices = vec![DesktopModelChoice {
            model_id: model_id.into(),
            label_override: None,
            one_m_mode: OneMMode::Auto,
        }];
        state.profile_base_url = Some(
            super::super::provider::endpoint_view(&provider)
                .unwrap()
                .profile_base_url,
        );
        (root, provider, state)
    }

    #[test]
    fn endpoint_and_removed_or_excluded_models_are_stale() {
        let (root, card, state) = fixture("claude-sonnet-4-6");
        assert!(stale_reasons_for(&state, &card, &root).is_empty());
        let mut changed = card.clone();
        changed.base_url = "https://another-relay.example".into();
        assert!(stale_reasons_for(&state, &changed, &root).contains(&"endpoint".into()));
        changed = card.clone();
        changed.models[0].included = false;
        assert!(
            stale_reasons_for(&state, &changed, &root).contains(&"model:claude-sonnet-4-6".into())
        );
        changed.models.clear();
        changed.model = "claude-haiku-4-5".into();
        assert!(
            stale_reasons_for(&state, &changed, &root).contains(&"model:claude-sonnet-4-6".into())
        );
    }

    #[test]
    fn auto_capability_changes_and_labels_use_original_choices() {
        let (root, mut card, mut state) = fixture("claude-sonnet-4-6");
        card.models[0].overrides.context_window = Some(1_000_000);
        assert!(
            stale_reasons_for(&state, &card, &root).contains(&"context:claude-sonnet-4-6".into())
        );
        state.model_choices[0].one_m_mode = OneMMode::Off;
        assert!(
            !stale_reasons_for(&state, &card, &root).contains(&"context:claude-sonnet-4-6".into())
        );
        card.models[0].display_name = "Updated label".into();
        assert!(stale_reasons_for(&state, &card, &root).contains(&"label:claude-sonnet-4-6".into()));
        state.model_choices[0].label_override = Some(state.models[0].label.clone());
        assert!(
            !stale_reasons_for(&state, &card, &root).contains(&"label:claude-sonnet-4-6".into())
        );
    }

    #[test]
    fn installed_fable_is_not_rejected_by_an_unknown_detection_version() {
        let (root, card, state) = fixture("claude-fable-5");
        assert!(stale_reasons_for(&state, &card, &root).is_empty());
    }

    #[test]
    fn a_legacy_isolated_identity_remains_isolated() {
        let (_, _, mut state) = fixture("claude-sonnet-4-6");
        state.paths_identity = format!(
            "{}|{}|isolated",
            paths::display(&state.normal_root),
            paths::display(&state.third_party_root)
        );
        state.paths_isolated = false;
        assert!(state_paths(&state).isolated);
        state.paths_overridden = true;
        assert!(state_paths(&state).overridden);
    }

    #[test]
    fn ended_sessions_do_not_report_orphaned_or_stale_provider_state() {
        let _env_guard = super::super::env_test_lock();
        let (root, _, mut state) = fixture("claude-sonnet-4-6");
        state.tombstone = true;
        let status = project(StatusInput {
            detection: DesktopDetection {
                platform: "fixture".into(),
                version: "unknown".into(),
                version_source: "fixture".into(),
                installed: true,
                notes: vec![],
            },
            state: Some(&state),
            pending: None,
            open_previews: false,
            store: &Store::default(),
            app_data_dir: &root,
            fallback_paths: None,
            isolated: false,
            backup_count: 0,
        });
        assert!(!status.is_managed);
        assert!(!status.provider_missing);
        assert_eq!(status.management_state, "unmanaged");
        assert!(status.models.is_empty() && status.model_choices.is_empty());
        assert!(status.conflicts.is_empty() && status.stale_reasons.is_empty());
    }
}
