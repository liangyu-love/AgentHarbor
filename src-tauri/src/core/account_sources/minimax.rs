//! MiniMax Code (mcode): the credits a MiniMax Code account gets (the daily
//! check-in's among them) and its M Plan, on MiniMax Code's own models. One
//! source for each site: China (minimax.cn) and international (minimax.io).
//!
//! Ported from magpie's community plugin and check-in (MIT, see
//! LICENSES/THIRD_PARTY.md): an account signs in with MiniMax Code's device
//! sign-in (OAuth 2 device flow with PKCE, client `mcode-public`); nothing is
//! read from or written to MiniMax Code's own sign-in (`~/.minimax`). Chats
//! are Anthropic messages at `<agent>/mavis/api/v1/llm/v1/messages`, sent with
//! MiniMax Code's headers. Its account API is signed as mcode signs it.
use super::{
    checkin::{Checkin, Outcome},
    oauth,
    store::{Quota, SourceModel},
    SourceKind,
};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const CLIENT_ID: &str = "mcode-public";
const SCOPE: &str = "agent.default";
const AUDIENCE: &str = "agent-backend";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// MiniMax Code's request signing (`x-signature`).
const SIGN_SECRET: &str = "I*7Cf%WZ#S&%1RlZJ&C2";
/// The key the Anthropic SDK is given in MiniMax Code: a placeholder, the
/// account going in Authorization.
const PLACEHOLDER_KEY: &str = "sk-xxx";
/// When the device code gives no end of its own.
const SIGN_IN_WINDOW: Duration = Duration::from_secs(600);

/// What the user should know before signing in, as the plugin's README says.
pub const RISK: &str = "MiniMax gives its credits for use in MiniMax Code; using them from other agents is not approved by MiniMax, which may limit, suspend or close an account used so.";

#[derive(Clone, Debug)]
pub struct Site {
    pub kind: SourceKind,
    region: &'static str,
    lang: &'static str,
    /// OAuth.
    account: String,
    /// Chats, the model list and the check-in.
    pub llm: String,
    /// The account and its membership.
    agent: String,
    /// The M Plan's windows.
    platform: String,
}

pub fn site(kind: SourceKind) -> Site {
    let global = kind == SourceKind::MinimaxCodeGlobal;
    let host = |cn: &str, io: &str| super::endpoint_override(kind).unwrap_or_else(|| (if global { io } else { cn }).to_owned());
    Site {
        kind,
        region: if global { "en" } else { "cn" },
        lang: if global { "en" } else { "zh" },
        account: host("https://account.minimax.cn", "https://account.minimax.io"),
        llm: host("https://agent.minimax.cn", "https://agent.minimax.io"),
        agent: host("https://agent.minimaxi.com", "https://agent.minimax.io"),
        platform: host("https://www.minimaxi.com", "https://platform.minimax.io"),
    }
}

