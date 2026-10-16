//! WorkBuddy (Tencent's CodeBuddy plan) and WorkBuddy AI, its international
//! build: the account's own sign-in, token refresh, credit allowance and the
//! request shape its OpenAI-compatible chat endpoint takes.
//!
//! Ported from magpie and its community plugin (MIT, see
//! LICENSES/THIRD_PARTY.md): the sign-in is WorkBuddy's external-link flow
//! (auth/state, then auth/token and login/account polled until ready);
//! requests carry the account's JWT with its uid, domain and enterprise, and
//! the client's own headers, as WorkBuddy's desktop app sends them. The
//! desktop app's own session is never read or written: every account signs
//! in here, so a refresh here can never sign the app out.
use super::{
    chat_fit,
    checkin::{Checkin, Outcome},
    store::{Quota, SourceModel},
    SourceKind,
};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
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
/// unapproved channel": both builds answer it to a chat holding words of
/// Codex's or Claude Code's own, whatever the headers. `FLAGGED` rewrites
/// the ones known; this is for one it doesn't know.
pub const REFUSED_HINT: &str = "WorkBuddy refuses chats from Codex and Claude Code (their system prompt); use it from OpenCode or Pi, or put another model after it in an aggregate";
/// WorkBuddy's 403 code 11140: the account is out until WorkBuddy lifts it.
pub const RESTRICTED_HINT: &str = "WorkBuddy restricted this account for unusual activity; it stays out until WorkBuddy lifts that, which an appeal (Submit feedback in WorkBuddy) asks for";
/// Code 14018: even 0-rate models need an active credit package.
pub const EXHAUSTED_HINT: &str = "WorkBuddy requires an active credit package even for 0-rate free models; check your account package balance";

/// The words WorkBuddy refuses a chat for, each with words that say the same
/// and pass: tried one by one against both builds with the requests Claude
/// Code 2.1.280 and Codex 0.159.2 send. WorkBuddy reads every message, not
/// only the system prompt, and pays no mind to case.
const FLAGGED: [(&str, &str); 4] = [
    ("You are Claude Code, Anthropic's official CLI for Claude", "You are Claude Code, Anthropic's CLI for Claude"),
    ("You are Claude Code, Anthropic\u{2019}s official CLI for Claude", "You are Claude Code, Anthropic's CLI for Claude"),
    ("Main branch (you will usually use this for PRs)", "Main branch (usually the base for PRs)"),
    ("Codex CLI is an open source project led by OpenAI.", "The Codex CLI is an open source project led by OpenAI."),
];

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
    /// The enterprise (team) the account signed in as; empty for a personal one.
    pub enterprise_id: String,
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

/// The enterprise an account signed in as, as WorkBuddy's client names it on
/// every request: without it WorkBuddy answers as the personal account.
fn enterprise_headers(creds: &Creds) -> Vec<(&'static str, String)> {
    let id = creds.enterprise_id.trim();
    if id.is_empty() { Vec::new() } else { vec![("x-enterprise-id", id.to_owned()), ("x-tenant-id", id.to_owned())] }
}

/// The headers WorkBuddy's own chats carry for this account: its own, and
/// the client's, which WorkBuddy's usage list shows as the client that
/// asked (使用端). A chat's ids are `conversation_headers`'.
pub fn auth_headers(site: &Site, creds: &Creds) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("authorization", format!("Bearer {}", creds.access)),
        ("x-user-id", creds.uid.clone()),
        ("x-domain", domain(site, creds)),
        ("x-product", "SaaS".to_owned()),
        ("x-ide-type", "WorkBuddy".to_owned()),
        ("user-agent", format!("WorkBuddy/{UA_VERSION}")),
        ("x-requested-with", "XMLHttpRequest".to_owned()),
        ("x-agent-intent", "craft".to_owned()),
        ("x-agent-purpose", "conversation".to_owned()),
        ("x-ide-name", "WorkBuddy".to_owned()),
        ("x-ide-version", UA_VERSION.to_owned()),
    ];
    headers.extend(enterprise_headers(creds));
    if site.kind == SourceKind::WorkbuddyAi {
        headers.push(("x-agent-type", "main".to_owned()));
    }
    headers
}

