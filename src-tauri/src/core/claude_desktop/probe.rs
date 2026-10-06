//! Claude Desktop 直连：Desktop 实际地址上的只读 Bearer 检测。
//!
//! 通用探测按供应商契约选择鉴权，其成功不能替代 Desktop 的地址与 Bearer 检测。
//! 这里独立检测 Desktop 真正会访问的 `/v1/models`：
//! - 只读 GET，不自动发送任何计费 Messages 请求；
//! - 不修改 Provider 的 `test_status`；
//! - 沿用现有超时、响应大小上限与禁止自动重定向边界；
//! - 状态或结构化分类，不返回响应原文里的密钥或 HTML。

use std::io::Read;
use std::time::Duration;

use serde::Serialize;

use crate::core::models::ApiProxyConfigPayload;
use crate::core::{newapi, router::Provider};

use super::provider::resolve_direct_endpoints;
use super::types::{DesktopError, DesktopResult};

const MAX_BODY: usize = 2 * 1024 * 1024;
const DETECT_TIMEOUT: Duration = Duration::from_secs(20);

/// 检测状态。`notChecked` 是默认值，绝不因为“配好了文件”而变成可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BearerStatus {
    NotChecked,
    ModelsAccessible,
    AuthRejected,
    EndpointUnavailable,
    NetworkFailed,
    InvalidResponse,
    RateLimited,
}

impl BearerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotChecked => "notChecked",
            Self::ModelsAccessible => "modelsAccessible",
            Self::AuthRejected => "authRejected",
            Self::EndpointUnavailable => "endpointUnavailable",
            Self::NetworkFailed => "networkFailed",
            Self::InvalidResponse => "invalidResponse",
            Self::RateLimited => "rateLimited",
        }
    }

    /// 只有 2xx 且列表结构合法才算通过。
    pub fn passed(self) -> bool {
        self == Self::ModelsAccessible
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BearerReport {
    pub provider_id: String,
    /// Desktop 实际访问的 `/v1/models` 地址。
    pub url: String,
    pub status: BearerStatus,
    pub http_status: Option<u16>,
    /// 检测出口：项目自身网络设置与服务端出口不同。
    pub egress: String,
    /// 检测时选中的模型是否出现在列表中。
    pub models_visible: Vec<String>,
    pub models_missing: Vec<String>,
    /// 模型列表条数；`None` 表示响应没有可识别的列表。
    pub listed_models: Option<usize>,
    pub checked_at: i64,
    /// 绑定供应商连接与凭据的指纹；相关输入变化后检测结果失效。
    pub fingerprint: String,
    pub note: Option<String>,
}

/// 凭据/连接指纹：只用于让过期检测结果失效，不是可反推密钥的摘要。
pub fn fingerprint(provider: &Provider, model_ids: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(provider.id.as_bytes());
    hasher.update([0]);
    hasher.update(provider.base_url.as_bytes());
    hasher.update([0]);
    hasher.update(provider.api_key.as_bytes());
    hasher.update([0]);
    hasher.update(super::provider::protocol_label(provider.protocol_for_model(&provider.model)).as_bytes());
    for model in model_ids {
        hasher.update([0]);
        hasher.update(model.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn probe_error(status: BearerStatus, message: &str, url: &str) -> BearerReport {
    BearerReport {
        provider_id: String::new(),
        url: url.to_owned(),
        status,
        http_status: None,
        egress: String::new(),
        models_visible: Vec::new(),
        models_missing: Vec::new(),
        listed_models: None,
        checked_at: chrono::Utc::now().timestamp(),
        fingerprint: String::new(),
        note: Some(message.to_owned()),
    }
}

/// 执行只读检测。密钥只用于请求头，不进入返回值、错误或日志。
pub fn run(
    provider: &Provider,
    model_ids: &[String],
    proxy: &ApiProxyConfigPayload,
    egress: &str,
) -> DesktopResult<BearerReport> {
    let (_, _, models_url) = resolve_direct_endpoints(provider)?;
    let key = super::provider::require_inference_key(provider)?;
    let url = reqwest::Url::parse(&models_url).map_err(|_| {
        Box::new(DesktopError::new(
            super::types::error_code::INVALID_URL,
            "claudeDesktop.errors.invalidUrl",
        ))
    })?;
    let effective = newapi::effective_proxy(provider.network_mode(), proxy);
    let client = newapi::client_with_timeout(&effective, DETECT_TIMEOUT).map_err(|_| {
        Box::new(DesktopError::new(
            super::types::error_code::PROBE_FAILED,
            "claudeDesktop.errors.network",
        ))
    })?;
    let header = newapi::auth_header(&key).map_err(|_| {
        Box::new(DesktopError::new(
            super::types::error_code::INVALID_INFERENCE_KEY,
            "claudeDesktop.errors.invalidInferenceKey",
        ))
    })?;

    let response = match client.get(url).header("Authorization", header).send() {
        Ok(response) => response,
        Err(error) => {
            let _ = &error;
            return Ok(BearerReport {
                note: Some(if error.is_timeout() {
                    "claudeDesktop.probe.timeout".to_owned()
                } else {
                    "claudeDesktop.probe.network".to_owned()
                }),
                ..probe_error(BearerStatus::NetworkFailed, "", &models_url)
            });
        }
    };
    let http_status = response.status();
    let code = http_status.as_u16();
    let mut body = Vec::new();
    let _ = response.take(MAX_BODY as u64 + 1).read_to_end(&mut body);
    let base = BearerReport {
        provider_id: provider.id.clone(),
        url: models_url.clone(),
        status: BearerStatus::NotChecked,
        http_status: Some(code),
        egress: egress.to_owned(),
        models_visible: Vec::new(),
        models_missing: Vec::new(),
        listed_models: None,
        checked_at: chrono::Utc::now().timestamp(),
        fingerprint: fingerprint(provider, model_ids),
        note: None,
    };

    if matches!(code, 401 | 403) {
        return Ok(BearerReport {
            status: BearerStatus::AuthRejected,
            note: Some("claudeDesktop.probe.auth".to_owned()),
            ..base
        });
    }
    if code == 404 || code == 405 {
        return Ok(BearerReport {
            status: BearerStatus::EndpointUnavailable,
            note: Some("claudeDesktop.probe.notFound".to_owned()),
            ..base
        });
    }
    if code == 429 {
        return Ok(BearerReport {
            status: BearerStatus::RateLimited,
            note: Some("claudeDesktop.probe.rateLimit".to_owned()),
            ..base
        });
    }
    if !http_status.is_success() {
        return Ok(BearerReport {
            status: BearerStatus::InvalidResponse,
            note: Some("claudeDesktop.probe.status".to_owned()),
            ..base
        });
    }
    if body.len() > MAX_BODY {
        return Ok(BearerReport {
            status: BearerStatus::InvalidResponse,
            note: Some("claudeDesktop.probe.tooLarge".to_owned()),
            ..base
        });
    }
    // 200 + HTML / 错误格式不算通过。
    let Some(listed) = parse_model_ids(&body) else {
        return Ok(BearerReport {
            status: BearerStatus::InvalidResponse,
            note: Some("claudeDesktop.probe.shape".to_owned()),
            ..base
        });
    };
    let visible: Vec<String> = model_ids
        .iter()
        .filter(|id| listed.iter().any(|entry| entry == *id))
        .cloned()
        .collect();
    let missing: Vec<String> = model_ids
        .iter()
        .filter(|id| !listed.iter().any(|entry| entry == *id))
        .cloned()
        .collect();
    Ok(BearerReport {
        status: BearerStatus::ModelsAccessible,
        models_visible: visible,
        models_missing: missing,
        listed_models: Some(listed.len()),
        note: Some("claudeDesktop.probe.egressNote".to_owned()),
        ..base
    })
}

/// 只接受可识别的模型列表，不把 HTML 或任意 JSON 当成模型列表。
///
/// 兼容 `{data:[...]}`、`{models:[...]}` 与顶层数组；条目可以是对象
/// （取 `id`）或裸字符串。混入无法识别的条目时整份响应按无效处理，避免把
/// 残缺响应算成“通过”。
fn parse_model_ids(body: &[u8]) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let items = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.get("models").and_then(serde_json::Value::as_array))
        .or_else(|| value.as_array())?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            serde_json::Value::String(id) => out.push(id.clone()),
            serde_json::Value::Object(map) => {
                let id = map.get("id").and_then(serde_json::Value::as_str)?;
                out.push(id.to_owned());
            }
            _ => return None,
        }
    }
    Some(out)
}