impl Site {
    pub fn api(&self) -> String {
        format!("{}/mavis/api/v1/llm/v1", self.llm.trim_end_matches('/'))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub access: String,
    pub refresh: String,
    /// Unix milliseconds.
    pub expires_at: i64,
    /// The MiniMax user id (`realUserID`).
    pub uid: String,
    pub email: String,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Creds {
    /// Lapses within two minutes.
    pub fn near_expiry(&self) -> bool {
        self.expires_at != 0 && now_ms() >= self.expires_at - 120_000
    }
}

/// Read a token reply; `refresh` is kept when none is sent.
fn token(body: &Value, refresh: &str) -> Option<Creds> {
    let access = body["access_token"].as_str().filter(|token| !token.is_empty())?;
    let expires_in = body["expires_in"].as_f64().filter(|seconds| *seconds > 0.0)?;
    Some(Creds {
        access: access.to_owned(),
        refresh: body["refresh_token"].as_str().filter(|token| !token.is_empty()).unwrap_or(refresh).to_owned(),
        expires_at: now_ms() + (expires_in * 1000.0) as i64,
        ..Default::default()
    })
}

pub struct DeviceStart {
    verifier: String,
    pub user_code: String,
    /// What the token endpoint is polled by: `device_code`, or MiniMax's own
    /// variant, the user code.
    poll: (&'static str, String),
    pub url: String,
    deadline: Instant,
    interval: Duration,
}

/// Ask MiniMax for a device code and the page to confirm it at.
pub async fn begin_sign_in(client: &Client, site: &Site) -> Result<DeviceStart, String> {
    let (verifier, challenge) = oauth::pkce();
    let reply = oauth::post_form(client, &format!("{}/oauth2/device/code", site.account), &[
        ("client_id", CLIENT_ID), ("scope", SCOPE), ("audience", AUDIENCE),
        ("code_challenge", &challenge), ("code_challenge_method", "S256"),
    ]).await.map_err(|error| format!("MiniMax sign-in: {error}"))?;
    if !reply.ok() {
        return Err(format!("MiniMax sign-in: {}", reply.error().unwrap_or("refused")));
    }
    let body = &reply.body;
    let text = |key: &str| body[key].as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned);
    let user_code = text("user_code").ok_or("MiniMax gave no sign-in code")?;
    let device_code = text("device_code");
    let expired_in = body["expired_in"].as_f64();
    // MiniMax's own variant: no device_code, the user code polled for, an
    // end that may be an at-ms, an interval in ms.
    let by_user = device_code.is_none() && expired_in.is_some();
    let uri = text("verification_uri_complete").or_else(|| text("verification_uri")).or_else(|| text("verification_url"))
        .ok_or("MiniMax gave no sign-in page")?;
    let at = expired_in.map(|value| if value < 1e12 { value } else { ((value - now_ms() as f64) / 1000.0).ceil() });
    let expires_in = body["expires_in"].as_f64().filter(|value| *value > 0.0).or(at.filter(|value| *value > 0.0));
    let interval = body["interval"].as_f64().filter(|value| *value > 0.0);
    let poll = match device_code {
        Some(code) => ("device_code", code),
        None if by_user => ("user_code", user_code.clone()),
        None => return Err("MiniMax gave no sign-in code".into()),
    };
    Ok(DeviceStart {
        verifier,
        user_code,
        poll,
        url: uri,
        deadline: Instant::now() + expires_in.map_or(SIGN_IN_WINDOW, |seconds| Duration::from_secs_f64(seconds)),
        interval: match interval {
            Some(ms) if by_user => Duration::from_millis(ms as u64),
            Some(seconds) => Duration::from_secs_f64(seconds),
            None => Duration::from_secs(5),
        },
    })
}

pub struct SignedIn {
    pub creds: Creds,
    pub name: String,
}

/// Poll until the code is confirmed, then read who signed in.
pub async fn finish_sign_in(client: &Client, site: &Site, start: DeviceStart, mut cancel: watch::Receiver<bool>, floor: Duration) -> Result<SignedIn, String> {
    let mut wait = start.interval.max(floor);
    let creds = loop {
        tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            _ = tokio::time::sleep(wait) => {}
        }
        if Instant::now() >= start.deadline {
            return Err("accounts.loginTimeout".into());
        }
        let Ok(reply) = oauth::post_form(client, &format!("{}/oauth2/token", site.account), &[
            ("grant_type", DEVICE_GRANT), (start.poll.0, &start.poll.1), ("client_id", CLIENT_ID), ("code_verifier", &start.verifier),
        ]).await else { continue };
        match (reply.ok(), reply.body["status"].as_str().unwrap_or_default(), reply.error().unwrap_or_default()) {
            (true, "pending", _) | (false, _, "authorization_pending") => continue,
            (true, "slow_down", _) | (false, _, "slow_down") => wait += Duration::from_secs(5),
            (true, "denied" | "access_denied", _) | (false, _, "access_denied") => return Err("accounts.loginCancelled".into()),
            (true, "expired" | "expired_token", _) | (false, _, "expired_token") => return Err("accounts.loginTimeout".into()),
            (true, _, _) => break token(&reply.body, "").ok_or("MiniMax gave no token")?,
            (false, _, error) => return Err(format!("MiniMax sign-in: {}", if error.is_empty() { "refused" } else { error })),
        }
    };
    let mut creds = creds;
    let claims = oauth::jwt_claims(&creds.access);
    let (uid, email, name) = identity(client, site, &creds.access).await.unwrap_or_default();
    creds.uid = [uid, claims["account_id"].as_str().unwrap_or_default().to_owned(), claims["sub"].as_str().unwrap_or_default().to_owned()]
        .into_iter().find(|id| !id.is_empty()).unwrap_or_default();
    if creds.uid.is_empty() {
        return Err("MiniMax gave no account".into());
    }
    creds.email = email.clone();
    let name = [email, name].into_iter().find(|text| !text.is_empty()).unwrap_or_else(|| creds.uid.clone());
    Ok(SignedIn { creds, name })
}

pub enum RefreshError {
    SignedOut(String),
    Transient,
}

/// Trade the refresh token for new tokens. MiniMax spends a refresh token
/// once: the caller serialises refreshes so none is sent twice.
pub async fn refresh(client: &Client, site: &Site, creds: &Creds) -> Result<Creds, RefreshError> {
    if creds.refresh.is_empty() {
        return Err(RefreshError::SignedOut("no refresh token".into()));
    }
    let reply = oauth::post_form(client, &format!("{}/oauth2/token", site.account), &[
        ("grant_type", "refresh_token"), ("refresh_token", &creds.refresh), ("client_id", CLIENT_ID), ("scope", SCOPE), ("audience", AUDIENCE),
    ]).await.map_err(|_| RefreshError::Transient)?;
    if reply.ok() {
        let next = token(&reply.body, &creds.refresh).ok_or(RefreshError::Transient)?;
        return Ok(Creds { uid: creds.uid.clone(), email: creds.email.clone(), ..next });
    }
    // Only invalid_grant with HTTP 400 signs the account out, as MiniMax Code reads it.
    if reply.status == 400 && reply.error() == Some("invalid_grant") {
        return Err(RefreshError::SignedOut("MiniMax refused the refresh token".into()));
    }
    Err(RefreshError::Transient)
}

/// The headers MiniMax Code's chats carry.
pub fn request_headers(creds: &Creds) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", creds.access)),
        ("x-api-key", PLACEHOLDER_KEY.to_owned()),
        ("user-agent", "MiniMaxAgent".to_owned()),
        ("x-mavis-agent-id", "main".to_owned()),
        ("x-mavis-timezone-offset", oauth::utc_offset_seconds().to_string()),
        ("x-mavis-session-id", session().to_owned()),
        ("anthropic-version", "2023-06-01".to_owned()),
    ]
}

