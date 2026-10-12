//! Kiro: a Kiro plan's models through Kiro's own API, the one kiro-cli talks
//! to. A request goes whole as a conversation — the earlier turns as its
//! history, the last as its current message, with the caller's tools — and
//! the reply comes back as an AWS event stream, read here into Chat events
//! (`wire`). The sign-in is the one the Kiro IDE runs: Kiro's page offers
//! Google, GitHub, AWS Builder ID and IAM Identity Center and sends the
//! browser back to a port on this machine. Each account signed in here is
//! AgentHarbor's own; kiro-cli's and the IDE's are left as they are. Ported
//! from magpie (MIT, see LICENSES/THIRD_PARTY.md).
use super::store::{Quota, SourceModel};
use super::wire::Event;
use base64::Engine as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::{net::TcpListener, sync::watch};

const PORTAL: &str = "https://app.kiro.dev";
const AUTH_SERVICE: &str = "https://prod.us-east-1.auth.desktop.kiro.dev";
/// The ports Kiro's sign-in page sends the browser back to, the IDE's.
const CALLBACK_PORTS: [u16; 10] = [3128, 4649, 6588, 8008, 9091, 49153, 50153, 51153, 52153, 53153];
/// What the IDE asks AWS for.
const SCOPES: [&str; 5] = ["codewhisperer:completions", "codewhisperer:analysis", "codewhisperer:conversations",
    "codewhisperer:transformations", "codewhisperer:taskassist"];
/// The profile a Builder ID sign-in, which has none of its own, is served under.
const BUILDER_ID_PROFILE: &str = "arn:aws:codewhisperer:us-east-1:638616132270:profile/AAAACCCCXXXX";
const DESKTOP_UA: &str = "Kiro-Desktop/0.2.13 (darwin; arm64)";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(600);
const MARGIN_SECONDS: i64 = 120;
pub const USAGE_URL: &str = "https://app.kiro.dev/account/usage";
/// What a turn with nothing to say says: Kiro takes no empty message.
const PROCEED: &str = "Please proceed with the task.";
/// What answers a tool call no result was sent for.
const NO_RESULT: &str = "Tool use was interrupted and did not produce a result.";
/// A tool's output is cut here, as Kiro's own agent cuts it.
const RESULT_LIMIT: usize = 250_000;

fn overridden() -> Option<String> {
    super::endpoint_override(super::SourceKind::Kiro)
}

fn portal() -> String {
    overridden().unwrap_or_else(|| PORTAL.into())
}

fn auth_service() -> String {
    overridden().unwrap_or_else(|| AUTH_SERVICE.into())
}

fn oidc(region: &str) -> String {
    overridden().unwrap_or_else(|| format!("https://oidc.{region}.amazonaws.com"))
}

fn social_refresh(region: &str) -> String {
    overridden().map(|base| format!("{base}/refreshToken"))
        .unwrap_or_else(|| format!("https://prod.{region}.auth.desktop.kiro.dev/refreshToken"))
}

/// Where the account's chat API is.
pub fn runtime(region: &str) -> String {
    overridden().unwrap_or_else(|| format!("https://runtime.{region}.kiro.dev"))
}

fn management(region: &str) -> String {
    overridden().unwrap_or_else(|| format!("https://management.{region}.kiro.dev"))
}

fn ide_ua() -> String {
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    format!("KiroIDE-1.1.70-{:x}", Sha256::digest(format!("agentharbor:{host}").as_bytes()))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub access: String,
    pub refresh: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// `social` (Google, GitHub) or `idc` (Builder ID, Identity Center).
    pub method: String,
    pub provider: String,
    /// Where the account signed in, for refreshing.
    pub region: String,
    pub profile: String,
    pub client_id: String,
    pub client_secret: String,
}

impl Creds {
    pub fn near_expiry(&self) -> bool {
        self.expires_at - chrono::Utc::now().timestamp() < MARGIN_SECONDS
    }

    /// Where the account's API is: its profile's region, else the nearer of
    /// Kiro's two to where it signed in.
    pub fn api_region(&self) -> String {
        if let Some(region) = self.profile.split(':').nth(3).filter(|region| !region.is_empty()) {
            return region.to_owned();
        }
        if self.region.starts_with("eu-") { "eu-central-1".into() } else { "us-east-1".into() }
    }
}

async fn post_json(client: &Client, url: &str, body: &Value, headers: &[(&str, &str)]) -> Result<Value, (u16, String)> {
    let mut request = client.post(url).header("accept", "application/json").json(body).timeout(Duration::from_secs(30));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.map_err(|_| (0, "accountSources.errors.kiroUnreachable".to_owned()))?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let message = value["error_description"].as_str().or(value["message"].as_str()).or(value["error"].as_str())
            .map(str::to_owned).unwrap_or_else(|| status.to_string());
        return Err((status.as_u16(), message));
    }
    Ok(value)
}

fn expiry(seconds: Option<i64>) -> i64 {
    chrono::Utc::now().timestamp() + seconds.filter(|seconds| *seconds > 0).unwrap_or(3600)
}

// ---- signing in -----------------------------------------------------------------

pub struct SignIn {
    pub url: String,
    listener: TcpListener,
    port: u16,
    redirect: String,
    state: String,
    verifier: String,
}

fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Listen on one of the ports Kiro's page sends browsers back to, and build that page.
pub async fn begin_sign_in() -> Result<SignIn, String> {
    let ports: Vec<u16> = if overridden().is_some() { vec![0] } else { CALLBACK_PORTS.to_vec() };
    let mut bound = None;
    for port in ports {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            bound = Some(listener);
            break;
        }
    }
    let listener = bound.ok_or("accountSources.errors.kiroPortsBusy")?;
    let port = listener.local_addr().map_err(|_| "accounts.loginFailed")?.port();
    let redirect = format!("http://localhost:{port}");
    let (state, verifier) = (super::chatgpt_api::random_token(24), super::chatgpt_api::random_token(48));
    let mut url = reqwest::Url::parse(&format!("{}/signin", portal())).map_err(|_| "accounts.loginFailed")?;
    url.query_pairs_mut()
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("redirect_uri", &redirect)
        .append_pair("redirect_from", "KiroIDE");
    Ok(SignIn { url: url.to_string(), listener, port, redirect, state, verifier })
}

