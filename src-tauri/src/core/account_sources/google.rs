//! Google's subscriptions: Gemini CLI's and Antigravity's Google sign-ins,
//! both served by Google's Code Assist backend (cloudcode-pa) under a Google
//! Cloud project Code Assist gives the account. Each signs in with its app's
//! own OAuth client (an installed app's, whose "secret" ships with the app);
//! requests go out in Code Assist's envelope (`codeassist`).
//!
//! Google serves Gemini CLI's sign-in only to Gemini Code Assist Standard
//! and Enterprise, which bill a project the user names. Antigravity's serves
//! personal accounts, but Google may suspend an account it sees used outside
//! Antigravity. Ported from magpie (MIT, see LICENSES/THIRD_PARTY.md).
use super::codeassist::App;
use super::store::{Quota, SourceModel};
use super::SourceKind;
use base64::Engine as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::{net::TcpListener, sync::watch};

const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const USER_INFO: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com";
const CODE_ASSIST_DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";
const ANTIGRAVITY_UPDATES: &str = "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml";
/// The Gemini CLI this sign-in says it is.
const GEMINI_CLI_VERSION: &str = "0.61.0";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
const MARGIN_MS: i64 = 120_000;

/// What the app says before an Antigravity account is added.
pub const ANTIGRAVITY_RISK: &str = "Google may suspend an Antigravity account it sees used outside Antigravity. Use one you can afford to lose.";

/// One of Google's apps whose sign-in is borrowed: its installed-app OAuth
/// client as the app itself ships it. An installed app's client secret is
/// public by design and identifies the app, not the user.
struct Client_ {
    id: &'static str,
    secret: &'static str,
    scopes: &'static [&'static str],
    callback: &'static str,
    loopback: &'static str,
}

fn oauth(app: App) -> Client_ {
    match app {
        App::GeminiCli => Client_ {
            id: "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
            secret: "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",
            scopes: &["https://www.googleapis.com/auth/cloud-platform", "https://www.googleapis.com/auth/userinfo.email",
                "https://www.googleapis.com/auth/userinfo.profile"],
            callback: "/oauth2callback",
            loopback: "127.0.0.1",
        },
        App::Antigravity => Client_ {
            id: "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
            secret: "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",
            scopes: &["https://www.googleapis.com/auth/cloud-platform", "https://www.googleapis.com/auth/userinfo.email",
                "https://www.googleapis.com/auth/userinfo.profile", "https://www.googleapis.com/auth/cclog",
                "https://www.googleapis.com/auth/experimentsandconfigs"],
            callback: "/oauth-callback",
            loopback: "localhost",
        },
    }
}

pub fn app_of(kind: SourceKind) -> App {
    if kind == SourceKind::Antigravity { App::Antigravity } else { App::GeminiCli }
}

fn kind_of(app: App) -> SourceKind {
    match app {
        App::GeminiCli => SourceKind::GeminiCli,
        App::Antigravity => SourceKind::Antigravity,
    }
}

fn overridden(app: App) -> Option<String> {
    super::endpoint_override(kind_of(app))
}

fn token_url(app: App) -> String {
    overridden(app).map(|base| format!("{base}/token")).unwrap_or_else(|| TOKEN.into())
}

fn user_info_url(app: App) -> String {
    overridden(app).map(|base| format!("{base}/userinfo")).unwrap_or_else(|| USER_INFO.into())
}

/// Where requests go: Antigravity's daily backend, Gemini CLI's production one.
pub fn base(app: App) -> String {
    overridden(app).unwrap_or_else(|| if app == App::Antigravity { CODE_ASSIST_DAILY } else { CODE_ASSIST }.into())
}

/// Where the account's project is found.
fn load_base(app: App) -> String {
    overridden(app).unwrap_or_else(|| CODE_ASSIST.into())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub email: String,
    pub access: String,
    /// Google's refresh tokens do not turn over.
    pub refresh: String,
    /// Unix milliseconds.
    pub expires_at: i64,
    /// The Google Cloud project requests are billed to; empty until Code
    /// Assist gives one or the user names it.
    pub project: String,
    /// The tier the account is on and the one it pays for (Antigravity names them when listing models).
    pub tier: String,
    pub paid: String,
}