/// One conversation id for the run's requests, as the plugin keeps one for
/// requests that come with none of their own.
fn session() -> &'static str {
    static SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// A request of MiniMax Code's account API, signed as mcode signs one: `yy`
/// over the path and query, the body (`{}` for a GET) and the time;
/// `x-signature` over the time (and a POST's body).
fn signed(client: &Client, site: &Site, path: &str, access: &str, uid: &str, body: Option<&Value>) -> reqwest::RequestBuilder {
    let now = now_ms();
    let query: Vec<(&str, String)> = vec![
        ("device_platform", "mcode".into()), ("biz_id", "3".into()), ("app_id", "3001".into()), ("version_code", "22201".into()),
        ("unix", now.to_string()), ("timezone_offset", oauth::utc_offset_seconds().to_string()), ("sys_language", site.lang.into()),
        ("lang", site.lang.into()), ("device_id", "0".into()), ("os_name", oauth::node_platform().into()), ("browser_name", "mcode".into()),
        ("user_id", if uid.trim().is_empty() { "0".into() } else { uid.trim().to_owned() }), ("client", "mcode".into()),
    ];
    let mut url = reqwest::Url::parse(&format!("{}{path}", site.agent.trim_end_matches('/'))).expect("an agent URL");
    url.query_pairs_mut().extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
    let at = format!("{}?{}", url.path(), url.query().unwrap_or_default());
    let seconds = now / 1000;
    let json = body.map(Value::to_string);
    let stamp = oauth::md5_hex(&now.to_string());
    let yy = oauth::md5_hex(&format!("{}_{}{stamp}ooui", oauth::encode_uri_component(&at), json.as_deref().unwrap_or("{}")));
    let signature = oauth::md5_hex(&format!("{seconds}{SIGN_SECRET}{}", json.as_deref().unwrap_or_default()));
    let request = client.request(if json.is_some() { Method::POST } else { Method::GET }, url)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("user-agent", "MiniMaxCode")
        .header("authorization", format!("Bearer {access}"))
        .header("yy", yy)
        .header("x-timestamp", seconds.to_string())
        .header("x-signature", signature)
        .timeout(Duration::from_secs(15));
    match json {
        Some(json) => request.body(json),
        None => request,
    }
}

