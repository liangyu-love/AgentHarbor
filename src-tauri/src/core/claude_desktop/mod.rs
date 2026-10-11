//! Claude Desktop 直连：状态、预览 ticket、提交与恢复的编排层。
//!
//! 这里负责把纯逻辑（`provider`/`profile`/`paths`）与事务（`transaction`）
//! 串成用户可预期的操作序列，并维护 Desktop 独立的内部状态文件：
//!
//! ```text
//! <app_data_dir>/claude-desktop/
//!   state.dat                    # schemaVersion、路径、请求引用、接管基线、owned patches
//!   pending.dat                  # 未完成事务：旧/新 state 与四文件 before/after
//!   previews.dat                 # 未消费的预览 ticket
//!   backups/<transactionId>.dat  # 人工恢复备份
//! ```
//!
//! 所有含文件快照或凭据的记录都经 `secret_file` 写入（Windows 为当前用户 DPAPI；
//! Unix 为 0600）。**现有工具在 Unix 不提供内容加密，因此不声称全平台加密。**

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::models_dev;
use crate::core::router::{Provider, Store};
use crate::core::secret_file;

pub mod capability;
pub mod constants;
pub mod desktop_app;
pub mod gateway;
pub mod lock;
pub mod paths;
pub mod probe;
pub mod profile;
pub mod provider;
pub mod routes;
pub mod status;
#[cfg(test)]
mod tests;
pub mod tickets;
pub mod transaction;
pub mod types;

use constants::{DATA_DIR_NAME, PROFILE_ID};
use paths::{role, DesktopPaths};
use profile::MetaEntry;
use transaction::{AppliedMode, DesiredState, Edition, ManagedValues, Pending, State};
use types::{
    error_code, ActionRequest, ApplyDirectRequest, DesktopAction, DesktopApplied,
    DesktopCandidates, DesktopDetection, DesktopError, DesktopPreview, DesktopResult,
    DesktopStatus, ManagedModelView, OneMMode, PathsChoice, ProviderCandidateView,
};

const STATE_FILE: &str = "state.dat";
const PENDING_FILE: &str = "pending.dat";
const PREVIEWS_FILE: &str = "previews.dat";

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 进程级环境变量（`AGENT_HARBOR_CLIENT_HOME`）是全局状态：凡是会影响它、
/// 或依赖它读取结果的测试都必须串行，否则会互相看到对方设置的隔离根。
#[cfg(test)]
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

// ---------------------------------------------------------------------------
// 内部记录
// ---------------------------------------------------------------------------