pub struct SignedIn {
    pub creds: Creds,
    pub email: String,
    pub plan: Option<String>,
}

/// An AWS sign-in Kiro's page sent the browser on to.
struct AwsStep {
    region: String,
    client_id: String,
    client_secret: String,
    provider: String,
    state: String,
    verifier: String,
}

/// Wait for the browser to come back from Kiro's page (Google or GitHub),
/// or from AWS's after Kiro's page sent it there; trade the code and read
/// who the account is.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, mut cancel: watch::Receiver<bool>) -> Result<SignedIn, String> {
    let aws_redirect = format!("http://127.0.0.1:{}/oauth/callback", sign_in.port);
    let flow = async {
        let mut aws: Option<AwsStep> = None;
        loop {
            let (mut socket, _) = sign_in.listener.accept().await.map_err(|_| "accounts.loginFailed".to_owned())?;
            let Some((path, query)) = super::loopback::read_request(&mut socket).await else {
                super::loopback::not_found(&mut socket).await;
                continue;
            };
            if path != "/oauth/callback" && path != "/signin/callback" {
                super::loopback::not_found(&mut socket).await;
                continue;
            }
            if let Some(error) = query.get("error") {
                let detail = query.get("error_description").unwrap_or(error).clone();
                super::loopback::reply(&mut socket, false, &detail).await;
                return Err(if error == "access_denied" { "accounts.loginCancelled".to_owned() } else { format!("Kiro: {detail}") });
            }
            let state = query.get("state").map(String::as_str).unwrap_or_default();
            let code = query.get("code").cloned().unwrap_or_default();
            let creds = if let Some(step) = aws.as_ref().filter(|step| step.state == state) {
                // Back from AWS.
                let reply = post_json(client, &format!("{}/token", oidc(&step.region)), &json!({
                    "clientId": step.client_id, "clientSecret": step.client_secret, "grantType": "authorization_code",
                    "redirectUri": aws_redirect, "code": code, "codeVerifier": step.verifier,
                }), &[]).await.map_err(|(_, message)| format!("AWS: {message}"));
                match reply {
                    Ok(reply) if reply["accessToken"].as_str().is_some_and(|token| !token.is_empty()) => Creds {
                        access: reply["accessToken"].as_str().unwrap_or_default().into(),
                        refresh: reply["refreshToken"].as_str().unwrap_or_default().into(),
                        expires_at: expiry(reply["expiresIn"].as_i64()), method: "idc".into(), provider: step.provider.clone(),
                        region: step.region.clone(), client_id: step.client_id.clone(), client_secret: step.client_secret.clone(),
                        ..Default::default()
                    },
                    Ok(_) | Err(_) => {
                        super::loopback::reply(&mut socket, false, "AWS sent back no token.").await;
                        return Err("accountSources.errors.kiroNoToken".into());
                    }
                }
            } else if state != sign_in.state {
                super::loopback::reply(&mut socket, false, "This sign-in was not started here.").await;
                continue;
            } else {
                match query.get("login_option").map(String::as_str).unwrap_or_default() {
                    option @ ("google" | "github") => {
                        let reply = post_json(client, &format!("{}/oauth/token", auth_service()), &json!({
                            "code": code, "code_verifier": sign_in.verifier,
                            "redirect_uri": format!("{}{path}?login_option={option}", sign_in.redirect),
                        }), &[("user-agent", &ide_ua())]).await;
                        match reply {
                            Ok(reply) if reply["accessToken"].as_str().is_some_and(|token| !token.is_empty()) => Creds {
                                access: reply["accessToken"].as_str().unwrap_or_default().into(),
                                refresh: reply["refreshToken"].as_str().unwrap_or_default().into(),
                                expires_at: expiry(reply["expiresIn"].as_i64()), method: "social".into(),
                                provider: if option == "google" { "Google".into() } else { "Github".into() },
                                region: "us-east-1".into(), profile: reply["profileArn"].as_str().unwrap_or_default().into(),
                                ..Default::default()
                            },
                            _ => {
                                super::loopback::reply(&mut socket, false, "Kiro sent back no token.").await;
                                return Err("accountSources.errors.kiroNoToken".into());
                            }
                        }
                    }
                    option @ ("builderid" | "awsidc" | "internal") => {
                        // On to AWS, with a client registered for this sign-in.
                        let (issuer, region) = (query.get("issuer_url").cloned().unwrap_or_default(), query.get("idc_region").cloned().unwrap_or_default());
                        if issuer.is_empty() || region.is_empty() {
                            super::loopback::reply(&mut socket, false, "Kiro's page did not say where to sign in at AWS.").await;
                            return Err("accounts.loginFailed".into());
                        }
                        let registered = post_json(client, &format!("{}/client/register", oidc(&region)), &json!({
                            "clientName": "Kiro IDE", "clientType": "public", "scopes": SCOPES,
                            "grantTypes": ["authorization_code", "refresh_token"],
                            "redirectUris": ["http://127.0.0.1/oauth/callback"], "issuerUrl": issuer,
                        }), &[("user-agent", &ide_ua())]).await.map_err(|(_, message)| format!("AWS: {message}"))?;
                        let step = AwsStep {
                            client_id: registered["clientId"].as_str().unwrap_or_default().into(),
                            client_secret: registered["clientSecret"].as_str().unwrap_or_default().into(),
                            provider: match option { "builderid" => "BuilderId", "awsidc" => "Enterprise", _ => "Internal" }.into(),
                            state: super::chatgpt_api::random_token(24),
                            verifier: super::chatgpt_api::random_token(48),
                            region,
                        };
                        let mut authorize = reqwest::Url::parse(&format!("{}/authorize", oidc(&step.region))).map_err(|_| "accounts.loginFailed")?;
                        authorize.query_pairs_mut()
                            .append_pair("response_type", "code")
                            .append_pair("client_id", &step.client_id)
                            .append_pair("redirect_uri", &aws_redirect)
                            .append_pair("scopes", &SCOPES.join(","))
                            .append_pair("state", &step.state)
                            .append_pair("code_challenge", &challenge(&step.verifier))
                            .append_pair("code_challenge_method", "S256");
                        super::loopback::redirect(&mut socket, authorize.as_str()).await;
                        aws = Some(step);
                        continue;
                    }
                    other => {
                        let detail = if other == "external_idp" {
                            "A company's own identity provider cannot sign in here yet.".to_owned()
                        } else {
                            format!("Kiro's page came back with a sign-in AgentHarbor does not know ({other}).")
                        };
                        super::loopback::reply(&mut socket, false, &detail).await;
                        return Err("accountSources.errors.kiroUnsupportedSignIn".into());
                    }
                }
            };
            let result = complete(client, creds).await;
            match &result {
                Ok(done) => super::loopback::reply(&mut socket, true, &format!("{} is added to AgentHarbor. You can close this tab.", done.email)).await,
                Err(error) => super::loopback::reply(&mut socket, false, error).await,
            }
            return result;
        }
    };
    tokio::select! {
        biased;
        _ = super::cancelled(&mut cancel) => Err("accounts.loginCancelled".into()),
        result = tokio::time::timeout(SIGN_IN_WINDOW, flow) => result.unwrap_or_else(|_| Err("accounts.loginTimeout".into())),
    }
}