/// What the ids are derived with, so an id says nothing of the words it was
/// made from; new in each run, as WorkBuddy's own ids are a run's own.
fn salt() -> &'static str {
    static SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SALT.get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
}

fn hex_digest(text: &str, bytes: usize) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(text.as_bytes()).iter().take(bytes).map(|byte| format!("{byte:02x}")).collect()
}

/// The 32 hex digits WorkBuddy's ids are made of, for a thing of `kind` named by `key`.
fn id_for(kind: &str, key: &str) -> String {
    hex_digest(&format!("{}|{kind}|{key}", salt()), 16)
}

/// A message's signature: its text, and a digest of each part that isn't
/// text (an image); a tool result's part is left out.
fn signature_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().map(|part| match part["type"].as_str().unwrap_or_default() {
            "" | "text" => part["text"].as_str().unwrap_or_default().to_owned(),
            "tool_result" | "tool-result" => String::new(),
            kind => format!("[{kind}:{}]", hex_digest(&part.to_string(), 4)),
        }).collect(),
        _ => String::new(),
    }
}

/// The user send a chat answers: its last user message that is the user's
/// own (not one answering a tool's call), by place and signature.
fn turn_key(messages: &[Value]) -> String {
    for index in (0..messages.len()).rev() {
        if messages[index]["role"] != "user" {
            continue;
        }
        if let Some(before) = index.checked_sub(1).map(|before| &messages[before]) {
            let answered = before["role"] == "tool"
                || before["role"] == "assistant" && before["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty());
            if answered {
                continue;
            }
        }
        let signature = signature_of(&messages[index]["content"]);
        return if signature.is_empty() { String::new() } else { format!("u{index}:{signature}") };
    }
    String::new()
}

/// A chat's ids, as WorkBuddy's own client names them: its backend counts a
/// user send as one request by the turn's id, so every step of one send (a
/// tool result sent back, a retry) carries the same one and the user's next
/// message another. The conversation is named by its first user message,
/// which every later turn repeats.
pub fn conversation_headers(body: &[u8]) -> Vec<(&'static str, String)> {
    let message = uuid::Uuid::new_v4().simple().to_string();
    let mut headers = vec![("x-conversation-message-id", message.clone()), ("x-request-id", message)];
    let chat: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let messages = chat["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
    let conversation = messages.iter().filter(|message| message["role"] == "user")
        .map(|message| signature_of(&message["content"])).find(|signature| !signature.is_empty())
        .map(|signature| format!("first:{signature}")).unwrap_or_default();
    let turn = turn_key(messages);
    let request = if turn.is_empty() { uuid::Uuid::new_v4().simple().to_string() } else { id_for("turn", &format!("{conversation}|{turn}")) };
    headers.push(("x-conversation-request-id", request));
    if !conversation.is_empty() {
        headers.push(("x-conversation-id", id_for("conversation", &conversation)));
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
    let mut headers = vec![
        ("X-Refresh-Token", creds.refresh.clone()),
        ("X-Auth-Refresh-Source", "plugin".to_owned()),
        ("X-Domain", domain(site, creds)),
    ];
    headers.extend(enterprise_headers(creds));
    let data = call(client, Method::POST, &format!("{}/v2/plugin/auth/token/refresh", site.api), &headers, Some(json!({}))).await.map_err(|error| match error.code {
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
            // An enterprise account's summary lists no packages: its credits
            // are the enterprise meter's, the one WorkBuddy's usage page reads.
            if quota.limit <= 0.0 && !creds.enterprise_id.is_empty() {
                let meter = format!("{}/billing/meter/get-enterprise-user-usage", site.api);
                if let Ok(usage) = call(client, Method::POST, &meter, &headers, Some(json!({}))).await {
                    quota.limit = number(&usage["limitNum"]);
                    quota.used = number(&usage["credit"]);
                }
            }
        }
        Err(error) => quota.error = Some(error.to_string()),
    }
    quota
}

/// The plan's models as WorkBuddy's product config gives its CLI agent:
/// their names, context and reasoning levels. The CLI's User-Agent picks
/// WorkBuddy's config (a bare WorkBuddy/<v> gets CodeBuddy IDE's).
pub async fn fetch_models(client: &Client, site: &Site, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let mut request = client.get(format!("{}/v3/config", site.api)).timeout(Duration::from_secs(15));
    for (name, value) in auth_headers(site, creds) {
        request = request.header(name, value);
    }
    let response = request.header("user-agent", format!("CLI/{APP_VERSION} WorkBuddy/{UA_VERSION}"))
        .header("accept", "application/json").send().await.map_err(|_| "cannot reach WorkBuddy".to_owned())?;
    let status = response.status();
    let envelope: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("WorkBuddy's config: HTTP {}", status.as_u16()));
    }
    if envelope["code"].as_i64().is_some_and(|code| code != 0) {
        return Err(format!("WorkBuddy's config: {} ({})", envelope["msg"].as_str().unwrap_or_default(), envelope["code"]));
    }
    let config = &envelope["data"];
    let mut out: Vec<SourceModel> = Vec::new();
    for agent in config["agents"].as_array().into_iter().flatten().filter(|agent| agent["name"] == "cli") {
        for id in agent["models"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if out.iter().any(|seen| seen.id == id) {
                continue;
            }
            let mut model = SourceModel { id: id.to_owned(), name: id.to_owned(), ..Default::default() };
            if let Some(detail) = config["models"].as_array().into_iter().flatten().find(|model| model["id"] == id) {
                if let Some(name) = detail["name"].as_str().filter(|name| !name.is_empty()) {
                    model.name = name.to_owned();
                }
                model.context_window = detail["maxInputTokens"].as_u64().filter(|tokens| *tokens > 0);
                model.max_output_tokens = detail["maxOutputTokens"].as_u64().filter(|tokens| *tokens > 0);
                let efforts: Vec<String> = detail["reasoning"]["supportedEfforts"].as_array().into_iter().flatten()
                    .filter_map(Value::as_str).map(str::to_owned).collect();
                if !efforts.is_empty() {
                    let can_off = detail["reasoning"]["canDisableThinking"].as_bool().unwrap_or(true);
                    if detail["onlyReasoning"].as_bool() != Some(true) && can_off {
                        model.reasoning.push("none".into());
                    }
                    model.reasoning.extend(efforts);
                }
            }
            out.push(model);
        }
    }
    // Models the config names alike (deepseek-v4.1-flash and -sg) told apart.
    let mut first: HashMap<String, String> = HashMap::new();
    for model in &mut out {
        match first.get(&model.name) {
            None => {
                first.insert(model.name.clone(), model.id.clone());
            }
            Some(id) => {
                let rest = model.id.strip_prefix(&format!("{id}-")).map(str::to_ascii_uppercase).unwrap_or_else(|| model.id.clone());
                model.name = format!("{} ({rest})", model.name);
            }
        }
    }
    if out.is_empty() {
        return Err("WorkBuddy's config lists no models for its CLI agent".into());
    }
    Ok(out)
}

/// WorkBuddy's check-in answers, the codes its claim refuses with.
const CHECKIN_ALREADY: i64 = 1001;
const CHECKIN_INELIGIBLE: i64 = 1002;
const CHECKIN_ENDED: i64 = 1003;

/// Press WorkBuddy's daily 签到 as its app does: the event's status first,
/// and a claim only while it runs and today's isn't in. Only the Chinese
/// build has the event.
pub async fn check_in(client: &Client, site: &Site, creds: &Creds) -> Checkin {
    let headers: Vec<(&str, String)> = auth_headers(site, creds).into_iter()
        .filter(|(name, _)| !name.starts_with("x-conversation") && *name != "x-request-id").collect();
    let number = |value: &Value| match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0),
        Value::String(text) => text.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    };
    let status = match call(client, Method::POST, &format!("{}/v2/billing/meter/checkin-activity-status", site.api), &headers, Some(json!({}))).await {
        Ok(status) => status,
        Err(error) => return check_in_refused(client, site, &headers, error).await,
    };
    let streak = number(&status["streak_days"]) as u32;
    if status["active"].as_bool() != Some(true) {
        return Checkin::outcome(Outcome::Inactive);
    }
    if status["today_checked_in"].as_bool() == Some(true) {
        return Checkin::done(number(&status["today_credit"]), streak);
    }
    match call(client, Method::POST, &format!("{}/v2/billing/meter/daily-checkin", site.api), &headers, Some(json!({}))).await {
        Ok(got) => Checkin::claimed(number(&got["credit"]), number(&got["streak_days"]) as u32)
            .with_message(got["message"].as_str().unwrap_or_default()),
        Err(error) => {
            let mut result = check_in_refused(client, site, &headers, error).await;
            if result.outcome == Outcome::Done {
                result.streak = streak;
            }
            result
        }
    }
}

