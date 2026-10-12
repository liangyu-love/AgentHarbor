//! Grok (SuperGrok, X Premium+) through xAI's Grok CLI ("Grok Build"): the
//! account is signed in by the CLI's own `grok login`, in a Grok home of
//! AgentHarbor's so the user's own sign-in stays as it is, and its requests go
//! to the Responses API the CLI talks to. The CLI keeps and renews the token;
//! it is read from the home for each request and never stored here.
use super::cli;
use super::store::{Quota, SourceModel};
use crate::core::protocol::tool_namespaces::ToolNamespaces;
use futures_util::StreamExt;
use hyper::body::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

pub const DEFAULT_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
/// The CLI version said when the installed one cannot be asked.
const CLIENT_VERSION: &str = "1.0.41";
/// How close to its expiry the CLI is asked to renew the token first.
const MARGIN_SECONDS: i64 = 300;

pub fn api_base() -> String {
    super::endpoint_override(super::SourceKind::Grok).unwrap_or_else(|| DEFAULT_BASE.into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creds {
    /// The Grok home this account is signed in to, one of AgentHarbor's own.
    pub home: String,
}

#[cfg(test)]
pub(crate) static EXECUTABLE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The Grok CLI: its installer's folder, else one on PATH that lives there.
pub fn executable() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = EXECUTABLE.lock().unwrap().clone() {
        return Some(path);
    }
    let home = dirs::home_dir()?;
    let mut own: Vec<PathBuf> = std::env::var_os("GROK_BIN_DIR").map(PathBuf::from).into_iter().collect();
    own.push(home.join(".grok").join("bin"));
    let installed = |path: &Path| {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase()
            .contains("/.grok/")
    };
    cli::find(&["grok"], &own, installed)
        .or_else(|| cli::find(&["grok"], &[], |path| installed(path) || path.starts_with(home.join(".local"))))
}

/// Where the homes of the accounts signed in here live.
fn accounts_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("account-sources").join("grok")
}

/// A home made here, and nothing else, is removed.
pub fn forget(app_data_dir: &Path, creds: &Creds) {
    let home = PathBuf::from(&creds.home);
    if home.parent() == Some(accounts_dir(app_data_dir).as_path()) {
        let _ = std::fs::remove_dir_all(home);
    }
}

/// The sign-in the CLI keeps in a home: its token, email and expiry.
struct Credential {
    key: String,
    email: String,
    expires_at: Option<i64>,
}

fn read_credential(home: &Path) -> Option<Credential> {
    #[derive(Deserialize)]
    struct Stored {
        #[serde(default)]
        key: String,
        #[serde(default)]
        email: String,
        #[serde(default)]
        expires_at: Option<String>,
    }
    let all: std::collections::BTreeMap<String, Stored> =
        serde_json::from_slice(&std::fs::read(home.join("auth.json")).ok()?).ok()?;
    all.into_values().find(|stored| !stored.key.is_empty()).map(|stored| Credential {
        key: stored.key,
        email: stored.email,
        expires_at: stored.expires_at.as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp()),
    })
}

fn home_env(app_data_dir: &Path, home: &Path) -> Vec<(String, String)> {
    let mut env = cli::proxy_env(app_data_dir);
    env.push(("GROK_HOME".into(), home.to_string_lossy().into_owned()));
    env
}

/// The CLI run in a home, as the user would run it there.
fn in_home(exe: &Path, args: &[&str], app_data_dir: &Path, home: &Path) -> tokio::process::Command {
    let mut command = cli::command(exe, args, &home_env(app_data_dir, home));
    command.env_remove("GROK_AUTH_PROVIDER_COMMAND").env_remove("GROK_AUTH_EXPIRED");
    command
}

pub struct Pending {
    login: cli::Login,
    home: PathBuf,
    pub url: String,
}

pub struct Done {
    pub creds: Creds,
    pub email: String,
}