/// A signed call's answer, refused when its body says it failed.
async fn account_call(request: reqwest::RequestBuilder, what: &str) -> Result<Value, String> {
    let response = request.send().await.map_err(|_| format!("{what}: cannot reach MiniMax"))?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!("{what}: the sign-in was refused (HTTP {status})"));
    }
    if !(200..300).contains(&status) {
        return Err(format!("{what}: HTTP {status}"));
    }
    let body: Value = response.json().await.map_err(|_| format!("{what}: not JSON"))?;
    if let Some(code) = body["statusInfo"]["code"].as_i64().filter(|code| *code != 0) {
        let message = body["statusInfo"]["message"].as_str().or(body["statusInfo"]["msg"].as_str()).unwrap_or_default();
        return Err(format!("{what}: {}", if message.is_empty() { format!("status {code}") } else { message.to_owned() }));
    }
    if let Some(code) = body["base_resp"]["status_code"].as_i64().filter(|code| *code != 0) {
        return Err(format!("{what}: {}", body["base_resp"]["status_msg"].as_str().map_or_else(|| format!("status {code}"), str::to_owned)));
    }
    Ok(body)
}

/// Who the account is: its MiniMax user id, email and name.
async fn identity(client: &Client, site: &Site, access: &str) -> Result<(String, String, String), String> {
    let body = account_call(signed(client, site, "/v1/api/user/info", access, "", None), "account").await?;
    let user = [&body["data"]["userInfo"], &body["data"]["user_info"], &body["userInfo"], &body["user_info"]].into_iter()
        .find(|user| user.is_object()).cloned().unwrap_or(Value::Null);
    let pick = |keys: &[&str]| keys.iter().filter_map(|key| user[*key].as_str()).map(str::trim).find(|text| !text.is_empty()).unwrap_or_default().to_owned();
    let id = pick(&["realUserID", "real_user_id"]);
    if id.is_empty() {
        return Err("account: no user id in the reply".into());
    }
    Ok((id, pick(&["userEmail", "email", "userMail", "user_email"]), pick(&["name", "userName", "user_name"])))
}

#[derive(Default, Debug, PartialEq)]
struct Membership {
    has_plan: Option<bool>,
    group: Option<String>,
    tier: Option<String>,
    balance: Option<f64>,
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// A membership answer (or a workspace) as mcode reads it.
fn membership_of(entry: &Value) -> Membership {
    let pick = |key: &str| if entry.get(key).is_some() { &entry[key] } else { &entry["data"][key] };
    let text = |key: &str| pick(key).as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned);
    let summary = if entry["op_credit_summary"].is_object() { &entry["op_credit_summary"] } else { &entry["data"]["op_credit_summary"] };
    Membership {
        has_plan: pick("has_token_plan").as_bool(),
        group: text("op_group_id"),
        tier: text("token_plan_tier"),
        balance: number(&summary["total_remaining_amount"]).or_else(|| number(pick("opcredit_balance"))),
    }
}

