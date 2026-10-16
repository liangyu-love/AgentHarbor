//! ZCode (zcode.z.ai): Z.ai's GLM Coding Plan, or BigModel's (智谱), signed
//! in to the way ZCode signs in, and its plan's GLM models served over
//! Anthropic's Messages.
//!
//! Ported from magpie's community plugin (MIT, see LICENSES/THIRD_PARTY.md).
//! A browser sign-in is a flow zcode.z.ai opens and is polled for; with the
//! account's sign-in it finds or makes the key ZCode uses (`zcode-api-key` in
//! the account's default project) for the plan's endpoint,
//! `api.z.ai/api/anthropic` or `open.bigmodel.cn/api/anthropic`. An account
//! with no plan of its own may have a seat on a team's plan (its project's
//! `zcode-team-api-key`), or ZCode's free Start Plan, served by zcode.z.ai to
//! ZCode's own session token — and only to what looks like ZCode's own
//! request: its headers, its system prompt ahead of the agent's and the day
//! in a `<system-reminder>` turn (`dress`). An account with a coding plan and
//! gift plans also lists each gift model as `<model>-Trial`, which spends
//! the gift. The ZCode app's own sign-in (`~/.zcode/v2`, AES-GCM encrypted
//! with a key made from the machine's user) can be followed instead, and a
//! plan's API key pasted.
use super::{
    oauth,
    store::{Quota, SourceModel},
};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch;

const APP_VERSION: &str = "3.14.3";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
/// How long a plan's route (and ZCode's provider config) is trusted.
const ROUTE_TTL: Duration = Duration::from_secs(600);

fn ua() -> String {
    format!("ZCode/{APP_VERSION}")
}

/// Every host ZCode talks to; tests point them all at one loopback.
struct Hosts {
    zcode: String,
    zai_api: String,
    bigmodel_api: String,
    zai_base: String,
    bigmodel_base: String,
}

fn hosts() -> Hosts {
    match super::endpoint_override(super::SourceKind::Zcode) {
        Some(base) => Hosts {
            zcode: base.clone(), zai_api: base.clone(), bigmodel_api: format!("{base}/bigmodel"),
            zai_base: format!("{base}/api/anthropic"), bigmodel_base: format!("{base}/bigmodel/api/anthropic"),
        },
        None => Hosts {
            zcode: "https://zcode.z.ai".into(), zai_api: "https://api.z.ai".into(), bigmodel_api: "https://bigmodel.cn".into(),
            zai_base: "https://api.z.ai/api/anthropic".into(), bigmodel_base: "https://open.bigmodel.cn/api/anthropic".into(),
        },
    }
}

pub fn start_base() -> String {
    format!("{}/api/v1/zcode-plan/anthropic", hosts().zcode)
}

/// The plan's endpoint for a site.
pub fn site_base(site: &str) -> String {
    let hosts = hosts();
    if site == "bigmodel" { hosts.bigmodel_base } else { hosts.zai_base }
}

fn site_api(site: &str) -> String {
    let hosts = hosts();
    if site == "bigmodel" { hosts.bigmodel_api } else { hosts.zai_api }
}

fn site_name(site: &str) -> &'static str {
    if site == "bigmodel" { "BigModel" } else { "Z.ai" }
}

fn subscribe_url(site: &str) -> &'static str {
    if site == "bigmodel" { "bigmodel.cn/glm-coding" } else { "z.ai/subscribe" }
}

/// ZCode's name for this machine: `win32-x64`, `darwin-arm64`, `linux-x64`.
fn platform() -> String {
    format!("{}-{}", oauth::node_platform(), oauth::node_arch())
}

/// A device id for calls made signed out (the model config).
fn any_device() -> &'static str {
    static DEVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DEVICE.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    /// "zai" or "bigmodel".
    pub site: String,
    /// The coding plan's endpoint.
    pub base: String,
    /// The coding plan's (or team seat's) key, `<id>.<secret>`; empty for a
    /// Start Plan alone.
    pub key: String,
    /// ZCode's session token, the Start Plan's key; it can't be refreshed.
    pub jwt: String,
    /// The business API's Authorization, to make a team seat's key.
    pub token: String,
    pub org: String,
    pub project: String,
    pub plan: String,
    pub device: String,
    /// Follows the ZCode app's own sign-in, read for each request.
    pub from_app: bool,
    pub user: String,
}

impl Creds {
    pub fn is_team(&self) -> bool {
        !self.org.is_empty() && !self.project.is_empty()
    }

    fn device(&self) -> &str {
        if self.device.is_empty() { any_device() } else { &self.device }
    }

    /// As requests need it: the app's sign-in read now while ZCode is signed
    /// in to this same account, else as it was kept.
    pub fn current(&self) -> Creds {
        if self.from_app {
            if let Some(now) = own_sign_in().filter(|now| now.user == self.user) {
                return Creds { device: self.device.clone(), plan: self.plan.clone(), ..now };
            }
        }
        self.clone()
    }
}

fn jwt_expiry(jwt: &str) -> i64 {
    oauth::jwt_claims(jwt)["exp"].as_i64().unwrap_or(0)
}

pub fn jwt_expired(jwt: &str) -> bool {
    let exp = jwt_expiry(jwt);
    exp > 0 && chrono::Utc::now().timestamp() > exp
}

const EXPIRED: &str = "ZCode's sign-in has expired; sign in to ZCode again (or add the account again)";

#[derive(Debug)]
struct ZError {
    status: u16,
    message: String,
}

impl std::fmt::Display for ZError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One of Z.ai's JSON endpoints, which wrap what they say in {code, msg,
/// data}: a code of 0 or 200 is a success. zcode.z.ai is told the
/// machine's id, as ZCode tells it.
async fn call(client: &Client, method: Method, url: &str, auth: Option<&str>, body: Option<&Value>, headers: &[(&str, String)], device: &str) -> Result<Value, ZError> {
    let mut request = client.request(method, url).header("content-type", "application/json").header("accept", "application/json")
        .header("user-agent", ua()).timeout(Duration::from_secs(30));
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    if url.starts_with(&format!("{}/", hosts().zcode)) {
        request = request.header("x-device-mid", device);
    }
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    if let Some(body) = body {
        request = request.body(body.to_string());
    }
    let response = request.send().await.map_err(|_| ZError { status: 0, message: "cannot reach Z.ai".into() })?;
    let status = response.status().as_u16();
    let envelope: Value = response.json().await.unwrap_or(Value::Null);
    let code = match &envelope["code"] {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let message = envelope["msg"].as_str().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(ZError { status, message: if message.is_empty() { format!("HTTP {status}") } else { format!("{message} ({status})") } });
    }
    if !envelope.is_object() {
        return Err(ZError { status, message: "not JSON".into() });
    }
    if !code.is_empty() && code != "0" && code != "200" {
        return Err(ZError { status, message: if message.is_empty() { format!("error {code}") } else { message.to_owned() } });
    }
    Ok(envelope["data"].clone())
}

fn root_of(base: &str) -> String {
    reqwest::Url::parse(base).map(|url| format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default())
        + &url.port().map(|port| format!(":{port}")).unwrap_or_default()).unwrap_or_else(|_| hosts().zai_api)
}

/// The business API of the site a plan is served from.
fn biz_root(base: &str) -> String {
    if base.contains("bigmodel") { site_api("bigmodel") } else { root_of(base) }
}

/// The coding plan a key has, "" for none.
async fn plan(client: &Client, base: &str, key: &str) -> Result<String, ZError> {
    let subs = call(client, Method::GET, &format!("{}/api/biz/subscription/list", root_of(base)), Some(key), None, &[], any_device()).await?;
    Ok(subs.as_array().into_iter().flatten().find(|sub| sub["status"].as_str().is_some_and(|status| status.eq_ignore_ascii_case("VALID")))
        .and_then(|sub| sub["productName"].as_str()).unwrap_or_default().to_owned())
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// A gift bucket's models: its capabilities, `model:` ones and bare names.
fn bucket_models(bucket: &Value) -> Vec<String> {
    bucket["capabilities"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::trim)
        .filter(|capability| capability.starts_with("model:") || !capability.contains(':'))
        .map(|capability| capability.trim_start_matches("model:").trim().to_owned()).filter(|model| !model.is_empty()).collect()
}

/// A bucket is spent by its `remaining_units` (or used of total when it gives none).
fn bucket_spent(bucket: &Value) -> bool {
    if let Some(left) = number(&bucket["remaining_units"]) {
        return left <= 0.0;
    }
    matches!((number(&bucket["total_units"]), number(&bucket["used_units"])), (Some(total), Some(used)) if used >= total)
}

/// The gift an account can spend now: the models its live buckets serve
/// (`models`), every model its plans' buckets name (`all`), and how long
/// that holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gift {
    pub name: String,
    pub models: Vec<String>,
    pub all: Vec<String>,
    ttl: Duration,
}