/// Run `grok login --device-auth` in a new home and wait for its link.
pub async fn begin_sign_in(app_data_dir: &Path) -> Result<Pending, String> {
    let exe = executable().ok_or("accountSources.errors.grokCli")?;
    let home = accounts_dir(app_data_dir).join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&home).map_err(|_| "accountSources.errors.storage")?;
    let command = in_home(&exe, &["login", "--device-auth"], app_data_dir, &home);
    match cli::start_login(command, |_| true, Duration::from_secs(60)).await {
        Ok((login, url)) => Ok(Pending { login, home, url }),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&home);
            Err(error)
        }
    }
}

pub async fn finish_sign_in(pending: Pending, cancel: watch::Receiver<bool>) -> Result<Done, String> {
    let Pending { login, home, .. } = pending;
    let signed_in = login.finish(cancel, Duration::from_secs(900)).await
        .and_then(|()| read_credential(&home).filter(|found| !found.email.is_empty()).ok_or_else(|| "accountSources.errors.noAccount".to_owned()));
    match signed_in {
        Ok(found) => Ok(Done { creds: Creds { home: home.to_string_lossy().into_owned() }, email: found.email }),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&home);
            Err(error)
        }
    }
}

pub enum TokenError {
    SignedOut(String),
    Transient,
}

/// One CLI renewal at a time: its refresh token changes each time.
static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn near_expiry(found: &Credential) -> bool {
    found.expires_at.is_some_and(|at| at - chrono::Utc::now().timestamp() < MARGIN_SECONDS)
}

/// The account's token, renewed by the CLI in its home when about to expire.
pub async fn token(app_data_dir: &Path, creds: &Creds) -> Result<String, TokenError> {
    let home = PathBuf::from(&creds.home);
    let mut found = read_credential(&home);
    if found.as_ref().is_some_and(near_expiry) {
        if let Some(exe) = executable() {
            let _renewing = RENEWING.lock().await;
            found = read_credential(&home);
            if found.as_ref().is_some_and(near_expiry) {
                // Any use of the CLI renews its sign-in; listing models is the lightest.
                let _ = cli::run(in_home(&exe, &["models"], app_data_dir, &home), Duration::from_secs(30)).await;
                found = read_credential(&home);
            }
        }
    }
    match found {
        None => Err(TokenError::SignedOut("not signed in".into())),
        Some(found) if found.expires_at.is_some_and(|at| at <= chrono::Utc::now().timestamp()) => {
            Err(TokenError::SignedOut("the sign-in has expired".into()))
        }
        Some(found) if found.key.is_empty() => Err(TokenError::Transient),
        Some(found) => Ok(found.key),
    }
}

static VERSION: tokio::sync::Mutex<Option<(String, std::time::Instant)>> = tokio::sync::Mutex::const_new(None);

/// The CLI version the backend is told: the installed one when newer.
async fn version() -> String {
    let mut cached = VERSION.lock().await;
    if let Some((version, at)) = cached.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(600)) {
        let _ = at;
        return version.clone();
    }
    let mut version = CLIENT_VERSION.to_owned();
    if let Some(exe) = executable() {
        if let Ok(out) = cli::run(cli::command(&exe, &["--version"], &[]), Duration::from_secs(5)).await {
            if let Some(found) = semver(&out).filter(|found| newer(found, &version)) {
                version = found;
            }
        }
    }
    *cached = Some((version.clone(), std::time::Instant::now()));
    version
}

fn semver(text: &str) -> Option<String> {
    text.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find(|part| part.split('.').count() == 3 && part.split('.').all(|n| !n.is_empty()))
        .map(str::to_owned)
}

fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|n| n.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// The headers that say a request comes from the Grok CLI, whose backend
/// wants to know its version.
pub async fn request_headers(token: &str) -> Vec<(&'static str, String)> {
    let version = version().await;
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    vec![
        ("authorization", format!("Bearer {token}")),
        ("x-grok-client-version", version.clone()),
        ("x-xai-token-auth", "xai-grok-cli".into()),
        ("x-grok-client-identifier", "grok-shell".into()),
        ("x-grok-client-mode", "headless".into()),
        ("user-agent", format!("grok-shell/{version} ({os}; {arch})")),
    ]
}

