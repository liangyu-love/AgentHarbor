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
    // Models use /provider/v1; billing is always rooted at /alpha.
    if host == "api.commandcode.ai"
        && matches!(
            root.path().trim_end_matches('/'),
            "" | "/provider" | "/provider/v1"
        )
    {
        return command_code_balance(connection, proxy).map(|b| (b, "commandCode"));
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
            | "api.xiaomimimo.com"
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
    if newapi_status.as_ref().is_ok_and(is_newapi_public_status) {
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
        .is_ok_and(is_sub2api_public_settings)
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
        command_code: None,
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

/// Command Code exposes Coding Plan facts through the account endpoints used
/// by its official provider integration. Keep generic balance entries compatible
/// and attach the account-wide usage summary separately from the credit pools.
fn command_code_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    if connection.api_key.is_empty() {
        return Err("newapi.errors.apiKeyRequired".into());
    }
    let client = newapi::client(proxy)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, newapi::auth_header(&connection.api_key)?);
    let root = newapi::base_url(&connection.base_url)?;
    let credits = newapi::get(
        &client,
        root.join("/alpha/billing/credits")
            .map_err(|_| "newapi.errors.url")?,
        headers.clone(),
    )?;
    // This is the official account summary, not requests observed by this app.
    // A missing/unsupported summary must never discard a usable credit balance.
    let usage_url = root.join("/alpha/usage/summary").map_err(|_| "newapi.errors.url")?;
    let account_url = root.join("/alpha/whoami").map_err(|_| "newapi.errors.url")?;
    let mut subscription_url = root
        .join("/alpha/billing/subscriptions")
        .map_err(|_| "newapi.errors.url")?;
    // Keep the mandatory credit request's original failure behavior, then
    // overlap the new summary with identity/subscription reads. Its timeout
    // must not add another full serial request budget to a balance refresh.
    let (account, subscription, usage) = std::thread::scope(|scope| {
        let usage_request = scope.spawn(|| newapi::get(&client, usage_url, headers.clone()).ok());
        let account = newapi::get(&client, account_url, headers.clone()).ok();
        if let Some(org_id) = account
            .as_ref()
            .filter(|item| item.get("success").and_then(Value::as_bool) != Some(false))
            .and_then(|item| item.pointer("/org/id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            subscription_url.query_pairs_mut().append_pair("orgId", org_id);
        }
        let subscription = newapi::get(&client, subscription_url, headers.clone()).ok();
        (account, subscription, usage_request.join().ok().flatten())
    });
    let mut balance = parse_command_code_balance(&credits, subscription.as_ref())?;
    if let Some(details) = balance.command_code.as_mut() {
        details.account = parse_command_code_account(account.as_ref());
        details.usage = parse_command_code_usage(usage.as_ref());
        balance.request_count = details.usage.as_ref().and_then(|usage| usage.total_count);
    }
    Ok(balance)
}

fn command_code_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
        .map(|text| text.chars().take(160).collect())
}

fn parse_command_code_account(value: Option<&Value>) -> Option<newapi::CommandCodeBalanceAccount> {
    let value = value.filter(|value| value.get("success").and_then(Value::as_bool) != Some(false))?;
    let user = value.get("user")?.as_object()?;
    let name = command_code_text(user.get("name"));
    let user_name = command_code_text(user.get("userName"));
    (name.is_some() || user_name.is_some())
        .then_some(newapi::CommandCodeBalanceAccount { name, user_name })
}

fn command_code_count(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value.as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .or_else(|| {
            parse_number(Some(value))
                .filter(|count| *count >= 0.0 && count.fract() == 0.0 && *count <= 9_007_199_254_740_991.0)
                .map(|count| count as u64)
        })
        .filter(|count| *count <= 9_007_199_254_740_991)
}

fn parse_command_code_usage(value: Option<&Value>) -> Option<newapi::CommandCodeBalanceUsage> {
    let value = value
        .filter(|value| value.get("success").and_then(Value::as_bool) != Some(false))?
        .as_object()?;
    let amount = |key| parse_number(value.get(key)).filter(|amount| *amount >= 0.0);
    let usage = newapi::CommandCodeBalanceUsage {
        total_count: command_code_count(value.get("totalCount")),
        completed_count: command_code_count(value.get("completedCount")),
        failed_count: command_code_count(value.get("failedCount")),
        success_rate: amount("successRate").filter(|rate| *rate <= 100.0),
        total_cost: amount("totalCost"),
        total_credits: amount("totalCredits"),
        total_tokens_in: command_code_count(value.get("totalTokensIn")),
        total_tokens_out: command_code_count(value.get("totalTokensOut")),
        period_basis: command_code_text(value.get("periodBasis")),
    };
    (usage.total_count.is_some()
        || usage.completed_count.is_some()
        || usage.failed_count.is_some()
        || usage.success_rate.is_some()
        || usage.total_cost.is_some()
        || usage.total_credits.is_some()
        || usage.total_tokens_in.is_some()
        || usage.total_tokens_out.is_some())
    .then_some(usage)
}

