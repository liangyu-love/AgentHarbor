//! Trae (ByteDance's AI IDE): Trae CN (trae.cn) and Trae international
//! (trae.ai), each signed in to as the IDE signs in, its models asked on the
//! IDE agent's `llm_utils_chat`.
//!
//! Ported from magpie's community plugin (MIT, see LICENSES/THIRD_PARTY.md),
//! which worked the protocol out from two open-source relays
//! (wangqi233/trae2api, autumnsentiment/Trae2api-cn) — experimental there too.
//! The authorization page sends the browser back to a callback on 127.0.0.1
//! with the account's Cloud-IDE-JWT and refresh token. A device made at
//! sign-in (ids and a P-256 key) goes with every request; on Trae CN the JWT
//! is renewed bound to it (DeviceProof, as TRAE SOLO CN renews), which is
//! what Trae CN's check-in pays out to. Trae's chat takes text only: the
//! agent's tools are named natively and in a system prompt that asks for
//! each call as a tagged block, and the calls a model writes in its text —
//! in the block, GLM's template or DeepSeek's DSML — are lifted out. A model
//! is asked through the first of the IDE's chat functions that serves it.
use super::{
    checkin::{Checkin, Outcome},
    loopback, oauth,
    store::{Quota, SourceModel},
    wire::{self, Event},
    SourceKind,
};
use futures_util::StreamExt;
use hyper::body::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::watch;

const SIGN_IN_WINDOW: Duration = Duration::from_secs(600);
const DAY_MS: i64 = 24 * 3600 * 1000;
const FUNCTIONS: [&str; 4] = ["chat_v3", "solo_work_lite", "solo_agent", "solo_agent_lite"];
const BRAND: &str = "ASUS TUF Gaming A15 FA507RM_FA507RM";
const CLIENT_VERSION: &str = "0.1.69";
const CLIENT_VERSION_CODE: &str = "20260917";
const APP_ID: &str = "6eefa01c-1036-4c7e-9ca5-d891f63bfcd8";
const CLIENT_ID: &str = "ono9krqynydwx5";
/// A Max mode's model id: deepseek-v4.1-flash-max is deepseek-v4.1-flash in Max mode.
const MAX: &str = "-max";

#[derive(Clone, Debug)]
pub struct Site {
    pub kind: SourceKind,
    realm: &'static str,
    web_host: &'static str,
    web: String,
    pub auth: String,
    api: String,
    /// The international deployment's US model host, and its usage hosts.
    us: String,
    pay: String,
    us_pay: String,
    ide_version: &'static str,
    plugin_version: &'static str,
    /// Trae CN renews a JWT bound to the account's device.
    bound: bool,
    lapsed: &'static [i64],
    quota: &'static [i64],
}

pub fn site(kind: SourceKind) -> Site {
    let host = |real: &str| super::endpoint_override(kind).unwrap_or_else(|| real.to_owned());
    match kind {
        SourceKind::TraeGlobal => Site {
            kind, realm: "Trae Global", web_host: "trae.ai",
            web: host("https://www.trae.ai"), auth: host("https://growsg-normal.trae.ai"), api: host("https://coresg-normal.trae.ai"),
            us: host("https://coreva-normal.trae.ai"), pay: host("https://api-sg-central.trae.ai"), us_pay: host("https://api-us-east.trae.ai"),
            ide_version: "3.5.51", plugin_version: "2.3.62834", bound: false,
            lapsed: &[1001, 20101, 401], quota: &[4008, 1005, 4011],
        },
        _ => Site {
            kind, realm: "Trae CN", web_host: "trae.cn",
            web: host("https://www.trae.cn"), auth: host("https://api.trae.cn"), api: host("https://trae-api-cn.mchost.guru"),
            us: String::new(), pay: String::new(), us_pay: String::new(),
            ide_version: "3.3.65", plugin_version: "2.3.24254", bound: true,
            lapsed: &[1001, 401], quota: &[4008, 1005],
        },
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    /// The Cloud-IDE-JWT.
    pub token: String,
    pub refresh: String,
    /// Unix milliseconds.
    pub expires_at: i64,
    pub uid: String,
    pub name: String,
    pub client_id: String,
    pub device_id: String,
    pub machine_id: String,
    /// The device's P-256 key: SPKI PEM, and PKCS#8 (base64).
    pub device_public_key: String,
    pub device_private_key: String,
    /// A model host the sign-in named, and the account's region.
    pub api: String,
    pub region: String,
    /// A token host the sign-in named (an enterprise's).
    pub auth_host: String,
}

impl Creds {
    pub fn near_expiry(&self) -> bool {
        self.expires_at != 0 && self.expires_at - chrono::Utc::now().timestamp_millis() <= 2 * 60 * 1000
    }
}

/// A host a sign-in named, when it is an https one on Trae's own domains.
fn trae_host(host: &str) -> String {
    let Ok(url) = reqwest::Url::parse(host.trim()) else { return String::new() };
    let name = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let ours = ["trae.cn", "trae.ai", "mchost.guru"].iter().any(|domain| name == *domain || name.ends_with(&format!(".{domain}")));
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() || url.query().is_some() || !ours {
        return String::new();
    }
    format!("{}://{}{}", url.scheme(), name, url.path()).trim_end_matches('/').to_owned()
}

/// The model host for an account.
fn api_of(site: &Site, creds: &Creds) -> String {
    if super::endpoint_override(site.kind).is_some() {
        return site.api.clone();
    }
    let host = trae_host(&creds.api);
    let name = reqwest::Url::parse(&host).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
    match site.kind {
        SourceKind::TraeGlobal => {
            let region = creds.region.to_ascii_uppercase();
            if name.contains("coreva") || name.contains("api-us") || region.starts_with("US") {
                site.us.clone()
            } else if name.contains("coresg") || name.ends_with(".trae.ai") && !(name.starts_with("www.") || name.starts_with("api") || name.starts_with("grow")) {
                host
            } else {
                site.api.clone()
            }
        }
        _ => {
            let model_host = name.ends_with("mchost.guru") || name.starts_with("trae-api-")
                || name.ends_with(".trae.cn") && !(name.starts_with("www.") || name.starts_with("api.") || name.starts_with("grow"));
            if model_host { host } else { site.api.clone() }
        }
    }
}

fn auth_host_of(site: &Site, creds: &Creds) -> String {
    let host = trae_host(&creds.auth_host);
    let name = reqwest::Url::parse(&host).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
    if !host.is_empty() && (name == site.web_host || name.ends_with(&format!(".{}", site.web_host))) { host } else { site.auth.clone() }
}

/// One of Trae's expiry times: seconds, milliseconds or ISO, in ms.
fn when_of(value: &Value) -> i64 {
    if let Some(text) = value.as_str().filter(|text| !text.trim().chars().all(|ch| ch.is_ascii_digit())) {
        return chrono::DateTime::parse_from_rfc3339(text.trim()).map(|at| at.timestamp_millis()).unwrap_or(0);
    }
    let number = value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok())).unwrap_or(0.0);
    if number <= 0.0 { 0 } else if number < 1e11 { (number * 1000.0) as i64 } else { number as i64 }
}

fn jwt_exp(token: &str) -> i64 {
    when_of(&oauth::jwt_claims(token)["exp"])
}

fn parse(value: &Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

/// What Trae said went wrong, from its several shapes.
fn error_of(value: &Value) -> (String, String) {
    let meta = &value["ResponseMetadata"]["Error"];
    if meta.is_object() {
        return (text(&meta["Code"]), text(&meta["Message"]));
    }
    let error = if value["error"].is_object() { &value["error"] } else { value };
    let code = [&error["code"], &error["Code"], &value["code"]].into_iter().map(text).find(|code| !code.is_empty()).unwrap_or_default();
    let message = [&error["message"], &error["msg"], &error["Message"]].into_iter().map(text).find(|message| !message.is_empty())
        .or_else(|| value["error"].as_str().map(str::to_owned)).unwrap_or_default();
    (code, message)
}

// ---- the device -------------------------------------------------------------------

fn pem(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = body.as_bytes().chunks(64).map(|chunk| std::str::from_utf8(chunk).expect("base64")).collect();
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
}

/// A device: 19 digits, 32 hex, and its P-256 key pair.
fn new_device() -> (String, String, String, String) {
    let mut digits = String::new();
    for byte in (0..2).flat_map(|_| *uuid::Uuid::new_v4().as_bytes()) {
        if digits.len() < 19 {
            digits.push(char::from(b'0' + if digits.is_empty() { 1 + byte % 9 } else { byte % 10 }));
        }
    }
    let (public, private) = new_device_key();
    (digits, oauth::random_hex(16), public, private)
}

fn new_device_key() -> (String, String) {
    use base64::Engine;
    use ring::signature::KeyPair;
    let alg = &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &ring::rand::SystemRandom::new()).expect("a P-256 key");
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref(), &ring::rand::SystemRandom::new()).expect("a P-256 key");
    // SubjectPublicKeyInfo for an uncompressed P-256 point
    let mut spki = hex_bytes("3059301306072a8648ce3d020106082a8648ce3d030107034200");
    spki.extend_from_slice(pair.public_key().as_ref());
    (pem("PUBLIC KEY", &spki), base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref()))
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex")).collect()
}

/// A renewal's DeviceProof: the device key's ECDSA-SHA256 signature (DER,
/// base64) of the method, path, client, refresh token, time and nonce.
fn proof_of(path: &str, client_id: &str, refresh: &str, private_key: &str) -> Option<Value> {
    use base64::Engine;
    let pkcs8 = base64::engine::general_purpose::STANDARD.decode(private_key).ok()?;
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8, &ring::rand::SystemRandom::new()).ok()?;
    let at = chrono::Utc::now().timestamp();
    let nonce = oauth::random_hex(16);
    let message = ["POST", path, client_id, refresh, &at.to_string(), &nonce].join("\n");
    let signature = pair.sign(&ring::rand::SystemRandom::new(), message.as_bytes()).ok()?;
    Some(json!({"Signature": base64::engine::general_purpose::STANDARD.encode(signature.as_ref()), "Timestamp": at, "Nonce": nonce}))
}

// ---- tokens -----------------------------------------------------------------------

pub enum RefreshError {
    SignedOut(String),
    Transient,
}

/// Trade a refresh token for a new JWT, as the IDE does.
async fn exchange_token(client: &Client, site: &Site, refresh: &str, client_id: &str, host: &str) -> Result<(Creds, Value), RefreshError> {
    let body = json!({"ClientID": if client_id.is_empty() { CLIENT_ID } else { client_id }, "RefreshToken": refresh, "ClientSecret": "-", "UserID": ""});
    let response = client.post(format!("{}/cloudide/api/v3/trae/oauth/ExchangeToken", host.trim_end_matches('/')))
        .header("content-type", "application/json").body(body.to_string()).timeout(Duration::from_secs(20)).send().await
        .map_err(|_| RefreshError::Transient)?;
    let status = response.status().as_u16();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    let result = if value["Result"].is_object() { &value["Result"] } else if value["result"].is_object() { &value["result"] } else { &value };
    let token = [&result["Token"], &result["token"]].into_iter().map(text).find(|token| !token.is_empty());
    if let (true, Some(token)) = ((200..300).contains(&status), token) {
        let info = parse(if result["UserInfo"].is_null() { &result["userInfo"] } else { &result["UserInfo"] });
        let creds = Creds {
            refresh: [&result["RefreshToken"], &result["refreshToken"]].into_iter().map(text).find(|token| !token.is_empty()).unwrap_or_else(|| refresh.to_owned()),
            expires_at: [when_of(&result["TokenExpireAt"]), when_of(&result["tokenExpireAt"]), jwt_exp(&token)].into_iter().find(|at| *at > 0).unwrap_or(0),
            client_id: [&result["ClientID"], &result["clientId"]].into_iter().map(text).find(|id| !id.is_empty()).unwrap_or_else(|| client_id.to_owned()),
            token,
            ..Default::default()
        };
        return Ok((creds, info));
    }
    let (code, message) = error_of(&value);
    let lower = message.to_lowercase();
    if [400, 401, 403].contains(&status) || code == "10101" || ["refresh", "expired", "invalid", "not matched", "revoked", "unauthori"].iter().any(|word| lower.contains(word)) {
        return Err(RefreshError::SignedOut(format!("{}'s sign-in has expired ({message}); sign in again", site.realm)));
    }
    Err(RefreshError::Transient)
}

/// Renew the JWT bound to the account's device, as Trae CN's clients do:
/// tried as TRAE SOLO CN first, then as Trae CN's IDE.
async fn bound_exchange(client: &Client, creds: &Creds, host: &str) -> Option<Creds> {
    const PATH: &str = "/trae/api/v3/oauth/ExchangeToken";
    for (client_id, platform, version) in [("en1oxy7wnw8j9n", "SOLO_PC", "0.1.64"), (CLIENT_ID, "IDE_PC", "3.3.99")] {
        let proof = proof_of(PATH, client_id, &creds.refresh, &creds.device_private_key)?;
        let body = json!({"ClientID": client_id, "ClientSecret": "", "RefreshToken": creds.refresh, "IDEVersion": version, "DeviceProof": proof,
            "DeviceInfo": {"DeviceID": creds.device_id, "MachineID": creds.machine_id, "PlatformCode": platform, "DeviceType": "PC", "DeviceName": "",
                "DeviceModel": "", "ClientVersion": version, "DevicePublicKey": creds.device_public_key, "DeviceBrand": BRAND, "DeviceCPU": "AMD",
                "OSInfo": "Windows", "OSVersion": "10.0.22631"}});
        let mut request = client.post(format!("{}{PATH}", host.trim_end_matches('/'))).header("content-type", "application/json").body(body.to_string())
            .timeout(Duration::from_secs(20));
        if !creds.token.is_empty() {
            request = request.header("authorization", format!("Cloud-IDE-JWT {}", creds.token));
        }
        let Ok(response) = request.send().await else { continue };
        let ok = response.status().is_success();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        let result = if value["Result"].is_object() { &value["Result"] } else { &value["result"] };
        let token = [&result["Token"], &result["token"]].into_iter().map(text).find(|token| !token.is_empty());
        if let (true, Some(token)) = (ok, token) {
            return Some(Creds {
                refresh: [&result["RefreshToken"], &result["refreshToken"]].into_iter().map(text).find(|token| !token.is_empty()).unwrap_or_else(|| creds.refresh.clone()),
                expires_at: [when_of(&result["TokenExpireAt"]), when_of(&result["tokenExpireAt"]), jwt_exp(&token)].into_iter().find(|at| *at > 0).unwrap_or(0),
                token,
                ..creds.clone()
            });
        }
    }
    None
}