/// What a refusal of the check-in comes to: one of its answers for the day,
/// or a failure to try again. WorkBuddy's gateway answers a token it
/// refuses with a bare 401; when the account's credits read with the same
/// headers, the sign-in is good and only the check-in turns AgentHarbor
/// away — WorkBuddy's own app sends it with a device token from its security
/// SDK — so the user checks in in the app.
async fn check_in_refused(client: &Client, site: &Site, headers: &[(&str, String)], error: CallError) -> Checkin {
    match error.code {
        CHECKIN_ALREADY => Checkin::outcome(Outcome::Done).with_message(error.message),
        CHECKIN_INELIGIBLE => Checkin::outcome(Outcome::Ineligible).with_message(error.message),
        CHECKIN_ENDED => Checkin::outcome(Outcome::Inactive).with_message(error.message),
        401 => {
            let summary = format!("{}/billing/meter/get-user-resource-summary", site.api);
            if call(client, Method::POST, &summary, headers, Some(json!({}))).await.is_ok() {
                Checkin::outcome(Outcome::OwnApp)
                    .with_message("WorkBuddy answered the check-in 401 while this account's credits read fine: check in in the WorkBuddy app")
            } else {
                Checkin::failed("WorkBuddy refused this account's sign-in (401); sign in to it again", false)
            }
        }
        // No reply at all: no connection, or none in time.
        0 | 408 => Checkin::failed(error.to_string(), true),
        _ => Checkin::failed(error.to_string(), false),
    }
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
    creds.enterprise_id = enterprise_of(client, site, &creds, &account).await;
    let pick = |key: &str| account[key].as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned);
    let name = pick("nickname").or_else(|| pick("phoneNumber")).unwrap_or_else(|| creds.uid.clone());
    Ok(SignedIn { creds, name })
}

