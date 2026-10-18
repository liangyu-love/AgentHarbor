//! Cursor through its CLI's sign-in (`cursor-agent login`). The CLI keeps one
//! sign-in for the machine and renews its token; AgentHarbor reads that token
//! where the CLI keeps it and never stores it. Requests go to the agent API
//! the CLI talks to: one Run is a Connect stream both ways over HTTP/2, the
//! conversation sent as AI SDK messages in blobs the server asks for, the
//! caller's tools offered as MCP tools whose calls end the Run and go back to
//! the caller. The router speaks Chat to it, like to any relay.
use super::cli;
use super::store::{Quota, SourceModel};
use super::wire::{Event, Writer};
use crate::core::models::{ApiProxyConfigPayload, ApiProxyMode};
use base64::Engine;
use futures_util::StreamExt;
use hyper::body::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub const API: &str = "https://api2.cursor.sh";
/// Where Cursor shows the plan's usage in full.
pub const USAGE_URL: &str = "https://cursor.com/dashboard";
/// The agent API when the server config names none.
const AGENT: &str = "https://agentn.global.api5.cursor.sh";
const VERSION_FALLBACK: &str = "2026.09.23-86fc751";
/// The MCP server the caller's tools are offered under.
const SERVER: &str = "agentharbor";
/// How the model calls an MCP tool.
const CALL: &str = "CallDynamicTool";
/// The context Cursor gives a model it does not name as a 1M one.
const DEFAULT_CONTEXT: u64 = 200_000;
const MARGIN_SECONDS: i64 = 300;

fn api_base() -> String {
    super::endpoint_override(super::SourceKind::Cursor).unwrap_or_else(|| API.into())
}

fn agent_default() -> String {
    super::endpoint_override(super::SourceKind::Cursor).unwrap_or_else(|| AGENT.into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creds {
    /// Who the CLI said is signed in.
    pub email: String,
}

#[cfg(test)]
pub(crate) static EXECUTABLE: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(test)]
pub(crate) static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// The Cursor CLI: `cursor-agent`, or its `agent` when it is that one.
pub fn executable() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = EXECUTABLE.lock().unwrap().clone() {
        return Some(path);
    }
    let home = dirs::home_dir()?;
    let mut own = vec![home.join(".local").join("bin")];
    if let Some(local) = dirs::data_local_dir() {
        own.push(local.join("cursor-agent"));
    }
    cli::find(&["cursor-agent"], &own, |_| true).or_else(|| cli::find(&["agent"], &[], |path| {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().contains("cursor-agent")
    }))
}

/// Where the CLI keeps its sign-in outside a Mac's keychain.
fn auth_file() -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(dirs::config_dir()?.join("Cursor").join("auth.json"));
    }
    if cfg!(target_os = "macos") {
        return Some(dirs::home_dir()?.join(".cursor").join("auth.json"));
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|dir| dir.is_absolute())
        .or_else(|| Some(dirs::home_dir()?.join(".config")))?;
    Some(config.join("cursor").join("auth.json"))
}

/// The access token the CLI signed in with.
fn read_token() -> Option<String> {
    // A test's stand-in sign-in; an empty one is none.
    #[cfg(test)]
    if let Some(token) = TOKEN.lock().unwrap().clone() {
        return (!token.is_empty()).then_some(token);
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "cursor-access-token", "-a", "cursor-user", "-w"]).output().ok();
        if let Some(token) = out.filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|token| !token.is_empty())
        {
            return Some(token);
        }
    }
    let value: Value = serde_json::from_slice(&std::fs::read(auth_file()?).ok()?).ok()?;
    value["accessToken"].as_str().filter(|token| !token.is_empty()).map(str::to_owned)
}

/// When a JWT runs out, if it says.
fn expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["exp"].as_i64()
}

pub enum TokenError {
    SignedOut(String),
    Transient,
}

static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The CLI's token, renewed by the CLI when about to run out.
pub async fn token(app_data_dir: &Path) -> Result<String, TokenError> {
    let near = |token: &str| expiry(token).is_some_and(|at| at - chrono::Utc::now().timestamp() < MARGIN_SECONDS);
    let mut found = read_token();
    if found.as_deref().is_some_and(near) {
        if let Some(exe) = executable() {
            let _renewing = RENEWING.lock().await;
            found = read_token();
            if found.as_deref().is_some_and(near) {
                // Any run of the CLI renews its sign-in; `status` is the lightest.
                let _ = cli::run(cli::command(&exe, &["status"], &cli::proxy_env(app_data_dir)), Duration::from_secs(30)).await;
                found = read_token();
            }
        }
    }
    match found {
        None => Err(TokenError::SignedOut("Cursor is not signed in".into())),
        Some(token) if expiry(&token).is_some_and(|at| at <= chrono::Utc::now().timestamp()) => {
            Err(TokenError::SignedOut("Cursor's sign-in has run out".into()))
        }
        Some(token) => Ok(token),
    }
}

/// The CLI version the API is told it talks to: the installed one.
pub fn client_version() -> String {
    let dated = |name: &str| {
        let (date, hash) = name.split_once('-').unwrap_or_default();
        date.len() == 10 && date.chars().all(|ch| ch.is_ascii_digit() || ch == '.') && !hash.is_empty()
            && hash.chars().all(|ch| ch.is_ascii_hexdigit())
    };
    let mut version = executable().and_then(|exe| std::fs::canonicalize(exe).ok())
        .and_then(|real| real.parent()?.file_name()?.to_str().map(str::to_owned)).filter(|name| dated(name));
    if version.is_none() {
        let mut dirs = Vec::new();
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".local/share/cursor-agent/versions"));
        }
        if let Some(local) = dirs::data_local_dir() {
            dirs.push(local.join("cursor-agent").join("versions"));
        }
        version = dirs.iter().flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned)).filter(|name| dated(name)).max();
    }
    format!("cli-{}", version.as_deref().unwrap_or(VERSION_FALLBACK))
}

// ---- sign-in -------------------------------------------------------------

pub struct Pending {
    login: Option<cli::Login>,
    /// The page to sign in on; none when the CLI is signed in already.
    pub url: Option<String>,
}

pub struct Done {
    pub creds: Creds,
    pub plan: Option<String>,
}

/// Whether a link from `cursor-agent login` is all there: one the CLI cut
/// where it wrapped the line gets "This sign-in link is incomplete".
fn whole_link(link: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else { return false };
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let uuid = query.get("uuid").is_some_and(|uuid| uuid.len() == 36 && uuid.chars().filter(|ch| *ch == '-').count() == 4);
    query.get("challenge").is_some_and(|value| !value.is_empty()) && uuid
        && query.get("mode").map(String::as_str) == Some("login")
        && query.get("redirectTarget").map(String::as_str) == Some("cli")
        && query.get("supportsSelectedTeamLogin").is_some_and(|value| value == "true" || value == "false")
}