/// Renew an account's JWT: bound to its device where the site can, else
/// (or when Trae turns that away) at /cloudide/… as the IDE did before.
pub async fn refresh(client: &Client, site: &Site, creds: &Creds) -> Result<Creds, RefreshError> {
    if creds.refresh.is_empty() {
        return Err(RefreshError::SignedOut(format!("{}'s sign-in has no refresh token; sign in again", site.realm)));
    }
    let mut creds = creds.clone();
    if creds.device_private_key.is_empty() {
        (creds.device_public_key, creds.device_private_key) = new_device_key();
    }
    let host = auth_host_of(site, &creds);
    if site.bound {
        if let Some(next) = bound_exchange(client, &creds, &host).await {
            return Ok(next);
        }
    }
    let (next, _) = exchange_token(client, site, &creds.refresh, &creds.client_id, &host).await?;
    Ok(Creds { token: next.token, refresh: next.refresh, expires_at: next.expires_at, client_id: next.client_id, ..creds })
}

// ---- the browser sign-in --------------------------------------------------------

pub struct SignIn {
    pub url: String,
    listener: tokio::net::TcpListener,
    site: Site,
    device: (String, String, String, String),
}

/// Open the IDE's authorization page, which sends the browser back to a
/// callback here with the token pair and the account.
pub async fn begin_sign_in(site: Site) -> Result<SignIn, String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|_| "accounts.loginFailed".to_owned())?;
    let port = listener.local_addr().map_err(|_| "accounts.loginFailed".to_owned())?.port();
    let device = new_device();
    let mut url = reqwest::Url::parse(&format!("{}/authorization", site.web)).map_err(|_| "accounts.loginFailed".to_owned())?;
    url.query_pairs_mut().extend_pairs([
        ("login_version", "1"), ("auth_from", "solo"), ("login_channel", "native_ide"), ("plugin_version", site.plugin_version),
        ("auth_type", "local"), ("client_id", CLIENT_ID), ("redirect", "0"), ("login_trace_id", &uuid::Uuid::new_v4().to_string()),
        ("auth_callback_url", &format!("http://127.0.0.1:{port}/authorize")), ("machine_id", &device.1), ("device_id", &device.0),
        ("x_device_id", &device.0), ("x_machine_id", &device.1), ("x_device_brand", BRAND), ("x_device_type", "windows"),
        ("x_os_version", "Windows 10 Pro"), ("x_env", ""), ("x_app_version", site.ide_version), ("x_app_type", "stable"), ("hide_saas_login", "false"),
    ]);
    Ok(SignIn { url: url.to_string(), listener, site, device })
}

/// The account a callback names.
async fn signed_in(client: &Client, site: &Site, query: &HashMap<String, String>, device: &(String, String, String, String)) -> Result<Creds, String> {
    let get = |key: &str| query.get(key).cloned().unwrap_or_default();
    let jwt: Value = serde_json::from_str(&get("userJwt")).unwrap_or(Value::Null);
    let mut info: Value = serde_json::from_str(&get("userInfo")).unwrap_or(Value::Null);
    let pick = |value: &Value, keys: &[&str]| keys.iter().map(|key| text(&value[*key])).find(|found| !found.is_empty()).unwrap_or_default();
    let mut creds = Creds {
        token: pick(&jwt, &["Token", "token"]),
        refresh: [pick(&jwt, &["RefreshToken", "refreshToken"]), get("refreshToken")].into_iter().find(|token| !token.is_empty()).unwrap_or_default(),
        expires_at: when_of(if jwt["TokenExpireAt"].is_null() { &jwt["tokenExpireAt"] } else { &jwt["TokenExpireAt"] }),
        client_id: [pick(&jwt, &["ClientID", "clientId"]), get("clientId"), CLIENT_ID.to_owned()].into_iter().find(|id| !id.is_empty()).unwrap_or_default(),
        api: [get("host"), pick(&info, &["Host"])].into_iter().find(|host| !host.is_empty()).unwrap_or_default(),
        auth_host: [get("authHost"), get("auth_host"), pick(&info, &["AuthHost", "authHost"])].into_iter().find(|host| !host.is_empty()).unwrap_or_default(),
        device_id: device.0.clone(), machine_id: device.1.clone(), device_public_key: device.2.clone(), device_private_key: device.3.clone(),
        ..Default::default()
    };
    let host = auth_host_of(site, &creds);
    if creds.token.is_empty() && !creds.refresh.is_empty() {
        let (next, user) = exchange_token(client, site, &creds.refresh, &creds.client_id, &host).await.map_err(|error| match error {
            RefreshError::SignedOut(message) => message,
            RefreshError::Transient => format!("{}: renewing the sign-in failed", site.realm),
        })?;
        creds = Creds { token: next.token, refresh: next.refresh, expires_at: next.expires_at, client_id: next.client_id, ..creds };
        if !user.is_null() {
            info = user;
        }
    }
    if creds.token.is_empty() {
        return Err(format!("{} sent back no token", site.web_host));
    }
    if creds.refresh.is_empty() {
        return Err(format!("{} sent back no refresh token", site.web_host));
    }
    creds.uid = [pick(&info, &["UserID", "userId"]), get("userId")].into_iter().find(|id| !id.is_empty()).unwrap_or_default();
    creds.name = pick(&info, &["ScreenName", "screenName"]);
    creds.region = pick(&info, &["Region", "region", "AIRegion", "aiRegion"]);
    if creds.expires_at == 0 {
        creds.expires_at = [jwt_exp(&creds.token), chrono::Utc::now().timestamp_millis() + DAY_MS].into_iter().find(|at| *at > 0).unwrap_or(0);
    }
    // the token the page gives is bound to no device: renewed for this one at
    // once, so a check-in takes it; when Trae won't, it goes as is
    if site.bound {
        if let Some(bound) = bound_exchange(client, &creds, &host).await {
            creds = bound;
        }
    }
    Ok(creds)
}

/// Wait for the browser at the callback, answer it with a page.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, mut cancel: watch::Receiver<bool>) -> Result<Creds, String> {
    let deadline = tokio::time::Instant::now() + SIGN_IN_WINDOW;
    loop {
        let accepted = tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            accepted = tokio::time::timeout_at(deadline, sign_in.listener.accept()) => accepted,
        };
        let Ok(Ok((mut socket, _))) = accepted else { return Err("accounts.loginTimeout".into()) };
        let Some(request) = loopback::read_incoming(&mut socket).await else { continue };
        let cors = vec![("Access-Control-Allow-Origin", "*".to_owned()), ("Access-Control-Allow-Methods", "GET, OPTIONS".to_owned()),
            ("Access-Control-Allow-Headers", "Content-Type".to_owned())];
        if request.method == "OPTIONS" {
            loopback::respond(&mut socket, "204 No Content", &cors, "").await;
            continue;
        }
        if request.path != "/authorize" {
            loopback::respond(&mut socket, "404 Not Found", &cors, "").await;
            continue;
        }
        let mut html = cors.clone();
        html.push(("Content-Type", "text/html; charset=utf-8".to_owned()));
        if let Some(error) = request.query.get("error") {
            let message = request.query.get("error_description").unwrap_or(error).clone();
            loopback::respond(&mut socket, "200 OK", &html, &loopback::page(false, "Sign-in didn't finish", &message)).await;
            return Err(format!("{} sign-in: {message}", sign_in.site.realm));
        }
        if !request.query.contains_key("userJwt") && !request.query.contains_key("refreshToken") {
            loopback::respond(&mut socket, "200 OK", &html, &loopback::page(false, "Nothing to sign in with", "Start the sign-in again.")).await;
            continue;
        }
        let result = signed_in(client, &sign_in.site, &request.query, &sign_in.device).await;
        let page = match &result {
            Ok(creds) => loopback::page(true, "You're signed in", &format!("{} is signed in. You can close this tab.", if creds.name.is_empty() { &creds.uid } else { &creds.name })),
            Err(error) => loopback::page(false, "Sign-in didn't finish", error),
        };
        loopback::respond(&mut socket, "200 OK", &html, &page).await;
        return result;
    }
}

// ---- requests ---------------------------------------------------------------------

/// The IDE's headers, for the account and its device.
fn ide_headers(creds: &Creds) -> Vec<(&'static str, String)> {
    vec![
        ("content-type", "application/json".into()),
        ("authorization", format!("Cloud-IDE-JWT {}", creds.token)),
        ("x-cloudide-token", creds.token.clone()),
        ("x-ide-token", creds.token.clone()),
        ("x-uid", creds.uid.clone()),
        ("x-app-id", APP_ID.into()),
        ("x-device-id", creds.device_id.clone()),
        ("x-machine-id", creds.machine_id.clone()),
        ("x-request-id", uuid::Uuid::new_v4().to_string()),
        ("x-app-version", CLIENT_VERSION.into()),
        ("x-app-version-code", CLIENT_VERSION_CODE.into()),
        ("x-ide-version", CLIENT_VERSION.into()),
        ("x-ide-version-code", CLIENT_VERSION_CODE.into()),
        ("x-ide-version-type", "stable".into()),
        ("x-device-cpu", "AMD".into()),
        ("x-device-brand", BRAND.into()),
        ("x-device-type", "windows".into()),
        ("x-os-version", "Windows 10".into()),
        ("x-system-type", "Windows".into()),
    ]
}

fn request(client: &Client, url: &str, headers: &[(&'static str, String)]) -> reqwest::RequestBuilder {
    headers.iter().fold(client.post(url), |request, (name, value)| request.header(*name, value))
}

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";

/// The system prompt that names the agent's tools and asks for each call as
/// a tagged block.
fn tool_prompt(tools: &[Value], choice: &Value, parallel: &Value) -> String {
    let defs: Vec<Value> = tools.iter().map(|tool| {
        let function = if tool["function"].is_object() { &tool["function"] } else { tool };
        json!({"name": function["name"], "description": function["description"].as_str().unwrap_or_default(),
            "parameters": if function["parameters"].is_null() { json!({}) } else { function["parameters"].clone() }})
    }).collect();
    let mut lines = vec![
        "You can call the client's tools, listed below as JSON. They run on the user's machine, not on yours: use no built-in or server tool to reach the user's files.".to_owned(),
        format!("To call a tool, write one block per call and nothing after it: {OPEN}{{\"name\":\"tool_name\",\"arguments\":{{...}}}}{CLOSE}. Use an exact tool name and fill arguments from its schema. Then stop and wait: the result comes back in the next message."),
        "A call is not done until its result comes back; don't say it is, and don't repeat a call whose result you have.".to_owned(),
    ];
    if parallel == &json!(false) {
        lines.push("Call at most one tool at a time.".into());
    }
    if choice == "required" {
        lines.push("Call a tool before answering.".into());
    } else if let Some(name) = choice["function"]["name"].as_str().or(choice["name"].as_str()) {
        lines.push(format!("Call the tool {name} before answering."));
    }
    lines.push("Tools:".into());
    lines.push(Value::Array(defs).to_string());
    lines.join("\n")
}

fn part_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| match part {
            Value::String(text) => Some(text.clone()),
            _ => match part["type"].as_str() {
                Some("text" | "input_text") => Some(part["text"].as_str().unwrap_or_default().to_owned()),
                Some("image_url") => Some("[an image, which this model can't see]".to_owned()),
                _ => None,
            },
        }).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn tools_of(chat: &Value) -> Vec<Value> {
    if chat["tool_choice"] == "none" { Vec::new() } else { chat["tools"].as_array().cloned().unwrap_or_default() }
}

/// The conversation as Trae's chat takes it: text only, a call written as
/// its block, a result as the user's turn, no two turns of one role in a row.
fn trae_messages(chat: &Value) -> Vec<Value> {
    let tools = tools_of(chat);
    let mut names: HashMap<String, String> = HashMap::new();
    let mut out: Vec<(String, String)> = Vec::new();
    if !tools.is_empty() {
        out.push(("system".into(), tool_prompt(&tools, &chat["tool_choice"], &chat["parallel_tool_calls"])));
    }
    for message in chat["messages"].as_array().into_iter().flatten() {
        let role = match message["role"].as_str().unwrap_or_default() { "developer" => "system", other => other };
        let content = part_text(&message["content"]);
        match role {
            "assistant" => {
                let calls: Vec<String> = message["tool_calls"].as_array().into_iter().flatten().map(|call| {
                    let id = call["id"].as_str().unwrap_or_default().to_owned();
                    names.insert(id, call["function"]["name"].as_str().unwrap_or_default().to_owned());
                    let raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!(raw));
                    format!("{OPEN}{}{CLOSE}", json!({"name": call["function"]["name"], "arguments": arguments}))
                }).collect();
                let joined = std::iter::once(content).chain(calls).filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n");
                out.push(("assistant".into(), joined));
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().unwrap_or("?");
                let name = names.get(id).cloned().or_else(|| message["name"].as_str().map(str::to_owned)).unwrap_or_else(|| "the tool".into());
                out.push(("user".into(), format!("Result of {name} (call {id}):\n{content}")));
            }
            "system" => out.push(("system".into(), content)),
            _ => out.push(("user".into(), content)),
        }
    }
    let mut merged: Vec<(String, String)> = Vec::new();
    for (role, text) in out.into_iter().filter(|(_, text)| !text.is_empty()) {
        match merged.last_mut() {
            Some((last, kept)) if *last == role && role != "system" => {
                kept.push_str("\n\n");
                kept.push_str(&text);
            }
            _ => merged.push((role, text)),
        }
    }
    merged.into_iter().map(|(role, text)| json!({"role": role, "content": [{"type": "text", "text": text}]})).collect()
}

/// The salt conversation ids are made with: new in each run.
fn salt() -> &'static str {
    static SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SALT.get_or_init(|| oauth::random_hex(16))
}