/// The enterprise the signed-in account belongs to: the one login/account
/// names, else, for an account that isn't personal, the one WorkBuddy's
/// account list has for it, as WorkBuddy's own client picks it. None found
/// is personal.
async fn enterprise_of(client: &Client, site: &Site, creds: &Creds, who: &Value) -> String {
    if let Some(own) = who["enterpriseId"].as_str().map(str::trim).filter(|id| !id.is_empty()) {
        return own.to_owned();
    }
    let kind = who["type"].as_str().unwrap_or_default().trim().to_ascii_lowercase();
    if kind.is_empty() || kind == "personal" {
        return String::new();
    }
    let headers = [("Authorization", format!("Bearer {}", creds.access)), ("X-Domain", domain(site, creds))];
    let Ok(list) = call(client, Method::GET, &format!("{}/v2/plugin/accounts", site.api), &headers, None).await else {
        return String::new();
    };
    let enterprise = |entry: &Value| entry["enterpriseId"].as_str().map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned);
    let mine: Vec<&Value> = list["accounts"].as_array().into_iter().flatten()
        .filter(|entry| entry["uid"].as_str() == Some(creds.uid.as_str()) && enterprise(entry).is_some())
        .collect();
    let same: Vec<&Value> = mine.iter().copied().filter(|entry| entry["type"].as_str().unwrap_or_default().trim().eq_ignore_ascii_case(&kind)).collect();
    let last: Vec<&Value> = mine.iter().copied().filter(|entry| !entry["lastLogin"].is_null() && entry["lastLogin"] != false).collect();
    let pick = match (same.as_slice(), last.as_slice(), mine.as_slice()) {
        ([one], _, _) | (_, [one], _) | (_, _, [one]) => Some(*one),
        _ => None,
    };
    pick.and_then(enterprise).unwrap_or_default()
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