/// Who `cursor-agent about` says is signed in, and the plan.
async fn about(app_data_dir: &Path) -> Option<(String, Option<String>)> {
    let exe = executable()?;
    let out = cli::run(cli::command(&exe, &["about", "--format", "json"], &cli::proxy_env(app_data_dir)), Duration::from_secs(15)).await.ok()?;
    parse_about(&out)
}

fn parse_about(out: &str) -> Option<(String, Option<String>)> {
    let value: Value = serde_json::from_str(out.get(out.find('{')?..)?.trim()).ok()?;
    let email = value["userEmail"].as_str().map(str::trim).filter(|email| !email.is_empty())?.to_owned();
    let plan = value["subscriptionTier"].as_str().map(str::trim).filter(|plan| !plan.is_empty()).map(str::to_owned);
    Some((email, plan))
}

/// The CLI's sign-in: kept as it is when there is one, else `cursor-agent
/// login` without its browser, its link handed to the window.
pub async fn begin_sign_in(app_data_dir: &Path) -> Result<Pending, String> {
    let exe = executable().ok_or("accountSources.errors.cursorCli")?;
    if read_token().is_some() && about(app_data_dir).await.is_some() {
        return Ok(Pending { login: None, url: None });
    }
    let mut env = cli::proxy_env(app_data_dir);
    env.push(("NO_OPEN_BROWSER".into(), "1".into()));
    let (login, url) = cli::start_login(cli::command(&exe, &["login"], &env), whole_link, Duration::from_secs(60)).await?;
    Ok(Pending { login: Some(login), url: Some(url) })
}

pub async fn finish_sign_in(app_data_dir: &Path, pending: Pending, cancel: watch::Receiver<bool>) -> Result<Done, String> {
    if let Some(login) = pending.login {
        login.finish(cancel, Duration::from_secs(900)).await?;
    }
    let (email, plan) = about(app_data_dir).await.ok_or("accountSources.errors.noAccount")?;
    Ok(Done { creds: Creds { email }, plan })
}

// ---- models and allowance --------------------------------------------------

/// What the account can use, as `cursor-agent models` lists it.
pub async fn fetch_models(app_data_dir: &Path) -> Result<Vec<SourceModel>, String> {
    let exe = executable().ok_or("accountSources.errors.cursorCli")?;
    let out = cli::run(cli::command(&exe, &["models"], &cli::proxy_env(app_data_dir)), Duration::from_secs(30)).await
        .map_err(|error| format!("cursor-agent models: {error}"))?;
    let models = parse_models(&out);
    if models.is_empty() {
        return Err("accountSources.errors.noModels".into());
    }
    Ok(models)
}

/// "id - Name" a line; a name saying 1M holds that much, any other 200K.
fn parse_models(out: &str) -> Vec<SourceModel> {
    out.lines().filter_map(|line| {
        let (id, name) = line.trim().split_once(" - ")?;
        let id = id.trim();
        if id.is_empty() || !id.chars().next()?.is_ascii_alphanumeric()
            || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || "._:-".contains(ch))
        {
            return None;
        }
        let name = name.replace('\u{200b}', "");
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = name.trim_end_matches("(default)").trim().trim_end_matches("(current)").trim().to_owned();
        let million = name.split_whitespace().find_map(|word| word.strip_suffix('M')?.parse::<u64>().ok());
        Some(SourceModel {
            id: id.to_owned(),
            name: if name.is_empty() { id.to_owned() } else { name },
            context_window: Some(million.map_or(DEFAULT_CONTEXT, |m| m * 1_000_000)),
            ..Default::default()
        })
    }).collect()
}

fn connect_headers(request: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    request.bearer_auth(token)
        .header("connect-protocol-version", "1")
        .header("x-cursor-client-version", client_version())
        .header("x-cursor-client-type", "cli")
        // Privacy mode: nothing kept for training.
        .header("x-ghost-mode", "true")
}

/// How much of the period's allowance is gone.
pub async fn quota(client: &Client, token: &str) -> Quota {
    let checked_at = chrono::Utc::now().timestamp();
    let response = connect_headers(client.post(format!("{}/aiserver.v1.DashboardService/GetCurrentPeriodUsage", api_base())), token)
        .header("content-type", "application/json").body("{}").send().await;
    let value = match response {
        Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
        _ => None,
    };
    match value.as_ref().map(|value| &value["planUsage"]) {
        Some(usage) if usage.is_object() => Quota {
            used: usage["totalPercentUsed"].as_f64().unwrap_or_default().clamp(0.0, 100.0).round(),
            limit: 100.0, unit: "percent".into(), checked_at, ..Default::default()
        },
        Some(_) => Quota { unit: "percent".into(), checked_at, ..Default::default() },
        None => Quota { checked_at, error: Some("Cursor did not report its allowance".into()), ..Default::default() },
    }
}

// ---- the agent API ---------------------------------------------------------

/// The agent API last picked, for the token (by its hash) it was picked for.
static AGENT_URL: Mutex<Option<(String, String, std::time::Instant, bool)>> = Mutex::new(None);

/// The agent API to run on with this token, as the server config names it:
/// a team may be served in one region only.
pub async fn agent_url(client: &Client, token: &str, fresh: bool) -> String {
    let key = format!("{:x}", Sha256::digest(token.as_bytes()));
    if !fresh {
        if let Some((_, url, _, _)) = AGENT_URL.lock().unwrap().clone()
            .filter(|(cached, _, at, listed)| *cached == key && (*listed || at.elapsed() < Duration::from_secs(60)))
        {
            return url;
        }
    }
    let listed = async {
        let response = connect_headers(client.post(format!("{}/aiserver.v1.ServerConfigService/GetServerConfig", api_base())), token)
            .header("content-type", "application/json").body("{}").timeout(Duration::from_secs(10)).send().await.ok()?;
        let value: Value = response.error_for_status().ok()?.json().await.ok()?;
        ["agentUrl", "agentnUrl"].iter().filter_map(|key| value["agentUrlConfig"][key].as_str())
            .find(|url| reqwest::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host().is_some()))
            .map(|url| url.trim_end_matches('/').to_owned())
    }.await;
    let url = listed.clone().unwrap_or_else(agent_default);
    *AGENT_URL.lock().unwrap() = Some((key, url.clone(), std::time::Instant::now(), listed.is_some()));
    url
}

/// The agent API for this token, asked with the client a Run goes through.
pub async fn agent_url_with(proxy: &ApiProxyConfigPayload, token: &str) -> String {
    match duplex_client(proxy) {
        Ok(client) => agent_url(&client, token, false).await,
        Err(_) => agent_default(),
    }
}

