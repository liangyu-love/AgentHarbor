//! Claude Desktop 直连：状态、持久 journal 与四文件提交。
//!
//! 这是**四个单文件原子替换的协调事务**，不是四文件同时可见的系统事务。
//! 因此这里做三件事：
//! 1. 提交点之前把计划写进持久 pending，进程中断后可判定实际文件态；
//! 2. 所有权记录只撤回“能证明由本次管理写入”的字段与文件；
//! 3. 恢复幂等：只有当前值与预期吻合才写，可重复执行。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::core::{secret_file, storage};

use super::constants::MAX_BACKUPS;
use super::paths::{self, role, DesktopPaths};
use super::profile::{self, MetaDocument, MetaEntry};
use super::types::{
    error_code, DesktopError, DesktopModelChoice, DesktopResult, FileChangeView, ManagedChangeView,
    ManagedModelView,
};

pub const STATE_SCHEMA: u32 = 1;

// ---------------------------------------------------------------------------
// 记录类型
// ---------------------------------------------------------------------------

/// 受管值的领域形式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ManagedValues {
    /// `deploymentMode` 等标量字段。
    Fields { values: Map<String, Value> },
    /// `_meta.json`：自身条目 + 接管期间的 `appliedId`。
    Meta {
        #[serde(default)]
        own_entry: Option<Value>,
        /// `None` 表示该键当前不存在。
        #[serde(default)]
        applied_id: Option<Value>,
    },
    /// 自身 profile 是整体受管。
    Whole { value: Option<Value> },
}

impl ManagedValues {
    /// `self` 为期望值、`now` 为现状时，现状是否已被外部改写。
    pub fn drifted(&self, now: &ManagedValues) -> bool {
        match (self, now) {
            (Self::Fields { values }, Self::Fields { values: current }) => values
                .iter()
                .any(|(key, expected)| current.get(key) != Some(expected)),
            (
                Self::Meta {
                    own_entry,
                    applied_id,
                },
                Self::Meta {
                    own_entry: now_entry,
                    applied_id: now_id,
                },
            ) => {
                let entry_same = match (own_entry, now_entry) {
                    (None, None) => true,
                    (Some(expected), Some(now)) => expected == now,
                    // 接管前无条目却出现了条目，说明外部写入，按漂移处理。
                    _ => false,
                };
                let applied_same = match (applied_id, now_id) {
                    (None, None) => true,
                    (Some(expected), Some(now)) => expected == now,
                    _ => false,
                };
                !(entry_same && applied_same)
            }
            (Self::Whole { value }, Self::Whole { value: now }) => match (value, now) {
                (None, None) => false,
                (Some(expected), Some(now)) => expected != now,
                _ => true,
            },
            // 类型不同说明记录与现状不匹配，必须报冲突而不是猜。
            _ => true,
        }
    }
}

/// 一个受管目标的 before/after。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Edition {
    pub role: String,
    /// 事务开始前该文件是否存在。
    pub existed_before: bool,
    /// 文件级 before 原文；不存在为 `None`。
    pub before: Option<String>,
    /// 收回管理时要把受管目标恢复成的值。
    pub baseline: ManagedValues,
    /// 最近一次成功安装的受管值（漂移检测的比较基准）。
    pub installed: ManagedValues,
}

impl Edition {
    /// 事务写入了该文件时，按计划记录“最近安装值”。
    pub fn applied(&self, after: &ManagedValues) -> Self {
        let mut next = self.clone();
        next.installed = after.clone();
        next
    }
}

/// 从一份文档里读出该 edition 关心的受管值。
fn read_managed(document: &Value, edition: &Edition) -> ManagedValues {
    match &edition.baseline {
        ManagedValues::Fields { .. } => {
            let map = document.as_object();
            let mut out = Map::new();
            // 基线可能没有该键（接管前文件不存在），受管键的并集才完整。
            for key in managed_keys(edition) {
                if let Some(value) = map.and_then(|map| map.get(&key)) {
                    out.insert(key, value.clone());
                }
            }
            ManagedValues::Fields { values: out }
        }
        ManagedValues::Meta { .. } => meta_values(document),
        ManagedValues::Whole { .. } => ManagedValues::Whole {
            value: Some(document.clone()),
        },
    }
}