/// Where `needle` (ASCII-lowercased) is in `text`, from `from`, without regard
/// to ASCII case; byte offsets of `text` itself.
fn find_folded(text: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack = text.as_bytes();
    let needle = needle.as_bytes();
    (from..=haystack.len().checked_sub(needle.len())?)
        .find(|&at| text.is_char_boundary(at) && haystack[at..at + needle.len()].eq_ignore_ascii_case(needle))
}

/// `text` with FLAGGED's words in place and Claude Code's billing header
/// line gone, matched without regard to case.
fn unflagged(text: &str) -> String {
    let mut out = text.to_owned();
    while let Some(start) = find_folded(&out, "x-anthropic-billing-header:", 0) {
        let end = out[start..].find('\n').map_or(out.len(), |offset| start + offset + 1);
        out.replace_range(start..end, "");
    }
    for (flagged, instead) in FLAGGED {
        let mut from = 0;
        while let Some(found) = find_folded(&out, flagged, from) {
            // "the Codex CLI is…" already reads as the fix.
            if flagged.starts_with("Codex CLI") && found >= 4 && out[..found].to_ascii_lowercase().ends_with("the ") {
                from = found + flagged.len();
                continue;
            }
            out.replace_range(found..found + flagged.len(), instead);
            from = found + instead.len();
        }
    }
    out
}

/// FLAGGED's words out of every message of a chat; a system text that had
/// nothing else (the billing header's block) goes. Says whether it changed.
fn unflag(messages: &mut Vec<Value>) -> bool {
    let mut changed = false;
    for index in (0..messages.len()).rev() {
        let system = messages[index]["role"] == "system";
        let mut emptied = false;
        match &mut messages[index]["content"] {
            Value::String(text) => {
                let fixed = unflagged(text);
                if fixed != *text {
                    changed = true;
                    emptied = fixed.trim().is_empty();
                    *text = fixed;
                }
            }
            Value::Array(parts) => {
                let mut here = false;
                parts.retain_mut(|part| {
                    let Some(text) = part["text"].as_str() else { return true };
                    let fixed = unflagged(text);
                    if fixed == text {
                        return true;
                    }
                    here = true;
                    let keep = !fixed.trim().is_empty();
                    part["text"] = json!(fixed);
                    keep
                });
                changed |= here;
                emptied = here && parts.is_empty();
            }
            _ => {}
        }
        if emptied && system {
            messages.remove(index);
        }
    }
    changed
}