/// A version-4 UUID made of a hash.
fn uuid_of(seed: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("{}|{seed}", salt()).as_bytes());
    let mut bytes: [u8; 16] = digest[..16].try_into().expect("16 bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// The conversation a chat belongs to, as the account answering it: its
/// first user message, which every later turn repeats.
fn conversation_id(chat: &Value, who: &str) -> String {
    let first = chat["messages"].as_array().into_iter().flatten().find(|message| message["role"] == "user").map(|message| part_text(&message["content"])).unwrap_or_default();
    if first.trim().is_empty() { String::new() } else { uuid_of(&format!("magpie-trae-first|{who}|{first}")) }
}

/// How a model is asked: the function whose list has it, the __dev model it
/// names there and its max_tokens, and a Max mode's base and window.
#[derive(Clone, Debug, Default)]
pub struct Route {
    function: String,
    name: String,
    most: u64,
    max: Option<(String, u64)>,
}

/// The request for llm_utils_chat.
fn chat_body(chat: &Value, function: &str, route: Option<&Route>, session: &str) -> Value {
    let max = route.and_then(|route| route.max.clone());
    let config = max.as_ref().map_or_else(|| chat["model"].as_str().unwrap_or_default().to_owned(), |(base, _)| base.clone());
    let mut body = json!({"messages": trae_messages(chat), "function": function, "config_name": config, "model": config, "stream": true,
        "request_id": uuid::Uuid::new_v4().to_string(), "session_id": if session.is_empty() { uuid::Uuid::new_v4().to_string() } else { session.to_owned() }});
    let own = route.filter(|route| route.function == function || max.is_some());
    if let Some(route) = own.filter(|route| !route.name.is_empty()) {
        body["model_name"] = json!(route.name);
    }
    let most = own.map_or(0, |route| route.most);
    if let Some(asked) = chat["max_completion_tokens"].as_u64().or(chat["max_tokens"].as_u64()).filter(|asked| *asked > 0) {
        body["max_tokens"] = json!(if most > 0 { asked.min(most) } else { asked });
    }
    if let Some((_, window)) = max {
        if body["max_tokens"].is_null() && most > 0 {
            body["max_tokens"] = json!(most);
        }
        let reply = body["max_tokens"].as_u64().unwrap_or(0);
        body["user_message_context"] = json!({"model_info": {"prompt_max_tokens": window.saturating_sub(reply)}});
    }
    if let Some(temperature) = chat["temperature"].as_f64() {
        body["temperature"] = json!(temperature);
    }
    let tools = tools_of(chat);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(|tool| {
            let function = if tool["function"].is_object() { &tool["function"] } else { tool };
            let parameters = match &function["parameters"] {
                Value::String(text) => text.clone(),
                Value::Null => "{}".into(),
                other => other.to_string(),
            };
            json!({"type": "function", "function": {"name": function["name"], "description": function["description"].as_str().unwrap_or_default(), "parameters": parameters}})
        }).collect());
        match &chat["tool_choice"] {
            Value::String(choice) => body["tool_choice"] = json!(choice),
            choice @ Value::Object(_) => body["tool_choice"] = json!(choice["function"]["name"].as_str().or(choice["name"].as_str()).unwrap_or("auto")),
            _ => {}
        }
        if let Some(parallel) = chat["parallel_tool_calls"].as_bool() {
            body["parallel_tool_calls"] = json!(parallel);
        }
    }
    body
}

// ---- tool calls written as text -------------------------------------------------

const CUT_SHORT: &str = "the model's tool call, written as text, was cut short; it is not sent on as the answer";
const UNREADABLE: &str = "the model's tool call, written as text, can't be read; it is not sent on as the answer";
/// What text may be the start of, held back.
const MARKS: [&str; 9] = [OPEN, CLOSE, "<toolcall>", "</toolcall>", "<｜DSML｜", "<|DSML|", "</｜DSML｜", "</|DSML|", "{\"reasoning_content\""];

/// Text ends inside a code fence or inline code.
fn quoted(text: &str) -> bool {
    if text.matches("```").count() % 2 == 1 {
        return true;
    }
    text.rsplit('\n').next().unwrap_or_default().replace("```", "").matches('`').count() % 2 == 1
}

/// Control characters inside JSON strings escaped.
fn escape_raw(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut string, mut escaped) = (false, false);
    for ch in text.chars() {
        if string && !escaped && (ch as u32) < 0x20 {
            out.push_str(&match ch { '\n' => "\\n".into(), '\r' => "\\r".into(), '\t' => "\\t".into(), other => format!("\\u{:04x}", other as u32) });
            continue;
        }
        if string && ch == '\\' && !escaped {
            escaped = true;
        } else {
            if ch == '"' && !escaped {
                string = !string;
            }
            escaped = false;
        }
        out.push(ch);
    }
    out
}

/// A JSON object as a model writes one.
fn loose_json(text: &str) -> Option<Map<String, Value>> {
    [text.to_owned(), escape_raw(text)].iter().find_map(|candidate| serde_json::from_str::<Value>(candidate).ok()?.as_object().cloned())
}

/// A backslash that escapes nothing JSON knows made a backslash of its own.
fn fix_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' {
            match chars.get(index + 1) {
                Some('u') if chars.len() >= index + 6 && chars[index + 2..index + 6].iter().all(char::is_ascii_hexdigit) => {
                    out.extend(&chars[index..index + 6]);
                    index += 6;
                    continue;
                }
                Some(next @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                    out.push('\\');
                    out.push(*next);
                    index += 2;
                    continue;
                }
                Some('\'') => {
                    out.push('\'');
                    index += 2;
                    continue;
                }
                _ => {
                    out.push_str("\\\\");
                    index += 1;
                    continue;
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Quotes inside strings a model left unescaped, escaped.
fn quote_fix(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<char> = Vec::new();
    let mut key = false;
    let mut index = 0;
    let rest = |from: usize| chars[from..].iter().collect::<String>();
    while index < chars.len() {
        let ch = chars[index];
        if ch != '"' {
            match ch {
                '{' => {
                    stack.push('{');
                    key = true;
                }
                '[' => stack.push('['),
                '}' | ']' => {
                    stack.pop();
                }
                ',' => key = stack.last() == Some(&'{'),
                ':' => key = false,
                _ => {}
            }
            out.push(ch);
            index += 1;
            continue;
        }
        let is_key = key && stack.last() == Some(&'{');
        let in_array = stack.last() == Some(&'[');
        out.push(ch);
        index += 1;
        while index < chars.len() {
            let c = chars[index];
            if c == '\\' {
                out.push(c);
                if let Some(next) = chars.get(index + 1) {
                    out.push(*next);
                }
                index += 2;
                continue;
            }
            if c != '"' {
                out.push(c);
                index += 1;
                continue;
            }
            let after = rest(index + 1);
            let trimmed = after.trim_start();
            let ends = if is_key {
                trimmed.starts_with(':')
            } else {
                let closers = trimmed.trim_start_matches(|ch: char| ch == '}' || ch == ']' || ch.is_whitespace());
                closers.is_empty() || closers.strip_prefix(',').is_some_and(|next| {
                    let next = next.trim_start();
                    if in_array { next.starts_with(['"', '{', '[', '-', 't', 'f', 'n']) || next.starts_with(|ch: char| ch.is_ascii_digit()) } else { next.starts_with('"') }
                })
            };
            if ends {
                out.push(c);
                index += 1;
                break;
            }
            out.push_str("\\\"");
            index += 1;
        }
        key = false;
    }
    out
}

/// A JSON object as models get it wrong: every quote escaped, a stray
/// backslash, quotes left unescaped.
fn repaired_json(text: &str) -> Option<Map<String, Value>> {
    if let Some(value) = loose_json(text) {
        return Some(value);
    }
    let raw = escape_raw(text);
    let mut tries = Vec::new();
    if text.trim_start().starts_with("{\\\"") {
        tries.push(text.replace("\\\"", "\""));
    }
    let fixed = fix_escapes(&raw);
    tries.push(quote_fix(&fixed));
    tries.insert(0, fixed);
    tries.iter().find_map(|candidate| serde_json::from_str::<Value>(candidate).ok()?.as_object().cloned())
}

fn call_json(text: &str) -> Option<Map<String, Value>> {
    repaired_json(text).filter(|value| value.get("name").and_then(Value::as_str).is_some_and(|name| !name.is_empty()))
}

/// A block's arguments as JSON: under arguments, input or parameters, else
/// the block's other keys.
fn call_args(value: &Map<String, Value>) -> String {
    let arguments = ["arguments", "input", "parameters"].iter().find_map(|key| value.get(*key)).cloned().unwrap_or_else(|| {
        Value::Object(value.iter().filter(|(key, _)| !matches!(key.as_str(), "name" | "type" | "id")).map(|(key, value)| (key.clone(), value.clone())).collect())
    });
    match arguments {
        Value::String(text) if text.trim().is_empty() => "{}".into(),
        Value::String(text) => loose_json(&text).map_or(text, |object| Value::Object(object).to_string()),
        Value::Null => "{}".into(),
        other => other.to_string(),
    }
}

fn props_of<'a>(tools: &'a [Value], name: &str) -> Option<&'a Map<String, Value>> {
    tools.iter().map(|tool| if tool["function"].is_object() { &tool["function"] } else { tool })
        .find(|function| function["name"] == name).and_then(|function| function["parameters"]["properties"].as_object())
}

/// A value as the tool's schema has it: its text for a string, else its JSON.
fn arg_value(raw: &str, key: &str, props: Option<&Map<String, Value>>) -> Value {
    if props.and_then(|props| props.get(key)).is_some_and(|schema| schema["type"] == "string") {
        return json!(raw);
    }
    serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!(raw))
}

/// Pairs `open … middle … close` in `text`: (key, value) of each.
fn tag_pairs(text: &str, key_open: &str, key_close: &str, value_open: &str, value_close: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(key_open) {
        let after = &rest[start + key_open.len()..];
        let Some(key_end) = after.find(key_close) else { break };
        let key = after[..key_end].trim().to_owned();
        let after_key = after[key_end + key_close.len()..].trim_start();
        let Some(value_body) = after_key.strip_prefix(value_open) else { rest = &after[key_end..]; continue };
        let Some(value_end) = value_body.find(value_close) else { break };
        out.push((key, value_body[..value_end].to_owned()));
        rest = &value_body[value_end + value_close.len()..];
    }
    out
}

/// GLM's call template: the tool's name, then `<arg_key>k</arg_key><arg_value>v</arg_value>` pairs.
fn glm_call(text: &str, tools: &[Value]) -> Option<(String, String)> {
    let text = text.trim_end().trim_end_matches(CLOSE).trim_end_matches("</toolcall>");
    let at = text.find("<arg_key>");
    let name = at.map_or(text, |at| &text[..at]).trim();
    if name.is_empty() || name.len() > 128 || !name.chars().all(|ch| ch.is_alphanumeric() || "_.:-".contains(ch)) {
        return None;
    }
    if at.is_none() && props_of(tools, name).is_none() && !tools.iter().any(|tool| tool["function"]["name"] == name || tool["name"] == name) {
        return None;
    }
    let props = props_of(tools, name);
    let pairs = tag_pairs(text, "<arg_key>", "</arg_key>", "<arg_value>", "</arg_value>");
    if at.is_some() && pairs.is_empty() {
        return None;
    }
    let args: Map<String, Value> = pairs.into_iter().map(|(key, raw)| { let value = arg_value(&raw, &key, props); (key, value) }).collect();
    Some((name.to_owned(), Value::Object(args).to_string()))
}

/// `<parameter name="k">v</parameter>` (or `<parameter=k>`) arguments; also
/// whether each had its close.
fn param_args(text: &str, name: &str, tools: &[Value]) -> (String, bool) {
    let props = props_of(tools, name);
    let mut opens: Vec<(usize, usize, String)> = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("<parameter").map(|at| from + at) {
        let tail = &text[at + "<parameter".len()..];
        let Some(end) = tail.find('>') else { break };
        let head = tail[..end].trim();
        let key = if let Some(named) = head.strip_prefix('=') { named.trim().to_owned() }
            else { head.split_once('=').map(|(_, value)| value.trim().trim_matches('"').to_owned()).unwrap_or_default() };
        if !key.is_empty() {
            opens.push((at, at + "<parameter".len() + end + 1, key));
        }
        from = at + "<parameter".len() + end + 1;
    }
    let mut args = Map::new();
    let mut closed = !opens.is_empty();
    for (index, (_, body_start, key)) in opens.iter().enumerate() {
        let body_end = opens.get(index + 1).map_or(text.len(), |next| next.0);
        let mut value = &text[*body_start..body_end];
        match value.find("</parameter>") {
            Some(close) => value = &value[..close],
            None => closed = false,
        }
        let value = value.strip_prefix("\r\n").or_else(|| value.strip_prefix('\n')).unwrap_or(value);
        let value = value.strip_suffix("\r\n").or_else(|| value.strip_suffix('\n')).unwrap_or(value);
        args.insert(key.clone(), arg_value(value, key, props));
    }
    (Value::Object(args).to_string(), closed)
}

/// Where the JSON object starting at `from` (after blanks) ends: past its
/// closing brace; `None` when not whole yet, `Some(0)` when there's none.
fn object_end(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut at = from;
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    if at >= bytes.len() {
        return None;
    }
    if bytes[at] != b'{' {
        return Some(0);
    }
    let (mut depth, mut string, mut escaped) = (0, false, false);
    while at < bytes.len() {
        let byte = bytes[at];
        if string {
            if escaped { escaped = false } else if byte == b'\\' { escaped = true } else if byte == b'"' { string = false }
        } else if byte == b'"' {
            string = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(at + 1);
            }
        }
        at += 1;
    }
    None
}

/// An opener at or after `from`: `<tool_call>`, `<tool_call name="x">`,
/// `<toolcall>`, or `<tool_call` right before its JSON. (start, length, name).
fn open_at(text: &str, from: usize) -> Option<(usize, usize, Option<String>)> {
    let mut search = from;
    loop {
        let candidates = [text[search..].find("<tool_call"), text[search..].find("<toolcall")];
        let at = candidates.into_iter().flatten().min()? + search;
        let word = if text[at..].starts_with("<tool_call") { "<tool_call".len() } else { "<toolcall".len() };
        let rest = &text[at + word..];
        if let Some(stripped) = rest.strip_prefix('>') {
            let _ = stripped;
            return Some((at, word + 1, None));
        }
        if rest.starts_with(char::is_whitespace) {
            if let Some(end) = rest.find('>') {
                let attrs = &rest[..end];
                if !attrs.contains(['<', '{', '}']) {
                    let name = attrs.find("name").and_then(|position| {
                        let tail = attrs[position + 4..].trim_start().strip_prefix('=')?.trim_start().strip_prefix('"')?;
                        tail.find('"').map(|close| tail[..close].to_owned())
                    });
                    return Some((at, word + end + 1, name));
                }
            }
            if rest.trim_start().starts_with('{') {
                return Some((at, word, None));
            }
        }
        if rest.starts_with('{') {
            return Some((at, word, None));
        }
        search = at + word;
    }
}