/// 已固化的变更：包含四个文件的写入计划与将要保存的新状态。
#[derive(Debug)]
pub struct PreparedChange {
    pub action: DesktopAction,
    pub paths: DesktopPaths,
    pub plans: Vec<transaction::FilePlan>,
    pub before: Vec<transaction::FileSnapshot>,
    pub previous_state: Option<State>,
    pub next_state: State,
    pub revision: String,
    pub warnings: Vec<String>,
    pub conflicts: Vec<String>,
    pub models: Vec<ManagedModelView>,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub messages_url: Option<String>,
    pub models_url: Option<String>,
    pub external_profile_count: usize,
    pub managed_entry_present: bool,
    pub current_applied_id: Option<String>,
    pub requires_restart: bool,
    pub request: ActionRequest,
    pub listener_lease: Option<types::ListenerLease>,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Manager {
    app_data_dir: PathBuf,
    /// 由命令层从 `AppHandle` 注入的 Desktop 版本提示。
    version_hint: Option<String>,
    /// 显式路径覆盖；只用于隔离的集成测试，生产路径始终由发现决定。
    paths_override: Option<DesktopPaths>,
    listener_lease: Option<types::ListenerLease>,
    #[cfg(test)]
    stop_check: Option<std::sync::Arc<dyn Fn(&DesktopPaths) -> DesktopResult<()> + Send + Sync>>,
    #[cfg(test)]
    commit_check: Option<std::sync::Arc<dyn Fn(&DesktopPaths) -> DesktopResult<()> + Send + Sync>>,
}

impl Manager {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            app_data_dir: app_data_dir.to_path_buf(),
            version_hint: None,
            paths_override: None,
            listener_lease: None,
            #[cfg(test)]
            stop_check: None,
            #[cfg(test)]
            commit_check: None,
        }
    }

    pub fn with_version_hint(mut self, version: Option<String>) -> Self {
        self.version_hint = version;
        self
    }

    pub fn with_listener_lease(mut self, lease: types::ListenerLease) -> Self {
        self.listener_lease = Some(lease);
        self
    }

    /// 固定四文件路径，跳过平台发现。命令层不使用它；覆盖后**不得**触碰本机
    /// 真实安装（不查询 MSIX、不启动应用）。
    pub fn with_paths(mut self, mut paths: DesktopPaths) -> Self {
        paths.overridden = true;
        self.paths_override = Some(paths);
        self
    }

    /// 显式提供版本证据，供隔离测试避免读取生产日志。
    pub fn with_detected_version(mut self, version: Option<String>) -> Self {
        self.version_hint = version;
        self
    }

    pub fn data_dir(&self) -> PathBuf {
        self.app_data_dir.join(DATA_DIR_NAME)
    }

    fn state_path(&self) -> PathBuf {
        self.data_dir().join(STATE_FILE)
    }

    fn pending_path(&self) -> PathBuf {
        self.data_dir().join(PENDING_FILE)
    }

    fn previews_path(&self) -> PathBuf {
        self.data_dir().join(PREVIEWS_FILE)
    }

    pub fn load_state(&self) -> DesktopResult<Option<State>> {
        read_secret_json(&self.state_path())
    }

    pub fn load_pending(&self) -> DesktopResult<Option<Pending>> {
        read_secret_json(&self.pending_path())
    }

    fn save_state(&self, state: &State) -> DesktopResult<()> {
        write_secret_json(&self.state_path(), state)
    }

    fn save_pending(&self, pending: Option<&Pending>) -> DesktopResult<()> {
        match pending {
            Some(pending) => write_secret_json(&self.pending_path(), pending),
            None => clear_secret_file(&self.pending_path()),
        }
    }

    fn write_lock() -> DesktopResult<MutexGuard<'static, ()>> {
        WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| transaction::storage_error())
    }

    // -----------------------------------------------------------------------
    // 发现与状态
    // -----------------------------------------------------------------------

    /// 路径发现结果。显式覆盖（隔离测试）时**完全不扫描平台目录**：
    /// 那既会读到生产环境，也会被进程级环境变量影响，从而使测试互相干扰。
    fn discovery(&self) -> paths::Discovery {
        match &self.paths_override {
            Some(paths) => paths::Discovery {
                candidates: vec![paths::PathCandidate {
                    paths: paths.clone(),
                    install_evidence: paths.normal_has_install_evidence(),
                }],
                default: Some(paths.clone()),
                notes: Vec::new(),
            },
            None => paths::discover(),
        }
    }

    pub fn detection(&self) -> DesktopDetection {
        detect_desktop(&self.discovery(), self.version_hint.as_deref())
    }

    pub fn resolve_paths(&self, choice: &PathsChoice) -> DesktopResult<DesktopPaths> {
        if let Some(paths) = &self.paths_override {
            return Ok(paths.clone());
        }
        paths::resolve(choice)
    }

    /// 一个管理会话只负责最初选中的安装。不能通过恢复/重应用请求搬到别的根。
    fn session_paths(
        &self,
        active: Option<&State>,
        choice: &PathsChoice,
    ) -> DesktopResult<DesktopPaths> {
        let Some(state) = active else {
            return self.resolve_paths(choice);
        };
        let mut saved = paths_from_state(state);
        if let Some(overridden) = &self.paths_override {
            if saved.identity() != overridden.identity() {
                return Err(changed_error());
            }
            saved.overridden = true;
        } else {
            paths::validate_session_paths(&saved)?;
        }
        if *choice != PathsChoice::default() {
            let normal = choice
                .normal_root
                .as_ref()
                .map(PathBuf::from)
                .map(|path| paths::normalize(&path));
            let third_party = choice
                .third_party_root
                .as_ref()
                .map(PathBuf::from)
                .map(|path| paths::normalize(&path));
            if normal.as_ref() != Some(&saved.normal_root)
                || third_party.as_ref() != Some(&saved.third_party_root)
            {
                return Err(changed_error());
            }
        }
        Ok(saved)
    }

    fn ensure_stopped(&self, paths: &DesktopPaths) -> DesktopResult<()> {
        self.ensure_stopped_with(paths, &desktop_app::StopCheck::new(paths))
    }

    fn ensure_stopped_with(
        &self,
        _paths: &DesktopPaths,
        check: &desktop_app::StopCheck,
    ) -> DesktopResult<()> {
        #[cfg(test)]
        if let Some(check) = &self.stop_check {
            return check(_paths);
        }
        check.ensure_stopped()
    }

    fn confirm_shutdown(&self, _paths: &DesktopPaths, check: &desktop_app::StopCheck) -> DesktopResult<()> {
        #[cfg(test)]
        if let Some(check) = &self.stop_check { return check(_paths); }
        check.ensure_stopped_after_exit()
    }

    pub fn candidates(
        &self,
        store: &Store,
        choice: &PathsChoice,
    ) -> DesktopResult<DesktopCandidates> {
        let discovery = self.discovery();
        // 显式覆盖（隔离测试）优先；生产路径始终来自发现。
        let paths = if self.paths_override.is_none()
            && *choice == PathsChoice::default()
            && discovery.default.is_none()
        {
            None
        } else {
            Some(self.resolve_paths(choice)?)
        };
        let detection = self.detection();
        let policy = provider::DesktopVersionPolicy::from_version(Some(&detection.version));
        let catalog = models_dev::load(&self.app_data_dir);
        let mut views = Vec::with_capacity(store.providers.len());
        for candidate in store.providers.iter().filter(|candidate| !candidate.router_only()) {
            let (eligible, reason, models) =
                provider::provider_qualification(candidate, policy, &catalog);
            views.push(ProviderCandidateView {
                provider_id: candidate.id.clone(),
                name: candidate.name.clone(),
                enabled: candidate.enabled,
                has_inference_key: provider::inference_key(candidate).is_some(),
                eligible,
                reason,
                endpoint: provider::endpoint_view(candidate).ok(),
                models,
            });
        }
        // 已保存引用的卡片排在前面，方便用户继续同一张卡。
        let selected = self.load_state()?.and_then(|state| {
            (!state.tombstone && !state.provider_id.is_empty()).then_some(state.provider_id)
        });
        if let Some(id) = &selected {
            views.sort_by_key(|view| view.provider_id != *id);
        }
        let path_candidates = discovery
            .candidates
            .iter()
            .map(paths::PathCandidate::view)
            .collect::<Vec<_>>();
        let selected_path_id = discovery
            .candidates
            .iter()
            .find(|candidate| {
                paths
                    .as_ref()
                    .is_some_and(|paths| candidate.paths.identity() == paths.identity())
            })
            .map(paths::PathCandidate::id);
        Ok(DesktopCandidates {
            paths: paths.map(|paths| paths.summary()),
            path_candidates,
            selected_path_id,
            detector: detection,
            providers: views,
            selected_provider_id: selected,
            route_providers: routes::candidates(store, &catalog),
            route_aggregates: routes::aggregate_candidates(store, &catalog),
            route_roles: routes::roles(policy),
        })
    }

    /// 状态所描述的同一组四文件路径：已接管时取保存的路径，否则取发现的默认路径。
    /// 不做进程探测与事务规划，供仪表盘只读展示。
    pub fn current_paths(&self) -> Option<DesktopPaths> {
        match self.load_state().ok().flatten() {
            Some(state) => Some(status::state_paths(&state)),
            None => self.discovery().default,
        }
    }

    /// 只读状态。写路径与读路径分开：这里不做任何写入或事务规划。
    pub fn status(&self, store: &Store) -> DesktopStatus {
        let detection = self.detection();
        let state = self.load_state().ok().flatten();
        let pending = self.load_pending().ok().flatten();
        status::project(status::StatusInput {
            detection,
            state: state.as_ref(),
            pending: pending.as_ref(),
            open_previews: self.with_tickets(|store| store.has_open()),
            store,
            app_data_dir: &self.app_data_dir,
            fallback_paths: state.is_none().then(|| self.discovery().default).flatten(),
            isolated: paths::isolated_home().is_some(),
            backup_count: transaction::backup_count(&self.app_data_dir),
        })
    }

    // -----------------------------------------------------------------------
    // 预览
    // -----------------------------------------------------------------------

    pub fn prepare(
        &self,
        store: &Store,
        action: DesktopAction,
        request: &ActionRequest,
    ) -> DesktopResult<PreparedChange> {
        let previous = self.load_state()?;
        if let Some(pending) = self.load_pending()? {
            return Err(Box::new(
                DesktopError::new(
                    error_code::RECOVERY_REQUIRED,
                    "claudeDesktop.errors.pendingRecovery",
                )
                .param(pending.transaction_id)
                .action("recoverRollback"),
            ));
        }
        let active = previous.as_ref().filter(|state| !state.tombstone);
        if !matches!(
            action,
            DesktopAction::ApplyDirect
                | DesktopAction::ApplyRouter
                | DesktopAction::SwitchOfficial
                | DesktopAction::RecoverRollback
        ) && active.is_none()
        {
            return Err(Box::new(DesktopError::new(
                error_code::NOT_MANAGED,
                "claudeDesktop.errors.notManaged",
            )));
        }
        if request.rebaseline
            && !matches!(
                action,
                DesktopAction::ApplyDirect | DesktopAction::ApplyRouter | DesktopAction::Reapply
            )
        {
            return Err(transaction::conflict_error());
        }
        let paths = self.session_paths(active, &request.paths)?;
        // 预览与保存都允许 Desktop 继续运行，显式重启负责退出后的模式补写。
        let before = transaction::snapshot(&paths)?;
        if action == DesktopAction::SwitchOfficial && active.is_none() {
            return self.prepare_unmanaged_official(request, paths, before, previous);
        }
        // 释放只保存原文与结束管理；外部文件即使不再是合法 JSON 也不需要解析。
        if action == DesktopAction::Release {
            let state = active.expect("managed action checked above");
            if state.data.applied_mode == AppliedMode::Router {
                return Err(Box::new(DesktopError::new(
                    error_code::CONFLICT,
                    "claudeDesktop.errors.routerReleaseBlocked",
                )));
            }
            let generation = state.generation + 1;
            let transaction_id = uuid::Uuid::new_v4().to_string();
            let mut next_state = state.clone().into_tombstone(generation, &transaction_id);
            next_state.provider_id.clear();
            next_state.provider_name.clear();
            let plans = Vec::new();
            let models = Vec::new();
            let after_read = transaction::snapshot(&paths)?;
            if serde_json::to_value(&before).ok() != serde_json::to_value(&after_read).ok() {
                return Err(changed_error());
            }
            let revision = revision_of(
                &action,
                request,
                &paths,
                &before,
                &plans,
                &models,
                previous.as_ref(),
                &next_state,
            );
            return Ok(PreparedChange {
                action,
                paths,
                plans,
                before,
                previous_state: previous,
                next_state,
                revision,
                warnings: vec!["claudeDesktop.warnings.releaseLeavesProfile".to_owned()],
                conflicts: Vec::new(),
                models,
                provider_id: None,
                provider_name: None,
                messages_url: None,
                models_url: None,
                external_profile_count: 0,
                managed_entry_present: false,
                current_applied_id: None,
                requires_restart: false,
                request: request.clone(),
                listener_lease: None,
            });
        }
        let detection = self.detection();
        let policy = provider::DesktopVersionPolicy::from_version(Some(&detection.version));
        let catalog = models_dev::load(&self.app_data_dir);
        // 事务开始前的四文件基线；既有接管基线优先，保证 reapply 不移动恢复点。
        let editions = transaction::editions_before(&paths, previous.as_ref(), request.rebaseline)?;
        let meta_before =
            transaction::read_text(&paths.meta_path).map_err(|_| transaction::storage_error())?;
        let meta = profile::MetaDocument::parse(meta_before.as_deref())?;
        let external_profile_count = meta.external_entry_count()?;
        let managed_entry_present = meta.own_entry_present()?;
        let current_applied_id = meta.applied_id_str().map(str::to_owned);

        let mut warnings: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        // 预览阶段就检测受管漂移；未显式 re-baseline 时不允许静默覆盖。
        if let Some(state) = &previous {
            if !request.rebaseline {
                let drifted = transaction::drifted_roles(state, &paths)?;
                conflicts.extend(drifted.iter().map(|name| format!("drift:{name}")));
            }
        }

        let mut models: Vec<ManagedModelView> = Vec::new();
        let mut provider_id: Option<String> = None;
        let mut provider_name: Option<String> = None;
        let mut messages_url: Option<String> = None;
        let mut models_url: Option<String> = None;
        // 只有直连接管/重新应用才写密钥指纹；官方与撤回操作不写。
        let mut credential_fingerprint: Option<String> = None;
        let mut model_choices = Vec::new();
        let mut profile_base_url = None;
        let effective_action = if action == DesktopAction::Reapply
            && active.is_some_and(|s| s.data.applied_mode == AppliedMode::Router)
        {
            DesktopAction::ApplyRouter
        } else {
            action
        };
        if request.rotate_gateway_token && effective_action != DesktopAction::ApplyRouter {
            return Err(changed_error());
        }
        let mut router_config = None;
        let mut listener_lease = None;
        let desired = match effective_action {
            DesktopAction::ApplyRouter => {
                let lease = self
                    .listener_lease
                    .as_ref()
                    .filter(|lease| lease.bound_port != 0 && lease.bound_port == store.router.port)
                    .ok_or_else(|| {
                        Box::new(DesktopError::new(
                            error_code::CHANGED,
                            "claudeDesktop.errors.listenerUnavailable",
                        ))
                    })?;
                let choices = if action == DesktopAction::Reapply {
                    active
                        .and_then(|s| s.router.as_ref())
                        .map(|r| r.routes.clone())
                        .ok_or_else(changed_error)?
                } else {
                    request.routes.clone()
                };
                let resolved = routes::resolve_routes(store, &choices, policy, &catalog)?;
                let (token, token_version) =
                    self.router_token(previous.as_ref(), request.rotate_gateway_token)?;
                let prior_router = active.and_then(|s| s.router.as_ref());
                let projection = routes::file_projection(&resolved);
                let route_revision = match prior_router {
                    Some(prior)
                        if prior.routes == choices && prior.file_projection == projection =>
                    {
                        prior.route_revision
                    }
                    Some(prior) => prior.route_revision + 1,
                    None => active.map(|s| s.generation + 1).unwrap_or(1),
                };
                let config = gateway::RouterConfig {
                    routes: choices,
                    route_views: resolved.iter().map(|r| r.view.clone()).collect(),
                    file_projection: projection,
                    route_revision,
                    gateway_token_version: token_version,
                    gateway_token: token,
                    bound_port: lease.bound_port,
                };
                let base = format!("http://127.0.0.1:{}/claude-desktop", lease.bound_port);
                models = config
                    .route_views
                    .iter()
                    .map(|r| ManagedModelView {
                        model_id: r.role_alias.clone(),
                        label: r.label.clone(),
                        one_m: r.one_m,
                    })
                    .collect();
                profile_base_url = Some(base.clone());
                messages_url = Some(format!("{base}/v1/messages"));
                models_url = Some(format!("{base}/v1/models"));
                let value = profile::profile_json(&base, &config.gateway_token, &models);
                router_config = Some(config);
                listener_lease = Some(lease.clone());
                DesiredState {
                    mode: Some(profile::third_party_mode()),
                    profile: Some(value),
                    own_entry: Some(MetaEntry::own()),
                    applied_id: Some(PROFILE_ID.into()),
                    entry_insert_at: meta
                        .entry_list()?
                        .iter()
                        .position(MetaEntry::is_own)
                        .unwrap_or(0),
                }
            }
            DesktopAction::ApplyDirect | DesktopAction::Reapply => {
                let direct: ApplyDirectRequest = if action == DesktopAction::Reapply {
                    let state = active.expect("managed action checked above");
                    if request
                        .provider_id
                        .as_ref()
                        .is_some_and(|id| id != &state.provider_id)
                    {
                        return Err(changed_error());
                    }
                    let choices = if state.model_choices.is_empty() {
                        // 旧 journal 只保存结果：保守保留其标签和 1M 开关。
                        state
                            .models
                            .iter()
                            .map(|model| types::DesktopModelChoice {
                                model_id: model.model_id.clone(),
                                label_override: Some(model.label.clone()),
                                one_m_mode: if model.one_m {
                                    OneMMode::Auto
                                } else {
                                    OneMMode::Off
                                },
                            })
                            .collect()
                    } else {
                        state.model_choices.clone()
                    };
                    ApplyDirectRequest {
                        provider_id: state.provider_id.clone(),
                        models: choices,
                        paths: request.paths.clone(),
                        rebaseline: request.rebaseline,
                    }
                } else {
                    request.clone().into_direct()
                };
                let (provider, selected, endpoint) =
                    self.direct_plan(store, &direct, policy, &catalog)?;
                let api_key = provider::require_inference_key(provider)?;
                credential_fingerprint = provider::credential_fingerprint(provider);
                model_choices = direct.models;
                profile_base_url = Some(endpoint.0.clone());
                models = selected;
                provider_id = Some(provider.id.clone());
                provider_name = Some(provider.name.clone());
                messages_url = Some(endpoint.1.clone());
                models_url = Some(endpoint.2.clone());
                let insert_at = meta
                    .entry_list()?
                    .iter()
                    .position(MetaEntry::is_own)
                    .unwrap_or(0);
                DesiredState {
                    mode: Some(profile::third_party_mode()),
                    profile: Some(profile::profile_json(&endpoint.0, &api_key, &models)),
                    own_entry: Some(MetaEntry::own()),
                    applied_id: Some(PROFILE_ID.to_owned()),
                    entry_insert_at: insert_at,
                }
            }
            DesktopAction::RestorePrevious => {
                provider_id = previous.as_ref().map(|state| state.provider_id.clone());
                provider_name = previous.as_ref().map(|state| state.provider_name.clone());
                DesiredState {
                    // 恢复接管前配置：两处文件各自恢复自己的取值，并撤回自身 profile。
                    // 是「写回接管前内容」还是「删除」由 `build_plan` 的基线参数决定。
                    mode: None,
                    profile: None,
                    own_entry: baseline_meta_entry(&editions),
                    applied_id: baseline_applied_id(&editions),
                    entry_insert_at: 0,
                }
            }
            DesktopAction::SwitchOfficial => {
                provider_id = previous.as_ref().map(|state| state.provider_id.clone());
                provider_name = previous.as_ref().map(|state| state.provider_name.clone());
                // 当前 pointer 已被外部切换时不能清掉那条外部指针。
                if current_applied_id
                    .as_deref()
                    .is_some_and(|id| id != PROFILE_ID)
                {
                    conflicts.push("pointerMovedExternally".to_owned());
                }
                DesiredState {
                    mode: Some(profile::official_mode()),
                    profile: None,
                    // 只撤回本次管理的变更，接管前同 UUID 的外部条目也要保留。
                    own_entry: baseline_meta_entry(&editions),
                    applied_id: None,
                    entry_insert_at: 0,
                }
            }
            // `release` 不写任何 Desktop 文件；`recoverRollback` 只处理 pending。
            DesktopAction::Release | DesktopAction::RecoverRollback => DesiredState {
                mode: None,
                profile: None,
                own_entry: None,
                applied_id: None,
                entry_insert_at: 0,
            },
        };

        let write_plans = !matches!(
            action,
            DesktopAction::Release | DesktopAction::RecoverRollback
        );
        let plans = if write_plans {
            transaction::build_plan(
                &paths,
                &desired,
                &transaction::TakeoverBaseline::from_editions(&editions, false),
            )?
        } else {
            Vec::new()
        };

        if matches!(
            action,
            DesktopAction::RestorePrevious | DesktopAction::SwitchOfficial
        ) {
            warnings.push("claudeDesktop.warnings.keepsEnterprise".to_owned());
            if !plans.is_empty() {
                warnings.push("claudeDesktop.warnings.requiresRestart".to_owned());
            }
        }
        if action == DesktopAction::Release {
            warnings.push("claudeDesktop.warnings.releaseLeavesProfile".to_owned());
        }
        if external_profile_count > 0 {
            warnings.push("claudeDesktop.warnings.externalProfiles".to_owned());
        }
        if current_applied_id
            .as_deref()
            .is_some_and(|id| id != PROFILE_ID)
        {
            warnings.push("claudeDesktop.warnings.takesOverPointer".to_owned());
        }
        if previous.as_ref().is_some_and(|state| state.tombstone) {
            warnings.push("claudeDesktop.warnings.afterRelease".to_owned());
        }
        if request.rebaseline && !conflicts.is_empty() {
            warnings.push("claudeDesktop.warnings.rebaseline".to_owned());
        }

        let generation = previous
            .as_ref()
            .map(|state| state.generation + 1)
            .unwrap_or(1);
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let mut next_state = transaction::next_state(
            &editions,
            &plans,
            &paths,
            generation,
            &transaction_id,
            provider_id.as_deref().unwrap_or_default(),
            provider_name.as_deref().unwrap_or_default(),
            credential_fingerprint,
            &models,
            if desired.mode == Some(profile::official_mode()) {
                AppliedMode::Official
            } else if effective_action == DesktopAction::ApplyRouter {
                AppliedMode::Router
            } else {
                AppliedMode::Direct
            },
        )?;
        next_state.model_choices = model_choices;
        next_state.profile_base_url = profile_base_url;
        next_state.router = router_config;
        let restart_modes = transaction::restart_modes(&next_state, previous.as_ref(), &plans);
        if matches!(
            action,
            DesktopAction::RestorePrevious | DesktopAction::SwitchOfficial
        ) {
            next_state = next_state.into_tombstone(generation, &transaction_id);
        }
        // 待启动属于已应用会话的状态：一次无改动的重新应用不能替用户确认已启动。
        let requires_restart = !plans.is_empty()
            || previous.as_ref().is_some_and(|state| {
                state.restart_required && state.paths_identity == paths.identity()
            });
        next_state.restart_required = requires_restart;
        if requires_restart { next_state.restart_modes = restart_modes; }
        // 计划读取期间发生变化也必须重新预览；不能拿旧 before 配合新 after。
        let after_read = transaction::snapshot(&paths)?;
        if serde_json::to_value(&before).ok() != serde_json::to_value(&after_read).ok() {
            return Err(changed_error());
        }
        let revision = revision_of(
            &action,
            request,
            &paths,
            &before,
            &plans,
            &models,
            previous.as_ref(),
            &next_state,
        );
        let revision = match &listener_lease {
            Some(lease) => format!(
                "{}:{}:{}:{}",
                revision, lease.bound_port, lease.generation, lease.process_epoch
            ),
            None => revision,
        };

        Ok(PreparedChange {
            action,
            paths,
            plans,
            before,
            previous_state: previous,
            next_state,
            revision,
            warnings,
            conflicts,
            models,
            provider_id,
            provider_name,
            messages_url,
            models_url,
            external_profile_count,
            managed_entry_present,
            current_applied_id,
            requires_restart,
            request: request.clone(),
            listener_lease,
        })
    }

    /// 官方模式不要求先接管中转。未接管时只改两处模式，不获得外部 profile/meta 的所有权。
    fn prepare_unmanaged_official(
        &self,
        request: &ActionRequest,
        paths: DesktopPaths,
        before: Vec<transaction::FileSnapshot>,
        previous: Option<State>,
    ) -> DesktopResult<PreparedChange> {
        if request.rotate_gateway_token {
            return Err(changed_error());
        }
        let plans = transaction::official_mode_plan(&paths)?;
        let generation = previous
            .as_ref()
            .map(|state| state.generation + 1)
            .unwrap_or(1);
        let transaction_id = uuid::Uuid::new_v4().to_string();
        // 没有接管基线；仅保留事务/重启标记，恢复接管前配置仍应不可用。
        let mut next_state = transaction::next_state(
            &[],
            &plans,
            &paths,
            generation,
            &transaction_id,
            "",
            "",
            None,
            &[],
            AppliedMode::Official,
        )?
        .into_tombstone(generation, &transaction_id);
        let requires_restart = !plans.is_empty()
            || previous.as_ref().is_some_and(|state| {
                state.restart_required && state.paths_identity == paths.identity()
            });
        next_state.restart_required = requires_restart;
        if requires_restart {
            next_state.restart_modes = transaction::restart_modes(&next_state, previous.as_ref(), &plans);
        }
        // 外部 metadata 只供预览计数，不是此操作的写入目标；损坏也不阻止退出 3P。
        let meta = before
            .iter()
            .find(|file| file.role == role::META)
            .and_then(|file| profile::MetaDocument::parse(file.text.as_deref()).ok());
        let external_profile_count = meta
            .as_ref()
            .and_then(|meta| meta.external_entry_count().ok())
            .unwrap_or(0);
        let managed_entry_present = meta
            .as_ref()
            .and_then(|meta| meta.own_entry_present().ok())
            .unwrap_or(false);
        let current_applied_id = meta
            .as_ref()
            .and_then(|meta| meta.applied_id_str().map(str::to_owned));
        let mut warnings = vec!["claudeDesktop.warnings.keepsEnterprise".to_owned()];
        if requires_restart {
            warnings.push("claudeDesktop.warnings.requiresRestart".to_owned());
        }
        if external_profile_count > 0 {
            warnings.push("claudeDesktop.warnings.externalProfiles".to_owned());
        }
        let after_read = transaction::snapshot(&paths)?;
        if serde_json::to_value(&before).ok() != serde_json::to_value(&after_read).ok() {
            return Err(changed_error());
        }
        let revision = revision_of(
            &DesktopAction::SwitchOfficial,
            request,
            &paths,
            &before,
            &plans,
            &[],
            previous.as_ref(),
            &next_state,
        );
        Ok(PreparedChange {
            action: DesktopAction::SwitchOfficial,
            paths,
            plans,
            before,
            previous_state: previous,
            next_state,
            revision,
            warnings,
            conflicts: Vec::new(),
            models: Vec::new(),
            provider_id: None,
            provider_name: None,
            messages_url: None,
            models_url: None,
            external_profile_count,
            managed_entry_present,
            current_applied_id,
            requires_restart,
            request: request.clone(),
            listener_lease: None,
        })
    }

    fn router_token(&self, previous: Option<&State>, rotate: bool) -> DesktopResult<(String, u64)> {
        if !rotate {
            if let Some(router) = previous.and_then(|s| s.router.as_ref()) {
                return Ok((router.gateway_token.clone(), router.gateway_token_version));
            }
        }
        #[derive(Serialize, Deserialize)]
        struct Candidate {
            previous_transaction: Option<String>,
            token: String,
            version: u64,
        }
        let path = self.data_dir().join("gateway-candidate.dat");
        let previous_transaction = previous.map(|s| s.transaction_id.clone());
        if let Some(candidate) = read_secret_json::<Candidate>(&path)? {
            if candidate.previous_transaction == previous_transaction {
                return Ok((candidate.token, candidate.version));
            }
        }
        let candidate = Candidate {
            previous_transaction,
            token: format!(
                "ahd-{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            version: previous.map(|s| s.generation + 1).unwrap_or(1),
        };
        write_secret_json(&path, &candidate)?;
        Ok((candidate.token, candidate.version))
    }

    /// 解析直连卡片：校验资格、逐模型校验、投影地址。
    fn direct_plan<'a>(
        &self,
        store: &'a Store,
        request: &ApplyDirectRequest,
        policy: provider::DesktopVersionPolicy,
        catalog: &models_dev::Catalog,
    ) -> DesktopResult<(
        &'a Provider,
        Vec<ManagedModelView>,
        (String, String, String),
    )> {
        let provider = store
            .providers
            .iter()
            .find(|entry| entry.id == request.provider_id)
            .ok_or_else(|| {
                Box::new(
                    DesktopError::new(
                        error_code::PROVIDER_MISSING,
                        "claudeDesktop.errors.providerMissing",
                    )
                    .action("refreshProviders"),
                )
            })?;
        if request.models.is_empty() {
            return Err(Box::new(
                DesktopError::new(
                    error_code::NO_MODELS_SELECTED,
                    "claudeDesktop.errors.noModelsSelected",
                )
                .action("selectModels"),
            ));
        }
        let endpoint = provider::resolve_direct_endpoints(provider)?;
        let mut seen: Vec<String> = Vec::new();
        let mut models: Vec<ManagedModelView> = Vec::new();
        for choice in &request.models {
            let model_id = choice.model_id.clone();
            if seen.contains(&model_id) {
                return Err(Box::new(
                    DesktopError::new(
                        error_code::MODEL_REJECTED,
                        "claudeDesktop.errors.duplicateModel",
                    )
                    .model(&model_id),
                ));
            }
            seen.push(model_id.clone());
            // 逐条校验：任何一条失败都阻止整次提交，不静默删掉非法项。
            let candidate = provider::model_candidate(provider, &model_id, catalog, policy);
            if let Some(reason) = &candidate.exclude_reason {
                let message = match reason.as_str() {
                    types::reason::MODEL_NOT_INCLUDED => "claudeDesktop.errors.modelNotIncluded",
                    types::reason::MODEL_REMOVED => "claudeDesktop.errors.modelRemoved",
                    _ => "claudeDesktop.errors.modelRejected",
                };
                return Err(Box::new(
                    DesktopError::new(error_code::MODEL_REJECTED, message)
                        .model(&model_id)
                        .param(reason.clone()),
                ));
            }
            let label = match choice.label_override.as_deref() {
                Some(raw) => provider::normalize_label(raw).ok_or_else(|| {
                    Box::new(
                        DesktopError::new(
                            error_code::MODEL_REJECTED,
                            "claudeDesktop.errors.invalidLabel",
                        )
                        .model(&model_id),
                    )
                })?,
                None => candidate.label.clone(),
            };
            let one_m = match choice.one_m_mode {
                OneMMode::Auto => candidate.supports1m,
                OneMMode::Off => false,
            };
            models.push(ManagedModelView {
                model_id,
                label,
                one_m,
            });
        }
        Ok((provider, models, endpoint))
    }

    // -----------------------------------------------------------------------
    // 提交
    // -----------------------------------------------------------------------

    pub fn commit(
        &self,
        store: &Store,
        action: DesktopAction,
        revision: &str,
        request: &ActionRequest,
    ) -> DesktopResult<DesktopApplied> {
        let _guard = Self::write_lock()?;
        // 跨进程 writer 互斥：另一个实例/会话正在管理同一目录时立即失败，
        // 不排队、不覆盖。
        let _process_lock = lock::acquire(&self.app_data_dir, action.as_str())?;
        if action == DesktopAction::RecoverRollback {
            return self.recover_pending_locked();
        }
        // ticket 必须存在、未过期且未被消费：已消费或过期的 ticket 必须报
        // `preview_expired`，而不是被后一步的 revision 比较说成 `changed`。
        let ticket = self.take_ticket(revision)?;
        if ticket.action != action || ticket.request != *request {
            return Err(changed_error());
        }
        if ticket.listener_lease.is_some() && ticket.listener_lease != self.listener_lease {
            return Err(changed_error());
        }
        // 重读全部输入后重新计算 revision；不一致即 changed。
        let prepared = self.prepare(store, action, request)?;
        if prepared.revision != ticket.revision
            || ticket.paths_identity != prepared.paths.identity()
        {
            return Err(changed_error());
        }
        if action != DesktopAction::Release && !prepared.conflicts.is_empty() {
            return Err(transaction::conflict_error());
        }

        if action == DesktopAction::Release {
            return self.release_locked(&prepared);
        }

        self.persist_change_locked(
            &prepared.paths, action, &prepared.plans, &prepared.before,
            prepared.previous_state.as_ref(), prepared.next_state.clone(), prepared.warnings.clone(),
        )
    }

    // Caller holds both writer locks. Used by a previewed configuration save
    // and by the narrow mode repair after a verified shutdown.
    fn persist_change_locked(
        &self,
        paths: &DesktopPaths,
        action: DesktopAction,
        plans: &[transaction::FilePlan],
        before: &[transaction::FileSnapshot],
        previous: Option<&State>,
        mut next_state: State,
        mut warnings: Vec<String>,
    ) -> DesktopResult<DesktopApplied> {
        self.commit_checkpoint(paths)?;
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let generation = previous
            .map(|state| state.generation + 1)
            .unwrap_or(1);
        next_state.transaction_id = transaction_id.clone();
        next_state.generation = generation;

        // 1) 写前持久 journal：包含 before/after 与旧/新 state。
        let mut pending = Pending {
            schema: transaction::STATE_SCHEMA,
            transaction_id: transaction_id.clone(),
            generation,
            action: action.as_str().to_owned(),
            created_at: chrono::Utc::now().timestamp(),
            paths_identity: paths.identity(),
            files: plans.to_vec(),
            before: before.to_vec(),
            previous_state: previous.cloned(),
            next_state: next_state.clone(),
            backup_id: None,
        };
        self.save_pending(Some(&pending))?;

        // 2) 受保护备份；失败则不改外部文件。
        let backup_id = transaction::write_backup(&self.app_data_dir, &pending.before)?;
        pending.backup_id = Some(backup_id.clone());
        self.save_pending(Some(&pending))?;

        // 3) 逐文件写入，写前再次校验该文件未被外部改动。
        for plan in plans {
            self.commit_checkpoint(paths)?;
            if let Err(error) = verify_unchanged(&pending, plan) {
                return self.fail_and_rollback(&pending, error);
            }
            if let Err(error) = transaction::apply_plan(plan) {
                return self.fail_and_rollback(&pending, error);
            }
        }

        // 4) 逐项读回确认。
        for plan in plans {
            if let Err(error) = verify_written(plan) {
                return self.fail_and_rollback(&pending, error);
            }
        }

        // 5) 持久写入 nextState：这是提交点。
        self.commit_checkpoint(paths)?;
        if let Err(error) = self.save_state(&next_state) {
            return self.fail_and_rollback(&pending, error);
        }

        // 6) 删除 pending；清理失败时仍是已提交，但要提示阻止下一笔写入。
        if self.save_pending(None).is_err() {
            warnings.push("claudeDesktop.warnings.pendingCleanup".to_owned());
        }
        if self.consume_tickets(&paths.identity()).is_err() {
            warnings.push("claudeDesktop.warnings.ticketCleanup".to_owned());
        }
        // The committed State/Pending now owns the token. The preparation copy is never
        // an authorization source and can be removed without changing recovery semantics.
        let _ = clear_secret_file(&self.data_dir().join("gateway-candidate.dat"));

        // 没有新写入也不能清除之前尚未完成的重启要求。
        let wrote_files = !plans.is_empty();
        if !wrote_files {
            warnings.push("claudeDesktop.warnings.noChanges".to_owned());
        }
        Ok(DesktopApplied {
            transaction_id,
            changed_paths: plans
                .iter()
                .map(|plan| paths::display(&plan.path))
                .collect(),
            backup_id: Some(backup_id),
            management_state: if next_state.tombstone {
                "unmanaged"
            } else {
                "managed"
            }
            .to_owned(),
            requires_restart: next_state.restart_required,
            warnings,
        })
    }

    fn commit_checkpoint(&self, _paths: &DesktopPaths) -> DesktopResult<()> {
        #[cfg(test)]
        if let Some(check) = &self.commit_check { return check(_paths); }
        Ok(())
    }

    fn release_locked(&self, prepared: &PreparedChange) -> DesktopResult<DesktopApplied> {
        let previous = prepared.previous_state.clone().ok_or_else(|| {
            Box::new(DesktopError::new(
                error_code::NOT_MANAGED,
                "claudeDesktop.errors.notManaged",
            ))
        })?;
        // 释放管理不写任何 Desktop 文件，但要保留可恢复备份。
        let backup_id = transaction::write_backup(&self.app_data_dir, &prepared.before)?;
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let generation = previous.generation + 1;
        let mut tombstone = previous.into_tombstone(generation, &transaction_id);
        tombstone.provider_id.clear();
        tombstone.provider_name.clear();
        self.save_state(&tombstone)?;
        let mut warnings = prepared.warnings.clone();
        if self.consume_tickets(&prepared.paths.identity()).is_err() {
            warnings.push("claudeDesktop.warnings.ticketCleanup".to_owned());
        }
        warnings.push("claudeDesktop.warnings.releaseKeepsCredentials".to_owned());
        Ok(DesktopApplied {
            transaction_id,
            changed_paths: Vec::new(),
            backup_id: Some(backup_id),
            management_state: "unmanaged".to_owned(),
            requires_restart: false,
            warnings,
        })
    }

    fn fail_and_rollback(
        &self,
        pending: &Pending,
        error: Box<DesktopError>,
    ) -> DesktopResult<DesktopApplied> {
        // Each rollback compares the current owned values with this transaction's
        // before/after values. Running Desktop is allowed; a third value still
        // leaves the journal for explicit recovery instead of overwriting it.
        match transaction::rollback(pending) {
            Ok(_) => {
                // 全部恢复成功：还原 previousState，移除 pending，返回原失败。
                let restored = match &pending.previous_state {
                    Some(previous) => self.save_state(previous),
                    None => clear_secret_file(&self.state_path()),
                };
                if restored.is_err() || self.save_pending(None).is_err() {
                    return Err(Box::new(
                        DesktopError::new(
                            error_code::RECOVERY_REQUIRED,
                            "claudeDesktop.errors.recoveryRequired",
                        )
                        .param(pending.transaction_id.clone()),
                    ));
                }
                Err(error)
            }
            Err(recovery) => {
                // 部分恢复失败：保留 pending 与证据，返回 recoveryRequired。
                let _ = self.save_pending(Some(pending));
                Err(recovery)
            }
        }
    }

    /// 崩溃后按持久提交点判定实际状态。启动与显式操作都走这里。
    pub fn recover_pending(&self) -> DesktopResult<DesktopApplied> {
        let _guard = Self::write_lock()?;
        let _process_lock = lock::acquire(&self.app_data_dir, "recoverRollback")?;
        self.recover_pending_locked()
    }

    /// Complete journal recovery before serving Desktop. A committed journal only needs
    /// read-back and protected cleanup, so a running Desktop does not prevent that case.
    pub fn recover_at_startup(&self) -> DesktopResult<()> {
        if self.load_pending()?.is_some() {
            self.recover_pending()?;
        }
        self.cleanup_previews()
    }

    pub fn router_requirement(&self) -> DesktopResult<Option<u16>> {
        gateway::required_port(&self.app_data_dir)
    }

    pub fn cancel_preview(&self, revision: &str) -> DesktopResult<()> {
        let _guard = Self::write_lock()?;
        self.with_tickets(|store| store.cancel(revision))?;
        self.cleanup_unused_router_candidate()
    }

    fn recover_pending_locked(&self) -> DesktopResult<DesktopApplied> {
        let Some(pending) = self.load_pending()? else {
            return Err(Box::new(
                DesktopError::new(
                    error_code::NOT_MANAGED,
                    "claudeDesktop.errors.nothingToRecover",
                )
                .retryable(),
            ));
        };
        let state = self.load_state()?;
        let paths = self.session_paths(Some(&pending.next_state), &PathsChoice::default())?;
        if paths.identity() != pending.paths_identity {
            return Err(changed_error());
        }
        for snapshot in &pending.before {
            if expected_path(&paths, &snapshot.role) != Some(&snapshot.path) {
                return Err(changed_error());
            }
        }
        for plan in &pending.files {
            if expected_path(&paths, &plan.role) != Some(&plan.path) {
                return Err(changed_error());
            }
        }
        let committed = state.as_ref().is_some_and(|state| {
            state.transaction_id == pending.transaction_id && state.generation == pending.generation
        });
        if committed {
            // 事务已提交：只读回核实并完成 pending 清理，不回滚。
            for plan in &pending.files {
                if !transaction::after_matches(plan).unwrap_or(false) {
                    return Err(Box::new(
                        DesktopError::new(
                            error_code::RECOVERY_REQUIRED,
                            "claudeDesktop.errors.recoveryConflict",
                        )
                        .file(&plan.role)
                        .param(pending.transaction_id.clone()),
                    ));
                }
            }
            self.save_pending(None)?;
            return Ok(DesktopApplied {
                transaction_id: pending.transaction_id,
                changed_paths: pending
                    .files
                    .iter()
                    .map(|plan| paths::display(&plan.path))
                    .collect(),
                backup_id: pending.backup_id,
                management_state: if pending.next_state.tombstone {
                    "unmanaged"
                } else {
                    "managed"
                }
                .to_owned(),
                requires_restart: pending.next_state.restart_required,
                warnings: vec!["claudeDesktop.warnings.recoveredCommitted".to_owned()],
            });
        }
        let before_matches = match (&state, &pending.previous_state) {
            (None, None) => true,
            (Some(now), Some(previous)) => {
                now.transaction_id == previous.transaction_id
                    && now.generation == previous.generation
            }
            // 新装：没有旧 state，但事务前也不应有 state。
            (Some(_), None) => false,
            (None, Some(_)) => false,
        };
        if before_matches {
            self.ensure_stopped(&paths)?;
            // 未提交：按 before 安全恢复。
            transaction::rollback(&pending)?;
            self.save_pending(None)?;
            return Ok(DesktopApplied {
                transaction_id: pending.transaction_id,
                changed_paths: Vec::new(),
                backup_id: pending.backup_id,
                management_state: if pending
                    .previous_state
                    .as_ref()
                    .is_some_and(|state| !state.tombstone)
                {
                    "managed"
                } else {
                    "unmanaged"
                }
                .to_owned(),
                requires_restart: false,
                warnings: vec!["claudeDesktop.warnings.recoveredRolledBack".to_owned()],
            });
        }
        // state 不匹配前后任一版本：保留证据，报告冲突，不自动覆盖。
        Err(Box::new(
            DesktopError::new(
                error_code::RECOVERY_REQUIRED,
                "claudeDesktop.errors.recoveryConflict",
            )
            .param(pending.transaction_id),
        ))
    }

    /// 启动时清理过期预览 ticket（未完成事务不在此处清理）。
    /// 只清理 ticket 文件，不写 Desktop 文件，因此不需要 writer 锁。
    pub fn cleanup_previews(&self) -> DesktopResult<()> {
        let _guard = Self::write_lock()?;
        self.with_tickets(|store| store.prune_expired())?;
        self.cleanup_unused_router_candidate()
    }

    fn cleanup_unused_router_candidate(&self) -> DesktopResult<()> {
        if !self.with_tickets(|store| store.has_open()) && self.load_pending()?.is_none() {
            clear_secret_file(&self.data_dir().join("gateway-candidate.dat"))?;
        }
        Ok(())
    }

    /// 测试专用：直接写入 pending journal，用于模拟进程中断。
    #[cfg(test)]
    pub fn write_pending_for_test(&self, bytes: &[u8]) -> DesktopResult<()> {
        crate::core::secret_file::write(&self.pending_path(), bytes)
            .map_err(|_| transaction::storage_error())
    }

    /// 测试专用：直接写入 state，用于构造“提交点已落盘”的场景。
    #[cfg(test)]
    pub fn write_state_for_test(&self, state: &State) -> DesktopResult<()> {
        self.save_state(state)
    }

    /// 重启 Desktop 应用，使刚写入的配置立即生效。
    ///
    /// 先确认退出，再补回退出时回写的模式字段，最后启动。
    /// 配置提交本身允许运行，不会自动关闭 Desktop。
    pub fn restart_app(&self, paths: &DesktopPaths) -> DesktopResult<String> {
        self.restart_app_with(paths, |paths, after_stop| desktop_app::restart_after_stop(paths, after_stop))
    }

    fn restart_app_with(
        &self,
        paths: &DesktopPaths,
        restart: impl FnOnce(&DesktopPaths, &mut dyn FnMut() -> DesktopResult<()>) -> DesktopResult<String>,
    ) -> DesktopResult<String> {
        // 状态检查与停止/启动都在锁内，避免等锁期间启动的实例漏过停止阶段。
        // 与提交/恢复保持同一锁顺序；成功后更新 journal 也属于写操作。
        let _guard = Self::write_lock()?;
        let _process_lock = lock::acquire(&self.app_data_dir, "restartApp")?;
        if let Some(pending) = self.load_pending()? {
            return Err(Box::new(
                DesktopError::new(
                    error_code::RECOVERY_REQUIRED,
                    "claudeDesktop.errors.pendingRecovery",
                )
                .param(pending.transaction_id)
                .action("recoverRollback"),
            ));
        }
        let state = self.load_state()?;
        let current_paths = self.session_paths(state.as_ref(), &PathsChoice::default())?;
        if current_paths.identity() != paths.identity() {
            return Err(changed_error());
        }
        let launch_method = restart(&current_paths, &mut || self.reassert_saved_modes_locked(&current_paths))?;
        // The mode repair may have committed a new journal generation.
        let mut state = self.load_state()?;
        // 仅在停止和启动都确认成功后清除当前会话标记，失败时保留原状态。
        if let Some(state) = state.as_mut().filter(|state| state.restart_required) {
            state.restart_required = false;
            state.restart_modes.clear();
            self.save_state(state)?;
        }
        Ok(launch_method)
    }

    fn reassert_saved_modes_locked(&self, paths: &DesktopPaths) -> DesktopResult<()> {
        let Some(state) = self.load_state()?.filter(|state| state.restart_required) else { return Ok(()); };
        let stopped = desktop_app::StopCheck::new(paths);
        self.confirm_shutdown(paths, &stopped)?;
        // Derive targets for older State records that predate restart_modes.
        let modes = transaction::restart_modes(&state, Some(&state), &[]);
        let before = transaction::snapshot(paths)?;
        let plans = transaction::restart_mode_plan(paths, &modes, &before)?;
        // Quitting can rewrite preferences, but cannot authorize overwriting
        // another tool's profile/selection. Those owned values must still match.
        if transaction::drifted_roles(&state, paths)?.iter().any(|file_role| {
            !matches!(file_role.as_str(), role::NORMAL_CONFIG | role::THIRD_PARTY_CONFIG)
        }) { return Err(transaction::conflict_error()); }
        if plans.is_empty() { return Ok(()); }
        self.persist_change_locked(paths, DesktopAction::Reapply, &plans, &before, Some(&state), state.clone(), Vec::new())?;
        // A failed journal cleanup must finish before launch clears the marker.
        if self.load_pending()?.is_some() { self.recover_pending_locked()?; }
        self.confirm_shutdown(paths, &stopped)?;
        Ok(())
    }

    /// 当前 Desktop 应用状态，供命令层在重启前判断是否需要先停止。
    pub fn app_state(&self, paths: &DesktopPaths) -> desktop_app::AppState {
        desktop_app::state(paths)
    }

    /// 是否存在未完成事务：启动恢复入口用它决定是否要提示。
    pub fn has_pending(&self) -> bool {
        matches!(self.load_pending(), Ok(Some(_)))
    }

    // -----------------------------------------------------------------------
    // ticket：全部委托给 `tickets` 模块，这里只负责在正确的锁下调用
    // -----------------------------------------------------------------------

    /// ticket 集合绑定在预览文件路径上；用闭包式访问避免把临时 PathBuf 借出去。
    fn with_tickets<T>(&self, work: impl FnOnce(&tickets::Tickets<'_>) -> T) -> T {
        let path = self.previews_path();
        work(&tickets::Tickets::new(&path))
    }

    /// 固化一次预览并返回可提交的 DTO。
    pub fn issue_ticket(&self, prepared: &PreparedChange) -> DesktopResult<DesktopPreview> {
        let _guard = Self::write_lock()?;
        let ticket = tickets::Ticket {
            revision: prepared.revision.clone(),
            action: prepared.action,
            paths_identity: prepared.paths.identity(),
            created_at: chrono::Utc::now().timestamp(),
            request: prepared.request.clone(),
            process_epoch: tickets::process_epoch().to_owned(),
            listener_lease: prepared.listener_lease.clone(),
        };
        let ticket_id = self.with_tickets(|store| store.issue(ticket))?;
        Ok(DesktopPreview {
            revision: ticket_id,
            action: prepared.action,
            provider_id: prepared.provider_id.clone(),
            provider_name: prepared.provider_name.clone(),
            models: prepared.models.clone(),
            route_views: prepared
                .next_state
                .router
                .as_ref()
                .map(|r| r.route_views.clone())
                .unwrap_or_default(),
            router: prepared
                .next_state
                .router
                .as_ref()
                .map(gateway::RouterConfig::view),
            messages_url: prepared.messages_url.clone(),
            models_url: prepared.models_url.clone(),
            files: transaction::views(&prepared.plans, |role| {
                prepared
                    .before
                    .iter()
                    .find(|snapshot| snapshot.role == role)
                    .map(|snapshot| snapshot.existed)
                    .unwrap_or(false)
            }),
            warnings: prepared.warnings.clone(),
            conflicts: prepared.conflicts.clone(),
            requires_restart: prepared.requires_restart,
            external_profile_count: prepared.external_profile_count,
            managed_entry_present: prepared.managed_entry_present,
            current_applied_id: prepared.current_applied_id.clone(),
            applies_to: paths::display(&prepared.paths.third_party_root),
        })
    }

    /// 提交前消费 ticket：已消费/过期一律报「预览失效」。
    fn take_ticket(&self, revision: &str) -> DesktopResult<tickets::Ticket> {
        self.with_tickets(|store| store.take(revision))
    }

    fn consume_tickets(&self, paths_identity: &str) -> DesktopResult<()> {
        self.with_tickets(|store| store.consume_all(paths_identity))
    }

    // -----------------------------------------------------------------------
    // 检测
    // -----------------------------------------------------------------------

    pub fn probe(
        &self,
        store: &Store,
        provider_id: &str,
        model_ids: &[String],
        proxy: &crate::core::models::ApiProxyConfigPayload,
    ) -> DesktopResult<probe::BearerReport> {
        let provider = store
            .providers
            .iter()
            .find(|entry| entry.id == provider_id)
            .ok_or_else(|| {
                Box::new(
                    DesktopError::new(
                        error_code::PROVIDER_MISSING,
                        "claudeDesktop.errors.providerMissing",
                    )
                    .action("refreshProviders"),
                )
            })?;
        let egress = probe::egress_label(provider);
        probe::run(provider, model_ids, proxy, &egress)
    }
}