/// The account's own workspace's plan and credits.
async fn membership(client: &Client, site: &Site, creds: &Creds) -> Result<Membership, String> {
    let call = |path: &str, body: Value| signed(client, site, path, &creds.access, &creds.uid, Some(&body));
    let workspaces = match account_call(call("/matrix/api/v1/user/get_user_extra_info", json!({})), "workspace").await {
        Ok(body) => body,
        Err(error) if error.contains("sign-in was refused") => return Err(error),
        Err(_) => return Ok(membership_of(&account_call(call("/matrix/api/v1/commerce/get_membership_info", json!({})), "membership").await?)),
    };
    let list = workspaces["workspaces"].as_array().or(workspaces["data"]["workspaces"].as_array()).cloned().unwrap_or_default();
    let Some(own) = list.iter().find(|workspace| number(&workspace["workspace_type"]) == Some(0.0) && !workspace["workspace_id"].is_null()) else {
        return Ok(Membership::default());
    };
    let from_workspace = membership_of(own);
    match account_call(call("/matrix/api/v1/commerce/get_membership_info", json!({"workspace_id": own["workspace_id"]})), "membership").await {
        Ok(body) => {
            let from_api = membership_of(&body);
            Ok(Membership {
                has_plan: if from_workspace.has_plan == Some(true) || from_api.has_plan == Some(true) { Some(true) } else { from_api.has_plan.or(from_workspace.has_plan) },
                group: from_workspace.group.or(from_api.group),
                tier: from_api.tier.or(from_workspace.tier),
                balance: from_api.balance.or(from_workspace.balance),
            })
        }
        Err(error) if error.contains("sign-in was refused") => Err(error),
        Err(_) => Ok(from_workspace),
    }
}

/// The M Plan's tightest window, as the share of it used: the general
/// group's interval and week.
fn plan_used(body: &Value) -> Option<f64> {
    let mut tightest: Option<f64> = None;
    for group in body["model_remains"].as_array().into_iter().flatten() {
        if !group["model_name"].as_str().is_some_and(|name| name.trim().eq_ignore_ascii_case("general")) {
            continue;
        }
        for (left, status) in [("current_interval_remaining_percent", "current_interval_status"), ("current_weekly_remaining_percent", "current_weekly_status")] {
            let status = number(&group[status]).unwrap_or(0.0);
            let used = match number(&group[left]) {
                _ if status == 3.0 => continue,
                _ if status == 2.0 => 100.0,
                Some(left) => (100.0 - left).clamp(0.0, 100.0),
                None => continue,
            };
            tightest = Some(tightest.map_or(used, |was: f64| was.max(used)));
        }
    }
    tightest
}

/// The account's plan (the M Plan's tier, else Free), its credits left, and
/// how much of the M Plan's tightest window is used.
pub async fn quota(client: &Client, site: &Site, creds: &Creds) -> (Quota, Option<String>) {
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), unit: "credits".into(), ..Default::default() };
    let member = match membership(client, site, creds).await {
        Ok(member) => member,
        Err(error) => {
            quota.error = Some(error);
            return (quota, None);
        }
    };
    let on_plan = member.has_plan == Some(true);
    let plan = if on_plan { Some(member.tier.as_ref().map_or_else(|| "M Plan".to_owned(), |tier| format!("M Plan {tier}"))) } else { Some("Free".to_owned()) };
    quota.paid = Some(on_plan);
    quota.remaining = member.balance;
    if let (true, Some(group)) = (on_plan, &member.group) {
        let reply = client.get(format!("{}/v1/api/openplatform/coding_plan/remains", site.platform))
            .header("accept", "application/json").header("authorization", format!("Bearer {}", creds.access)).header("x-group-id", group)
            .timeout(Duration::from_secs(15)).send().await;
        match reply {
            Ok(response) if response.status().is_success() => {
                if let Some(used) = response.json::<Value>().await.ok().as_ref().and_then(plan_used) {
                    quota.unit = "percent".into();
                    quota.used = used;
                    quota.limit = 100.0;
                }
            }
            Ok(response) => quota.error = Some(format!("M Plan: HTTP {}", response.status().as_u16())),
            Err(_) => quota.error = Some("M Plan: cannot reach MiniMax".into()),
        }
    }
    (quota, plan)
}

