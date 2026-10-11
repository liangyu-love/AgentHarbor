//! WorkBuddy (Tencent's CodeBuddy plan) and WorkBuddy AI, its international
//! build: the account's own sign-in, token refresh, credit allowance and the
//! request shape its OpenAI-compatible chat endpoint takes.
//!
//! Ported from magpie (MIT, see LICENSES/THIRD_PARTY.md): the sign-in is
//! WorkBuddy's external-link flow (auth/state, then auth/token and
//! login/account polled until ready); requests carry the account's JWT with
//! its uid and domain, as WorkBuddy sends them. The WorkBuddy desktop app's
//! own session is never read or written: every account signs in here, so a
//! refresh here can never sign the app out.
use super::{
    store::{Quota, SourceModel},
    SourceKind,
};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// The WorkBuddy desktop version requests name. copilot.tencent.com refuses
/// a User-Agent it does not recognise (code 10085), chat and billing alike.
const UA_VERSION: &str = "5.5.6";
/// The plugin version the sign-in page is told.
const APP_VERSION: &str = "2.0.0";
const RETRY_TOKEN: i64 = 11217;
const RETRY_ACCOUNT: i64 = 12151;
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
/// What a chat that has no system message is sent with: WorkBuddy refuses one
/// whose first message is not the system prompt.
const DEFAULT_SYSTEM: &str = "You are a helpful assistant.";

/// What the user can do about WorkBuddy's "Illegal API invocation from an
/// unapproved channel": both builds answer it to a chat whose system prompt is
/// Codex's or Claude Code's own, whatever the headers. AgentHarbor never
/// rewrites that prompt.
pub const REFUSED_HINT: &str = "WorkBuddy refuses chats from Codex and Claude Code (their system prompt); use it from OpenCode or Pi, or put another model after it in an aggregate";

#[derive(Clone, Debug)]
pub struct Site {
    pub kind: SourceKind,
    pub api: String,
    platform: &'static str,
}

pub fn site(kind: SourceKind) -> Site {
    let (api, platform) = match kind {
        SourceKind::WorkbuddyAi => ("https://www.workbuddy.ai", "workbuddy-ai"),
        _ => ("https://copilot.tencent.com", "workbuddy"),
    };
    Site { kind, api: super::endpoint_override(kind).unwrap_or_else(|| api.to_owned()), platform }
}

