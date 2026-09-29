use super::{models::ApiProxyConfigPayload, newapi};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountPlatform {
    NewApi,
    Sub2Api,
    Unknown,
}

/// Query a balance without mixing credential protocols. Official hosts are
/// handled explicitly. Custom domains are identified through unauthenticated
/// public metadata before an account access token is sent.
pub fn query(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<(newapi::Balance, &'static str), String> {
    let host = host(connection)?;
    let root = newapi::base_url(&connection.base_url)?;
    if host == "opencode.ai" && root.path().trim_end_matches('/') == "/zen/go" {
        return official_balance(connection, proxy, "opencodeGo", "/zen/go/v1/usage")
            .map(|b| (b, "opencodeGo"));
    }
    if host == "api.kimi.com" && root.path().starts_with("/coding/") {
        return official_balance(connection, proxy, "kimiCoding", "/coding/v1/usages")
            .map(|b| (b, "kimiCoding"));
    }
    if matches!(host.as_str(), "api.minimaxi.com" | "api.minimax.io") {
        return official_balance(
            connection,
            proxy,
            "minimaxPlan",
            "/v1/api/openplatform/coding_plan/remains",
        )
        .map(|b| (b, "minimaxPlan"));
    }
    if matches!(host.as_str(), "open.bigmodel.cn" | "api.z.ai")
        && root.path().starts_with("/api/coding/")
    {
        return official_balance(
            connection,
            proxy,
            "glmPlan",
            "/api/monitor/usage/quota/limit",
        )
        .map(|b| (b, "glmPlan"));
    }
    // This endpoint is published by this site and uses the model key, not the
    // account session token. Other custom sites can opt in via a query script.
    if host == "api.iiiiitoken.com" && connection.access_token.is_empty() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, newapi::auth_header(&connection.api_key)?);
        let value = newapi::get(
            &newapi::client(proxy)?,
            root.join("/v1/usage").map_err(|_| "newapi.errors.url")?,
            headers,
        )?;
        return parse_custom_balance(value.get("data").unwrap_or(&value))
            .map(|b| (b, "apiKeyQuota"));
    }

    if let Some((source, path)) = official_adapter(&host) {
        if connection.api_key.is_empty() {
            return Err("newapi.errors.apiKeyRequired".into());
        }
        return official_balance(connection, proxy, source, path).map(|balance| (balance, source));
    }

    // These are first-party endpoints with no documented API-key balance
    // endpoint. Never probe them with NewAPI/Sub2API account credentials.
    if is_known_official_host(&host) {
        return Err("newapi.errors.balanceUnsupported".into());
    }

    if connection.access_token.is_empty() {
        return Err("newapi.errors.accessTokenRequired".into());
    }

    match detect_account_platform(connection, proxy)? {
        AccountPlatform::NewApi => {
            if connection.user_id.is_none() {
                return Err("newapi.errors.userId".into());
            }
            newapi::balance(connection, proxy).map(|balance| (balance, "newapi"))
        }
        AccountPlatform::Sub2Api => {
            sub2api_balance(connection, proxy).map(|balance| (balance, "sub2api"))
        }
        AccountPlatform::Unknown => Err("newapi.errors.balanceDetect".into()),
    }
}

fn host(connection: &newapi::Connection) -> Result<String, String> {
    Ok(newapi::base_url(&connection.base_url)?
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase())
}

fn is_known_official_host(host: &str) -> bool {
    matches!(
        host,
        "api.openai.com"
            | "api.anthropic.com"
            | "generativelanguage.googleapis.com"
            | "api.cohere.com"
            | "api.mistral.ai"
            | "api.x.ai"
            | "opencode.ai"
            | "api.commandcode.ai"
            | "token-plan-cn.xiaomimimo.com"
    )
}