impl Gift {
    fn serves(&self, model: &str) -> bool {
        self.models.iter().any(|served| served.eq_ignore_ascii_case(model))
    }

    /// The raw model a `<model>-Trial` entry names.
    fn trial(&self, id: &str) -> Option<String> {
        self.all.iter().find(|model| trial_id(model).eq_ignore_ascii_case(id)).cloned()
    }
}

/// ZCode's models before its config is read; the Start Plan has all but GLM-5.3.
const MODELS: [(&str, u64, u64, &[&str]); 4] = [
    ("GLM-5.3", 1_000_000, 128_000, &["low", "high", "max"]),
    ("GLM-5.3-Flash", 1_000_000, 128_000, &["low", "high", "max"]),
    ("GLM-5.2", 1_000_000, 128_000, &["none", "high", "max"]),
    ("GLM-5-Turbo", 200_000, 64_000, &["none", "high"]),
];

/// A gift model's trial entry, in the catalog's spelling: GLM-5.3-Flash-Trial.
fn trial_id(model: &str) -> String {
    let named = MODELS.iter().find(|(id, ..)| id.eq_ignore_ascii_case(model)).map_or(model, |(id, ..)| id);
    format!("{named}-Trial")
}

/// The billing balance's live plans and buckets.
async fn start_plan(client: &Client, jwt: &str, device: &str) -> Result<Option<Gift>, String> {
    if jwt.is_empty() {
        return Err("not signed in to ZCode".into());
    }
    if jwt_expired(jwt) {
        return Err(EXPIRED.into());
    }
    let url = format!("{}/api/v1/zcode-plan/billing/balance?app_version={APP_VERSION}", hosts().zcode);
    let balance = call(client, Method::GET, &url, Some(&format!("Bearer {jwt}")), None, &[], device).await.map_err(|error| error.to_string())?;
    let now = number(&balance["server_time"]).filter(|time| *time > 0.0).unwrap_or(chrono::Utc::now().timestamp() as f64);
    let same = |bucket: &Value, plan: &Value| match (bucket["user_plan_id"].as_str().filter(|id| !id.is_empty()), plan["user_plan_id"].as_str().filter(|id| !id.is_empty())) {
        (Some(a), Some(b)) => a == b,
        _ => bucket["plan_id"].to_string() == plan["plan_id"].to_string(),
    };
    let mut gift = Gift { ttl: ROUTE_TTL, ..Default::default() };
    let mut any = false;
    for plan in balance["plans"].as_array().into_iter().flatten() {
        let end = number(&plan["ends_at"]).unwrap_or(0.0);
        if !plan["status"].as_str().is_some_and(|status| status.trim().eq_ignore_ascii_case("active")) || end > 0.0 && end <= now {
            continue;
        }
        any = true;
        if end > 0.0 {
            gift.ttl = gift.ttl.min(Duration::from_secs_f64(end - now));
        }
        if gift.name.is_empty() {
            gift.name = plan["name"].as_str().unwrap_or_default().trim().to_owned();
        }
        for bucket in balance["balances"].as_array().into_iter().flatten().filter(|bucket| same(bucket, plan)) {
            let expires = number(&bucket["expires_at"]).unwrap_or(0.0);
            if expires > 0.0 && expires <= now {
                continue;
            }
            if expires > 0.0 {
                gift.ttl = gift.ttl.min(Duration::from_secs_f64(expires - now));
            }
            for model in bucket_models(bucket) {
                if !gift.all.iter().any(|known| known.eq_ignore_ascii_case(&model)) {
                    gift.all.push(model.clone());
                }
                if !bucket_spent(bucket) && !gift.models.contains(&model) {
                    gift.models.push(model);
                }
            }
        }
    }
    if gift.name.is_empty() {
        gift.name = "Start Plan".into();
    }
    Ok(any.then_some(gift))
}

/// Where an account's requests go: an account with no coding plan sends
/// every request to the gift (`start`); one with a plan may also hold gift
/// plans (`gift`). Asked again after ten minutes (a minute when unsure).
#[derive(Clone, Debug, Default)]
pub struct Route {
    pub start: bool,
    pub gift: Option<Gift>,
}

fn routes() -> &'static Mutex<HashMap<String, (Route, Instant, Duration)>> {
    static ROUTES: std::sync::OnceLock<Mutex<HashMap<String, (Route, Instant, Duration)>>> = std::sync::OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// Gift models said spent, by key, token and model, until when.
fn blocked() -> &'static Mutex<HashMap<String, Instant>> {
    static BLOCKED: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    BLOCKED.get_or_init(Default::default)
}

fn block_key(creds: &Creds, model: &str) -> String {
    format!("{}\0{}\0{}", creds.key, creds.jwt, model.to_ascii_lowercase())
}

pub async fn route(client: &Client, creds: &Creds) -> Route {
    if creds.is_team() || creds.jwt.is_empty() {
        return Route::default();
    }
    if creds.key.is_empty() {
        return Route { start: true, gift: None };
    }
    let id = format!("{}\0{}", creds.key, creds.jwt);
    if let Some((route, at, ttl)) = routes().lock().unwrap().get(&id).cloned() {
        if at.elapsed() < ttl {
            return route;
        }
    }
    let (route, sure) = match plan(client, &creds.base, &creds.key).await {
        Ok(name) if name.is_empty() => (Route { start: true, gift: None }, true),
        Ok(_) => (Route { start: false, gift: start_plan(client, &creds.jwt, creds.device()).await.ok().flatten() }, true),
        Err(_) => (Route { start: false, gift: start_plan(client, &creds.jwt, creds.device()).await.ok().flatten() }, false),
    };
    let ttl = route.gift.as_ref().map_or(ROUTE_TTL, |gift| gift.ttl).min(if sure { ROUTE_TTL } else { Duration::from_secs(60) });
    routes().lock().unwrap().insert(id, (route.clone(), Instant::now(), ttl));
    route
}

// ---- keys ------------------------------------------------------------------------

fn team_headers(base: &str, org: &str, project: &str) -> Vec<(&'static str, String)> {
    vec![
        ("bigmodel-organization", org.to_owned()),
        ("bigmodel-project", project.to_owned()),
        ("set-language", if base.contains("bigmodel") { "zh" } else { "en" }.to_owned()),
        ("accept-language", "en-US,en".to_owned()),
    ]
}

fn keys_url(root: &str, org: &str, project: &str) -> String {
    format!("{root}/api/biz/v1/organization/{}/projects/{}/api_keys", oauth::encode_uri_component(org), oauth::encode_uri_component(project))
}