// ---------------------------------------------------------------------------
// 自由函数
// ---------------------------------------------------------------------------

fn read_secret_json<T: for<'de> Deserialize<'de>>(path: &Path) -> DesktopResult<Option<T>> {
    let Some(bytes) = secret_file::read(path).map_err(|_| transaction::storage_error())? else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice(&bytes).map_err(|_| transaction::storage_error())?,
    ))
}

fn write_secret_json<T: Serialize>(path: &Path, value: &T) -> DesktopResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| transaction::storage_error())?;
    secret_file::write(path, &bytes).map_err(|_| transaction::storage_error())
}

fn clear_secret_file(path: &Path) -> DesktopResult<()> {
    crate::core::storage::remove_file_durable(path).map_err(|_| transaction::storage_error())
}

/// 由已保存状态重建四文件路径；命令层重启时用它，避免重新发现到别的安装。
pub fn paths_from_state(state: &State) -> DesktopPaths {
    status::state_paths(state)
}

fn edition_of<'a>(editions: &'a [Edition], file_role: &str) -> Option<&'a Edition> {
    editions.iter().find(|edition| edition.role == file_role)
}

fn baseline_meta_entry(editions: &[Edition]) -> Option<MetaEntry> {
    match edition_of(editions, role::META).map(|edition| &edition.baseline) {
        Some(ManagedValues::Meta { own_entry, .. }) => own_entry
            .as_ref()
            .and_then(|value| serde_json::from_value::<MetaEntry>(value.clone()).ok()),
        _ => None,
    }
}