/// A fresh sign-in's profile, who it is and its plan.
async fn complete(client: &Client, mut creds: Creds) -> Result<SignedIn, String> {
    if creds.profile.is_empty() {
        creds.profile = profile(client, &creds).await?;
    }
    let limits = usage_limits(client, &creds, true).await.ok();
    let email = limits.as_ref().and_then(|limits| limits["userInfo"]["email"].as_str()).filter(|email| !email.is_empty())
        .map(str::to_owned).unwrap_or_else(|| format!("Kiro ({})", creds.provider));
    let plan = limits.as_ref().and_then(plan_of);
    Ok(SignedIn { creds, email, plan })
}

/// The profile a sign-in that names none uses: Builder ID's, or the first
/// Kiro lists in either of its regions.
async fn profile(client: &Client, creds: &Creds) -> Result<String, String> {
    if creds.provider.eq_ignore_ascii_case("BuilderId") {
        return Ok(BUILDER_ID_PROFILE.into());
    }
    let mut last = "accountSources.errors.kiroNoProfile".to_owned();
    for region in ["us-east-1", "eu-central-1"] {
        match post_json(client, &format!("{}/List-Available-Profiles", management(region)), &json!({}),
            &[("authorization", &format!("Bearer {}", creds.access))]).await
        {
            Ok(reply) => {
                if let Some(arn) = reply["profiles"].as_array().into_iter().flatten().find_map(|profile| profile["arn"].as_str()).filter(|arn| !arn.is_empty()) {
                    return Ok(arn.to_owned());
                }
            }
            Err((403, message)) if creds.method == "idc" && message.contains("Builder ID") => return Ok(BUILDER_ID_PROFILE.into()),
            Err((_, message)) => last = message,
        }
    }
    Err(last)
}

pub enum RefreshError {
    SignedOut(String),
    Transient,
}

/// A fresh token, by the sign-in's own way: Kiro's auth service for Google
/// and GitHub, AWS's for Builder ID and Identity Center. The refresh token
/// may be replaced.
pub async fn refresh(client: &Client, creds: &Creds) -> Result<Creds, RefreshError> {
    if creds.refresh.is_empty() {
        return Err(RefreshError::SignedOut("no refresh token".into()));
    }
    let reply = if creds.method == "idc" {
        post_json(client, &format!("{}/token", oidc(&creds.region)), &json!({
            "clientId": creds.client_id, "clientSecret": creds.client_secret, "refreshToken": creds.refresh, "grantType": "refresh_token",
        }), &[]).await
    } else {
        post_json(client, &social_refresh(&creds.region), &json!({"refreshToken": creds.refresh}), &[("user-agent", DESKTOP_UA)]).await
    };
    let reply = reply.map_err(|(status, message)| if matches!(status, 400 | 401 | 403) { RefreshError::SignedOut(message) } else { RefreshError::Transient })?;
    let access = reply["accessToken"].as_str().unwrap_or_default();
    if access.is_empty() {
        return Err(RefreshError::Transient);
    }
    let mut next = creds.clone();
    next.access = access.to_owned();
    next.expires_at = expiry(reply["expiresIn"].as_i64());
    if let Some(refresh) = reply["refreshToken"].as_str().filter(|token| !token.is_empty()) {
        next.refresh = refresh.to_owned();
    }
    if let Some(profile) = reply["profileArn"].as_str().filter(|profile| !profile.is_empty()) {
        next.profile = profile.to_owned();
    }
    Ok(next)
}

async fn management_get(client: &Client, creds: &Creds, method: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let response = client.get(format!("{}/{method}", management(&creds.api_region()))).query(query)
        .bearer_auth(&creds.access).header("accept", "application/json").timeout(Duration::from_secs(30))
        .send().await.map_err(|_| "accountSources.errors.kiroUnreachable".to_owned())?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(value["message"].as_str().map(|message| format!("Kiro: {message}")).unwrap_or_else(|| format!("Kiro: {status}")));
    }
    Ok(value)
}

async fn usage_limits(client: &Client, creds: &Creds, email: bool) -> Result<Value, String> {
    management_get(client, creds, "Get-Usage-Limits", &[("origin", "KIRO_CLI"), ("profileArn", &creds.profile),
        ("resourceType", "CREDIT"), ("isEmailRequired", if email { "true" } else { "false" })]).await
}