/// The model asked for, and the conversation, which keeps it on the machine
/// that has its prompt cached.
pub fn body_headers(body: &[u8]) -> Vec<(&'static str, String)> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else { return Vec::new() };
    let mut headers = Vec::new();
    if let Some(model) = value["model"].as_str().filter(|model| !model.is_empty()) {
        headers.push(("x-grok-model-override", model.to_owned()));
    }
    if let Some(key) = value["prompt_cache_key"].as_str().filter(|key| !key.is_empty()) {
        headers.push(("x-grok-conv-id", key.to_owned()));
    }
    headers
}

/// The tool types Grok's backend takes; it turns a whole request away over
/// another (Codex's freeform apply_patch, a namespace).
const TOOLS: [&str; 11] = ["function", "web_search", "x_search", "image_generation", "collections_search", "file_search",
    "code_execution", "code_interpreter", "mcp", "shell", "tool_search"];

/// The request as Grok's backend takes it, `None` when unchanged: namespaced
/// functions flat (restored in the reply), no tool type it refuses, every
/// function's parameters an object at the root, no tool_choice without
/// tools, a Codex agent_message as the user's message, and no null content
/// on handed-back reasoning.
pub fn shape_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let mut dirty = false;
    let has_namespace = value["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| tool["type"] == "namespace"))
        || value["input"].as_array().is_some_and(|items| items.iter().any(|item| item.get("namespace").is_some()));
    if has_namespace && ToolNamespaces::prepare(&mut value).is_ok() {
        dirty = true;
    }
    if let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        tools.retain(|tool| tool["type"].as_str().is_some_and(|kind| TOOLS.contains(&kind)));
        dirty |= tools.len() != before;
        for tool in tools.iter_mut() {
            if let Some(object) = tool.as_object_mut() {
                dirty |= object.remove("external_web_access").is_some();
            }
            if tool["type"] == "function" {
                if let Some(parameters) = tool.get_mut("parameters").filter(|parameters| parameters.is_object()) {
                    dirty |= object_root(parameters);
                }
            }
        }
    }
    let no_tools = value["tools"].as_array().is_none_or(Vec::is_empty);
    if let Some(object) = value.as_object_mut() {
        let refused = object.get("tool_choice").is_some_and(|choice| {
            no_tools || choice.as_object().is_some_and(|choice| choice["type"].as_str().is_some_and(|kind| !TOOLS.contains(&kind)))
        });
        if refused {
            object.remove("tool_choice");
            dirty = true;
        }
    }
    if let Some(items) = value.get_mut("input").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            if let Some(message) = agent_message(item) {
                *item = message;
                dirty = true;
            }
            if item["type"] == "reasoning" && item.get("content").is_some_and(Value::is_null) {
                item.as_object_mut().map(|object| object.remove("content"));
                dirty = true;
            }
        }
    }
    dirty.then(|| serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

/// A plain Codex agent_message (a subagent's task or reply) as the user's
/// message, its sender and recipient said first; a sealed one stays.
fn agent_message(item: &Value) -> Option<Value> {
    if item["type"] != "agent_message" || item.get("encrypted_content").is_some() {
        return None;
    }
    let parts = item["content"].as_array().filter(|parts| !parts.is_empty())?;
    let plain = parts.iter().all(|part| part.get("encrypted_content").is_none()
        && ((part["type"] == "input_text" && part["text"].is_string()) || (part["type"] == "input_image" && part["image_url"].is_string())));
    if !plain {
        return None;
    }
    let header = match (item["author"].as_str().filter(|s| !s.is_empty()), item["recipient"].as_str().filter(|s| !s.is_empty())) {
        (Some(from), Some(to)) => format!("From {from} to {to}"),
        (Some(from), None) => format!("From {from}"),
        (None, Some(to)) => format!("To {to}"),
        (None, None) => String::new(),
    };
    let mut content = Vec::new();
    if !header.is_empty() {
        content.push(json!({"type": "input_text", "text": format!("{header}\n\n")}));
    }
    content.extend(parts.iter().cloned());
    Some(json!({"type": "message", "role": "user", "content": content}))
}

/// A tool's parameters as a plain object at the root, which Grok's backend
/// wants: allOf branches merged, and of anyOf / oneOf the object branches'
/// properties merged, a field each requires staying required.
fn object_root(schema: &mut Value) -> bool {
    let Some(object) = schema.as_object_mut() else { return false };
    let unions = ["anyOf", "oneOf", "allOf"].iter().any(|key| object.contains_key(*key));
    if object.get("type").is_some_and(|kind| kind == "object") && !unions {
        return false;
    }
    let mut properties = object.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut required: Vec<Value> = object.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
    if let Some(Value::Array(all)) = object.remove("allOf") {
        for branch in &all {
            for (key, field) in branch["properties"].as_object().into_iter().flatten() {
                properties.entry(key.clone()).or_insert_with(|| field.clone());
            }
            required.extend(branch["required"].as_array().into_iter().flatten().cloned());
        }
    }
    let mut common: Option<Vec<Value>> = None;
    let mut offered: Map<String, Value> = Map::new();
    for key in ["anyOf", "oneOf"] {
        let Some(Value::Array(branches)) = object.remove(key) else { continue };
        for mut branch in branches {
            if ["anyOf", "oneOf", "allOf"].iter().any(|nested| branch.get(*nested).is_some()) {
                object_root(&mut branch);
            }
            if branch["type"] != "object" && branch.get("properties").is_none() {
                continue;
            }
            for (name, field) in branch["properties"].as_object().into_iter().flatten() {
                if properties.contains_key(name) {
                    continue;
                }
                match offered.get_mut(name) {
                    None => {
                        offered.insert(name.clone(), field.clone());
                    }
                    Some(existing) if existing == field => {}
                    Some(existing) => {
                        let mut choices = match existing.get("anyOf").filter(|_| existing.as_object().is_some_and(|o| o.len() == 1)) {
                            Some(Value::Array(choices)) => choices.clone(),
                            _ => vec![existing.clone()],
                        };
                        if !choices.contains(field) {
                            choices.push(field.clone());
                        }
                        *existing = json!({"anyOf": choices});
                    }
                }
            }
            let branch_required: Vec<Value> = branch["required"].as_array().cloned().unwrap_or_default();
            common = Some(match common {
                None => branch_required,
                Some(kept) => kept.into_iter().filter(|name| branch_required.contains(name)).collect(),
            });
        }
    }
    properties.extend(offered);
    required.extend(common.unwrap_or_default());
    let mut seen = std::collections::HashSet::new();
    required.retain(|name| name.as_str().is_none_or(|name| seen.insert(name.to_owned())));
    object.insert("type".into(), json!("object"));
    object.insert("properties".into(), Value::Object(properties));
    if required.is_empty() {
        object.remove("required");
    } else {
        object.insert("required".into(), Value::Array(required));
    }
    true
}

/// The reply as the client takes it: namespaced calls under their own names
/// again, and whole numbers in a call's arguments without the zero fraction
/// Grok's models write (`2500.0`), which a client typing its arguments
/// strictly (Codex) turns away.
pub struct ReplyFix {
    namespaces: ToolNamespaces,
}

/// What reads this request's reply, from the request before it was shaped.
pub fn reply_fix(body: &[u8]) -> ReplyFix {
    let namespaces = serde_json::from_slice::<Value>(body).ok()
        .and_then(|mut value| ToolNamespaces::prepare(&mut value).ok())
        .unwrap_or_default();
    ReplyFix { namespaces }
}

impl ReplyFix {
    pub async fn apply(self, response: reqwest::Response) -> reqwest::Response {
        if !response.status().is_success() {
            return response;
        }
        let status = response.status();
        let streamed = response.headers().get(hyper::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("event-stream"));
        if !streamed {
            let bytes = response.bytes().await.unwrap_or_default();
            let body = match serde_json::from_slice::<Value>(&bytes) {
                Ok(mut value) => {
                    self.namespaces.restore_response(&mut value);
                    fix_output(&mut value);
                    value.to_string().into_bytes()
                }
                Err(_) => bytes.to_vec(),
            };
            return reqwest::Response::from(hyper::Response::builder().status(status)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(body).expect("static response parts"));
        }
        let state = (response.bytes_stream(), self, String::new(), HashMap::<String, Integral>::new());
        let stream = futures_util::stream::unfold(Some(state), |state| async move {
            let (mut upstream, fix, mut pending, mut calls) = state?;
            loop {
                match upstream.next().await {
                    Some(Ok(chunk)) => {
                        pending.push_str(&String::from_utf8_lossy(&chunk));
                        let mut out = String::new();
                        while let Some(end) = pending.find("\n\n") {
                            let event: String = pending.drain(..end + 2).collect();
                            out.push_str(&fix.event(event, &mut calls));
                        }
                        if !out.is_empty() {
                            return Some((Ok::<_, std::io::Error>(Bytes::from(out)), Some((upstream, fix, pending, calls))));
                        }
                    }
                    Some(Err(error)) => return Some((Err(std::io::Error::other(error.to_string())), None)),
                    None => {
                        let rest = if pending.trim().is_empty() { String::new() } else { fix.event(std::mem::take(&mut pending), &mut calls) };
                        return (!rest.is_empty()).then(|| (Ok(Bytes::from(rest)), None));
                    }
                }
            }
        });
        reqwest::Response::from(hyper::Response::builder().status(status)
            .header(hyper::header::CONTENT_TYPE, "text/event-stream")
            .body(reqwest::Body::wrap_stream(stream)).expect("static response parts"))
    }

    fn event(&self, event: String, calls: &mut HashMap<String, Integral>) -> String {
        let event = self.namespaces.restore_event(event);
        let Some((prefix, data)) = event.split_once("data: ") else { return event };
        let Ok(mut value) = serde_json::from_str::<Value>(data.trim()) else { return event };
        match value["type"].as_str().unwrap_or_default() {
            "response.function_call_arguments.delta" => {
                let id = value["item_id"].as_str().unwrap_or_default().to_owned();
                if let Some(delta) = value["delta"].as_str() {
                    let fixed = calls.entry(id).or_default().feed(delta);
                    value["delta"] = json!(fixed);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = value["arguments"].as_str() {
                    value["arguments"] = json!(integral(arguments));
                }
                if let Some(id) = value["item_id"].as_str() {
                    calls.remove(id);
                }
            }
            _ => {
                if let Some(item) = value.get_mut("item") {
                    fix_item(item);
                }
                if let Some(response) = value.get_mut("response") {
                    fix_output(response);
                }
            }
        }
        format!("{prefix}data: {value}\n\n")
    }
}

fn fix_item(item: &mut Value) {
    if item["type"] == "function_call" {
        if let Some(arguments) = item["arguments"].as_str() {
            item["arguments"] = json!(integral(arguments));
        }
    }
}

fn fix_output(response: &mut Value) {
    for item in response.get_mut("output").and_then(Value::as_array_mut).into_iter().flatten() {
        fix_item(item);
    }
}

/// A call's arguments with each number written as a whole with a zero
/// fraction (`2500.0`) written as the whole (`2500`); strings, other
/// fractions and exponents stay as written.
pub fn integral(text: &str) -> String {
    let mut state = Integral::default();
    let mut out = state.feed(text);
    out.push_str(&state.finish());
    out
}

/// The same rule over arguments arriving in pieces cut anywhere: a `.0…`
/// is held until what follows shows whether it goes.
#[derive(Default)]
pub struct Integral {
    in_string: bool,
    escaped: bool,
    number: Number,
    held: String,
}

#[derive(Default, PartialEq)]
enum Number {
    #[default]
    None,
    Whole,
    /// `.` and zeros after a whole, held.
    Zeros,
    /// A fraction or exponent: written as it is.
    Other,
}

impl Integral {
    pub fn feed(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if self.in_string {
                out.push(ch);
                if self.escaped {
                    self.escaped = false;
                } else if ch == '\\' {
                    self.escaped = true;
                } else if ch == '"' {
                    self.in_string = false;
                }
                continue;
            }
            match self.number {
                Number::Whole if ch == '.' => {
                    self.held.push(ch);
                    self.number = Number::Zeros;
                    continue;
                }
                Number::Whole if ch.is_ascii_digit() => {
                    out.push(ch);
                    continue;
                }
                Number::Whole if ch == 'e' || ch == 'E' => {
                    out.push(ch);
                    self.number = Number::Other;
                    continue;
                }
                Number::Zeros if ch == '0' => {
                    self.held.push(ch);
                    continue;
                }
                Number::Zeros if ch.is_ascii_digit() || ch == 'e' || ch == 'E' => {
                    out.push_str(&std::mem::take(&mut self.held));
                    out.push(ch);
                    self.number = Number::Other;
                    continue;
                }
                Number::Zeros => {
                    // `2500.` alone is no JSON number; anything else ends a whole one.
                    if self.held == "." {
                        out.push('.');
                    }
                    self.held.clear();
                    self.number = Number::None;
                }
                Number::Other if ch.is_ascii_digit() || "eE+-.".contains(ch) => {
                    out.push(ch);
                    continue;
                }
                _ => self.number = Number::None,
            }
            if ch == '"' {
                self.in_string = true;
            } else if ch.is_ascii_digit() || ch == '-' {
                self.number = Number::Whole;
            }
            out.push(ch);
        }
        out
    }

    /// What is held when the arguments end.
    pub fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        self.number = Number::None;
        if held == "." { held } else { String::new() }
    }
}

