//! A ChatGPT plan used through OpenAI's own API, as OpenAI's Sign in with
//! ChatGPT lets an app that runs on the user's machine: the account's access
//! token goes to api.openai.com/v1/responses with the client's own
//! instructions, nothing of Codex's in it. It is separate from the Codex
//! accounts, which sign in as Codex CLI and stay Codex's.
//!
//! The sign-in registers AgentHarbor with OpenAI on the way (a public client,
//! no secret): the browser opens OpenAI's authorize page as
//! `dynamic_agent_client` and comes back with the client id OpenAI issued,
//! which the code exchange, every refresh and the sign-out use. Ported from
//! magpie (MIT, see LICENSES/THIRD_PARTY.md).
use super::store::SourceModel;
use base64::Engine as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};

const ISSUER: &str = "https://auth.openai.com";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME: &str = "AgentHarbor";
const RESOURCE: &str = "https://api.openai.com/v1";
/// Lets the token be sent to api.openai.com itself.
const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// How long before it expires a token is refreshed.
const MARGIN_SECONDS: i64 = 180;
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

/// Refusals of a refresh token that will never work again.
const DEAD: &[&str] = &["invalid_grant", "invalid_refresh_token", "token_expired", "refresh_token_expired",
    "refresh_token_invalidated", "refresh_token_reused"];

/// Responses fields a ChatGPT sign-in refuses; the request goes without them.
/// Nothing is stored, so `previous_response_id` cannot be resolved either.
const DROPPED: &[&str] = &["background", "conversation", "max_output_tokens", "max_tool_calls", "metadata",
    "moderation", "multi_agent", "prompt", "prompt_cache_retention", "safety_identifier", "temperature",
    "top_logprobs", "top_p", "truncation", "user", "previous_response_id"];
/// Tools OpenAI runs itself that it will not run for a ChatGPT token.
const HOSTED_TOOLS: &[&str] = &["image_generation", "file_search", "code_interpreter", "computer_use",
    "computer_use_preview", "computer", "mcp", "tool_search"];

fn auth_base() -> String {
    super::endpoint_override(super::SourceKind::ChatgptApi).unwrap_or_else(|| ISSUER.to_owned())
}

pub fn api_base() -> String {
    super::endpoint_override(super::SourceKind::ChatgptApi)
        .map(|base| format!("{base}/v1"))
        .unwrap_or_else(|| RESOURCE.to_owned())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub client_id: String,
    pub host_id: String,
    pub subject: String,
    pub email: String,
    pub access: String,
    pub refresh: String,
    /// Unix seconds.
    pub expires_at: i64,
}

impl Creds {
    pub fn near_expiry(&self) -> bool {
        self.expires_at - chrono::Utc::now().timestamp() <= MARGIN_SECONDS
    }
}

/// A fresh `urn:uuid:` id naming this machine to OpenAI.
pub fn new_host_id() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