// ---------------------------------------------------------------------------
// Messages 兼容性核对（用户显式触发，会产生极少量计费）
// ---------------------------------------------------------------------------

/// Messages 端点的真实可用性。与 `/models` 检测分开：`/models` 通过不能证明
/// Desktop 只带 Bearer 时 `/messages` 也能通过。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MessagesStatus {
    NotChecked,
    /// 2xx 且返回可识别的 Messages 响应。
    MessagesAccessible,
    /// 401/403：鉴权被拒。
    AuthRejected,
    /// 404/405：该地址没有 Messages 端点。
    EndpointUnavailable,
    /// 400/422：请求被拒（常见于不接受裸 Bearer 或模型名不受支持）。
    RequestRejected,
    RateLimited,
    NetworkFailed,
    /// 其它非 2xx。
    ServerError,
    /// 2xx 但响应不是 Messages 形态。
    InvalidResponse,
}

impl MessagesStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotChecked => "notChecked",
            Self::MessagesAccessible => "messagesAccessible",
            Self::AuthRejected => "authRejected",
            Self::EndpointUnavailable => "endpointUnavailable",
            Self::RequestRejected => "requestRejected",
            Self::RateLimited => "rateLimited",
            Self::NetworkFailed => "networkFailed",
            Self::ServerError => "serverError",
            Self::InvalidResponse => "invalidResponse",
        }
    }

    /// 只有确实拿到 Messages 响应才算通过。
    pub fn passed(self) -> bool {
        self == Self::MessagesAccessible
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesReport {
    pub url: String,
    pub model: String,
    pub status: MessagesStatus,
    pub http_status: Option<u16>,
    /// 上游自报的模型；用于区分“中转改写了模型名”。
    pub response_model: Option<String>,
    pub stop_reason: Option<String>,
    pub checked_at: i64,
    /// 只带安全的分类说明或上游错误类型，绝不含响应正文或密钥。
    pub note: Option<String>,
}

fn messages_report(
    url: &str,
    model: &str,
    status: MessagesStatus,
    http_status: Option<u16>,
) -> MessagesReport {
    MessagesReport {
        url: url.to_owned(),
        model: model.to_owned(),
        status,
        http_status,
        response_model: None,
        stop_reason: None,
        checked_at: chrono::Utc::now().timestamp(),
        note: None,
    }
}

/// 用最小请求核对 Messages 端点是否接受**裸 Bearer**。
///
/// 调用方必须由用户显式触发：这会向真实上游发一次 `max_tokens: 1` 的请求，
/// 可能产生极少量的计费。不写入任何配置文件，也不改动 Provider 的测试状态。
pub fn run_messages_check(
    provider: &Provider,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> DesktopResult<MessagesReport> {
    let (_, messages_url, _) = resolve_direct_endpoints(provider)?;
    let key = super::provider::require_inference_key(provider)?;
    let effective = newapi::effective_proxy(provider.network_mode(), proxy);
    let client = newapi::client_with_timeout(&effective, Duration::from_secs(60)).map_err(|_| {
        Box::new(DesktopError::new(
            super::types::error_code::PROBE_FAILED,
            "claudeDesktop.errors.network",
        ))
    })?;
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "hi" }],
    });
    let response = match client
        .post(&messages_url)
        // Claude Desktop 只发 Authorization: Bearer，不带 x-api-key。
        .header("Authorization", format!("Bearer {key}"))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
    {
        Ok(response) => response,
        Err(_) => {
            return Ok(messages_report(
                &messages_url,
                model,
                MessagesStatus::NetworkFailed,
                None,
            ))
        }
    };
    let code = response.status().as_u16();
    let mut text = String::new();
    let _ = response.take(8192).read_to_string(&mut text);
    let status = match code {
        200..=299 => MessagesStatus::NotChecked,
        401 | 403 => MessagesStatus::AuthRejected,
        404 | 405 => MessagesStatus::EndpointUnavailable,
        400 | 422 => MessagesStatus::RequestRejected,
        429 => MessagesStatus::RateLimited,
        _ => MessagesStatus::ServerError,
    };
    if status != MessagesStatus::NotChecked {
        let mut report = messages_report(&messages_url, model, status, Some(code));
        // 只保留上游错误的类型名，绝不回传正文。
        report.note = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|error| error.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        return Ok(report);
    }
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            return Ok(messages_report(
                &messages_url,
                model,
                MessagesStatus::InvalidResponse,
                Some(code),
            ))
        }
    };
    // 可识别的 Messages 响应：有 content 数组或明确的 type=message。
    let recognizable = parsed
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some()
        || parsed
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind == "message");
    if !recognizable {
        return Ok(messages_report(
            &messages_url,
            model,
            MessagesStatus::InvalidResponse,
            Some(code),
        ));
    }
    let mut report = messages_report(
        &messages_url,
        model,
        MessagesStatus::MessagesAccessible,
        Some(code),
    );
    report.response_model = parsed
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    report.stop_reason = parsed
        .get("stop_reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(report)
}
// ---------------------------------------------------------------------------
#[cfg(test)]
mod parse_tests {
    use super::parse_model_ids;