impl Creds {
    pub fn near_expiry(&self) -> bool {
        self.expires_at - chrono::Utc::now().timestamp_millis() < MARGIN_MS
    }
}

fn platform() -> (&'static str, &'static str) {
    let os = match std::env::consts::OS { "windows" => "win32", "macos" => "darwin", other => other };
    let arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", other => other };
    (os, arch)
}

static ANTIGRAVITY_VERSION: Mutex<Option<(String, i64)>> = Mutex::new(None);

/// Antigravity's newest version, which its requests carry; asked of its
/// updater at most every six hours.
async fn antigravity_version(client: &Client) -> String {
    let now = chrono::Utc::now().timestamp();
    if let Some((version, at)) = ANTIGRAVITY_VERSION.lock().ok().and_then(|cached| cached.clone()) {
        if now - at < 6 * 3600 {
            return version;
        }
    }
    let mut version = "2.9.1".to_owned();
    if overridden(App::Antigravity).is_none() {
        if let Ok(response) = client.get(ANTIGRAVITY_UPDATES).header("user-agent", "electron-builder")
            .timeout(Duration::from_secs(5)).send().await
        {
            if let Ok(text) = response.text().await {
                if let Some(found) = text.lines().find_map(|line| line.strip_prefix("version:"))
                    .map(|value| value.trim().trim_matches(['\'', '"']).to_owned())
                    .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.'))
                {
                    version = found;
                }
            }
        }
    }
    if let Ok(mut cached) = ANTIGRAVITY_VERSION.lock() {
        *cached = Some((version.clone(), now));
    }
    version
}

/// What the app says it is to Code Assist.
pub async fn user_agent(client: &Client, app: App, model: &str) -> String {
    let (os, arch) = platform();
    match app {
        App::Antigravity => format!("antigravity/hub/{} {}/{}", antigravity_version(client).await, std::env::consts::OS, match arch {
            "x64" => "amd64", other => other,
        }),
        App::GeminiCli => format!("GeminiCLI/{GEMINI_CLI_VERSION}/{} ({os}; {arch}; terminal)", if model.is_empty() { "gemini-2.5-pro" } else { model }),
    }
}

// ---- signing in ---------------------------------------------------------------

pub struct SignIn {
    pub url: String,
    listener: TcpListener,
    redirect: String,
    state: String,
    verifier: String,
    app: App,
}

/// Listen for the browser on a free loopback port and build Google's page.
pub async fn begin_sign_in(app: App) -> Result<SignIn, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|_| "accounts.loginFailed")?;
    let port = listener.local_addr().map_err(|_| "accounts.loginFailed")?.port();
    let client = oauth(app);
    let redirect = format!("http://{}:{port}{}", client.loopback, client.callback);
    let (state, verifier) = (super::chatgpt_api::random_token(24), super::chatgpt_api::random_token(48));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = reqwest::Url::parse(&overridden(app).map(|base| format!("{base}/authorize")).unwrap_or_else(|| AUTHORIZE.into()))
        .map_err(|_| "accounts.loginFailed")?;
    url.query_pairs_mut()
        .append_pair("client_id", client.id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", &client.scopes.join(" "))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(SignIn { url: url.to_string(), listener, redirect, state, verifier, app })
}

pub struct SignedIn {
    pub creds: Creds,
    pub plan: Option<String>,
    /// Why Code Assist has no project for the account yet (Gemini CLI).
    pub note: Option<String>,
}