/// The project's key named `name` (of `key_type`, when given), made when it
/// isn't there, with its secret: `<id>.<secret>`.
async fn project_key(client: &Client, site: &str, keys: &str, auth: &str, headers: &[(&'static str, String)], name: &str, key_type: Option<i64>) -> Result<String, String> {
    let label = site_name(site);
    let list = call(client, Method::GET, keys, Some(auth), None, headers, any_device()).await.map_err(|error| format!("{label} API keys: {error}"))?;
    let typed = |entry: &Value| key_type.is_none_or(|kind| entry["keyType"].to_string().trim_matches('"') == kind.to_string());
    let mut id = list.as_array().into_iter().flatten()
        .filter(|entry| entry["name"] == name && typed(entry))
        .filter_map(|entry| entry["apiKey"].as_str().map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned))
        .last().unwrap_or_default();
    if id.is_empty() {
        let mut want = json!({"name": name});
        if let Some(kind) = key_type {
            want["keyType"] = json!(kind);
        }
        let made = call(client, Method::POST, keys, Some(auth), Some(&want), headers, any_device()).await.map_err(|error| format!("{label} API key: {error}"))?;
        id = made["apiKey"].as_str().unwrap_or_default().trim().to_owned();
    }
    if id.is_empty() {
        return Err(format!("{label} gave no API key"));
    }
    let copy = call(client, Method::GET, &format!("{keys}/copy/{}", oauth::encode_uri_component(&id)), Some(auth), None, headers, any_device()).await
        .map_err(|error| format!("{label} API key: {error}"))?;
    let secret = copy["secretKey"].as_str().unwrap_or_default().trim();
    match (secret.is_empty(), key_type.is_some()) {
        // a team's key with no secret goes as it is
        (true, true) => Ok(id),
        (true, false) => Err(format!("{label} gave no API key")),
        (false, _) => Ok(format!("{id}.{secret}")),
    }
}

/// A team seat's key when the sign-in kept none.
pub async fn team_key(client: &Client, creds: &Creds) -> Result<String, String> {
    if !creds.key.is_empty() {
        return Ok(creds.key.clone());
    }
    let site = if creds.base.contains("bigmodel") { "bigmodel" } else { "zai" };
    project_key(client, site, &keys_url(&site_api(site), &creds.org, &creds.project), &creds.token,
        &team_headers(&creds.base, &creds.org, &creds.project), "zcode-team-api-key", Some(2)).await
        .map_err(|error| format!("ZCode's team plan: {error} — sign in to ZCode again"))
}

// ---- signing in ------------------------------------------------------------------

/// The business API's Authorization for a sign-in: Z.ai's token exchanged
/// for a business one, as a Bearer; BigModel's goes bare.
async fn biz_auth(client: &Client, site: &str, token: &str) -> Result<String, String> {
    if site == "bigmodel" {
        return Ok(token.to_owned());
    }
    let biz = call(client, Method::POST, &format!("{}/api/auth/z/login", hosts().zai_api), None, Some(&json!({"token": token})), &[], any_device()).await
        .map_err(|error| format!("Z.ai sign-in: {error}"))?;
    let access = biz["access_token"].as_str().filter(|token| !token.is_empty()).ok_or("Z.ai sign-in: no token")?;
    Ok(format!("Bearer {access}"))
}

/// What a sign-in gives: the account's own coding plan key and plan; or,
/// with none of its own, a seat on a team's plan; or else ZCode's token for
/// its Start Plan while the account has one.
async fn signed_in(client: &Client, site: &str, token: &str, jwt: &str, device: &str) -> Result<Creds, String> {
    let name = site_name(site);
    let root = site_api(site);
    let base = site_base(site);
    let blank = Creds { site: site.into(), base: base.clone(), jwt: jwt.into(), device: device.into(), ..Default::default() };
    let mut own: Option<Creds> = None;
    let mut failure: Option<String> = None;
    let mut team_why = String::new();
    match biz_auth(client, site, token).await {
        Err(error) => failure = Some(error),
        Ok(auth) => match call(client, Method::GET, &format!("{root}/api/biz/customer/getCustomerInfo"), Some(&auth), None, &[], device).await {
            Err(error) => failure = Some(format!("{name} account: {error}")),
            Ok(info) => {
                match mint_key(client, site, &root, &auth, &info).await {
                    Ok((key, plan_name)) if !plan_name.is_empty() => return Ok(Creds { key, plan: plan_name, ..blank }),
                    Ok((key, _)) => own = Some(Creds { key, ..blank.clone() }),
                    Err(error) => failure = Some(error),
                }
                match team_sign_in(client, site, &root, &auth, &info).await {
                    Ok(Some(team)) => return Ok(Creds { jwt: jwt.into(), device: device.into(), ..team }),
                    Ok(None) => {}
                    Err(why) => team_why = why,
                }
            }
        },
    }
    let mut start_error = None;
    if !jwt.is_empty() {
        match start_plan(client, jwt, device).await {
            Ok(Some(gift)) => {
                let kept = if failure.is_none() { own.unwrap_or_else(|| blank.clone()) } else { blank.clone() };
                return Ok(Creds { plan: gift.name, ..kept });
            }
            Ok(None) => {}
            Err(error) => start_error = Some(error),
        }
    }
    Err(match (team_why.is_empty(), failure, start_error) {
        (false, _, Some(start)) => format!("{team_why}; ZCode's Start Plan: {start}"),
        (false, _, None) => team_why,
        (true, Some(error), Some(start)) => format!("{error}; ZCode's Start Plan: {start}"),
        (true, Some(error), None) => error,
        (true, None, Some(start)) => format!("this {name} account has no GLM Coding Plan, of its own or a team's, and ZCode's Start Plan could not be read: {start}"),
        (true, None, None) => format!("this {name} account has no GLM Coding Plan, of its own or a team's, and ZCode's Start Plan has ended or was never started — subscribe at {}, then sign in again", subscribe_url(site)),
    })
}

/// The account's own coding plan key, in its default project, and its plan.
async fn mint_key(client: &Client, site: &str, root: &str, auth: &str, info: &Value) -> Result<(String, String), String> {
    let mut pick: Option<(String, String)> = None;
    for organization in info["organizations"].as_array().into_iter().flatten() {
        let Some(org) = organization["organizationId"].as_str().filter(|id| !id.is_empty()) else { continue };
        let projects: Vec<&Value> = organization["projects"].as_array().into_iter().flatten()
            .filter(|project| project["projectId"].as_str().is_some_and(|id| !id.is_empty()) && project["projectType"].to_string().trim_matches('"') != "2")
            .collect();
        let Some(first) = projects.first() else { continue };
        let default = projects.iter().find(|project| project["projectName"].as_str().is_some_and(|name| name.contains("默认项目"))).unwrap_or(first);
        let named = organization["organizationName"].as_str().is_some_and(|name| name.contains("默认机构"));
        if pick.is_none() || named {
            pick = Some((org.to_owned(), default["projectId"].as_str().unwrap_or_default().to_owned()));
            if named {
                break;
            }
        }
    }
    let (org, project) = pick.ok_or_else(|| format!("this {} account has no project for an API key", site_name(site)))?;
    let key = project_key(client, site, &keys_url(root, &org, &project), auth, &[], "zcode-api-key", None).await?;
    let plan_name = plan(client, &site_base(site), &key).await.map_err(|error| format!("GLM Coding Plan: {error}"))?;
    Ok((key, plan_name))
}

/// A seat on a team's plan: the first team project whose plan is in force
/// and gives the account a seat. `Ok(None)` when it is in no team.
async fn team_sign_in(client: &Client, site: &str, root: &str, auth: &str, info: &Value) -> Result<Option<Creds>, String> {
    let base = site_base(site);
    let name = site_name(site);
    let mut teams = Vec::new();
    for organization in info["organizations"].as_array().into_iter().flatten() {
        for project in organization["projects"].as_array().into_iter().flatten() {
            if let (Some(org), Some(id)) = (organization["organizationId"].as_str(), project["projectId"].as_str()) {
                if project["projectType"].to_string().trim_matches('"') == "2" {
                    teams.push((org.to_owned(), id.to_owned()));
                }
            }
        }
    }
    if teams.is_empty() {
        return Ok(None);
    }
    let (mut why, mut last) = (String::new(), String::new());
    for (org, project) in teams {
        let headers = team_headers(&base, &org, &project);
        let detail = match call(client, Method::GET, &format!("{root}/api/biz/team/subscribe/product/querySubscribeDetail"), Some(auth), None, &headers, any_device()).await {
            Ok(detail) => detail,
            Err(error) => {
                last = error.to_string();
                continue;
            }
        };
        let status = detail["status"].as_str().unwrap_or_default().to_ascii_uppercase();
        let grant = detail["memberGrantStatus"].as_str().unwrap_or_default().to_ascii_uppercase();
        if !(detail["hasSubscription"] != false && status == "EFFECTIVE" && grant == "VALID") {
            if why.is_empty() && status == "EFFECTIVE" && grant == "UNASSIGNED" {
                why = format!("this {name} account is in a team with a GLM Coding Plan but has no seat on it yet — ask the team's admin to give it one, then sign in again");
            }
            if why.is_empty() && status == "EXPIRED" {
                why = format!("the {name} team's GLM Coding Plan this account is in has expired");
            }
            continue;
        }
        let key = project_key(client, site, &keys_url(root, &org, &project), auth, &headers, "zcode-team-api-key", Some(2)).await
            .map_err(|error| format!("the team's GLM Coding Plan: {error}"))?;
        let plan_name = detail["productName"].as_str().filter(|name| !name.is_empty()).unwrap_or("GLM Coding Team").to_owned();
        return Ok(Some(Creds { site: site.into(), base: base.clone(), key, token: auth.into(), org, project, plan: plan_name, ..Default::default() }));
    }
    Err(if why.is_empty() { format!("the team's GLM Coding Plan: {}", if last.is_empty() { "no seat on it" } else { &last }) } else { why })
}

pub struct SignIn {
    pub url: String,
    site: String,
    flow: String,
    poll_auth: String,
    device: String,
    interval: Duration,
    deadline: Instant,
}

/// Open ZCode's sign-in flow on a site ("zai" or "bigmodel").
pub async fn begin_sign_in(client: &Client, site: &str) -> Result<SignIn, String> {
    let poll_auth = format!("Bearer {}", oauth::random_hex(32));
    let device = uuid::Uuid::new_v4().to_string();
    let zcode = hosts().zcode;
    let flow = call(client, Method::POST, &format!("{zcode}/api/v1/oauth/cli/init"), Some(&poll_auth), Some(&json!({"provider": site})), &[], &device).await
        .map_err(|error| format!("ZCode sign-in: {error}"))?;
    let id = flow["flow_id"].as_str().filter(|id| !id.is_empty()).ok_or("ZCode gave no sign-in page")?.to_owned();
    let mut url = reqwest::Url::parse(flow["authorize_url"].as_str().unwrap_or_default()).map_err(|_| "ZCode gave no sign-in page".to_owned())?;
    if url.scheme() != "https" && super::endpoint_override(super::SourceKind::Zcode).is_none() {
        return Err("ZCode gave no sign-in page".into());
    }
    // where Z.ai or BigModel sends the browser back, as ZCode sets it
    let back = format!("{zcode}/app/oauth/login?redirect={}&app_version={APP_VERSION}", oauth::encode_uri_component("zcode://oauth/callback"));
    url.query_pairs_mut().append_pair(if site == "bigmodel" { "redirect" } else { "redirect_uri" }, &back);
    let interval = Duration::from_secs(flow["poll_interval_sec"].as_u64().unwrap_or(1).max(1));
    let deadline = flow["expires_at"].as_i64().filter(|at| *at > 0)
        .map(|at| Instant::now() + Duration::from_secs((at - chrono::Utc::now().timestamp()).max(0) as u64))
        .unwrap_or_else(|| Instant::now() + SIGN_IN_WINDOW);
    Ok(SignIn { url: url.to_string(), site: site.into(), flow: id, poll_auth, device, interval, deadline })
}

/// The account a ZCode user record names.
fn who(user: &Value) -> String {
    [user["email"].as_str().unwrap_or_default().trim_end_matches("@phone.local"), user["name"].as_str().unwrap_or_default(),
        user["user_id"].as_str().unwrap_or_default()]
        .into_iter().map(str::trim).find(|text| !text.is_empty()).unwrap_or("ZCode").to_owned()
}

/// Poll the flow until the browser is done, then find the account's plan.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, mut cancel: watch::Receiver<bool>, floor: Duration) -> Result<Creds, String> {
    let url = format!("{}/api/v1/oauth/cli/poll/{}", hosts().zcode, oauth::encode_uri_component(&sign_in.flow));
    loop {
        tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            _ = tokio::time::sleep(if cfg!(test) { floor } else { sign_in.interval.max(floor) }) => {}
        }
        if Instant::now() >= sign_in.deadline {
            return Err("accounts.loginTimeout".into());
        }
        let got = match call(client, Method::GET, &url, Some(&sign_in.poll_auth), None, &[], &sign_in.device).await {
            Ok(got) => got,
            Err(error) if (400..500).contains(&error.status) && error.status != 408 && error.status != 429 => return Err(format!("ZCode sign-in: {error}")),
            Err(_) => continue,
        };
        let token = if sign_in.site == "bigmodel" {
            got["bigmodel"]["access_token"].as_str().or(got["bigmodel"]["accessToken"].as_str())
        } else {
            got["zai"]["access_token"].as_str()
        }.unwrap_or_default().trim().to_owned();
        match got["status"].as_str().unwrap_or_default() {
            "" | "pending" => continue,
            "failed" => return Err(format!("the sign-in was declined on {}", site_name(&sign_in.site))),
            "ready" if !token.is_empty() => {
                let jwt = got["token"].as_str().unwrap_or_default().trim().to_owned();
                let mut creds = signed_in(client, &sign_in.site, &token, &jwt, &sign_in.device).await?;
                creds.user = who(&got["user"]);
                return Ok(creds);
            }
            other => return Err(format!("ZCode sign-in: unexpected answer {other}")),
        }
    }
}