/// The HTTP/2 client a Run needs (a Connect stream both ways), with the app proxy.
fn duplex_client(proxy: &ApiProxyConfigPayload) -> Result<Client, String> {
    static CLIENTS: Mutex<Option<(ApiProxyConfigPayload, Client)>> = Mutex::new(None);
    if let Some((_, client)) = CLIENTS.lock().unwrap().clone().filter(|(cached, _)| cached == proxy) {
        return Ok(client);
    }
    let builder = Client::builder().connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(900));
    // Tests talk HTTP/2 in the clear to a loopback stand-in, past any system proxy.
    #[cfg(test)]
    let builder = builder.http2_prior_knowledge();
    let config = if cfg!(test) {
        ApiProxyConfigPayload { mode: ApiProxyMode::Direct, url: None }
    } else {
        crate::core::api_client::sanitize_proxy_config(proxy).map_err(|_| "newapi.errors.proxy")?
    };
    let builder = match config.mode {
        // Cached for the app's lifetime: the system proxy is looked up per request.
        ApiProxyMode::System => builder.no_proxy().proxy(crate::core::api_client::system_proxy()),
        ApiProxyMode::Direct => builder.no_proxy(),
        ApiProxyMode::Manual => builder.no_proxy().proxy(reqwest::Proxy::all(config.url.as_deref().unwrap_or_default())
            .map_err(|_| "newapi.errors.proxy")?),
    };
    let client = builder.build().map_err(|_| "newapi.errors.network")?;
    *CLIENTS.lock().unwrap() = Some((proxy.clone(), client.clone()));
    Ok(client)
}

// ---- protobuf ----------------------------------------------------------------

/// A protobuf message being written.
#[derive(Clone, Default)]
struct Pb(Vec<u8>);

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

impl Pb {
    fn varint(mut self, num: u64, n: u64) -> Self {
        put_varint(&mut self.0, num << 3);
        put_varint(&mut self.0, n);
        self
    }
    fn bytes(mut self, num: u64, data: &[u8]) -> Self {
        put_varint(&mut self.0, num << 3 | 2);
        put_varint(&mut self.0, data.len() as u64);
        self.0.extend_from_slice(data);
        self
    }
    fn msg(self, num: u64, message: &Pb) -> Self {
        self.bytes(num, &message.0)
    }
    fn str(self, num: u64, text: &str) -> Self {
        self.bytes(num, text.as_bytes())
    }
    fn double(mut self, num: u64, value: f64) -> Self {
        put_varint(&mut self.0, num << 3 | 1);
        self.0.extend_from_slice(&value.to_bits().to_le_bytes());
        self
    }
}

/// A field of a message read: its number, a varint (or fixed) value, or bytes.
struct Field<'a> {
    num: u64,
    n: u64,
    data: &'a [u8],
}

fn read_varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        n |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

fn fields(bytes: &[u8]) -> Vec<Field<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(key) = read_varint(bytes, &mut at) else { break };
        let (num, kind) = (key >> 3, key & 7);
        let field = match kind {
            0 => read_varint(bytes, &mut at).map(|n| Field { num, n, data: &[] }),
            1 => bytes.get(at..at + 8).map(|raw| {
                at += 8;
                Field { num, n: u64::from_le_bytes(raw.try_into().unwrap()), data: &[] }
            }),
            2 => read_varint(bytes, &mut at).and_then(|len| {
                let data = bytes.get(at..at + len as usize)?;
                at += len as usize;
                Some(Field { num, n: 0, data })
            }),
            5 => bytes.get(at..at + 4).map(|raw| {
                at += 4;
                Field { num, n: u64::from(u32::from_le_bytes(raw.try_into().unwrap())), data: &[] }
            }),
            _ => None,
        };
        match field {
            Some(field) => out.push(field),
            None => break,
        }
    }
    out
}