fn baseline_applied_id(editions: &[Edition]) -> Option<String> {
    match edition_of(editions, role::META).map(|edition| &edition.baseline) {
        Some(ManagedValues::Meta { applied_id, .. }) => applied_id
            .as_ref()
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

/// 版本化 revision：覆盖预览涉及的每一项输入。
fn revision_of(
    action: &DesktopAction,
    request: &ActionRequest,
    paths: &DesktopPaths,
    before: &[transaction::FileSnapshot],
    plans: &[transaction::FilePlan],
    models: &[ManagedModelView],
    previous: Option<&State>,
    next_state: &State,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"claude-desktop-revision-v2");
    hasher.update(action.as_str().as_bytes());
    hasher.update(paths.identity().as_bytes());
    // 排除预览中随机生成的 transactionId/time，只绑定会影响实际提交的输入。
    let bytes = serde_json::to_vec(&(
        request,
        before,
        plans,
        models,
        previous,
        &next_state.provider_id,
        &next_state.provider_name,
        &next_state.model_choices,
        &next_state.profile_base_url,
        &next_state.credential_fingerprint,
        next_state.generation,
        next_state.router.as_ref().map(|router| {
            (
                &router.routes,
                &router.file_projection,
                router.route_revision,
                router.gateway_token_version,
                &router.gateway_token,
                router.bound_port,
            )
        }),
        next_state.paths_isolated,
        next_state.paths_overridden,
        next_state.tombstone,
    ))
    .expect("revision inputs are JSON serializable");
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn changed_error() -> Box<DesktopError> {
    Box::new(
        DesktopError::new(error_code::CHANGED, "claudeDesktop.errors.changed")
            .retryable()
            .action("repreview"),
    )
}

fn expected_path<'a>(paths: &'a DesktopPaths, role: &str) -> Option<&'a PathBuf> {
    match role {
        role::NORMAL_CONFIG => Some(&paths.normal_config),
        role::THIRD_PARTY_CONFIG => Some(&paths.third_party_config),
        role::PROFILE => Some(&paths.profile_path),
        role::META => Some(&paths.meta_path),
        _ => None,
    }
}