/// Wait for the browser, trade the code, find whose account it is and set it up for Code Assist.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, cancel: watch::Receiver<bool>) -> Result<SignedIn, String> {
    let app = sign_in.app;
    let (params, mut socket) = super::loopback::wait(&sign_in.listener, oauth(app).callback, &sign_in.state, SIGN_IN_WINDOW, cancel).await?;
    let result = async {
        let code = params.get("code").cloned().unwrap_or_default();
        let oauth = oauth(app);
        let reply: Value = client.post(token_url(app)).form(&[
            ("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", sign_in.redirect.as_str()),
            ("client_id", oauth.id), ("client_secret", oauth.secret), ("code_verifier", sign_in.verifier.as_str()),
        ]).timeout(Duration::from_secs(30)).send().await.map_err(|_| "accountSources.errors.googleUnreachable".to_owned())?
            .json().await.map_err(|_| "accountSources.errors.googleNoToken".to_owned())?;
        let access = reply["access_token"].as_str().unwrap_or_default().to_owned();
        let refresh = reply["refresh_token"].as_str().unwrap_or_default().to_owned();
        if access.is_empty() || refresh.is_empty() {
            return Err("accountSources.errors.googleNoToken".to_owned());
        }
        let expires_at = chrono::Utc::now().timestamp_millis() + reply["expires_in"].as_i64().unwrap_or(3600) * 1000;
        let who: Value = client.get(user_info_url(app)).bearer_auth(&access).timeout(Duration::from_secs(30)).send().await
            .map_err(|_| "accountSources.errors.googleUnreachable".to_owned())?.json().await.unwrap_or(Value::Null);
        let email = who["email"].as_str().unwrap_or_default().to_owned();
        if email.is_empty() {
            return Err("accountSources.errors.googleNoUser".to_owned());
        }
        let mut creds = Creds { email, access, refresh, expires_at, ..Default::default() };
        // Kept even without a project: a Standard account names one afterwards.
        let (plan, note) = match setup(client, app, &mut creds).await {
            Ok(plan) => (plan, None),
            Err(why) => (None, Some(why)),
        };
        Ok(SignedIn { creds, plan, note })
    }.await;
    match &result {
        Ok(done) => super::loopback::reply(&mut socket, true, &format!("{} is added to AgentHarbor. You can close this tab.", done.creds.email)).await,
        Err(error) => super::loopback::reply(&mut socket, false, error).await,
    }
    result
}

pub enum RefreshError {
    SignedOut(String),
    Transient,
}

/// A fresh access token.
pub async fn refresh(client: &Client, app: App, creds: &Creds) -> Result<Creds, RefreshError> {
    let oauth = oauth(app);
    let response = client.post(token_url(app)).form(&[
        ("grant_type", "refresh_token"), ("refresh_token", creds.refresh.as_str()), ("client_id", oauth.id), ("client_secret", oauth.secret),
    ]).timeout(Duration::from_secs(30)).send().await.map_err(|_| RefreshError::Transient)?;
    let status = response.status();
    let reply: Value = response.json().await.unwrap_or(Value::Null);
    let access = reply["access_token"].as_str().unwrap_or_default();
    if !status.is_success() || access.is_empty() {
        return Err(if matches!(status.as_u16(), 400 | 401) {
            RefreshError::SignedOut(reply["error"].as_str().unwrap_or("signed out").to_owned())
        } else {
            RefreshError::Transient
        });
    }
    Ok(Creds { access: access.to_owned(), expires_at: chrono::Utc::now().timestamp_millis() + reply["expires_in"].as_i64().unwrap_or(3600) * 1000,
        ..creds.clone() })
}

// ---- Code Assist ----------------------------------------------------------------

/// Post one of Code Assist's methods.
async fn call(client: &Client, app: App, creds: &Creds, base: &str, method: &str, body: Value, headers: &[(&str, String)]) -> Result<Value, String> {
    let mut request = client.post(format!("{base}/v1internal:{method}")).bearer_auth(&creds.access)
        .header("user-agent", user_agent(client, app, "").await).json(&body).timeout(Duration::from_secs(30));
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request.send().await.map_err(|_| "accountSources.errors.googleUnreachable".to_owned())?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(value["error"]["message"].as_str().map(str::to_owned).unwrap_or_else(|| format!("Code Assist {method}: {status}")));
    }
    Ok(value)
}