/// MiniMax Code's model config, as mcode asks it at start: the account's
/// first, then, when that is refused, no one's.
pub async fn fetch_models(client: &Client, site: &Site, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let url = format!("{}/mavis/api/v1/models?region={}&buildEnv=prod", site.llm.trim_end_matches('/'), site.region);
    let ask = |auth: Option<&str>| {
        let request = client.get(&url).header("accept", "application/json").timeout(Duration::from_secs(10));
        match auth {
            Some(token) => request.header("authorization", format!("Bearer {token}")),
            None => request,
        }
    };
    let mut response = ask(Some(&creds.access)).send().await.map_err(|_| "cannot reach MiniMax".to_owned())?;
    if matches!(response.status().as_u16(), 401 | 403) {
        response = ask(None).send().await.map_err(|_| "cannot reach MiniMax".to_owned())?;
    }
    if !response.status().is_success() {
        return Err(format!("MiniMax's models: HTTP {}", response.status().as_u16()));
    }
    let models = parse_models(&response.json().await.unwrap_or(Value::Null));
    Ok(if models.is_empty() { fallback_models() } else { models })
}

/// A model's levels as the router asks them: `none` where thinking can be
/// turned off, its efforts where it takes one, else `high` for on.
fn levels(mode: &str, efforts: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if mode == "switchable" {
        out.push("none".to_owned());
    }
    if !efforts.is_empty() {
        out.extend(efforts.iter().cloned());
    } else if mode == "switchable" {
        out.push("high".to_owned());
    }
    out
}

/// MiniMax Code's model config (`providers[minimax].config.models`).
fn parse_models(body: &Value) -> Vec<SourceModel> {
    let Some(provider) = body["providers"].as_array().into_iter().flatten().find(|provider| provider["providerId"] == "minimax") else {
        return Vec::new();
    };
    let Some(models) = provider["config"]["models"].as_object() else { return Vec::new() };
    let mut ids: Vec<&String> = provider["config"]["model_order"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).filter_map(|id| models.keys().find(|key| *key == id)).collect();
    ids.extend(models.keys().filter(|key| !ids.contains(key)).collect::<Vec<_>>());
    ids.into_iter().filter(|id| !id.trim().is_empty()).map(|id| {
        let detail = &models[id];
        let mode = detail["thinking_config"]["mode"].as_str().unwrap_or_default();
        let efforts: Vec<String> = [&detail["thinking"]["effortOptions"], &detail["thinking"]["effort_options"], &detail["effort_options"]].into_iter()
            .find_map(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
            .filter(|effort| !matches!(*effort, "" | "default" | "none")).map(str::to_owned).collect();
        let mode = match mode {
            "switchable" | "forced_on" => mode,
            "forced_off" => "off",
            _ if detail["reasoning"] == false => "off",
            _ => "on",
        };
        SourceModel {
            id: id.clone(),
            name: detail["name"].as_str().filter(|name| !name.trim().is_empty()).unwrap_or(id).to_owned(),
            context_window: number(&detail["limit"]["context"]).map(|tokens| tokens as u64).filter(|tokens| *tokens > 0),
            max_output_tokens: number(&detail["limit"]["output"]).map(|tokens| tokens as u64).filter(|tokens| *tokens > 0),
            reasoning: levels(mode, &efforts),
            ..Default::default()
        }
    }).collect()
}

/// MiniMax Code's own list, for when the live one can't be had.
pub fn fallback_models() -> Vec<SourceModel> {
    let model = |id: &str, name: &str, context: u64, mode: &str, efforts: &[&str]| SourceModel {
        id: id.into(), name: name.into(), context_window: Some(context), max_output_tokens: Some(128_000),
        reasoning: levels(mode, &efforts.iter().map(|effort| (*effort).to_owned()).collect::<Vec<_>>()), ..Default::default()
    };
    vec![
        model("MiniMax-M3.1-Flash-Preview", "M3.1-Flash-Preview", 512_000, "forced_on", &["low", "medium", "high", "xhigh", "max"]),
        model("MiniMax-M3", "MiniMax-M3", 512_000, "switchable", &[]),
        model("MiniMax-M2.7", "MiniMax-M2.7", 200_000, "on", &[]),
        model("MiniMax-M2.7-highspeed", "MiniMax-M2.7-highspeed", 200_000, "on", &[]),
    ]
}

/// A Messages body's thinking as MiniMax Code sends it: adaptive thinking
/// (at an effort, for a model that takes one) where the router asked for a
/// budget. `efforts` are the model's levels.
pub fn shape_body(body: &[u8], efforts: &[String]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    if value["thinking"]["type"] != "enabled" {
        return None;
    }
    let budget = value["thinking"]["budget_tokens"].as_u64().unwrap_or(24_000);
    let effort = if budget < 4_000 { "low" } else if budget < 16_000 { "medium" } else { "high" };
    value["thinking"] = json!({"type": "adaptive"});
    let takes_effort = efforts.iter().any(|level| !matches!(level.as_str(), "none" | "high"));
    if takes_effort && value["output_config"]["effort"].is_null() {
        value["output_config"]["effort"] = json!(effort);
    }
    serde_json::to_vec(&value).ok()
}

/// A refusal for running out of credits or plan, which another account (an
/// aggregate's next member) can take over.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if !matches!(status, 402 | 403) {
        return None;
    }
    let text = String::from_utf8_lossy(body).to_lowercase();
    let quota = ["insufficient", "balance", "credit", "quota", "exhaust", "limit", "余额", "积分", "额度", "不足", "用完", "上限"];
    quota.iter().any(|word| text.contains(word))
        .then(|| "this MiniMax Code account's credits or M Plan have run out; check in daily for more, or add another account".to_owned())
}

