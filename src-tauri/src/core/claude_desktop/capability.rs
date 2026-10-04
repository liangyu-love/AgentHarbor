//! Claude Desktop 直连：上游能力核对（用户显式触发）。
//!
//! 与 [`super::probe`] 的只读检测分开：那里的 `/models`、`/messages` 检测回答
//! 「这个地址能不能用」，这里回答「这条链路支持哪些能力」。直连模式下流式与
//! 工具调用由 Desktop 自己发起，profile 只决定地址、凭据与模型菜单，因此把两者
//! 混在一个模块里会掩盖这个区别。

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::core::models::ApiProxyConfigPayload;
use crate::core::{newapi, router::Provider};

use super::provider::resolve_direct_endpoints;
use super::types::{DesktopError, DesktopResult};
// 能力核对：上游是否支持流式与工具调用
// ---------------------------------------------------------------------------

/// 逐项能力核对结果。全部只读探测（除工具往返与一次流式请求外不发别的请求）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    /// 上游 `/v1/models` 返回的全部模型 id。
    pub available_models: Vec<String>,
    /// 其中符合 Desktop 命名与协议要求、可以放进菜单的模型 id。
    pub selectable_models: Vec<String>,
    pub stream_status: Option<u16>,
    pub stream_content_type: Option<String>,
    /// SSE 里出现过的 `event:` 名称（去重后）。
    pub stream_events: Vec<String>,
    pub stream_message_start: bool,
    pub stream_content_delta: bool,
    pub stream_message_stop: bool,
    pub stream_error: bool,
    pub tool_status: Option<u16>,
    pub tool_stop_reason: Option<String>,
    pub tool_block_types: Vec<String>,
    pub tool_has_use: bool,
    pub tool_error_type: Option<String>,
}

/// 用与 Claude Desktop 相同的调用形态核对该上游支持什么。
///
/// 直连模式下这些能力由 Desktop 自己发起，profile 只决定地址、凭据与模型菜单；
/// 因此这里回答的是「这条链路支持什么」，而不是「Desktop 会怎么做」。
pub fn probe_capabilities(
    provider: &Provider,
    model: &str,
    proxy: &ApiProxyConfigPayload,
    app_data_dir: &Path,
) -> DesktopResult<CapabilityReport> {
    let (_, messages_url, models_url) = resolve_direct_endpoints(provider)?;
    let key = super::provider::require_inference_key(provider)?;
    let effective = newapi::effective_proxy(provider.network_mode(), proxy);
    let client = newapi::client_with_timeout(&effective, Duration::from_secs(90)).map_err(|_| {
        Box::new(DesktopError::new(
            super::types::error_code::PROBE_FAILED,
            "claudeDesktop.errors.network",
        ))
    })?;
    let auth = format!("Bearer {key}");

    // 1) 可放进菜单的模型。
    let mut available_models: Vec<String> = Vec::new();
    if let Ok(response) = client
        .get(&models_url)
        .header("Authorization", auth.clone())
        .send()
    {
        if let Ok(value) = response.json::<serde_json::Value>() {
            available_models = value
                .get("data")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            item.get("id")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    let policy = super::provider::DesktopVersionPolicy::from_version(Some("2.19675.0"));
    let catalog = crate::core::models_dev::load(app_data_dir);
    let selectable_models: Vec<String> = available_models
        .iter()
        .filter(|id| {
            super::provider::model_candidate(provider, id, &catalog, policy).selectable
        })
        .cloned()
        .collect();

    // 2) 流式。
    let mut report = CapabilityReport {
        available_models,
        selectable_models,
        stream_status: None,
        stream_content_type: None,
        stream_events: Vec::new(),
        stream_message_start: false,
        stream_content_delta: false,
        stream_message_stop: false,
        stream_error: false,
        tool_status: None,
        tool_stop_reason: None,
        tool_block_types: Vec::new(),
        tool_has_use: false,
        tool_error_type: None,
    };
    let stream_body = serde_json::json!({
        "model": model,
        "max_tokens": 32,
        "stream": true,
        "messages": [{ "role": "user", "content": "count: one two three" }],
    });
    if let Ok(response) = client
        .post(&messages_url)
        .header("Authorization", auth.clone())
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&stream_body)
        .send()
    {
        report.stream_status = Some(response.status().as_u16());
        report.stream_content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut text = String::new();
        let _ = response.take(256 * 1024).read_to_string(&mut text);
        let mut events: Vec<String> = text
            .lines()
            .filter_map(|line| line.strip_prefix("event: ").map(str::to_owned))
            .collect();
        events.sort();
        events.dedup();
        report.stream_message_start = text.contains("message_start");
        report.stream_content_delta = text.contains("content_block_delta");
        report.stream_message_stop = text.contains("message_stop");
        report.stream_error = text.contains("event: error") || text.contains("\"type\":\"error\"");
        report.stream_events = events;
    }

    // 3) 工具调用往返。
    let tool_body = serde_json::json!({
        "model": model,
        "max_tokens": 128,
        "tools": [{
            "name": "get_time",
            "description": "Return the current time for a city",
            "input_schema": {
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"]
            }
        }],
        "tool_choice": { "type": "tool", "name": "get_time" },
        "messages": [{ "role": "user", "content": "What time is it in Shanghai?" }],
    });
    if let Ok(response) = client
        .post(&messages_url)
        .header("Authorization", auth)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&tool_body)
        .send()
    {
        report.tool_status = Some(response.status().as_u16());
        let mut text = String::new();
        let _ = response.take(256 * 1024).read_to_string(&mut text);
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            report.tool_stop_reason = value
                .get("stop_reason")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            report.tool_error_type = value
                .get("error")
                .and_then(|error| error.get("type"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            report.tool_block_types = value
                .get("content")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            item.get("type")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            report.tool_has_use = report.tool_block_types.iter().any(|kind| kind == "tool_use");
        }
    }
    Ok(report)
}