fn detect_account_platform(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<AccountPlatform, String> {
    let client = newapi::client(proxy)?;
    let root = newapi::base_url(&connection.base_url)?;

    let newapi_status = newapi::get(
        &client,
        root.join("api/status").map_err(|_| "newapi.errors.url")?,
        HeaderMap::new(),
    );
    if newapi_status
        .as_ref()
        .is_ok_and(|value| is_newapi_public_status(value))
    {
        return Ok(AccountPlatform::NewApi);
    }

    let sub2api_settings = newapi::get(
        &client,
        root.join("api/v1/settings/public")
            .map_err(|_| "newapi.errors.url")?,
        HeaderMap::new(),
    );
    if sub2api_settings
        .as_ref()
        .is_ok_and(|value| is_sub2api_public_settings(value))
    {
        return Ok(AccountPlatform::Sub2Api);
    }

    for result in [newapi_status, sub2api_settings] {
        if let Err(error) = result {
            if error == "newapi.errors.network" || error == "newapi.errors.timeout" {
                return Err(error);
            }
        }
    }
    Ok(AccountPlatform::Unknown)
}

fn is_newapi_public_status(value: &Value) -> bool {
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return false;
    };
    value.get("success").and_then(Value::as_bool) == Some(true)
        && [
            "system_name",
            "version",
            "start_time",
            "quota_per_unit",
            "quota_display_type",
            "display_in_currency",
        ]
        .iter()
        .any(|key| data.contains_key(*key))
}

fn is_sub2api_public_settings(value: &Value) -> bool {
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return false;
    };
    value.get("code").and_then(Value::as_i64) == Some(0)
        && data.get("site_name").and_then(Value::as_str).is_some()
        && data
            .get("registration_enabled")
            .and_then(Value::as_bool)
            .is_some()
}

fn sub2api_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        newapi::auth_header(&connection.access_token)?,
    );
    let url = newapi::base_url(&connection.base_url)?
        .join("api/v1/auth/me")
        .map_err(|_| "newapi.errors.url".to_owned())?;
    let value = newapi::get(&client, url, headers)?;
    parse_sub2api_balance(&value, connection.user_id)
}

fn parse_sub2api_balance(
    value: &Value,
    expected_user_id: Option<u64>,
) -> Result<newapi::Balance, String> {
    if value.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("newapi.errors.rejected".into());
    }
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or("newapi.errors.response")?;
    if let Some(expected) = expected_user_id {
        let actual = data
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("newapi.errors.response")?;
        if actual != expected {
            return Err("newapi.errors.userMismatch".into());
        }
    }
    let remaining = parse_number(data.get("balance")).ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        entries: vec![],
        remaining,
        used: None,
        unit: "USD".into(),
        raw_quota: remaining,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
    })
}

fn parse_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value.as_f64().or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<f64>().ok())
            })
        })
        .filter(|value| value.is_finite())
}

// Match parsed hostnames exactly: a lookalike URL must never receive credentials.
fn official_adapter(host: &str) -> Option<(&'static str, &'static str)> {
    match host {
        "api.deepseek.com" => Some(("deepseek", "/user/balance")),
        "api.stepfun.com" | "api.stepfun.ai" => Some(("stepfun", "/v1/accounts")),
        "api.siliconflow.cn" => Some(("siliconflow", "/v1/user/info")),
        "api.siliconflow.com" => Some(("siliconflowIntl", "/v1/user/info")),
        "openrouter.ai" => Some(("openrouter", "/api/v1/credits")),
        "api.novita.ai" => Some(("novita", "/v3/user/balance")),
        _ => None,
    }
}

fn official_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
    source: &str,
    path: &str,
) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        if source == "glmPlan" {
            newapi::api_key_header(&connection.api_key)?
        } else {
            newapi::auth_header(&connection.api_key)?
        },
    );
    let url = newapi::base_url(&connection.base_url)?
        .join(path)
        .map_err(|_| "newapi.errors.url".to_owned())?;
    parse_official_balance(source, &newapi::get(&client, url, headers)?)
}