/// An opener still being written at the end of the text.
fn open_hold(text: &str) -> Option<usize> {
    for word in ["<tool_call", "<toolcall"] {
        if let Some(at) = text.rfind(word) {
            let tail = &text[at + word.len()..];
            if tail.len() <= 200 && !tail.contains(['<', '>', '{', '}', '\n']) {
                return Some(at);
            }
        }
    }
    None
}

/// The next close, `</tool_call>` or `</toolcall>`, at or after `from`.
fn close_at(text: &str, from: usize) -> Option<(usize, usize)> {
    [text[from..].find(CLOSE).map(|at| (at + from, CLOSE.len())), text[from..].find("</toolcall>").map(|at| (at + from, "</toolcall>".len()))]
        .into_iter().flatten().min_by_key(|(at, _)| *at)
}

/// DeepSeek's DSML tag start (`<｜DSML｜`, `</|DSML|`) at or after `from`:
/// (start, the end of its prefix).
fn dsml_at(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut search = from;
    while let Some(at) = text[search..].find('<').map(|at| at + search) {
        let mut rest = text[at + 1..].trim_start();
        rest = rest.strip_prefix('/').unwrap_or(rest).trim_start();
        if rest.starts_with(['|', '｜']) {
            let after_bars = rest.trim_start_matches(|ch: char| ch == '|' || ch == '｜' || ch.is_whitespace());
            if let Some(after) = after_bars.strip_prefix("DSML") {
                let after = after.trim_start_matches(|ch: char| ch == '|' || ch == '｜' || ch.is_whitespace());
                return Some((at, text.len() - after.len()));
            }
        }
        search = at + 1;
    }
    None
}

/// A DSML invoke's arguments.
fn dsml_args(body: &str) -> String {
    let mut args = Map::new();
    let mut from = 0;
    while let Some((at, prefix_end)) = dsml_at(body, from) {
        let rest = &body[prefix_end..];
        from = prefix_end;
        let Some(head) = rest.strip_prefix("parameter") else { continue };
        let Some(close) = head.find('>') else { break };
        let attrs = &head[..close];
        let Some(name) = attrs.split("name").nth(1).and_then(|tail| tail.trim_start().strip_prefix('=')).and_then(|tail| tail.trim_start().strip_prefix('"'))
            .and_then(|tail| tail.find('"').map(|end| tail[..end].to_owned())) else { continue };
        let value_start = prefix_end + "parameter".len() + close + 1;
        let Some((end, _)) = dsml_at(body, value_start) else { break };
        let raw = &body[value_start..end];
        let is_string = attrs.contains("string=\"true\"") || attrs.contains("string = \"true\"");
        args.insert(name, if is_string { json!(raw) } else { serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!(raw)) });
        from = end;
        let _ = at;
    }
    Value::Object(args).to_string()
}

/// Splits the model's text into what the agent sees and the tool calls it
/// wrote, holding back the start of a block until it is whole.
#[derive(Default)]
struct TextTools {
    buf: String,
    after: bool,
    tools: Vec<Value>,
    sent: String,
}

#[derive(Default, Debug)]
struct Pushed {
    text: String,
    calls: Vec<(String, String)>,
    reasoning: String,
    error: Option<&'static str>,
}

enum Block {
    Wait,
    Text,
    Error(&'static str),
    Call((String, String), String),
}

impl TextTools {
    /// What is left after a call: blanks, closes, DeepSeek's tags but an invoke.
    fn strip_leftover(&mut self) {
        loop {
            let trimmed = self.buf.trim_start();
            let mut next = trimmed;
            for tag in ["</tool_calls>", "</tool_call>", "</toolcalls>", "</toolcall>", "<tool_calls>", "<toolcalls>"] {
                if let Some(rest) = next.strip_prefix(tag) {
                    next = rest;
                }
            }
            if next.len() == trimmed.len() {
                if let Some((0, prefix_end)) = dsml_at(trimmed, 0) {
                    if !trimmed[prefix_end..].starts_with("invoke") {
                        if let Some(close) = trimmed[prefix_end..].find('>') {
                            next = &trimmed[prefix_end + close + 1..];
                        }
                    }
                }
            }
            if next.len() == self.buf.len() {
                return;
            }
            self.buf = next.to_owned();
        }
    }

    fn push(&mut self, input: &str, end: bool) -> Pushed {
        self.buf.push_str(input);
        let mut out = Pushed::default();
        loop {
            if self.after {
                self.strip_leftover();
                if self.buf.is_empty() || !end && MARKS.iter().any(|mark| mark.starts_with(self.buf.as_str())) {
                    break;
                }
                self.after = false;
            }
            let open = open_at(&self.buf, 0);
            let dsml = dsml_at(&self.buf, 0);
            let reason = self.buf.find("{\"reasoning_content\"").or_else(|| {
                let at = self.buf.find('{')?;
                self.buf[at + 1..].trim_start().starts_with("\"reasoning_content\"").then_some(at)
            });
            let first = [open.as_ref().map(|(at, ..)| *at), dsml.map(|(at, _)| at), reason].into_iter().flatten().min();
            let Some(first) = first else { break };
            if Some(first) == reason {
                match object_end(&self.buf, first) {
                    None if !end => {
                        out.text.push_str(&self.buf[..first]);
                        self.buf = self.buf[first..].to_owned();
                        break;
                    }
                    Some(close) if close > 0 => {
                        if let Some(text) = loose_json(&self.buf[first..close]).and_then(|value| value.get("reasoning_content").and_then(Value::as_str).map(str::to_owned)) {
                            out.text.push_str(&self.buf[..first]);
                            out.reasoning.push_str(&text);
                            self.buf = self.buf[close..].to_owned();
                            continue;
                        }
                    }
                    _ => {}
                }
                out.text.push_str(&self.buf[..=first]);
                self.buf = self.buf[first + 1..].to_owned();
                continue;
            }
            if Some(first) == dsml.map(|(at, _)| at) {
                let (at, prefix_end) = dsml.expect("a DSML tag");
                out.text.push_str(&self.buf[..at]);
                let rest = self.buf[prefix_end..].to_owned();
                self.buf = self.buf[at..].to_owned();
                let prefix = prefix_end - at;
                let Some(tag_end) = rest.find('>') else {
                    let next = self.buf[1..].find('<').map(|next| next + 1);
                    if next.is_none() && !end {
                        break;
                    }
                    self.buf = next.map_or_else(String::new, |next| self.buf[next..].to_owned());
                    continue;
                };
                let tag = &rest[..tag_end];
                let invoke = tag.strip_prefix("invoke").and_then(|attrs| attrs.split("name").nth(1)).and_then(|tail| tail.trim_start().strip_prefix('='))
                    .and_then(|tail| tail.trim_start().strip_prefix('"')).and_then(|tail| tail.find('"').map(|close| tail[..close].to_owned()));
                let Some(name) = invoke.filter(|_| !self.buf[1..].trim_start().starts_with('/')) else {
                    self.buf = self.buf[prefix + tag_end + 1..].to_owned();
                    self.after = true;
                    continue;
                };
                let body = self.buf[prefix + tag_end + 1..].to_owned();
                let mut stop = None;
                let mut close_len = 0;
                let mut search = 0;
                while let Some((at, prefix_end)) = dsml_at(&body, search) {
                    let tail = &body[prefix_end..];
                    if body[at + 1..].trim_start().starts_with('/') && tail.starts_with("invoke") {
                        stop = Some(at);
                        close_len = prefix_end - at + tail.find('>').map_or(0, |end| end + 1);
                        break;
                    }
                    if tail.starts_with("invoke") {
                        stop = Some(at);
                        break;
                    }
                    search = prefix_end;
                }
                let stop = match stop {
                    Some(stop) => stop,
                    None if !end => break,
                    None => body.len(),
                };
                out.calls.push((name, dsml_args(&body[..stop])));
                self.buf = body[(stop + close_len).min(body.len())..].to_owned();
                self.after = true;
                continue;
            }
            let (at, length, named) = open.expect("an opener");
            let head = self.buf[..at].to_owned();
            let body = self.buf[at + length..].to_owned();
            if quoted(&format!("{}{}{head}", self.sent, out.text)) {
                out.text.push_str(&self.buf[..at + length]);
                self.buf = body;
                continue;
            }
            let inner = body.trim_start();
            if named.is_none() && (inner.starts_with('<') || end && inner.is_empty()) {
                out.text.push_str(&head);
                self.buf = inner.to_owned();
                self.after = true;
                continue;
            }
            match self.block(&body, named.as_deref(), end) {
                Block::Wait => break,
                Block::Text => {
                    out.text.push_str(&self.buf[..at + length]);
                    self.buf = body;
                }
                Block::Error(error) => {
                    out.text.push_str(&head);
                    self.buf.clear();
                    self.sent.push_str(&out.text);
                    out.error = Some(error);
                    return out;
                }
                Block::Call(call, rest) => {
                    out.text.push_str(&head);
                    out.calls.push(call);
                    self.buf = rest;
                    self.after = true;
                }
            }
        }
        if end {
            if let Some(hold) = open_hold(&self.buf).filter(|hold| !quoted(&format!("{}{}{}", self.sent, out.text, &self.buf[..*hold]))) {
                out.text.push_str(&self.buf[..hold]);
                self.buf.clear();
                self.sent.push_str(&out.text);
                out.error = Some(CUT_SHORT);
                return out;
            }
            out.text.push_str(&std::mem::take(&mut self.buf));
        } else {
            let starts = [open_at(&self.buf, 0).map(|(at, ..)| at), open_hold(&self.buf), dsml_at(&self.buf, 0).map(|(at, _)| at), self.buf.find("{\"reasoning_content\"")];
            let mut keep = starts.into_iter().flatten().min().map_or(0, |at| self.buf.len() - at);
            if keep == 0 {
                for count in (1..=self.buf.len().min(20)).rev() {
                    if !self.buf.is_char_boundary(self.buf.len() - count) {
                        continue;
                    }
                    let tail = &self.buf[self.buf.len() - count..];
                    if MARKS.iter().any(|mark| mark.starts_with(tail)) {
                        keep = count;
                        break;
                    }
                }
            }
            let split = self.buf.len() - keep;
            out.text.push_str(&self.buf[..split]);
            self.buf = self.buf[split..].to_owned();
        }
        self.sent.push_str(&out.text);
        out
    }

    /// What follows an opener: a call and what is after it, a wait while it
    /// may still become one, text when it is no call, an error when it can't be read.
    fn block(&self, body: &str, named: Option<&str>, end: bool) -> Block {
        if !end && body.trim().is_empty() {
            return Block::Wait;
        }
        let close = close_at(body, 0);
        let region = |close: Option<(usize, usize)>| close.map_or(body, |(at, _)| &body[..at]);
        let rest = |close: Option<(usize, usize)>| close.map_or_else(String::new, |(at, length)| body[at + length..].to_owned());
        let failed = |close: Option<(usize, usize)>| Block::Error(if close.is_some() { UNREADABLE } else { CUT_SHORT });
        if let Some(name) = named {
            if close.is_none() && !end {
                return Block::Wait;
            }
            let inside = region(close);
            if inside.trim_start().starts_with('{') {
                return match repaired_json(inside.trim()) {
                    Some(value) if value.get("name").and_then(Value::as_str).is_some() => {
                        Block::Call((value["name"].as_str().unwrap_or(name).to_owned(), call_args(&value)), rest(close))
                    }
                    Some(value) => Block::Call((name.to_owned(), Value::Object(value).to_string()), rest(close)),
                    None => failed(close),
                };
            }
            if inside.contains("<arg_key>") {
                return glm_call(&format!("{name}{inside}"), &self.tools).map_or_else(|| failed(close), |call| Block::Call(call, rest(close)));
            }
            let (args, closed) = param_args(inside, name, &self.tools);
            if close.is_none() && !closed {
                return Block::Error(CUT_SHORT);
            }
            return Block::Call((name.to_owned(), args), rest(close));
        }
        if body.trim_start().starts_with('{') {
            // DeepSeek's half-and-half: the name as JSON, then its parameters
            let trimmed = body.trim_start();
            if let Some(head) = trimmed.strip_prefix('{').map(str::trim_start).and_then(|rest| rest.strip_prefix("\"name\"")) {
                let after = head.trim_start().strip_prefix(':').map(str::trim_start).and_then(|rest| rest.strip_prefix('"'));
                if let Some((name, tail)) = after.and_then(|rest| rest.find('"').map(|close| (&rest[..close], &rest[close + 1..]))) {
                    let tail = tail.trim_start().trim_start_matches('}').trim_start().trim_start_matches('>').trim_start();
                    if tail.starts_with("<parameter") {
                        if close.is_none() && !end {
                            return Block::Wait;
                        }
                        let start = body.len() - tail.len();
                        let inside = close.map_or(&body[start..], |(at, _)| &body[start..at]);
                        let (args, closed) = param_args(inside, name, &self.tools);
                        if close.is_none() && !closed {
                            return Block::Error(CUT_SHORT);
                        }
                        return Block::Call((name.to_owned(), args), rest(close));
                    }
                }
            }
            if let Some(object) = object_end(body, 0).filter(|at| *at > 0) {
                if let Some(value) = call_json(&body[..object]) {
                    let tail = &body[object..];
                    let junk = tail.len() - tail.trim_start_matches(|ch: char| ch.is_whitespace() || "}]\"'>".contains(ch)).len();
                    let next = &tail[junk..];
                    let call = (value["name"].as_str().unwrap_or_default().to_owned(), call_args(&value));
                    if value.len() == 1 && !end && "<parameter".starts_with(next.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '>')) {
                        return Block::Wait;
                    }
                    if !end && (next.is_empty() || [CLOSE, "</toolcall>"].iter().any(|close| close.starts_with(next))) {
                        return Block::Wait;
                    }
                    if let Some((0, length)) = close_at(next, 0) {
                        return Block::Call(call, next[length..].to_owned());
                    }
                    if !tail[..junk].contains(|ch: char| !ch.is_whitespace()) && !next.starts_with(|ch: char| !ch.is_whitespace() && ch != '<') {
                        return Block::Call(call, tail.to_owned());
                    }
                    if close.is_none() && end {
                        return Block::Call(call, tail.to_owned());
                    }
                }
            }
            let mut search = close;
            while let Some((at, length)) = search {
                if let Some(value) = call_json(&body[..at]) {
                    return Block::Call((value["name"].as_str().unwrap_or_default().to_owned(), call_args(&value)), body[at + length..].to_owned());
                }
                search = close_at(body, at + 1);
            }
            if !end {
                return Block::Wait;
            }
            return call_json(body).map_or_else(|| failed(close), |value| Block::Call((value["name"].as_str().unwrap_or_default().to_owned(), call_args(&value)), String::new()));
        }
        // GLM's own (a name, then arg pairs), or text
        let word: String = body.trim_start().chars().take_while(|ch| ch.is_alphanumeric() || "_.:-".contains(*ch)).collect();
        let after_word = body.trim_start()[word.len()..].trim_start();
        if word.is_empty() || word.len() > 128 || !(after_word.is_empty() || after_word.starts_with('<')) {
            return Block::Text;
        }
        if close.is_none() && !end {
            return Block::Wait;
        }
        if let Some(call) = glm_call(region(close), &self.tools) {
            return Block::Call(call, rest(close));
        }
        if after_word.starts_with("<arg_key>") {
            return failed(close);
        }
        Block::Text
    }
}