/// A chat as WorkBuddy's endpoint takes it: FLAGGED's words rewritten,
/// `max_tokens` and `system` for `max_completion_tokens` and `developer`, a
/// system message first (WorkBuddy refuses a chat whose first message isn't
/// one), tools every model behind it takes and `tool_choice` in its string
/// form. `None` when nothing changed.
pub fn shape_chat_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let messages = value.get_mut("messages")?.as_array_mut().filter(|messages| !messages.is_empty())?;
    let mut changed = unflag(messages);
    for message in messages.iter_mut().filter(|message| message["role"] == "developer") {
        message["role"] = json!("system");
        changed = true;
    }
    if !messages.first().is_some_and(|first| first["role"] == "system") {
        messages.insert(0, json!({"role": "system", "content": DEFAULT_SYSTEM}));
        changed = true;
    }
    if let Some(object) = value.as_object_mut() {
        if let Some(max) = object.remove("max_completion_tokens") {
            object.entry("max_tokens").or_insert(max);
            changed = true;
        }
    }
    changed |= chat_fit::fit_tools(&mut value).0;
    changed |= chat_fit::string_tool_choice(&mut value);
    changed.then(|| serde_json::to_vec(&value).ok()).flatten()
}

/// The names `shape_chat_body` gives the chat's tools, each to the client's own.
pub fn tool_names(body: &[u8]) -> chat_fit::Names {
    serde_json::from_slice::<Value>(body).map(|mut value| chat_fit::fit_tools(&mut value).1).unwrap_or_default()
}

/// Whether `text` holds `"code": <code>` (or `"code":"<code>"`) with nothing
/// numeric after it: 140185 is another error than 14018.
fn has_code(text: &str, code: &str) -> bool {
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    ["\"code\":", "'code':"].iter().any(|key| {
        compact.match_indices(key).any(|(at, _)| {
            let rest = compact[at + key.len()..].trim_start_matches(['"', '\'']);
            rest.strip_prefix(code).is_some_and(|after| !after.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.'))
        })
    })
}

/// What the user can do about a WorkBuddy refusal: its channel check, an
/// account restricted for unusual activity, credits used up.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status < 400 {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    let lower = text.to_lowercase();
    if lower.contains("unapproved channel") || lower.contains("illegal api invocation") {
        return Some(REFUSED_HINT.to_owned());
    }
    if has_code(&text, "11140") || lower.contains("account has been restricted due to unusual activity") || text.contains("账号存在异常行为") {
        return Some(RESTRICTED_HINT.to_owned());
    }
    if has_code(&text, "14018") || lower.contains("credits exhausted") || text.contains("额度已用尽") {
        return Some(EXHAUSTED_HINT.to_owned());
    }
    None
}