impl Site {
    pub fn chat_base(&self) -> String {
        format!("{}/v2", self.api.trim_end_matches('/'))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub uid: String,
    pub access: String,
    pub refresh: String,
    /// Unix milliseconds when the access token lapses; 0 when unknown.
    pub expires_at: i64,
    pub refresh_expires_at: i64,
    pub domain: String,
}

/// A WorkBuddy reply's `{code, msg}` refusal, or the HTTP status without one.
#[derive(Debug)]
pub struct CallError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() { write!(f, "error {}", self.code) } else { f.write_str(&self.message) }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One of WorkBuddy's JSON endpoints, which wrap what they say in
/// `{code, msg, data}`: code 0 is a success.
async fn call(client: &Client, method: Method, url: &str, headers: &[(&str, String)], body: Option<Value>) -> Result<Value, CallError> {
    let mut request = client.request(method, url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", format!("WorkBuddy/{UA_VERSION}"))
        .timeout(Duration::from_secs(30));
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    if let Some(body) = body {
        request = request.body(body.to_string());
    }
    let response = request.send().await.map_err(|error| CallError {
        code: if error.is_timeout() { 408 } else { 0 },
        message: if error.is_timeout() { "timed out".into() } else { "cannot reach WorkBuddy".into() },
    })?;
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let code = match &envelope["code"] {
        Value::Number(number) => number.as_i64().unwrap_or(0),
        Value::String(text) => text.parse().unwrap_or(0),
        _ => 0,
    };
    let message = envelope["msg"].as_str().unwrap_or_default().to_owned();
    if !status.is_success() {
        return Err(CallError { code: if code != 0 { code } else { i64::from(status.as_u16()) }, message });
    }
    if code != 0 {
        return Err(CallError { code, message });
    }
    Ok(envelope.get("data").cloned().unwrap_or(Value::Null))
}

fn domain(site: &Site, creds: &Creds) -> String {
    if !creds.domain.is_empty() {
        return creds.domain.clone();
    }
    reqwest::Url::parse(&site.api).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default()
}

/// The headers WorkBuddy's own chats carry for this account.
pub fn auth_headers(site: &Site, creds: &Creds) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("authorization", format!("Bearer {}", creds.access)),
        ("x-user-id", creds.uid.clone()),
        ("x-domain", domain(site, creds)),
        ("x-product", "SaaS".to_owned()),
        ("x-ide-type", "WorkBuddy".to_owned()),
        ("user-agent", format!("WorkBuddy/{UA_VERSION}")),
    ];
    if site.kind == SourceKind::WorkbuddyAi {
        // WorkBuddy AI's own client names itself and the conversation turn.
        let id = uuid::Uuid::new_v4().simple().to_string();
        headers.extend([
            ("x-requested-with", "XMLHttpRequest".to_owned()),
            ("x-agent-intent", "craft".to_owned()),
            ("x-agent-type", "main".to_owned()),
            ("x-ide-name", "WorkBuddy".to_owned()),
            ("x-ide-version", UA_VERSION.to_owned()),
            ("x-conversation-id", id.clone()),
            ("x-conversation-request-id", id.clone()),
            ("x-conversation-message-id", id.clone()),
            ("x-request-id", id),
        ]);
    }
    headers
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TokenReply {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    expires_in: i64,
    refresh_expires_at: i64,
    refresh_expires_in: i64,
    domain: String,
}

fn merge(mut creds: Creds, reply: TokenReply) -> Creds {
    let now = now_ms();
    creds.access = reply.access_token;
    if !reply.refresh_token.is_empty() {
        creds.refresh = reply.refresh_token;
    }
    if !reply.domain.is_empty() {
        creds.domain = reply.domain;
    }
    if reply.expires_at > 0 {
        creds.expires_at = reply.expires_at;
    } else if reply.expires_in > 0 {
        creds.expires_at = now + reply.expires_in * 1000;
    }
    if reply.refresh_expires_at > 0 {
        creds.refresh_expires_at = reply.refresh_expires_at;
    } else if reply.refresh_expires_in > 0 {
        creds.refresh_expires_at = now + reply.refresh_expires_in * 1000;
    }
    creds
}

/// Whether the access token lapses within a minute (an unknown expiry is trusted).
pub fn near_expiry(creds: &Creds) -> bool {
    creds.expires_at != 0 && now_ms() >= creds.expires_at - 60_000
}

pub enum RefreshError {
    /// The refresh token is gone or refused: sign in again.
    SignedOut,
    /// A hiccup; the token in hand may still work.
    Transient(String),
}

/// Trade the refresh token for a fresh access token, as WorkBuddy's plugin does.
pub async fn refresh(client: &Client, site: &Site, creds: &Creds) -> Result<Creds, RefreshError> {
    if creds.refresh.is_empty() || (creds.refresh_expires_at > 0 && now_ms() >= creds.refresh_expires_at) {
        return Err(RefreshError::SignedOut);
    }
    let data = call(client, Method::POST, &format!("{}/v2/plugin/auth/token/refresh", site.api), &[
        ("X-Refresh-Token", creds.refresh.clone()),
        ("X-Auth-Refresh-Source", "plugin".to_owned()),
        ("X-Domain", domain(site, creds)),
    ], Some(json!({}))).await.map_err(|error| match error.code {
        401 | 403 => RefreshError::SignedOut,
        _ => RefreshError::Transient(format!("WorkBuddy token refresh: {error}")),
    })?;
    let reply: TokenReply = serde_json::from_value(data).unwrap_or_default();
    if reply.access_token.is_empty() {
        return Err(RefreshError::Transient("WorkBuddy gave no refreshed token".into()));
    }
    Ok(merge(creds.clone(), reply))
}

/// The plan's credits used against what the cycle grants.
pub async fn quota(client: &Client, site: &Site, creds: &Creds) -> Quota {
    let headers: Vec<(&str, String)> = auth_headers(site, creds).into_iter()
        .filter(|(name, _)| !name.starts_with("x-conversation") && *name != "x-request-id").collect();
    let mut quota = Quota { unit: "credits".into(), checked_at: chrono::Utc::now().timestamp(), ..Default::default() };
    // The resource summary sits outside the /v2 gateway prefix.
    match call(client, Method::POST, &format!("{}/billing/meter/get-user-resource-summary", site.api), &headers, Some(json!({}))).await {
        Ok(data) => {
            let number = |value: &Value| match value {
                Value::Number(number) => number.as_f64().unwrap_or(0.0),
                Value::String(text) => text.trim().parse().unwrap_or(0.0),
                _ => 0.0,
            };
            for package in data["Packages"].as_array().into_iter().flatten() {
                quota.limit += number(&package["CycleTotalCapacity"]);
                quota.used += number(&package["CycleUsedCapacity"]);
            }
            quota.paid = data["IsPaidUser"].as_bool();
        }
        Err(error) => quota.error = Some(error.to_string()),
    }
    quota
}

pub struct SignIn {
    pub url: String,
    state: String,
}

/// Ask WorkBuddy for a sign-in page and the state its answer is polled by.
pub async fn begin_sign_in(client: &Client, site: &Site) -> Result<SignIn, String> {
    let data = call(client, Method::POST,
        &format!("{}/v2/plugin/auth/state?platform={}", site.api, site.platform),
        &[("X-No-Authorization", "true".into()), ("X-No-User-Id", "true".into()),
          ("X-No-Enterprise-Id", "true".into()), ("X-No-Department-Info", "true".into())],
        Some(json!({}))).await.map_err(|error| format!("WorkBuddy sign-in: {error}"))?;
    let state = data["state"].as_str().unwrap_or_default().to_owned();
    let mut url = reqwest::Url::parse(data["authUrl"].as_str().unwrap_or_default())
        .map_err(|_| "WorkBuddy gave no sign-in page".to_owned())?;
    if state.is_empty() || (url.scheme() != "https" && !super::endpoint_override(site.kind).is_some()) {
        return Err("WorkBuddy gave no sign-in page".into());
    }
    url.query_pairs_mut()
        .append_pair("version", APP_VERSION)
        .append_pair("loginSessionId", &uuid::Uuid::new_v4().simple().to_string());
    Ok(SignIn { url: url.to_string(), state })
}

pub struct SignedIn {
    pub creds: Creds,
    pub name: String,
}

/// Poll until the browser sign-in is done, then read who signed in.
pub async fn finish_sign_in(client: &Client, site: &Site, sign_in: SignIn, cancel: watch::Receiver<bool>, interval: Duration) -> Result<SignedIn, String> {
    let deadline = Instant::now() + SIGN_IN_WINDOW;
    let state = sign_in.state;
    let token = poll(client, &format!("{}/v2/plugin/auth/token?state={state}", site.api), &[], RETRY_TOKEN, deadline, interval, cancel.clone()).await?;
    let reply: TokenReply = serde_json::from_value(token).unwrap_or_default();
    if reply.access_token.is_empty() {
        return Err("WorkBuddy gave no token".into());
    }
    let mut creds = merge(Creds::default(), reply);
    let account = poll(client, &format!("{}/v2/plugin/login/account?state={state}", site.api), &[
        ("Authorization", format!("Bearer {}", creds.access)),
        ("X-Domain", domain(site, &creds)),
        ("X-No-User-Id", "true".into()),
        ("X-No-Enterprise-Id", "true".into()),
    ], RETRY_ACCOUNT, deadline, interval, cancel).await?;
    creds.uid = account["uid"].as_str().unwrap_or_default().to_owned();
    if creds.uid.is_empty() {
        return Err("WorkBuddy gave no account".into());
    }
    let pick = |key: &str| account[key].as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned);
    let name = pick("nickname").or_else(|| pick("phoneNumber")).unwrap_or_else(|| creds.uid.clone());
    Ok(SignedIn { creds, name })
}