fn text(fields: &[Field<'_>], num: u64) -> String {
    fields.iter().find(|field| field.num == num).map(|field| String::from_utf8_lossy(field.data).into_owned()).unwrap_or_default()
}

fn number(fields: &[Field<'_>], num: u64) -> u64 {
    fields.iter().find(|field| field.num == num).map_or(0, |field| field.n)
}

/// A google.protobuf.Value.
fn pb_value(value: &Value) -> Pb {
    match value {
        Value::Null => Pb::default().varint(1, 0),
        Value::Number(number) => Pb::default().double(2, number.as_f64().unwrap_or_default()),
        Value::String(text) => Pb::default().str(3, text),
        Value::Bool(flag) => Pb::default().varint(4, u64::from(*flag)),
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort();
            let fields = keys.into_iter().fold(Pb::default(), |out, key| {
                out.msg(1, &Pb::default().str(1, key).msg(2, &pb_value(&object[key])))
            });
            Pb::default().msg(5, &fields)
        }
        Value::Array(items) => Pb::default().msg(6, &items.iter().fold(Pb::default(), |out, item| out.msg(1, &pb_value(item)))),
    }
}

/// A google.protobuf.Value read back.
fn value_of(bytes: &[u8]) -> Value {
    for field in fields(bytes) {
        return match field.num {
            1 => Value::Null,
            2 => serde_json::Number::from_f64(f64::from_bits(field.n)).map_or(Value::Null, |number| {
                // A whole number goes back as one: a client may type it strictly.
                let n = f64::from_bits(field.n);
                if n.fract() == 0.0 && n.abs() < 9e15 { json!(n as i64) } else { Value::Number(number) }
            }),
            3 => Value::String(String::from_utf8_lossy(field.data).into_owned()),
            4 => Value::Bool(field.n != 0),
            5 => Value::Object(fields(field.data).into_iter().filter(|entry| entry.num == 1).map(|entry| {
                let parts = fields(entry.data);
                let value = parts.iter().find(|part| part.num == 2).map_or(Value::Null, |part| value_of(part.data));
                (text(&parts, 1), value)
            }).collect()),
            6 => Value::Array(fields(field.data).into_iter().filter(|item| item.num == 1).map(|item| value_of(item.data)).collect()),
            _ => continue,
        };
    }
    Value::Null
}

/// One Connect envelope: flags, length, message.
fn frame(message: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(message.len() + 5);
    out.push(0);
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    Bytes::from(out)
}

/// The next whole envelope in `buffer`, taken out: its flags and message.
fn take_frame(buffer: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if buffer.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes(buffer[1..5].try_into().unwrap()) as usize;
    if buffer.len() < 5 + len {
        return None;
    }
    let flags = buffer[0];
    let message = buffer[5..5 + len].to_vec();
    buffer.drain(..5 + len);
    Some((flags, message))
}

// ---- the conversation ------------------------------------------------------

#[derive(Clone)]
struct Tool {
    name: String,
    description: String,
    schema: Value,
}

fn tools_of(chat: &Value) -> Vec<Tool> {
    chat["tools"].as_array().into_iter().flatten().filter_map(|tool| {
        let function = &tool["function"];
        let name = function["name"].as_str().filter(|name| !name.is_empty())?;
        Some(Tool {
            name: name.to_owned(),
            description: function["description"].as_str().unwrap_or_default().to_owned(),
            schema: function.get("parameters").filter(|schema| schema.is_object()).cloned().unwrap_or_else(|| json!({"type": "object"})),
        })
    }).collect()
}

/// The caller's tools named for the model, which sees MCP tools only through
/// Cursor's own calls: without the list it says it has no such tool.
fn catalog(tools: &[Tool]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut out = format!("\n\n<dynamic_tool_catalog>\nThe tools below are available in the MCP namespace \"{SERVER}\". Call one with `{CALL}` (namespace \"{SERVER}\", toolName, arguments). Their schemas are given here, so there is no need to call `GetDynamicTools` first.\n");
    for tool in tools {
        out.push_str(&format!("<tool name=\"{}\">\n{}\ninput schema: {}\n</tool>\n", tool.name, tool.description, tool.schema));
    }
    out.push_str("</dynamic_tool_catalog>");
    out
}

/// A Chat content's text parts.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A call's id as Cursor gave it: an OpenAI model's carries a line, sent on as `__`.
fn cursor_call_id(id: &str) -> String {
    if id.starts_with("call_") { id.replacen("__fc_", "\nfc_", 1) } else { id.to_owned() }
}

fn tool_result(id: &str, text: &str, error: bool) -> Value {
    let result = serde_json::from_str::<Value>(text).ok().filter(|_| !text.trim().is_empty()).unwrap_or_else(|| json!(text));
    let mut result = json!({"type": "tool-result", "toolCallId": cursor_call_id(id), "toolName": CALL, "result": result,
        "experimental_content": [{"type": "text", "text": text}]});
    if error {
        result["isError"] = json!(true);
    }
    result
}

/// The conversation as AI SDK messages, in JSON: the system prompt with the
/// caller's tools listed, then each turn; a call left unanswered is answered
/// for, as the API wants every call to have its result.
fn messages_of(chat: &Value, tools: &[Tool]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let messages: Vec<&Value> = chat["messages"].as_array().into_iter().flatten().collect();
    let system: Vec<String> = messages.iter().filter(|message| matches!(message["role"].as_str(), Some("system" | "developer")))
        .map(|message| content_text(&message["content"])).filter(|text| !text.is_empty()).collect();
    let system = system.join("\n\n") + &catalog(tools);
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}).to_string().into_bytes());
    }
    let mut pending: Vec<String> = Vec::new();
    let mut results: Vec<Value> = Vec::new();
    let flush = |out: &mut Vec<Vec<u8>>, pending: &mut Vec<String>, results: &mut Vec<Value>| {
        for id in pending.drain(..) {
            results.push(tool_result(&id, "The tool returned no result.", true));
        }
        if !results.is_empty() {
            out.push(json!({"role": "tool", "content": std::mem::take(results)}).to_string().into_bytes());
        }
    };
    for message in messages {
        match message["role"].as_str() {
            Some("tool") => {
                let id = message["tool_call_id"].as_str().unwrap_or_default().to_owned();
                if let Some(index) = pending.iter().position(|pending| *pending == id) {
                    pending.remove(index);
                    results.push(tool_result(&id, &content_text(&message["content"]), false));
                }
            }
            Some("assistant") => {
                flush(&mut out, &mut pending, &mut results);
                let mut content = Vec::new();
                let said = content_text(&message["content"]);
                if !said.is_empty() {
                    content.push(json!({"type": "text", "text": said}));
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let id = call["id"].as_str().unwrap_or_default().to_owned();
                    let arguments = call["function"]["arguments"].as_str()
                        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok()).unwrap_or_else(|| json!({}));
                    content.push(json!({"type": "tool-call", "toolCallId": cursor_call_id(&id), "toolName": CALL,
                        "args": {"namespace": SERVER, "toolName": call["function"]["name"], "arguments": arguments}}));
                    pending.push(id);
                }
                if !content.is_empty() {
                    out.push(json!({"role": "assistant", "content": content}).to_string().into_bytes());
                }
            }
            Some("user") => {
                flush(&mut out, &mut pending, &mut results);
                let parts: Vec<Value> = match &message["content"] {
                    Value::String(text) => vec![json!({"type": "text", "text": text})],
                    Value::Array(parts) => parts.iter().filter_map(|part| match part["type"].as_str() {
                        Some("text") => part["text"].as_str().map(|text| json!({"type": "text", "text": text})),
                        Some("image_url") => {
                            let url = part["image_url"]["url"].as_str().or(part["image_url"].as_str()).unwrap_or_default();
                            Some(image_part(url))
                        }
                        _ => None,
                    }).collect(),
                    _ => Vec::new(),
                };
                if !parts.is_empty() {
                    out.push(json!({"role": "user", "content": parts}).to_string().into_bytes());
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut pending, &mut results);
    out
}

/// An image as the AI SDK carries it inline; one that is no data URL is named.
fn image_part(url: &str) -> Value {
    let inline = url.strip_prefix("data:").and_then(|rest| rest.split_once(";base64,"))
        .and_then(|(mime, data)| Some((mime, base64::engine::general_purpose::STANDARD.decode(data).ok()?)));
    match inline {
        Some((mime, bytes)) => json!({"type": "image", "mimeType": mime, "image": {"__type": "Uint8Array",
            "hex": bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>()}}),
        None => json!({"type": "text", "text": format!("[image: {url}]")}),
    }
}

/// The user's last words, which the turn is named by.
fn last_user(chat: &Value) -> String {
    chat["messages"].as_array().into_iter().flatten().rev().filter(|message| message["role"] == "user")
        .map(|message| content_text(&message["content"])).find(|text| !text.is_empty()).unwrap_or_else(|| ".".into())
}

fn uuid_of(bytes: &[u8]) -> String {
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&bytes[..16]);
    raw[6] = raw[6] & 0x0f | 0x40;
    raw[8] = raw[8] & 0x3f | 0x80;
    uuid::Uuid::from_bytes(raw).hyphenated().to_string()
}

/// The conversation's id, the same on each of its requests so Cursor keeps
/// it where its prompt is cached: from what names the session and the first
/// user message; a new one when nothing names the session.
fn conversation(chat: &Value) -> String {
    let session = ["prompt_cache_key", "user"].iter().find_map(|key| chat[key].as_str().filter(|value| !value.is_empty()));
    let Some(session) = session else { return uuid::Uuid::new_v4().to_string() };
    let first = chat["messages"].as_array().into_iter().flatten().find(|message| message["role"] == "user")
        .map(|message| content_text(&message["content"])).unwrap_or_default();
    uuid_of(&Sha256::digest(format!("cursor conversation\0{session}\0{first}").as_bytes()))
}