/// The request's own name for a tool a model named otherwise.
fn tool_named(name: &str, tools: &[Value]) -> String {
    let names: Vec<&str> = tools.iter().filter_map(|tool| tool["function"]["name"].as_str().or(tool["name"].as_str())).collect();
    if names.is_empty() || names.contains(&name) {
        return name.to_owned();
    }
    let norm = |text: &str| text.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    names.iter().find(|known| norm(known) == norm(name)).map_or_else(|| name.to_owned(), |known| (*known).to_owned())
}

/// A call's arguments with a layer a model put around them taken off.
fn unwrapped(args: &str, name: &str, tools: &[Value]) -> String {
    let Some(props) = tools.iter().map(|tool| if tool["function"].is_object() { &tool["function"] } else { tool }).find(|function| function["name"] == name)
        .map(|function| function["parameters"]["properties"].as_object().cloned().unwrap_or_default()) else { return args.to_owned() };
    let Some(mut value) = loose_json(args) else { return args.to_owned() };
    let mut changed = false;
    for _ in 0..3 {
        let keys: Vec<&String> = value.keys().collect();
        if keys.len() != 1 || !["arguments", "input", "parameters", "args"].contains(&keys[0].as_str()) || props.contains_key(keys[0]) {
            break;
        }
        let inner = value.values().next().cloned().unwrap_or(Value::Null);
        let inner = match inner {
            Value::String(text) => loose_json(&text),
            Value::Object(object) => Some(object),
            _ => None,
        };
        let Some(inner) = inner else { break };
        value = inner;
        changed = true;
    }
    if changed { Value::Object(value).to_string() } else { args.to_owned() }
}

fn done(args: &str) -> bool {
    !args.is_empty() && serde_json::from_str::<Value>(args).is_ok()
}

/// The tool calls Trae streams, put together by id or index.
#[derive(Default)]
struct NativeCalls {
    calls: Vec<(String, Option<u64>, String, String)>,
}

impl NativeCalls {
    fn push(&mut self, call: &Value) {
        let function = if call["function"].is_object() { &call["function"] } else if call["function_call"].is_object() { &call["function_call"] } else { call };
        let name = function["name"].as_str().or(call["tool_name"].as_str()).unwrap_or_default().to_owned();
        let index = call["index"].as_u64();
        let raw = [&function["arguments"], &function["args"], &call["params"], &call["input"], &call["parameters"]].into_iter().find(|value| !value.is_null());
        let args = match raw {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let id = call["id"].as_str().or(call["tool_call_id"].as_str()).unwrap_or_default().to_owned();
        if name.is_empty() && index.is_none() && id.is_empty() && args.is_empty() {
            return;
        }
        let found = self.calls.iter().position(|(known, at, ..)| !id.is_empty() && *known == id || id.is_empty() && index.is_some() && *at == index)
            .or_else(|| (name.is_empty() && id.is_empty()).then(|| self.calls.iter().rposition(|(.., args)| !done(args))).flatten());
        let Some(found) = found else {
            if !name.is_empty() {
                self.calls.push((id, index, name, args));
            }
            return;
        };
        let entry = &mut self.calls[found];
        if entry.2.is_empty() {
            entry.2 = name;
        }
        if args.is_empty() || entry.3.starts_with(&args) {
            return;
        }
        if args.starts_with(&entry.3) || done(&entry.3) {
            entry.3 = args;
        } else {
            entry.3.push_str(&args);
        }
    }

    fn take(&mut self, tools: &[Value]) -> Vec<(String, String, String)> {
        std::mem::take(&mut self.calls).into_iter().filter(|(.., name, _)| !name.is_empty()).map(|(id, _, mut name, mut args)| {
            let held = if name.trim_start().starts_with('{') { loose_json(&name).map(|value| (value["name"].as_str().unwrap_or_default().to_owned(), call_args(&value))) }
                else if name.contains("<arg_key>") { glm_call(&name, tools) } else { None };
            if let Some((held_name, held_args)) = held.filter(|(held_name, _)| !held_name.is_empty()) {
                name = held_name;
                if args.trim().is_empty() || args.trim() == "{}" {
                    args = held_args;
                }
            }
            let args = if done(&args) { args } else { loose_json(&args).map_or_else(|| if args.is_empty() { "{}".into() } else { args.clone() }, |value| Value::Object(value).to_string()) };
            (if id.is_empty() { format!("call_{}", oauth::random_hex(12)) } else { id }, name, args)
        }).collect()
    }
}

/// Reasoning sent on so far: a piece Trae sends again sends only what is new.
#[derive(Default)]
struct Thoughts {
    all: String,
    last: String,
}

impl Thoughts {
    fn add(&mut self, piece: &str) -> String {
        let last = std::mem::replace(&mut self.last, piece.to_owned());
        if piece == last && piece.len() >= 8 || self.all.len() >= 16 && piece.len() >= 16 && self.all.ends_with(piece) {
            return String::new();
        }
        let new = if self.all.len() >= 16 && piece.starts_with(&self.all) { &piece[self.all.len()..] } else { piece };
        self.all.push_str(new);
        new.to_owned()
    }
}

/// What Trae's answer is made of, or a failure with Trae's code.
#[derive(Debug, PartialEq)]
enum Part {
    Event(Event),
    Failed(String, String),
}

/// Reads Trae's SSE into the answer's parts.
struct Parser {
    buffer: String,
    event: String,
    data: Vec<String>,
    tools: Vec<Value>,
    text: TextTools,
    native: NativeCalls,
    thoughts: Thoughts,
    finish: &'static str,
    calls: usize,
    over: bool,
}

/// `TokenUsage`, `token-usage` → `token_usage`.
fn event_name(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 && (chars[index - 1].is_ascii_lowercase() || chars[index - 1].is_ascii_digit()) {
            out.push('_');
        }
        out.push(if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' });
    }
    out.trim_matches('_').to_owned()
}

fn usage_of(value: &Value) -> Option<Event> {
    let number = |keys: &[&str]| keys.iter().find_map(|key| value[*key].as_u64()).unwrap_or(0);
    let input = number(&["prompt_tokens", "input_tokens", "promptTokens", "inputTokens"]);
    let output = number(&["completion_tokens", "output_tokens", "completionTokens", "outputTokens"]);
    if input == 0 && output == 0 {
        return None;
    }
    let cached = value["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or_else(|| number(&["cache_read_input_tokens", "cacheReadInputTokens"]));
    let reasoning = value["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or_else(|| number(&["reasoning_tokens"]));
    Some(Event::Usage { input, output, cached, reasoning })
}

impl Parser {
    fn new(tools: Vec<Value>) -> Self {
        Parser { buffer: String::new(), event: String::new(), data: Vec::new(), text: TextTools { tools: tools.clone(), ..Default::default() },
            tools, native: NativeCalls::default(), thoughts: Thoughts::default(), finish: "", calls: 0, over: false }
    }

    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Part>) {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        while let Some(end) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=end).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                self.flush(out);
            } else if line.starts_with(':') {
                continue;
            } else if let Some(name) = line.strip_prefix("event:") {
                self.flush(out);
                self.event = name.trim().to_owned();
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data.push(data.trim_start().to_owned());
            }
        }
    }

    fn flush(&mut self, out: &mut Vec<Part>) {
        if self.data.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.data).join("\n");
        let event = std::mem::take(&mut self.event);
        if self.over {
            return;
        }
        let data: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw.clone()));
        let name = event_name(&if event.is_empty() { data["event"].as_str().or(data["type"].as_str()).unwrap_or_default().to_owned() } else { event });
        self.handle(&name, &data, out);
    }

    fn call(&mut self, name: &str, args: &str, id: String, out: &mut Vec<Part>) {
        let name = tool_named(name, &self.tools);
        let args = unwrapped(args, &name, &self.tools);
        self.calls += 1;
        out.push(Part::Event(Event::ToolStart { id: id.clone(), name }));
        out.push(Part::Event(Event::ToolArgs { id, text: args }));
    }

    fn pushed(&mut self, pushed: Pushed, out: &mut Vec<Part>) {
        if !pushed.reasoning.is_empty() {
            let new = self.thoughts.add(&pushed.reasoning);
            if !new.is_empty() {
                out.push(Part::Event(Event::Reasoning(new)));
            }
        }
        if !pushed.text.is_empty() {
            out.push(Part::Event(Event::Text(pushed.text)));
        }
        for (name, args) in pushed.calls {
            self.call(&name, &args, format!("call_{}", oauth::random_hex(12)), out);
        }
        if let Some(error) = pushed.error {
            self.over = true;
            out.push(Part::Failed(String::new(), error.into()));
        }
    }

    fn handle(&mut self, name: &str, data: &Value, out: &mut Vec<Part>) {
        if data == "[DONE]" {
            return self.end(out);
        }
        let empty = Value::Null;
        let d = if data.is_object() { data } else { &empty };
        if name == "error" || name != "output" && !d["code"].is_null() && !d["message"].is_null() && d["response"].is_null() && d["content"].is_null() {
            let (code, message) = error_of(d);
            self.over = true;
            out.push(Part::Failed(code, if message.is_empty() { "Trae returned an error".into() } else { message }));
            return;
        }
        if name == "token_usage" {
            if let Some(usage) = usage_of(if d["usage"].is_object() { &d["usage"] } else { d }) {
                out.push(Part::Event(usage));
            }
            return;
        }
        if matches!(name, "done" | "response_done" | "stream_done") {
            if let Some(usage) = usage_of(&d["usage"]) {
                out.push(Part::Event(usage));
            }
            self.finish = finish_of(d).unwrap_or(self.finish);
            return self.end(out);
        }
        if !["", "output", "message", "delta", "content", "text", "answer", "chunk", "output_delta", "message_delta"].contains(&name) {
            return;
        }
        let delta = if d["delta"].is_object() { &d["delta"] } else { &empty };
        if let Some(reasoning) = [&d["reasoning_content"], &d["reasoning"], &delta["reasoning_content"]].into_iter().find_map(Value::as_str).filter(|text| !text.is_empty()) {
            let new = self.thoughts.add(reasoning);
            if !new.is_empty() {
                out.push(Part::Event(Event::Reasoning(new)));
            }
        }
        let text = [&d["response"], &d["content"], &delta["content"], &d["delta"]].into_iter().find_map(Value::as_str).unwrap_or_default();
        let text = if text.starts_with("Building prompt:") || text.starts_with("Completed building prompt") { "" } else { text };
        if !text.is_empty() {
            let pushed = self.text.push(text, false);
            self.pushed(pushed, out);
            if self.over {
                return;
            }
        }
        for call in d["tool_calls"].as_array().into_iter().flatten() {
            self.native.push(call);
        }
        if let Some(usage) = usage_of(&d["usage"]) {
            out.push(Part::Event(usage));
        }
        self.finish = finish_of(d).unwrap_or(self.finish);
    }

    fn end(&mut self, out: &mut Vec<Part>) {
        if self.over {
            return;
        }
        let pushed = self.text.push("", true);
        self.pushed(pushed, out);
        if self.over {
            return;
        }
        self.over = true;
        for (id, name, args) in self.native.take(&self.tools) {
            self.call(&name, &args, id, out);
        }
        let finish = if self.calls > 0 { "tool_calls" } else if self.finish.is_empty() { "stop" } else { self.finish };
        out.push(Part::Event(Event::Finish(finish)));
    }

    fn finish(&mut self, out: &mut Vec<Part>) {
        let rest = std::mem::take(&mut self.buffer);
        if !rest.is_empty() {
            self.feed(format!("{rest}\n").as_bytes(), out);
        }
        self.flush(out);
        self.end(out);
    }
}

fn finish_of(value: &Value) -> Option<&'static str> {
    let finish = [&value["finish_reason"], &value["stop_reason"], &value["finishReason"]].into_iter().find_map(Value::as_str)?.to_ascii_lowercase();
    match finish.as_str() {
        "length" | "max_tokens" | "max_output_tokens" | "max_length" => Some("length"),
        "content_filter" | "sensitive" => Some("content_filter"),
        _ => None,
    }
}

// ---- the exchange --------------------------------------------------------------

/// A model or function this account's chat function doesn't take: another may.
fn wrong_function(code: &str) -> bool {
    matches!(code, "4001" | "4023" | "1005")
}

fn error_response(status: u16, message: &str) -> reqwest::Response {
    let kind = if status == 429 { "rate_limit_error" } else { "api_error" };
    let body = json!({"error": {"message": message, "type": kind, "code": null}}).to_string();
    reqwest::Response::from(hyper::Response::builder().status(status).header(hyper::header::CONTENT_TYPE, "application/json").body(body).expect("response parts"))
}

/// The response for an error Trae answered with.
fn failure(site: &Site, status: u16, code: &str, message: &str) -> reqwest::Response {
    let message = if message.is_empty() { format!("HTTP {status}") } else { message.to_owned() };
    let number: i64 = code.parse().unwrap_or(0);
    let lower = message.to_lowercase();
    let lapsed = ["not login", "not logged in", "unauthorized", "token expired", "token is expired", "token invalid", "jwt", "未登录", "登录过期", "登录已过期", "登录失效"];
    if status == 401 || site.lapsed.contains(&number) || status == 403 && lapsed.iter().any(|word| lower.contains(word)) {
        return error_response(401, &format!("{}'s sign-in has expired ({message}); sign in again", site.realm));
    }
    let quota = ["quota", "credit", "insufficient", "exceed", "limit", "额度", "积分", "次数", "上限", "用完"];
    if status == 429 || site.quota.contains(&number) || status == 403 && quota.iter().any(|word| lower.contains(word)) {
        return error_response(429, &format!("{}: {message}", site.realm));
    }
    let suffix = if code.is_empty() { String::new() } else { format!(" (code {code})") };
    error_response(if status >= 400 { status } else { 502 }, &format!("{}: {message}{suffix}", site.realm))
}