/// A pasted plan key, of the site it is from.
pub async fn sign_in_with_key(client: &Client, site: &str, key: &str) -> Result<Creds, String> {
    let base = site_base(site);
    let plan_name = plan(client, &base, key).await.map_err(|error| format!("GLM Coding Plan: {error}"))?;
    Ok(Creds { site: site.into(), base, key: key.to_owned(), plan: plan_name, device: any_device().into(), user: format!("{} API key", site_name(site)), ..Default::default() })
}

// ---- the ZCode app's own sign-in --------------------------------------------------

/// Tests point ZCode's home at a folder of their own.
#[cfg(test)]
pub(crate) static TEST_HOME: Mutex<Option<PathBuf>> = Mutex::new(None);

fn home() -> PathBuf {
    #[cfg(test)]
    if let Some(home) = TEST_HOME.lock().unwrap().clone() {
        return home;
    }
    dirs::home_dir().unwrap_or_default()
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Where ZCode keeps setting.json, moved or not.
fn setting_dir() -> PathBuf {
    home().join(".zcode").join("v2")
}

/// Where ZCode keeps credentials.json and runtime/provider: under the
/// folder its data was moved to (setting.json's dataBaseDir), else home.
fn zcode_dir() -> PathBuf {
    let moved = read_json(&setting_dir().join("setting.json")).and_then(|setting| setting["dataBaseDir"].as_str().map(str::trim).map(PathBuf::from))
        .filter(|dir| dir.is_absolute());
    moved.unwrap_or_else(home).join(".zcode").join("v2")
}

/// The key ZCode encrypts its credentials with.
fn secret() -> [u8; 32] {
    let seed = std::env::var("ZCODE_CREDENTIAL_SECRET").ok().filter(|seed| !seed.is_empty()).unwrap_or_else(|| {
        let name = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_default();
        let name = name.rsplit('\\').next().unwrap_or_default().to_owned();
        format!("zcode-credential-fallback:{}:{}:{name}", oauth::node_platform(), home().display())
    });
    use sha2::Digest;
    sha2::Sha256::digest(seed.as_bytes()).into()
}

/// "enc:v1:<iv>.<tag>.<data>", AES-256-GCM.
fn decrypt(key: &[u8; 32], value: &Value) -> Option<String> {
    use base64::Engine;
    let rest = value.as_str()?.strip_prefix("enc:v1:")?;
    let parts: Vec<Vec<u8>> = rest.split('.').map(|part| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok())
        .collect::<Option<_>>()?;
    let [iv, tag, data] = parts.as_slice() else { return None };
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(iv).ok()?;
    let key = ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, key).ok()?);
    let mut sealed = [data.as_slice(), tag.as_slice()].concat();
    let open = key.open_in_place(nonce, ring::aead::Aad::empty(), &mut sealed).ok()?;
    String::from_utf8(open.to_vec()).ok()
}