fn model(id: &str, name: &str, context: u64, reasoning: &[&str]) -> SourceModel {
    SourceModel { id: id.into(), name: name.into(), context_window: Some(context),
        reasoning: reasoning.iter().map(|level| (*level).to_owned()).collect(), ..Default::default() }
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
    fn refusals_are_explained_on_errors_only() {
        assert_eq!(explain(400, br#"{"error":{"message":"WorkBuddy AI: Illegal API invocation from an unapproved channel"}}"#).as_deref(), Some(REFUSED_HINT));
        assert_eq!(explain(403, br#"{"error":{"message":"request illegal","code":11140}}"#).as_deref(), Some(RESTRICTED_HINT));
        assert_eq!(explain(402, br#"{"code": "14018", "msg":"x"}"#).as_deref(), Some(EXHAUSTED_HINT));
        assert!(explain(400, br#"{"code":140185}"#).is_none(), "a longer code is another error");
        assert!(explain(200, b"unapproved channel").is_none());
        assert!(explain(400, br#"{"error":"first message is not system prompt"}"#).is_none());
    }

    #[test]
    fn flagged_words_spellings_and_tools_are_fitted() {
        let body = json!({
            "max_completion_tokens": 100,
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1\n"},
                    {"type": "text", "text": "You are Claude Code, Anthropic\u{2019}s official CLI for Claude."}]},
                {"role": "developer", "content": "Main branch (YOU WILL USUALLY USE THIS FOR PRS): main"},
                {"role": "user", "content": "the Codex CLI is an open source project led by OpenAI. Codex CLI is an open source project led by OpenAI."},
            ],
            "tools": [{"type": "function", "function": {"name": "a.b", "parameters": {"type": "object"}}}],
            "tool_choice": {"type": "function", "function": {"name": "a.b"}},
        });
        let shaped: Value = serde_json::from_slice(&shape_chat_body(body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(shaped["max_tokens"], 100);
        assert!(shaped.get("max_completion_tokens").is_none());
        assert_eq!(shaped["messages"][0]["content"], json!([{"type": "text", "text": "You are Claude Code, Anthropic's CLI for Claude."}]));
        assert_eq!(shaped["messages"][1], json!({"role": "system", "content": "Main branch (usually the base for PRs): main"}));
        assert_eq!(shaped["messages"][2]["content"],
            "the Codex CLI is an open source project led by OpenAI. The Codex CLI is an open source project led by OpenAI.");
        assert_eq!(shaped["tools"][0]["function"]["name"], "a_b");
        assert_eq!(shaped["tool_choice"], "a_b");
        assert_eq!(tool_names(body.to_string().as_bytes())["a_b"], "a.b");
        let header_only = json!({"messages": [{"role": "system", "content": "x-anthropic-billing-header: v"}, {"role": "user", "content": "hi"}]});
        let shaped: Value = serde_json::from_slice(&shape_chat_body(header_only.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(shaped["messages"][0]["content"], DEFAULT_SYSTEM, "a system message of the header alone goes, and the default takes its place");
    }

    #[test]
    fn a_send_keeps_its_turn_id_and_the_next_one_gets_another() {
        let first = json!({"messages": [{"role": "system", "content": "s"}, {"role": "user", "content": "fix it"}]});
        let step = json!({"messages": [{"role": "system", "content": "s"}, {"role": "user", "content": "fix it"},
            {"role": "assistant", "tool_calls": [{"id": "1", "function": {"name": "bash", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "1", "content": "ok"}, {"role": "user", "content": [{"type": "image_url", "image_url": {"url": "x"}}]}]});
        let next = json!({"messages": [{"role": "system", "content": "s"}, {"role": "user", "content": "fix it"}, {"role": "assistant", "content": "done"}, {"role": "user", "content": "thanks"}]});
        let get = |body: &Value, name: &str| conversation_headers(body.to_string().as_bytes()).into_iter().find(|(key, _)| *key == name).map(|(_, value)| value);
        assert_eq!(get(&first, "x-conversation-request-id"), get(&step, "x-conversation-request-id"), "a tool step is the same send");
        assert_ne!(get(&first, "x-conversation-request-id"), get(&next, "x-conversation-request-id"));
        assert_eq!(get(&first, "x-conversation-id"), get(&next, "x-conversation-id"));
        assert_ne!(get(&first, "x-request-id"), get(&first, "x-request-id"), "each request has its own message id");
        assert_eq!(get(&first, "x-conversation-id").unwrap().len(), 32);
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
        assert_eq!(get("x-agent-purpose"), Some("conversation"), "both builds name the client that asked");
        assert!(get("x-request-id").is_none() && get("x-enterprise-id").is_none() && get("x-agent-type").is_none());
        let team = Creds { enterprise_id: "ent-9".into(), ..creds.clone() };
        let headers = auth_headers(&site, &team);
        assert!(headers.contains(&("x-enterprise-id", "ent-9".into())) && headers.contains(&("x-tenant-id", "ent-9".into())));
        let ai = Site { kind: SourceKind::WorkbuddyAi, api: "https://www.workbuddy.ai".into(), platform: "workbuddy-ai" };
        assert!(auth_headers(&ai, &creds).iter().any(|(key, _)| *key == "x-agent-type"));
    }

    #[test]
    fn a_refreshed_token_keeps_what_the_reply_leaves_out() {
        let before = Creds { uid: "u".into(), access: "a".into(), refresh: "r".into(), domain: "d".into(), ..Default::default() };
        let after = merge(before, TokenReply { access_token: "b".into(), expires_in: 60, ..Default::default() });
        assert_eq!((after.access.as_str(), after.refresh.as_str(), after.domain.as_str(), after.uid.as_str()), ("b", "r", "d", "u"));
        assert!(after.expires_at > now_ms());
    }
}