/// A chat answered by Trae: through the function that lists the model, then
/// the others, an error before the answer moving on to the next.
pub struct Exchange {
    client: Client,
    site: Site,
    creds: Creds,
    chat: Value,
    model: String,
    route: Option<Route>,
}

pub fn exchange(client: &Client, site: &Site, creds: &Creds, body: &[u8], model: &str) -> Result<Exchange, (hyper::StatusCode, &'static str)> {
    let chat: Value = serde_json::from_slice(body).map_err(|_| (hyper::StatusCode::BAD_REQUEST, "invalid chat request"))?;
    let model = chat["model"].as_str().filter(|model| !model.is_empty()).unwrap_or(model).to_owned();
    let route = routes().lock().unwrap().get(&creds.uid).and_then(|routes| routes.get(&model).cloned());
    Ok(Exchange { client: client.clone(), site: site.clone(), creds: creds.clone(), chat, model, route })
}

/// Each account's routes by model, from its lists.
fn routes() -> &'static Mutex<HashMap<String, HashMap<String, Route>>> {
    static ROUTES: std::sync::OnceLock<Mutex<HashMap<String, HashMap<String, Route>>>> = std::sync::OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// The function that served each account last.
fn last_function() -> &'static Mutex<HashMap<String, String>> {
    static LAST: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    LAST.get_or_init(Default::default)
}

impl Exchange {
    pub fn url(&self) -> String {
        format!("{}/api/agent/v3/llm_utils_chat", api_of(&self.site, &self.creds))
    }

    /// The Chat reply: SSE or one completion, or Trae's refusal.
    pub async fn run(self) -> reqwest::Response {
        let who = if self.creds.uid.is_empty() { self.creds.name.clone() } else { self.creds.uid.clone() };
        let mut functions: Vec<String> = Vec::new();
        for function in [self.route.as_ref().map(|route| route.function.clone()), last_function().lock().unwrap().get(&who).cloned()].into_iter().flatten()
            .chain(FUNCTIONS.iter().map(|function| (*function).to_owned())) {
            if !functions.contains(&function) {
                functions.push(function);
            }
        }
        let session = conversation_id(&self.chat, &who);
        let stream = self.chat["stream"] == true;
        let tools = self.chat["tools"].as_array().cloned().unwrap_or_default();
        let mut last = None;
        for function in functions {
            let body = chat_body(&self.chat, &function, self.route.as_ref(), &session);
            let mut headers = ide_headers(&self.creds);
            headers.push(("accept", "text/event-stream".into()));
            let sent = request(&self.client, &self.url(), &headers).body(body.to_string()).send().await;
            let response = match sent {
                Ok(response) => response,
                Err(_) => return error_response(502, &format!("{}: cannot reach Trae", self.site.realm)),
            };
            let status = response.status().as_u16();
            let json_body = response.headers().get(hyper::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("json") && !value.contains("event-stream"));
            if !(200..300).contains(&status) || json_body {
                let value: Value = response.json().await.unwrap_or(Value::Null);
                let (code, message) = error_of(&value);
                last = Some(failure(&self.site, if json_body { 502 } else { status }, &code, &message));
                if (status == 400 || json_body) && wrong_function(&code) {
                    continue;
                }
                break;
            }
            let mut parser = Parser::new(tools.clone());
            let mut upstream = response.bytes_stream();
            let mut held: Vec<Part> = Vec::new();
            let mut ended = false;
            while !held.iter().any(|part| !matches!(part, Part::Event(Event::Usage { .. }))) {
                match upstream.next().await {
                    Some(Ok(chunk)) => parser.feed(&chunk, &mut held),
                    Some(Err(_)) => {
                        held.push(Part::Failed(String::new(), "Trae's stream broke off".into()));
                        break;
                    }
                    None => {
                        parser.finish(&mut held);
                        ended = true;
                        break;
                    }
                }
            }
            if let Some(Part::Failed(code, message)) = held.iter().find(|part| matches!(part, Part::Failed(..))) {
                last = Some(failure(&self.site, 502, code, message));
                if wrong_function(code) {
                    continue;
                }
                break;
            }
            last_function().lock().unwrap().insert(who.clone(), function);
            return answer(&self.model, stream, held, if ended { None } else { Some((upstream, parser)) }).await;
        }
        last.unwrap_or_else(|| error_response(502, "Trae served no chat function"))
    }
}

fn events_of(parts: Vec<Part>) -> Vec<Event> {
    parts.into_iter().map(|part| match part {
        Part::Event(event) => event,
        Part::Failed(_, message) => Event::Error(format!("Trae: {message}")),
    }).collect()
}

type Upstream = std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// The answer as Chat: one completion, or a stream of what was held and
/// what comes after.
async fn answer(model: &str, stream: bool, held: Vec<Part>, rest: Option<(impl futures_util::Stream<Item = reqwest::Result<Bytes>> + Send + 'static, Parser)>) -> reqwest::Response {
    if !stream {
        let mut parts = held;
        if let Some((upstream, mut parser)) = rest {
            let mut upstream = Box::pin(upstream);
            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(chunk) => parser.feed(&chunk, &mut parts),
                    Err(_) => break,
                }
            }
            parser.finish(&mut parts);
        }
        if let Some(Part::Failed(_, message)) = parts.iter().find(|part| matches!(part, Part::Failed(..))) {
            return error_response(502, &format!("Trae: {message}"));
        }
        let body = wire::Writer::new(model, false).collect(events_of(parts));
        return reqwest::Response::from(hyper::Response::builder().status(200).header(hyper::header::CONTENT_TYPE, "application/json")
            .body(body.to_string()).expect("response parts"));
    }
    let mut writer = wire::Writer::new(model, true);
    let first = writer.write(events_of(held));
    let rest: Option<(Upstream, Parser)> = rest.map(|(upstream, parser)| (Box::pin(upstream) as Upstream, parser));
    let state = (Some(first), rest, writer);
    let body = futures_util::stream::unfold(Some(state), |state| async move {
        let (first, mut rest, mut writer) = state?;
        if let Some(first) = first.filter(|first| !first.is_empty()) {
            return Some((Ok::<_, std::io::Error>(Bytes::from(first)), Some((None, rest, writer))));
        }
        let Some((upstream, parser)) = rest.as_mut() else {
            return Some((Ok(Bytes::from(writer.done())), None));
        };
        loop {
            let mut parts = Vec::new();
            match upstream.next().await {
                Some(Ok(chunk)) => parser.feed(&chunk, &mut parts),
                Some(Err(_)) => parts.push(Part::Failed(String::new(), "Trae's stream broke off".into())),
                None => {
                    parser.finish(&mut parts);
                    let mut out = writer.write(events_of(parts));
                    out.push_str(&writer.done());
                    return Some((Ok(Bytes::from(out)), None));
                }
            }
            let out = writer.write(events_of(parts));
            if !out.is_empty() {
                return Some((Ok(Bytes::from(out)), Some((None, rest, writer))));
            }
        }
    });
    reqwest::Response::from(hyper::Response::builder().status(200).header(hyper::header::CONTENT_TYPE, "text/event-stream")
        .body(reqwest::Body::wrap_stream(body)).expect("response parts"))
}

// ---- models -----------------------------------------------------------------------

/// An entry of a function's list that is a model to chat with.
fn chat_model(entry: &Value, site: &Site) -> bool {
    let id = text(&entry["config_name"]);
    if id.is_empty() || id.to_ascii_lowercase().starts_with("custom_model") {
        return false;
    }
    if entry["usage"].as_str().is_some_and(|usage| usage != "chat_completion") || entry["config_switch"] == false {
        return false;
    }
    if entry["is_custom_model"] == true || entry["display_config"]["is_custom_model"] == true {
        return false;
    }
    !(site.kind == SourceKind::TraeGlobal && entry["is_invisible_to_user"] == true)
}

fn dev_model(entry: &Value) -> Option<&Value> {
    entry["model_detail_list"].as_array()?.iter().find(|detail| detail["model_name"].as_str().is_some_and(|name| name.ends_with("__dev")))
}

fn name_of(entry: &Value) -> String {
    [&entry["display_config"]["display_name"], &entry["display_name"], &entry["display_model_name"], &entry["config_name"]].into_iter()
        .map(text).find(|name| !name.is_empty()).unwrap_or_default()
}

async fn post(client: &Client, url: &str, headers: &[(&'static str, String)], body: &Value) -> Result<(u16, Value), String> {
    let response = request(client, url, headers).header("accept", "application/json").body(body.to_string()).timeout(Duration::from_secs(20)).send().await
        .map_err(|_| "cannot reach Trae".to_owned())?;
    let status = response.status().as_u16();
    Ok((status, response.json().await.unwrap_or(Value::Null)))
}

/// Every chat function's model list, as TRAE SOLO CN asks them, else one
/// function at a time.
async fn lists(client: &Client, site: &Site, creds: &Creds) -> Result<Vec<(String, Vec<Value>)>, String> {
    let api = api_of(site, creds);
    let headers = ide_headers(creds);
    let lapsed = |status: u16, value: &Value| status == 401 || site.lapsed.contains(&error_of(value).0.parse().unwrap_or(0));
    let batch = json!({"functions": FUNCTIONS, "agent_type": "", "current_config_info": {"config_name": "", "is_custom_model": false},
        "mode_type": 0, "access_type": 0, "ab_force_vids": "", "ab_autotest_advanced_mode": 0, "show_custom_model": true});
    if let Ok((status, value)) = post(client, &format!("{api}/api/ide/v1/batch_get_detail_param"), &headers, &batch).await {
        if lapsed(status, &value) {
            return Err("accountSources.errors.signedOut".into());
        }
        let groups = value["function_configs"].as_array().or(value["data"]["function_configs"].as_array()).cloned().unwrap_or_default();
        let mut out: Vec<(String, Vec<Value>)> = Vec::new();
        for group in groups {
            let function = text(&group["function"]);
            if !FUNCTIONS.contains(&function.as_str()) {
                continue;
            }
            let entries: Vec<Value> = group["config_info_list"].as_array().into_iter().flatten().filter(|entry| !text(&entry["config_name"]).is_empty()).cloned().collect();
            match out.iter_mut().find(|(known, _)| *known == function) {
                Some((_, kept)) => kept.extend(entries),
                None => out.push((function, entries)),
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    let mut out = Vec::new();
    let mut failure = None;
    for function in FUNCTIONS {
        let body = json!({"function": function, "config_names": null, "need_prompt": false, "current_config_info": null, "poly_prompt": true, "mode_type": null, "agent_type": null});
        match post(client, &format!("{api}/api/ide/v1/get_detail_param"), &headers, &body).await {
            Ok((status, value)) if lapsed(status, &value) => return Err("accountSources.errors.signedOut".into()),
            Ok((200, value)) => {
                let entries = value["config_info_list"].as_array().or(value["data"]["config_info_list"].as_array()).cloned().unwrap_or_default();
                out.push((function.to_owned(), entries.into_iter().filter(|entry| !text(&entry["config_name"]).is_empty()).collect()));
            }
            Ok((status, _)) => failure = Some(format!("{} models: HTTP {status}", site.realm)),
            Err(error) => failure = Some(error),
        }
    }
    if out.is_empty() {
        return Err(failure.unwrap_or_else(|| format!("{} listed no models", site.realm)));
    }
    Ok(out)
}

/// The account's models, each asked through the function whose entry for it
/// is worth most (names a __dev model, isn't misnamed, carries the name the
/// lists agree on), and a model's Max mode as one of its own.
pub async fn fetch_models(client: &Client, site: &Site, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let lists = lists(client, site, creds).await?;
    let mut names: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for (_, entries) in &lists {
        for entry in entries.iter().filter(|entry| chat_model(entry, site)) {
            *names.entry(text(&entry["config_name"])).or_default().entry(name_of(entry)).or_default() += 1;
        }
    }
    let agreed: HashMap<String, String> = names.iter().map(|(id, by)| {
        let top = by.values().max().copied().unwrap_or(0);
        let most: Vec<&String> = by.iter().filter(|(_, count)| **count == top).map(|(name, _)| name).collect();
        (id.clone(), if most.len() == 1 { most[0].clone() } else { String::new() })
    }).collect();
    let misnamed = |entries: &[Value], entry: &Value| {
        let id = text(&entry["config_name"]);
        let name = name_of(entry);
        names.get(&id).is_some_and(|by| by.len() > 1) && entries.iter().any(|other| chat_model(other, site) && text(&other["config_name"]) != id && name_of(other) == name)
    };
    let mut best: Vec<(String, Value, String, u8)> = Vec::new();
    for (function, entries) in &lists {
        for entry in entries.iter().filter(|entry| chat_model(entry, site)) {
            let id = text(&entry["config_name"]);
            let worth = if dev_model(entry).is_some() { 2 } else { 0 } + u8::from(!misnamed(entries, entry))
                + u8::from(agreed.get(&id).is_some_and(|name| *name == name_of(entry)));
            match best.iter_mut().find(|(known, ..)| *known == id) {
                Some(kept) if worth > kept.3 => *kept = (id, entry.clone(), function.clone(), worth),
                Some(_) => {}
                None => best.push((id, entry.clone(), function.clone(), worth)),
            }
        }
    }
    let mut account_routes = HashMap::new();
    let mut models = Vec::new();
    for (id, entry, function, _) in &best {
        let dev = dev_model(entry);
        account_routes.insert(id.clone(), Route { function: function.clone(), name: dev.map(|dev| text(&dev["model_name"])).unwrap_or_default(),
            most: dev.and_then(|dev| dev["max_tokens"].as_u64()).unwrap_or(0), max: None });
        let window = [&entry["context_window_tokens"]["dev"], &entry["context_window_tokens"]["max"], &entry["prompt_max_tokens"]].into_iter()
            .find_map(Value::as_u64).filter(|window| *window > 0).unwrap_or(128_000);
        let output = dev.and_then(|dev| dev["max_tokens"].as_u64()).filter(|tokens| *tokens > 0);
        models.push(SourceModel { id: id.clone(), name: name_of(entry), context_window: Some(window), max_output_tokens: output, ..Default::default() });
        if best.iter().any(|(other, ..)| *other == format!("{id}{MAX}")) {
            continue;
        }
        // its Max: the first __max model an entry of it names, when its window is bigger
        let entries: Vec<(&String, &Value)> = std::iter::once((function, entry)).chain(lists.iter().flat_map(|(function, entries)| {
            entries.iter().filter(|other| chat_model(other, site) && text(&other["config_name"]) == *id).map(move |other| (function, other))
        })).collect();
        let details = |entry: &Value| entry["model_detail_list"].as_array().cloned().unwrap_or_default();
        let Some((max_function, max_entry)) = entries.iter().find(|(_, entry)| details(entry).iter().any(|detail| detail["model_name"].as_str().is_some_and(|name| name.ends_with("__max")))) else { continue };
        let first = |key: &str| entries.iter().map(|(_, entry)| entry["context_window_tokens"][key].as_u64().unwrap_or(0)).find(|window| *window > 0).unwrap_or(0);
        let max_window = max_entry["context_window_tokens"]["max"].as_u64().filter(|window| *window > 0).unwrap_or_else(|| first("max"));
        if max_window <= first("dev") {
            continue;
        }
        let detail = details(max_entry).into_iter().find(|detail| detail["model_name"].as_str().is_some_and(|name| name.ends_with("__max"))).unwrap_or(Value::Null);
        let most = detail["max_tokens"].as_u64().unwrap_or(0);
        account_routes.insert(format!("{id}{MAX}"), Route { function: (*max_function).clone(), name: text(&detail["model_name"]), most, max: Some((id.clone(), max_window)) });
        models.push(SourceModel { id: format!("{id}{MAX}"), name: format!("{} (Max)", name_of(entry)), context_window: Some(max_window),
            max_output_tokens: if most > 0 { Some(most) } else { output }, ..Default::default() });
    }
    routes().lock().unwrap().insert(creds.uid.clone(), account_routes);
    if models.is_empty() {
        return Err(format!("{} listed no models to chat with", site.realm));
    }
    Ok(models)
}

/// Whether the account's routes are known here (they're kept in memory).
pub fn routes_known(creds: &Creds) -> bool {
    routes().lock().unwrap().contains_key(&creds.uid)
}

// ---- usage --------------------------------------------------------------------------

/// The account's allowance: Trae CN's credits, Trae Global's dollar usage.
pub async fn quota(client: &Client, site: &Site, creds: &Creds) -> (Quota, Option<String>) {
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), ..Default::default() };
    let headers = ide_headers(creds);
    let global = site.kind == SourceKind::TraeGlobal;
    let (url, body) = if global {
        let us = creds.region.to_ascii_uppercase().starts_with("US") || creds.api.contains("api-us");
        (format!("{}/trae/api/v1/pay/user_current_entitlement_list", if us { &site.us_pay } else { &site.pay }), json!({}))
    } else {
        (format!("{}/trae/api/v2/pay/ide_user_ent_usage", auth_host_of(site, creds)), json!({"require_usage": true, "req_source": 0}))
    };
    let (status, value) = match post(client, &url, &headers, &body).await {
        Ok(reply) => reply,
        Err(error) => {
            quota.error = Some(error);
            return (quota, None);
        }
    };
    let (code, message) = error_of(&value);
    if status == 401 || site.lapsed.contains(&code.parse().unwrap_or(0)) {
        quota.error = Some(format!("{}'s sign-in has expired — sign in again", site.realm));
        return (quota, None);
    }
    if status != 200 || !code.is_empty() && code != "0" {
        quota.error = Some(format!("{} usage: {}", site.realm, if message.is_empty() { format!("HTTP {status}") } else { message }));
        return (quota, None);
    }
    let number = |value: &Value| value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok())).unwrap_or(0.0);
    let packs = value["user_entitlement_pack_list"].as_array().cloned().unwrap_or_default();
    if global {
        let Some(pack) = packs.iter().find(|pack| !pack["usage"].is_null() || !pack["entitlement_base_info"]["quota"].is_null()) else {
            quota.error = Some("Trae Global usage: no entitlement pack in the answer".into());
            return (quota, None);
        };
        let limits = &pack["entitlement_base_info"]["quota"];
        quota.unit = "usd".into();
        quota.limit = number(&limits["basic_usage_limit"]) + number(&limits["bonus_usage_limit"]);
        quota.used = number(&pack["usage"]["basic_usage_amount"]) + number(&pack["usage"]["bonus_usage_amount"]);
        let plan = text(&pack["display_desc"]);
        return (quota, Some(if plan.is_empty() { "Free".into() } else { plan }));
    }
    quota.unit = "credits".into();
    for pack in &packs {
        let limit = &pack["entitlement_base_info"]["quota"]["credits_limit"];
        if limit.is_null() {
            continue;
        }
        let limit = number(limit);
        if limit >= 0.0 {
            quota.limit += limit;
        }
        quota.used += number(&pack["usage"]["credits_amount"]).max(0.0);
    }
    quota.paid = Some(value["is_credits_billing"] == true);
    (quota, Some(if value["is_credits_billing"] == true { "Credits".into() } else { "Free".into() }))
}

// ---- the daily check-in (Trae CN) ---------------------------------------------------

/// The User-Agent a growth page is sent with: TRAE SOLO CN's Electron shell.
fn ug_agent() -> String {
    format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) TRAE-SOLO-CN/{CLIENT_VERSION} Chrome/120.0.6099.291 Electron/28.2.10 Safari/537.36")
}