/// Code Assist's project, which comes as an id or as {id, name}.
fn project_of(value: &Value) -> String {
    value.as_str().or(value["id"].as_str()).unwrap_or_default().to_owned()
}

fn tier_name(tier: &Value) -> Option<String> {
    tier["name"].as_str().or(tier["id"].as_str()).filter(|name| !name.is_empty()).map(str::to_owned)
}

/// Google's reason for not serving the account, when it gave one.
fn ineligible(load: &Value) -> Option<String> {
    load["ineligibleTiers"].as_array()?.iter().find_map(|tier| tier["reasonMessage"].as_str().filter(|text| !text.is_empty()).map(str::to_owned))
}

/// Set the account up for Code Assist the way its app does, keeping the
/// project in `creds`; the plan's name when Google says it.
pub async fn setup(client: &Client, app: App, creds: &mut Creds) -> Result<Option<String>, String> {
    let load_body = match app {
        App::GeminiCli => {
            let mut meta = json!({"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"});
            let mut body = json!({"metadata": meta});
            if !creds.project.is_empty() {
                meta["duetProject"] = json!(creds.project);
                body = json!({"metadata": meta, "cloudaicompanionProject": creds.project});
            }
            body
        }
        App::Antigravity => json!({"metadata": {"ideType": "ANTIGRAVITY"}}),
    };
    let load = call(client, app, creds, &load_base(app), "loadCodeAssist", load_body, &[]).await?;
    creds.tier = load["currentTier"]["id"].as_str().unwrap_or_default().to_owned();
    creds.paid = load["paidTier"]["id"].as_str().unwrap_or_default().to_owned();
    let plan = tier_name(&load["paidTier"]).or_else(|| tier_name(&load["currentTier"]));
    let given = project_of(&load["cloudaicompanionProject"]);
    if !given.is_empty() {
        creds.project = given;
        return Ok(plan);
    }
    if !creds.project.is_empty() && (app == App::Antigravity || load["currentTier"].is_object()) {
        return Ok(plan);
    }
    let tiers = load["allowedTiers"].as_array().cloned().unwrap_or_default();
    let tier = tiers.iter().find(|tier| tier["isDefault"] == true).or(tiers.first()).cloned()
        .unwrap_or_else(|| json!({"id": "legacy-tier", "userDefinedCloudaicompanionProject": true}));
    let needs_project = || ineligible(&load).map(|why| format!("{why} — {GEMINI_NEEDS_PROJECT}")).unwrap_or_else(|| NEEDS_PROJECT_KEY.into());
    let (request, base, headers) = match app {
        App::GeminiCli => {
            if load["currentTier"].is_object() || (tier["userDefinedCloudaicompanionProject"] == true && creds.project.is_empty()) {
                return Err(needs_project());
            }
            let mut request = json!({"tierId": tier["id"], "metadata": {"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"}});
            if tier["id"] != "free-tier" {
                request["cloudaicompanionProject"] = json!(creds.project);
            }
            (request, load_base(app), Vec::new())
        }
        App::Antigravity => {
            let id = tier["id"].as_str().filter(|id| !id.is_empty() && *id != "legacy-tier").unwrap_or("free-tier");
            let version = antigravity_version(client).await;
            (json!({"tier_id": id, "metadata": {"ide_type": "ANTIGRAVITY", "ide_version": version, "ide_name": "antigravity"}}),
             base(app), vec![("user-agent", format!("{} google-api-nodejs-client/10.3.0", user_agent(client, app, "").await)),
                ("x-goog-api-client", "gl-node/22.21.1".to_owned())])
        }
    };
    for attempt in 0..5 {
        let lro = call(client, app, creds, &base, "onboardUser", request.clone(), &headers).await?;
        if lro["done"] == true {
            let project = project_of(&lro["response"]["cloudaicompanionProject"]);
            if project.is_empty() && creds.project.is_empty() {
                return Err(match app {
                    App::GeminiCli => needs_project(),
                    App::Antigravity => ineligible(&load).map(|why| format!("Antigravity won't serve this Google account: {why}"))
                        .unwrap_or_else(|| "Antigravity hasn't set this Google account up; sign in to the Antigravity app with it once, then try again".into()),
                });
            }
            if !project.is_empty() {
                creds.project = project;
            }
            if creds.tier.is_empty() {
                creds.tier = tier["id"].as_str().unwrap_or_default().to_owned();
            }
            return Ok(plan.or_else(|| tier_name(&tier)));
        }
        if attempt < 4 {
            tokio::time::sleep(if overridden(app).is_some() { Duration::from_millis(5) } else { Duration::from_secs(2) }).await;
        }
    }
    Err("Code Assist is taking long to set the account up; try again in a minute".into())
}

