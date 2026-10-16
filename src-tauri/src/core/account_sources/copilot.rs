//! GitHub Copilot: a Copilot plan's models through Copilot's own API. The
//! account signs in with GitHub's device flow under the OAuth app Copilot's
//! editors use; its GitHub token is traded for a short-lived Copilot token,
//! and requests carry the editor headers Copilot's API expects. Models whose
//! terms the account has not accepted yet are left out: they are enabled in
//! GitHub's Copilot settings, never by AgentHarbor. Ported from magpie (MIT,
//! see LICENSES/THIRD_PARTY.md).
use super::store::{Quota, SourceModel};
use crate::core::router::ModelProtocol;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::sync::watch;

/// The OAuth app of Copilot's editors.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB: &str = "https://github.com";
const GITHUB_API: &str = "https://api.github.com";
const COPILOT_API: &str = "https://api.githubcopilot.com";
/// Where a model's terms are accepted and the premium allowance is shown.
pub const SETTINGS_URL: &str = "https://github.com/settings/copilot/features";
const EDITOR_HEADERS: [(&str, &str); 4] = [
    ("editor-version", "vscode/1.140.0"),
    ("editor-plugin-version", "copilot-chat/0.68.0"),
    ("copilot-integration-id", "vscode-chat"),
    ("user-agent", "GitHubCopilotChat/0.68.0"),
];
/// Copilot Chat's API version, sent on every chat request.
const API_VERSION: &str = "2026-01-09";
/// A Copilot token this close to expiry is traded again first.
const MARGIN_SECONDS: i64 = 120;

fn github() -> String {
    super::endpoint_override(super::SourceKind::Copilot).unwrap_or_else(|| GITHUB.to_owned())
}

fn github_api() -> String {
    super::endpoint_override(super::SourceKind::Copilot).unwrap_or_else(|| GITHUB_API.to_owned())
}

/// Copilot's API, before a token names its own.
pub fn api_base() -> String {
    super::endpoint_override(super::SourceKind::Copilot).unwrap_or_else(|| COPILOT_API.to_owned())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    /// The GitHub OAuth token; it does not expire until revoked.
    pub token: String,
    pub login: String,
}

/// A device code the user types at GitHub.
pub struct DeviceCode {
    device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    interval: Duration,
    expires_in: Duration,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DeviceReply {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

async fn post_form(client: &Client, url: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    let response = client.post(url).header("accept", "application/json").form(form)
        .timeout(Duration::from_secs(30)).send().await.map_err(|_| "accountSources.errors.copilotUnreachable".to_owned())?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("GitHub: {}", value["error_description"].as_str().or(value["error"].as_str()).unwrap_or(status.as_str())));
    }
    Ok(value)
}

/// Ask GitHub for a device code.
pub async fn begin_sign_in(client: &Client) -> Result<DeviceCode, String> {
    let value = post_form(client, &format!("{}/login/device/code", github()), &[("client_id", CLIENT_ID), ("scope", "read:user")]).await?;
    let reply: DeviceReply = serde_json::from_value(value).unwrap_or_default();
    if reply.device_code.is_empty() || reply.user_code.is_empty() {
        return Err("accountSources.errors.copilotNoCode".into());
    }
    Ok(DeviceCode {
        device_code: reply.device_code,
        user_code: reply.user_code,
        verification_uri: if reply.verification_uri.is_empty() { format!("{}/login/device", github()) } else { reply.verification_uri },
        interval: Duration::from_secs(reply.interval),
        expires_in: Duration::from_secs(if reply.expires_in == 0 { 900 } else { reply.expires_in }),
    })
}

pub struct SignedIn {
    pub creds: Creds,
    pub plan: Option<String>,
}