fn meta_values(document: &Value) -> ManagedValues {
    let text = serde_json::to_string(document).unwrap_or_default();
    match MetaDocument::parse(Some(&text)) {
        Ok(meta) => ManagedValues::Meta {
            own_entry: meta
                .entry_list()
                .ok()
                .and_then(|entries| entries.into_iter().find(MetaEntry::is_own))
                .map(|entry| entry.to_value()),
            applied_id: meta.applied_id.clone(),
        },
        Err(_) => ManagedValues::Meta {
            own_entry: None,
            applied_id: None,
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyValue {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// 事务中单个文件的目标内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePlan {
    pub role: String,
    pub path: PathBuf,
    /// `None` 表示事务后应删除该文件。
    pub after_text: Option<String>,
    /// 该文件在事务结束时应有的受管值。
    pub after: ManagedValues,
    /// 供预览显示的受管变更；秘密值固定脱敏。
    pub after_keys: Vec<KeyValue>,
}

impl FilePlan {
    /// 预览里显示的动作。由「写入前是否存在」与「事务后是否留下内容」推导，
    /// 不再单独存一份：两个字段各存一次就可能互相矛盾。
    pub fn kind(&self, existed_before: bool) -> &'static str {
        match (existed_before, self.after_text.is_some()) {
            (false, true) => "create",
            (true, true) => "update",
            (true, false) => "delete",
            (false, false) => "noop",
        }
    }

    pub fn view(&self, existed_before: bool) -> FileChangeView {
        FileChangeView {
            role: self.role.clone(),
            path: paths::display(&self.path),
            kind: self.kind(existed_before).to_owned(),
            managed_changes: self
                .after_keys
                .iter()
                .map(|change| ManagedChangeView {
                    path: change.path.clone(),
                    before: change.before.clone(),
                    after: change.after.clone(),
                })
                .collect(),
            note: None,
        }
    }
}

/// 渲染预览。`existed_before` 由调用方从基线里取：计划本身不存这个信息。
pub fn views(plans: &[FilePlan], existed_before: impl Fn(&str) -> bool) -> Vec<FileChangeView> {
    plans
        .iter()
        .map(|plan| plan.view(existed_before(&plan.role)))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSnapshot {
    pub role: String,
    pub path: PathBuf,
    pub existed: bool,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppliedMode {
    Direct,
    Router,
    Official,
}

/// 状态里的「受管文件」部分。模型列表只保留在 [`State::models`] 一处，
/// 两份副本既冗余又可能不一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedData {
    pub files: Vec<Edition>,
    pub applied_mode: AppliedMode,
}

/// 重启时只补回模式字段；保留原有文件的存在状态，不保存其它偏好或凭据。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartMode {
    pub value: Option<String>,
    pub file_present: bool,
}

/// Desktop 内部状态。所有包含文件快照或凭据的记录都用 `secret_file` 写入。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub schema: u32,
    pub transaction_id: String,
    pub generation: u64,
    pub paths_identity: String,
    pub normal_root: PathBuf,
    pub third_party_root: PathBuf,
    #[serde(default)]
    pub paths_isolated: bool,
    #[serde(default)]
    pub paths_overridden: bool,
    pub provider_id: String,
    pub provider_name: String,
    /// 上次应用时所用推理密钥的**指纹**（SHA-256 十六进制）。
    ///
    /// 只存指纹不存密钥：足以判断「卡片换了密钥、Desktop 还在用旧的」，
    /// 又不会在受保护记录里多留一份明文副本。旧状态没有该字段时为 `None`，
    /// 此时不据此报 stale（宁可漏报也不误报）。
    #[serde(default)]
    pub credential_fingerprint: Option<String>,
    pub models: Vec<ManagedModelView>,
    /// 用户原始选择：Auto/Off 和显式展示名必须在重新应用时保留。
    #[serde(default)]
    pub model_choices: Vec<DesktopModelChoice>,
    /// Desktop 实际使用的端点根，用于判断供应商端点变更。
    #[serde(default)]
    pub profile_base_url: Option<String>,
    #[serde(default)]
    pub router: Option<super::gateway::RouterConfig>,
    pub last_applied_at: i64,
    pub restart_required: bool,
    /// Only the two owned mode fields need reasserting after Desktop flushes
    /// its in-memory preferences on quit. No credentials or restore baseline.
    #[serde(default)]
    pub restart_modes: std::collections::BTreeMap<String, RestartMode>,
    pub data: ManagedData,
    /// 释放管理后留下的非秘密 tombstone。
    #[serde(default)]
    pub tombstone: bool,
}

impl State {
    /// 释放管理：结束恢复责任但保留识别信息，不再持有任何凭据或文件快照。
    pub fn into_tombstone(mut self, generation: u64, transaction_id: &str) -> Self {
        self.generation = generation;
        self.transaction_id = transaction_id.to_owned();
        self.tombstone = true;
        self.data.files.clear();
        self.models.clear();
        self.model_choices.clear();
        self.profile_base_url = None;
        self.router = None;
        // 管理已结束：不再保留凭据指纹。
        self.credential_fingerprint = None;
        self.restart_required = false;
        self.restart_modes.clear();
        self
    }

    pub fn edition(&self, file_role: &str) -> Option<&Edition> {
        self.data
            .files
            .iter()
            .find(|edition| edition.role == file_role)
    }
}

/// 持久 journal。`next_state` 与 state 的 transactionId/generation 一致即为提交点。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub schema: u32,
    pub transaction_id: String,
    pub generation: u64,
    pub action: String,
    pub created_at: i64,
    pub paths_identity: String,
    pub files: Vec<FilePlan>,
    /// 四个文件的 before/after 快照。
    pub before: Vec<FileSnapshot>,
    pub previous_state: Option<State>,
    pub next_state: State,
    pub backup_id: Option<String>,
}

// ---------------------------------------------------------------------------
// 读写辅助
// ---------------------------------------------------------------------------

/// 读取一个文件的当前文本（不存在返回 `None`）。
pub fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn storage_error() -> Box<DesktopError> {
    Box::new(DesktopError::new(error_code::STORAGE, "claudeDesktop.errors.storage").retryable())
}

pub fn conflict_error() -> Box<DesktopError> {
    Box::new(
        DesktopError::new(error_code::CONFLICT, "claudeDesktop.errors.conflict")
            .action("rebaseline"),
    )
}

fn read_text_or(path: &Path) -> DesktopResult<Option<String>> {
    read_text(path).map_err(|_| storage_error())
}

fn write_text(path: &Path, text: &str) -> DesktopResult<()> {
    storage::write_atomic(path, text.as_bytes()).map_err(|_| storage_error())
}

pub fn write_document(path: &Path, document: &Value, file_role: &str) -> DesktopResult<()> {
    let text = profile::to_text(document, file_role)?;
    write_text(path, &text)
}

pub fn remove_file(path: &Path) -> DesktopResult<()> {
    storage::remove_file_durable(path).map_err(|_| storage_error())
}