/// 写前校验：当前文件必须仍等于事务开始时的快照。
fn verify_unchanged(pending: &Pending, plan: &transaction::FilePlan) -> DesktopResult<()> {
    let Some(snapshot) = pending.before.iter().find(|item| item.role == plan.role) else {
        return Err(changed_error());
    };
    let current = transaction::read_text(&plan.path).map_err(|_| transaction::storage_error())?;
    if current != snapshot.text {
        return Err(Box::new(
            DesktopError::new(error_code::CHANGED, "claudeDesktop.errors.externalChange")
                .file(&plan.role)
                .retryable()
                .action("repreview"),
        ));
    }
    Ok(())
}

fn verify_written(plan: &transaction::FilePlan) -> DesktopResult<()> {
    let current = transaction::read_text(&plan.path).map_err(|_| transaction::storage_error())?;
    let failure = || {
        Box::new(
            DesktopError::new(error_code::STORAGE, "claudeDesktop.errors.verifyFailed")
                .file(&plan.role),
        )
    };
    match (&plan.after_text, current) {
        (Some(expected), Some(now)) => {
            // 结构等价即可：外部工具可能只改了缩进。
            let matches = match (
                serde_json::from_str::<Value>(&now),
                serde_json::from_str::<Value>(expected),
            ) {
                (Ok(now), Ok(expected)) => now == expected,
                _ => now == *expected,
            };
            if matches {
                Ok(())
            } else {
                Err(failure())
            }
        }
        (None, None) => Ok(()),
        _ => Err(failure()),
    }
}

