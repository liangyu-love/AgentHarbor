//! Claude Desktop 直连：profile 与共享文件的纯函数构造。
//!
//! 所有函数都只做 JSON 值变换，不落盘、不读盘，便于单测覆盖每一种外部文件
//! 形态（缺失、正常对象、含 MCP、含外部 profile、结构非法）。

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::constants::{
    DEPLOYMENT_MODE_OFFICIAL, DEPLOYMENT_MODE_THIRD_PARTY, INFERENCE_AUTH_SCHEME_BEARER,
    INFERENCE_PROVIDER_GATEWAY, PROFILE_DISPLAY_NAME, PROFILE_ID,
};
use super::types::{error_code, DesktopError, DesktopResult, ManagedModelView};

pub const KEY_DEPLOYMENT_MODE: &str = "deploymentMode";
pub const KEY_DISABLE_CHOOSER: &str = "disableDeploymentModeChooser";
pub const KEY_INFERENCE_PROVIDER: &str = "inferenceProvider";
pub const KEY_AUTH_SCHEME: &str = "inferenceGatewayAuthScheme";
pub const KEY_BASE_URL: &str = "inferenceGatewayBaseUrl";
pub const KEY_API_KEY: &str = "inferenceGatewayApiKey";
pub const KEY_MODELS: &str = "inferenceModels";
pub const KEY_EGRESS_HOSTS: &str = "coworkEgressAllowedHosts";
pub const KEY_ENTRIES: &str = "entries";
pub const KEY_APPLIED_ID: &str = "appliedId";

fn storage_error(file_role: &str) -> Box<DesktopError> {
    Box::new(
        DesktopError::new(error_code::STORAGE, "claudeDesktop.errors.storage")
            .file(file_role)
            .retryable(),
    )
}

/// 解析一个已存在的共享 JSON 文件。
///
/// 语法错误、非对象根都不能被当成 `{}` 丢弃：那会静默覆盖用户或其它工具的
/// 未知内容。调用方必须在返回错误时零写入。
pub fn parse_object(text: Option<&str>, file_role: &str) -> DesktopResult<Map<String, Value>> {
    let Some(text) = text else {
        return Ok(Map::new());
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            return Err(Box::new(
                DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.invalidJson")
                    .file(file_role),
            ))
        }
    };
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(Box::new(
            DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.notAnObject")
                .file(file_role),
        )),
    }
}

/// 读取 `deploymentMode`。缺失或非字符串都按“未知”处理，不猜测。
pub fn deployment_mode(map: &Map<String, Value>) -> Option<&str> {
    map.get(KEY_DEPLOYMENT_MODE).and_then(Value::as_str)
}

/// 只改 `deploymentMode`，保留同文件里的 MCP、偏好等所有无关键。
pub fn patch_deployment_mode(map: &mut Map<String, Value>, mode: &str) {
    map.insert(
        KEY_DEPLOYMENT_MODE.to_owned(),
        Value::String(mode.to_owned()),
    );
}

/// `_meta.json` 中的一个条目。未知字段必须原样保留。
#[derive(Debug, Clone, Deserialize)]
pub struct MetaEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MetaEntry {
    pub fn own() -> Self {
        Self {
            id: PROFILE_ID.to_owned(),
            name: Some(PROFILE_DISPLAY_NAME.to_owned()),
            extra: Map::new(),
        }
    }

    pub fn is_own(&self) -> bool {
        self.id == PROFILE_ID
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), Value::String(self.id.clone()));
        if let Some(name) = &self.name {
            map.insert("name".into(), Value::String(name.clone()));
        }
        for (key, value) in &self.extra {
            map.insert(key.clone(), value.clone());
        }
        Value::Object(map)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetaDocument {
    /// `entries` 的原始值；缺失或非法时为 `None`。
    pub entries: Option<Value>,
    pub applied_id: Option<Value>,
    /// 无法识别但必须保留的其它键。
    pub extra: Map<String, Value>,
}