/// The same, as the Accounts page words it.
const NEEDS_PROJECT_KEY: &str = "accountSources.errors.geminiNeedsProject";
pub const GEMINI_NEEDS_PROJECT: &str = "Google serves Gemini CLI's sign-in only to Gemini Code Assist Standard and Enterprise, which bill a Google Cloud project: set the account's project. A personal Google account is served to Antigravity instead.";

// ---- models ---------------------------------------------------------------------

/// The experiments that decide which models Gemini CLI offers an account,
/// and the id each goes out as.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    gemini31: bool,
    no_pro: bool,
    latest_flash: bool,
    latest_flash_lite: bool,
}

static FLAGS: Mutex<Option<HashMap<String, (Flags, i64)>>> = Mutex::new(None);

/// Ask Code Assist which experiments the account is in, at most hourly.
pub async fn flags(client: &Client, creds: &Creds) -> Flags {
    let key = format!("{}\u{0}{}", creds.refresh, creds.project);
    let now = chrono::Utc::now().timestamp();
    if let Some((flags, at)) = FLAGS.lock().ok().and_then(|cached| cached.as_ref()?.get(&key).copied()) {
        if now - at < 3600 {
            return flags;
        }
    }
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "WINDOWS_AMD64", ("macos", "aarch64") => "DARWIN_ARM64", ("macos", _) => "DARWIN_AMD64",
        ("linux", "aarch64") => "LINUX_ARM64", ("linux", _) => "LINUX_AMD64", _ => "PLATFORM_UNSPECIFIED",
    };
    let body = json!({"project": creds.project, "metadata": {"ideName": "IDE_UNSPECIFIED", "pluginType": "GEMINI",
        "ideVersion": GEMINI_CLI_VERSION, "platform": platform, "updateChannel": "stable", "duetProject": creds.project}});
    let mut flags = Flags::default();
    if let Ok(reply) = call(client, App::GeminiCli, creds, &base(App::GeminiCli), "listExperiments", body, &[]).await {
        for flag in reply["flags"].as_array().into_iter().flatten() {
            let on = flag["boolValue"] == true;
            match flag["flagId"].as_i64() {
                Some(45760185) => flags.gemini31 = on,
                Some(45768879) => flags.no_pro = on,
                Some(45842815) => flags.latest_flash = on,
                Some(45827489) => flags.latest_flash_lite = on,
                _ => {}
            }
        }
    }
    if let Ok(mut cached) = FLAGS.lock() {
        cached.get_or_insert_with(HashMap::new).insert(key, (flags, now));
    }
    flags
}

impl Flags {
    /// The id Gemini CLI sends Code Assist a model as.
    pub fn wire(self, model: &str) -> String {
        match model {
            "gemini-3.5-flash" | "gemini-3-flash" if self.latest_flash => "gemini-3.8-flash",
            "gemini-3.5-flash" | "gemini-3.8-flash" if !self.latest_flash => "gemini-3-flash",
            "gemini-3.1-flash-lite" if self.latest_flash_lite => "gemini-3.5-flash-lite",
            "gemini-3.5-flash-lite" if !self.latest_flash_lite => "gemini-3.1-flash-lite",
            other => other,
        }.to_owned()
    }

