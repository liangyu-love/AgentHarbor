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
    error_code, DesktopError, DesktopResult, FileChangeView, ManagedChangeView, ManagedModelView,
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
    pub last_applied_at: i64,
    pub restart_required: bool,
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
        // 管理已结束：不再保留凭据指纹。
        self.credential_fingerprint = None;
        self.restart_required = false;
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
    Box::new(
        DesktopError::new(error_code::STORAGE, "claudeDesktop.errors.storage").retryable(),
    )
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
        Some(text) if !text.trim().is_empty() => Some(serde_json::from_str::<Value>(text).map_err(|_| {
            Box::new(
                DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.invalidJson")
                    .file(role::PROFILE),
            )
        })?),
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
        config_edition(read_text_or(&paths.third_party_config)?.as_deref(), role::THIRD_PARTY_CONFIG)?,
        config_edition(read_text_or(&paths.normal_config)?.as_deref(), role::NORMAL_CONFIG)?,
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
        .map(|edition| match previous.and_then(|state| state.edition(&edition.role)) {
            Some(kept) => kept.clone(),
            None => edition.clone(),
        })
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

fn profile_plan_keys(before: Option<&str>) -> Vec<KeyValue> {
    let previous = |key: &str| -> Option<String> {
        if key == profile::KEY_API_KEY {
            return before.map(|_| "<redacted>".to_owned());
        }
        let value: Value = serde_json::from_str(before?).ok()?;
        Some(render(value.get(key)?))
    };
    let mut keys = vec![KeyValue {
        path: profile::KEY_BASE_URL.to_owned(),
        before: previous(profile::KEY_BASE_URL),
        after: None,
    }];
    for key in [
        profile::KEY_DISABLE_CHOOSER,
        profile::KEY_INFERENCE_PROVIDER,
        profile::KEY_AUTH_SCHEME,
        profile::KEY_MODELS,
    ] {
        keys.push(KeyValue {
            path: key.to_owned(),
            before: previous(key),
            after: None,
        });
    }
    // 密钥前后值都固定替换为 <redacted>，不回传任何可比较的摘要。
    keys.push(KeyValue {
        path: profile::KEY_API_KEY.to_owned(),
        before: before.map(|_| "<redacted>".to_owned()),
        after: Some("<redacted>".to_owned()),
    });
    keys
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
                    if let Some(mode) = values
                        .get(profile::KEY_DEPLOYMENT_MODE)
                        .and_then(Value::as_str)
                    {
                        baseline.modes.insert(edition.role.clone(), mode.to_owned());
                    }
                }
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
        Some(value) => plans.push(FilePlan {
            role: role::PROFILE.to_owned(),
            path: paths.profile_path.clone(),
            after_text: Some(profile::to_text(value, role::PROFILE)?),
            after: ManagedValues::Whole {
                value: Some(value.clone()),
            },
            after_keys: profile_plan_keys(profile_before.as_deref()),
        }),
        None => {
            match &baseline.profile {
                // 接管前就有内容：写回原始字节，不能凭同一个 id 删除。
                ProfileBaseline::PreExisting(baseline) => {
                    let text = profile::to_text(baseline, role::PROFILE)?;
                    if profile_before.as_deref() != Some(text.as_str()) {
                        plans.push(FilePlan {
                            role: role::PROFILE.to_owned(),
                            path: paths.profile_path.clone(),
                            after_text: Some(text),
                            after: ManagedValues::Whole {
                                value: Some(baseline.clone()),
                            },
                            after_keys: vec![KeyValue {
                                path: paths::display(&paths.profile_path),
                                before: Some("<redacted>".to_owned()),
                                after: Some("<redacted>".to_owned()),
                            }],
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
                    after_keys: vec![KeyValue {
                        path: paths::display(&paths.profile_path),
                        before: Some("<redacted>".to_owned()),
                        after: None,
                    }],
                });
            }
        }
    }

    let meta_document = MetaDocument::parse(meta_before.as_deref())?;
    let meta_next = meta_document.render(
        desired.own_entry.as_ref(),
        Some(desired.applied_id.as_deref()),
        desired.entry_insert_at,
    )?;
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
                before: Some(applied_label(before_applied.as_deref(), &desired.applied_id)),
                after: Some(applied_label(desired.applied_id.as_deref(), &before_applied)),
            });
        }
        plans.push(FilePlan {
            role: role::META.to_owned(),
            path: paths.meta_path.clone(),
            after_text: Some(profile::to_text(&meta_next, role::META)?),
            after: ManagedValues::Meta {
                own_entry: desired.own_entry.as_ref().map(MetaEntry::to_value),
                applied_id: desired
                    .applied_id
                    .as_ref()
                    .map(|id| Value::String(id.clone())),
            },
            after_keys: keys,
        });
    }

    for (file_role, path, before) in [
        (role::THIRD_PARTY_CONFIG, &paths.third_party_config, &threep_before),
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
        plans.push(FilePlan {
            role: file_role.to_owned(),
            path: path.to_path_buf(),
            after_text: Some(profile::to_text(&Value::Object(map), file_role)?),
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
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        credential_fingerprint: credential_fingerprint.clone(),
        models: models.to_vec(),
        last_applied_at: chrono::Utc::now().timestamp(),
        restart_required: true,
        data: ManagedData { files, applied_mode },
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

// ---------------------------------------------------------------------------
// 漂移检测
// ---------------------------------------------------------------------------

/// 检查最近安装值是否仍等于当前受管值。返回发生漂移的文件角色。
pub fn drifted_roles(state: &State, paths: &DesktopPaths) -> DesktopResult<Vec<String>> {
    let mut out = Vec::new();
    for edition in &state.data.files {
        let Some(path) = edition_path(edition, paths) else {
            continue;
        };
        let text = read_text_or(&path)?;
        let current = match &text {
            None => null_for(edition),
            Some(text) => parse_for(edition, text)?,
        };
        let now = read_managed(&current, edition);
        if edition.installed.drifted(&now) {
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
    let current = read_text_or(path)?;
    match &edition.baseline {
        ManagedValues::Whole { value: baseline_value } => {
            let Some(text) = &current else {
                // 文件已不存在：目标态已达成。
                return Ok(());
            };
            let now: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            // 已经被外部改写时不能整体还原，只报告冲突。
            let installed = value_of(after);
            let is_installed = !installed.is_null() && now == installed;
            let is_baseline = baseline_value.as_ref().is_some_and(|value| now == *value);
            if !is_installed && !is_baseline {
                return Err(conflict_error());
            }
            match baseline_value {
                // 文件在本工具接管前存在：写回原字节。
                Some(_) => match &edition.before {
                    Some(text) => write_text(path, text),
                    None => Ok(()),
                },
                // 接管前不存在：只有内容确实仍是我们写入的 profile 才删除。
                None => {
                    if is_installed {
                        remove_file(path)
                    } else {
                        Ok(())
                    }
                }
            }
        }
        ManagedValues::Fields { values } => {
            let Some(text) = current else {
                return Ok(());
            };
            let mut map = profile::parse_object(Some(&text), &edition.role)?;
            // 只在当前受管值仍等于安装值时才撤；否则说明外部改过。
            for key in managed_keys(edition) {
                let installed = installed_field(edition, &key);
                let now = map.get(&key).cloned();
                if installed.is_some() && now != installed {
                    return Err(conflict_error());
                }
            }
            for key in managed_keys(edition) {
                match values.get(&key) {
                    Some(value) => {
                        map.insert(key.clone(), value.clone());
                    }
                    None => {
                        map.remove(&key);
                    }
                }
            }
            if !edition.existed_before && map.is_empty() {
                return remove_file(path);
            }
            write_document(path, &Value::Object(map), &edition.role)
        }
        ManagedValues::Meta { applied_id, .. } => {
            let Some(text) = current else {
                return Ok(());
            };
            let meta = MetaDocument::parse(Some(&text))?;
            // 自身条目仍是我们写入的才撤；已被外部改写则冲突。
            let current_own = meta
                .entry_list()?
                .into_iter()
                .find(MetaEntry::is_own)
                .map(|entry| entry.to_value());
            let installed_own = installed_own_entry(edition);
            if current_own.is_some() && current_own != installed_own {
                return Err(conflict_error());
            }
            if let Some(pointer) = meta.applied_id_str() {
                if pointer != super::constants::PROFILE_ID {
                    return Err(Box::new(
                        DesktopError::new(
                            error_code::CONFLICT,
                            "claudeDesktop.errors.pointerMovedExternally",
                        )
                        .file(role::META)
                        .action("rebaseline"),
                    ));
                }
            }
            let next = meta.render(None, Some(applied_id.as_ref().and_then(Value::as_str)), 0)?;
            // 接管前该文件不存在、且清空自身内容后确实没有别的键时才删除；
            // 仍有外部条目或未知键时保留文件。
            if !edition.existed_before && meta_is_empty(&next) {
                return remove_file(path);
            }
            write_document(path, &next, &edition.role)
        }
    }
}

/// 空 meta：没有条目、没有 `appliedId`，也没有未知键。
fn meta_is_empty(document: &Value) -> bool {
    let Ok(meta) = MetaDocument::parse(Some(&document.to_string())) else {
        return false;
    };
    let entries_empty = meta.entry_list().map(|list| list.is_empty()).unwrap_or(false);
    entries_empty && meta.applied_id.is_none() && meta.extra.is_empty()
}

fn value_of(values: &ManagedValues) -> Value {
    match values {
        ManagedValues::Whole { value } => value.clone().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn installed_field(edition: &Edition, key: &str) -> Option<Value> {
    match &edition.installed {
        ManagedValues::Fields { values } => values.get(key).cloned(),
        _ => None,
    }
}

fn installed_own_entry(edition: &Edition) -> Option<Value> {
    match &edition.installed {
        ManagedValues::Meta { own_entry, .. } => own_entry.clone(),
        _ => None,
    }
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
    app_data_dir.join(super::constants::DATA_DIR_NAME).join("backups")
}

/// 写一份受保护的恢复备份；失败则不继续修改外部文件。
pub fn write_backup(app_data_dir: &Path, snapshots: &[FileSnapshot]) -> DesktopResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = backups_dir(app_data_dir).join(format!("{id}.dat"));
    let bytes = serde_json::to_vec(snapshots).map_err(|_| storage_error())?;
    secret_file::write(&path, &bytes).map_err(|_| storage_error())?;
    storage::prune_backups(&backups_dir(app_data_dir), MAX_BACKUPS, |path| {
        path.extension()
            .is_some_and(|extension| extension == "dat")
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
pub fn snapshot(paths: &DesktopPaths) -> Vec<FileSnapshot> {
    [
        (role::NORMAL_CONFIG, &paths.normal_config),
        (role::THIRD_PARTY_CONFIG, &paths.third_party_config),
        (role::PROFILE, &paths.profile_path),
        (role::META, &paths.meta_path),
    ]
    .into_iter()
    .map(|(file_role, path)| FileSnapshot {
        role: file_role.to_owned(),
        path: path.clone(),
        existed: path.exists(),
        text: read_text(path).ok().flatten(),
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
    for plan in &pending.files {
        let Some(edition) = pending
            .previous_state
            .as_ref()
            .and_then(|state| state.edition(&plan.role))
        else {
            // 没有基线记录（首次接管前状态缺失）时用 pending.before 原文兜底。
            let snapshot = pending.before.iter().find(|item| item.role == plan.role);
            let result = match snapshot {
                Some(snapshot) => match &snapshot.text {
                    Some(text) => write_text(&plan.path, text),
                    None => remove_file(&plan.path),
                },
                None => Ok(()),
            };
            if result.is_err() {
                failed.push(plan.role.clone());
            }
            continue;
        };
        if restore_edition(edition, &plan.path, &plan.after).is_err() {
            failed.push(plan.role.clone());
        }
    }
    if failed.is_empty() {
        Ok(Vec::new())
    } else {
        Err(Box::new(
            DesktopError::new(error_code::RECOVERY_REQUIRED, "claudeDesktop.errors.recoveryRequired")
                .param(failed.join(",")),
        ))
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
            DesktopError::new(error_code::RECOVERY_REQUIRED, "claudeDesktop.errors.recoveryRequired")
                .param(failed.join(",")),
        ))
    }
}