/// A day of MiniMax's check-in panel, by its status.
const DAY_CLAIMED: i64 = 3;
const DAY_DISABLED: i64 = 4;

/// A check-in request as MiniMax Code's page sends it, signed with
/// `x-timestamp` and `x-signature` (MiniMax doesn't check it now).
async fn check_in_call(client: &Client, site: &Site, creds: &Creds, method: Method, path: &str) -> Result<Value, (String, bool)> {
    let body = if method == Method::POST { "{}" } else { "" };
    let seconds = chrono::Utc::now().timestamp();
    let mut request = client.request(method, format!("{}/minimax-cloud/api/v1/signin{path}?timezone_id=Asia%2FShanghai", site.llm.trim_end_matches('/')))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", creds.access))
        .header("user-agent", "MiniMaxAgent")
        .header("x-timestamp", seconds.to_string())
        .header("x-signature", oauth::md5_hex(&format!("{seconds}{SIGN_SECRET}{body}")))
        .timeout(Duration::from_secs(20));
    if !body.is_empty() {
        request = request.body(body);
    }
    let response = request.send().await.map_err(|error| (format!("cannot reach MiniMax: {error}"), true))?;
    let status = response.status().as_u16();
    if status == 401 {
        return Err(("MiniMax Code's sign-in has expired — sign in again".into(), false));
    }
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if let Some(code) = value["base_resp"]["status_code"].as_i64().filter(|code| *code != 0) {
        return Err((format!("code {code}: {}", value["base_resp"]["status_msg"].as_str().unwrap_or_default()), false));
    }
    if !(200..300).contains(&status) || value.is_null() {
        return Err((format!("HTTP {status}"), false));
    }
    Ok(value)
}