pub fn render(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 基线
// ---------------------------------------------------------------------------

/// 共享 config 的基线：接管前是否存在、原文与受管字段原值。
pub fn config_edition(before: Option<&str>, file_role: &str) -> DesktopResult<Edition> {
    let map = profile::parse_object(before, file_role)?;
    let mut values = Map::new();
    if let Some(value) = map.get(profile::KEY_DEPLOYMENT_MODE) {
        values.insert(profile::KEY_DEPLOYMENT_MODE.to_owned(), value.clone());
    }
    let baseline = ManagedValues::Fields { values };
    Ok(Edition {
        role: file_role.to_owned(),
        existed_before: before.is_some(),
        before: before.map(str::to_owned),
        installed: baseline.clone(),
        baseline,
    })
}

/// `_meta.json` 的基线：只记录自身条目与当前 `appliedId`。
pub fn meta_edition(before: Option<&str>) -> DesktopResult<Edition> {
    let meta = MetaDocument::parse(before)?;
    let own_entry = meta
        .entry_list()?
        .into_iter()
        .find(MetaEntry::is_own)
        .map(|entry| entry.to_value());
    let baseline = ManagedValues::Meta {
        own_entry,
        applied_id: meta.applied_id.clone(),
    };
    Ok(Edition {
        role: role::META.to_owned(),
        existed_before: before.is_some(),
        before: before.map(str::to_owned),
        installed: baseline.clone(),
        baseline,
    })
}

pub fn profile_edition(before: Option<&str>) -> DesktopResult<Edition> {
    let value = match before {
        Some(text) if !text.trim().is_empty() => {
            Some(serde_json::from_str::<Value>(text).map_err(|_| {
                Box::new(
                    DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.invalidJson")
                        .file(role::PROFILE),
                )
            })?)
        }
        _ => None,
    };
    let baseline = ManagedValues::Whole {
        value: value.clone(),
    };
    Ok(Edition {
        role: role::PROFILE.to_owned(),
        existed_before: before.is_some(),
        before: before.map(str::to_owned),
        installed: baseline.clone(),
        baseline,
    })
}

/// 读取四项基线。任一文件结构非法时返回定位到该文件的错误，调用方零写入。
pub fn read_editions(paths: &DesktopPaths) -> DesktopResult<Vec<Edition>> {
    Ok(vec![
        profile_edition(read_text_or(&paths.profile_path)?.as_deref())?,
        meta_edition(read_text_or(&paths.meta_path)?.as_deref())?,
        config_edition(
            read_text_or(&paths.third_party_config)?.as_deref(),
            role::THIRD_PARTY_CONFIG,
        )?,
        config_edition(
            read_text_or(&paths.normal_config)?.as_deref(),
            role::NORMAL_CONFIG,
        )?,
    ])
}

/// 计划前的受管值。
///
/// - 普通 reapply（`rebaseline = false`）沿用既有基线：恢复点始终是**首次接管**
///   时的状态，不能被后来的现状悄悄替换。
/// - 显式 `rebaseline = true`（“以现状重新接管”）则改用磁盘现状作为新的基线与
///   安装值：这正是该操作被设计出来的用途。
pub fn editions_before(
    paths: &DesktopPaths,
    previous: Option<&State>,
    rebaseline: bool,
) -> DesktopResult<Vec<Edition>> {
    let fresh = read_editions(paths)?;
    if rebaseline {
        return Ok(fresh);
    }
    Ok(fresh
        .iter()
        .map(
            |edition| match previous.and_then(|state| state.edition(&edition.role)) {
                Some(kept) => kept.clone(),
                None => edition.clone(),
            },
        )
        .collect())
}

// ---------------------------------------------------------------------------
// 计划
// ---------------------------------------------------------------------------

/// 每个受管目标的目标值。
#[derive(Debug, Clone)]
pub struct DesiredState {
    /// `Some`：两处 config 都写成该模式；`None`：各自恢复自己的接管前取值。
    pub mode: Option<&'static str>,
    /// 要写入的 profile；`None` 表示撤回自己写入的内容。
    pub profile: Option<Value>,
    pub own_entry: Option<MetaEntry>,
    pub applied_id: Option<String>,
    /// meta entries 中自身条目的插入位置。
    pub entry_insert_at: usize,
}

fn profile_plan_keys(before: Option<&str>, after: Option<&Value>) -> Vec<KeyValue> {
    let before = before.and_then(|text| serde_json::from_str::<Value>(text).ok());
    [
        profile::KEY_BASE_URL,
        profile::KEY_DISABLE_CHOOSER,
        profile::KEY_INFERENCE_PROVIDER,
        profile::KEY_AUTH_SCHEME,
        profile::KEY_MODELS,
        profile::KEY_EGRESS_HOSTS,
        profile::KEY_API_KEY,
    ]
    .into_iter()
    .map(|key| KeyValue {
        path: key.to_owned(),
        before: before
            .as_ref()
            .and_then(|value| profile_preview_value(value, key)),
        after: after.and_then(|value| profile_preview_value(value, key)),
    })
    .collect()
}

/// 只投影 profile 契约字段；外部模型对象的未知扩展也不能夹带秘密进入预览。
fn profile_preview_value(document: &Value, key: &str) -> Option<String> {
    let value = document.get(key)?;
    match key {
        profile::KEY_API_KEY => Some("<redacted>".to_owned()),
        profile::KEY_MODELS => {
            let models = value
                .as_array()?
                .iter()
                .filter_map(|model| {
                    if model.is_string() {
                        return Some(model.clone());
                    }
                    let object = model.as_object()?;
                    let name = object.get("name")?.as_str()?;
                    let mut safe = Map::new();
                    safe.insert("name".into(), Value::String(name.to_owned()));
                    if let Some(label) = object.get("labelOverride").and_then(Value::as_str) {
                        safe.insert("labelOverride".into(), Value::String(label.to_owned()));
                    }
                    if let Some(enabled) = object.get("supports1m").and_then(Value::as_bool) {
                        safe.insert("supports1m".into(), Value::Bool(enabled));
                    }
                    Some(Value::Object(safe))
                })
                .collect();
            Some(render(&Value::Array(models)))
        }
        profile::KEY_DISABLE_CHOOSER => value.as_bool().map(|enabled| enabled.to_string()),
        profile::KEY_EGRESS_HOSTS => value.as_array().map(|hosts| {
            render(&Value::Array(
                hosts
                    .iter()
                    .filter(|host| host.is_string())
                    .cloned()
                    .collect(),
            ))
        }),
        _ => value.as_str().map(str::to_owned),
    }
}

/// 接管时 profile 文件的状态。
///
/// **必须**由调用方从基线给出，不能从磁盘现状推断：普通 reapply 时该文件当然
/// 存在（那是我们自己写的），按现状判断会得出「接管前就有 profile」，于是
/// 「恢复接管前配置」会写回自己的 profile 而不是删除它。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileBaseline {
    /// 接管前该文件不存在：撤回应删除它。
    Absent,
    /// 接管前它已存在，内容已记录：撤回应写回这份内容。
    PreExisting(Value),
    /// 无法判断接管前的状态。
    ///
    /// 只在「没有管理记录、但磁盘上已有一个 profile」时出现。此时**不声称**
    /// 它就是接管前的内容——那份记录已经丢了，声称等于把工具自己的旧文件当成
    /// 用户原有配置，「恢复接管前配置」会写回自己。这种状态应当接管现状并提示，
    /// 而不是猜。
    Unknown,
}

impl Default for ProfileBaseline {
    fn default() -> Self {
        Self::Absent
    }
}