/// The headers Trae CN's IDE sends a growth page with, and only those; the
/// device of this machine's own Trae CN client when it is signed in as this
/// very account.
fn ug_headers(creds: &Creds, own: Option<&str>) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("content-type", "application/json".to_owned()),
        ("accept", "application/json".to_owned()),
        ("authorization", format!("Cloud-IDE-JWT {}", creds.token)),
        ("user-agent", ug_agent()),
        ("x-app-version", CLIENT_VERSION.to_owned()),
    ];
    match own {
        Some(device) => headers.extend([("x-device-id", device.to_owned()), ("x-device-type", "windows".to_owned()),
            ("x-os-version", format!("Windows {}", oauth::os_release()))]),
        None => headers.extend([("x-device-id", creds.device_id.clone()), ("x-device-brand", BRAND.to_owned()), ("x-device-type", "Windows".to_owned()),
            ("x-os-version", "10.0.22631".to_owned())]),
    }
    headers
}

/// Where Trae CN's desktop client keeps storage.json.
fn client_storage_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let file = |root: PathBuf, name: &str| root.join(name).join("User").join("globalStorage").join("storage.json");
    if cfg!(windows) {
        let mut roots = vec![home.join("AppData").join("Roaming")];
        if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
            if !roots.contains(&appdata) {
                roots.insert(0, appdata);
            }
        }
        roots.into_iter().flat_map(|root| ["Trae CN", "trae-cn"].map(|name| file(root.clone(), name))).collect()
    } else if cfg!(target_os = "macos") {
        vec![file(home.join("Library").join("Application Support"), "Trae CN")]
    } else {
        let root = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        ["trae-cn", "Trae CN"].map(|name| file(root.clone(), name)).into()
    }
}

/// The client's codec's salts, by the value's 6-byte header.
fn codec_salt(header: &[u8]) -> Option<Vec<u8>> {
    let xor = |a: &[u8; 64], b: &[u8; 64]| a.iter().zip(b).map(|(x, y)| x ^ y).collect::<Vec<u8>>();
    match header {
        [0x74, 0x63, 0x05, 0x10, 0x00, 0x00] => Some(xor(
            &[82, 9, 106, 213, 48, 54, 165, 56, 191, 64, 163, 158, 129, 243, 215, 251, 124, 227, 57, 130, 155, 47, 255, 135, 52, 142, 67, 68, 196, 222, 233, 203, 84, 123, 148, 50, 166, 194, 35, 61, 238, 76, 149, 11, 66, 250, 195, 78, 8, 46, 161, 102, 40, 217, 36, 178, 118, 91, 162, 73, 109, 139, 209, 37],
            &[31, 221, 168, 51, 136, 7, 199, 49, 177, 18, 16, 89, 39, 128, 236, 95, 96, 81, 127, 169, 25, 181, 74, 13, 45, 229, 122, 159, 147, 201, 156, 239, 160, 224, 59, 77, 174, 42, 245, 176, 200, 235, 187, 60, 131, 83, 153, 97, 23, 43, 4, 126, 186, 119, 214, 38, 225, 105, 20, 99, 85, 33, 12, 125],
        )),
        [0x12, 0x39, 0x20, 0x20, 0x02, 0x03] => Some(xor(
            &[191, 192, 216, 250, 122, 246, 220, 97, 31, 254, 98, 27, 8, 72, 71, 176, 135, 99, 96, 18, 127, 101, 203, 104, 211, 102, 191, 125, 37, 72, 150, 156, 51, 229, 121, 35, 17, 153, 141, 177, 110, 131, 150, 128, 172, 255, 254, 6, 18, 140, 55, 62, 236, 249, 135, 64, 135, 12, 117, 4, 89, 149, 168, 209],
            &[246, 204, 26, 232, 232, 70, 129, 109, 223, 146, 169, 242, 23, 241, 105, 145, 50, 196, 165, 42, 254, 120, 3, 54, 244, 207, 209, 85, 53, 6, 138, 106, 175, 148, 31, 204, 186, 186, 165, 182, 87, 142, 49, 10, 39, 110, 26, 154, 86, 56, 173, 125, 18, 64, 198, 225, 99, 99, 83, 82, 191, 134, 76, 170],
        )),
        _ => None,
    }
}

/// The client's codec undone: a 6-byte header naming the salt, 32 random
/// bytes, then AES-128-CBC under sha512(sha512(random) + salt); the
/// plaintext is led by its own sha512, which is checked.
fn client_decode(encoded: &str) -> Option<String> {
    use aes::cipher::{BlockDecrypt, KeyInit};
    use base64::Engine;
    use sha2::Digest;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    if bytes.len() <= 102 {
        return None;
    }
    let salt = codec_salt(&bytes[..6])?;
    let inner = sha2::Sha512::digest(&bytes[6..38]);
    let key = sha2::Sha512::digest([inner.as_slice(), &salt].concat());
    let cipher = aes::Aes128::new_from_slice(&key[..16]).ok()?;
    let mut previous: [u8; 16] = key[16..32].try_into().ok()?;
    let data = &bytes[38..];
    if data.len() % 16 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let mut block = aes::Block::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        out.extend(block.iter().zip(previous).map(|(byte, prev)| byte ^ prev));
        previous = chunk.try_into().ok()?;
    }
    let pad = *out.last()? as usize;
    if pad == 0 || pad > 16 || out.len() < pad {
        return None;
    }
    out.truncate(out.len() - pad);
    if out.len() < 64 || sha2::Sha512::digest(&out[64..]).as_slice() != &out[..64] {
        return None;
    }
    String::from_utf8(out[64..].to_vec()).ok()
}

/// The device id of this machine's Trae CN client, when it is signed in as `uid`.
fn client_device(uid: &str) -> Option<String> {
    for path in client_storage_paths() {
        let Ok(contents) = std::fs::read_to_string(&path) else { continue };
        let document: Value = serde_json::from_str(&contents).ok()?;
        let object = document.as_object()?;
        let ids: Vec<&str> = object.keys().filter_map(|key| key.strip_prefix("iCubeAuthInfo://icube-dc:")).collect();
        let [id] = ids.as_slice() else { return None };
        if !(8..=20).contains(&id.len()) || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let raw = object.get("iCubeAuthInfo://icube.cloudide")?.as_str()?.trim();
        let auth: Value = serde_json::from_str(&if raw.starts_with('{') { raw.to_owned() } else { client_decode(raw)? }).ok()?;
        return (text(&auth["userId"]) == uid).then(|| (*id).to_owned());
    }
    None
}

/// Press Trae CN's daily 签到 as its IDE does: the status first, a claim
/// while it is on and today's isn't in.
pub async fn check_in(client: &Client, site: &Site, creds: &Creds) -> Checkin {
    let own = client_device(&creds.uid);
    let headers = ug_headers(creds, own.as_deref());
    let base = format!("{}/trae/api/v2/ug/checkin_credits", site.auth);
    let body = json!({"req_source": 1});
    let refused = |code: i64, message: &str| match code {
        9095 => Checkin::outcome(Outcome::Ineligible).with_message(if message.is_empty() { "this device has checked in today" } else { message }),
        1001 => Checkin::failed("Trae CN's sign-in has expired — sign in again", false),
        9074 => Checkin::failed(if message.is_empty() { "asked too often" } else { message }, false),
        code => Checkin::failed(format!("code {code}: {message}"), false),
    };
    let status = match post(client, &format!("{base}/status"), &headers, &body).await {
        Ok((401, _)) => return Checkin::failed("Trae CN's sign-in has expired — sign in again", false),
        Ok((_, value)) => value,
        Err(error) => return Checkin::failed(error, true),
    };
    let code = status["code"].as_i64().unwrap_or(0);
    if code != 0 {
        return refused(code, status["message"].as_str().unwrap_or_default());
    }
    if status["enable"] != true {
        return Checkin::outcome(Outcome::Inactive);
    }
    let credits = status["credits"].as_f64().unwrap_or(0.0);
    if status["checked_in"] == true {
        return Checkin::done(credits, 0);
    }
    let got = match post(client, &format!("{base}/claim"), &headers, &body).await {
        Ok((401, _)) => return Checkin::failed("Trae CN's sign-in has expired — sign in again", false),
        Ok((_, value)) => value,
        Err(error) => return Checkin::failed(error, true),
    };
    let message = got["message"].as_str().unwrap_or_default();
    match got["code"].as_i64().unwrap_or(0) {
        // status said on and not in, so the claim itself is refused: Trae pays only its own app
        9074 => Checkin::outcome(Outcome::OwnApp).with_message(if message.is_empty() { "Trae CN gives check-in credits only to its own app" } else { message }),
        0 => {
            let credit = got["credits"].as_f64().filter(|credit| *credit > 0.0).unwrap_or(credits);
            Checkin::claimed(credit + got["extra_credits"].as_f64().unwrap_or(0.0), 0)
        }
        code => refused(code, message),
    }
}