/// Wait for the user to enter the code at GitHub, then read the account.
/// `floor` is the shortest wait between polls.
pub async fn finish_sign_in(client: &Client, code: DeviceCode, mut cancel: watch::Receiver<bool>, floor: Duration) -> Result<SignedIn, String> {
    let mut interval = code.interval.max(floor);
    let poll = async {
        loop {
            tokio::time::sleep(interval).await;
            let reply = post_form(client, &format!("{}/login/oauth/access_token", github()), &[
                ("client_id", CLIENT_ID),
                ("device_code", &code.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]).await;
            // A hiccup is asked again.
            let Ok(reply) = reply else { continue };
            match reply["error"].as_str().unwrap_or_default() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += Duration::from_secs(5).max(floor);
                    continue;
                }
                "expired_token" => return Err("accounts.loginTimeout".to_owned()),
                "access_denied" => return Err("accounts.loginCancelled".to_owned()),
                "" => {}
                other => return Err(format!("GitHub: {other}")),
            }
            let token = reply["access_token"].as_str().unwrap_or_default().to_owned();
            if token.is_empty() {
                continue;
            }
            let (login, plan) = account(client, &token).await?;
            return Ok(SignedIn { creds: Creds { token, login }, plan });
        }
    };
    tokio::select! {
        biased;
        _ = super::cancelled(&mut cancel) => Err("accounts.loginCancelled".into()),
        result = tokio::time::timeout(code.expires_in, poll) => result.unwrap_or_else(|_| Err("accounts.loginTimeout".into())),
    }
}

async fn github_get(client: &Client, url: &str, token: &str) -> Result<Value, u16> {
    let response = client.get(url).header("authorization", format!("token {token}")).header("accept", "application/json")
        .headers(editor_headers()).timeout(Duration::from_secs(30)).send().await.map_err(|_| 0u16)?;
    let status = response.status();
    if !status.is_success() {
        return Err(status.as_u16());
    }
    response.json().await.map_err(|_| 0u16)
}

fn editor_headers() -> reqwest::header::HeaderMap {
    EDITOR_HEADERS.iter().filter_map(|(name, value)| Some((
        reqwest::header::HeaderName::from_static(name), reqwest::header::HeaderValue::from_static(value),
    ))).collect()
}

/// The plan's name as Copilot's own pages give it.
fn plan_label(plan: &str, sku: &str) -> Option<String> {
    match sku {
        "free_educational_quota" => return Some("Education".into()),
        "free_limited_copilot" => return Some("Free".into()),
        _ => {}
    }
    let known = match plan.to_ascii_lowercase().as_str() {
        "free" => "Free",
        "individual" => "Pro",
        "individual_pro" => "Pro+",
        "individual_max" => "Max",
        "individual_edu" => "Education",
        "business" => "Business",
        "enterprise" => "Enterprise",
        "" => return None,
        _ => return Some(plan.to_owned()),
    };
    Some(known.into())
}

/// Whose GitHub token this is, and the Copilot plan it has; no plan, no Copilot.
async fn account(client: &Client, token: &str) -> Result<(String, Option<String>), String> {
    let user = github_get(client, &format!("{}/user", github_api()), token).await
        .map_err(|_| "accountSources.errors.copilotNoUser".to_owned())?;
    let login = user["login"].as_str().unwrap_or_default().to_owned();
    if login.is_empty() {
        return Err("accountSources.errors.copilotNoUser".into());
    }
    let copilot = github_get(client, &format!("{}/copilot_internal/user", github_api()), token).await.map_err(|status| {
        if matches!(status, 401 | 403 | 404) { "accountSources.errors.copilotNoPlan".to_owned() } else { "accountSources.errors.copilotUnreachable".to_owned() }
    })?;
    Ok((login, plan_label(copilot["copilot_plan"].as_str().unwrap_or_default(), copilot["access_type_sku"].as_str().unwrap_or_default())))
}

/// The premium-request allowance of this billing cycle.
pub async fn quota(client: &Client, creds: &Creds) -> Quota {
    let checked_at = chrono::Utc::now().timestamp();
    match github_get(client, &format!("{}/copilot_internal/user", github_api()), &creds.token).await {
        Ok(value) => {
            let premium = &value["quota_snapshots"]["premium_interactions"];
            let plan = value["copilot_plan"].as_str().unwrap_or_default();
            if premium["unlimited"].as_bool() == Some(true) || premium.is_null() {
                return Quota { unit: "premium".into(), paid: Some(plan != "free"), checked_at, ..Default::default() };
            }
            let limit = premium["entitlement"].as_f64().unwrap_or_default();
            let remaining = premium["remaining"].as_f64().unwrap_or(limit).clamp(0.0, limit.max(0.0));
            Quota { used: limit - remaining, limit, unit: "premium".into(), paid: Some(plan != "free"), checked_at, error: None, remaining: None }
        }
        Err(_) => Quota { checked_at, error: Some("Copilot did not report its allowance".into()), ..Default::default() },
    }
}