/// 接管时两处 config 的 `deploymentMode`，以及 profile 的状态。
#[derive(Debug, Clone, Default)]
pub struct TakeoverBaseline {
    pub profile: ProfileBaseline,
    /// 原文仅在撤回自己 profile 时使用，确保既有文件字节不被重排。
    pub profile_text: Option<String>,
    pub meta_before: Option<String>,
    pub configs_existed: std::collections::HashMap<String, bool>,
    /// 按文件角色索引的接管前模式。
    pub modes: std::collections::HashMap<String, String>,
}
impl TakeoverBaseline {
    /// 首次接管（或显式 rebaseline）前的四文件状态。
    ///
    /// `profile_managed` 表示「磁盘上这个 profile 是本工具自己写的」。据此决定
    /// 基线是 `Absent`（我们自己写的，撤回应删除）还是 `PreExisting`（用户原有）。
    pub fn from_editions(editions: &[Edition], profile_managed: bool) -> Self {
        let mut baseline = Self::default();
        for edition in editions {
            match (edition.role.as_str(), &edition.baseline) {
                (role::PROFILE, ManagedValues::Whole { value }) => {
                    baseline.profile_text = edition.before.clone();
                    baseline.profile = match value {
                        None => ProfileBaseline::Absent,
                        Some(text) => {
                            if profile_managed {
                                // 磁盘上那份是我们自己写的，不是接管前的内容。
                                ProfileBaseline::Absent
                            } else {
                                // 我们没写过它，只能按现状接管，并按原样记录。
                                ProfileBaseline::PreExisting(text.clone())
                            }
                        }
                    };
                }
                (_, ManagedValues::Fields { values }) => {
                    baseline
                        .configs_existed
                        .insert(edition.role.clone(), edition.existed_before);
                    if let Some(mode) = values
                        .get(profile::KEY_DEPLOYMENT_MODE)
                        .and_then(Value::as_str)
                    {
                        baseline.modes.insert(edition.role.clone(), mode.to_owned());
                    }
                }
                (role::META, _) => baseline.meta_before = edition.before.clone(),
                _ => {}
            }
        }
        baseline
    }
}
/// 依据当前四文件、接管基线与目标状态构造写入计划。
pub fn build_plan(
    paths: &DesktopPaths,
    desired: &DesiredState,
    baseline: &TakeoverBaseline,
) -> DesktopResult<Vec<FilePlan>> {
    let profile_before = read_text_or(&paths.profile_path)?;
    let meta_before = read_text_or(&paths.meta_path)?;
    let threep_before = read_text_or(&paths.third_party_config)?;
    let normal_before = read_text_or(&paths.normal_config)?;

    let mut plans: Vec<FilePlan> = Vec::new();

    // 顺序：自己的 profile → meta 注册/选中 → 3P config → normal config。
    // 先准备好 profile 再切 normal mode，减少指向尚不存在 profile 的时间。
    match &desired.profile {
        Some(value) => {
            let text = profile::to_text(value, role::PROFILE)?;
            if profile_before.as_deref() != Some(text.as_str()) {
                plans.push(FilePlan {
                    role: role::PROFILE.to_owned(),
                    path: paths.profile_path.clone(),
                    after_text: Some(text),
                    after: ManagedValues::Whole {
                        value: Some(value.clone()),
                    },
                    after_keys: profile_plan_keys(profile_before.as_deref(), Some(value)),
                });
            }
        }
        None => {
            match &baseline.profile {
                // 接管前就有内容：写回原始字节，不能凭同一个 id 删除。
                ProfileBaseline::PreExisting(original) => {
                    let text = match &baseline.profile_text {
                        Some(text) => text.clone(),
                        None => profile::to_text(original, role::PROFILE)?,
                    };
                    if profile_before.as_deref() != Some(text.as_str()) {
                        plans.push(FilePlan {
                            role: role::PROFILE.to_owned(),
                            path: paths.profile_path.clone(),
                            after_text: Some(text),
                            after: ManagedValues::Whole {
                                value: Some(original.clone()),
                            },
                            after_keys: profile_plan_keys(
                                profile_before.as_deref(),
                                Some(original),
                            ),
                        });
                    }
                }
                // 状态不明：不写、不删，让上层先做一次显式接管。
                ProfileBaseline::Unknown => {}
                ProfileBaseline::Absent => {}
            }
            if baseline.profile == ProfileBaseline::Absent && profile_before.is_some() {
                // 接管前不存在：这就是我们写的，撤回应删除它。
                plans.push(FilePlan {
                    role: role::PROFILE.to_owned(),
                    path: paths.profile_path.clone(),
                    after_text: None,
                    after: ManagedValues::Whole { value: None },
                    after_keys: profile_plan_keys(profile_before.as_deref(), None),
                });
            }
        }
    }

    let meta_document = MetaDocument::parse(meta_before.as_deref())?;
    let mut meta_next = meta_document.render(
        desired.own_entry.as_ref(),
        Some(desired.applied_id.as_deref()),
        desired.entry_insert_at,
    )?;
    if desired.mode.is_none() && desired.profile.is_none() {
        let original = MetaDocument::parse(baseline.meta_before.as_deref())?;
        let original_entries = original.entry_list()?;
        let original_index = original_entries
            .iter()
            .position(MetaEntry::is_own)
            .unwrap_or(0);
        meta_next = meta_document.render(desired.own_entry.as_ref(), Some(None), original_index)?;
        if let Some(object) = meta_next.as_object_mut() {
            if let Some(value) = original.applied_id {
                object.insert(profile::KEY_APPLIED_ID.to_owned(), value);
            }
            if original.entries.is_none()
                && object
                    .get(profile::KEY_ENTRIES)
                    .and_then(Value::as_array)
                    .is_some_and(|entries| entries.is_empty())
            {
                object.remove(profile::KEY_ENTRIES);
            }
        }
    }
    let meta_current = Value::Object(profile::parse_object(meta_before.as_deref(), role::META)?);
    if meta_next != meta_current {
        let own_present = meta_document.own_entry_present()?;
        let mut keys = Vec::new();
        if own_present != desired.own_entry.is_some() {
            keys.push(KeyValue {
                path: format!("{}.{}", profile::KEY_ENTRIES, super::constants::PROFILE_ID),
                before: Some(own_state_label(own_present)),
                after: Some(own_state_label(desired.own_entry.is_some())),
            });
        }
        let before_applied = meta_document.applied_id_str().map(str::to_owned);
        if before_applied != desired.applied_id {
            keys.push(KeyValue {
                path: profile::KEY_APPLIED_ID.to_owned(),
                before: Some(applied_label(
                    before_applied.as_deref(),
                    &desired.applied_id,
                )),
                after: Some(applied_label(
                    desired.applied_id.as_deref(),
                    &before_applied,
                )),
            });
        }
        let after_text = if desired.mode.is_none()
            && desired.profile.is_none()
            && baseline.meta_before.is_none()
            && meta_is_empty(&meta_next)
        {
            None
        } else {
            Some(profile::to_text(&meta_next, role::META)?)
        };
        plans.push(FilePlan {
            role: role::META.to_owned(),
            path: paths.meta_path.clone(),
            after_text,
            after: meta_values(&meta_next),
            after_keys: keys,
        });
    }

    for (file_role, path, before) in [
        (
            role::THIRD_PARTY_CONFIG,
            &paths.third_party_config,
            &threep_before,
        ),
        (role::NORMAL_CONFIG, &paths.normal_config, &normal_before),
    ] {
        let mut map = profile::parse_object(before.as_deref(), file_role)?;
        let before_mode = profile::deployment_mode(&map).map(str::to_owned);
        // 撤销接管时两处文件各自恢复自己的取值，不能共用一个模式：
        // 接管前 normal 是 1p、3P 是 3p 的情况很常见。
        let target = match desired.mode {
            Some(mode) => Some(mode.to_owned()),
            None => baseline.modes.get(file_role).cloned(),
        };
        let after_value = match (&target, before_mode.as_deref()) {
            (Some(target), Some(current)) if target == current => continue,
            (Some(target), _) => {
                profile::patch_deployment_mode(&mut map, target);
                Value::String(target.clone())
            }
            // 接管前没有该键且不指定模式：撤回时删除它，而不是写一个猜测值。
            (None, None) => continue,
            (None, Some(_)) => {
                map.remove(profile::KEY_DEPLOYMENT_MODE);
                Value::Null
            }
        };
        let after_fields = if after_value.is_null() {
            Map::new()
        } else {
            single_field(profile::KEY_DEPLOYMENT_MODE, after_value.clone())
        };
        let after_text = if desired.mode.is_none()
            && map.is_empty()
            && baseline.configs_existed.get(file_role) == Some(&false)
        {
            None
        } else {
            Some(profile::to_text(&Value::Object(map), file_role)?)
        };
        plans.push(FilePlan {
            role: file_role.to_owned(),
            path: path.to_path_buf(),
            after_text,
            after: ManagedValues::Fields {
                values: after_fields,
            },
            after_keys: vec![KeyValue {
                path: profile::KEY_DEPLOYMENT_MODE.to_owned(),
                before: before_mode,
                after: after_value.as_str().map(str::to_owned),
            }],
        });
    }

    if desired.profile.is_none() {
        // 撤回时先取消启用/选中，最后撤回 profile，避免指向已删除文件。
        plans.sort_by_key(|plan| match plan.role.as_str() {
            role::NORMAL_CONFIG => 0,
            role::THIRD_PARTY_CONFIG => 1,
            role::META => 2,
            _ => 3,
        });
    }
    Ok(plans)
}

