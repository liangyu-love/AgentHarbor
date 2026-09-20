use super::{models::ApiProxyConfigPayload, newapi};
use serde_json::Value;

/// Try the balance protocols commonly exposed by New API relays and official
/// providers. Authentication errors are preserved so a bad credential is not
/// hidden behind the generic auto-detection error.
pub fn query(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<(newapi::Balance, &'static str), String> {
    let mut errors = Vec::new();

    if connection.user_id.is_some() && !connection.access_token.is_empty() {
        match newapi::balance(connection, proxy) {
            Ok(balance) => return Ok((balance, "newapi")),
            Err(error) => errors.push(error),
        }
    }
    if !connection.api_key.is_empty() {
        match billing_balance(connection, proxy) {
            Ok(balance) => return Ok((balance, "billing")),
            Err(error) => errors.push(error),
        }
        match deepseek_balance(connection, proxy) {
            Ok(balance) => return Ok((balance, "deepseek")),
            Err(error) => errors.push(error),
        }
    }

    errors
        .into_iter()
        .find(|error| {
            matches!(
                error.as_str(),
                "newapi.errors.auth" | "newapi.errors.userMismatch" | "newapi.errors.rejected"
            )
        })
        .map_or_else(|| Err("newapi.errors.balanceDetect".into()), Err)
}

fn parse_first_f64(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
    })
}

// OpenAI-compatible billing endpoints exposed by one-api / new-api style relays.
fn billing_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        newapi::auth_header(&connection.api_key)?,
    );
    let root = newapi::base_url(&connection.base_url)?;
    let fetch = |path: &str| -> Result<Value, String> {
        let url = root
            .join(path)
            .map_err(|_| "newapi.errors.url".to_owned())?;
        newapi::get(&client, url, headers.clone())
    };
    let subscription = fetch("v1/dashboard/billing/subscription")
        .or_else(|_| fetch("dashboard/billing/subscription"))?;
    let hard_limit = parse_first_f64(
        &subscription,
        &["hard_limit_usd", "hard_limit", "system_hard_limit_usd"],
    )
    .ok_or("newapi.errors.response")?;
    let usage = fetch("v1/dashboard/billing/usage")
        .or_else(|_| fetch("dashboard/billing/usage"))
        .ok();
    let used_cents = usage
        .as_ref()
        .and_then(|value| parse_first_f64(value, &["total_usage"]))
        .unwrap_or(0.0);
    let remaining = hard_limit - used_cents / 100.0;
    Ok(newapi::Balance {
        remaining,
        used: Some(used_cents / 100.0),
        unit: "USD".into(),
        raw_quota: remaining,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
    })
}

fn deepseek_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    let host = newapi::base_url(&connection.base_url)?
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host != "api.deepseek.com" {
        return Err("newapi.errors.balanceDetect".into());
    }
    let client = newapi::client(proxy)?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        newapi::auth_header(&connection.api_key)?,
    );
    let url = newapi::base_url(&connection.base_url)?
        .join("api/user/balance")
        .map_err(|_| "newapi.errors.url".to_owned())?;
    let value = newapi::get(&client, url, headers)?;
    let info = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or("newapi.errors.response")?;
    let total = info
        .get("total_balance")
        .and_then(Value::as_str)
        .and_then(|text| text.trim().parse::<f64>().ok())
        .or_else(|| parse_first_f64(info, &["total_balance"]))
        .ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        remaining: total,
        used: None,
        unit: info
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        raw_quota: total,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
    })
}