/// A Copilot token for one stretch of requests.
#[derive(Clone)]
pub struct Session {
    token: String,
    expires_at: i64,
    /// The API this account's requests go to.
    pub api: String,
}

/// Copilot tokens by GitHub token: one trade serves every request until it nears expiry.
static SESSIONS: Mutex<Option<HashMap<String, Session>>> = Mutex::new(None);

pub enum SessionError {
    /// GitHub refuses this token for Copilot: sign in again.
    SignedOut(String),
    Transient,
}

/// The account's Copilot token, traded anew when near expiry.
pub async fn session(client: &Client, creds: &Creds) -> Result<Session, SessionError> {
    let now = chrono::Utc::now().timestamp();
    if let Some(session) = SESSIONS.lock().ok().and_then(|sessions| sessions.as_ref()?.get(&creds.token).cloned()) {
        if session.expires_at - now > MARGIN_SECONDS {
            return Ok(session);
        }
    }
    let value = github_get(client, &format!("{}/copilot_internal/v2/token", github_api()), &creds.token).await.map_err(|status| match status {
        401 => SessionError::SignedOut("GitHub signed this account out".into()),
        403 | 404 => SessionError::SignedOut("this GitHub account has no Copilot plan".into()),
        _ => SessionError::Transient,
    })?;
    let token = value["token"].as_str().unwrap_or_default().to_owned();
    if token.is_empty() {
        return Err(SessionError::Transient);
    }
    let api = value["endpoints"]["api"].as_str().filter(|api| !api.is_empty()).map(|api| api.trim_end_matches('/').to_owned())
        .filter(|_| super::endpoint_override(super::SourceKind::Copilot).is_none())
        .unwrap_or_else(api_base);
    let session = Session { token, expires_at: value["expires_at"].as_i64().unwrap_or(now + 600), api };
    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.get_or_insert_with(HashMap::new).insert(creds.token.clone(), session.clone());
    }
    Ok(session)
}

/// Forget a Copilot token Copilot refused, so the next request trades again.
pub fn forget(creds: &Creds) {
    if let Ok(mut sessions) = SESSIONS.lock() {
        if let Some(sessions) = sessions.as_mut() {
            sessions.remove(&creds.token);
        }
    }
}

/// The headers every request with this session carries.
pub fn request_headers(session: &Session) -> Vec<(&'static str, String)> {
    let mut headers = vec![("authorization", format!("Bearer {}", session.token))];
    headers.extend(EDITOR_HEADERS.iter().map(|(name, value)| (*name, (*value).to_owned())));
    headers.push(("x-github-api-version", API_VERSION.to_owned()));
    headers.push(("openai-intent", "conversation-panel".to_owned()));
    headers
}

/// The role of the last turn: a turn the user typed is billed as one, a
/// tool's reply is not. Tool results ride in a user message in Messages and
/// have no role in Responses.
fn last_role(body: &Value) -> &str {
    if let Some(last) = body["messages"].as_array().and_then(|messages| messages.last()) {
        let role = last["role"].as_str().unwrap_or_default();
        let blocks = last["content"].as_array();
        if role == "user" && blocks.is_some_and(|blocks| !blocks.is_empty() && blocks.iter().all(|block| block["type"] == "tool_result")) {
            return "tool";
        }
        return role;
    }
    match &body["input"] {
        Value::String(_) => "user",
        Value::Array(items) => items.last().and_then(|item| item["role"].as_str()).unwrap_or_default(),
        _ => "",
    }
}

/// Headers that depend on the request: who started the turn, and whether it holds an image.
pub fn body_headers(protocol: ModelProtocol, body: &[u8]) -> Vec<(&'static str, String)> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else { return Vec::new() };
    let mut headers = vec![("x-initiator", if last_role(&value) == "user" { "user" } else { "agent" }.to_owned())];
    let text = String::from_utf8_lossy(body);
    if ["\"image_url\"", "\"input_image\"", "\"type\":\"image\""].iter().any(|needle| text.contains(needle)) {
        headers.push(("copilot-vision-request", "true".to_owned()));
    }
    if protocol == ModelProtocol::AnthropicMessages {
        headers.push(("anthropic-version", "2023-06-01".to_owned()));
    }
    headers
}