pub(super) fn random_token(bytes: usize) -> String {
    let raw: Vec<u8> = (0..bytes.div_ceil(16)).flat_map(|_| *uuid::Uuid::new_v4().as_bytes()).take(bytes).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

pub struct SignIn {
    pub url: String,
    listener: TcpListener,
    redirect: String,
    state: String,
    nonce: String,
    verifier: String,
    host_id: String,
}

/// Bind the callback the browser returns to (OpenAI's usual 1455, another
/// loopback port when that one is busy) and build the authorize page.
pub async fn begin_sign_in(host_id: &str) -> Result<SignIn, String> {
    let listener = match TcpListener::bind("127.0.0.1:1455").await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind("127.0.0.1:0").await.map_err(|_| "accounts.loginFailed")?,
    };
    let port = listener.local_addr().map_err(|_| "accounts.loginFailed")?.port();
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let (state, nonce, verifier) = (random_token(24), random_token(24), random_token(48));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = reqwest::Url::parse(&format!("{}/api/accounts/authorize", auth_base())).map_err(|_| "accounts.loginFailed")?;
    url.query_pairs_mut()
        .append_pair("client_id", DYNAMIC_CLIENT)
        .append_pair("agent_name_hint", AGENT_NAME)
        .append_pair("ext_agent_host_id", host_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("resource", RESOURCE)
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(SignIn { url: url.to_string(), listener, redirect, state, nonce, verifier, host_id: host_id.to_owned() })
}

pub struct SignedIn {
    pub creds: Creds,
    pub name: String,
    pub plan: Option<String>,
}

/// Wait for the browser to come back, then trade the code for tokens.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, mut cancel: watch::Receiver<bool>) -> Result<SignedIn, String> {
    let wait = async {
        loop {
            let (mut socket, _) = sign_in.listener.accept().await.map_err(|_| "accounts.loginFailed".to_owned())?;
            let Some(query) = read_callback(&mut socket).await else {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            };
            let params: std::collections::HashMap<String, String> = reqwest::Url::parse(&format!("http://localhost/?{query}"))
                .map(|url| url.query_pairs().into_owned().collect()).unwrap_or_default();
            let result = if params.get("state") != Some(&sign_in.state) {
                Err("accounts.loginFailed".to_owned())
            } else if let Some(error) = params.get("error") {
                Err(authorize_error(error, params.get("error_description").map(String::as_str)))
            } else {
                exchange(client, &sign_in, params.get("code").map(String::as_str).unwrap_or_default(),
                    params.get("client_id").map(|id| id.trim()).unwrap_or_default()).await
            };
            let (title, detail) = match &result {
                Ok(done) => ("Signed in", format!("{} is added to AgentHarbor. You can close this tab.", done.name)),
                Err(error) if error == WORKSPACE_NOT_ENABLED => ("Sign-in didn't finish", WORKSPACE_HINT.to_owned()),
                Err(error) => ("Sign-in didn't finish", error.clone()),
            };
            let page = format!("<!doctype html><meta charset=utf-8><title>AgentHarbor</title><h1>{}</h1><p>{}</p>",
                html_escape(title), html_escape(&detail));
            let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes()).await;
            return result;
        }
    };
    tokio::select! {
        biased;
        _ = super::cancelled(&mut cancel) => Err("accounts.loginCancelled".into()),
        result = tokio::time::timeout(SIGN_IN_WINDOW, wait) => result.unwrap_or_else(|_| Err("accounts.loginTimeout".into())),
    }
}

const WORKSPACE_NOT_ENABLED: &str = "accountSources.errors.workspaceNotEnabled";
const WORKSPACE_HINT: &str = "This ChatGPT workspace has not allowed Sign in with ChatGPT apps. Ask its admin to allow them, \
    or sign in again and pick a personal workspace.";

/// OpenAI's refusal of the sign-in itself. A Team, Business or Enterprise
/// workspace lets these apps in only when its admin allows them.
fn authorize_error(error: &str, description: Option<&str>) -> String {
    let text = format!("{error} {}", description.unwrap_or_default()).to_ascii_lowercase();
    if ["organization", "organisation", "workspace", "not enabled", "admin"].iter().any(|needle| text.contains(needle)) {
        return WORKSPACE_NOT_ENABLED.into();
    }
    format!("ChatGPT sign-in: {}", description.unwrap_or(error))
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The query of a `GET /auth/callback` request, `None` for anything else.
async fn read_callback(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 16 * 1024 {
        let read = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut chunk)).await.ok()?.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer);
    let target = head.lines().next()?.strip_prefix("GET ")?.split(' ').next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (path == "/auth/callback").then(|| query.to_owned())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TokenReply {
    access_token: String,
    refresh_token: String,
    id_token: String,
    expires_in: f64,
    scope: String,
}

#[derive(Debug)]
struct TokenError {
    code: String,
    message: String,
}