/// 未接管时切回官方仅需两处模式字段。外部 profile/meta 不参与解析与写入。
pub fn official_mode_plan(paths: &DesktopPaths) -> DesktopResult<Vec<FilePlan>> {
    let mut plans = Vec::new();
    for (file_role, path) in [
        (role::NORMAL_CONFIG, &paths.normal_config),
        (role::THIRD_PARTY_CONFIG, &paths.third_party_config),
    ] {
        let before = read_text_or(path)?;
        let mut map = profile::parse_object(before.as_deref(), file_role)?;
        let before_mode = profile::deployment_mode(&map).map(str::to_owned);
        if before_mode.as_deref() == Some(profile::official_mode()) {
            continue;
        }
        profile::patch_deployment_mode(&mut map, profile::official_mode());
        plans.push(FilePlan {
            role: file_role.to_owned(),
            path: path.clone(),
            after_text: Some(profile::to_text(&Value::Object(map), file_role)?),
            after: ManagedValues::Fields {
                values: single_field(
                    profile::KEY_DEPLOYMENT_MODE,
                    Value::String(profile::official_mode().into()),
                ),
            },
            after_keys: vec![KeyValue {
                path: profile::KEY_DEPLOYMENT_MODE.to_owned(),
                before: before_mode,
                after: Some(profile::official_mode().to_owned()),
            }],
        });
    }
    Ok(plans)
}

fn single_field(key: &str, value: Value) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    map
}

fn own_state_label(present: bool) -> String {
    if present {
        "present".to_owned()
    } else {
        "absent".to_owned()
    }
}

/// `appliedId` 的展示值：只区分“是自己 / 是别人 / 没有”，不回传外部 profile 原文。
fn applied_label(value: Option<&str>, other: &Option<String>) -> String {
    match value {
        None => "<none>".to_owned(),
        Some(id) if id == super::constants::PROFILE_ID => "agentharbor".to_owned(),
        Some(id) => {
            let _ = other;
            let _ = id;
            "external".to_owned()
        }
    }
}

// ---------------------------------------------------------------------------
// 状态演算
// ---------------------------------------------------------------------------

/// 用计划结果演算出下一份状态。
///
/// `before` 必须是**事务开始前**的四项基线（见 [`editions_before`]）；每个文件
/// 的接管基线在其中保持不变，只有最近安装值随计划更新。
pub fn next_state(
    before: &[Edition],
    plans: &[FilePlan],
    paths: &DesktopPaths,
    generation: u64,
    transaction_id: &str,
    provider_id: &str,
    provider_name: &str,
    credential_fingerprint: Option<String>,
    models: &[ManagedModelView],
    applied_mode: AppliedMode,
) -> DesktopResult<State> {
    let mut files: Vec<Edition> = Vec::new();
    for edition in before {
        match plans.iter().find(|plan| plan.role == edition.role) {
            Some(plan) => files.push(edition.applied(&plan.after)),
            // 本次没有改动这个文件时，`installed` 必须原样保留：它记录的是
            // **上一次成功写入的值**，不是“文件里现在恰好是什么”。用事务前的
            // 快照去重算会把自己先前写入的 meta 条目擦掉，于是刚应用完就报 drift。
            None => files.push(edition.clone()),
        }
    }

    Ok(State {
        schema: STATE_SCHEMA,
        transaction_id: transaction_id.to_owned(),
        generation,
        paths_identity: paths.identity(),
        normal_root: paths.normal_root.clone(),
        third_party_root: paths.third_party_root.clone(),
        paths_isolated: paths.isolated,
        paths_overridden: paths.overridden,
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        credential_fingerprint: credential_fingerprint.clone(),
        models: models.to_vec(),
        model_choices: Vec::new(),
        profile_base_url: None,
        router: None,
        last_applied_at: chrono::Utc::now().timestamp(),
        restart_required: true,
        restart_modes: std::collections::BTreeMap::new(),
        data: ManagedData {
            files,
            applied_mode,
        },
        tombstone: false,
    })
}

fn edition_path(edition: &Edition, paths: &DesktopPaths) -> Option<PathBuf> {
    Some(match edition.role.as_str() {
        role::NORMAL_CONFIG => paths.normal_config.clone(),
        role::THIRD_PARTY_CONFIG => paths.third_party_config.clone(),
        role::PROFILE => paths.profile_path.clone(),
        role::META => paths.meta_path.clone(),
        _ => return None,
    })
}

/// Retain only the mode fields while a saved configuration awaits activation.
/// Reapplying or ending management must not discard that pending intent.
pub fn restart_modes(
    state: &State,
    previous: Option<&State>,
    plans: &[FilePlan],
) -> std::collections::BTreeMap<String, RestartMode> {
    let mut modes = previous
        .filter(|old| old.restart_required && old.paths_identity == state.paths_identity)
        .map(|old| old.restart_modes.clone())
        .unwrap_or_default();
    for edition in &state.data.files {
        if !matches!(edition.role.as_str(), role::NORMAL_CONFIG | role::THIRD_PARTY_CONFIG) { continue; }
        if let ManagedValues::Fields { values } = &edition.installed {
            let value = values.get(profile::KEY_DEPLOYMENT_MODE).and_then(Value::as_str).map(str::to_owned);
            modes.insert(edition.role.clone(), RestartMode {
                file_present: value.is_some() || edition.existed_before,
                value,
            });
        }
    }
    for plan in plans {
        if !matches!(plan.role.as_str(), role::NORMAL_CONFIG | role::THIRD_PARTY_CONFIG) { continue; }
        if let ManagedValues::Fields { values } = &plan.after {
            modes.insert(plan.role.clone(), RestartMode {
                value: values.get(profile::KEY_DEPLOYMENT_MODE).and_then(Value::as_str).map(str::to_owned),
                file_present: plan.after_text.is_some(),
            });
        }
    }
    modes
}