impl MetaDocument {
    /// 解析 `_meta.json`。`entries` 存在但不是数组时返回错误而不是当作空数组。
    pub fn parse(text: Option<&str>) -> DesktopResult<Self> {
        let map = parse_object(text, super::paths::role::META)?;
        let mut doc = Self::default();
        for (key, value) in map {
            match key.as_str() {
                KEY_ENTRIES => {
                    if !value.is_array() {
                        return Err(Box::new(
                            DesktopError::new(
                                error_code::INVALID_FILE,
                                "claudeDesktop.errors.invalidMetaEntries",
                            )
                            .file(super::paths::role::META),
                        ));
                    }
                    doc.entries = Some(value);
                }
                KEY_APPLIED_ID => doc.applied_id = Some(value),
                _ => {
                    doc.extra.insert(key, value);
                }
            }
        }
        Ok(doc)
    }

    /// 条目列表；单个元素结构非法时整份文件按非法处理，避免按位置误删。
    pub fn entry_list(&self) -> DesktopResult<Vec<MetaEntry>> {
        let Some(value) = &self.entries else {
            return Ok(Vec::new());
        };
        let items = value.as_array().ok_or_else(|| {
            Box::new(
                DesktopError::new(error_code::INVALID_FILE, "claudeDesktop.errors.invalidMetaEntries")
                    .file(super::paths::role::META),
            )
        })?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let entry: MetaEntry = serde_json::from_value(item.clone()).map_err(|_| {
                Box::new(
                    DesktopError::new(
                        error_code::INVALID_FILE,
                        "claudeDesktop.errors.invalidMetaEntry",
                    )
                    .file(super::paths::role::META),
                )
            })?;
            out.push(entry);
        }
        Ok(out)
    }

    /// 当前 `appliedId` 的字符串形式；非字符串返回 `None` 视为无效。
    pub fn applied_id_str(&self) -> Option<&str> {
        self.applied_id.as_ref().and_then(Value::as_str)
    }

    /// 非自身条目数量：预览只报告数量，不回传外部 profile 原文。
    pub fn external_entry_count(&self) -> DesktopResult<usize> {
        Ok(self
            .entry_list()?
            .iter()
            .filter(|entry| !entry.is_own())
            .count())
    }

    pub fn own_entry_present(&self) -> DesktopResult<bool> {
        Ok(self.entry_list()?.iter().any(MetaEntry::is_own))
    }

    /// 生成写入值：移除自身条目，再按 `insert_at` 位置插入或更新一个自身条目。
    /// 外部条目顺序与未知字段保持不变。
    pub fn render(
        &self,
        own: Option<&MetaEntry>,
        applied_id: Option<Option<&str>>,
        insert_at: usize,
    ) -> DesktopResult<Value> {
        let mut entries = self.entry_list()?;
        entries.retain(|entry| !entry.is_own());
        if let Some(own) = own {
            let index = insert_at.min(entries.len());
            entries.insert(index, own.clone());
        }
        let mut map = self.extra.clone();
        map.insert(
            KEY_ENTRIES.into(),
            Value::Array(entries.iter().map(MetaEntry::to_value).collect()),
        );
        match applied_id {
            Some(Some(id)) => {
                map.insert(KEY_APPLIED_ID.into(), Value::String(id.to_owned()));
            }
            Some(None) => {
                map.remove(KEY_APPLIED_ID);
            }
            // 不变：保留原值（可能是非字符串的无效类型，保存证据而不是清除）。
            None => {
                if let Some(value) = &self.applied_id {
                    map.insert(KEY_APPLIED_ID.into(), value.clone());
                }
            }
        }
        Ok(Value::Object(map))
    }
}