/// An McpToolDefinition.
fn tool_def(tool: &Tool) -> Pb {
    let mut def = Pb::default().str(1, &tool.name);
    if !tool.description.is_empty() {
        def = def.str(2, &tool.description);
    }
    def.msg(3, &pb_value(&tool.schema)).str(4, SERVER).str(5, &tool.name).str(6, &tool.schema.to_string())
}

fn os_name() -> &'static str {
    if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" }
}

/// The request context Cursor asks for: the platform, a temp folder, UTC.
fn environment() -> Pb {
    Pb::default().str(1, os_name()).str(2, &std::env::temp_dir().to_string_lossy()).str(10, "UTC")
}

/// What one Run is built from.
#[derive(Clone)]
struct Plan {
    messages: Vec<Vec<u8>>,
    last_user: String,
    tools: Vec<Tool>,
    model: String,
    conversation: String,
}

/// The Run's first message (an AgentClientMessage with its run_request) and
/// the blobs it names. Max Mode asks for a model Cursor serves only that way.
fn build_run(plan: &Plan, max_mode: bool) -> (Vec<u8>, HashMap<Vec<u8>, Vec<u8>>) {
    let mut blobs = HashMap::new();
    let mut put = |bytes: Vec<u8>| {
        let id = Sha256::digest(&bytes).to_vec();
        blobs.insert(id.clone(), bytes);
        id
    };
    let mut state = Pb::default();
    for message in &plan.messages {
        state = state.bytes(1, &put(message.clone()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let user = Pb::default().str(1, &plan.last_user).str(2, &id).varint(4, 1);
    let turn = Pb::default().msg(1, &Pb::default().bytes(1, &put(user.0)).str(10, &id));
    let state = state.bytes(8, &put(turn.0)).varint(10, 1).str(22, "cli");
    let mut context = Pb::default().msg(4, &environment());
    let mut mcp = Pb::default();
    for tool in &plan.tools {
        context = context.msg(7, &tool_def(tool));
        mcp = mcp.msg(1, &tool_def(tool));
    }
    let action = Pb::default().msg(2, &Pb::default().msg(2, &context));
    let (mut details, mut requested) = (Pb::default().str(1, &plan.model).str(3, &plan.model).str(4, &plan.model),
        Pb::default().str(1, &plan.model));
    if max_mode {
        details = details.varint(7, 1);
        requested = requested.varint(2, 1);
    }
    let run = Pb::default().msg(1, &state).msg(2, &action).msg(3, &details).msg(4, &mcp)
        .str(5, &plan.conversation).msg(9, &requested).varint(19, 1);
    (Pb::default().msg(1, &run).0, blobs)
}

// ---- failures ------------------------------------------------------------------

/// The status and words for a Connect error, whose details say it best.
fn failure(status: u16, body: &[u8]) -> (u16, String) {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        let text = String::from_utf8_lossy(body).trim().to_owned();
        return classify(status, "", &text);
    };
    let error = if value["error"].is_object() { &value["error"] } else { &value };
    let mut message = error["message"].as_str().unwrap_or_default().to_owned();
    for detail in error["details"].as_array().into_iter().flatten() {
        let said = format!("{}: {}", detail["debug"]["details"]["title"].as_str().unwrap_or_default(),
            detail["debug"]["details"]["detail"].as_str().unwrap_or_default());
        let said = said.trim_matches(|ch: char| ch == ':' || ch.is_whitespace()).to_owned();
        if !said.is_empty() {
            message = said;
        }
    }
    let code = error["code"].as_str().unwrap_or_default();
    if message.is_empty() || message == "Error" {
        message = code.to_owned();
    }
    if code.is_empty() && (200..300).contains(&status) {
        return (status, message);
    }
    classify(status, code, &message)
}

fn regional(message: &str) -> bool {
    message.to_ascii_lowercase().contains("region")
}

fn max_required(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("max mode required") || lower.contains("max_mode_required") || lower.contains("requires max mode")
}

fn classify(status: u16, code: &str, message: &str) -> (u16, String) {
    let message = if message.is_empty() { format!("HTTP {status}") } else { message.to_owned() };
    let lower = message.to_ascii_lowercase();
    if regional(&message) {
        (403, format!("{message} — Cursor serves your team only in some regions; signing in again won't change that"))
    } else if code == "permission_denied" {
        (403, message)
    } else if code == "unauthenticated" || status == 401 || lower.contains("expired") {
        (401, message)
    } else if code == "resource_exhausted" || ["quota", "rate limit", "usage limit"].iter().any(|word| lower.contains(word)) {
        (429, format!("usage limit reached: {message}"))
    } else if ["too long", "context length", "too many tokens"].iter().any(|word| lower.contains(word)) {
        (400, format!("input is too long for the model's context: {message}"))
    } else if code == "invalid_argument" {
        (400, message)
    } else if code == "unavailable" {
        (503, message)
    } else if !code.is_empty() || (200..300).contains(&status) {
        (502, message)
    } else {
        (status, message)
    }
}

pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    match status {
        401 => Some("Cursor refused the sign-in; sign in again on AgentHarbor's Accounts page or run `cursor-agent login`".into()),
        403 if text.contains("region") => None,
        403 => Some("Cursor refused this request for the account's plan".into()),
        429 => Some("the Cursor plan's allowance is used up for now".into()),
        _ => None,
    }
}

// ---- one exchange ----------------------------------------------------------------

/// The ids Cursor said it serves in Max Mode only.
static MAX_ONLY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// A Chat request answered through one or more Runs on the agent API.
pub struct Exchange {
    client: Client,
    token: String,
    base: String,
    plan: Plan,
    model: String,
    stream: bool,
    /// A guess at the prompt's size for a step Cursor counts nothing of.
    estimate: u64,
}

/// What a Run's reader hands on: an event, or a failure before any.
enum Out {
    Event(Event),
    Failed(u16, String),
}

pub fn exchange(proxy: &ApiProxyConfigPayload, token: &str, base: &str, body: &[u8], model: &str)
    -> Result<Exchange, (hyper::StatusCode, &'static str)>
{
    let chat: Value = serde_json::from_slice(body).map_err(|_| (hyper::StatusCode::BAD_REQUEST, "invalid chat request"))?;
    let tools = tools_of(&chat);
    let messages = messages_of(&chat, &tools);
    let requested = chat["model"].as_str().filter(|model| !model.is_empty()).unwrap_or(model).to_owned();
    let estimate = messages.iter().map(|message| message.len() as u64).sum::<u64>() / 4;
    Ok(Exchange {
        client: duplex_client(proxy).map_err(|_| (hyper::StatusCode::BAD_GATEWAY, "cannot reach Cursor"))?,
        token: token.to_owned(),
        base: base.to_owned(),
        plan: Plan {
            last_user: last_user(&chat),
            conversation: conversation(&chat),
            // Cursor's pick, which its API calls default.
            model: if requested == "auto" { "default".into() } else { requested.clone() },
            messages,
            tools,
        },
        model: requested,
        stream: chat["stream"].as_bool().unwrap_or(false),
        estimate,
    })
}

impl Exchange {
    /// The Chat reply: SSE or one completion, or Cursor's refusal with its status.
    pub async fn run(self) -> reqwest::Response {
        let max_only = |id: &str| MAX_ONLY.lock().unwrap().as_ref().is_some_and(|ids| ids.contains(id));
        let mut max_mode = max_only(&self.plan.model);
        let mut base = self.base.clone();
        let mut outcome = self.attempt(&base, max_mode).await;
        if let Err((_, message)) = &outcome {
            if regional(message) {
                // The team moved, or the config was kept from before.
                let fresh = agent_url(&self.client, &self.token, true).await;
                if fresh != base {
                    base = fresh;
                    outcome = self.attempt(&base, max_mode).await;
                }
            }
        }
        if let Err((_, message)) = &outcome {
            if !max_mode && max_required(message) {
                MAX_ONLY.lock().unwrap().get_or_insert_with(HashSet::new).insert(self.plan.model.clone());
                max_mode = true;
                outcome = self.attempt(&base, max_mode).await;
            }
        }
        match outcome {
            Err((status, message)) => reqwest::Response::from(hyper::Response::builder().status(status)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(json!({"error": {"message": format!("Cursor: {message}"), "type": "upstream_error"}}).to_string())
                .expect("static response parts")),
            Ok((first, rest)) => self.respond(first, rest).await,
        }
    }

    async fn respond(self, first: Event, mut rest: mpsc::Receiver<Out>) -> reqwest::Response {
        let mut writer = Writer::new(&self.model, self.stream);
        if !self.stream {
            let mut events = vec![first];
            while let Some(out) = rest.recv().await {
                events.push(match out {
                    Out::Event(event) => event,
                    Out::Failed(_, message) => Event::Error(message),
                });
            }
            let body = writer.collect(events);
            return reqwest::Response::from(hyper::Response::builder().status(200)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(body.to_string()).expect("static response parts"));
        }
        let head = writer.write(vec![first]);
        let stream = futures_util::stream::unfold(Some((rest, writer, Some(head))), |state| async move {
            let (mut rest, mut writer, head) = state?;
            if let Some(head) = head {
                return Some((Ok::<_, std::io::Error>(Bytes::from(head)), Some((rest, writer, None))));
            }
            loop {
                match rest.recv().await {
                    Some(out) => {
                        let event = match out {
                            Out::Event(event) => event,
                            Out::Failed(_, message) => Event::Error(message),
                        };
                        let out = writer.write(vec![event]);
                        if !out.is_empty() {
                            return Some((Ok(Bytes::from(out)), Some((rest, writer, None))));
                        }
                    }
                    None => return Some((Ok(Bytes::from(writer.done())), None)),
                }
            }
        });
        reqwest::Response::from(hyper::Response::builder().status(200)
            .header(hyper::header::CONTENT_TYPE, "text/event-stream")
            .body(reqwest::Body::wrap_stream(stream)).expect("static response parts"))
    }

    /// One Run: its first event and the rest, or why it failed before any.
    async fn attempt(&self, base: &str, max_mode: bool) -> Result<(Event, mpsc::Receiver<Out>), (u16, String)> {
        let (run, blobs) = build_run(&self.plan, max_mode);
        let (sender, receiver) = mpsc::unbounded_channel::<Bytes>();
        let _ = sender.send(frame(&run));
        let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|bytes| (Ok::<_, std::io::Error>(bytes), receiver))
        }));
        let request = connect_headers(self.client.post(format!("{base}/agent.v1.AgentService/Run")), &self.token)
            .header("content-type", "application/connect+proto")
            .header("x-request-id", uuid::Uuid::new_v4().to_string())
            // None of Cursor's own tools, only the caller's.
            .header("x-cursor-agent-allowed-tools", "mcp_tool_call,get_mcp_tools_tool_call")
            .body(body);
        let response = request.send().await.map_err(|error| {
            let mut message = error.to_string();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                message.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            (502, message)
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let bytes = response.bytes().await.unwrap_or_default();
            return Err(failure(status, &bytes));
        }
        let (stop, stopped) = watch::channel(false);
        let beat = sender.clone();
        tokio::spawn(heartbeat(beat, stopped));
        let (out, mut events) = mpsc::channel::<Out>(64);
        let reader = Reader { sender, blobs, tools: self.plan.tools.clone(), estimate: self.estimate, stop };
        tokio::spawn(reader.read(response, out));
        match events.recv().await {
            None => Err((502, "an empty reply".into())),
            Some(Out::Failed(status, message)) => Err((status, message)),
            Some(Out::Event(first)) => Ok((first, events)),
        }
    }
}