async fn post_token(client: &Client, form: &[(&str, &str)]) -> Result<TokenReply, TokenError> {
    let response = client.post(format!("{}/api/accounts/oauth/token", auth_base()))
        .header("Accept", "application/json")
        .form(form)
        .timeout(Duration::from_secs(30))
        .send().await
        .map_err(|_| TokenError { code: String::new(), message: "cannot reach OpenAI".into() })?;
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if !status.is_success() {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = value["error"].as_str().or_else(|| value["error"]["code"].as_str()).or_else(|| value["code"].as_str())
            .unwrap_or_default().to_owned();
        let message = value["error_description"].as_str().or_else(|| value["error"]["message"].as_str())
            .map(str::to_owned).unwrap_or_else(|| status.to_string());
        return Err(TokenError { code, message });
    }
    serde_json::from_slice(&bytes).map_err(|_| TokenError { code: String::new(), message: "unreadable token answer".into() })
}

/// The claims of a JWT, unverified: the token came straight from OpenAI over TLS.
pub fn claims(token: &str) -> Value {
    token.split('.').nth(1)
        .and_then(|part| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn with_tokens(mut creds: Creds, reply: TokenReply) -> Result<Creds, String> {
    if reply.access_token.is_empty() || reply.refresh_token.is_empty() || reply.expires_in <= 0.0 {
        return Err("OpenAI sent no usable token".into());
    }
    if !reply.scope.split_whitespace().any(|scope| scope == DIRECT_SCOPE) {
        return Err("accountSources.errors.chatgptNotEligible".into());
    }
    creds.access = reply.access_token;
    creds.refresh = reply.refresh_token;
    creds.expires_at = chrono::Utc::now().timestamp() + reply.expires_in as i64;
    Ok(creds)
}

/// The ID token belongs to this sign-in: OpenAI's, for the issued client, with
/// this sign-in's nonce, not expired.
fn check_id(id: &Value, client_id: &str, nonce: &str) -> Result<(), String> {
    if id["iss"].as_str().map(|iss| iss.trim_end_matches('/')) != Some(ISSUER) {
        return Err("the ID token is not OpenAI's".into());
    }
    let audience = match &id["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(list) => list.iter().any(|aud| aud == client_id),
        _ => false,
    };
    if !audience {
        return Err("the ID token is for another app".into());
    }
    if id["nonce"].as_str() != Some(nonce) {
        return Err("the ID token is not this sign-in's".into());
    }
    if id["exp"].as_i64().is_some_and(|exp| exp < chrono::Utc::now().timestamp() - 60) {
        return Err("the ID token has expired".into());
    }
    Ok(())
}

/// The ChatGPT plan a token names.
pub fn plan_of(token: &str) -> Option<String> {
    claims(token)["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str()
        .filter(|plan| !plan.is_empty()).map(str::to_owned)
}

async fn exchange(client: &Client, sign_in: &SignIn, code: &str, client_id: &str) -> Result<SignedIn, String> {
    if client_id.is_empty() {
        return Err("accountSources.errors.chatgptNoClient".into());
    }
    let reply = post_token(client, &[("grant_type", "authorization_code"), ("client_id", client_id), ("code", code),
        ("code_verifier", &sign_in.verifier), ("redirect_uri", &sign_in.redirect), ("resource", RESOURCE)]).await
        .map_err(|error| format!("ChatGPT sign-in: {} {}", error.code, error.message).trim().to_owned())?;
    let id_token = reply.id_token.clone();
    if id_token.is_empty() {
        return Err("ChatGPT sign-in: OpenAI sent no ID token".into());
    }
    let mut creds = with_tokens(Creds { client_id: client_id.into(), host_id: sign_in.host_id.clone(), ..Default::default() }, reply)?;
    let id = claims(&id_token);
    check_id(&id, client_id, &sign_in.nonce).map_err(|error| format!("ChatGPT sign-in: {error}"))?;
    creds.subject = id["sub"].as_str().unwrap_or_default().to_owned();
    creds.email = id["email"].as_str().unwrap_or_default().to_owned();
    let name = [creds.email.as_str(), creds.subject.as_str()].into_iter().find(|value| !value.is_empty())
        .ok_or("ChatGPT sign-in: OpenAI did not say which account signed in")?.to_owned();
    let plan = plan_of(&id_token).or_else(|| plan_of(&creds.access));
    Ok(SignedIn { creds, name, plan })
}

pub enum RefreshError {
    SignedOut(String),
    Transient(String),
}

/// Refresh the access token; the refresh token rotates with it.
pub async fn refresh(client: &Client, creds: &Creds) -> Result<Creds, RefreshError> {
    match post_token(client, &[("grant_type", "refresh_token"), ("client_id", &creds.client_id),
        ("refresh_token", &creds.refresh), ("resource", RESOURCE)]).await
    {
        Ok(reply) => with_tokens(creds.clone(), reply).map_err(RefreshError::Transient),
        Err(error) if DEAD.contains(&error.code.as_str()) || error.code == "invalid_client" =>
            Err(RefreshError::SignedOut(format!("{} {}", error.code, error.message))),
        Err(error) => Err(RefreshError::Transient(format!("ChatGPT token refresh: {}", error.message))),
    }
}

/// Have OpenAI revoke the refresh token, at the endpoint its discovery names.
pub async fn revoke(client: &Client, creds: &Creds) {
    let discovery = client.get(format!("{}/.well-known/openid-configuration", auth_base()))
        .timeout(Duration::from_secs(15)).send().await;
    let Ok(discovery) = discovery else { return };
    let Ok(document) = discovery.json::<Value>().await else { return };
    let Some(endpoint) = document["revocation_endpoint"].as_str() else { return };
    let _ = client.post(endpoint)
        .form(&[("token", creds.refresh.as_str()), ("token_type_hint", "refresh_token"), ("client_id", creds.client_id.as_str())])
        .timeout(Duration::from_secs(15)).send().await;
}

/// The account's models from `/v1/models` (Codex's `models[]` shape or the
/// API's `data[]`), with the Codex catalog's listed models added: the token
/// is served as Codex on the plan and runs its newest models too.
pub async fn fetch_models(client: &Client, creds: &Creds, codex: &[Value]) -> Result<Vec<SourceModel>, String> {
    let response = client.get(format!("{}/models", api_base()))
        .bearer_auth(&creds.access).timeout(Duration::from_secs(30)).send().await
        .map_err(|_| "cannot reach OpenAI".to_owned())?;
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if !status.is_success() {
        return Err(explain(status.as_u16(), &bytes).unwrap_or_else(|| format!("ChatGPT models: {status}")));
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let (mut models, hidden) = parse_models(&value);
    for entry in codex {
        let Some(slug) = entry["slug"].as_str().filter(|slug| !slug.is_empty() && !slug.contains('/')) else { continue };
        if entry["visibility"].as_str().is_some_and(|visibility| visibility != "list")
            || hidden.iter().any(|id| id == slug) || models.iter().any(|model| model.id == slug) {
            continue;
        }
        models.push(model_of(entry, slug));
    }
    if models.is_empty() {
        return Err("accountSources.errors.chatgptNoModels".into());
    }
    Ok(models)
}

fn model_of(entry: &Value, id: &str) -> SourceModel {
    SourceModel {
        id: id.to_owned(),
        name: entry["display_name"].as_str().filter(|name| !name.is_empty()).unwrap_or(id).to_owned(),
        context_window: entry["context_window"].as_u64().filter(|value| *value > 0),
        reasoning: entry["supported_reasoning_levels"].as_array().into_iter().flatten()
            .filter_map(|level| level["effort"].as_str().map(str::to_owned)).collect(),
        ..Default::default()
    }
}

fn parse_models(value: &Value) -> (Vec<SourceModel>, Vec<String>) {
    let mut models = Vec::new();
    let mut hidden = Vec::new();
    for entry in value["models"].as_array().into_iter().flatten() {
        let Some(slug) = entry["slug"].as_str().filter(|slug| !slug.is_empty()) else { continue };
        if entry["visibility"].as_str().is_some_and(|visibility| visibility != "list") {
            hidden.push(slug.to_owned());
        } else {
            models.push(model_of(entry, slug));
        }
    }
    for entry in value["data"].as_array().into_iter().flatten() {
        if let Some(id) = entry["id"].as_str().filter(|id| !id.is_empty() && !models.iter().any(|model| model.id == *id)) {
            models.push(model_of(entry, id));
        }
    }
    (models, hidden)
}

/// A Responses request the ChatGPT sign-in takes: stored nowhere, without the
/// fields and hosted tools it refuses, and a system message sent as the
/// developer one it takes instead. The instructions stay the client's own.
pub fn shape_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object_mut()?;
    for key in DROPPED {
        object.remove(*key);
    }
    object.insert("store".into(), json!(false));
    match object.get_mut("input") {
        Some(Value::String(text)) => {
            let text = std::mem::take(text);
            object.insert("input".into(), json!([{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}]));
        }
        Some(Value::Array(items)) => {
            for item in items.iter_mut() {
                if item["role"] == "system" {
                    item["role"] = json!("developer");
                }
            }
        }
        _ => {}
    }
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.retain(|tool| !tool["type"].as_str().is_some_and(|kind| HOSTED_TOOLS.contains(&kind)));
        if tools.is_empty() {
            object.remove("tools");
            object.remove("tool_choice");
        }
    }
    serde_json::to_vec(&value).ok()
}

/// What the user can do about one of OpenAI's refusals of a ChatGPT sign-in.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = value["error"]["code"].as_str().or_else(|| value["detail"]["code"].as_str())
        .or_else(|| value["error"]["type"].as_str()).unwrap_or_default();
    let param = value["error"]["param"].as_str().or_else(|| value["detail"]["param"].as_str()).unwrap_or_default();
    Some(match code {
        "subscription_sharing_usage_limit_exceeded" => format!(
            "the ChatGPT plan's usage limit, shared with ChatGPT and the other apps it is used in, is reached; see {USAGE_URL}"),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" =>
            "OpenAI can't tell the account's usage right now; try again in a moment".into(),
        "subscription_sharing_user_not_eligible" => "this ChatGPT account can't be used through OpenAI's API (its plan isn't eligible)".into(),
        "subscription_sharing_unsupported_capability" if !param.is_empty() => format!("a ChatGPT sign-in can't use {param} through OpenAI's API"),
        "subscription_sharing_unsupported_capability" => "a ChatGPT sign-in can't use part of this request through OpenAI's API".into(),
        "subscription_sharing_route_not_supported" => "a ChatGPT sign-in can't use this endpoint of OpenAI's API".into(),
        "subscription_sharing_invalid_user" => "OpenAI no longer takes this ChatGPT sign-in; sign in again".into(),
        _ if status == 401 => "OpenAI turned the ChatGPT sign-in away; sign in again".into(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        let part = |value: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.sig", part(&json!({"alg":"none"})), part(&claims))
    }

    #[test]
    fn the_id_token_must_be_this_sign_ins() {
        let future = chrono::Utc::now().timestamp() + 600;
        let good = json!({"iss":"https://auth.openai.com/","aud":["client-1"],"nonce":"n1","exp":future});
        assert!(check_id(&good, "client-1", "n1").is_ok());
        assert!(check_id(&json!({"iss":"https://evil.example","aud":"client-1","nonce":"n1"}), "client-1", "n1").is_err());
        assert!(check_id(&json!({"iss":ISSUER,"aud":"other","nonce":"n1"}), "client-1", "n1").is_err());
        assert!(check_id(&json!({"iss":ISSUER,"aud":"client-1","nonce":"n2"}), "client-1", "n1").is_err());
        assert!(check_id(&json!({"iss":ISSUER,"aud":"client-1","nonce":"n1","exp":1}), "client-1", "n1").is_err());
        assert_eq!(claims(&jwt(json!({"sub":"s"})))["sub"], "s");
        assert_eq!(plan_of(&jwt(json!({"https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}))).as_deref(), Some("plus"));
    }

    #[test]
    fn a_token_without_the_direct_scope_is_refused() {
        let reply = |scope: &str| TokenReply { access_token: "a".into(), refresh_token: "r".into(), expires_in: 3600.0, scope: scope.into(), ..Default::default() };
        assert_eq!(with_tokens(Creds::default(), reply("openid")).unwrap_err(), "accountSources.errors.chatgptNotEligible");
        let creds = with_tokens(Creds::default(), reply(SCOPES)).unwrap();
        assert_eq!((creds.access.as_str(), creds.refresh.as_str()), ("a", "r"));
        assert!(!creds.near_expiry());
    }

    #[test]
    fn requests_drop_what_the_sign_in_refuses() {
        let shaped = shape_body(json!({
            "model":"gpt-5.5","input":[{"role":"system","content":"s"},{"role":"user","content":"u"}],
            "previous_response_id":"r1","temperature":0.2,"max_output_tokens":10,"store":true,
            "tools":[{"type":"function","name":"f"},{"type":"image_generation"}],"tool_choice":"auto"
        }).to_string().as_bytes()).unwrap();
        let value: Value = serde_json::from_slice(&shaped).unwrap();
        assert_eq!(value["store"], false);
        assert_eq!(value["input"][0]["role"], "developer");
        for key in ["previous_response_id", "temperature", "max_output_tokens"] {
            assert!(value.get(key).is_none(), "{key}");
        }
        assert_eq!(value["tools"], json!([{"type":"function","name":"f"}]));
        assert_eq!(value["tool_choice"], "auto");
        let only_hosted: Value = serde_json::from_slice(&shape_body(br#"{"input":"hi","tools":[{"type":"mcp"}],"tool_choice":"auto"}"#).unwrap()).unwrap();
        assert!(only_hosted.get("tools").is_none() && only_hosted.get("tool_choice").is_none());
        assert_eq!(only_hosted["input"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn models_come_from_either_list_shape_and_codex_fills_the_gaps() {
        let (models, hidden) = parse_models(&json!({"models":[
            {"slug":"gpt-5.5","display_name":"GPT-5.5","context_window":272000,"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},
            {"slug":"hidden-one","visibility":"hide"}]}));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].reasoning, ["low", "high"]);
        assert_eq!(hidden, ["hidden-one"]);
        let (models, _) = parse_models(&json!({"data":[{"id":"gpt-5.4"}]}));
        assert_eq!(models[0].name, "gpt-5.4");
    }

    #[test]
    fn refusals_say_what_to_do() {
        assert!(explain(429, br#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#).unwrap().contains(USAGE_URL));
        assert!(explain(400, br#"{"detail":{"code":"subscription_sharing_unsupported_capability","param":"reasoning.summary"}}"#).unwrap().contains("reasoning.summary"));
        assert!(explain(401, b"{}").is_some());
        assert!(explain(500, b"{}").is_none());
    }

    #[test]
    fn a_workspace_that_has_not_allowed_the_app_is_named() {
        assert_eq!(authorize_error("access_denied", Some("Your organization has not enabled access to this application")),
            WORKSPACE_NOT_ENABLED);
        assert_eq!(authorize_error("unauthorized_client", Some("Workspace admin approval required")), WORKSPACE_NOT_ENABLED);
        assert_eq!(authorize_error("access_denied", Some("The user denied the request")), "ChatGPT sign-in: The user denied the request");
        assert_eq!(authorize_error("server_error", None), "ChatGPT sign-in: server_error");
    }
}