async fn poll(client: &Client, url: &str, headers: &[(&str, String)], retry_code: i64, deadline: Instant, interval: Duration, mut cancel: watch::Receiver<bool>) -> Result<Value, String> {
    loop {
        tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            _ = tokio::time::sleep(interval) => {}
        }
        if Instant::now() >= deadline {
            return Err("accounts.loginTimeout".into());
        }
        match call(client, Method::GET, url, headers, None).await {
            Ok(Value::Null) => continue,
            Ok(data) => return Ok(data),
            Err(error) if error.code == retry_code || error.code == 408 || error.code == 429 => continue,
            Err(error) => return Err(format!("WorkBuddy sign-in: {error}")),
        }
    }
}

/// Start a chat with a system message when it has none.
pub fn shape_chat_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let messages = value.get_mut("messages")?.as_array_mut()?;
    if messages.first().is_some_and(|first| first["role"] == "system") {
        return None;
    }
    messages.insert(0, json!({"role": "system", "content": DEFAULT_SYSTEM}));
    serde_json::to_vec(&value).ok()
}

/// WorkBuddy's channel refusal of Codex's and Claude Code's prompts.
pub fn refused(status: u16, body: &[u8]) -> bool {
    if status < 400 {
        return false;
    }
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    text.contains("unapproved channel") || text.contains("illegal api invocation")
}