// ---------------------------------------------------------------------------
// 版本探测
// ---------------------------------------------------------------------------

/// 探测 Desktop 版本；失败返回 `unknown`，不编造版本。
pub fn detect_desktop(discovery: &paths::Discovery, hint: Option<&str>) -> DesktopDetection {
    let installed = discovery
        .candidates
        .iter()
        .any(|item| item.install_evidence);
    let mut notes = discovery.notes.clone();
    let (version, source) = match hint.filter(|value| paths::is_version_like(value)) {
        Some(hint) => (
            hint.to_owned(),
            "claudeDesktop.versionSources.package".to_owned(),
        ),
        None => match read_version_from_logs(discovery) {
            Some((version, source)) => (version, source),
            None => (
                "unknown".to_owned(),
                "claudeDesktop.versionSources.unknown".to_owned(),
            ),
        },
    };
    if discovery.default.is_none() && !discovery.candidates.is_empty() {
        notes.push("claudeDesktop.notes.selectPaths".to_owned());
    }
    DesktopDetection {
        platform: std::env::consts::OS.to_owned(),
        version,
        version_source: source,
        installed,
        notes,
    }
}

/// 只从本机 Desktop 自己的日志里读版本：可核实、无需执行进程。
///
/// 一个日志文件包含该渠道的**全部历史启动**，因此必须取文件里最后一条
/// `appVersion`：第一条是安装时的旧版本，会严重低报当前版本。
/// 3P 根优先于 normal 根，因为本功能管理的正是 3P 渠道。
fn read_version_from_logs(discovery: &paths::Discovery) -> Option<(String, String)> {
    let mut fallback: Option<(String, String)> = None;
    for candidate in &discovery.candidates {
        for root in [
            &candidate.paths.third_party_root,
            &candidate.paths.normal_root,
        ] {
            let log = root.join("logs").join("main.log");
            if let Some(version) = read_app_version(&log) {
                let source = format!("log:{}", paths::display(&log));
                let is_third_party = root == &candidate.paths.third_party_root;
                if is_third_party {
                    return Some((version, source));
                }
                if fallback.is_none() {
                    fallback = Some((version, source));
                }
            }
        }
    }
    fallback
}