    #[test]
    fn accepts_data_models_and_bare_arrays_but_rejects_html() {
        assert_eq!(
            parse_model_ids(br#"{"data":[{"id":"a"},{"id":"b"}]}"#).unwrap(),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(
            parse_model_ids(br#"{"models":["a","b"]}"#).unwrap(),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(parse_model_ids(br#"["a"]"#).unwrap(), vec!["a".to_owned()]);
        assert!(parse_model_ids(b"<!doctype html><html></html>").is_none());
        assert!(parse_model_ids(br#"{"error":"nope"}"#).is_none());
        // 对象与裸字符串混排是合法形态。
        assert_eq!(
            parse_model_ids(br#"{"data":[{"id":"a"},"b"]}"#).unwrap(),
            vec!["a".to_owned(), "b".to_owned()]
        );
        // 缺 id 的条目、数字条目等无法识别的形态不算通过。
        assert!(parse_model_ids(br#"{"data":[{"name":"a"}]}"#).is_none());
        assert!(parse_model_ids(br#"{"data":[1,2]}"#).is_none());
        assert!(parse_model_ids(br#"{"data":[{"id":7}]}"#).is_none());
        assert!(parse_model_ids(br#"{"data":[]}"#).unwrap().is_empty());
    }
}

/// 检测出口说明，用于 DTO。
pub fn egress_label(provider: &Provider) -> String {
    if provider.network_mode() == "direct" {
        "direct".to_owned()
    } else {
        "proxy".to_owned()
    }
}