fn model(id: &str, name: &str, context: u64, reasoning: &[&str]) -> SourceModel {
    SourceModel { id: id.into(), name: name.into(), context_window: Some(context),
        reasoning: reasoning.iter().map(|level| (*level).to_owned()).collect() }
}

/// The plan's models: WorkBuddy's CLI agent's picks, with the names and
/// context windows its product config gives them.
pub fn models(kind: SourceKind) -> Vec<SourceModel> {
    match kind {
        SourceKind::WorkbuddyAi => vec![
            model("default-model", "Default", 176_000, &[]),
            model("fast-model", "Fast", 200_000, &[]),
            model("balanced-model", "Balanced", 256_000, &[]),
            model("primary-model", "Primary", 272_000, &[]),
            model("deep-model", "Deep", 176_000, &[]),
            model("gpt-5.5", "GPT-5.5", 1_000_000, &[]),
            model("gpt-5.4", "GPT-5.4", 272_000, &[]),
            model("gpt-5.3-codex", "GPT-5.3-Codex", 272_000, &[]),
            model("gemini-3.1-pro", "Gemini-3.1-Pro", 400_000, &[]),
            model("gemini-3.5-flash", "Gemini-3.5-Flash", 1_000_000, &[]),
            model("glm-5.3", "GLM-5.3", 1_000_000, &["low", "high", "max"]),
            model("glm-5.2", "GLM-5.2", 1_000_000, &["high", "xhigh"]),
            model("hy3", "Hy3", 192_000, &["low", "high"]),
            model("kimi-k3", "Kimi-K3", 1_000_000, &[]),
            model("kimi-k2.6", "Kimi-K2.6", 256_000, &[]),
            model("minimax-m3", "MiniMax-M3", 512_000, &[]),
        ],
        _ => vec![
            model("auto", "Auto", 168_000, &[]),
            model("hy4-preview-f", "Hy4 preview", 1_000_000, &["high"]),
            model("hy3", "Hy3", 192_000, &["low", "high"]),
            model("hy3-x", "Hy3-X", 192_000, &["low", "high"]),
            model("deepseek-v4.1-flash", "Deepseek-V4.1-Flash", 1_000_000, &[]),
            model("glm-5.3", "GLM-5.3", 1_000_000, &["low", "high", "max"]),
            model("glm-5.3-flash", "GLM-5.3-Flash", 1_000_000, &["low", "high", "max"]),
            model("glm-5.2", "GLM-5.2", 1_000_000, &["high", "xhigh"]),
            model("glm-5.1", "GLM-5.1", 200_000, &[]),
            model("glm-5v-turbo", "GLM-5v-Turbo", 200_000, &[]),
            model("minimax-m3", "MiniMax-M3", 512_000, &[]),
            model("kimi-k3-1", "Kimi-K3", 1_000_000, &["low", "high", "xhigh"]),
            model("kimi-k2.7", "Kimi-K2.7-Code", 256_000, &[]),
            model("kimi-k2.6", "Kimi-K2.6", 256_000, &[]),
            model("deepseek-v4-pro", "Deepseek-V4-Pro", 1_000_000, &["none", "high", "xhigh"]),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_without_a_system_message_gets_one_first() {
        let shaped = shape_chat_body(br#"{"model":"glm-5.3","messages":[{"role":"user","content":"hi"}]}"#).unwrap();
        let value: Value = serde_json::from_slice(&shaped).unwrap();
        assert_eq!(value["messages"][0], json!({"role":"system","content":DEFAULT_SYSTEM}));
        assert_eq!(value["messages"][1]["content"], "hi");
        assert!(shape_chat_body(br#"{"messages":[{"role":"system","content":"x"},{"role":"user","content":"hi"}]}"#).is_none());
        assert!(shape_chat_body(b"not json").is_none());
    }

    #[test]
    fn the_channel_refusal_is_recognised_only_on_errors() {
        assert!(refused(400, br#"{"error":{"message":"WorkBuddy AI: Illegal API invocation from an unapproved channel"}}"#));
        assert!(!refused(200, b"unapproved channel"));
        assert!(!refused(400, br#"{"error":"first message is not system prompt"}"#));
    }

    #[test]
    fn requests_carry_the_account_and_the_app_identity() {
        let site = Site { kind: SourceKind::Workbuddy, api: "https://copilot.tencent.com".into(), platform: "workbuddy" };
        let creds = Creds { uid: "u1".into(), access: "tok".into(), ..Default::default() };
        let headers = auth_headers(&site, &creds);
        let get = |name: &str| headers.iter().find(|(key, _)| *key == name).map(|(_, value)| value.as_str());
        assert_eq!(get("authorization"), Some("Bearer tok"));
        assert_eq!(get("x-user-id"), Some("u1"));
        assert_eq!(get("x-domain"), Some("copilot.tencent.com"));
        assert_eq!(get("user-agent"), Some("WorkBuddy/5.5.6"));
        assert!(get("x-request-id").is_none());
        let ai = Site { kind: SourceKind::WorkbuddyAi, api: "https://www.workbuddy.ai".into(), platform: "workbuddy-ai" };
        assert!(auth_headers(&ai, &creds).iter().any(|(key, _)| *key == "x-request-id"));
    }

    #[test]
    fn a_refreshed_token_keeps_what_the_reply_leaves_out() {
        let before = Creds { uid: "u".into(), access: "a".into(), refresh: "r".into(), domain: "d".into(), ..Default::default() };
        let after = merge(before, TokenReply { access_token: "b".into(), expires_in: 60, ..Default::default() });
        assert_eq!((after.access.as_str(), after.refresh.as_str(), after.domain.as_str(), after.uid.as_str()), ("b", "r", "d", "u"));
        assert!(after.expires_at > now_ms());
    }
}