/// Tells the server the client is still there, as the CLI does.
async fn heartbeat(sender: mpsc::UnboundedSender<Bytes>, mut stopped: watch::Receiver<bool>) {
    let mut ticks = tokio::time::interval(Duration::from_secs(5));
    ticks.tick().await;
    loop {
        tokio::select! {
            _ = ticks.tick() => if sender.send(frame(&Pb::default().bytes(7, &[]).0)).is_err() { return },
            _ = stopped.changed() => return,
        }
    }
}

/// Follows one Run's server messages, answering what it asks of the client.
struct Reader {
    sender: mpsc::UnboundedSender<Bytes>,
    blobs: HashMap<Vec<u8>, Vec<u8>>,
    tools: Vec<Tool>,
    estimate: u64,
    stop: watch::Sender<bool>,
}

impl Reader {
    fn send(&self, message: Pb) {
        let _ = self.sender.send(frame(&message.0));
    }

    async fn read(self, response: reqwest::Response, out: mpsc::Sender<Out>) {
        let mut upstream = response.bytes_stream();
        let mut buffer = Vec::new();
        let (mut calls, mut listed, mut said) = (0usize, 0usize, 0usize);
        let mut usage: Option<Event> = None;
        let finish = |calls: usize, said: usize, usage: Option<Event>| {
            let usage = usage.unwrap_or(Event::Usage { input: self.estimate, output: (said as u64).div_ceil(4), cached: 0, reasoning: 0 });
            vec![usage, Event::Finish(if calls > 0 { "tool_calls" } else { "stop" })]
        };
        let ended = 'run: loop {
            while let Some((flags, message)) = take_frame(&mut buffer) {
                if flags & 2 != 0 {
                    let (status, words) = failure(200, &message);
                    break 'run if !(200..300).contains(&status) {
                        Err((status, words))
                    } else if said == 0 && calls == 0 {
                        Err((502, "an empty reply".into()))
                    } else {
                        Ok(finish(calls, said, usage.take()))
                    };
                }
                for field in fields(&message) {
                    match field.num {
                        // interaction_update
                        1 => for update in fields(field.data) {
                            let parts = fields(update.data);
                            match update.num {
                                1 | 4 => {
                                    let chunk = text(&parts, 1);
                                    if chunk.is_empty() {
                                        continue;
                                    }
                                    said += chunk.len();
                                    let event = if update.num == 1 { Event::Text(chunk) } else { Event::Reasoning(chunk) };
                                    if out.send(Out::Event(event)).await.is_err() {
                                        break 'run Ok(Vec::new());
                                    }
                                }
                                // The turn ended, with what it used: the whole prompt in
                                // its input, the cached part read and written apart.
                                14 => {
                                    usage = Some(Event::Usage { input: number(&parts, 1), output: number(&parts, 2),
                                        cached: number(&parts, 3), reasoning: number(&parts, 5) });
                                    if calls == 0 {
                                        break 'run Ok(finish(calls, said, usage.take()));
                                    }
                                }
                                // How many tool calls the step makes, sent before them.
                                27 => {
                                    listed = number(&parts, 1) as usize;
                                    if listed > 0 && calls >= listed {
                                        break 'run Ok(finish(calls, said, usage.take()));
                                    }
                                }
                                _ => {}
                            }
                        },
                        // exec_server_message: the server asks the client to run something.
                        2 => {
                            if let Some((id, name, arguments)) = self.exec(field.data) {
                                calls += 1;
                                said += arguments.len();
                                let sent = out.send(Out::Event(Event::ToolStart { id: id.clone(), name })).await.is_ok()
                                    && out.send(Out::Event(Event::ToolArgs { id, text: arguments })).await.is_ok();
                                if !sent {
                                    break 'run Ok(Vec::new());
                                }
                                if listed > 0 && calls >= listed {
                                    // Every call made: the caller runs them.
                                    break 'run Ok(finish(calls, said, usage.take()));
                                }
                            }
                        }
                        // kv_server_message: a blob wanted, or one to keep.
                        4 => self.kv(field.data),
                        _ => {}
                    }
                }
            }
            match upstream.next().await {
                Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                Some(Err(error)) if calls == 0 => break Err((502, format!("the reply broke off: {error}"))),
                None if calls == 0 => break Err((502, "the reply broke off".into())),
                _ => break Ok(finish(calls, said, usage.take())),
            }
        };
        let _ = self.stop.send(true);
        match ended {
            Ok(events) => {
                for event in events {
                    if out.send(Out::Event(event)).await.is_err() {
                        break;
                    }
                }
            }
            Err((status, message)) => {
                let _ = out.send(Out::Failed(status, message)).await;
            }
        }
        // Dropping the sender and the reply ends the Run.
    }

    /// Answers what the server asks the client to run: a call of the caller's
    /// tools is handed on, the list of them given, anything else refused.
    fn exec(&self, message: &[u8]) -> Option<(String, String, String)> {
        let parts = fields(message);
        let id = number(&parts, 1);
        let exec_id = text(&parts, 15);
        let answer = |num: u64, result: Pb| {
            self.send(Pb::default().msg(2, &Pb::default().varint(1, id).str(15, &exec_id).msg(num, &result)));
            self.send(Pb::default().msg(5, &Pb::default().msg(1, &Pb::default().varint(1, id))));
        };
        for part in &parts {
            match part.num {
                // mcp_args: a call of the caller's tools.
                11 => {
                    let call = fields(part.data);
                    let arguments: serde_json::Map<String, Value> = call.iter().filter(|entry| entry.num == 2).map(|entry| {
                        let pair = fields(entry.data);
                        let value = pair.iter().find(|item| item.num == 2).map_or(Value::Null, |item| value_of(item.data));
                        (text(&pair, 1), value)
                    }).collect();
                    let mut name = text(&call, 5);
                    if name.is_empty() {
                        name = text(&call, 1).trim_start_matches(&format!("{SERVER}-")).to_owned();
                    }
                    let mut call_id = text(&call, 3).replace('\n', "__");
                    if call_id.is_empty() {
                        call_id = format!("call_{}", uuid::Uuid::new_v4().simple());
                    }
                    return Some((call_id, name, Value::Object(arguments).to_string()));
                }
                // mcp_state_exec_args: the tools there are.
                36 => {
                    let server = self.tools.iter().fold(Pb::default().str(1, SERVER).str(2, SERVER).str(7, "connected"),
                        |server, tool| server.msg(5, &tool_def(tool)));
                    answer(36, Pb::default().msg(1, &Pb::default().msg(1, &server)));
                    return None;
                }
                // request_context_args
                10 => {
                    answer(10, Pb::default().msg(1, &Pb::default().msg(1, &Pb::default().msg(4, &environment()))));
                    return None;
                }
                _ => {}
            }
        }
        // A shell, a file read or an edit of Cursor's own: never run here.
        self.send(Pb::default().msg(5, &Pb::default().msg(2, &Pb::default().varint(1, id).str(2, "not available"))));
        self.send(Pb::default().msg(5, &Pb::default().msg(1, &Pb::default().varint(1, id))));
        None
    }

    fn kv(&self, message: &[u8]) {
        let parts = fields(message);
        let id = number(&parts, 1);
        for part in &parts {
            match part.num {
                2 => {
                    let wanted = fields(part.data).into_iter().find(|field| field.num == 1).map(|field| field.data.to_vec()).unwrap_or_default();
                    let result = match self.blobs.get(&wanted) {
                        Some(blob) => Pb::default().bytes(1, blob),
                        None => Pb::default().msg(2, &Pb::default().str(1, "blob not found")),
                    };
                    self.send(Pb::default().msg(3, &Pb::default().varint(1, id).msg(2, &result)));
                }
                // The server's own record of the turn, not needed here.
                3 => self.send(Pb::default().msg(3, &Pb::default().varint(1, id).bytes(3, &[]))),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn decode_fields(bytes: &[u8]) -> Vec<(u64, u64, Vec<u8>)> {
        fields(bytes).into_iter().map(|field| (field.num, field.n, field.data.to_vec())).collect()
    }

    /// What a stand-in agent API sends: Connect envelopes of server messages.
    pub(crate) mod server {
        use super::super::*;
        pub fn envelope(message: &[u8]) -> Bytes {
            frame(message)
        }
        pub fn end(trailer: &str) -> Bytes {
            let mut out = vec![2];
            out.extend_from_slice(&(trailer.len() as u32).to_be_bytes());
            out.extend_from_slice(trailer.as_bytes());
            Bytes::from(out)
        }
        fn update(num: u64, inner: Pb) -> Vec<u8> {
            Pb::default().msg(1, &Pb::default().msg(num, &inner)).0
        }
        pub fn text(said: &str) -> Vec<u8> {
            update(1, Pb::default().str(1, said))
        }
        pub fn thinking(said: &str) -> Vec<u8> {
            update(4, Pb::default().str(1, said))
        }
        pub fn turn_ended(input: u64, output: u64, cached: u64) -> Vec<u8> {
            update(14, Pb::default().varint(1, input).varint(2, output).varint(3, cached))
        }
        pub fn calls(count: u64) -> Vec<u8> {
            update(27, Pb::default().varint(1, count))
        }
        pub fn get_blob(id: u64, blob: &[u8]) -> Vec<u8> {
            Pb::default().msg(4, &Pb::default().varint(1, id).msg(2, &Pb::default().bytes(1, blob))).0
        }
        pub fn mcp_call(id: u64, exec_id: &str, name: &str, call_id: &str, arguments: &Value) -> Vec<u8> {
            let mut call = Pb::default().str(1, &format!("{SERVER}-{name}")).str(3, call_id).str(5, name);
            for (key, value) in arguments.as_object().unwrap() {
                call = call.msg(2, &Pb::default().str(1, key).msg(2, &pb_value(value)));
            }
            Pb::default().msg(2, &Pb::default().varint(1, id).str(15, exec_id).msg(11, &call)).0
        }
        pub fn shell(id: u64) -> Vec<u8> {
            Pb::default().msg(2, &Pb::default().varint(1, id).msg(3, &Pb::default().str(1, "rm -rf /"))).0
        }
        /// The next whole envelope a client sent, taken out of `buffer`.
        pub fn take(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
            take_frame(buffer).map(|(_, message)| message)
        }
    }

    #[test]
    fn protobuf_values_round_trip_and_frames_split_where_they_end() {
        let value = json!({"cmd": "ls -la", "timeout": 2500, "ratio": 0.5, "flags": [true, null], "nested": {"a": "b"}});
        assert_eq!(value_of(&pb_value(&value).0), value);
        let mut buffer = frame(b"one").to_vec();
        buffer.extend_from_slice(&frame(b"two")[..4]);
        assert_eq!(take_frame(&mut buffer), Some((0, b"one".to_vec())));
        assert_eq!(take_frame(&mut buffer), None, "half a frame waits for the rest");
        let message = Pb::default().varint(1, 300).str(2, "hi").msg(3, &Pb::default().varint(1, 7));
        let read = decode_fields(&message.0);
        assert_eq!((read[0].0, read[0].1), (1, 300));
        assert_eq!(read[1].2, b"hi");
        assert_eq!(decode_fields(&read[2].2)[0].1, 7);
    }

    #[test]
    fn a_chat_conversation_goes_as_ai_sdk_messages_with_every_call_answered() {
        let chat = json!({"model": "auto", "messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAE="}}]},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1__fc_2", "type": "function", "function": {"name": "read", "arguments": "{\"path\":\"a\"}"}},
                {"id": "call_3", "type": "function", "function": {"name": "read", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call_1__fc_2", "content": "{\"ok\":true}"},
            {"role": "user", "content": "thanks"}],
            "tools": [{"type": "function", "function": {"name": "read", "description": "Read a file", "parameters": {"type": "object"}}}]});
        let tools = tools_of(&chat);
        let messages: Vec<Value> = messages_of(&chat, &tools).iter().map(|bytes| serde_json::from_slice(bytes).unwrap()).collect();
        assert_eq!(messages.len(), 5);
        assert!(messages[0]["content"].as_str().unwrap().starts_with("Be brief.\n\n<dynamic_tool_catalog>"));
        assert!(messages[0]["content"].as_str().unwrap().contains("<tool name=\"read\">"));
        assert_eq!(messages[1]["content"][1], json!({"type": "image", "mimeType": "image/png", "image": {"__type": "Uint8Array", "hex": "0001"}}));
        assert_eq!(messages[2]["content"][0]["toolCallId"], "call_1\nfc_2", "an OpenAI model's call id gets its line back");
        assert_eq!(messages[2]["content"][0]["args"], json!({"namespace": SERVER, "toolName": "read", "arguments": {"path": "a"}}));
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["content"][0]["result"], json!({"ok": true}));
        assert_eq!(messages[3]["content"][1]["isError"], true, "a call left unanswered is answered for");
        assert_eq!(last_user(&chat), "thanks");
        let (run, blobs) = build_run(&Plan { messages: messages_of(&chat, &tools), last_user: last_user(&chat), tools,
            model: "default".into(), conversation: "conv".into() }, true);
        assert_eq!(blobs.len(), 7, "every message, the user's turn and the turn itself");
        let request = decode_fields(&decode_fields(&run)[0].2);
        let details = request.iter().find(|field| field.0 == 3).unwrap();
        assert!(decode_fields(&details.2).iter().any(|field| field.0 == 7 && field.1 == 1), "Max Mode asked for");
        assert_eq!(request.iter().find(|field| field.0 == 5).unwrap().2, b"conv");
    }

    #[test]
    fn the_cli_and_the_api_are_read_as_they_answer() {
        assert!(whole_link("https://cursor.com/loginDeepControl?challenge=abc&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427&mode=login&redirectTarget=cli&supportsSelectedTeamLogin=true"));
        assert!(!whole_link("https://cursor.com/loginDeepControl?challenge=abc&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427"));
        assert_eq!(parse_about("Update available\n{\"userEmail\":\"me@cursor.sh\",\"subscriptionTier\":\"Pro\"}"),
            Some(("me@cursor.sh".into(), Some("Pro".into()))));
        let models = parse_models("Available models\nauto - Auto (current)\nclaude-opus-5-5-high - Claude Opus 5.5 1M\ngpt-6\u{200b} - GPT 6\n");
        assert_eq!(models.iter().map(|model| (model.id.as_str(), model.name.as_str(), model.context_window)).collect::<Vec<_>>(),
            vec![("auto", "Auto", Some(200_000)), ("claude-opus-5-5-high", "Claude Opus 5.5 1M", Some(1_000_000))]);
        let refusal = json!({"code": "resource_exhausted", "message": "Error", "details": [{"debug": {"details": {"title": "Usage limit", "detail": "Upgrade to continue"}}}]});
        assert_eq!(failure(429, refusal.to_string().as_bytes()), (429, "usage limit reached: Usage limit: Upgrade to continue".into()));
        assert_eq!(failure(400, br#"{"code":"permission_denied","message":"Max Mode Required: requires max mode"}"#).0, 403);
        assert!(max_required("Max Mode Required: The model requires Max Mode to be enabled."));
        assert_eq!(failure(200, br#"{"error":{"code":"unauthenticated","message":"token expired"}}"#).0, 401);
        let token = format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1900000000}"#));
        assert_eq!(expiry(&token), Some(1_900_000_000));
    }
}