/// What the account can use, as the CLI's backend lists it.
pub async fn fetch_models(client: &Client, token: &str) -> Result<Vec<SourceModel>, String> {
    let mut request = client.get(format!("{}/models", api_base().trim_end_matches('/')));
    for (name, value) in request_headers(token).await {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|_| "accountSources.errors.network")?;
    if !response.status().is_success() {
        return Err(format!("Grok models: HTTP {}", response.status().as_u16()));
    }
    let models = parse_models(&response.bytes().await.map_err(|_| "accountSources.errors.network")?);
    if models.is_empty() {
        return Err("accountSources.errors.noModels".into());
    }
    Ok(models)
}

fn parse_models(body: &[u8]) -> Vec<SourceModel> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else { return Vec::new() };
    value["data"].as_array().into_iter().flatten().filter_map(|model| {
        let id = model["id"].as_str().filter(|id| !id.is_empty())?;
        if model["api_backend"].as_str().is_some_and(|backend| !backend.is_empty() && backend != "responses") {
            return None;
        }
        // Listed hardest first; kept easiest first, as the other sources keep them.
        let mut reasoning: Vec<String> = model["reasoning_efforts"].as_array().into_iter().flatten()
            .filter_map(|effort| effort["value"].as_str().filter(|value| !value.is_empty()).map(str::to_owned)).collect();
        reasoning.reverse();
        Some(SourceModel {
            id: id.to_owned(),
            name: model["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(id).to_owned(),
            context_window: model["context_window"].as_u64().filter(|window| *window > 0),
            reasoning,
            ..Default::default()
        })
    }).collect()
}