/// 构造本产品的完整 profile。
///
/// `models` 非空由调用方保证：Desktop 的 fail-all 校验器会因一条非法模型
/// 拒收整份 profile，因此这里不再做兜底补全。
/// 这份 profile 是否由本工具写入（而不是用户或其它工具原有的）。
///
/// 判据是「本工具写的那组键同时存在」，而不是文件名：文件名只说明它占用了我们
/// 的固定 UUID。用于在**丢失管理记录**时避免把自己的旧文件当成用户原有配置。
pub fn looks_like_ours(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    let is_gateway = map
        .get(KEY_INFERENCE_PROVIDER)
        .and_then(Value::as_str)
        == Some(INFERENCE_PROVIDER_GATEWAY);
    let is_bearer = map
        .get(KEY_AUTH_SCHEME)
        .and_then(Value::as_str)
        == Some(INFERENCE_AUTH_SCHEME_BEARER);
    is_gateway && is_bearer && map.contains_key(KEY_BASE_URL) && map.contains_key(KEY_API_KEY)
}

pub fn profile_json(base_url: &str, api_key: &str, models: &[ManagedModelView]) -> Value {
    let mut map = Map::new();
    // Unset confines tool traffic to the inference host (127.0.0.1 in router
    // mode). Allow public web access, as CC Switch does; this does not add a
    // provider-side WebSearch implementation or override managed policies.
    map.insert(KEY_EGRESS_HOSTS.into(), json!(["*"]));
    map.insert(KEY_DISABLE_CHOOSER.into(), Value::Bool(true));
    map.insert(
        KEY_INFERENCE_PROVIDER.into(),
        Value::String(INFERENCE_PROVIDER_GATEWAY.into()),
    );
    map.insert(
        KEY_AUTH_SCHEME.into(),
        Value::String(INFERENCE_AUTH_SCHEME_BEARER.into()),
    );
    map.insert(KEY_BASE_URL.into(), Value::String(base_url.to_owned()));
    map.insert(KEY_API_KEY.into(), Value::String(api_key.to_owned()));
    map.insert(
        KEY_MODELS.into(),
        Value::Array(models.iter().map(inference_model_json).collect()),
    );
    Value::Object(map)
}

/// 没有 label 与 1M 选项时使用字符串；有选项时使用对象，`supports1m=false`
/// 沿用参考输出形态并省略。
/// 一条带 supports1m=true 的记录会由 Desktop 展开为普通和 1M 两个菜单项。
fn inference_model_json(model: &ManagedModelView) -> Value {
    let label = model.label.trim();
    let has_label = !label.is_empty() && label != model.model_id.trim();
    if !model.one_m && !has_label {
        return Value::String(model.model_id.clone());
    }
    let mut item = Map::new();
    item.insert("name".into(), Value::String(model.model_id.clone()));
    if has_label {
        item.insert("labelOverride".into(), Value::String(model.label.clone()));
    }
    if model.one_m {
        item.insert("supports1m".into(), Value::Bool(true));
    }
    Value::Object(item)
}

/// 移除 profile 中本产品写入的推理键，用于切回官方模式时不再保留自身凭据。
pub fn strip_inference_keys(profile: &mut Map<String, Value>) {
    for key in [
        KEY_DISABLE_CHOOSER,
        KEY_INFERENCE_PROVIDER,
        KEY_AUTH_SCHEME,
        KEY_BASE_URL,
        KEY_API_KEY,
        KEY_MODELS,
        KEY_EGRESS_HOSTS,
    ] {
        profile.remove(key);
    }
}

pub fn official_mode() -> &'static str {
    DEPLOYMENT_MODE_OFFICIAL
}

pub fn third_party_mode() -> &'static str {
    DEPLOYMENT_MODE_THIRD_PARTY
}

/// 序列化为带缩进的 JSON 文本（与参考实现一致的文件形态）。
pub fn to_text(value: &Value, file_role: &str) -> DesktopResult<String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|_| storage_error(file_role))?;
    text.push('\n');
    Ok(text)
}

/// 空的 profile 占位值，仅用于测试与“已删除”判断。
pub fn empty_profile() -> Value {
    json!({})
}