/// "KIRO PRO+" said the way plans are: "Kiro Pro+".
fn plan_of(limits: &Value) -> Option<String> {
    let title = limits["subscriptionInfo"]["subscriptionTitle"].as_str()?.to_ascii_lowercase();
    let words: Vec<String> = title.split_whitespace().map(|word| if word == "kiro" { "Kiro".to_owned() } else {
        let mut chars = word.chars();
        chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
    }).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The account's models, Kiro's default first.
pub async fn fetch_models(client: &Client, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let reply = management_get(client, creds, "List-Available-Models", &[("origin", "KIRO_CLI"), ("profileArn", &creds.profile)]).await?;
    let default = reply["defaultModel"]["modelId"].as_str().unwrap_or_default();
    let mut models: Vec<SourceModel> = Vec::new();
    for model in reply["models"].as_array().into_iter().flatten() {
        let id = model["modelId"].as_str().unwrap_or_default();
        if id.is_empty() || models.iter().any(|existing| existing.id == id) {
            continue;
        }
        let name = model["modelName"].as_str().filter(|name| !name.is_empty() && *name != id)
            .map(str::to_owned).unwrap_or_else(|| if id == "auto" { "Auto".into() } else { id.to_owned() });
        let entry = SourceModel {
            id: id.to_owned(), name,
            context_window: model["tokenLimits"]["maxInputTokens"].as_u64().filter(|value| *value > 0),
            max_output_tokens: model["tokenLimits"]["maxOutputTokens"].as_u64().filter(|value| *value > 0),
            reasoning: if id.contains("claude") || id == "auto" { vec!["low".into(), "medium".into(), "high".into()] } else { Vec::new() },
            ..Default::default()
        };
        if id == default { models.insert(0, entry) } else { models.push(entry) }
    }
    if models.is_empty() {
        return Err("accountSources.errors.kiroNoModels".into());
    }
    Ok(models)
}

/// The month's credits (or a free trial's while it runs).
pub async fn quota(client: &Client, creds: &Creds) -> (Quota, Option<String>) {
    let checked_at = chrono::Utc::now().timestamp();
    match usage_limits(client, creds, false).await {
        Ok(limits) => {
            let plan = plan_of(&limits);
            let mut used = 0.0;
            let mut limit = 0.0;
            for usage in limits["usageBreakdownList"].as_array().into_iter().flatten() {
                let trial = &usage["freeTrialInfo"];
                if trial["freeTrialStatus"].as_str().is_some_and(|status| status.eq_ignore_ascii_case("ACTIVE")) {
                    used += trial["currentUsageWithPrecision"].as_f64().unwrap_or_default();
                    limit += trial["usageLimitWithPrecision"].as_f64().unwrap_or_default();
                }
                used += usage["currentUsageWithPrecision"].as_f64().unwrap_or_default();
                limit += usage["usageLimitWithPrecision"].as_f64().unwrap_or_default();
            }
            let paid = plan.as_deref().map(|plan| !plan.to_ascii_lowercase().contains("free"));
            (Quota { used, limit, unit: "credits".into(), paid, checked_at, error: None }, plan)
        }
        Err(error) => (Quota { checked_at, error: Some(error), ..Default::default() }, None),
    }
}

/// The headers that sign a chat request.
pub fn request_headers(creds: &Creds) -> Vec<(&'static str, String)> {
    let invocation = uuid::Uuid::new_v4().to_string();
    let ua = format!("aws-sdk-rust/1.0.0 ua/2.1 os/other lang/rust api/codewhispererstreaming#1.28.3 m/E app/AmazonQ-For-CLI md/appVersion-1.28.3-{}",
        uuid::Uuid::new_v4().simple());
    vec![
        ("authorization", format!("Bearer {}", creds.access)),
        ("accept", "application/vnd.amazon.eventstream".into()),
        ("x-amzn-codewhisperer-optout", "true".into()),
        ("amz-sdk-invocation-id", invocation),
        ("amz-sdk-request", "attempt=1; max=1".into()),
        ("x-amzn-kiro-agent-mode", "vibe".into()),
        ("x-amz-user-agent", ua.clone()),
        ("user-agent", ua),
    ]
}

// ---- the request ------------------------------------------------------------------

/// A tool call's id as Kiro takes one; another vendor's becomes Kiro's own,
/// the same each time.
fn tool_id(id: &str) -> String {
    if !id.is_empty() && id.len() <= 64 && id.chars().all(|ch| ch.is_ascii_alphanumeric() || "_.:-".contains(ch)) {
        return id.to_owned();
    }
    let sum = Sha256::digest(id.as_bytes());
    format!("t_{}", &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sum)[..32])
}

/// A tool's name as Kiro takes one: a longer one (an MCP server's) is cut
/// and told apart by a hash; `names` maps it back.
fn tool_name(name: &str) -> String {
    if !name.is_empty() && name.len() <= 64 && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
        return name.to_owned();
    }
    let short: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' }).take(55).collect();
    let sum = Sha256::digest(name.as_bytes());
    format!("{short}_{:02x}{:02x}{:02x}{:02x}", sum[0], sum[1], sum[2], sum[3])
}

/// Whether the model is asked to think aloud: a Claude model (or Kiro's
/// Auto, which picks one) with a reasoning effort above low.
fn thinks(chat: &Value, model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    chat["reasoning_effort"].as_str().is_some_and(|effort| !matches!(effort, "low" | "none" | "minimal"))
        && (model.contains("claude") || model == "auto")
}

fn thinking_budget(effort: &str) -> u32 {
    match effort {
        "low" => 10_000,
        "high" => 30_000,
        "xhigh" | "max" => 50_000,
        _ => 20_000,
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().filter_map(|item| item["text"].as_str()).collect::<Vec<_>>().join("\n\n"),
        _ => String::new(),
    }
}