fn parse_official_balance(source: &str, value: &Value) -> Result<newapi::Balance, String> {
    use newapi::BalanceEntry;
    let required =
        |value: Option<&Value>| parse_number(value).ok_or("newapi.errors.response".to_owned());
    let entry = |remaining: f64, unit: &str| BalanceEntry {
        label: None,
        resets_at: None,
        remaining,
        used: None,
        total: None,
        unit: unit.into(),
        valid: remaining > 0.0,
    };
    let entries = match source {
        "glmPlan" => {
            if value.get("success").and_then(Value::as_bool) == Some(false) {
                return Err("newapi.errors.rejected".into());
            }
            value
                .pointer("/data/limits")
                .and_then(Value::as_array)
                .ok_or("newapi.errors.response")?
                .iter()
                .filter_map(|window| {
                    let used = parse_number(window.get("percentage"))?;
                    let label = match window.get("unit").and_then(Value::as_i64) {
                        Some(3) => "rolling",
                        Some(6) => "weekly",
                        _ => window
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("quota"),
                    };
                    Some(BalanceEntry {
                        remaining: (100.0 - used).max(0.0),
                        used: Some(used),
                        total: Some(100.0),
                        unit: "%".into(),
                        valid: used < 100.0,
                        label: Some(label.into()),
                        resets_at: window
                            .get("nextResetTime")
                            .and_then(Value::as_i64)
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|v| v.to_rfc3339()),
                    })
                })
                .collect()
        }
        "minimaxPlan" => {
            if value
                .pointer("/base_resp/status_code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0)
            {
                return Err("newapi.errors.rejected".into());
            }
            let windows = value
                .get("model_remains")
                .and_then(Value::as_array)
                .ok_or("newapi.errors.response")?;
            let mut entries = Vec::new();
            for window in windows {
                let name = window
                    .get("model_name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == "general" {
                    for (label, field, reset) in [
                        ("rolling", "current_interval_remaining_percent", "end_time"),
                        (
                            "weekly",
                            "current_weekly_remaining_percent",
                            "weekly_end_time",
                        ),
                    ] {
                        if label == "weekly"
                            && window.get("current_weekly_status").and_then(Value::as_i64)
                                != Some(1)
                        {
                            continue;
                        }
                        if let Some(remaining) = parse_number(window.get(field)) {
                            entries.push(BalanceEntry {
                                remaining,
                                used: Some(100.0 - remaining),
                                total: Some(100.0),
                                unit: "%".into(),
                                valid: remaining > 0.0,
                                label: Some(label.into()),
                                resets_at: window
                                    .get(reset)
                                    .and_then(Value::as_i64)
                                    .and_then(chrono::DateTime::from_timestamp_millis)
                                    .map(|v| v.to_rfc3339()),
                            });
                        }
                    }
                } else if name.to_lowercase().starts_with("minimax") {
                    if let (Some(total), Some(remaining)) = (
                        parse_number(window.get("current_interval_total_count")),
                        parse_number(window.get("current_interval_usage_count")),
                    ) {
                        entries.push(BalanceEntry {
                            remaining,
                            used: Some((total - remaining).max(0.0)),
                            total: Some(total),
                            unit: "quota".into(),
                            valid: remaining > 0.0,
                            label: Some(name.into()),
                            resets_at: None,
                        });
                    }
                }
            }
            entries
        }
        "opencodeGo" => ["rolling", "weekly", "monthly"]
            .iter()
            .filter_map(|name| {
                let window = value.get("usage")?.get(*name)?;
                let used = parse_number(window.get("percent"))?;
                Some(BalanceEntry {
                    remaining: (100.0 - used).max(0.0),
                    used: Some(used),
                    total: Some(100.0),
                    unit: "%".into(),
                    valid: window.get("status").and_then(Value::as_str) != Some("rate-limited")
                        && used < 100.0,
                    label: Some((*name).into()),
                    resets_at: if used > 0.0 {
                        window
                            .get("resetsAt")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    } else {
                        None
                    },
                })
            })
            .collect(),
        "kimiCoding" => {
            let mut windows = Vec::new();
            if let Some(limits) = value.get("limits").and_then(Value::as_array) {
                for item in limits {
                    if let Some(detail) = item.get("detail") {
                        windows.push(("rolling", detail));
                    }
                }
            }
            if let Some(usage) = value.get("usage") {
                windows.push(("weekly", usage));
            }
            windows
                .into_iter()
                .filter_map(|(name, window)| {
                    let total = parse_number(window.get("limit"))?;
                    let remaining = parse_number(window.get("remaining"))?;
                    if total <= 0.0 {
                        return None;
                    }
                    Some(BalanceEntry {
                        remaining,
                        used: Some((total - remaining).max(0.0)),
                        total: Some(total),
                        unit: "quota".into(),
                        valid: remaining > 0.0,
                        label: Some(name.into()),
                        resets_at: window
                            .get("resetTime")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect()
        }
        "deepseek" => value
            .get("balance_infos")
            .and_then(Value::as_array)
            .ok_or("newapi.errors.response")?
            .iter()
            .map(|info| {
                let unit = info
                    .get("currency")
                    .and_then(Value::as_str)
                    .filter(|unit| !unit.is_empty())
                    .ok_or("newapi.errors.response")?;
                let mut item = entry(required(info.get("total_balance"))?, unit);
                item.valid &= value
                    .get("is_available")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                Ok(item)
            })
            .collect::<Result<Vec<_>, String>>()?,
        "stepfun" => vec![entry(required(value.get("balance"))?, "CNY")],
        "siliconflow" | "siliconflowIntl" => {
            if value
                .get("code")
                .is_some_and(|code| code.as_i64() != Some(20000) && code.as_i64() != Some(0))
            {
                return Err("newapi.errors.rejected".into());
            }
            let data = value.get("data").ok_or("newapi.errors.response")?;
            vec![entry(
                required(data.get("totalBalance").or_else(|| data.get("balance")))?,
                if source == "siliconflow" {
                    "CNY"
                } else {
                    "USD"
                },
            )]
        }
        "openrouter" => {
            let data = value.get("data").ok_or("newapi.errors.response")?;
            let total = required(data.get("total_credits"))?;
            let used = required(data.get("total_usage"))?;
            let mut item = entry(total - used, "USD");
            if !item.remaining.is_finite() {
                return Err("newapi.errors.response".into());
            }
            item.total = Some(total);
            item.used = Some(used);
            vec![item]
        }
        "novita" => vec![entry(
            required(value.get("availableBalance"))? / 10000.0,
            "USD",
        )],
        _ => return Err("newapi.errors.balanceUnsupported".into()),
    };
    balance_from_entries(entries)
}

pub(crate) fn balance_from_entries(
    entries: Vec<newapi::BalanceEntry>,
) -> Result<newapi::Balance, String> {
    let first = entries.first().ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        remaining: first.remaining,
        used: first.used,
        unit: first.unit.clone(),
        raw_quota: first.remaining,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
        entries,
    })
}

pub(crate) fn parse_custom_balance(value: &Value) -> Result<newapi::Balance, String> {
    let values = value
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value.clone()]);
    if values.len() > 32 {
        return Err("newapi.errors.response".into());
    }
    balance_from_entries(
        values
            .iter()
            .map(|v| {
                let remaining = parse_number(
                    v.get("remaining")
                        .filter(|v| !v.is_null())
                        .or_else(|| v.pointer("/quota/remaining").filter(|v| !v.is_null()))
                        .or_else(|| v.get("balance")),
                )
                .ok_or("newapi.errors.response")?;
                Ok(newapi::BalanceEntry {
                    remaining,
                    used: parse_number(v.get("used")),
                    total: parse_number(v.get("total")),
                    unit: v
                        .get("unit")
                        .or_else(|| v.pointer("/quota/unit"))
                        .and_then(Value::as_str)
                        .unwrap_or("quota")
                        .chars()
                        .take(24)
                        .collect(),
                    valid: v
                        .get("isValid")
                        .or_else(|| v.get("is_active"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    label: v
                        .get("planName")
                        .and_then(Value::as_str)
                        .map(|s| s.chars().take(80).collect()),
                    resets_at: v.get("resetsAt").and_then(Value::as_str).map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn coding_plans_preserve_window_units_reset_and_missing_data() {
        let go = parse_official_balance(
            "opencodeGo",
            &json!({"usage": {
                "rolling":{"percent":0,"resetsAt":"2030-01-01T00:00:00Z","status":"ok"},
                "weekly":{"percent":"42","resetsAt":"2030-01-02T00:00:00Z","status":"ok"},
                "monthly":{"percent":100,"status":"rate-limited"}
            }}),
        )
        .unwrap();
        assert_eq!(go.entries.len(), 3);
        assert_eq!(go.entries[1].remaining, 58.0);
        assert_eq!(go.entries[0].resets_at, None);
        assert!(!go.entries[2].valid);
        let kimi =
            parse_official_balance("kimiCoding", &json!({"usage":{"limit":100,"remaining":25}}))
                .unwrap();
        assert_eq!(kimi.entries[0].used, Some(75.0));
        let mini = parse_official_balance("minimaxPlan", &json!({"model_remains":[{"model_name":"general","current_interval_remaining_percent":60,"current_weekly_status":3,"current_weekly_remaining_percent":100}]})).unwrap();
        assert_eq!(mini.entries.len(), 1);
        assert_eq!(mini.remaining, 60.0);
        let glm = parse_official_balance("glmPlan", &json!({"success":true,"data":{"limits":[{"type":"TOKENS_LIMIT","unit":6,"percentage":20}]}})).unwrap();
        assert_eq!(glm.entries[0].label.as_deref(), Some("weekly"));
        for source in ["opencodeGo", "kimiCoding", "minimaxPlan", "glmPlan"] {
            assert!(parse_official_balance(source, &json!({})).is_err());
        }
        assert!(parse_custom_balance(&json!({"isValid":true})).is_err());
        assert_eq!(
            parse_custom_balance(
                &json!({"remaining":null,"quota":{"remaining":null},"balance":0,"isValid":false})
            )
            .unwrap()
            .remaining,
            0.0
        );
    }
    #[test]
    fn official_balances_validate_units_and_do_not_invent_missing_amounts() {
        let balance = parse_official_balance(
            "deepseek",
            &json!({"is_available":false,"balance_infos":[
                {"currency":"CNY","total_balance":"12.5"}, {"currency":"USD","total_balance":2}
            ]}),
        )
        .unwrap();
        assert_eq!(balance.entries.len(), 2);
        assert_eq!(balance.entries[1].unit, "USD");
        assert!(!balance.entries[0].valid);
        for (source, body, remaining, unit) in [
            ("stepfun", json!({"balance":"8.5"}), 8.5, "CNY"),
            (
                "siliconflow",
                json!({"code":20000,"data":{"totalBalance":"4.5","balance":1}}),
                4.5,
                "CNY",
            ),
            ("siliconflowIntl", json!({"data":{"balance":3}}), 3.0, "USD"),
            (
                "openrouter",
                json!({"data":{"total_credits":20,"total_usage":7}}),
                13.0,
                "USD",
            ),
            ("novita", json!({"availableBalance":"25000"}), 2.5, "USD"),
        ] {
            let balance = parse_official_balance(source, &body).unwrap();
            assert_eq!(balance.remaining, remaining);
            assert_eq!(balance.unit, unit);
            assert!(parse_official_balance(source, &json!({})).is_err());
        }
        assert!(parse_official_balance("deepseek", &json!({"balance_infos":[]})).is_err());
        assert!(parse_official_balance("stepfun", &json!({"balance":"NaN"})).is_err());
        assert!(
            parse_official_balance("siliconflow", &json!({"code":401,"data":{"balance":100}}))
                .is_err()
        );
        assert!(official_adapter("api.deepseek.com.evil.test").is_none());
        assert!(official_adapter("evil.test/api.deepseek.com").is_none());
        assert_eq!(
            official_adapter("api.deepseek.com").unwrap().1,
            "/user/balance"
        );
        assert_eq!(
            official_adapter("openrouter.ai").unwrap().1,
            "/api/v1/credits"
        );
        assert_eq!(
            official_adapter("api.novita.ai").unwrap().1,
            "/v3/user/balance"
        );
    }
    use std::{
        io::{Read, Write},
        sync::{Arc, Mutex},
        time::Duration,
    };

    fn mock_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = [0u8; 16 * 1024];
                let count = stream.read(&mut bytes).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes[..count]).to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}/"), requests, handle)
    }

    #[test]
    fn identifies_newapi_and_sub2api_public_metadata() {
        assert!(is_newapi_public_status(&json!({
            "success": true,
            "data": {"system_name": "Relay", "quota_per_unit": 500000}
        })));
        assert!(!is_newapi_public_status(
            &json!({"success": true, "data": {}})
        ));

        assert!(is_sub2api_public_settings(&json!({
            "code": 0,
            "data": {"site_name": "Sub2API", "registration_enabled": true}
        })));
        assert!(!is_sub2api_public_settings(&json!({
            "code": 0,
            "data": {"site_name": "unrelated"}
        })));
    }

    #[test]
    fn parses_sub2api_balance_and_checks_optional_user_id() {
        let value = json!({"code": 0, "data": {"id": 37, "balance": "12.50"}});
        let balance = parse_sub2api_balance(&value, Some(37)).unwrap();
        assert_eq!(balance.remaining, 12.5);
        assert_eq!(balance.unit, "USD");
        assert_eq!(
            parse_sub2api_balance(&value, Some(38)).unwrap_err(),
            "newapi.errors.userMismatch"
        );
    }

    #[test]
    fn parses_deepseek_balance_without_newapi_fields() {
        let value = json!({
            "is_available": true,
            "balance_infos": [{"currency": "CNY", "total_balance": "23.75"}]
        });
        let balance = parse_official_balance("deepseek", &value).unwrap();
        assert_eq!(balance.remaining, 23.75);
        assert_eq!(balance.unit, "CNY");
    }

    #[test]
    fn official_hosts_never_enter_account_protocol_detection() {
        for host in [
            "api.openai.com",
            "api.anthropic.com",
            "opencode.ai",
            "api.commandcode.ai",
            "token-plan-cn.xiaomimimo.com",
        ] {
            assert!(is_known_official_host(host));
        }
        assert!(!is_known_official_host("relay.example.com"));

        for base_url in [
            "https://api.openai.com/v1",
            "https://opencode.ai/zen/v1",
            "https://api.commandcode.ai/provider/v1",
        ] {
            let connection = newapi::Connection {
                base_url: base_url.into(),
                user_id: Some(37),
                access_token: "account-token-must-not-be-used".into(),
                api_key: "model-key-must-not-be-used".into(),
                ..Default::default()
            };
            assert_eq!(
                query(&connection, &ApiProxyConfigPayload::default()).unwrap_err(),
                "newapi.errors.balanceUnsupported"
            );
        }
    }

    #[test]
    fn newapi_adapter_detects_public_metadata_before_sending_account_credentials() {
        let (base_url, requests, server) = mock_server(vec![
            (
                "200 OK",
                r#"{"success":true,"data":{"system_name":"Relay","quota_per_unit":500000}}"#,
            ),
            (
                "200 OK",
                r#"{"success":true,"data":{"id":37,"quota":750000,"used_quota":250000}}"#,
            ),
            (
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
            ),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "account-token".into(),
            api_key: "model-key".into(),
            ..Default::default()
        };
        let (balance, source) = query(&connection, &ApiProxyConfigPayload::default()).unwrap();
        server.join().unwrap();
        assert_eq!(source, "newapi");
        assert_eq!(balance.remaining, 1.5);
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /api/status "));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(requests[1].starts_with("GET /api/user/self "));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer account-token"));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("new-api-user: 37"));
    }

    #[test]
    fn sub2api_adapter_uses_auth_me_and_never_uses_the_model_key() {
        let (base_url, requests, server) = mock_server(vec![
            ("404 Not Found", r#"{}"#),
            (
                "200 OK",
                r#"{"code":0,"data":{"site_name":"Sub2API","registration_enabled":true}}"#,
            ),
            ("200 OK", r#"{"code":0,"data":{"id":37,"balance":9.25}}"#),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "sub2-account-token".into(),
            api_key: "model-key-must-not-be-used".into(),
            ..Default::default()
        };
        let (balance, source) = query(&connection, &ApiProxyConfigPayload::default()).unwrap();
        server.join().unwrap();
        assert_eq!(source, "sub2api");
        assert_eq!(balance.remaining, 9.25);
        let requests = requests.lock().unwrap();
        assert!(requests[2].starts_with("GET /api/v1/auth/me "));
        assert!(requests[2]
            .to_ascii_lowercase()
            .contains("authorization: bearer sub2-account-token"));
        assert!(requests
            .iter()
            .all(|request| !request.contains("model-key-must-not-be-used")));
    }

    #[test]
    fn unknown_sites_receive_only_credential_free_detection_requests() {
        let (base_url, requests, server) = mock_server(vec![
            ("200 OK", r#"{"hello":"world"}"#),
            ("200 OK", r#"{"code":0,"data":{"site_name":"Other"}}"#),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "account-token-must-not-leak".into(),
            api_key: "model-key-must-not-leak".into(),
            ..Default::default()
        };
        assert_eq!(
            query(&connection, &ApiProxyConfigPayload::default()).unwrap_err(),
            "newapi.errors.balanceDetect"
        );
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("authorization:")));
        assert!(requests
            .iter()
            .all(|request| !request.contains("must-not-leak")));
    }

    #[test]
    fn deepseek_adapter_uses_only_the_official_balance_shape_and_model_key() {
        let (base_url, requests, server) = mock_server(vec![(
            "200 OK",
            r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"8.50"}]}"#,
        )]);
        let connection = newapi::Connection {
            base_url,
            access_token: "account-token-must-not-be-used".into(),
            api_key: "deepseek-model-key".into(),
            ..Default::default()
        };
        let balance = official_balance(
            &connection,
            &ApiProxyConfigPayload::default(),
            "deepseek",
            "/user/balance",
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(balance.remaining, 8.5);
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /user/balance "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer deepseek-model-key"));
        assert!(!requests[0].contains("account-token-must-not-be-used"));
    }
}