/// Called only after a verified shutdown. Merge mode fields into the latest
/// preferences so Desktop's quit-time flush cannot undo the saved selection.
pub fn restart_mode_plan(
    paths: &DesktopPaths,
    modes: &std::collections::BTreeMap<String, RestartMode>,
    snapshots: &[FileSnapshot],
) -> DesktopResult<Vec<FilePlan>> {
    let mut plans = Vec::new();
    for (file_role, path) in [
        (role::THIRD_PARTY_CONFIG, &paths.third_party_config),
        (role::NORMAL_CONFIG, &paths.normal_config),
    ] {
        let Some(target) = modes.get(file_role) else { continue; };
        if target.value.as_deref().is_some_and(|value| {
            value != profile::official_mode() && value != profile::third_party_mode()
        }) {
            return Err(conflict_error());
        }
        let before = snapshots.iter()
            .find(|snapshot| snapshot.role == file_role)
            .ok_or_else(conflict_error)?
            .text.as_deref();
        let mut object = profile::parse_object(before, file_role)?;
        let desired = target.value.as_ref().map(|value| Value::String(value.clone()));
        let delete_empty_file = before.is_some() && object.is_empty() && !target.file_present;
        let create_empty_file = before.is_none() && target.file_present;
        if object.get(profile::KEY_DEPLOYMENT_MODE) == desired.as_ref()
            && !delete_empty_file && !create_empty_file {
            continue;
        }
        match &target.value {
            Some(value) => { profile::patch_deployment_mode(&mut object, value); }
            None => { object.remove(profile::KEY_DEPLOYMENT_MODE); }
        }
        let values = object.get(profile::KEY_DEPLOYMENT_MODE)
            .map(|value| single_field(profile::KEY_DEPLOYMENT_MODE, value.clone()))
            .unwrap_or_default();
        let after_text = if object.is_empty() && !target.file_present {
            None
        } else {
            Some(profile::to_text(&Value::Object(object), file_role)?)
        };
        plans.push(FilePlan {
            role: file_role.to_owned(), path: path.clone(), after_text,
            after: ManagedValues::Fields { values }, after_keys: Vec::new(),
        });
    }
    Ok(plans)
}

// ---------------------------------------------------------------------------
// 漂移检测
// ---------------------------------------------------------------------------

/// 一个受管文件现在的受管值；路径不属于本会话时为 `None`。
fn current_managed(edition: &Edition, paths: &DesktopPaths) -> DesktopResult<Option<ManagedValues>> {
    let Some(path) = edition_path(edition, paths) else {
        return Ok(None);
    };
    let text = read_text_or(&path)?;
    let current = match &text {
        None => null_for(edition),
        Some(text) => parse_for(edition, text)?,
    };
    Ok(Some(if text.is_none() && matches!(edition.baseline, ManagedValues::Whole { .. }) {
        ManagedValues::Whole { value: None }
    } else {
        read_managed(&current, edition)
    }))
}

/// 检查最近安装值是否仍等于当前受管值。返回发生漂移的文件角色。
pub fn drifted_roles(state: &State, paths: &DesktopPaths) -> DesktopResult<Vec<String>> {
    let mut out = Vec::new();
    for edition in &state.data.files {
        if current_managed(edition, paths)?.is_some_and(|now| edition.installed.drifted(&now)) {
            out.push(edition.role.clone());
        }
    }
    Ok(out)
}

/// Roles whose managed values already equal the takeover baseline exactly:
/// restoring them changes nothing, so whatever brought them back there (the
/// user, or Desktop writing its mode on quit) is no conflict for a restore.
pub fn at_baseline_roles(state: &State, paths: &DesktopPaths) -> DesktopResult<Vec<String>> {
    let mut out = Vec::new();
    for edition in &state.data.files {
        if current_managed(edition, paths)?.is_some_and(|now| now == edition.baseline) {
            out.push(edition.role.clone());
        }
    }
    Ok(out)
}

fn null_for(edition: &Edition) -> Value {
    match &edition.baseline {
        ManagedValues::Fields { .. } => Value::Object(Map::new()),
        _ => Value::Null,
    }
}

fn parse_for(edition: &Edition, text: &str) -> DesktopResult<Value> {
    Ok(match edition.role.as_str() {
        role::THIRD_PARTY_CONFIG | role::NORMAL_CONFIG => {
            Value::Object(profile::parse_object(Some(text), &edition.role)?)
        }
        _ => serde_json::from_str(text).map_err(|_| {
            Box::new(
                DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.invalidJson")
                    .file(&edition.role),
            )
        })?,
    })
}

// ---------------------------------------------------------------------------
// 恢复
// ---------------------------------------------------------------------------

/// 依据基线撤回本工具写入的受管目标。幂等：只有当前值与预期吻合才写。
pub fn restore_edition(edition: &Edition, path: &Path, after: &ManagedValues) -> DesktopResult<()> {
    let after_text = match after {
        ManagedValues::Whole { value: Some(value) } => {
            Some(profile::to_text(value, &edition.role)?)
        }
        _ => None,
    };
    let plan = FilePlan {
        role: edition.role.clone(),
        path: path.to_path_buf(),
        after_text,
        after: after.clone(),
        after_keys: Vec::new(),
    };
    let before = FileSnapshot {
        role: edition.role.clone(),
        path: path.to_path_buf(),
        existed: edition.existed_before,
        text: edition.before.clone(),
    };
    // Fields/Meta 只有受管值，没有完整的 after 文本；None 不能被当成删除快照。
    rollback_file(&plan, &before, matches!(after, ManagedValues::Whole { .. }))
}

/// 空 meta：没有条目、没有 `appliedId`，也没有未知键。
fn meta_is_empty(document: &Value) -> bool {
    let Ok(meta) = MetaDocument::parse(Some(&document.to_string())) else {
        return false;
    };
    let entries_empty = meta
        .entry_list()
        .map(|list| list.is_empty())
        .unwrap_or(false);
    entries_empty && meta.applied_id.is_none() && meta.extra.is_empty()
}

fn managed_keys(edition: &Edition) -> Vec<String> {
    match (&edition.baseline, &edition.installed) {
        (ManagedValues::Fields { values }, ManagedValues::Fields { values: installed }) => {
            let mut keys: Vec<String> = values.keys().chain(installed.keys()).cloned().collect();
            keys.sort();
            keys.dedup();
            keys
        }
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 人工备份
// ---------------------------------------------------------------------------

pub fn backups_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join(super::constants::DATA_DIR_NAME)
        .join("backups")
}

/// 写一份受保护的恢复备份；失败则不继续修改外部文件。
pub fn write_backup(app_data_dir: &Path, snapshots: &[FileSnapshot]) -> DesktopResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = backups_dir(app_data_dir).join(format!("{id}.dat"));
    let bytes = serde_json::to_vec(snapshots).map_err(|_| storage_error())?;
    secret_file::write(&path, &bytes).map_err(|_| storage_error())?;
    storage::prune_backups(&backups_dir(app_data_dir), MAX_BACKUPS, |path| {
        path.extension().is_some_and(|extension| extension == "dat")
    });
    Ok(id)
}