fn images_of(content: &Value) -> Vec<Value> {
    content.as_array().into_iter().flatten().filter_map(|item| {
        let url = item["image_url"]["url"].as_str()?;
        let (mime, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
        let format = match mime.trim_start_matches("image/") { "jpg" => "jpeg", "" => "png", other => other };
        Some(json!({"format": format, "source": {"bytes": data}}))
    }).collect()
}

#[derive(Default)]
struct User {
    content: String,
    images: Vec<Value>,
    results: Vec<Value>,
}

#[derive(Default)]
struct Assistant {
    content: String,
    calls: Vec<Value>,
}

enum Entry {
    User(User),
    Assistant(Assistant),
}

fn join(left: &mut String, right: &str) {
    if right.is_empty() {
        return;
    }
    if !left.is_empty() {
        left.push_str("\n\n");
    }
    left.push_str(right);
}

/// The generateAssistantResponse body for a Chat request, and the caller's
/// name for each tool name Kiro is sent cut.
pub(crate) fn kiro_request(chat: &Value, model: &str, profile: &str) -> (Value, HashMap<String, String>, bool) {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut note = |name: &str| {
        let short = tool_name(name);
        if short != name {
            names.insert(short.clone(), name.to_owned());
        }
        short
    };
    let mut system = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    for message in chat["messages"].as_array().into_iter().flatten() {
        match message["role"].as_str().unwrap_or("user") {
            "system" | "developer" => {
                let text = text_of(&message["content"]);
                if !text.is_empty() {
                    system.push(text);
                }
            }
            "assistant" => {
                let mut turn = Assistant { content: text_of(&message["content"]), calls: Vec::new() };
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let input: Value = call["function"]["arguments"].as_str().and_then(|text| serde_json::from_str(text).ok())
                        .filter(Value::is_object).unwrap_or_else(|| json!({}));
                    turn.calls.push(json!({"name": note(call["function"]["name"].as_str().unwrap_or_default()),
                        "toolUseId": tool_id(call["id"].as_str().unwrap_or_default()), "input": input}));
                }
                if turn.content.is_empty() && turn.calls.is_empty() {
                    continue;
                }
                match entries.last_mut() {
                    Some(Entry::Assistant(previous)) => {
                        join(&mut previous.content, &turn.content);
                        previous.calls.extend(turn.calls);
                    }
                    _ => entries.push(Entry::Assistant(turn)),
                }
            }
            role => {
                let mut turn = User::default();
                if role == "tool" {
                    let mut output = text_of(&message["content"]);
                    if output.len() > RESULT_LIMIT {
                        let mut cut = RESULT_LIMIT;
                        while !output.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        output.truncate(cut);
                        output.push_str("\n… (cut)");
                    }
                    if output.is_empty() {
                        output = "(no output)".into();
                    }
                    turn.results.push(json!({"toolUseId": tool_id(message["tool_call_id"].as_str().unwrap_or_default()),
                        "status": "success", "content": [{"text": output}]}));
                } else {
                    turn.content = text_of(&message["content"]);
                    turn.images = images_of(&message["content"]);
                }
                match entries.last_mut() {
                    Some(Entry::User(previous)) => {
                        join(&mut previous.content, &turn.content);
                        previous.images.extend(turn.images);
                        previous.results.extend(turn.results);
                    }
                    _ => entries.push(Entry::User(turn)),
                }
            }
        }
    }
    // The conversation opens with something the user said …
    while let Some(first) = entries.first_mut() {
        match first {
            Entry::User(user) if user.content.is_empty() && user.images.is_empty() => {
                entries.remove(0);
            }
            Entry::User(user) => {
                user.results.clear();
                break;
            }
            Entry::Assistant(_) => {
                entries.remove(0);
            }
        }
    }
    // … and ends with it, the message being answered.
    if !matches!(entries.last(), Some(Entry::User(_))) {
        entries.push(Entry::User(User::default()));
    }
    // Each call is answered in the message after it, and only calls are.
    for index in 0..entries.len() {
        let calls: Vec<String> = match index.checked_sub(1).and_then(|previous| entries.get(previous)) {
            Some(Entry::Assistant(turn)) => turn.calls.iter().filter_map(|call| call["toolUseId"].as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        };
        if let Entry::User(user) = &mut entries[index] {
            let mut answered: Vec<String> = Vec::new();
            let mut kept = Vec::new();
            for result in std::mem::take(&mut user.results) {
                let id = result["toolUseId"].as_str().unwrap_or_default().to_owned();
                if calls.contains(&id) && !answered.contains(&id) {
                    answered.push(id);
                    kept.push(result);
                }
            }
            for id in &calls {
                if !answered.contains(id) {
                    kept.push(json!({"toolUseId": id, "status": "error", "content": [{"text": NO_RESULT}]}));
                }
            }
            user.results = kept;
            if user.content.is_empty() && user.results.is_empty() {
                user.content = PROCEED.into();
            }
        }
    }
    // Only the latest images go again, as Kiro's own agent does.
    let latest = entries.iter().rposition(|entry| matches!(entry, Entry::User(user) if !user.images.is_empty()));
    for (index, entry) in entries.iter_mut().enumerate() {
        if let Entry::User(user) = entry {
            if Some(index) != latest {
                user.images.clear();
            }
        }
    }
    let thinking = thinks(chat, model);
    let mut instructions = system.join("\n\n");
    if thinking {
        let budget = thinking_budget(chat["reasoning_effort"].as_str().unwrap_or("medium"));
        let tag = format!("<thinking_mode>enabled</thinking_mode><max_thinking_length>{budget}</max_thinking_length>");
        instructions = if instructions.is_empty() { tag } else { format!("{tag}\n{instructions}") };
    }
    if let (false, Some(Entry::User(first))) = (instructions.is_empty(), entries.first_mut()) {
        first.content = format!("{instructions}\n\n{}", first.content);
    }
    // The caller's tools, and any the conversation used that it no longer
    // offers: Kiro refuses a history naming a tool it was not given.
    let mut tools: Vec<Value> = Vec::new();
    let mut offered: Vec<String> = Vec::new();
    for tool in chat["tools"].as_array().into_iter().flatten() {
        let function = &tool["function"];
        let original = function["name"].as_str().unwrap_or_default();
        let name = note(original);
        let schema = if function["parameters"].is_object() { function["parameters"].clone() } else { json!({"type": "object", "properties": {}}) };
        let description = function["description"].as_str().filter(|text| !text.is_empty()).unwrap_or(original);
        tools.push(json!({"toolSpecification": {"name": name, "description": description, "inputSchema": {"json": schema}}}));
        offered.push(name);
    }
    for entry in &entries {
        if let Entry::Assistant(turn) = entry {
            for call in &turn.calls {
                let name = call["name"].as_str().unwrap_or_default().to_owned();
                if !offered.contains(&name) {
                    tools.push(json!({"toolSpecification": {"name": name, "description": "Tool", "inputSchema": {"json": {"type": "object", "properties": {}}}}}));
                    offered.push(name);
                }
            }
        }
    }
    let user_message = |user: &User, tools: Option<&Vec<Value>>| {
        let mut message = json!({"content": user.content, "modelId": model, "origin": "KIRO_CLI"});
        if !user.images.is_empty() {
            message["images"] = json!(user.images);
        }
        let mut context = serde_json::Map::new();
        if !user.results.is_empty() {
            context.insert("toolResults".into(), json!(user.results));
        }
        if let Some(tools) = tools.filter(|tools| !tools.is_empty()) {
            context.insert("tools".into(), json!(tools));
        }
        if !context.is_empty() {
            message["userInputMessageContext"] = Value::Object(context);
        }
        message
    };
    let last = entries.len() - 1;
    let history: Vec<Value> = entries[..last].iter().map(|entry| match entry {
        Entry::User(user) => json!({"userInputMessage": user_message(user, None)}),
        Entry::Assistant(turn) => {
            let mut message = json!({"content": turn.content});
            if !turn.calls.is_empty() {
                message["toolUses"] = json!(turn.calls);
            }
            json!({"assistantResponseMessage": message})
        }
    }).collect();
    let Entry::User(current) = &entries[last] else { unreachable!("the conversation ends with the user") };
    let mut state = json!({"chatTriggerType": "MANUAL", "agentTaskType": "vibe", "conversationId": uuid::Uuid::new_v4().to_string(),
        "currentMessage": {"userInputMessage": user_message(current, Some(&tools))}});
    if !history.is_empty() {
        state["history"] = json!(history);
    }
    let mut body = json!({"conversationState": state, "agentMode": "vibe"});
    if !profile.is_empty() {
        body["profileArn"] = json!(profile);
    }
    (body, names, thinking)
}

// ---- the reply --------------------------------------------------------------------

/// One message of an AWS event stream: its string headers and payload.
struct Frame {
    headers: HashMap<String, String>,
    payload: Vec<u8>,
}

/// A whole frame at the head of `buffer`, taken off it; `Err` for a stream
/// that cannot be one.
fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<Frame>, String> {
    if buffer.len() < 12 {
        return Ok(None);
    }
    let total = u32::from_be_bytes(buffer[0..4].try_into().expect("four bytes")) as usize;
    let header_length = u32::from_be_bytes(buffer[4..8].try_into().expect("four bytes")) as usize;
    if total < 16 || total > 16 << 20 || header_length > total - 16 {
        return Err("a malformed event stream".into());
    }
    if buffer.len() < total {
        return Ok(None);
    }
    let message: Vec<u8> = buffer.drain(..total).collect();
    let mut headers = HashMap::new();
    let mut raw = &message[12..12 + header_length];
    while !raw.is_empty() {
        let name_length = raw[0] as usize;
        if raw.len() < 2 + name_length {
            break;
        }
        let name = String::from_utf8_lossy(&raw[1..1 + name_length]).into_owned();
        let kind = raw[1 + name_length];
        raw = &raw[2 + name_length..];
        let size = match kind {
            0 | 1 => 0,
            2 => 1,
            3 => 2,
            4 => 4,
            5 | 8 => 8,
            9 => 16,
            6 | 7 => {
                if raw.len() < 2 {
                    break;
                }
                let length = u16::from_be_bytes([raw[0], raw[1]]) as usize;
                if raw.len() < 2 + length {
                    break;
                }
                if kind == 7 {
                    headers.insert(name, String::from_utf8_lossy(&raw[2..2 + length]).into_owned());
                }
                raw = &raw[2 + length..];
                continue;
            }
            _ => break,
        };
        if raw.len() < size {
            break;
        }
        raw = &raw[size..];
    }
    Ok(Some(Frame { headers, payload: message[12 + header_length..total - 4].to_vec() }))
}

/// Splits a reply that opens with `<thinking>…</thinking>` into its
/// thinking and the text after, however the tags fall across pieces.
#[derive(Default)]
struct ThinkSplitter {
    /// 0 not known yet, 1 thinking, 2 text.
    state: u8,
    held: String,
    said: bool,
}

const THINK_OPEN: &str = "<thinking>";
const THINK_CLOSE: &str = "</thinking>";

impl ThinkSplitter {
    fn feed(&mut self, text: &str, out: &mut Vec<Event>) {
        self.held.push_str(text);
        loop {
            match self.state {
                0 => {
                    let trimmed = self.held.trim_start().to_owned();
                    if let Some(rest) = trimmed.strip_prefix(THINK_OPEN) {
                        self.held = rest.to_owned();
                        self.state = 1;
                        continue;
                    }
                    if trimmed.len() < THINK_OPEN.len() && THINK_OPEN.starts_with(&trimmed) {
                        return;
                    }
                    self.state = 2;
                }
                1 => {
                    if !self.said {
                        self.held = self.held.trim_start_matches(['\r', '\n']).to_owned();
                        if self.held.is_empty() {
                            return;
                        }
                    }
                    if let Some(at) = self.held.find(THINK_CLOSE) {
                        if at > 0 {
                            out.push(Event::Reasoning(self.held[..at].to_owned()));
                        }
                        self.held = self.held[at + THINK_CLOSE.len()..].trim_start_matches(['\r', '\n']).to_owned();
                        self.state = 2;
                        continue;
                    }
                    let keep = (1..THINK_CLOSE.len().min(self.held.len() + 1)).rev()
                        .find(|length| self.held.ends_with(&THINK_CLOSE[..*length])).unwrap_or(0);
                    let cut = self.held.len() - keep;
                    if cut > 0 && self.held.is_char_boundary(cut) {
                        out.push(Event::Reasoning(self.held[..cut].to_owned()));
                        self.held = self.held[cut..].to_owned();
                        self.said = true;
                    }
                    return;
                }
                _ => {
                    if !self.held.is_empty() {
                        out.push(Event::Text(std::mem::take(&mut self.held)));
                    }
                    return;
                }
            }
        }
    }

    fn end(&mut self, out: &mut Vec<Event>) {
        if self.held.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.held);
        out.push(if self.state == 1 { Event::Reasoning(text) } else { Event::Text(text) });
        if self.state == 0 {
            self.state = 2;
        }
    }
}