/// What the user can do about a Trae refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status == 401 {
        return Some("Trae's sign-in has expired; sign in again on the Accounts page".into());
    }
    let _ = body;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(chunks: &[&str], tools: Vec<Value>) -> Vec<Part> {
        let mut parser = Parser::new(tools);
        let mut out = Vec::new();
        for chunk in chunks {
            parser.feed(chunk.as_bytes(), &mut out);
        }
        parser.finish(&mut out);
        out
    }

    fn sse(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| format!("event: output\ndata: {}\n\n", json!({"response": text}))).collect()
    }

    fn calls(parts: &[Part]) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        for part in parts {
            match part {
                Part::Event(Event::ToolStart { name, .. }) => out.push((name.clone(), Value::Null)),
                Part::Event(Event::ToolArgs { text, .. }) => out.last_mut().unwrap().1 = serde_json::from_str(text).unwrap(),
                _ => {}
            }
        }
        out
    }

    fn texts(parts: &[Part]) -> String {
        parts.iter().filter_map(|part| match part { Part::Event(Event::Text(text)) => Some(text.as_str()), _ => None }).collect()
    }

    #[test]
    fn a_tagged_block_split_across_events_is_one_call() {
        let tools = vec![json!({"type": "function", "function": {"name": "Bash", "parameters": {"properties": {"command": {"type": "string"}}}}})];
        let chunks = sse(&["let me run it <tool", "_call>{\"name\": \"bash\", \"argu", "ments\": {\"command\": \"ls\"}}</tool_call>"]);
        let parts = read_all(&chunks.iter().map(String::as_str).collect::<Vec<_>>(), tools);
        assert_eq!(texts(&parts), "let me run it ");
        assert_eq!(calls(&parts), [("Bash".to_owned(), json!({"command": "ls"}))], "named as the agent named its tool");
        assert_eq!(parts.last(), Some(&Part::Event(Event::Finish("tool_calls"))));
    }

    #[test]
    fn glm_s_template_deepseek_s_dsml_and_parameters_are_calls_too() {
        let tools = vec![json!({"function": {"name": "write", "parameters": {"properties": {"path": {"type": "string"}, "n": {"type": "integer"}}}}})];
        let glm = sse(&["<tool_call>write<arg_key>path</arg_key><arg_value>a.txt</arg_value><arg_key>n</arg_key><arg_value>3</arg_value></tool_call>"]);
        assert_eq!(calls(&read_all(&glm.iter().map(String::as_str).collect::<Vec<_>>(), tools.clone())), [("write".to_owned(), json!({"path": "a.txt", "n": 3}))]);
        let dsml = sse(&["ok <｜DSML｜function_calls>\n<｜DSML｜invoke name=\"write\">\n<｜DSML｜parameter name=\"path\" string=\"true\">b.txt</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜function_calls>"]);
        let parts = read_all(&dsml.iter().map(String::as_str).collect::<Vec<_>>(), tools.clone());
        assert_eq!(calls(&parts), [("write".to_owned(), json!({"path": "b.txt"}))]);
        assert_eq!(texts(&parts).trim(), "ok");
        let named = sse(&["<tool_call name=\"write\"><parameter=path>c.txt</parameter></tool_call>"]);
        assert_eq!(calls(&read_all(&named.iter().map(String::as_str).collect::<Vec<_>>(), tools.clone())), [("write".to_owned(), json!({"path": "c.txt"}))]);
        let quoted = sse(&["use `<tool_call>` to call"]);
        assert_eq!(texts(&read_all(&quoted.iter().map(String::as_str).collect::<Vec<_>>(), tools.clone())), "use `<tool_call>` to call");
        let repaired = sse(&["<tool_call>{\"name\":\"write\",\"arguments\":{\"path\":\"C:\\Users\\x.txt\"}}</tool_call>"]);
        assert_eq!(calls(&read_all(&repaired.iter().map(String::as_str).collect::<Vec<_>>(), tools)), [("write".to_owned(), json!({"path": "C:\\Users\\x.txt"}))]);
    }

    #[test]
    fn a_call_cut_short_or_unreadable_fails_the_turn() {
        let cut = sse(&["<tool_call>{\"name\": \"x\", \"arguments\": {\"a\""]);
        let parts = read_all(&cut.iter().map(String::as_str).collect::<Vec<_>>(), vec![]);
        assert_eq!(parts.last(), Some(&Part::Failed(String::new(), CUT_SHORT.into())));
        let error = "event: error\ndata: {\"code\":4008,\"message\":\"quota exceeded\"}\n\n";
        assert_eq!(read_all(&[error], vec![]), [Part::Failed("4008".into(), "quota exceeded".into())]);
    }

    #[test]
    fn native_calls_reasoning_and_usage_are_read() {
        let chunks = [
            "event: output\ndata: {\"reasoning_content\":\"thinking hard\"}\n\n",
            "event: output\ndata: {\"reasoning_content\":\"thinking hard\"}\n\n",
            "event: output\ndata: {\"tool_calls\":[{\"id\":\"t1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\"}}]}\n\n",
            "event: output\ndata: {\"tool_calls\":[{\"id\":\"t1\",\"function\":{\"arguments\":\"\\\"a\\\"}\"}}]}\n\n",
            "event: token_usage\ndata: {\"prompt_tokens\":5,\"completion_tokens\":2,\"cache_read_input_tokens\":3}\n\n",
            "event: done\ndata: {}\n\n",
        ];
        let parts = read_all(&chunks, vec![]);
        assert_eq!(parts[0], Part::Event(Event::Reasoning("thinking hard".into())));
        assert!(parts.contains(&Part::Event(Event::Usage { input: 5, output: 2, cached: 3, reasoning: 0 })));
        assert_eq!(calls(&parts), [("read".to_owned(), json!({"path": "a"}))]);
        assert_eq!(parts.iter().filter(|part| matches!(part, Part::Event(Event::Reasoning(_)))).count(), 1, "a piece sent again sends nothing new");
    }

    #[test]
    fn the_request_names_tools_natively_and_in_the_prompt() {
        let chat = json!({"model": "glm-5.2", "max_tokens": 64000, "messages": [{"role": "system", "content": "be brief"}, {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "ls", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "a.txt"}, {"role": "user", "content": "more"}],
            "tools": [{"type": "function", "function": {"name": "ls", "parameters": {"type": "object"}}}]});
        let route = Route { function: "solo_agent".into(), name: "glm-5.2__dev".into(), most: 32_000, max: None };
        let body = chat_body(&chat, "solo_agent", Some(&route), "s-1");
        assert_eq!(body["model_name"], "glm-5.2__dev");
        assert_eq!(body["max_tokens"], 32_000, "held to what the function gives the model");
        assert_eq!(body["tools"][0]["function"]["parameters"], "{\"type\":\"object\"}");
        let messages = body["messages"].as_array().unwrap();
        assert!(messages[0]["content"][0]["text"].as_str().unwrap().contains("<tool_call>{\"name\":\"tool_name\""));
        assert_eq!(messages[2]["content"][0]["text"], "hi");
        assert!(messages[3]["content"][0]["text"].as_str().unwrap().contains("<tool_call>{\"arguments\":{},\"name\":\"ls\"}</tool_call>")
            || messages[3]["content"][0]["text"].as_str().unwrap().contains("\"name\":\"ls\""));
        assert!(messages[4]["content"][0]["text"].as_str().unwrap().starts_with("Result of ls (call c1):\na.txt\n\nmore"), "a result and the user's next words are one turn");
        let max = Route { function: "chat_v3".into(), name: "glm-5.2__max".into(), most: 64_000, max: Some(("glm-5.2".into(), 400_000)) };
        let body = chat_body(&json!({"model": "glm-5.2-max", "messages": []}), "chat_v3", Some(&max), "");
        assert_eq!((body["config_name"].as_str(), body["max_tokens"].as_u64()), (Some("glm-5.2"), Some(64_000)));
        assert_eq!(body["user_message_context"]["model_info"]["prompt_max_tokens"], 336_000);
    }

    #[test]
    fn the_device_proof_verifies_with_the_device_s_public_key() {
        use base64::Engine;
        let (public, private) = new_device_key();
        assert!(public.starts_with("-----BEGIN PUBLIC KEY-----\nMFkw") && public.ends_with("-----END PUBLIC KEY-----\n"));
        let proof = proof_of("/p", "c", "r", &private).unwrap();
        let der = base64::engine::general_purpose::STANDARD.decode(public.lines().filter(|line| !line.starts_with("-----")).collect::<String>()).unwrap();
        let point = &der[der.len() - 65..];
        let message = ["POST", "/p", "c", "r", &proof["Timestamp"].to_string(), proof["Nonce"].as_str().unwrap()].join("\n");
        let signature = base64::engine::general_purpose::STANDARD.decode(proof["Signature"].as_str().unwrap()).unwrap();
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point).verify(message.as_bytes(), &signature).unwrap();
        let (device, machine, ..) = new_device();
        assert_eq!((device.len(), machine.len()), (19, 32));
        assert!(!device.starts_with('0'));
    }

    /// What TextTools makes of `text` given whole, or a few characters at a time.
    fn read(text: &str, size: usize) -> (String, Vec<(String, Value)>, Option<&'static str>) {
        let tools = vec![
            json!({"type": "function", "function": {"name": "bash", "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}}}),
            json!({"type": "function", "function": {"name": "write", "parameters": {"type": "object", "properties": {"content": {"type": "string"}, "filePath": {"type": "string"}}}}}),
        ];
        let mut parser = TextTools { tools, ..Default::default() };
        let chars: Vec<char> = text.chars().collect();
        let mut pieces: Vec<String> = if size == 0 { vec![text.to_owned()] } else { chars.chunks(size).map(|chunk| chunk.iter().collect()).collect() };
        pieces.push(String::new());
        let (mut said, mut calls) = (String::new(), Vec::new());
        let last = pieces.len() - 1;
        for (index, piece) in pieces.iter().enumerate() {
            let pushed = parser.push(piece, index == last);
            said.push_str(&pushed.text);
            calls.extend(pushed.calls.into_iter().map(|(name, args)| (name, serde_json::from_str::<Value>(&args).unwrap())));
            if pushed.error.is_some() {
                return (said, calls, pushed.error);
            }
        }
        (said, calls, None)
    }

    #[test]
    fn text_calls_read_as_the_plugin_s_cases_read_them() {
        let call = |name: &str, args: Value| vec![(name.to_owned(), args)];
        for size in [0, 1, 7] {
            let arno = "官方资料齐了。我把完整分析写成文件，聊天里给你重点。\n<tool_call>{\"name\":\"write\",\"arguments\":{\"content\":\"xxxxxxxx\"}}";
            assert_eq!(read(arno, size), ("官方资料齐了。我把完整分析写成文件，聊天里给你重点。\n".into(), call("write", json!({"content": "xxxxxxxx"})), None), "size {size}");
        }
        let contents = [("# 分析\n他说\"官方\"的资料齐了", "# 分析\n他说\"官方\"的资料齐了"), (r"snake\_case 和 \*强调\*", r"snake\_case 和 \*强调\*"),
            (r"路径 C:\Users\arno\x", r"路径 C:\Users\arno\x"), ("```json\n{\"a\": 1, \"b\": \"c\"}\n```\n完", "```json\n{\"a\": 1, \"b\": \"c\"}\n```\n完")];
        for (content, want) in contents {
            let text = format!("重写。\n<tool_call>{{\"name\":\"write\",\"arguments\":{{\"content\":\"{content}\"}}}}</tool_call>");
            for size in [0, 5] {
                assert_eq!(read(&text, size), ("重写。\n".into(), call("write", json!({"content": want})), None), "{content:?} size {size}");
            }
        }
        assert_eq!(read("Sure. <tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"ls\"}}</tool_call>", 0), ("Sure. ".into(), call("bash", json!({"command": "ls"})), None));
        for size in [0, 3] {
            assert_eq!(read("<tool_call>{\\\"name\\\":\\\"bash\\\",\\\"arguments\\\":{\\\"command\\\":\\\"ls\\\"}}</tool_call>", size).1, call("bash", json!({"command": "ls"})));
            assert_eq!(read("<tool_call name=\"bash\">\n<parameter name=\"command\">ls</parameter>\n</tool_call>", size).1, call("bash", json!({"command": "ls"})));
            assert_eq!(read("<tool_call>{\"name\":\"bash\"}>\n<parameter name=\"command\">ls -la</parameter>\n</tool_call>", size).1, call("bash", json!({"command": "ls -la"})));
        }
        for cut in ["<tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"cd \"C:/x\"", "<tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"ls\"}",
            "<tool_call name=\"bash\">\n<parameter name=\"command\">ls -", "<tool_call name", "<tool_call", "<toolcall name"] {
            for size in [0, 4] {
                let (said, calls, error) = read(&format!("Running it.\n{cut}"), size);
                assert_eq!((said.as_str(), calls.len(), error), ("Running it.\n", 0, Some(CUT_SHORT)), "{cut:?} size {size}");
            }
        }
        assert_eq!(read("<tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":}}</tool_call>", 0).2, Some(UNREADABLE));
        for size in [0, 1, 4] {
            assert_eq!(read("<toolcall></toolcall>", size), (String::new(), vec![], None), "an empty block is no call and no text");
            assert_eq!(read("<toolcall>{\"name\":\"bash\",\"arguments\":{\"command\":\"ls\"}}</toolcall>", size).1, call("bash", json!({"command": "ls"})));
        }
        for prose in ["Each call goes in a `<toolcall>` block.", "Write `<toolcall>{\"name\":\"x\"}</toolcall>` to call.",
            "Example:\n```\n<tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"ls\"}}</tool_call>\n```\nThat is all.",
            "The <tool_call> tag wraps a call; </tool_call> ends it.", "<tool_call>hello</tool_call> ok"] {
            for size in [0, 1, 6] {
                assert_eq!(read(prose, size), (prose.to_owned(), vec![], None), "{prose:?} size {size}");
            }
        }
    }

    #[test]
    fn hosts_named_by_a_sign_in_are_trae_s_own() {
        assert_eq!(trae_host("https://trae-api-cn.mchost.guru/"), "https://trae-api-cn.mchost.guru");
        assert_eq!(trae_host("https://evil.example.com"), "");
        assert_eq!(trae_host("http://api.trae.cn"), "");
        assert_eq!(event_name("TokenUsage"), "token_usage");
        assert_eq!(event_name("token-usage"), "token_usage");
    }
}