/// Where a model is asked: Messages for a model Copilot serves there, then
/// Responses, else Chat (a model listing no endpoints is Chat-only).
fn protocol_of(endpoints: &[Value]) -> ModelProtocol {
    let has = |needle: &str| endpoints.iter().filter_map(Value::as_str).any(|endpoint| {
        endpoint.trim_end_matches('/').trim_start_matches("/v1").trim_start_matches('/') == needle
    });
    if has("messages") {
        ModelProtocol::AnthropicMessages
    } else if has("responses") {
        ModelProtocol::Responses
    } else {
        ModelProtocol::ChatCompletions
    }
}

/// Copilot's own helper models, which no picker offers.
fn internal(id: &str) -> bool {
    ["copilot-search", "exec-agent", "trajectory"].iter().any(|prefix| id.starts_with(prefix))
        || ["-secondary", "-tertiary", "-4th", "-free-auto"].iter().any(|suffix| id.ends_with(suffix))
}

/// The chat models this account may call now.
pub(crate) fn parse_models(value: &Value) -> Vec<SourceModel> {
    let mut models: Vec<SourceModel> = Vec::new();
    for model in value["data"].as_array().into_iter().flatten() {
        let id = model["id"].as_str().unwrap_or_default();
        let picker = model["model_picker_enabled"].as_bool().unwrap_or(false);
        let category = model["model_picker_category"].as_str().unwrap_or_default();
        if id.is_empty() || model["capabilities"]["type"] != "chat" || internal(id) || model["vendor"] == "Experimental"
            || (!picker && category.is_empty()) || models.iter().any(|existing| existing.id == id)
        {
            continue;
        }
        let enabled = match model["policy"]["state"].as_str() {
            Some(state) => state == "enabled",
            None => picker,
        };
        if !enabled {
            continue;
        }
        let limits = &model["capabilities"]["limits"];
        models.push(SourceModel {
            id: id.to_owned(),
            name: model["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(id).to_owned(),
            context_window: limits["max_context_window_tokens"].as_u64(),
            max_output_tokens: limits["max_output_tokens"].as_u64(),
            reasoning: model["capabilities"]["supports"]["reasoning_effort"].as_array().into_iter().flatten()
                .filter_map(|effort| effort.as_str().map(str::to_owned)).collect(),
            protocol: Some(protocol_of(model["supported_endpoints"].as_array().map(Vec::as_slice).unwrap_or_default())),
        });
    }
    models
}

/// Ask Copilot which chat models this account may call.
pub async fn fetch_models(client: &Client, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let session = session(client, creds).await.map_err(|error| match error {
        SessionError::SignedOut(_) => "accountSources.errors.signedOut".to_owned(),
        SessionError::Transient => "accountSources.errors.copilotUnreachable".to_owned(),
    })?;
    let mut request = client.get(format!("{}/models", session.api)).timeout(Duration::from_secs(30));
    for (name, value) in request_headers(&session) {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|_| "accountSources.errors.copilotUnreachable".to_owned())?;
    if !response.status().is_success() {
        return Err("accountSources.errors.copilotUnreachable".into());
    }
    let value: Value = response.json().await.map_err(|_| "accountSources.errors.copilotUnreachable".to_owned())?;
    let models = parse_models(&value);
    if models.is_empty() {
        return Err("accountSources.errors.copilotNoModels".into());
    }
    Ok(models)
}

/// What the user can do about a refusal Copilot's own words leave unclear.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    if status == 400 && text.contains("no model endpoints available") {
        return Some("Copilot does not serve this model on this API; it is not about the plan".into());
    }
    if status == 400 && (text.contains("model_not_supported") || text.contains("requested model is not supported")) {
        return Some(format!("Copilot does not serve this model to this account's plan; pick another model, or enable it at {SETTINGS_URL} and refresh the models"));
    }
    match status {
        401 => Some("Copilot refused the account's token; refresh the models or sign in again on the Accounts page".into()),
        403 => Some(format!("Copilot refused this account (its plan or an organization policy); check {SETTINGS_URL}")),
        429 => Some(format!("Copilot's rate limit or premium-request allowance is reached; see {SETTINGS_URL}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn models_keep_what_the_account_may_call_on_its_best_api() {
        let models = parse_models(&json!({"data":[
            {"id":"claude-sonnet-4.6","name":"Claude Sonnet 4.6","vendor":"Anthropic","model_picker_enabled":true,
             "capabilities":{"type":"chat","limits":{"max_context_window_tokens":200000,"max_output_tokens":32000},"supports":{}},
             "supported_endpoints":["/chat/completions","/v1/messages"],"policy":{"state":"enabled"}},
            {"id":"gpt-5.5","name":"GPT-5.5","model_picker_enabled":true,"capabilities":{"type":"chat","supports":{"reasoning_effort":["low","high"]}},
             "supported_endpoints":["/responses","ws:/responses"]},
            {"id":"gpt-4.1","name":"GPT-4.1","model_picker_enabled":true,"capabilities":{"type":"chat"}},
            {"id":"gemini-3-pro","model_picker_enabled":true,"capabilities":{"type":"chat"},"policy":{"state":"unconfigured","terms":"…"}},
            {"id":"gpt-4o-mini-free-auto","model_picker_enabled":true,"capabilities":{"type":"chat"}},
            {"id":"text-embedding-3","capabilities":{"type":"embeddings"},"model_picker_enabled":true},
            {"id":"old-snapshot","capabilities":{"type":"chat"}},
            {"id":"gpt-4.1","name":"duplicate","model_picker_enabled":true,"capabilities":{"type":"chat"}}
        ]}));
        let ids: Vec<_> = models.iter().map(|model| (model.id.as_str(), model.protocol)).collect();
        assert_eq!(ids, [
            ("claude-sonnet-4.6", Some(ModelProtocol::AnthropicMessages)),
            ("gpt-5.5", Some(ModelProtocol::Responses)),
            ("gpt-4.1", Some(ModelProtocol::ChatCompletions)),
        ], "terms not accepted, helpers, embeddings and unpicked snapshots are left out");
        assert_eq!((models[0].context_window, models[0].max_output_tokens), (Some(200_000), Some(32_000)));
        assert_eq!(models[1].reasoning, ["low", "high"]);
    }

    #[test]
    fn a_typed_turn_is_the_users_and_a_tool_reply_the_agents() {
        let headers = |body: Value, protocol| body_headers(protocol, body.to_string().as_bytes());
        let initiator = |headers: Vec<(&str, String)>| headers.into_iter().find(|(name, _)| *name == "x-initiator").unwrap().1;
        assert_eq!(initiator(headers(json!({"messages":[{"role":"user","content":"hi"}]}), ModelProtocol::ChatCompletions)), "user");
        assert_eq!(initiator(headers(json!({"messages":[{"role":"tool","content":"42"}]}), ModelProtocol::ChatCompletions)), "agent");
        assert_eq!(initiator(headers(json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t"}]}]}),
            ModelProtocol::AnthropicMessages)), "agent");
        assert_eq!(initiator(headers(json!({"input":"hi"}), ModelProtocol::Responses)), "user");
        assert_eq!(initiator(headers(json!({"input":[{"type":"function_call_output","call_id":"c"}]}), ModelProtocol::Responses)), "agent");
        let vision = headers(json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:"}}]}]}), ModelProtocol::ChatCompletions);
        assert!(vision.iter().any(|(name, value)| *name == "copilot-vision-request" && value == "true"));
        assert!(headers(json!({"messages":[]}), ModelProtocol::AnthropicMessages).iter().any(|(name, _)| *name == "anthropic-version"));
    }

    #[test]
    fn plans_read_as_copilot_names_them() {
        assert_eq!(plan_label("individual", "").as_deref(), Some("Pro"));
        assert_eq!(plan_label("individual", "free_educational_quota").as_deref(), Some("Education"));
        assert_eq!(plan_label("", ""), None);
        assert!(explain(400, br#"{"error":{"code":"model_not_supported"}}"#).unwrap().contains(SETTINGS_URL));
        assert!(explain(500, b"{}").is_none());
    }
}