/// What the ZCode app is signed in to now: its coding plan key, its session
/// token and the team plan it is switched to; `None` when it has none.
pub fn own_sign_in() -> Option<Creds> {
    let store = read_json(&zcode_dir().join("credentials.json"))?;
    let store = store.as_object()?;
    let key = secret();
    let hosts = hosts();
    let mut found: Option<(String, String)> = None;
    for (name, value) in store {
        if !name.contains(":coding-plan:") || !name.ends_with(":api-key") {
            continue;
        }
        let Some(plan_key) = decrypt(&key, value).filter(|plan_key| plan_key.contains('.')) else { continue };
        let base = if name.contains(":bigmodel-") { hosts.bigmodel_base.clone() } else { hosts.zai_base.clone() };
        // Z.ai's first, as ZCode lists it
        if found.as_ref().is_none_or(|(_, kept)| base == hosts.zai_base && *kept != hosts.zai_base) {
            found = Some((plan_key, base));
        }
    }
    if found.is_none() {
        // ZCode 3.15 keeps the plan key in config.json, the provider's options.apiKey
        let providers = read_json(&zcode_dir().join("config.json")).map(|config| config["provider"].clone()).unwrap_or(Value::Null);
        'search: for (ids, base) in [(["builtin:zai-coding-plan", "account:zai-individual-coding-plan"], &hosts.zai_base),
            (["builtin:bigmodel-coding-plan", "account:bigmodel-individual-coding-plan"], &hosts.bigmodel_base)] {
            for id in ids {
                let Some(raw) = providers[id]["options"]["apiKey"].as_str().map(str::trim) else { continue };
                let plan_key = if raw.starts_with("enc:") { decrypt(&key, &json!(raw)).unwrap_or_default() } else { raw.to_owned() };
                if plan_key.contains('.') && !plan_key.contains(|ch: char| ch.is_whitespace() || ch == '{' || ch == '$') {
                    found = Some((plan_key, base.clone()));
                    break 'search;
                }
            }
        }
    }
    let jwt = decrypt(&key, &store.get("zcodejwttoken").cloned().unwrap_or(Value::Null)).unwrap_or_default().trim().trim_start_matches("Bearer ").to_owned();
    let mut creds = match own_team(store, &key) {
        Some(team) => Creds { jwt: jwt.clone(), ..team },
        None => {
            let (plan_key, base) = found.unwrap_or_else(|| (String::new(), hosts.zai_base.clone()));
            let site = if base == hosts.bigmodel_base { "bigmodel" } else { "zai" };
            Creds { site: site.into(), base, key: plan_key, jwt, ..Default::default() }
        }
    };
    if creds.key.is_empty() && creds.jwt.is_empty() && !creds.is_team() {
        return None;
    }
    let mut info = Value::Null;
    for name in ["oauth:zai:user_info", "oauth:bigmodel:user_info"] {
        if who(&info) != "ZCode" {
            break;
        }
        if let Some(text) = store.get(name).and_then(|value| decrypt(&key, value)) {
            info = serde_json::from_str(&text).unwrap_or(Value::Null);
        }
    }
    creds.user = who(&info);
    creds.from_app = true;
    Some(creds)
}

/// The team plan the ZCode app switched to, when it did.
fn own_team(store: &serde_json::Map<String, Value>, key: &[u8; 32]) -> Option<Creds> {
    let setting = read_json(&setting_dir().join("setting.json"))?;
    let families: Vec<&str> = match setting["providerFamilyDomain"].as_str() {
        Some(family @ ("zai" | "bigmodel")) => vec![family],
        _ => vec!["zai", "bigmodel"],
    };
    for family in families {
        let selection = &setting["providerFamilyConnectionSelections"][family];
        let (org, project) = if selection.is_object() {
            if selection["kind"] != "team-coding-plan" {
                continue;
            }
            (selection["organizationId"].as_str().unwrap_or_default().to_owned(), selection["projectId"].as_str().unwrap_or_default().to_owned())
        } else {
            let legacy = setting["modelProviderFamilySelectedKeys"][family].as_str().unwrap_or_default();
            let Some(rest) = legacy.strip_prefix(&format!("team-plan:builtin:{family}-coding-plan:")) else { continue };
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() != 3 {
                continue;
            }
            let decode = |text: &str| reqwest::Url::parse(&format!("http://x/?v={text}")).ok()
                .and_then(|url| url.query_pairs().next().map(|(_, value)| value.into_owned())).unwrap_or_default();
            (decode(parts[1]), decode(parts[2]))
        };
        if org.is_empty() || project.is_empty() {
            continue;
        }
        let token = decrypt(key, store.get(&format!("oauth:{family}:access_token")).unwrap_or(&Value::Null)).unwrap_or_default();
        if token.is_empty() {
            continue;
        }
        // a stale token ZCode won't use either
        if family == "bigmodel" && decrypt(key, store.get("zcodejwttoken").unwrap_or(&Value::Null)).as_deref() == Some(token.as_str()) {
            continue;
        }
        return Some(Creds { site: family.into(), base: site_base(family), token, org, project, ..Default::default() });
    }
    None
}

// ---- models ----------------------------------------------------------------------

/// ZCode's providerId for the plan served at a base.
fn plan_id(base: &str) -> &'static str {
    if base.contains("/zcode-plan/") {
        "account:zai-start-plan"
    } else if base.contains("bigmodel") {
        "account:bigmodel-individual-coding-plan"
    } else {
        "account:zai-individual-coding-plan"
    }
}

/// A plan's models in ZCode's provider config.
fn models_of(config: &Value, plan: &str) -> Vec<SourceModel> {
    let config = &config["config"];
    let mut ids: Vec<String> = Vec::new();
    let mut add = |id: &str| if !ids.iter().any(|known| known.eq_ignore_ascii_case(id)) { ids.push(id.to_owned()) };
    for rule in config["providerConfigRules"]["providerRules"].as_array().into_iter().flatten().filter(|rule| rule["providerId"] == plan) {
        rule["builtinModelIds"].as_array().into_iter().flatten().filter_map(Value::as_str).for_each(&mut add);
    }
    let mut off = Vec::new();
    for rule in config["modelConfigRules"]["builtinProviderModelRules"].as_array().into_iter().flatten().filter(|rule| rule["providerId"] == plan) {
        let Some(id) = rule["modelId"].as_str() else { continue };
        if rule["config"]["enabled"] == false { off.push(id.to_ascii_lowercase()) } else { add(id) }
    }
    ids.into_iter().filter(|id| !off.contains(&id.to_ascii_lowercase())).map(|id| {
        let mut model = SourceModel { id: id.clone(), name: id.clone(), ..Default::default() };
        for rule in config["modelConfigRules"]["modelRules"].as_array().into_iter().flatten() {
            let pattern = rule["modelMatch"].as_str().unwrap_or_default();
            if !regex_lite_match(pattern, &id) {
                continue;
            }
            let settings = &rule["config"];
            if let Some(window) = settings["properties"]["contextWindow"].as_u64().filter(|window| *window > 0) {
                model.context_window = Some(window);
            }
            if let Some(max) = settings["optionSpecs"]["maxOutputTokens"]["max"].as_u64().filter(|max| *max > 0) {
                model.max_output_tokens = Some(max);
            }
            if let Some(values) = settings["optionSpecs"]["reasoningLevel"]["values"].as_array().filter(|values| !values.is_empty()) {
                model.reasoning.clear();
                for value in values.iter().filter_map(Value::as_str) {
                    let level = match value { "disabled" => "none", "enabled" => "high", other => other }.to_owned();
                    if !model.reasoning.contains(&level) {
                        model.reasoning.push(level);
                    }
                }
            }
        }
        model
    }).collect()
}