pub fn backup_count(app_data_dir: &Path) -> usize {
    std::fs::read_dir(backups_dir(app_data_dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "dat")
                })
                .count()
        })
        .unwrap_or(0)
}

/// 读取四文件当前快照。
pub fn snapshot(paths: &DesktopPaths) -> DesktopResult<Vec<FileSnapshot>> {
    [
        (role::NORMAL_CONFIG, &paths.normal_config),
        (role::THIRD_PARTY_CONFIG, &paths.third_party_config),
        (role::PROFILE, &paths.profile_path),
        (role::META, &paths.meta_path),
    ]
    .into_iter()
    .map(|(file_role, path)| {
        let text = read_text_or(path)?;
        Ok(FileSnapshot {
            role: file_role.to_owned(),
            path: path.clone(),
            existed: text.is_some(),
            text,
        })
    })
    .collect()
}

/// 应用单个文件计划。`None` 的 `after_text` 表示删除。
pub fn apply_plan(plan: &FilePlan) -> DesktopResult<()> {
    match &plan.after_text {
        Some(text) => {
            let current = read_text_or(&plan.path)?;
            // 目标态已经达成时不再写：恢复可重复执行，也不会产生多余写盘。
            if current.as_deref() == Some(text.as_str()) {
                return Ok(());
            }
            write_text(&plan.path, text)
        }
        None => {
            if read_text_or(&plan.path)?.is_none() {
                return Ok(());
            }
            remove_file(&plan.path)
        }
    }
}

/// 依据 before 快照逆向恢复已经改动的文件。
pub fn rollback(pending: &Pending) -> DesktopResult<Vec<String>> {
    let mut failed = Vec::new();
    // 与写入相反的顺序，先撤回启用指针，再撤回 profile。
    for plan in pending.files.iter().rev() {
        let snapshot = pending.before.iter().find(|item| item.role == plan.role);
        if snapshot
            .ok_or_else(conflict_error)
            .and_then(|before| rollback_file(plan, before, true))
            .is_err()
        {
            failed.push(plan.role.clone());
        }
    }
    if failed.is_empty() {
        Ok(Vec::new())
    } else {
        Err(Box::new(
            DesktopError::new(
                error_code::RECOVERY_REQUIRED,
                "claudeDesktop.errors.recoveryRequired",
            )
            .param(failed.join(",")),
        ))
    }
}

fn text_equivalent(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => match (
            serde_json::from_str::<Value>(left),
            serde_json::from_str::<Value>(right),
        ) {
            (Ok(left), Ok(right)) => left == right,
            _ => left == right,
        },
        _ => false,
    }
}

/// 已提交的恢复只核对本工具负责的值，外部新增 MCP/偏好/meta 条目不阻止清理。
pub fn after_matches(plan: &FilePlan) -> DesktopResult<bool> {
    let current = read_text_or(&plan.path)?;
    managed_after_matches(plan, current.as_deref())
}

fn managed_after_matches(plan: &FilePlan, current: Option<&str>) -> DesktopResult<bool> {
    Ok(match &plan.after {
        ManagedValues::Whole { value } => {
            let now = current
                .map(serde_json::from_str::<Value>)
                .transpose()
                .map_err(|_| conflict_error())?;
            &now == value
        }
        ManagedValues::Fields { values } => {
            let now = profile::parse_object(current, &plan.role)?;
            now.get(profile::KEY_DEPLOYMENT_MODE) == values.get(profile::KEY_DEPLOYMENT_MODE)
        }
        ManagedValues::Meta {
            own_entry,
            applied_id,
        } => {
            let now = MetaDocument::parse(current)?;
            let entry = now
                .entry_list()?
                .iter()
                .find(|entry| entry.is_own())
                .map(MetaEntry::to_value);
            &entry == own_entry && &now.applied_id == applied_id
        }
    })
}

/// 回滚只回到本笔事务开始时的状态，绝不使用首次接管基线。
/// 共享文件允许保留外部无关键，但受管值若是 before/after 之外的第三值必须报冲突。
fn rollback_file(
    plan: &FilePlan,
    before: &FileSnapshot,
    full_after_snapshot: bool,
) -> DesktopResult<()> {
    if before.path != plan.path || before.existed != before.text.is_some() {
        return Err(conflict_error());
    }
    let current = read_text_or(&plan.path)?;
    if text_equivalent(current.as_deref(), before.text.as_deref()) {
        return Ok(());
    }
    let target = if full_after_snapshot
        && text_equivalent(current.as_deref(), plan.after_text.as_deref())
        && managed_after_matches(plan, current.as_deref())?
    {
        before.text.clone()
    } else {
        match &plan.after {
            ManagedValues::Whole { .. } => return Err(conflict_error()),
            ManagedValues::Fields { values: after } => {
                let baseline = profile::parse_object(before.text.as_deref(), &plan.role)?;
                let mut now = profile::parse_object(current.as_deref(), &plan.role)?;
                if now.get(profile::KEY_DEPLOYMENT_MODE)
                    == baseline.get(profile::KEY_DEPLOYMENT_MODE)
                {
                    return Ok(());
                }
                let keys = [profile::KEY_DEPLOYMENT_MODE];
                for key in keys {
                    if now.get(key) != baseline.get(key) && now.get(key) != after.get(key) {
                        return Err(conflict_error());
                    }
                    match baseline.get(key) {
                        Some(value) => {
                            now.insert(key.to_owned(), value.clone());
                        }
                        None => {
                            now.remove(key);
                        }
                    }
                }
                if !before.existed && now.is_empty() {
                    None
                } else {
                    Some(profile::to_text(&Value::Object(now), &plan.role)?)
                }
            }
            ManagedValues::Meta {
                own_entry: after_entry,
                applied_id: after_id,
            } => {
                let baseline = MetaDocument::parse(before.text.as_deref())?;
                let now = MetaDocument::parse(current.as_deref())?;
                let entries = baseline.entry_list()?;
                let old_entry = entries.iter().find(|entry| entry.is_own());
                let old_value = old_entry.map(MetaEntry::to_value);
                let current_entry = now
                    .entry_list()?
                    .iter()
                    .find(|entry| entry.is_own())
                    .map(MetaEntry::to_value);
                if current_entry == old_value && now.applied_id == baseline.applied_id {
                    return Ok(());
                }
                if (current_entry != old_value && current_entry != *after_entry)
                    || (now.applied_id != baseline.applied_id && now.applied_id != *after_id)
                {
                    return Err(conflict_error());
                }
                let index = entries.iter().position(MetaEntry::is_own).unwrap_or(0);
                let mut merged = now.render(old_entry, Some(None), index)?;
                let object = merged.as_object_mut().ok_or_else(conflict_error)?;
                if let Some(value) = &baseline.applied_id {
                    object.insert(profile::KEY_APPLIED_ID.to_owned(), value.clone());
                }
                // render 会生成 entries=[]；接管前没有这个键时撤回空容器。
                if baseline.entries.is_none()
                    && object
                        .get(profile::KEY_ENTRIES)
                        .and_then(Value::as_array)
                        .is_some_and(|entries| entries.is_empty())
                {
                    object.remove(profile::KEY_ENTRIES);
                }
                if !before.existed && object.is_empty() {
                    None
                } else {
                    Some(profile::to_text(&merged, &plan.role)?)
                }
            }
        }
    };
    // 不在读取与回写之间静默覆盖新修改。
    if read_text_or(&plan.path)? != current {
        return Err(conflict_error());
    }
    match target {
        Some(text) => write_text(&plan.path, &text),
        None => remove_file(&plan.path),
    }
}