/// Reads Kiro's event stream into events.
pub struct Decoder {
    buffer: Vec<u8>,
    thinking: bool,
    think: ThinkSplitter,
    names: HashMap<String, String>,
    tool: String,
    tools: usize,
    stop: String,
    input: u64,
    output: u64,
    cached: u64,
    context_percent: f64,
    window: u64,
    said: usize,
    failed: Option<String>,
}

impl Decoder {
    pub fn new(names: HashMap<String, String>, thinking: bool, window: Option<u64>) -> Self {
        Decoder { buffer: Vec::new(), thinking, think: ThinkSplitter::default(), names, tool: String::new(), tools: 0,
            stop: String::new(), input: 0, output: 0, cached: 0, context_percent: 0.0, window: window.unwrap_or_default(), said: 0, failed: None }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        if self.failed.is_some() {
            return;
        }
        self.buffer.extend_from_slice(bytes);
        loop {
            match take_frame(&mut self.buffer) {
                Ok(Some(frame)) => self.frame(frame, out),
                Ok(None) => return,
                Err(error) => {
                    self.failed = Some(error);
                    return;
                }
            }
            if self.failed.is_some() {
                return;
            }
        }
    }

    fn frame(&mut self, frame: Frame, out: &mut Vec<Event>) {
        let message_type = frame.headers.get(":message-type").map(String::as_str).unwrap_or_default();
        let mut kind = frame.headers.get(":event-type").cloned().unwrap_or_default();
        if message_type == "exception" || message_type == "error" {
            kind = frame.headers.get(":exception-type").or(frame.headers.get(":error-code")).cloned().unwrap_or_default();
            self.failed = Some(exception(&kind, &frame.payload));
            return;
        }
        let Ok(payload) = serde_json::from_slice::<Value>(&frame.payload) else { return };
        match kind.as_str() {
            "assistantResponseEvent" => {
                let text = payload["content"].as_str().unwrap_or_default();
                self.said += text.len();
                if self.thinking {
                    self.think.feed(text, out);
                } else if !text.is_empty() {
                    out.push(Event::Text(text.to_owned()));
                }
            }
            "reasoningContentEvent" => {
                if let Some(text) = payload["text"].as_str().filter(|text| !text.is_empty()) {
                    self.said += text.len();
                    out.push(Event::Reasoning(text.to_owned()));
                }
            }
            "toolUseEvent" => {
                let id = payload["toolUseId"].as_str().unwrap_or_default();
                let name = payload["name"].as_str().unwrap_or_default();
                let name = self.names.get(name).cloned().unwrap_or_else(|| name.to_owned());
                if !id.is_empty() && id != self.tool {
                    // The text before a call is all said.
                    self.think.end(out);
                    self.tool = id.to_owned();
                    self.tools += 1;
                    out.push(Event::ToolStart { id: id.to_owned(), name });
                }
                let input = match &payload["input"] {
                    Value::String(text) => text.clone(),
                    Value::Null => String::new(),
                    other if other == &json!({}) => String::new(),
                    other => other.to_string(),
                };
                if !input.is_empty() {
                    self.said += input.len();
                    out.push(Event::ToolArgs { id: self.tool.clone(), text: input });
                }
            }
            "metadataEvent" | "messageMetadataEvent" => {
                if let Some(stop) = payload["stopReason"].as_str() {
                    self.stop = stop.to_owned();
                }
                let usage = &payload["tokenUsage"];
                let number = |key: &str| usage[key].as_u64().unwrap_or_default();
                let input = if number("uncachedInputTokens") > 0 { number("uncachedInputTokens") } else { number("inputTokens") };
                self.input += input;
                self.output += number("outputTokens");
                self.cached += number("cacheReadInputTokens");
                if let Some(percent) = usage["contextUsagePercentage"].as_f64().filter(|percent| *percent > 0.0) {
                    self.context_percent = percent;
                }
            }
            "contextUsageEvent" => {
                if let Some(percent) = payload["contextUsagePercentage"].as_f64().filter(|percent| *percent > 0.0) {
                    self.context_percent = percent;
                }
            }
            "error" | "throttlingError" | "validationError" | "serviceUnavailableError" | "internalServerException" => {
                self.failed = Some(exception(&kind, &frame.payload));
            }
            _ => {}
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Event>) {
        self.think.end(out);
        if let Some(error) = self.failed.take() {
            out.push(Event::Error(error));
            return;
        }
        let mut input = self.input;
        if input == 0 && self.context_percent > 0.0 && self.window > 0 {
            // Kiro says how full the context is rather than how many tokens.
            input = (self.context_percent / 100.0 * self.window as f64) as u64;
        }
        let output = if self.output > 0 { self.output } else { (self.said as u64).div_ceil(4) };
        let stop = if self.tools > 0 {
            "tool_calls"
        } else if self.stop.eq_ignore_ascii_case("MAX_TOKENS") {
            "length"
        } else if self.stop.eq_ignore_ascii_case("CONTENT_FILTERED") {
            "content_filter"
        } else {
            "stop"
        };
        out.push(Event::Finish(stop));
        out.push(Event::Usage { input, output, cached: self.cached, reasoning: 0 });
    }
}

fn exception(kind: &str, payload: &[u8]) -> String {
    let value: Value = serde_json::from_slice(payload).unwrap_or(Value::Null);
    let mut message = value["message"].as_str().or(value["Message"].as_str()).map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(payload).trim().to_owned());
    if let Some(reason) = value["reason"].as_str() {
        message = format!("{message} ({reason})");
    }
    if kind.to_ascii_lowercase().contains("throttl") {
        format!("rate limited: {message}")
    } else if kind.is_empty() {
        message
    } else {
        format!("{kind}: {message}")
    }
}