/// ZCode's `modelMatch` patterns are alternations of names with `.`, `*`
/// and `.*`: matched case-insensitively and whole.
fn regex_lite_match(pattern: &str, text: &str) -> bool {
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.first() {
            None => text.is_empty(),
            Some('\\') if pattern.len() > 1 => !text.is_empty() && text[0] == pattern[1] && matches(&pattern[2..], &text[1..]),
            Some(_) if pattern.get(1) == Some(&'*') => {
                let single = |ch: char| pattern[0] == '.' || pattern[0] == ch;
                matches(&pattern[2..], text) || (!text.is_empty() && single(text[0]) && matches(pattern, &text[1..]))
            }
            Some(_) if pattern.get(1) == Some(&'?') => {
                let single = |ch: char| pattern[0] == '.' || pattern[0] == ch;
                matches(&pattern[2..], text) || (!text.is_empty() && single(text[0]) && matches(&pattern[2..], &text[1..]))
            }
            Some('.') => !text.is_empty() && matches(&pattern[1..], &text[1..]),
            Some(&ch) => !text.is_empty() && text[0] == ch && matches(&pattern[1..], &text[1..]),
        }
    }
    let pattern = pattern.trim_start_matches('^').trim_end_matches('$').trim_start_matches("(?:").trim_end_matches(')').to_ascii_lowercase();
    let text: Vec<char> = text.to_ascii_lowercase().chars().collect();
    pattern.split('|').any(|branch| matches(&branch.chars().collect::<Vec<_>>(), &text))
}

/// The latest provider config an installed ZCode has.
fn local_config() -> Option<Value> {
    let mut best: Option<Value> = None;
    let root = zcode_dir().join("runtime").join("provider");
    let level = |dir: &std::path::Path| std::fs::read_dir(dir).into_iter().flatten().flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect::<Vec<_>>();
    for a in level(&root) {
        for b in level(&a) {
            for c in level(&b) {
                if let Some(config) = read_json(&c.join("zcode-builtin.json")) {
                    if best.as_ref().is_none_or(|kept| config["revision"].as_i64().unwrap_or(0) > kept["revision"].as_i64().unwrap_or(0)) {
                        best = Some(config);
                    }
                }
            }
        }
    }
    best
}

/// ZCode's provider config, the later of zcode.z.ai's release and an
/// installed ZCode's, kept for ten minutes.
async fn zcode_config(client: &Client) -> Option<Value> {
    static CACHE: Mutex<Option<(Value, Instant)>> = Mutex::new(None);
    if let Some((config, at)) = CACHE.lock().unwrap().clone() {
        if at.elapsed() < ROUTE_TTL {
            return Some(config);
        }
    }
    let remote = async {
        let configs = call(client, Method::GET, &format!("{}/api/v1/client/configs?app_version={APP_VERSION}&platform={}", hosts().zcode, platform()),
            None, None, &[], any_device()).await.ok()?;
        let url = configs["configs"]["builtin_provider_config_json"].as_str().filter(|url| url.starts_with("https://") || cfg!(test))?;
        client.get(url).header("user-agent", ua()).timeout(Duration::from_secs(8)).send().await.ok()?.json::<Value>().await.ok()
    };
    let mut config = remote.await;
    if let Some(local) = local_config() {
        if config.as_ref().is_none_or(|remote| local["revision"].as_i64().unwrap_or(0) > remote["revision"].as_i64().unwrap_or(0)) {
            config = Some(local);
        }
    }
    if let Some(config) = &config {
        *CACHE.lock().unwrap() = Some((config.clone(), Instant::now()));
    }
    config
}

fn fallback(start: bool) -> Vec<SourceModel> {
    MODELS.iter().filter(|(id, ..)| !start || *id != "GLM-5.3").map(|(id, context, output, efforts)| SourceModel {
        id: (*id).into(), name: (*id).into(), context_window: Some(*context), max_output_tokens: Some(*output),
        reasoning: efforts.iter().map(|effort| (*effort).to_owned()).collect(), ..Default::default()
    }).collect()
}

/// The account's plan's models as ZCode's config lists them, and its gift
/// plans' models again as `<model>-Trial` entries (a gift-only account's
/// gift models by their own names too).
pub async fn fetch_models(client: &Client, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let creds = creds.current();
    let route = route(client, &creds).await;
    let gift = if route.start { start_plan(client, &creds.jwt, creds.device()).await.ok().flatten() } else { route.gift.clone() };
    let base = if route.start { start_base() } else { creds.base.clone() };
    let config = zcode_config(client).await.unwrap_or(Value::Null);
    let mut models = models_of(&config, plan_id(&base));
    if models.is_empty() {
        models = fallback(route.start);
    }
    let details: Vec<SourceModel> = models.iter().cloned().chain(models_of(&config, plan_id(&start_base())))
        .chain(models_of(&config, plan_id(&hosts().zai_base))).chain(fallback(false)).collect();
    let about = |raw: &str| details.iter().find(|model| model.id.eq_ignore_ascii_case(raw)).cloned()
        .unwrap_or_else(|| SourceModel { id: raw.into(), name: raw.into(), ..Default::default() });
    for raw in gift.iter().flat_map(|gift| gift.all.clone()) {
        if route.start && !models.iter().any(|model| model.id.eq_ignore_ascii_case(&raw)) {
            models.push(about(&raw));
        }
        let trial = trial_id(&raw);
        if !models.iter().any(|model| model.id.eq_ignore_ascii_case(&trial)) {
            models.push(SourceModel { id: trial.clone(), name: trial, ..about(&raw) });
        }
    }
    Ok(models)
}

// ---- the allowance ---------------------------------------------------------------

/// The share of the tightest window that counts: limits that stop the
/// models (MCP calls and uncapped ones set aside).
fn tightest(limits: &Value) -> Option<f64> {
    let mut out: Option<f64> = None;
    for limit in limits["limits"].as_array().into_iter().flatten() {
        if limit["type"].as_str().is_some_and(|kind| kind.eq_ignore_ascii_case("TIME_LIMIT")) || limit["usage"] == 0 {
            continue;
        }
        let mut used = number(&limit["percentage"]).unwrap_or(0.0);
        if let Some(total) = number(&limit["usage"]).filter(|total| *total > 0.0) {
            if let Some(left) = number(&limit["remaining"]) {
                used = 100.0 * (total - left) / total;
            } else if let (Some(current), None) = (number(&limit["currentValue"]), number(&limit["percentage"])) {
                used = 100.0 * current / total;
            }
        }
        out = Some(out.map_or(used, |was: f64| was.max(used)));
    }
    out
}

/// The plan's allowance: a coding plan's or team seat's windows, or the
/// Start Plan's buckets (the least used live one).
pub async fn quota(client: &Client, creds: &Creds) -> (Quota, Option<String>) {
    let creds = creds.current();
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), unit: "percent".into(), limit: 100.0, ..Default::default() };
    let fail = |mut quota: Quota, error: String| {
        quota.error = Some(error);
        quota.limit = 0.0;
        (quota, None)
    };
    if creds.is_team() {
        let key = match team_key(client, &creds).await {
            Ok(key) => key,
            Err(error) => return fail(quota, error),
        };
        let root = biz_root(&creds.base);
        match call(client, Method::GET, &format!("{root}/api/monitor/usage/quota/limit?type=2"), Some(&key), None,
            &team_headers(&creds.base, &creds.org, &creds.project), any_device()).await {
            Ok(limits) => quota.used = tightest(&limits).unwrap_or(0.0),
            Err(error) => return fail(quota, error.to_string()),
        }
        quota.paid = Some(true);
        return (quota, None);
    }
    let route = route(client, &creds).await;
    if route.start {
        if jwt_expired(&creds.jwt) {
            return fail(quota, EXPIRED.into());
        }
        let url = format!("{}/api/v1/zcode-plan/billing/balance?app_version={APP_VERSION}", hosts().zcode);
        let balance = match call(client, Method::GET, &url, Some(&format!("Bearer {}", creds.jwt)), None, &[], creds.device()).await {
            Ok(balance) => balance,
            Err(error) => return fail(quota, error.to_string()),
        };
        let gift = start_plan(client, &creds.jwt, creds.device()).await.ok().flatten();
        let Some(gift) = gift else {
            return fail(quota, "this account has no GLM Coding Plan, and ZCode's Start Plan has ended or was never started".into());
        };
        // the least used live bucket: the one a request spends
        let mut least: Option<f64> = None;
        for bucket in balance["balances"].as_array().into_iter().flatten().filter(|bucket| !bucket_spent(bucket)) {
            let total = number(&bucket["total_units"]).filter(|total| *total > 0.0);
            let used = number(&bucket["used_units"]).or_else(|| Some(total? - number(&bucket["remaining_units"])?));
            if let (Some(total), Some(used)) = (total, used) {
                let share = 100.0 * used / total;
                least = Some(least.map_or(share, |was: f64| was.min(share)));
            }
        }
        quota.used = least.unwrap_or(100.0);
        quota.paid = Some(false);
        return (quota, Some(gift.name));
    }
    let root = root_of(&creds.base);
    match call(client, Method::GET, &format!("{root}/api/monitor/usage/quota/limit"), Some(&creds.key), None, &[], any_device()).await {
        Ok(limits) => {
            quota.used = tightest(&limits).unwrap_or(0.0);
            quota.paid = Some(true);
            let level = limits["level"].as_str().filter(|level| !level.is_empty())
                .map(|level| format!("GLM Coding {}{}", level[..1].to_ascii_uppercase(), &level[1..]));
            (quota, level)
        }
        Err(error) => fail(quota, error.to_string()),
    }
}