/// 从基线的 before 原文恢复全部四个文件（用于 restore / switchOfficial）。
pub fn restore_all(state: &State, paths: &DesktopPaths) -> DesktopResult<Vec<String>> {
    let mut failed = Vec::new();
    for edition in &state.data.files {
        let Some(path) = edition_path(edition, paths) else {
            continue;
        };
        let after = match &edition.installed {
            ManagedValues::Whole { value } => ManagedValues::Whole {
                value: value.clone(),
            },
            other => other.clone(),
        };
        if restore_edition(edition, &path, &after).is_err() {
            failed.push(edition.role.clone());
        }
    }
    if failed.is_empty() {
        Ok(Vec::new())
    } else {
        Err(Box::new(
            DesktopError::new(
                error_code::RECOVERY_REQUIRED,
                "claudeDesktop.errors.recoveryRequired",
            )
            .param(failed.join(",")),
        ))
    }
}

#[cfg(test)]
mod preview_projection_tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, DesktopPaths) {
        let root = tempfile::tempdir().unwrap();
        let paths =
            DesktopPaths::from_roots(root.path().join("Claude"), root.path().join("Claude-3p"))
                .unwrap();
        std::fs::create_dir_all(paths.profile_path.parent().unwrap()).unwrap();
        (root, paths)
    }

    fn desired(model_id: &str) -> DesiredState {
        DesiredState {
            mode: Some(profile::third_party_mode()),
            profile: Some(profile::profile_json(
                "https://relay.example",
                "new-secret-sentinel",
                &[ManagedModelView {
                    model_id: model_id.into(),
                    label: model_id.into(),
                    one_m: false,
                }],
            )),
            own_entry: Some(MetaEntry::own()),
            applied_id: Some(super::super::constants::PROFILE_ID.into()),
            entry_insert_at: 0,
        }
    }

    #[test]
    fn creating_a_profile_previews_the_selected_opus_model_and_endpoint() {
        let (_root, paths) = fixture();
        let plans = build_plan(
            &paths,
            &desired("claude-opus-4-7"),
            &TakeoverBaseline::default(),
        )
        .unwrap();
        let view = plans
            .iter()
            .find(|plan| plan.role == role::PROFILE)
            .unwrap()
            .view(false);
        assert_eq!(view.kind, "create");
        let models = view
            .managed_changes
            .iter()
            .find(|change| change.path == profile::KEY_MODELS)
            .unwrap();
        assert!(models.before.is_none());
        assert_eq!(models.after.as_deref(), Some("[\"claude-opus-4-7\"]"));
        let endpoint = view
            .managed_changes
            .iter()
            .find(|change| change.path == profile::KEY_BASE_URL)
            .unwrap();
        assert_eq!(endpoint.after.as_deref(), Some("https://relay.example"));
        let egress = view
            .managed_changes
            .iter()
            .find(|change| change.path == profile::KEY_EGRESS_HOSTS)
            .unwrap();
        assert!(egress.before.is_none());
        assert_eq!(egress.after.as_deref(), Some("[\"*\"]"));
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("new-secret-sentinel"));
        assert!(encoded.contains("<redacted>"));
        assert!(
            !paths.profile_path.exists(),
            "preview must remain read-only"
        );
    }

    #[test]
    fn updating_a_profile_previews_both_models_without_unknown_extensions_or_keys() {
        let (_root, paths) = fixture();
        let old = json!({
            profile::KEY_MODELS: [{ "name": "claude-opus-4-6", "supports1m": true, "privateToken": "extension-secret-sentinel" }],
            profile::KEY_BASE_URL: "https://old-relay.example",
            profile::KEY_API_KEY: "old-secret-sentinel",
            "unknownPrivateField": "root-secret-sentinel",
        });
        std::fs::write(&paths.profile_path, old.to_string()).unwrap();
        let plans = build_plan(
            &paths,
            &desired("claude-opus-4-7"),
            &TakeoverBaseline::default(),
        )
        .unwrap();
        let view = plans
            .iter()
            .find(|plan| plan.role == role::PROFILE)
            .unwrap()
            .view(true);
        let models = view
            .managed_changes
            .iter()
            .find(|change| change.path == profile::KEY_MODELS)
            .unwrap();
        assert!(models.before.as_ref().unwrap().contains("claude-opus-4-6"));
        assert_eq!(models.after.as_deref(), Some("[\"claude-opus-4-7\"]"));
        let encoded = serde_json::to_string(&view).unwrap();
        for secret in [
            "extension-secret-sentinel",
            "old-secret-sentinel",
            "root-secret-sentinel",
            "new-secret-sentinel",
        ] {
            assert!(!encoded.contains(secret));
        }
        assert_eq!(
            std::fs::read_to_string(&paths.profile_path).unwrap(),
            old.to_string()
        );
    }

    #[test]
    fn restoring_a_profile_projects_its_actual_target_and_deletion_has_no_target() {
        let (_root, paths) = fixture();
        let current = desired("claude-opus-4-7").profile.unwrap();
        std::fs::write(&paths.profile_path, current.to_string()).unwrap();
        let old = desired("claude-opus-4-6").profile.unwrap();
        let restore = DesiredState {
            mode: None,
            profile: None,
            own_entry: None,
            applied_id: None,
            entry_insert_at: 0,
        };
        let baseline = TakeoverBaseline {
            profile: ProfileBaseline::PreExisting(old),
            ..Default::default()
        };
        let plans = build_plan(&paths, &restore, &baseline).unwrap();
        let plan = plans
            .iter()
            .find(|plan| plan.role == role::PROFILE)
            .unwrap();
        let models = plan
            .after_keys
            .iter()
            .find(|change| change.path == profile::KEY_MODELS)
            .unwrap();
        assert_eq!(models.before.as_deref(), Some("[\"claude-opus-4-7\"]"));
        assert_eq!(models.after.as_deref(), Some("[\"claude-opus-4-6\"]"));
        let plans = build_plan(&paths, &restore, &TakeoverBaseline::default()).unwrap();
        let plan = plans
            .iter()
            .find(|plan| plan.role == role::PROFILE)
            .unwrap();
        assert_eq!(plan.kind(true), "delete");
        assert!(plan.after_keys.iter().all(|change| change.after.is_none()));
    }
}
