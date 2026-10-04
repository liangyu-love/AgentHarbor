//! Claude Desktop 直连：只读状态投影。
//!
//! 这里把「磁盘上的四文件 + 内部记录 + 供应商列表」整理成卡片要显示的状态。
//! 它**不做任何写入**，也不规划事务——那是 `transaction` 与 `mod::Manager` 的职责。
//! 单独成模块是为了让读路径与写路径分开：排查「状态显示不对」时不必读事务代码。

use super::desktop_app;
use super::paths::{self, DesktopPaths};
use super::transaction;
use super::types::{DesktopAction, DesktopActionView, DesktopDetection, DesktopStatus};
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

    let paths = state.map(state_paths).or(fallback_paths);
    let mode = paths
        .as_ref()
        .map(read_mode)
        .unwrap_or_else(|| "unknown".to_owned());

    let provider_id = state.map(|state| state.provider_id.clone());
    let provider = provider_id
        .as_ref()
        .and_then(|id| store.providers.iter().find(|entry| &entry.id == id));
    let provider_missing =
        provider_id.as_ref().is_some_and(|id| !id.is_empty() && provider.is_none());

    let stale_reasons = match (state, provider) {
        (Some(state), Some(provider)) => stale_reasons_for(state, provider, app_data_dir),
        _ => Vec::new(),
    };

    if let (Some(state), Some(paths)) = (state, &paths) {
        match transaction::drifted_roles(state, paths) {
            Ok(roles) => conflicts.extend(roles.iter().map(|name| format!("drift:{name}"))),
            Err(error) => warnings.push(error.message.clone()),
        }
    }

    let recovery_state = pending
        .map(|pending| format!("{}:{}", pending.action, pending.transaction_id));
    if pending.is_some() {
        warnings.push("claudeDesktop.warnings.pendingRecovery".to_owned());
    }
    if state.is_some_and(|state| state.tombstone) {
        warnings.push("claudeDesktop.warnings.released".to_owned());
    }
    if open_previews {
        warnings.push("claudeDesktop.warnings.openPreviews".to_owned());
    }
    if isolated {
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
            .or_else(|| state.map(|state| state.provider_name.clone())),
        provider_missing,
        models: state.map(|state| state.models.clone()).unwrap_or_default(),
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
        isolated: false,
        overridden: false,
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
    let Ok(map) = super::profile::parse_object(Some(&text), super::paths::role::THIRD_PARTY_CONFIG) else {
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
    let catalog = models_dev::load(app_data_dir);
    // 版本只影响可用角色；stale 判定不用版本策略，否则一次版本探测差异
    // 会凭空制造 stale。
    let policy = super::provider::DesktopVersionPolicy::from_version(None);
    for model in &state.models {
        let candidate = super::provider::model_candidate(provider, &model.model_id, &catalog, policy);
        if candidate.exclude_reason.is_some() {
            reasons.push(format!("model:{}", model.model_id));
            continue;
        }
        // 用户显式填写的展示名被固定下来：卡片改名不应让它变成 stale。
        let pinned = model.label != candidate.label && model.label != model.model_id;
        if !pinned && candidate.label != model.label {
            reasons.push(format!("label:{}", model.model_id));
        }
        if model.one_m && !candidate.supports1m {
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