// ---- requests --------------------------------------------------------------------

/// The plan's headers: the key both ways the endpoint takes it.
pub fn coding_headers(key: &str) -> Vec<(&'static str, String)> {
    vec![("x-api-key", key.to_owned()), ("authorization", format!("Bearer {key}")), ("anthropic-version", "2023-06-01".to_owned())]
}

/// ZCode's own headers, which the Start Plan is served to, the jwt its key.
pub fn start_headers(jwt: &str) -> Vec<(&'static str, String)> {
    let locale = sys_locale();
    let mut headers = vec![
        ("authorization", format!("Bearer {jwt}")),
        ("anthropic-version", "2023-06-01".to_owned()),
        ("http-referer", hosts().zcode),
        ("user-agent", format!("{} ai-sdk/anthropic/3.0.81", ua())),
        ("x-zcode-app-version", APP_VERSION.to_owned()),
        ("x-title", "Z Code@cli".to_owned()),
        ("x-release-channel", "production".to_owned()),
        ("x-client-language", locale),
        ("x-zcode-agent", "glm".to_owned()),
        ("x-platform", platform()),
        ("x-os-category", if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" }.to_owned()),
        ("x-os-version", os_version()),
        ("x-request-id", uuid::Uuid::new_v4().to_string()),
        ("x-zcode-session-type", "main".to_owned()),
        ("x-zcode-trace-id", uuid::Uuid::new_v4().to_string()),
    ];
    if let Ok(zone) = std::env::var("TZ") {
        headers.push(("x-client-timezone", zone));
    }
    headers
}

fn sys_locale() -> String {
    ["LC_ALL", "LANG"].iter().filter_map(|name| std::env::var(name).ok()).map(|value| value.split('.').next().unwrap_or_default().replace('_', "-"))
        .find(|value| !value.is_empty()).unwrap_or_else(|| "en-US".into())
}

/// `win32 10.0.26200 x64`, as ZCode says its OS.
fn os_version() -> String {
    format!("{} {} {}", oauth::node_platform(), oauth::os_release(), oauth::node_arch())
}

#[derive(Deserialize)]
struct Prompt {
    #[serde(rename = "cliPrefix")]
    cli_prefix: String,
    #[serde(rename = "stableSections")]
    stable_sections: Vec<String>,
    #[serde(rename = "beforeEnvironment")]
    before_environment: String,
    #[serde(rename = "afterEnvironment")]
    after_environment: String,
    environment: HashMap<String, String>,
    context: HashMap<String, String>,
}

/// ZCode's own prompt texts, which zcode.z.ai looks for on a Start Plan
/// request, byte for byte (ZCode 3.14.3's, from magpie).
fn prompt() -> &'static Prompt {
    static PROMPT: std::sync::OnceLock<Prompt> = std::sync::OnceLock::new();
    PROMPT.get_or_init(|| serde_json::from_str(include_str!("zcode_prompt.json")).expect("ZCode's prompt texts"))
}

/// A Messages body as ZCode sends it to the Start Plan: ZCode's three
/// cached system blocks, then the agent's own system text uncached; the day
/// in a `<system-reminder>` turn first; a cache mark on the last block of
/// the last turn alone; `metadata.user_id` naming the sign-in's device.
pub fn dress(body: &[u8], site: &str, device: &str) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let messages = value["messages"].as_array()?.clone();
    let texts = prompt();
    let cached = |text: String| json!({"type": "text", "text": text, "cache_control": {"type": "ephemeral"}});
    let model = value["model"].as_str().unwrap_or_default().to_owned();
    let mut own = Vec::new();
    match &value["system"] {
        Value::String(text) if !text.trim().is_empty() => own.push(json!({"type": "text", "text": text})),
        Value::Array(blocks) => own.extend(blocks.iter().filter(|block| block["type"] == "text" && block["text"].as_str().is_some_and(|text| !text.is_empty()))
            .map(|block| json!({"type": "text", "text": block["text"]}))),
        _ => {}
    }
    let env = &texts.environment;
    let mut lines = vec![env["heading"].clone(), env["invokedLine"].clone(),
        format!("- {}: {}", env["platformLabel"], oauth::node_platform()), format!("- {}: {}", env["osVersionLabel"], os_version())];
    if !model.is_empty() {
        let provider = if site == "bigmodel" { "bigmodel-api" } else { "zai-api" };
        lines.push(env["poweredByLine"].replace("{provider}", provider).replace("{model}", &model));
    }
    let mut system = vec![cached(texts.cli_prefix.clone()), cached(texts.stable_sections.join("\n\n")),
        cached(format!("\n\n{}", [texts.before_environment.clone(), lines.join("\n"), texts.after_environment.clone()].join("\n\n")))];
    system.extend(own);
    value["system"] = Value::Array(system);
    let context = &texts.context;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let reminder = [context["intro"].clone(), format!("{}\n{}", context["currentDateHeading"], context["currentDateLine"].replace("{date}", &today)),
        String::new(), context["outro"].clone()].join("\n");
    let mut turns = vec![json!({"role": "user", "content": [{"type": "text", "text": format!("<system-reminder>{reminder}</system-reminder>")}]})];
    turns.extend(messages);
    for turn in turns.iter_mut().filter(|turn| turn["role"] != "system") {
        for block in turn["content"].as_array_mut().into_iter().flatten() {
            if let Some(block) = block.as_object_mut() {
                block.remove("cache_control");
            }
        }
    }
    if let Some(last) = turns.iter_mut().rev().find(|turn| turn["role"] != "system") {
        match &mut last["content"] {
            Value::String(text) => last["content"] = json!([cached(text.clone())]),
            Value::Array(blocks) => if let Some(block) = blocks.last_mut().and_then(Value::as_object_mut) {
                block.entry("cache_control").or_insert(json!({"type": "ephemeral"}));
            },
            _ => {}
        }
    }
    value["messages"] = Value::Array(turns);
    for tool in value["tools"].as_array_mut().into_iter().flatten() {
        if let Some(tool) = tool.as_object_mut() {
            tool.remove("cache_control");
        }
    }
    let user_id = json!({"device_id": device, "account_uuid": "", "session_id": ""}).to_string();
    if !value["metadata"].is_object() {
        value["metadata"] = json!({});
    }
    value["metadata"]["user_id"] = json!(user_id);
    serde_json::to_vec(&value).ok()
}

/// What a request on a ZCode account needs: where it goes and how it is
/// signed, by the account's route.
#[derive(Clone)]
pub struct Target {
    pub creds: Creds,
    pub route: Route,
}

impl Target {
    pub fn base(&self) -> String {
        if self.route.start { start_base() } else { self.creds.base.clone() }
    }

    pub fn headers(&self) -> Vec<(&'static str, String)> {
        if self.route.start { start_headers(&self.creds.jwt) } else { coding_headers(&self.creds.key) }
    }

    /// A gift-only account's body, dressed as ZCode's own.
    pub fn shape(&self, body: &[u8]) -> Option<Vec<u8>> {
        self.route.start.then(|| dress(body, &self.creds.site, self.creds.device())).flatten()
    }