fn command_code_plan_id(value: &Value, subscription: Option<&Value>) -> Option<String> {
    subscription
        .and_then(|item| item.pointer("/data/planId").or_else(|| item.get("planId")))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| {
            value
                .pointer("/credits/planId")
                .or_else(|| value.get("planId"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

fn command_code_plan_name(plan_id: Option<&str>) -> String {
    let normalized = plan_id.unwrap_or("").to_ascii_lowercase().replace('_', "-");
    if normalized == "individual-goat" {
        "GOAT".into()
    } else if normalized == "individual-provider" {
        "Provider".into()
    } else if normalized == "individual-go-v1" || normalized == "individual-go"
    {
        "Go".into()
    } else if normalized == "individual-pro-v1"
        || normalized == "individual-pro"
    {
        "Pro".into()
    } else if normalized == "individual-ultra" {
        "Ultra".into()
    } else if normalized == "individual-max" {
        "Max".into()
    } else if normalized == "teams-pro" {
        "Teams Pro".into()
    } else if let Some(plan_id) = plan_id.filter(|id| !id.is_empty()) {
        plan_id.to_owned()
    } else {
        "Coding Plan".into()
    }
}

fn command_code_plan_monthly(plan_id: Option<&str>) -> Option<f64> {
    let normalized = plan_id.unwrap_or("").to_ascii_lowercase().replace('_', "-");
    if normalized == "individual-goat" {
        Some(70.0)
    } else if normalized == "individual-provider" {
        Some(15.0)
    } else if normalized == "individual-go-v1" || normalized == "individual-go"
    {
        Some(10.0)
    } else if normalized == "individual-pro-v1" {
        Some(80.0)
    } else if normalized == "individual-pro" {
        Some(30.0)
    } else if normalized == "individual-ultra" {
        Some(300.0)
    } else if normalized == "individual-max" {
        Some(150.0)
    } else if normalized == "teams-pro" {
        Some(40.0)
    } else {
        None
    }
}

fn command_code_reset(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| {
        if let Some(text) = value.as_str() {
            if let Ok(time) = chrono::DateTime::parse_from_rfc3339(text) {
                return Some(time.to_rfc3339());
            }
        }
        parse_number(Some(value))
            .filter(|time| time.is_finite() && *time > 0.0 && *time < i64::MAX as f64)
            .and_then(|time| {
                if time < 100_000_000_000.0 {
                    chrono::DateTime::from_timestamp(time as i64, 0)
                } else {
                    chrono::DateTime::from_timestamp_millis(time as i64)
                }
            })
            .map(|time| time.to_rfc3339())
    })
}

fn parse_command_code_balance(
    value: &Value,
    subscription: Option<&Value>,
) -> Result<newapi::Balance, String> {
    let subscription = subscription
        .filter(|item| item.get("success").and_then(Value::as_bool) != Some(false));
    let credits = value.get("credits").and_then(Value::as_object);
    let plan_id = command_code_plan_id(value, subscription);
    let plan_name = command_code_plan_name(plan_id.as_deref());
    let monthly_total = command_code_plan_monthly(plan_id.as_deref());
    let mut entries = Vec::new();
    if let Some(remaining) = parse_number(credits.and_then(|item| item.get("monthlyCredits"))) {
        entries.push(newapi::BalanceEntry {
            label: Some(format!("Command Code · {plan_name}")),
            resets_at: command_code_reset(subscription.and_then(|item| {
                item.pointer("/data/currentPeriodEnd")
                    .or_else(|| item.get("currentPeriodEnd"))
            })),
            remaining,
            used: monthly_total.map(|total| (total - remaining).max(0.0)),
            total: monthly_total,
            unit: "credits".into(),
            valid: remaining > 0.0,
        });
    }
    for (label, key) in [("purchased", "purchasedCredits"), ("free", "freeCredits")] {
        if let Some(remaining) = parse_number(credits.and_then(|item| item.get(key))) {
            entries.push(newapi::BalanceEntry {
                label: Some(label.into()),
                resets_at: None,
                remaining,
                used: None,
                total: None,
                unit: "credits".into(),
                valid: remaining > 0.0,
            });
        }
    }
    if let Some(windows) = value.get("windowLimits").and_then(Value::as_object) {
        for (label, key) in [("fiveHour", "fiveHour"), ("weekly", "weekly")] {
            let Some(window) = windows.get(key).and_then(Value::as_object) else {
                continue;
            };
            // Missing fields are unknown; a zero cap is uncapped, not an
            // exhausted zero-balance window. Neither is a remaining quota.
            let (Some(used), Some(cap)) = (
                parse_number(window.get("used")),
                parse_number(window.get("cap")),
            ) else {
                continue;
            };
            if used < 0.0 || cap <= 0.0 {
                continue;
            }
            entries.push(newapi::BalanceEntry {
                label: Some(label.into()),
                resets_at: command_code_reset(window.get("resetAt")),
                remaining: (cap - used).max(0.0),
                used: Some(used),
                total: Some(cap),
                unit: "credits".into(),
                valid: window.get("exceeded").and_then(Value::as_bool) != Some(true)
                    && (cap <= 0.0 || used < cap),
            });
        }
    }
    let mut balance = balance_from_entries(entries)?;
    let subscription_data = subscription
        .map(|item| item.get("data").unwrap_or(item))
        .and_then(Value::as_object);
    balance.command_code = Some(newapi::CommandCodeBalanceDetails {
        account: None,
        plan: (plan_id.is_some() || subscription_data.is_some()).then_some(newapi::CommandCodeBalancePlan {
            name: plan_id.as_ref().map(|_| plan_name),
            id: plan_id,
            status: command_code_text(subscription_data.and_then(|item| item.get("status"))),
            current_period_start: command_code_reset(subscription_data.and_then(|item| item.get("currentPeriodStart"))),
            current_period_end: command_code_reset(subscription_data.and_then(|item| item.get("currentPeriodEnd"))),
        }),
        usage: None,
    });
    Ok(balance)
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
                    resets_at: window
                        .get("resetsAt")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
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
        command_code: None,
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
        assert_eq!(
            go.entries[0].resets_at.as_deref(),
            Some("2030-01-01T00:00:00Z")
        );
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

    fn local_mock_network() -> ApiProxyConfigPayload {
        // Local fixtures must not depend on the user's Windows system proxy.
        ApiProxyConfigPayload { mode: super::super::models::ApiProxyMode::Direct, url: None }
    }

    fn accept_mock_connection(listener: &std::net::TcpListener) -> std::net::TcpStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => { stream.set_nonblocking(false).unwrap(); return stream; }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "missing request to local balance fixture");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("local balance fixture accept failed: {error}"),
            }
        }
    }

    fn mock_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let mut stream = accept_mock_connection(&listener);
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

    // The summary and identity reads overlap; fixtures must dispatch by path,
    // not accidentally depend on which request reaches the server first.
    fn mock_command_code_server(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            for _ in 0..responses.len() {
                let mut stream = accept_mock_connection(&listener);
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut bytes = [0u8; 16 * 1024];
                let count = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]).to_string();
                let path = request.split_whitespace().nth(1).unwrap();
                let (_, status, body) = responses.iter().find(|(expected, _, _)| *expected == path)
                    .unwrap_or_else(|| panic!("Unexpected mock endpoint: {path}"));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                captured.lock().unwrap().push(request);
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
    fn parses_command_code_plan_and_quota_windows() {
        let balance = parse_command_code_balance(
            &json!({
                "planId": "individual-pro-v1",
                "credits": {
                    "monthlyCredits": 62.5,
                    "purchasedCredits": 4,
                    "freeCredits": 1
                },
                "windowLimits": {
                    "fiveHour": {"used": 30, "cap": 100, "exceeded": false, "resetAt": 1_900_000_000_000_i64},
                    "weekly": {"used": 120, "cap": 200, "exceeded": false, "resetAt": 1_900_100_000_000_i64}
                }
            }),
            Some(&json!({"currentPeriodEnd": 1_900_200_000_000_i64})),
        )
        .unwrap();
        assert_eq!(balance.entries.len(), 5);
        assert_eq!(
            balance.entries[0].label.as_deref(),
            Some("Command Code · Pro")
        );
        assert_eq!(balance.entries[0].remaining, 62.5);
        assert_eq!(balance.entries[0].total, Some(80.0));
        assert_eq!(balance.entries[1].label.as_deref(), Some("purchased"));
        assert_eq!(balance.entries[3].label.as_deref(), Some("fiveHour"));
        assert_eq!(balance.entries[3].remaining, 70.0);
        assert_eq!(balance.entries[4].label.as_deref(), Some("weekly"));
        assert_eq!(balance.entries[4].remaining, 80.0);
    }

    #[test]
    fn command_code_known_plans_missing_windows_and_dates_are_distinct() {
        for (id, total) in [
            ("individual-go", 10.0),
            ("individual-go-v1", 10.0),
            ("individual-goat", 70.0),
            ("individual-pro", 30.0),
            ("individual-pro-v1", 80.0),
            ("individual-provider", 15.0),
            ("individual-max", 150.0),
            ("individual-ultra", 300.0),
            ("teams-pro", 40.0),
        ] {
            assert_eq!(command_code_plan_monthly(Some(id)), Some(total), "{id}");
        }
        assert_eq!(
            command_code_plan_name(Some("individual-provider")),
            "Provider"
        );
        let balance = parse_command_code_balance(&json!({"credits": {"planId": "individual-pro", "monthlyCredits": 0},
            "windowLimits": {"fiveHour": {"cap": 10}, "weekly": {"cap": 0, "used": 4}}}),
            Some(&json!({"data": {"planId": "individual-provider", "currentPeriodEnd": "2030-03-17T00:00:00Z"}}))).unwrap();
        assert_eq!(
            balance.entries.len(),
            1,
            "Missing or unlimited windows must not become exhausted balances"
        );
        assert_eq!(balance.remaining, 0.0);
        assert_eq!(balance.entries[0].total, Some(15.0));
        assert!(balance.entries[0]
            .resets_at
            .as_ref()
            .unwrap()
            .starts_with("2030-03-17"));
        assert!(command_code_reset(Some(&json!(0))).is_none());
        assert!(command_code_reset(Some(&json!("invalid"))).is_none());
        assert!(parse_command_code_balance(&json!({"credits":{}}), None).is_err());
        let window_only = parse_command_code_balance(
            &json!({"windowLimits":{"weekly":{"cap":10,"used":3}}}),
            None,
        )
        .unwrap();
        assert_eq!(window_only.remaining, 7.0);
        let unknown = parse_command_code_balance(
            &json!({"credits":{"planId":"future-tier","monthlyCredits":4}}),
            None,
        )
        .unwrap();
        assert_eq!(unknown.entries[0].total, None);
    }

    #[test]
    fn command_code_billing_uses_root_paths_model_key_and_org_scope() {
        let (base_url, requests, server) = mock_command_code_server(vec![
            ("/alpha/billing/credits", "200 OK", r#"{"credits":{"monthlyCredits":5}}"#),
            ("/alpha/whoami", "200 OK", r#"{"user":{"name":"Example","userName":"example-user","apiKey":"response-secret-must-not-leak"},"org":{"id":"team one"}}"#),
            ("/alpha/billing/subscriptions?orgId=team+one", "200 OK", r#"{"data":{"planId":"teams-pro","status":"active","currentPeriodStart":"2030-01-01T00:00:00Z","currentPeriodEnd":"2030-02-01T00:00:00Z"}}"#),
            ("/alpha/usage/summary", "200 OK", r#"{"totalCount":2160,"completedCount":2159,"failedCount":1,"totalCost":2.8453,"totalCredits":2.85,"successRate":99.95,"totalTokensIn":124800000,"totalTokensOut":1100000,"periodBasis":"billing-period","apiKey":"usage-secret-must-not-leak"}"#),
        ]);
        let balance = command_code_balance(
            &newapi::Connection {
                base_url: format!("{base_url}provider/v1/"),
                api_key: "synthetic-model-key".into(),
                access_token: "account-token-must-not-be-used".into(),
                ..Default::default()
            },
            &local_mock_network(),
        )
        .unwrap();
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /alpha/billing/credits "));
        assert!(requests.iter().any(|request| request.starts_with("GET /alpha/whoami ")));
        assert!(requests.iter().any(|request| request.starts_with("GET /alpha/usage/summary ")));
        assert!(requests.iter().any(|request| request.starts_with("GET /alpha/billing/subscriptions?orgId=team+one ")));
        for request in requests.iter() {
            assert!(request.contains("Bearer synthetic-model-key"));
            assert!(!request.contains("account-token-must-not-be-used"));
        }
        assert_eq!(balance.entries[0].total, Some(40.0));
        assert_eq!(
            balance.entries[0].label.as_deref(),
            Some("Command Code · Teams Pro")
        );
        let details = balance.command_code.as_ref().unwrap();
        let account = details.account.as_ref().unwrap();
        assert_eq!(account.name.as_deref(), Some("Example"));
        assert_eq!(account.user_name.as_deref(), Some("example-user"));
        let plan = details.plan.as_ref().unwrap();
        assert_eq!(plan.status.as_deref(), Some("active"));
        assert!(plan.current_period_start.as_ref().unwrap().starts_with("2030-01-01"));
        assert!(plan.current_period_end.as_ref().unwrap().starts_with("2030-02-01"));
        let usage = details.usage.as_ref().unwrap();
        assert_eq!(balance.request_count, Some(2160));
        assert_eq!(usage.completed_count, Some(2159));
        assert_eq!(usage.failed_count, Some(1));
        assert_eq!(usage.success_rate, Some(99.95));
        assert_eq!(usage.total_cost, Some(2.8453));
        assert_eq!(usage.total_credits, Some(2.85));
        assert_eq!(usage.total_tokens_in, Some(124_800_000));
        assert_eq!(usage.total_tokens_out, Some(1_100_000));
        assert_eq!(usage.period_basis.as_deref(), Some("billing-period"));
        assert_eq!(balance.unit, "credits");
        let serialized = serde_json::to_string(&balance).unwrap();
        assert!(!serialized.contains("secret-must-not-leak"));
        assert!(!serialized.contains("apiKey"));
        let missing_key = newapi::Connection {
            base_url: "https://api.commandcode.ai/provider/v1".into(),
            access_token: "must-not-be-used".into(),
            ..Default::default()
        };
        assert_eq!(
            query(&missing_key, &local_mock_network()).unwrap_err(),
            "newapi.errors.apiKeyRequired"
        );
    }

    #[test]
    fn command_code_auxiliary_failures_keep_credits_without_invented_usage() {
        for (status, body) in [
            ("404 Not Found", "{}"),
            ("503 Service Unavailable", "{}"),
            ("200 OK", "not valid JSON"),
            ("200 OK", "{}"),
            ("200 OK", r#"{"success":false,"totalCount":999}"#),
        ] {
            let (base_url, requests, server) = mock_command_code_server(vec![
                ("/alpha/billing/credits", "200 OK", r#"{"credits":{"planId":"individual-goat","monthlyCredits":58.5,"purchasedCredits":0,"freeCredits":0}}"#),
                ("/alpha/whoami", "401 Unauthorized", "{}"),
                ("/alpha/billing/subscriptions", "200 OK", r#"{"success":false,"data":{"planId":"individual-pro","currentPeriodEnd":"2030-01-01T00:00:00Z"}}"#),
                ("/alpha/usage/summary", status, body),
            ]);
            let balance = command_code_balance(
                &newapi::Connection {
                    base_url,
                    api_key: "synthetic-model-key".into(),
                    ..Default::default()
                },
                &local_mock_network(),
            ).unwrap();
            server.join().unwrap();
            assert_eq!(requests.lock().unwrap().len(), 4);
            assert_eq!(balance.remaining, 58.5);
            assert_eq!(balance.entries[0].total, Some(70.0));
            assert_eq!(balance.entries[0].resets_at, None);
            assert_eq!(balance.entries[1].remaining, 0.0);
            assert_eq!(balance.entries[2].remaining, 0.0);
            assert_eq!(balance.request_count, None);
            let details = balance.command_code.unwrap();
            assert!(details.account.is_none());
            assert!(details.usage.is_none());
            assert_eq!(details.plan.unwrap().name.as_deref(), Some("GOAT"));
        }
    }

    #[test]
    fn command_code_usage_preserves_zero_and_keeps_missing_or_malformed_fields_unknown() {
        for value in [json!(null), json!([]), json!({}), json!({"periodBasis":"billing-period"})] {
            assert!(parse_command_code_usage(Some(&value)).is_none());
        }
        let usage = parse_command_code_usage(Some(&json!({
            "totalCount": -1,
            "completedCount": 0.0,
            "failedCount": "0",
            "successRate": 0,
            "totalCost": "NaN",
            "totalCredits": 0,
            "totalTokensIn": 1.5,
            "totalTokensOut": "invalid",
            "periodBasis": "billing-period"
        }))).unwrap();
        assert_eq!(usage.total_count, None);
        assert_eq!(usage.completed_count, Some(0));
        assert_eq!(usage.failed_count, Some(0));
        assert_eq!(usage.success_rate, Some(0.0));
        assert_eq!(usage.total_cost, None);
        assert_eq!(usage.total_credits, Some(0.0));
        assert_eq!(usage.total_tokens_in, None);
        assert_eq!(usage.total_tokens_out, None);
        assert_eq!(usage.period_basis.as_deref(), Some("billing-period"));
        let serialized = serde_json::to_value(&usage).unwrap();
        assert_eq!(serialized["totalCount"], Value::Null);
        assert_eq!(serialized["completedCount"], 0);
        for rate in [-1.0, 100.1] {
            let usage = parse_command_code_usage(Some(&json!({"totalCount": 3, "successRate": rate}))).unwrap();
            assert_eq!(usage.success_rate, None);
        }
        for count in [json!(9_007_199_254_740_992_u64), json!("9007199254740992"), json!(9_007_199_254_740_992.0)] {
            assert_eq!(command_code_count(Some(&count)), None);
        }
        for count in [json!(9_007_199_254_740_991_u64), json!("9007199254740991")] {
            assert_eq!(command_code_count(Some(&count)), Some(9_007_199_254_740_991));
        }
        let account = parse_command_code_account(Some(&json!({
            "user":{"name":12,"userName":"valid-user","token":"must-not-leak"}
        }))).unwrap();
        assert_eq!(account.name, None);
        assert_eq!(account.user_name.as_deref(), Some("valid-user"));
        assert!(!serde_json::to_string(&account).unwrap().contains("must-not-leak"));
    }

    #[test]
    fn command_code_credit_failure_still_fails_before_auxiliary_requests() {
        let (base_url, requests, server) = mock_server(vec![("401 Unauthorized", "{}")]);
        assert!(command_code_balance(
            &newapi::Connection { base_url, api_key: "synthetic-model-key".into(), ..Default::default() },
            &local_mock_network(),
        ).is_err());
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /alpha/billing/credits "));
    }

    #[test]
    fn command_code_details_are_optional_in_saved_generic_balances() {
        let legacy = json!({
            "entries":[{"remaining":5,"used":null,"total":null,"unit":"USD","valid":true}],
            "remaining":5,"used":null,"unit":"USD","rawQuota":5,
            "quotaPerUnit":null,"requestCount":null,"checkedAt":1,"conversionAvailable":true
        });
        let balance: newapi::Balance = serde_json::from_value(legacy.clone()).unwrap();
        assert!(balance.command_code.is_none());
        assert!(serde_json::to_value(&balance).unwrap().get("commandCode").is_none());
        let mut old_details = legacy;
        old_details["commandCode"] = json!({"usage":{"completedCount":1}});
        let balance: newapi::Balance = serde_json::from_value(old_details).unwrap();
        let details = balance.command_code.unwrap();
        assert!(details.account.is_none());
        assert!(details.plan.is_none());
        let usage = details.usage.unwrap();
        assert_eq!(usage.completed_count, Some(1));
        assert_eq!(usage.total_cost, None);
        assert_eq!(usage.success_rate, None);
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
            "https://api.commandcode.ai/unrelated/v1",
            "https://api.xiaomimimo.com/v1",
            "https://token-plan-cn.xiaomimimo.com/v1",
        ] {
            let connection = newapi::Connection {
                base_url: base_url.into(),
                user_id: Some(37),
                access_token: "account-token-must-not-be-used".into(),
                api_key: "model-key-must-not-be-used".into(),
                ..Default::default()
            };
            assert_eq!(
                query(&connection, &local_mock_network()).unwrap_err(),
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
        let (balance, source) = query(&connection, &local_mock_network()).unwrap();
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
        let (balance, source) = query(&connection, &local_mock_network()).unwrap();
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
            query(&connection, &local_mock_network()).unwrap_err(),
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
            &local_mock_network(),
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
    #[test]
    fn unknown_plan_versions_and_epoch_units_are_safe() {
        for id in ["individual-pro-v2", "individual-go-v2", "individual-goat-plus", "teams-pro-new"] {
            assert_eq!(command_code_plan_monthly(Some(id)), None);
            assert_eq!(command_code_plan_name(Some(id)), id);
        }
        assert_eq!(command_code_reset(Some(&json!(1900000000))), command_code_reset(Some(&json!(1900000000000i64))));
        assert!(command_code_reset(Some(&json!(1900000000))).unwrap().starts_with("2030-"));
    }

}