/// What Kiro's own words leave unclear.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    if text.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD") || text.to_ascii_lowercase().contains("input is too long") {
        return Some("the conversation no longer fits Kiro's context; compact it or start a new one".into());
    }
    if text.contains("MONTHLY_REQUEST_COUNT") || text.contains("USAGE_LIMIT") {
        return Some(format!("this Kiro account's credits for the month are used up; see {USAGE_URL}"));
    }
    if text.contains("INSUFFICIENT_MODEL_CAPACITY") {
        return Some("Kiro has no capacity for this model right now; try again shortly or pick another".into());
    }
    match status {
        401 | 403 => Some("Kiro refused the account's token; the next request refreshes it, or sign in again on the Accounts page".into()),
        429 => Some(format!("Kiro's rate limit or credits are reached; see {USAGE_URL}")),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One AWS event-stream message with string headers, as Kiro sends it.
    pub(crate) fn frame(event: &str, payload: &Value) -> Vec<u8> {
        let mut headers = Vec::new();
        for (name, value) in [(":message-type", "event"), (":event-type", event), (":content-type", "application/json")] {
            headers.push(name.len() as u8);
            headers.extend_from_slice(name.as_bytes());
            headers.push(7);
            headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
            headers.extend_from_slice(value.as_bytes());
        }
        let payload = payload.to_string().into_bytes();
        let total = 12 + headers.len() + payload.len() + 4;
        let mut out = Vec::new();
        out.extend_from_slice(&(total as u32).to_be_bytes());
        out.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&headers);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    #[test]
    fn a_chat_conversation_becomes_kiros_history_and_current_message() {
        let long_tool = "mcp__filesystem__read_text_file_with_a_very_long_name_from_a_server_xx";
        let chat = json!({"messages":[
            {"role":"system","content":"be brief"},
            {"role":"assistant","content":"stray opening"},
            {"role":"user","content":"read it"},
            {"role":"assistant","content":"","tool_calls":[{"id":"call:1","type":"function","function":{"name":long_tool,"arguments":"{\"path\":\"a\"}"}},
                {"id":"call 2","type":"function","function":{"name":"bash","arguments":"oops"}}]},
            {"role":"tool","tool_call_id":"call:1","content":"text"},
            {"role":"user","content":[{"type":"text","text":"and this"},{"type":"image_url","image_url":{"url":"data:image/jpg;base64,QQ=="}}]}
        ],"tools":[{"type":"function","function":{"name":"bash","description":"run","parameters":{"type":"object"}}}],"reasoning_effort":"high"});
        let (body, names, thinking) = kiro_request(&chat, "claude-sonnet-4.5", "arn:aws:codewhisperer:us-east-1:1:profile/P");
        assert!(thinking, "a Claude model asked to think hard thinks aloud");
        let state = &body["conversationState"];
        let history = state["history"].as_array().unwrap();
        assert_eq!(history.len(), 2, "the stray opening reply goes; the user's first message and the calls stay");
        let first = history[0]["userInputMessage"]["content"].as_str().unwrap();
        assert!(first.starts_with("<thinking_mode>enabled</thinking_mode><max_thinking_length>30000</max_thinking_length>\nbe brief\n\nread it"));
        let uses = history[1]["assistantResponseMessage"]["toolUses"].as_array().unwrap();
        let short = uses[0]["name"].as_str().unwrap();
        assert!(short.len() <= 64 && names[short] == long_tool, "a long tool name is cut and mapped back");
        assert_eq!(uses[0]["toolUseId"], "call:1");
        assert!(uses[1]["toolUseId"].as_str().unwrap().starts_with("t_"), "an id Kiro would refuse is replaced, the same each time");
        assert_eq!(uses[1]["input"], json!({}));
        let current = &state["currentMessage"]["userInputMessage"];
        assert_eq!(current["content"], "and this");
        assert_eq!(current["images"][0]["format"], "jpeg");
        let results = current["userInputMessageContext"]["toolResults"].as_array().unwrap();
        assert_eq!(results.len(), 2, "every call is answered, the unanswered one as interrupted");
        assert_eq!(results[1]["status"], "error");
        let tools: Vec<_> = current["userInputMessageContext"]["tools"].as_array().unwrap().iter()
            .map(|tool| tool["toolSpecification"]["name"].as_str().unwrap().to_owned()).collect();
        assert!(tools.contains(&"bash".to_owned()) && tools.contains(&short.to_owned()), "a tool the history used is declared too");
        assert_eq!(body["profileArn"], "arn:aws:codewhisperer:us-east-1:1:profile/P");
        assert_eq!(current["modelId"], "claude-sonnet-4.5");
    }

    #[test]
    fn the_event_stream_becomes_events_however_it_is_cut() {
        let mut stream = Vec::new();
        stream.extend(frame("assistantResponseEvent", &json!({"content":"<think"})));
        stream.extend(frame("assistantResponseEvent", &json!({"content":"ing>\nplan</thin"})));
        stream.extend(frame("assistantResponseEvent", &json!({"content":"king>\nDone"})));
        stream.extend(frame("toolUseEvent", &json!({"toolUseId":"tu-1","name":"short_ab12cd34","input":"{\"a\":"})));
        stream.extend(frame("toolUseEvent", &json!({"toolUseId":"tu-1","name":"short_ab12cd34","input":"1}","stop":true})));
        stream.extend(frame("metadataEvent", &json!({"tokenUsage":{"uncachedInputTokens":12,"outputTokens":5,"cacheReadInputTokens":3}})));
        let mut decoder = Decoder::new(HashMap::from([("short_ab12cd34".to_owned(), "original/tool".to_owned())]), true, Some(1000));
        let mut out = Vec::new();
        for piece in stream.chunks(7) {
            decoder.feed(piece, &mut out);
        }
        decoder.finish(&mut out);
        let text: String = out.iter().filter_map(|event| match event { Event::Text(text) => Some(text.as_str()), _ => None }).collect();
        let reasoning: String = out.iter().filter_map(|event| match event { Event::Reasoning(text) => Some(text.as_str()), _ => None }).collect();
        assert_eq!((reasoning.as_str(), text.as_str()), ("plan", "Done"));
        assert!(out.contains(&Event::ToolStart { id: "tu-1".into(), name: "original/tool".into() }));
        let args: String = out.iter().filter_map(|event| match event { Event::ToolArgs { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert_eq!(args, "{\"a\":1}");
        assert_eq!(out[out.len() - 2], Event::Finish("tool_calls"));
        assert_eq!(out[out.len() - 1], Event::Usage { input: 12, output: 5, cached: 3, reasoning: 0 });
    }

    #[test]
    fn an_exception_in_the_stream_is_an_error() {
        let mut stream = frame("x", &json!({"message":"slow down"}));
        // Mark it an exception of the throttling kind.
        let name = b":message-type";
        let at = stream.windows(name.len()).position(|window| window == name).unwrap() + name.len() + 3;
        stream[at..at + 5].copy_from_slice(b"error");
        let mut decoder = Decoder::new(HashMap::new(), false, None);
        let mut out = Vec::new();
        decoder.feed(&stream, &mut out);
        decoder.finish(&mut out);
        assert!(matches!(out.as_slice(), [Event::Error(message)] if message.contains("slow down")));
        assert!(explain(400, b"{\"reason\":\"CONTENT_LENGTH_EXCEEDS_THRESHOLD\"}").unwrap().contains("context"));
        assert_eq!(Creds { profile: "arn:aws:codewhisperer:eu-central-1:1:profile/P".into(), ..Default::default() }.api_region(), "eu-central-1");
    }
}