/// Press MiniMax Code's daily 签到 as its page does: the week's panel first,
/// a claim only while today's is claimable. The streak is today's day of
/// the week's panel.
pub async fn check_in(client: &Client, site: &Site, creds: &Creds) -> Checkin {
    let status = match check_in_call(client, site, creds, Method::GET, "/status").await {
        Ok(status) => status,
        Err((message, offline)) => return Checkin::failed(message, offline),
    };
    let Some(today) = status["data"]["days"].as_array().into_iter().flatten().find(|day| day["is_today"] == true).cloned() else {
        return Checkin::outcome(Outcome::Inactive);
    };
    let points = number(&today["points"]).unwrap_or(0.0);
    let day = today["day_no"].as_u64().unwrap_or(0) as u32;
    match today["status"].as_i64() {
        Some(DAY_CLAIMED) => return Checkin::done(points, day),
        Some(DAY_DISABLED) => return Checkin::outcome(Outcome::Ineligible).with_message("today's check-in is closed for this account"),
        _ => {}
    }
    let got = match check_in_call(client, site, creds, Method::POST, "/claim").await {
        Ok(got) => got,
        Err((message, offline)) => return Checkin::failed(message, offline),
    };
    let day = got["data"]["day_no"].as_u64().filter(|day| *day > 0).map_or(day, |day| day as u32);
    let credit = number(&got["data"]["points"]).filter(|points| *points > 0.0).unwrap_or(points);
    match got["data"]["claim_result"].as_i64() {
        Some(1) => Checkin::claimed(credit, day),
        Some(2) => Checkin::done(credit, day),
        other => Checkin::failed(format!("claim_result {}", other.map_or("missing".into(), |code| code.to_string())), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_read_mcode_s_config_in_its_order() {
        let body = json!({"providers": [{"providerId": "minimax", "config": {"model_order": ["b"], "models": {
            "a": {"name": "A", "limit": {"context": 200000, "output": 1000}, "thinking_config": {"mode": "switchable"}},
            "b": {"name": "B", "limit": {"context": "512000"}, "thinking_config": {"mode": "forced_on"}, "thinking": {"effortOptions": ["default", "low", "high"]}},
            "c": {"reasoning": false},
        }}}]});
        let models = parse_models(&body);
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
        assert_eq!(models[0].reasoning, ["low", "high"]);
        assert_eq!(models[0].context_window, Some(512_000));
        assert_eq!(models[1].reasoning, ["none", "high"], "a switchable model turns thinking off or on");
        assert!(models[2].reasoning.is_empty());
        assert!(parse_models(&json!({})).is_empty());
    }

    #[test]
    fn a_budget_becomes_adaptive_thinking_with_an_effort_only_where_taken() {
        let body = json!({"model": "m", "thinking": {"type": "enabled", "budget_tokens": 10000}});
        let effortful: Value = serde_json::from_slice(&shape_body(body.to_string().as_bytes(), &["low".into(), "max".into()]).unwrap()).unwrap();
        assert_eq!(effortful["thinking"], json!({"type": "adaptive"}));
        assert_eq!(effortful["output_config"]["effort"], "medium");
        let switchable: Value = serde_json::from_slice(&shape_body(body.to_string().as_bytes(), &["none".into(), "high".into()]).unwrap()).unwrap();
        assert!(switchable.get("output_config").is_none());
        assert!(shape_body(br#"{"thinking":{"type":"disabled"}}"#, &[]).is_none(), "off stays off");
    }

    #[test]
    fn membership_and_the_plan_window_read_as_mcode_reads_them() {
        let member = membership_of(&json!({"data": {"has_token_plan": true, "token_plan_tier": "Plus", "op_group_id": "g1"},
            "op_credit_summary": {"total_remaining_amount": "1234.50"}}));
        assert_eq!(member, Membership { has_plan: Some(true), group: Some("g1".into()), tier: Some("Plus".into()), balance: Some(1234.5) });
        let remains = json!({"base_resp": {"status_code": 0}, "model_remains": [
            {"model_name": "general", "current_interval_remaining_percent": 70, "current_interval_status": 1, "current_weekly_remaining_percent": 40, "current_weekly_status": 1},
            {"model_name": "video", "current_interval_remaining_percent": 0, "current_interval_status": 2},
        ]});
        assert_eq!(plan_used(&remains), Some(60.0), "the tightest of the general group's windows; other groups aside");
    }

    #[test]
    fn credits_running_out_is_explained() {
        assert!(explain(403, "{\"base_resp\":{\"status_msg\":\"余额不足\"}}".as_bytes()).is_some());
        assert!(explain(403, b"{\"error\":\"forbidden\"}").is_none());
        assert!(explain(500, b"insufficient").is_none());
    }
}