/// How much of the period's allowance is gone, as the CLI's /usage reads it.
pub async fn quota(client: &Client, token: &str) -> Quota {
    let checked_at = chrono::Utc::now().timestamp();
    let response = client.get(format!("{}/billing?format=credits", api_base().trim_end_matches('/')))
        .bearer_auth(token).header("accept", "application/json").send().await;
    let value = match response {
        Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
        _ => None,
    };
    match value.as_ref().and_then(|value| value["config"]["creditUsagePercent"].as_f64()) {
        Some(used) => Quota { used: used.clamp(0.0, 100.0).round(), limit: 100.0, unit: "percent".into(), checked_at, ..Default::default() },
        None => Quota { checked_at, error: Some("Grok did not report its allowance".into()), ..Default::default() },
    }
}

/// What the user can do about a refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    match status {
        401 | 403 if text.contains("outdated") || text.contains("version") => {
            Some("Grok turned away this Grok CLI version; update Grok Build (`grok update`) and try again".into())
        }
        401 | 403 => Some("Grok refused the sign-in; sign in to Grok again on AgentHarbor's Accounts page".into()),
        429 => Some("the Grok subscription's allowance is used up for now; it resets with its period".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_numbers_lose_a_zero_fraction_and_nothing_else_changes() {
        let arguments = r#"{"yield_time_ms":2500.0,"session_id":17277.00,"ratio":2.5,"big":2.0e3,"neg":-3.0,"text":"v1.0 and 2.0","list":[1.0,0.0],"plain":7}"#;
        assert_eq!(integral(arguments), r#"{"yield_time_ms":2500,"session_id":17277,"ratio":2.5,"big":2.0e3,"neg":-3,"text":"v1.0 and 2.0","list":[1,0],"plain":7}"#);
        // The same, cut anywhere.
        for cut in 1..arguments.len() {
            if !arguments.is_char_boundary(cut) {
                continue;
            }
            let mut state = Integral::default();
            let mut out = state.feed(&arguments[..cut]);
            out.push_str(&state.feed(&arguments[cut..]));
            out.push_str(&state.finish());
            assert_eq!(out, integral(arguments), "cut at {cut}");
        }
        assert_eq!(integral(r#"{"escaped":"a\"1.0","n":1.0}"#), r#"{"escaped":"a\"1.0","n":1}"#);
    }

    #[test]
    fn a_codex_request_is_shaped_into_what_grok_takes() {
        let body = json!({
            "model": "grok-4.7",
            "prompt_cache_key": "thread-1",
            "tools": [
                {"type": "function", "name": "exec", "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}},
                {"type": "custom", "name": "apply_patch"},
                {"type": "web_search", "external_web_access": true},
                {"type": "namespace", "name": "collaboration", "tools": [{"type": "function", "name": "spawn_agent", "parameters": {"type": "object"}}]},
                {"type": "function", "name": "update", "parameters": {"anyOf": [
                    {"type": "object", "properties": {"id": {"type": "string"}, "a": {"type": "string"}}, "required": ["id", "a"]},
                    {"type": "object", "properties": {"id": {"type": "string"}, "b": {"type": "number"}}, "required": ["id"]}
                ]}}
            ],
            "tool_choice": {"type": "custom", "name": "apply_patch"},
            "input": [
                {"type": "agent_message", "author": "lead", "recipient": "worker", "content": [{"type": "input_text", "text": "do it"}]},
                {"type": "reasoning", "summary": [], "content": null, "encrypted_content": "x"}
            ]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let shaped: Value = serde_json::from_slice(&shape_body(&bytes).unwrap()).unwrap();
        let types: Vec<_> = shaped["tools"].as_array().unwrap().iter().map(|tool| tool["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["function", "web_search", "function", "function"]);
        assert!(shaped["tools"][1].get("external_web_access").is_none());
        let flat = shaped["tools"][2]["name"].as_str().unwrap();
        assert!(flat.starts_with("ahns_"), "a namespaced function goes flat");
        let update = &shaped["tools"][3]["parameters"];
        assert_eq!(update["type"], "object");
        assert!(update.get("anyOf").is_none());
        assert_eq!(update["required"], json!(["id"]));
        assert_eq!(update["properties"]["b"]["type"], "number");
        assert!(shaped.get("tool_choice").is_none());
        assert_eq!(shaped["input"][0], json!({"type": "message", "role": "user", "content": [
            {"type": "input_text", "text": "From lead to worker\n\n"}, {"type": "input_text", "text": "do it"}]}));
        assert!(shaped["input"][1].get("content").is_none());
        assert_eq!(body_headers(&bytes), vec![("x-grok-model-override", "grok-4.7".to_owned()), ("x-grok-conv-id", "thread-1".to_owned())]);
        let plain = serde_json::to_vec(&json!({"model": "grok-4.7", "input": "hi"})).unwrap();
        assert!(shape_body(&plain).is_none(), "a request Grok takes as it is stays as it is");
    }

    #[tokio::test]
    async fn a_reply_names_calls_as_the_client_offered_them_with_whole_numbers() {
        let request = serde_json::to_vec(&json!({"tools": [{"type": "namespace", "name": "collaboration",
            "tools": [{"type": "function", "name": "spawn_agent", "parameters": {"type": "object"}}]}], "input": "go"})).unwrap();
        let shaped: Value = serde_json::from_slice(&shape_body(&request).unwrap()).unwrap();
        let flat = shaped["tools"][0]["name"].as_str().unwrap().to_owned();
        let events = [
            json!({"type": "response.output_item.added", "item": {"type": "function_call", "id": "fc", "name": flat, "arguments": ""}}),
            json!({"type": "response.function_call_arguments.delta", "item_id": "fc", "delta": "{\"wait\":25"}),
            json!({"type": "response.function_call_arguments.delta", "item_id": "fc", "delta": "00.0"}),
            json!({"type": "response.function_call_arguments.delta", "item_id": "fc", "delta": "}"}),
            json!({"type": "response.function_call_arguments.done", "item_id": "fc", "arguments": "{\"wait\":2500.0}"}),
            json!({"type": "response.completed", "response": {"output": [{"type": "function_call", "name": flat, "arguments": "{\"wait\":2500.0}"}]}}),
        ];
        let sse: String = events.iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect();
        let upstream = reqwest::Response::from(hyper::Response::builder().status(200)
            .header("content-type", "text/event-stream").body(sse).unwrap());
        let text = reply_fix(&request).apply(upstream).await.text().await.unwrap();
        let read: Vec<Value> = text.split("\n\n").filter_map(|event| event.split_once("data: ")).map(|(_, data)| serde_json::from_str(data).unwrap()).collect();
        assert_eq!(read[0]["item"]["name"], "spawn_agent");
        assert_eq!(read[0]["item"]["namespace"], "collaboration");
        let deltas: String = read[1..4].iter().map(|event| event["delta"].as_str().unwrap()).collect();
        assert_eq!(deltas, "{\"wait\":2500}");
        assert_eq!(read[4]["arguments"], "{\"wait\":2500}");
        assert_eq!(read[5]["response"]["output"][0]["name"], "spawn_agent");
        assert_eq!(read[5]["response"]["output"][0]["arguments"], "{\"wait\":2500}");
    }

    #[test]
    fn models_and_the_sign_in_are_read_as_the_cli_keeps_them() {
        let models = parse_models(br#"{"data":[
            {"id":"grok-4.7","name":"Grok 4.7","context_window":2000000,"api_backend":"responses","reasoning_efforts":[{"value":"high"},{"value":"low"}]},
            {"id":"grok-imagine","api_backend":"images"}]}"#);
        assert_eq!(models, vec![SourceModel { id: "grok-4.7".into(), name: "Grok 4.7".into(), context_window: Some(2_000_000),
            reasoning: vec!["low".into(), "high".into()], ..Default::default() }]);
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("auth.json"), r#"{"https://auth.x.ai":{"key":"tok","email":"me@x.ai","expires_at":"2099-01-01T00:00:00Z"}}"#).unwrap();
        let found = read_credential(home.path()).unwrap();
        assert_eq!((found.key.as_str(), found.email.as_str()), ("tok", "me@x.ai"));
        assert!(!near_expiry(&found));
        assert_eq!(semver("grok 1.0.52 (4220f3b224a6)").as_deref(), Some("1.0.52"));
        assert!(newer("1.0.52", "1.0.41") && !newer("1.0.9", "1.0.41"));
    }
}