    /// What Gemini CLI's model picker offers the account: previews only to
    /// one with an allowance for one, 3.1 Pro once launched to it, and the
    /// newest Flash and Flash Lite where rolled out.
    fn offered(self, preview: bool) -> Vec<(&'static str, &'static str)> {
        let skip = |id: &str| match id {
            "gemini-3.1-pro-preview" => !self.gemini31 || !preview || self.no_pro,
            "gemini-3-pro-preview" => self.gemini31 || !preview || self.no_pro,
            "gemini-3-flash-preview" => !preview,
            "gemini-3.8-flash" => !self.latest_flash,
            "gemini-3.5-flash" => self.latest_flash,
            "gemini-3.5-flash-lite" => !self.latest_flash_lite,
            "gemini-3.1-flash-lite" => self.latest_flash_lite,
            "gemini-2.5-pro" => self.no_pro,
            "gemini-2.5-flash" => self.latest_flash,
            _ => false,
        };
        GEMINI_MODELS.iter().copied().filter(|(id, _)| !skip(id)).collect()
    }
}

/// The models Gemini CLI offers.
const GEMINI_MODELS: [(&str, &str); 10] = [
    ("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
    ("gemini-3-pro-preview", "Gemini 3 Pro Preview"),
    ("gemini-3-flash-preview", "Gemini 3 Flash Preview"),
    ("gemini-3.8-flash", "Gemini 3.8 Flash"),
    ("gemini-3.5-flash", "Gemini 3.5 Flash"),
    ("gemini-3.5-flash-lite", "Gemini 3.5 Flash Lite"),
    ("gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite"),
    ("gemini-2.5-pro", "Gemini 2.5 Pro"),
    ("gemini-2.5-flash", "Gemini 2.5 Flash"),
    ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
];

/// Models Antigravity lists that are not for chat.
fn antigravity_hidden(id: &str) -> bool {
    matches!(id, "chat_20706" | "chat_23310" | "tab_flash_lite_preview" | "tab_jump_flash_lite_preview" | "gemini-2.5-flash-thinking" | "gemini-2.5-pro")
        || id.starts_with("chat_") || id.starts_with("tab_")
}

/// Antigravity's own list: its models with how much of each is left, by
/// fetchAvailableModels, naming the account's tier as Antigravity does.
async fn antigravity_list(client: &Client, creds: &Creds) -> Result<Value, String> {
    let mut entitlements: Vec<&str> = [creds.paid.as_str(), creds.tier.as_str()].into_iter()
        .filter(|tier| !tier.is_empty() && *tier != "free-tier" && *tier != "legacy-tier").collect();
    entitlements.dedup();
    let mut last = String::new();
    for tier in entitlements.into_iter().map(Some).chain([None]) {
        let mut body = json!({"project": creds.project});
        if let Some(tier) = tier {
            body["entitlement"] = json!({"userTier": tier});
        }
        match call(client, App::Antigravity, creds, &base(App::Antigravity), "fetchAvailableModels", body, &[]).await {
            Ok(reply) => return Ok(reply),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// The account's models.
pub async fn fetch_models(client: &Client, app: App, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    if creds.project.is_empty() {
        return Err(match app {
            App::GeminiCli => NEEDS_PROJECT_KEY.into(),
            App::Antigravity => "accountSources.errors.googleNoProject".into(),
        });
    }
    match app {
        App::GeminiCli => {
            let quota = call(client, app, creds, &base(app), "retrieveUserQuota", json!({"project": creds.project}), &[]).await?;
            let preview = quota["buckets"].as_array().into_iter().flatten()
                .any(|bucket| bucket["modelId"].as_str().is_some_and(|id| id.contains("-preview")));
            let flags = flags(client, creds).await;
            Ok(flags.offered(preview).into_iter().map(|(id, name)| SourceModel {
                id: id.into(), name: name.into(), context_window: Some(1_048_576), ..Default::default()
            }).collect())
        }
        App::Antigravity => {
            let reply = antigravity_list(client, creds).await?;
            let mut order: HashMap<String, usize> = HashMap::new();
            for sort in reply["agentModelSorts"].as_array().into_iter().flatten() {
                for group in sort["groups"].as_array().into_iter().flatten() {
                    for id in group["modelIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                        let next = order.len();
                        order.entry(id.to_owned()).or_insert(next);
                    }
                }
            }
            let mut models: Vec<SourceModel> = reply["models"].as_object().into_iter().flatten()
                .filter(|(id, _)| !antigravity_hidden(id))
                .map(|(id, model)| SourceModel {
                    id: id.clone(),
                    name: model["displayName"].as_str().filter(|name| !name.is_empty()).unwrap_or(id).to_owned(),
                    context_window: model["maxTokens"].as_u64().filter(|value| *value > 0),
                    max_output_tokens: model["maxOutputTokens"].as_u64().filter(|value| *value > 0),
                    ..Default::default()
                }).collect();
            models.sort_by_key(|model| (order.get(&model.id).copied().unwrap_or(usize::MAX), model.id.clone()));
            if models.is_empty() {
                return Err("accountSources.errors.googleNoModels".into());
            }
            Ok(models)
        }
    }
}

/// How much is left of the account's tightest model allowance, in percent.
pub async fn quota(client: &Client, app: App, creds: &Creds) -> Quota {
    let checked_at = chrono::Utc::now().timestamp();
    let failed = |error: String| Quota { checked_at, error: Some(error), ..Default::default() };
    if creds.project.is_empty() {
        return failed(NEEDS_PROJECT_KEY.into());
    }
    let remaining: Vec<f64> = match app {
        App::GeminiCli => match call(client, app, creds, &base(app), "retrieveUserQuota", json!({"project": creds.project}), &[]).await {
            Ok(reply) => reply["buckets"].as_array().into_iter().flatten().filter_map(|bucket| bucket["remainingFraction"].as_f64()).collect(),
            Err(error) => return failed(error),
        },
        App::Antigravity => match antigravity_list(client, creds).await {
            Ok(reply) => reply["models"].as_object().into_iter().flatten()
                .filter(|(id, model)| !antigravity_hidden(id) && model["quotaInfo"].is_object())
                .map(|(_, model)| model["quotaInfo"]["remainingFraction"].as_f64().unwrap_or(0.0)).collect(),
            Err(error) => return failed(error),
        },
    };
    let Some(tightest) = remaining.into_iter().reduce(f64::min) else {
        return Quota { checked_at, unit: "percent".into(), ..Default::default() };
    };
    Quota { used: ((1.0 - tightest.clamp(0.0, 1.0)) * 100.0).round(), limit: 100.0, unit: "percent".into(),
        paid: Some(!creds.paid.is_empty()), checked_at, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_cli_offers_and_sends_models_as_its_experiments_say() {
        let plain = Flags::default();
        let ids: Vec<_> = plain.offered(false).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["gemini-3.5-flash", "gemini-3.1-flash-lite", "gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.5-flash-lite"]);
        assert_eq!(plain.wire("gemini-3.5-flash"), "gemini-3-flash");
        let rolled = Flags { gemini31: true, latest_flash: true, latest_flash_lite: true, no_pro: false };
        let ids: Vec<_> = rolled.offered(true).into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&"gemini-3.1-pro-preview") && ids.contains(&"gemini-3.8-flash") && !ids.contains(&"gemini-3-pro-preview"));
        assert_eq!(rolled.wire("gemini-3.5-flash"), "gemini-3.8-flash");
        assert_eq!(rolled.wire("gemini-3.1-flash-lite"), "gemini-3.5-flash-lite");
    }

    #[test]
    fn antigravity_keeps_chat_models_only() {
        assert!(antigravity_hidden("tab_flash_lite_preview") && antigravity_hidden("chat_99") && !antigravity_hidden("claude-sonnet-4-6"));
        assert_eq!(project_of(&json!({"id":"p-1","name":"x"})), "p-1");
        assert_eq!(project_of(&json!("p-2")), "p-2");
    }
}