/// 取日志里**最后一次**记录的启动版本；文件的其余内容一概不解析。
fn read_app_version(log: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(log).ok()?;
    let mut latest: Option<String> = None;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Some(rest) = line.split("appVersion:").nth(1) else {
            continue;
        };
        // 日志里既可能是 `appVersion: '1.2.3',` 也可能是双引号或无引号。
        let version = rest
            .trim()
            .trim_start_matches(['"', '\''])
            .trim_end_matches(',')
            .trim_end_matches(['"', '\''])
            .trim();
        if paths::is_version_like(version) {
            latest = Some(version.to_owned());
        }
    }
    latest
}

#[cfg(test)]
mod version_tests {
    use super::read_app_version;

    fn write_log(name: &str, lines: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentharbor-desktop-log-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.log");
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn the_last_app_version_wins_over_the_installed_one() {
        // 日志保留了该渠道的全部历史：首条是安装时的旧版本。
        let log = write_log(
            "history",
            &[
                "2026-06-14 10:08:24 [info] Starting app {",
                "  appVersion: '1.12603.1',",
                "  isPackaged: true,",
                "2026-07-01 09:00:00 [info] Starting app {",
                "  appVersion: '2.2553.0',",
                "  isPackaged: true,",
                "2026-10-04 17:52:00 [info] Starting app {",
                "  appVersion: '2.19675.0',",
                "}",
            ],
        );
        assert_eq!(read_app_version(&log).as_deref(), Some("2.19675.0"));
    }

    #[test]
    fn a_single_entry_and_a_missing_file_are_handled() {
        let log = write_log(
            "single",
            &["  appVersion: '1.12603.1',", "  isPackaged: true,"],
        );
        assert_eq!(read_app_version(&log).as_deref(), Some("1.12603.1"));
        assert!(read_app_version(&log.with_file_name("missing.log")).is_none());
        let junk = write_log("junk", &["no version here", "appVersion: 'unknown'"]);
        assert!(read_app_version(&junk).is_none());
    }
}