    /// A trial entry on an account with a coding plan: the gift's model,
    /// sent where the gift is served, dressed as ZCode's own. `Err` when the
    /// gift has no live quota for it.
    pub fn trial(&self, body: &[u8]) -> Option<Result<(String, Vec<(&'static str, String)>, Vec<u8>, String), (u16, String)>> {
        if self.route.start {
            return None;
        }
        let mut value: Value = serde_json::from_slice(body).ok()?;
        let asked = value["model"].as_str()?.to_owned();
        if !asked.to_ascii_lowercase().ends_with("-trial") {
            return None;
        }
        let Some(raw) = self.route.gift.as_ref().and_then(|gift| gift.trial(&asked)) else {
            return Some(Err((400, format!("ZCode has no active limited quota for {asked}"))));
        };
        let usable = !jwt_expired(&self.creds.jwt) && self.route.gift.as_ref().is_some_and(|gift| gift.serves(&raw))
            && blocked().lock().unwrap().get(&block_key(&self.creds, &raw)).is_none_or(|until| Instant::now() >= *until);
        if !usable {
            return Some(Err((429, format!("ZCode's limited quota for {raw} is exhausted or temporarily unavailable"))));
        }
        value["model"] = json!(raw);
        let body = dress(value.to_string().as_bytes(), &self.creds.site, self.creds.device())?;
        Some(Ok((start_base(), start_headers(&self.creds.jwt), body, block_key(&self.creds, &raw))))
    }
}

/// A gift's answer to a trial entry: spent (its 1113 or 1005, even inside a
/// 200, or any 429) is a 429, and keeps that model off the gift a minute.
pub async fn gift_reply(response: reqwest::Response, block: String) -> reqwest::Response {
    let status = response.status().as_u16();
    let json = response.headers().get(hyper::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).is_some_and(|value| value.contains("application/json"));
    if status < 400 && !json {
        return response;
    }
    let headers = response.headers().clone();
    let text = response.text().await.unwrap_or_default();
    let spent = status == 429 || serde_json::from_str::<Value>(&text).ok().is_some_and(|value| {
        let code = |value: &Value| value.to_string().trim_matches('"').to_owned();
        ["1113", "1005"].contains(&code(&value["code"]).as_str()) || ["1113", "1005"].contains(&code(&value["error"]["code"]).as_str())
    });
    if spent {
        blocked().lock().unwrap().insert(block, Instant::now() + Duration::from_secs(60));
    }
    let status = if spent && status < 400 { 429 } else { status };
    let mut rebuilt = hyper::Response::builder().status(status);
    for (name, value) in &headers {
        if name != hyper::header::CONTENT_LENGTH && name != hyper::header::TRANSFER_ENCODING {
            rebuilt = rebuilt.header(name, value);
        }
    }
    reqwest::Response::from(rebuilt.body(text).expect("response parts"))
}

/// What the user can do about a ZCode refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status < 400 {
        return None;
    }
    let text = String::from_utf8_lossy(body).to_lowercase();
    if status == 405 && text.contains("unusual activity") {
        return Some("zcode.z.ai turned the Start Plan request away as not ZCode's own; sign in to ZCode again".into());
    }
    if text.contains("\"1113\"") || text.contains(":1113") || text.contains("insufficient balance") {
        return Some("this ZCode plan has no resource package left; check its allowance on the Accounts page".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_plan_body_is_dressed_as_zcode_s_own() {
        let body = json!({"model": "GLM-5.3-Flash", "system": "agent rules", "messages": [
            {"role": "user", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}]},
            {"role": "assistant", "content": "b"}, {"role": "user", "content": "c"}],
            "tools": [{"name": "t", "cache_control": {"type": "ephemeral"}}], "metadata": {"x": 1}});
        let dressed: Value = serde_json::from_slice(&dress(body.to_string().as_bytes(), "zai", "dev-1").unwrap()).unwrap();
        let system = dressed["system"].as_array().unwrap();
        assert_eq!(system[0]["text"], "You are ZCode, an interactive coding agent");
        assert_eq!(system.len(), 4);
        assert_eq!(system[3], json!({"type": "text", "text": "agent rules"}), "the agent's own text follows, uncached");
        assert!(system[2]["text"].as_str().unwrap().contains("zai-api"));
        let turns = dressed["messages"].as_array().unwrap();
        assert!(turns[0]["content"][0]["text"].as_str().unwrap().starts_with("<system-reminder>"));
        assert!(turns[1]["content"][0].get("cache_control").is_none(), "only the last turn keeps a cache mark");
        assert_eq!(turns[3]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(dressed["tools"][0].get("cache_control").is_none());
        assert_eq!(dressed["metadata"]["x"], 1);
        assert_eq!(serde_json::from_str::<Value>(dressed["metadata"]["user_id"].as_str().unwrap()).unwrap()["device_id"], "dev-1");
    }

    #[test]
    fn config_rules_pick_a_plan_s_models() {
        let config = json!({"config": {
            "providerConfigRules": {"providerRules": [{"providerId": "account:zai-individual-coding-plan", "builtinModelIds": ["GLM-5.3", "GLM-4.6"]}]},
            "modelConfigRules": {
                "builtinProviderModelRules": [{"providerId": "account:zai-individual-coding-plan", "modelId": "GLM-4.6", "config": {"enabled": false}},
                    {"providerId": "account:zai-individual-coding-plan", "modelId": "GLM-5-Turbo"}],
                "modelRules": [{"modelMatch": "glm-5.*", "config": {"properties": {"contextWindow": 1000000},
                    "optionSpecs": {"maxOutputTokens": {"max": 128000}, "reasoningLevel": {"values": ["disabled", "low", "enabled"]}}}}],
            }}});
        let models = models_of(&config, "account:zai-individual-coding-plan");
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["GLM-5.3", "GLM-5-Turbo"]);
        assert_eq!(models[0].reasoning, ["none", "low", "high"]);
        assert_eq!(models[0].context_window, Some(1_000_000));
        assert!(regex_lite_match("^(?:GLM-5\\.3|glm-5-turbo)$", "glm-5-turbo"));
        assert!(!regex_lite_match("glm-5", "glm-5.3"));
    }

    #[test]
    fn app_credentials_decrypt_with_the_machine_s_key() {
        use base64::Engine;
        let key = [7u8; 32];
        let sealing = ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key).unwrap());
        let iv = [3u8; 12];
        let mut data = b"abc.def".to_vec();
        let tag = sealing.seal_in_place_separate_tag(ring::aead::Nonce::assume_unique_for_key(iv), ring::aead::Aad::empty(), &mut data).unwrap();
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let value = json!(format!("enc:v1:{}.{}.{}", b64(&iv), b64(tag.as_ref()), b64(&data)));
        assert_eq!(decrypt(&key, &value).as_deref(), Some("abc.def"));
        assert!(decrypt(&[8u8; 32], &value).is_none(), "another machine's key opens nothing");
        assert!(decrypt(&key, &json!("plain")).is_none());
    }

    #[test]
    fn gift_buckets_and_trial_entries() {
        let gift = Gift { name: "Start Plan".into(), models: vec!["glm-5.3-flash".into()], all: vec!["glm-5.3-flash".into(), "glm-5.3".into()], ttl: ROUTE_TTL };
        assert_eq!(trial_id("glm-5.3-flash"), "GLM-5.3-Flash-Trial");
        assert_eq!(gift.trial("GLM-5.3-Flash-Trial").as_deref(), Some("glm-5.3-flash"));
        assert!(gift.serves("GLM-5.3-Flash") && !gift.serves("GLM-5.3"));
        assert!(bucket_spent(&json!({"remaining_units": 0})) && !bucket_spent(&json!({"total_units": 10})));
        assert_eq!(bucket_models(&json!({"capabilities": ["model:glm-5.3", "tool:x", "glm-5-turbo"]})), ["glm-5.3", "glm-5-turbo"]);
        assert_eq!(tightest(&json!({"limits": [{"type": "TOKENS_LIMIT", "usage": 100, "remaining": 40}, {"type": "TIME_LIMIT", "percentage": 99}]})), Some(60.0));
    }
}
