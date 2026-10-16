//! Qoder (qoder.com) and Qoder CN (qoder.cn): the subscription served on
//! the API Qoder's own client talks to.
//!
//! Ported from magpie's community plugin (MIT, see LICENSES/THIRD_PARTY.md),
//! whose protocol (endpoints, COSY envelope, body codec, device flow) came
//! from CLIProxyAPI's Qoder support (MIT). An account signs in the way
//! Qoder's desktop client does — a PKCE device flow, whose device token is
//! traded for the job token the model calls use (Qoder CN's CLI chats on the
//! device token itself when qoder.cn won't trade it) — or with a Personal
//! Access Token, or, on Qoder CN, on an enterprise's own VPC instance.
//! Requests go to `agent_chat_generation`, written as Qoder's request (its
//! own system line before the agent's), encoded with the client's body codec
//! and signed with its COSY envelope; the reply's SSE comes back as Chat,
//! tool calls Qoder writes in its text lifted out as calls. The daily
//! credits are campaigns claimed on the device token; the international
//! site's need Qoder's own native device identity, read from the
//! `runtime-info` helper Qoder's desktop app or CLI installs.
use super::{
    checkin::{Checkin, Outcome},
    oauth,
    store::{Quota, SourceModel},
    wire::Event,
    SourceKind,
};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub const CHAT_PATH: &str = "algo/api/v2/service/pro/sse/agent_chat_generation?FetchKeys=llm_model_result&AgentId=agent_common&Encode=1";
const MODELS_PATH: &str = "/algo/api/v2/model/list?Encode=1";
const COSY_VERSION: &str = "1.1.49";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(15 * 60);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const CAMPAIGNS: &str = "/sash/api/v1/me/campaigns";

/// Qoder's embedded 1024-bit RSA key: it wraps the per-request AES key of
/// the COSY Authorization header.
const RSA_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDA8iMH5c02LilrsERw9t6Pv5Nc
4k6Pz1EaDicBMpdpxKduSZu5OANqUq8er4GM95omAGIOPOh+Nx0spthYA2BqGz+l
6HRkPJ7S236FZz73In/KVuLnwI8JJ2CbuJap8kvheCCZpmAWpb/cPx/3Vr/J6I17
XcW+ML9FoCI6AOvOzwIDAQAB
-----END PUBLIC KEY-----";

/// The line Qoder's own requests open their system prompt with.
const QODER_SYS: &str = "You are a Qoder agent. Use the instructions below and the tools available to you to assist the user.";

#[derive(Clone, Debug)]
pub struct Site {
    pub kind: SourceKind,
    name: &'static str,
    client_id: &'static str,
    web: String,
    pub openapi: String,
    pub api: String,
    redirect: &'static str,
    /// A refused job token falls back to chatting on the device token.
    device_chat: bool,
    vpc: Option<String>,
}

pub fn site(kind: SourceKind) -> Site {
    let host = |real: &str| super::endpoint_override(kind).unwrap_or_else(|| real.to_owned());
    match kind {
        SourceKind::QoderCn => Site {
            kind, name: "Qoder CN", client_id: "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb",
            web: host("https://qoder.cn"), openapi: host("https://openapi.qoder.com.cn"), api: host("https://gateway.qoder.com.cn"),
            redirect: "", device_chat: true, vpc: None,
        },
        _ => Site {
            kind, name: "Qoder", client_id: "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa",
            web: host("https://qoder.com"), openapi: host("https://openapi.qoder.sh"), api: host("https://api3.qoder.sh"),
            redirect: "qoder-app://", device_chat: false, vpc: None,
        },
    }
}

const VPC_DOMAIN: &str = "vpc.qoder.com.cn";

/// The VPC instance a user's entry names, as Qoder CN's CLI reads it: a bare
/// name, or one of its hosts (also as an https address with no path).
pub fn vpc_instance(input: &str) -> Option<String> {
    let mut entry = input.trim().to_ascii_lowercase();
    if let Some(rest) = entry.strip_prefix("https://") {
        entry = rest.trim_end_matches('/').to_owned();
        if entry.contains(['/', '?', '#', '@', ':']) {
            return None;
        }
    } else if entry.contains("://") {
        return None;
    }
    if entry.split('.').count() == 4 && entry.split('.').all(|part| part.parse::<u8>().is_ok()) {
        return None;
    }
    let name = match entry.strip_suffix(&format!(".{VPC_DOMAIN}")) {
        Some(host) => host.trim_end_matches("-openapi").trim_end_matches("-gateway").to_owned(),
        None if entry.contains('.') => return None,
        None => entry,
    };
    let ok = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !name.starts_with('-') && !name.ends_with('-');
    ok.then_some(name)
}

/// The site an account is served from: its VPC instance's hosts when it was
/// signed in to one.
pub fn site_of(kind: SourceKind, creds: &Creds) -> Result<Site, String> {
    let mut base = site(kind);
    if creds.vpc.is_empty() {
        return Ok(base);
    }
    let name = vpc_instance(&creds.vpc).ok_or_else(|| format!("the account's VPC instance \"{}\" isn't one; sign in again", creds.vpc))?;
    if super::endpoint_override(kind).is_none() {
        base.web = format!("https://{name}.{VPC_DOMAIN}");
        base.openapi = format!("https://{name}-openapi.{VPC_DOMAIN}");
        base.api = format!("https://{name}-gateway.{VPC_DOMAIN}");
    }
    base.vpc = Some(name);
    Ok(base)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    /// The chat token: a job token, or the device token on Qoder CN.
    pub access: String,
    pub refresh: String,
    /// Unix milliseconds.
    pub expires_at: i64,
    pub uid: String,
    pub email: String,
    pub name: String,
    /// This sign-in's machine id, which every envelope names.
    pub machine_id: String,
    /// Reads usage and claims campaigns.
    pub device_token: String,
    pub device_refresh: String,
    /// The device token is the chat token (Qoder CN's CLI way).
    pub device_chat: bool,
    /// A Qoder CN enterprise's VPC instance.
    pub vpc: String,
    /// A Personal Access Token, exchanged again when the job refresh lapses.
    pub pat: String,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Creds {
    /// The chat token lapses within five minutes.
    pub fn near_expiry(&self) -> bool {
        self.expires_at - now_ms() <= 5 * 60 * 1000
    }
}

/// One of Qoder's account pages: its status and JSON.
async fn openapi(client: &Client, site: &Site, method: Method, path: &str, token: Option<&str>, body: Option<Value>, query: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let mut url = reqwest::Url::parse(&format!("{}{path}", site.openapi)).map_err(|_| "invalid Qoder URL".to_owned())?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    let mut request = client.request(method, url).header("accept", "application/json").timeout(Duration::from_secs(20));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(body) = body {
        request = request.header("content-type", "application/json").body(body.to_string());
    }
    let response = request.send().await.map_err(|_| format!("cannot reach {}", site.name))?;
    let status = response.status().as_u16();
    Ok((status, response.json().await.unwrap_or(Value::Null)))
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

/// When a token Qoder gave runs out: a date, else `expires_in` in the unit
/// given, else a day.
fn expiry(token: &Value, unit_ms: i64) -> i64 {
    for key in ["expire_time", "expires_at"] {
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(token[key].as_str().unwrap_or_default().trim()) {
            return at.timestamp_millis();
        }
    }
    match token["expires_in"].as_i64().filter(|value| *value > 0) {
        Some(value) => now_ms() + value * unit_ms,
        None => now_ms() + DAY_MS,
    }
}

pub struct SignIn {
    pub url: String,
    site: Site,
    nonce: String,
    verifier: String,
    machine_id: String,
}

/// Open Qoder's device sign-in page.
pub fn begin_sign_in(site: Site) -> SignIn {
    use base64::Engine;
    let raw: Vec<u8> = (0..4).flat_map(|_| *uuid::Uuid::new_v4().as_bytes()).collect();
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    use sha2::Digest;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let nonce = uuid::Uuid::new_v4().to_string();
    let machine_id = uuid::Uuid::new_v4().to_string();
    let mut url = reqwest::Url::parse(&format!("{}/device/selectAccounts", site.web)).expect("a Qoder URL");
    url.query_pairs_mut().append_pair("challenge", &challenge).append_pair("challenge_method", "S256").append_pair("nonce", &nonce)
        .append_pair("machine_id", &machine_id).append_pair("client_id", site.client_id);
    if !site.redirect.is_empty() {
        url.query_pairs_mut().append_pair("redirect_uri", site.redirect);
    }
    SignIn { url: url.to_string(), site, nonce, verifier, machine_id }
}

pub struct SignedIn {
    pub creds: Creds,
    pub name: String,
}

/// Who a token signs in as: email, name and id.
async fn user_info(client: &Client, site: &Site, token: &str) -> (String, String, String) {
    match openapi(client, site, Method::GET, "/api/v1/userinfo", Some(token), None, &[]).await {
        Ok((200..=299, info)) => (text(&info["email"]), text(&info["name"]),
            [&info["id"], &info["user_id"], &info["uid"]].into_iter().map(text).find(|id| !id.is_empty()).unwrap_or_default()),
        _ => Default::default(),
    }
}

/// Poll until the browser authorizes the device, then trade its token.
pub async fn finish_sign_in(client: &Client, sign_in: SignIn, mut cancel: watch::Receiver<bool>, floor: Duration) -> Result<SignedIn, String> {
    let site = sign_in.site;
    let deadline = Instant::now() + SIGN_IN_WINDOW;
    let device = loop {
        tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            _ = tokio::time::sleep(if cfg!(test) { floor } else { Duration::from_secs(2).max(floor) }) => {}
        }
        if Instant::now() >= deadline {
            return Err("accounts.loginTimeout".into());
        }
        let query = [("nonce", sign_in.nonce.as_str()), ("verifier", sign_in.verifier.as_str()), ("challenge_method", "S256")];
        if let Ok((200, token)) = openapi(client, &site, Method::GET, "/api/v1/deviceToken/poll", None, None, &query).await {
            if !text(&token["token"]).is_empty() {
                break token;
            }
        }
    };
    let device_token = text(&device["token"]);
    let (status, job) = openapi(client, &site, Method::POST, "/api/v1/me/jobToken", Some(&device_token), Some(json!({"clientId": site.client_id})), &[]).await?;
    let mut creds = Creds {
        uid: text(&device["user_id"]), machine_id: sign_in.machine_id, device_token: device_token.clone(),
        device_refresh: text(&device["refresh_token"]), vpc: site.vpc.clone().unwrap_or_default(), ..Default::default()
    };
    if status == 200 {
        creds.access = text(&job["token"]);
        if creds.access.is_empty() {
            return Err(format!("{} job token: empty token in response", site.name));
        }
        creds.refresh = text(&job["refresh_token"]);
        creds.expires_at = expiry(&json!({"expires_in": job["expires_in"]}), 1);
    } else if site.device_chat && (400..500).contains(&status) {
        // Qoder CN's CLI never makes a job token: its device token signs the
        // model calls, renewed as a device token
        creds.access = device_token.clone();
        creds.refresh = creds.device_refresh.clone();
        creds.expires_at = expiry(&device, 1000);
        creds.device_chat = true;
    } else {
        return Err(format!("{} job token: status {status}", site.name));
    }
    let (email, name, id) = user_info(client, &site, &device_token).await;
    if creds.uid.is_empty() {
        creds.uid = id;
    }
    if creds.uid.is_empty() {
        return Err(format!("{}: the sign-in named no user", site.name));
    }
    creds.email = email.clone();
    creds.name = if name.is_empty() { text(&device["user_name"]) } else { name };
    let shown = if email.is_empty() { creds.uid.clone() } else { email };
    let name = match &site.vpc { Some(vpc) => format!("{shown} ({vpc})"), None => shown };
    Ok(SignedIn { creds, name })
}

/// Trade a Personal Access Token for a job token.
pub async fn sign_in_with_pat(client: &Client, site: &Site, pat: &str) -> Result<SignedIn, String> {
    let (status, token) = openapi(client, site, Method::POST, "/api/v1/jobToken/exchange", None, Some(json!({"personal_token": pat})), &[]).await?;
    if matches!(status, 400 | 401 | 403) {
        return Err(format!("{} refused the PAT — check it is still valid", site.name));
    }
    if status != 200 {
        return Err(format!("{} PAT exchange: status {status}", site.name));
    }
    let access = [&token["token"], &token["device_token"], &token["access_token"]].into_iter().map(text).find(|token| !token.is_empty())
        .ok_or_else(|| format!("{} PAT exchange: empty token in response", site.name))?;
    let (email, name, uid) = user_info(client, site, &access).await;
    if uid.is_empty() {
        return Err(format!("{} PAT sign-in: could not determine user id", site.name));
    }
    let refresh = [&token["refresh_token"], &token["refreshToken"]].into_iter().map(text).find(|token| !token.is_empty()).unwrap_or_default();
    let creds = Creds { access, refresh, expires_at: expiry(&token, 1), uid: uid.clone(), email: email.clone(), name,
        machine_id: uuid::Uuid::new_v4().to_string(), pat: pat.to_owned(), ..Default::default() };
    Ok(SignedIn { creds, name: if email.is_empty() { uid } else { email } })
}

pub enum RefreshError {
    SignedOut(String),
    Transient,
}

/// Renew the chat token: a job token with its refresh token (a PAT's
/// exchanged again once that lapses), or the device token Qoder CN chats on.
pub async fn refresh(client: &Client, site: &Site, creds: &Creds) -> Result<Creds, RefreshError> {
    if creds.device_chat {
        let (token, refresh, expires) = refresh_device(client, site, &creds.refresh, true).await?;
        return Ok(Creds { access: token.clone(), refresh: refresh.clone(), expires_at: expires, device_token: token, device_refresh: refresh, ..creds.clone() });
    }
    let refused = if creds.refresh.is_empty() {
        true
    } else {
        let (status, job) = openapi(client, site, Method::POST, "/api/v1/jobToken/refresh", None, Some(json!({"refresh_token": creds.refresh})), &[]).await
            .map_err(|_| RefreshError::Transient)?;
        match status {
            200 => {
                let (access, refresh) = (text(&job["token"]), text(&job["refresh_token"]));
                if access.is_empty() || refresh.is_empty() {
                    return Err(RefreshError::Transient);
                }
                return Ok(Creds { access, refresh, expires_at: expiry(&json!({"expires_in": job["expires_in"]}), 1), ..creds.clone() });
            }
            401 | 403 => true,
            _ => return Err(RefreshError::Transient),
        }
    };
    if refused && !creds.pat.is_empty() {
        return match sign_in_with_pat(client, site, &creds.pat).await {
            Ok(done) => Ok(Creds { access: done.creds.access, refresh: done.creds.refresh, expires_at: done.creds.expires_at, ..creds.clone() }),
            Err(error) if error.contains("refused the PAT") => Err(RefreshError::SignedOut(error)),
            Err(_) => Err(RefreshError::Transient),
        };
    }
    Err(RefreshError::SignedOut(format!("{} sign-in has expired — sign in again", site.name)))
}

/// Rotate the device token: (token, refresh, expiry).
async fn refresh_device(client: &Client, site: &Site, refresh: &str, chat: bool) -> Result<(String, String, i64), RefreshError> {
    if refresh.is_empty() {
        return Err(if chat { RefreshError::SignedOut(format!("{}: the sign-in lapsed; sign in again", site.name)) } else { RefreshError::Transient });
    }
    let (status, token) = openapi(client, site, Method::POST, "/api/v1/deviceToken/refresh", None, Some(json!({"refresh_token": refresh})), &[]).await
        .map_err(|_| RefreshError::Transient)?;
    match status {
        200 => {
            let access = [&token["token"], &token["device_token"]].into_iter().map(text).find(|token| !token.is_empty()).unwrap_or_default();
            let next = text(&token["refresh_token"]);
            if access.is_empty() || next.is_empty() {
                return Err(RefreshError::Transient);
            }
            Ok((access, next, expiry(&token, 1000)))
        }
        401 | 403 if chat => Err(RefreshError::SignedOut(format!("{} sign-in has expired — sign in again", site.name))),
        _ => Err(RefreshError::Transient),
    }
}

// ---- the COSY envelope ----------------------------------------------------------

/// Random bytes for RSA's padding, from the OS (through uuid's getrandom).
struct OsBytes;

impl rsa::rand_core::RngCore for OsBytes {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(16) {
            chunk.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..chunk.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rsa::rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl rsa::rand_core::CryptoRng for OsBytes {}

/// AES-128-CBC with PKCS#7 padding.
fn aes_cbc(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockEncrypt, KeyInit};
    let cipher = aes::Aes128::new(key.into());
    let pad = 16 - data.len() % 16;
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(pad as u8, pad));
    let mut previous = *iv;
    let mut out = Vec::with_capacity(padded.len());
    for chunk in padded.chunks(16) {
        let mut block = [0u8; 16];
        for (index, byte) in block.iter_mut().enumerate() {
            *byte = chunk[index] ^ previous[index];
        }
        let mut block = aes::Block::from(block);
        cipher.encrypt_block(&mut block);
        previous = block.into();
        out.extend_from_slice(&previous);
    }
    out
}

/// The account as the envelope carries it: AES-128-CBC under a fresh key
/// that is its own IV, the key wrapped with Qoder's RSA key.
fn user_blob(creds: &Creds) -> (String, String) {
    use base64::Engine;
    use rsa::pkcs8::DecodePublicKey;
    let raw = json!({"uid": creds.uid, "aid": "", "name": creds.name, "email": creds.email, "security_oauth_token": creds.access}).to_string();
    let hex = uuid::Uuid::new_v4().simple().to_string();
    let key: [u8; 16] = hex.as_bytes()[..16].try_into().expect("16 bytes");
    let info = base64::engine::general_purpose::STANDARD.encode(aes_cbc(&key, &key, raw.as_bytes()));
    let public = rsa::RsaPublicKey::from_public_key_pem(RSA_KEY).expect("Qoder's RSA key");
    let wrapped = public.encrypt(&mut OsBytes, rsa::Pkcs1v15Encrypt, &key).expect("RSA encryption of 16 bytes");
    (info, base64::engine::general_purpose::STANDARD.encode(wrapped))
}

/// The machine as Qoder's desktop clients name it: `x86_64_win32`.
fn machine_os() -> String {
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    format!("{arch}_{}", oauth::node_platform())
}

/// The headers that sign a call to `url` whose body, as sent, is `body`.
pub fn cosy_headers(url: &str, creds: &Creds, body: &str) -> Vec<(&'static str, String)> {
    use base64::Engine;
    let (info, key) = user_blob(creds);
    let payload = base64::engine::general_purpose::STANDARD.encode(json!({"version": "v1", "requestId": uuid::Uuid::new_v4().simple().to_string(),
        "info": info, "cosyVersion": COSY_VERSION, "ideVersion": ""}).to_string());
    let path = reqwest::Url::parse(url).map(|url| url.path().to_owned()).unwrap_or_default();
    let path = path.strip_prefix("/algo").unwrap_or(&path).to_owned();
    let seconds = chrono::Utc::now().timestamp();
    let signature = oauth::md5_hex(&format!("{payload}\n{key}\n{seconds}\n{body}\n{path}"));
    let machine = creds.machine_id.clone();
    vec![
        ("accept", "application/json".into()),
        ("accept-encoding", "identity".into()),
        ("content-type", "application/json".into()),
        ("authorization", format!("Bearer COSY.{payload}.{signature}")),
        ("cosy-business-product", "app".into()),
        ("cosy-business-type", "agent".into()),
        ("cosy-clientip", machine.clone()),
        ("cosy-clienttype", "10".into()),
        ("cosy-data-policy", "disagree".into()),
        ("cosy-date", seconds.to_string()),
        ("cosy-key", key),
        ("cosy-machineid", machine.clone()),
        ("cosy-machinetoken", machine),
        ("cosy-machinetype", "5".into()),
        ("cosy-machineos", machine_os()),
        ("cosy-scene", "app".into()),
        ("cosy-user", creds.uid.clone()),
        ("cosy-version", COSY_VERSION.into()),
        ("login-version", "v2".into()),
    ]
}

// ---- the body codec -----------------------------------------------------------

/// Base64 through the client's shuffled alphabet (`$` pads), then the first
/// and last thirds swapped.
const ALPHABET: &[u8; 64] = b"_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!";

fn swap_thirds(text: &str) -> String {
    let third = text.len() / 3;
    if third == 0 {
        return text.to_owned();
    }
    format!("{}{}{}", &text[text.len() - third..], &text[third..text.len() - third], &text[..third])
}

pub fn encode_body(text: &str) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        acc = ((acc << 8) | u32::from(byte)) & 0xff_ffff;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 63) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (6 - bits)) & 63) as usize] as char);
    }
    while out.len() % 4 != 0 {
        out.push('$');
    }
    swap_thirds(&out)
}

#[cfg(test)]
fn decode_body(wire: &str) -> String {
    let text = swap_thirds(wire);
    let mut out = Vec::new();
    for group in text.as_bytes().chunks(4) {
        let values: Vec<u64> = group.iter().filter(|byte| **byte != b'$')
            .map(|byte| ALPHABET.iter().position(|known| known == byte).unwrap_or(0) as u64).collect();
        let x = values.iter().fold(0u64, |acc, value| acc * 64 + value);
        let count = 6 * values.len() / 8;
        for k in 0..count {
            out.push(((x >> (6 * values.len() - 8 * (k + 1))) & 255) as u8);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---- models -------------------------------------------------------------------

const EFFORT_ORDER: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// What a request on a model needs of its listing.
#[derive(Clone, Debug, Default)]
pub struct ModelInfo {
    pub key: String,
    source: String,
    name: String,
    context: u64,
    windows: Vec<u64>,
    default_window: u64,
    thinks: bool,
    always_thinks: bool,
    efforts: Vec<String>,
    default_effort: String,
    config: Value,
}

fn model_info(raw: &Value) -> ModelInfo {
    let mut windows: Vec<u64> = Vec::new();
    let mut default_window = 0;
    if let Some(config) = raw["context_config"].as_object().or(raw["contextConfig"].as_object()) {
        for entry in config.values() {
            let Some(tokens) = entry["token_count"].as_u64().filter(|tokens| *tokens > 0) else { continue };
            if !windows.contains(&tokens) {
                windows.push(tokens);
            }
            if entry["is_default"] == true || entry["is_default"] == 1 || entry["is_default"] == "1" {
                default_window = tokens;
            }
        }
    }
    if windows.is_empty() {
        windows = raw["available_context_windows"].as_array().into_iter().flatten().filter_map(Value::as_u64).filter(|tokens| *tokens > 0).collect();
        default_window = raw["default_context_window"].as_u64().filter(|tokens| windows.contains(tokens)).unwrap_or(0);
    }
    windows.sort_unstable();
    windows.dedup();
    let mut info = ModelInfo {
        key: text(&raw["key"]),
        source: text(&raw["source"]),
        name: [&raw["display_name"], &raw["key"]].into_iter().map(text).find(|name| !name.is_empty()).unwrap_or_default(),
        context: windows.last().copied().or(raw["max_input_tokens"].as_u64()).unwrap_or(0),
        windows,
        default_window,
        thinks: raw["is_reasoning"] == true,
        efforts: raw["reasoning_efforts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
        config: raw.clone(),
        ..Default::default()
    };
    if let Some(thinking) = raw["thinking_config"].as_object() {
        info.thinks = thinking.get("enabled").is_some_and(|enabled| !enabled.is_null());
        info.always_thinks = info.thinks && thinking.get("disabled").is_none_or(Value::is_null);
        info.efforts.clear();
        if info.thinks {
            for (name, effort) in raw["thinking_config"]["enabled"]["efforts"].as_object().into_iter().flatten() {
                info.efforts.push(name.clone());
                if effort["is_default"] == true {
                    info.default_effort = name.clone();
                }
            }
            let rank = |effort: &String| EFFORT_ORDER.iter().position(|known| known == effort).unwrap_or(EFFORT_ORDER.len());
            info.efforts.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
        }
    }
    info
}

/// The models an agent can pick: the "app" scene's, in its order, then any
/// "chat" model the app no longer lists after the model it followed there;
/// enabled ones, "auto" and "default" (which route inside Qoder) left out.
fn model_infos(listing: &Value) -> Vec<ModelInfo> {
    let mut scene: Vec<Value> = listing["app"].as_array().cloned().unwrap_or_default();
    let mut after: isize = -1;
    for model in listing["chat"].as_array().into_iter().flatten() {
        if let Some(index) = scene.iter().position(|known| known["key"] == model["key"]) {
            after = index as isize;
            continue;
        }
        after += 1;
        scene.insert(after as usize, model.clone());
    }
    scene.iter().filter(|model| model["enable"] == true && !matches!(text(&model["key"]).as_str(), "" | "auto" | "default"))
        .map(model_info).collect()
}

/// The account's listings, by deployment and uid, for what a request carries.
fn listings() -> &'static Mutex<HashMap<String, (Vec<ModelInfo>, Instant)>> {
    static LISTINGS: std::sync::OnceLock<Mutex<HashMap<String, (Vec<ModelInfo>, Instant)>>> = std::sync::OnceLock::new();
    LISTINGS.get_or_init(Default::default)
}

fn listing_key(creds: &Creds) -> String {
    format!("{}\0{}", creds.vpc, creds.uid)
}

/// The account's own listing, as Qoder's client asks it; kept half an hour.
pub async fn model_list(client: &Client, site: &Site, creds: &Creds, again: bool) -> Result<Vec<ModelInfo>, String> {
    if !again {
        if let Some((models, at)) = listings().lock().unwrap().get(&listing_key(creds)) {
            if at.elapsed() < Duration::from_secs(1800) {
                return Ok(models.clone());
            }
        }
    }
    let url = format!("{}{MODELS_PATH}", site.api);
    let mut request = client.get(&url).timeout(Duration::from_secs(15));
    for (name, value) in cosy_headers(&url, creds, "") {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|_| format!("cannot reach {}", site.name))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("{} models: HTTP {status}", site.name));
    }
    let models = model_infos(&response.json().await.map_err(|_| format!("{} models: not JSON", site.name))?);
    listings().lock().unwrap().insert(listing_key(creds), (models.clone(), Instant::now()));
    Ok(models)
}

pub async fn fetch_models(client: &Client, site: &Site, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let models = model_list(client, site, creds, true).await?;
    Ok(models.into_iter().map(|model| {
        let mut reasoning = Vec::new();
        if model.thinks {
            if !model.always_thinks {
                reasoning.push("none".to_owned());
            }
            reasoning.extend(model.efforts.iter().cloned());
            if model.efforts.is_empty() {
                reasoning.push("high".to_owned());
            }
        }
        SourceModel { id: model.key, name: model.name, context_window: (model.context > 0).then_some(model.context), reasoning, ..Default::default() }
    }).collect())
}

// ---- the request --------------------------------------------------------------

const EFFORT_RANK: [&str; 8] = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

fn fit_effort(want: &str, levels: &[String]) -> String {
    let want = if want == "ultra" && !levels.iter().any(|level| level == "ultra") { "max" } else { want };
    if levels.is_empty() || levels.iter().any(|level| level == want) {
        return want.to_owned();
    }
    let Some(at) = EFFORT_RANK.iter().position(|level| *level == want) else { return want.to_owned() };
    let mut best = (want.to_owned(), usize::MAX);
    for level in levels.iter().filter(|level| *level != "none") {
        let Some(index) = EFFORT_RANK.iter().position(|known| known == level) else { continue };
        let distance = index.abs_diff(at);
        if distance < best.1 || distance == best.1 && index > at {
            best = (level.clone(), distance);
        }
    }
    best.0
}

/// Whether to think and at which of the model's efforts ("" for none).
fn effort_for(asked: &str, model: &ModelInfo) -> (bool, String) {
    let asked = asked.to_ascii_lowercase();
    let want = if EFFORT_RANK.contains(&asked.as_str()) { asked.as_str() } else { "" };
    if !model.thinks || want == "none" && !model.always_thinks {
        return (false, String::new());
    }
    if model.efforts.is_empty() {
        return (true, String::new());
    }
    match want {
        "" => (true, model.default_effort.clone()),
        "none" => (true, model.efforts[0].clone()),
        want => (true, fit_effort(want, &model.efforts)),
    }
}

fn blocks_of(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) if !text.is_empty() => vec![json!({"type": "text", "text": text})],
        Value::Array(parts) => parts.iter().filter_map(|part| match part["type"].as_str() {
            Some("text") if !part["text"].is_null() => Some(json!({"type": "text", "text": part["text"]})),
            Some("image_url") => part["image_url"].as_str().or(part["image_url"]["url"].as_str())
                .map(|url| json!({"type": "image_url", "image_url": {"url": url}})),
            _ => None,
        }).collect(),
        _ => Vec::new(),
    }
}

fn text_of(content: &Value) -> String {
    blocks_of(content).iter().filter_map(|block| block["text"].as_str()).collect()
}

/// The chat's turns as Qoder takes them; a tool turn holds text only, so the
/// images a tool returned follow in a user turn.
fn qoder_messages(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut seen: Vec<Value> = Vec::new();
    for message in messages {
        let role = message["role"].as_str().unwrap_or_default();
        if role == "system" || role == "developer" {
            continue;
        }
        if role != "user" && role != "tool" && !seen.is_empty() {
            out.push(json!({"role": "user", "content": std::mem::take(&mut seen)}));
        }
        if role == "tool" {
            let id = message["tool_call_id"].as_str().unwrap_or_default();
            let mut content = text_of(&message["content"]);
            let images: Vec<Value> = blocks_of(&message["content"]).into_iter().filter(|block| block["type"] == "image_url").collect();
            if !images.is_empty() {
                let call = format!("tool call {id}");
                let of = match names.get(id).cloned().or_else(|| message["name"].as_str().map(str::to_owned)) {
                    Some(name) => format!("{name} ({call})"),
                    None => call,
                };
                seen.push(json!({"type": "text", "text": format!("[From the result of {of}:]")}));
                let note = if images.len() == 1 { "[The tool returned an image; it follows in the next message.]".to_owned() }
                    else { format!("[The tool returned {} images; they follow in the next message.]", images.len()) };
                seen.extend(images);
                content = if content.trim().is_empty() { format!("{content}{note}") } else { format!("{content}\n\n{note}") };
            }
            out.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
            continue;
        }
        if role == "assistant" && message["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty()) {
            let calls: Vec<Value> = message["tool_calls"].as_array().into_iter().flatten().map(|call| {
                let id = call["id"].as_str().filter(|id| !id.is_empty()).map_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()), str::to_owned);
                names.insert(id.clone(), call["function"]["name"].as_str().unwrap_or_default().to_owned());
                let arguments = match &call["function"]["arguments"] {
                    Value::String(text) => text.clone(),
                    other => if other.is_null() { "{}".to_owned() } else { other.to_string() },
                };
                json!({"id": id, "type": "function", "function": {"name": call["function"]["name"], "arguments": arguments}})
            }).collect();
            out.push(json!({"role": "assistant", "content": text_of(&message["content"]), "tool_calls": calls}));
            continue;
        }
        let mut blocks = blocks_of(&message["content"]);
        if role == "assistant" {
            blocks.retain(|block| block["type"] == "text");
        }
        if blocks.is_empty() {
            continue;
        }
        if role == "user" && !seen.is_empty() {
            let mut joined = std::mem::take(&mut seen);
            joined.extend(blocks);
            blocks = joined;
        }
        out.push(json!({"role": role, "content": blocks}));
    }
    if !seen.is_empty() {
        out.push(json!({"role": "user", "content": seen}));
    }
    out
}

/// The context window a request is sent: the default while the request
/// (a token for every 3 bytes, and the reply's max_tokens) fits it, else the
/// smallest that holds it, else the largest.
fn window_for(chat: &Value, model: &ModelInfo, reply: u64) -> u64 {
    if model.windows.is_empty() {
        return model.context;
    }
    let base = [model.default_window, model.config["max_input_tokens"].as_u64().unwrap_or(0), model.context].into_iter().find(|window| *window > 0).unwrap_or(0);
    let size = json!({"m": chat["messages"], "t": chat["tools"]}).to_string().len() as u64;
    let need = size.div_ceil(3) + reply;
    if base > 0 && need <= base {
        return base;
    }
    model.windows.iter().copied().find(|window| *window >= need).unwrap_or(*model.windows.last().expect("windows"))
}

/// The plaintext agent_chat_generation takes for a chat completion.
pub fn qoder_body(chat: &Value, model: &ModelInfo) -> Value {
    let messages = chat["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
    let system: Vec<String> = messages.iter().filter(|message| message["role"] == "system" || message["role"] == "developer")
        .map(|message| text_of(&message["content"])).filter(|text| !text.is_empty()).collect();
    let tools: Vec<&Value> = if chat["tool_choice"] == "none" { Vec::new() } else {
        chat["tools"].as_array().into_iter().flatten().filter(|tool| tool["type"] == "function" && tool["function"]["name"].is_string()).collect()
    };
    let mut system_text = if system.is_empty() { QODER_SYS.to_owned() } else { format!("{QODER_SYS}\n\n{}", system.join("\n\n")) };
    if chat["tool_choice"] == "required" && !tools.is_empty() {
        system_text.push_str("\nYou must call an available function in this response.");
    }
    let sys = json!({"type": "text", "text": system_text});
    let (thinking, effort) = effort_for(chat["reasoning_effort"].as_str().unwrap_or_default(), model);
    let max_tokens = chat["max_completion_tokens"].as_u64().or(chat["max_tokens"].as_u64()).unwrap_or(32_000);
    let mut parameters = json!({"enable_thinking": thinking, "max_tokens": max_tokens});
    if !effort.is_empty() {
        parameters["reasoning_effort"] = json!(effort);
    }
    let window = window_for(chat, model, max_tokens);
    if window > 0 {
        parameters["context_length"] = json!(window);
    }
    let mut turns = vec![json!({"role": "system", "content": [sys.clone()]})];
    turns.extend(qoder_messages(messages));
    let mut body = json!({
        "parameters": parameters,
        "business": {"product": "app", "version": COSY_VERSION, "type": "agent", "id": uuid::Uuid::new_v4().simple().to_string(),
            "name": "agentharbor session", "begin_at": chrono::Utc::now().timestamp_millis(), "stage": "start"},
        "agent_id": "agent_common",
        "task_id": "common",
        "session_type": "app",
        "model_config": model.config,
        "system": [sys],
        "messages": turns,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(|tool| json!({"type": "function", "function": {
            "name": tool["function"]["name"], "description": tool["function"]["description"].as_str().unwrap_or_default(),
            "parameters": if tool["function"]["parameters"].is_object() { tool["function"]["parameters"].clone() } else { json!({"type": "object", "properties": {}}) },
        }})).collect());
    }
    body
}

/// A chat request as Qoder's: its URL below the API host, signed headers and
/// encoded body. `Err` for a model the listing doesn't have.
pub fn encode(api: &str, creds: &Creds, models: &[ModelInfo], chat: &Value, model: &str) -> Result<(Vec<u8>, Vec<(&'static str, String)>), String> {
    let info = models.iter().find(|info| info.key == model).ok_or_else(|| format!("unknown or disabled model \"{model}\""))?;
    let wire = encode_body(&qoder_body(chat, info).to_string());
    let mut headers = cosy_headers(&format!("{}/{CHAT_PATH}", api.trim_end_matches('/')), creds, &wire);
    headers.retain(|(name, _)| *name != "accept");
    headers.extend([("accept", "text/event-stream".to_owned()), ("cache-control", "no-cache".to_owned()),
        ("x-model-key", info.key.clone()), ("x-model-source", info.source.clone())]);
    Ok((wire.into_bytes(), headers))
}

// ---- the reply ----------------------------------------------------------------

const CALL_OPEN: &str = "<tool_call>";
const CALL_CLOSE: &str = "</tool_call>";
const CUT_SHORT: &str = "the model's tool call, written as text, was cut short; it is not sent on as the answer";
const UNREADABLE: &str = "the model's tool call, written as text, can't be read; it is not sent on as the answer";

/// One tool-call block: JSON {name, arguments}, or the XML function form.
fn parse_call(block: &str) -> Option<(String, String)> {
    if let Ok(value) = serde_json::from_str::<Value>(block.trim()) {
        if let (Some(name), Some(arguments)) = (value["name"].as_str().map(str::trim).filter(|name| !name.is_empty()), value["arguments"].as_object()) {
            return Some((name.to_owned(), Value::Object(arguments.clone()).to_string()));
        }
    }
    let start = block.find("<function=")?;
    let rest = &block[start + "<function=".len()..];
    let end = rest.find('>')?;
    let name = rest[..end].trim();
    let body_end = rest.find("</function>")?;
    if name.is_empty() || body_end < end {
        return None;
    }
    let mut body = &rest[end + 1..body_end];
    let mut args = serde_json::Map::new();
    while let Some(open) = body.find("<parameter=") {
        let after = &body[open + "<parameter=".len()..];
        let Some(close_name) = after.find('>') else { break };
        let key = after[..close_name].trim().to_owned();
        let value_start = &after[close_name + 1..];
        let Some(value_end) = value_start.find("</parameter>") else { break };
        let value = value_start[..value_end].trim();
        if !key.is_empty() {
            args.insert(key, serde_json::from_str(value).unwrap_or_else(|_| json!(value)));
        }
        body = &value_start[value_end + "</parameter>".len()..];
    }
    Some((name.to_owned(), Value::Object(args).to_string()))
}

/// Text ends inside a code fence or an inline code span: a `<tool_call>`
/// there is prose about one.
fn quoted(text: &str) -> bool {
    if text.matches("```").count() % 2 == 1 {
        return true;
    }
    let line = text.rsplit('\n').next().unwrap_or_default().replace("```", "");
    line.matches('`').count() % 2 == 1
}

fn callish(block: &str) -> bool {
    block.trim_start().starts_with('{') || block.contains("<function=")
}

/// Reads Qoder's SSE into chat events: each data line's envelope has a
/// "body" that is an OpenAI chunk, whose content may hold tool calls.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    text_buf: String,
    call_buf: String,
    in_call: bool,
    saw_tool: bool,
    sent: String,
    native: Option<u64>,
    native_id: String,
    finish: String,
    usage: Option<(u64, u64, u64, u64)>,
    done: bool,
}

impl Decoder {
    fn text(&mut self, out: &mut Vec<Event>, text: String) {
        if !text.is_empty() {
            self.sent.push_str(&text);
            out.push(Event::Text(text));
        }
    }

    fn call(&mut self, out: &mut Vec<Event>, name: String, args: String) {
        self.saw_tool = true;
        let id = format!("call_{}", uuid::Uuid::new_v4().simple());
        out.push(Event::ToolStart { id: id.clone(), name });
        out.push(Event::ToolArgs { id, text: args });
    }

    fn fail(&mut self, out: &mut Vec<Event>, message: &str) {
        self.done = true;
        out.push(Event::Error(message.to_owned()));
    }

    /// Text in, with whole tool-call blocks lifted out.
    fn content(&mut self, mut input: String, out: &mut Vec<Event>) {
        while !input.is_empty() && !self.done {
            if !self.in_call {
                let combined = std::mem::take(&mut self.text_buf) + &input;
                match combined.find(CALL_OPEN) {
                    None => {
                        let keep = (1..CALL_OPEN.len()).rev().find(|count| combined.ends_with(&CALL_OPEN[..*count])).unwrap_or(0);
                        let (said, held) = combined.split_at(combined.len() - keep);
                        self.text(out, said.to_owned());
                        self.text_buf = held.to_owned();
                        return;
                    }
                    Some(at) => {
                        self.text(out, combined[..at].to_owned());
                        input = combined[at + CALL_OPEN.len()..].to_owned();
                        self.in_call = true;
                    }
                }
                continue;
            }
            let combined = std::mem::take(&mut self.call_buf) + &input;
            let Some(end) = combined.find(CALL_CLOSE) else {
                self.call_buf = combined;
                return;
            };
            let block = &combined[..end];
            match parse_call(block) {
                Some((name, args)) => self.call(out, name, args),
                None if callish(block) && !quoted(&self.sent) => {
                    self.in_call = false;
                    return self.fail(out, UNREADABLE);
                }
                None => self.text(out, format!("{CALL_OPEN}{block}{CALL_CLOSE}")),
            }
            input = combined[end + CALL_CLOSE.len()..].to_owned();
            self.in_call = false;
        }
    }

    fn flush(&mut self, out: &mut Vec<Event>) {
        if self.in_call {
            self.in_call = false;
            let block = std::mem::take(&mut self.call_buf);
            if callish(&block) && !quoted(&self.sent) {
                match parse_call(&block) {
                    Some((name, args)) => self.call(out, name, args),
                    None => self.fail(out, CUT_SHORT),
                }
                return;
            }
            self.text(out, format!("{CALL_OPEN}{block}"));
        }
        let held = std::mem::take(&mut self.text_buf);
        self.text(out, held);
    }

    fn line(&mut self, line: &str, out: &mut Vec<Event>) {
        if self.done {
            return;
        }
        let Some(payload) = line.trim().strip_prefix("data:") else { return };
        let Ok(envelope) = serde_json::from_str::<Value>(payload.trim()) else { return };
        let inner = envelope["body"].as_str().unwrap_or_default();
        if let Some(status) = envelope["statusCodeValue"].as_i64().filter(|status| *status != 200) {
            let message = failure_message(status as u16, if inner.is_empty() { payload } else { inner });
            return self.fail(out, &message);
        }
        if inner.trim() == "[DONE]" {
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(inner) else { return };
        let delta = &chunk["choices"][0]["delta"];
        if let Some(reasoning) = delta["reasoning_content"].as_str().filter(|text| !text.is_empty()) {
            out.push(Event::Reasoning(reasoning.to_owned()));
        }
        if let Some(content) = delta["content"].as_str() {
            self.content(content.to_owned(), out);
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0);
            if self.native != Some(index) {
                self.native = Some(index);
                self.flush(out);
                self.saw_tool = true;
                self.native_id = call["id"].as_str().filter(|id| !id.is_empty()).map_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()), str::to_owned);
                out.push(Event::ToolStart { id: self.native_id.clone(), name: call["function"]["name"].as_str().unwrap_or_default().to_owned() });
            }
            if let Some(args) = call["function"]["arguments"].as_str().filter(|args| !args.is_empty()) {
                out.push(Event::ToolArgs { id: self.native_id.clone(), text: args.to_owned() });
            }
        }
        if let Some(finish) = chunk["choices"][0]["finish_reason"].as_str().filter(|finish| !finish.is_empty()) {
            self.finish = finish.to_owned();
        }
        let usage = &chunk["usage"];
        if usage.is_object() {
            self.usage = Some((usage["prompt_tokens"].as_u64().unwrap_or(0), usage["completion_tokens"].as_u64().unwrap_or(0),
                usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0), usage["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0)));
        }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            self.line(&String::from_utf8_lossy(&line), out);
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Event>) {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned();
        self.line(&rest, out);
        if self.done {
            return;
        }
        self.flush(out);
        if self.done {
            return;
        }
        self.done = true;
        out.push(Event::Finish(if self.saw_tool { "tool_calls" } else if self.finish == "length" { "length" } else { "stop" }));
        if let Some((input, output, cached, reasoning)) = self.usage {
            out.push(Event::Usage { input, output, cached, reasoning });
        }
    }
}

const QUOTA_CODES: [&str; 10] = ["110", "112", "113", "114", "115", "116", "117", "118", "119", "122"];
const REQUEST_CODES: [&str; 8] = ["406", "416", "430", "80411", "80412", "100400", "100401", "100403"];

/// A Qoder failure's words, read as Qoder's client reads its codes.
fn failure_message(status: u16, body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = text(&value["code"]);
    let mut message = value["message"].as_str().map_or_else(|| body.trim().to_owned(), str::to_owned);
    let queue: Value = if value["queue"].is_object() { value["queue"].clone() } else { serde_json::from_str(&message).unwrap_or(Value::Null) };
    if code == "10605" || queue["isQueued"] == true || queue["serviceAvailable"] == false {
        let wait = queue["retryAfterSeconds"].as_f64().filter(|wait| *wait > 0.0).map_or(30, |wait| wait.ceil() as u64);
        let model = queue["modelKey"].as_str().map(|model| format!(" for {model}")).unwrap_or_default();
        let what = if queue["isQueued"] == true { "queued the request" } else { "is busy" };
        return format!("Qoder {what}{model}; try again in {wait}s");
    }
    if message.is_empty() {
        message = format!("HTTP {status}");
    }
    if QUOTA_CODES.contains(&code.as_str()) {
        return format!("usage limit reached: {message}");
    }
    if REQUEST_CODES.contains(&code.as_str()) {
        return format!("Qoder refused the request ({code}): {message}");
    }
    message
}

/// What the user can do about a Qoder refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status < 400 {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = self::text(&value["code"]);
    if code == "10605" || text.contains("isQueued") {
        return Some(failure_message(status, &text));
    }
    if QUOTA_CODES.contains(&code.as_str()) {
        return Some("this Qoder account's credits are used up; check in daily for more, or add another account".into());
    }
    if code == "105" || (status == 401 || status == 403) && code.is_empty() {
        return Some("Qoder refused the sign-in; sign in again on the Accounts page".into());
    }
    None
}

// ---- usage and check-in --------------------------------------------------------

/// The account's credits, read with the device token (rotated once Qoder
/// refuses it). Also gives a rotated device pair, to keep.
pub async fn quota(client: &Client, site: &Site, creds: &Creds) -> (Quota, Option<String>, Option<(String, String)>) {
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), unit: "credits".into(), ..Default::default() };
    let token = if creds.device_token.is_empty() { creds.access.clone() } else { creds.device_token.clone() };
    let ask = |token: String| async move {
        let response = client.get(format!("{}/sash/api/v2/me/usage", site.openapi)).header("accept", "application/json")
            .header("authorization", format!("Bearer {token}")).header("cosy-clienttype", "10").header("user-agent", "Qoder")
            .timeout(Duration::from_secs(20)).send().await.map_err(|_| (0u16, "cannot reach Qoder".to_owned()))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err((status, format!("Qoder usage: HTTP {status}")));
        }
        response.json::<Value>().await.map_err(|_| (status, "Qoder usage: not JSON".to_owned()))
    };
    let mut rotated = None;
    let mut reply = ask(token).await;
    if matches!(reply, Err((401 | 403, _))) && !creds.device_refresh.is_empty() {
        if let Ok((token, refresh, _)) = refresh_device(client, site, &creds.device_refresh, false).await {
            rotated = Some((token.clone(), refresh));
            reply = ask(token).await;
        }
    }
    let envelope = match reply {
        Ok(envelope) => envelope,
        Err((_, error)) => {
            quota.error = Some(error);
            return (quota, None, rotated);
        }
    };
    if envelope["displayMode"] == "enterprise" {
        return (quota, Some("Enterprise".into()), rotated);
    }
    let usage = &envelope["qoderUsage"];
    let field = |a: &str, b: &str| if usage.get(a).is_some() { usage[a].clone() } else { usage[b].clone() };
    for bucket in [field("userQuota", "user_quota"), field("addOnQuota", "add_on_quota")] {
        let Some(total) = bucket["total"].as_f64().or(bucket["cap"].as_f64()).filter(|total| *total > 0.0) else { continue };
        let Some(used) = bucket["used"].as_f64().or_else(|| Some(total - bucket["remaining"].as_f64()?)) else { continue };
        quota.limit += total;
        quota.used += used.max(0.0);
    }
    let plan = field("userType", "user_type").as_str().map(str::to_owned);
    quota.paid = plan.as_ref().map(|plan| !plan.to_ascii_lowercase().contains("free"));
    (quota, plan, rotated)
}

/// Qoder's native device identity helper, installed by its desktop app or
/// cached by its CLI.
fn runtime_info() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("QODER_RUNTIME_INFO").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let home = dirs::home_dir()?;
    let name = if cfg!(windows) { "runtime-info.exe" } else { "runtime-info" };
    let mut candidates = Vec::new();
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
        candidates.push(local.join("Programs").join("Qoder").join("resources").join("umid").join(name));
    } else if cfg!(target_os = "macos") {
        for dir in [PathBuf::from("/Applications"), home.join("Applications")] {
            candidates.push(dir.join("Qoder.app").join("Contents").join("Resources").join("umid").join(name));
        }
    } else {
        candidates.push(PathBuf::from("/opt/Qoder/resources/umid").join(name));
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }
    let config = std::env::var_os("QODER_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let base = std::env::var_os("QODER_CLI_HOME").map(PathBuf::from).unwrap_or_else(|| home.clone());
        base.join(std::env::var("QODER_CONFIG_DIR_NAME").unwrap_or_else(|_| ".qoder".into()))
    });
    let arch = oauth::node_arch();
    let mut found: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(config.join(".bin")).into_iter().flatten().flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let path = if cfg!(windows) {
            (file_name.starts_with(&format!("umid-win32-{arch}-")) && entry.path().is_dir()).then(|| entry.path().join(name))
        } else {
            (file_name.starts_with(&format!("runtime-info-{}-{arch}-", oauth::node_platform())) && !file_name.ends_with(".tmp")).then(|| entry.path())
        };
        if let Some(path) = path.filter(|path| path.is_file()) {
            let modified = path.metadata().and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH);
            found.push((path, modified));
        }
    }
    found.into_iter().max_by_key(|(_, modified)| *modified).map(|(path, _)| path)
}

/// The SDK identity for the signed-in uid, as Qoder reads it: environment 3
/// is international; on Windows and macOS the uid goes over stdin. Kept an
/// hour per uid, as Qoder's desktop client renews it hourly.
async fn campaign_machine(uid: &str) -> Result<Vec<(&'static str, String)>, String> {
    static MACHINES: Mutex<Vec<(String, Vec<(&'static str, String)>, Instant)>> = Mutex::new(Vec::new());
    if let Some((_, headers, _)) = MACHINES.lock().unwrap().iter().find(|(known, _, at)| known == uid && at.elapsed() < Duration::from_secs(3600)) {
        return Ok(headers.clone());
    }
    let exe = runtime_info().ok_or("accountSources.errors.qoderRuntime")?;
    let linux = cfg!(target_os = "linux");
    let mut command = tokio::process::Command::new(&exe);
    command.arg("3");
    if !linux {
        command.arg("--account-stdin");
    }
    command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|_| "Qoder check-in: could not run Qoder's device identity helper".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if !linux {
            let _ = stdin.write_all(format!("{}\n", json!({"account": uid})).as_bytes()).await;
        }
    }
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output()).await
        .map_err(|_| "Qoder check-in: the device identity helper didn't answer".to_owned())?
        .map_err(|_| "Qoder check-in: could not read device identity".to_owned())?;
    let first = String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or_default().to_owned();
    let identity: Value = serde_json::from_str(&first).map_err(|_| "Qoder check-in: invalid device identity from Qoder; restart or update Qoder".to_owned())?;
    let mut headers = Vec::new();
    for (header, field) in [("cosy-machinetoken", "machineToken"), ("cosy-machinetype", "machineType")] {
        let value = identity[field].as_str().map(str::trim).unwrap_or_default();
        if value.is_empty() || value.len() > 4096 || !value.bytes().all(|byte| (0x20..0x7f).contains(&byte)) {
            return Err("Qoder check-in: invalid device identity from Qoder; restart or update Qoder".into());
        }
        headers.push((header, value.to_owned()));
    }
    let mut machines = MACHINES.lock().unwrap();
    machines.retain(|(known, _, _)| known != uid);
    machines.push((uid.to_owned(), headers.clone(), Instant::now()));
    Ok(headers)
}

/// When a time Qoder gives falls, in seconds or milliseconds.
fn seconds_of(value: &Value) -> i64 {
    let number = value.as_f64().unwrap_or(0.0);
    if number > 1e12 { (number / 1000.0) as i64 } else { number as i64 }
}

/// Claim each of the account's claimable daily credits (campaigns of
/// actionType CLAIM_BENEFIT) on the device token. A claimed one holds until
/// its window ends, which need not be Beijing midnight.
pub async fn check_in(client: &Client, site: &Site, creds: &Creds) -> Checkin {
    let machine = if site.kind == SourceKind::Qoder {
        match campaign_machine(&creds.uid).await {
            Ok(headers) => headers,
            Err(error) => return Checkin::failed(error, false),
        }
    } else {
        Vec::new()
    };
    let token = if creds.device_token.is_empty() { creds.access.clone() } else { creds.device_token.clone() };
    let call = |method: Method, url: String| {
        let mut request = client.request(method.clone(), url).header("accept", "application/json").header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}")).header("cosy-clienttype", "10").header("user-agent", "Qoder").timeout(Duration::from_secs(20));
        for (name, value) in &machine {
            request = request.header(*name, value);
        }
        if method == Method::POST {
            request = request.body("{}");
        }
        request
    };
    let list = match call(Method::GET, format!("{}{CAMPAIGNS}", site.openapi)).send().await {
        Ok(response) if response.status().is_success() => response.json::<Value>().await.unwrap_or(Value::Null),
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => return Checkin::failed("Qoder's sign-in has expired — sign in again", false),
        Ok(response) => return Checkin::failed(format!("HTTP {}", response.status().as_u16()), false),
        Err(error) => return Checkin::failed(error.to_string(), true),
    };
    let now = chrono::Utc::now().timestamp();
    let daily: Vec<&Value> = list["campaigns"].as_array().into_iter().flatten().filter(|campaign| campaign["actionType"] == "CLAIM_BENEFIT").collect();
    let active = |campaign: &Value| now >= seconds_of(&campaign["startAt"]) && now < seconds_of(&campaign["endAt"]);
    let until = daily.iter().filter(|campaign| active(campaign)).map(|campaign| seconds_of(&campaign["endAt"])).min();
    let (mut credit, mut claimed, mut failure) = (0.0, 0, None);
    for campaign in daily.iter().filter(|campaign| campaign["claimStatus"] == "CLAIMABLE") {
        let Some(id) = campaign["campaignId"].as_str().filter(|id| !id.is_empty()) else { continue };
        let url = format!("{}{CAMPAIGNS}/{}/claim", site.openapi, oauth::encode_uri_component(id));
        let reply = match call(Method::POST, url).send().await {
            Ok(response) if response.status().is_success() => response.json::<Value>().await.unwrap_or(Value::Null),
            Ok(response) => {
                failure = Some(format!("HTTP {}", response.status().as_u16()));
                continue;
            }
            Err(error) => {
                failure = Some(error.to_string());
                continue;
            }
        };
        let got = if reply["data"].is_object() { &reply["data"] } else { &reply };
        if got["status"] != "CLAIMED" {
            failure = Some(format!("the claim answered {}", got["status"]));
            continue;
        }
        claimed += 1;
        credit += got["benefit"]["amount"].as_f64().filter(|amount| *amount > 0.0).or(campaign["benefit"]["amount"].as_f64()).unwrap_or(0.0);
    }
    let mut result = match (failure, claimed) {
        (Some(why), _) => Checkin::failed(why, false),
        (None, count) if count > 0 => Checkin::claimed(credit, 0),
        _ if daily.is_empty() => Checkin::outcome(Outcome::Inactive),
        _ => match daily.iter().find(|campaign| campaign["claimStatus"] == "CLAIMED" && active(campaign)) {
            Some(campaign) => Checkin::done(campaign["benefit"]["amount"].as_f64().unwrap_or(0.0), 0),
            None => Checkin::failed("today's credits aren't offered yet", false),
        },
    };
    if result.checked() {
        result.valid_until = until;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_codec_round_trips() {
        for text in ["", "a", "{\"model\":\"ultimate\",\"messages\":[]}", "多字节 ✓ text"] {
            assert_eq!(decode_body(&encode_body(text)), text);
        }
        assert_eq!(encode_body("abc").len() % 4, 0);
        // as Qoder's client writes them (the plugin's encodeBody)
        assert_eq!(encode_body("{\"model\":\"ultimate\"}"), "t#xCLBEw$oByBEFrJHLmYKtu(CLu");
        assert_eq!(encode_body("多字节 ✓"), "cNzf$$(xyBsoBlQP,PQP");
    }

    #[test]
    fn the_envelope_is_signed_over_payload_key_time_body_and_path() {
        let creds = Creds { uid: "u1".into(), access: "jt".into(), machine_id: "m-1".into(), ..Default::default() };
        let headers = cosy_headers("https://api3.qoder.sh/algo/api/v2/model/list?Encode=1", &creds, "");
        let get = |name: &str| headers.iter().find(|(key, _)| *key == name).map(|(_, value)| value.clone()).unwrap();
        let authorization = get("authorization");
        let parts: Vec<&str> = authorization.trim_start_matches("Bearer COSY.").split('.').collect();
        assert_eq!(parts.len(), 2);
        let expected = oauth::md5_hex(&format!("{}\n{}\n{}\n\n/api/v2/model/list", parts[0], get("cosy-key"), get("cosy-date")));
        assert_eq!(parts[1], expected);
        assert_eq!(get("cosy-user"), "u1");
        use base64::Engine;
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(get("cosy-key")).unwrap().len(), 128, "a 1024-bit RSA block");
        // the account's blob as Node's aes-128-cbc writes it, the key its own IV
        let key = *b"0123456789abcdef";
        assert_eq!(base64::engine::general_purpose::STANDARD.encode(aes_cbc(&key, &key, br#"{"uid":"u1","aid":""}"#)),
            "D+wDizmqo1CTud0uPrzdrS08Bn7l3remYxgPGZiymUs=");
    }

    #[test]
    fn the_listing_s_app_scene_leads_and_chat_s_extras_follow() {
        let listing = json!({
            "app": [{"key": "auto", "enable": true}, {"key": "fast", "enable": true, "display_name": "Fast"}, {"key": "ultimate", "enable": true,
                "thinking_config": {"enabled": {"efforts": {"xhigh": {}, "high": {"is_default": true}, "low": {}}}},
                "context_config": {"200K": {"token_count": 200000, "is_default": true}, "1M": {"token_count": 1000000}}}],
            "chat": [{"key": "fast"}, {"key": "performance", "enable": true}, {"key": "off", "enable": false}],
        });
        let models = model_infos(&listing);
        assert_eq!(models.iter().map(|model| model.key.as_str()).collect::<Vec<_>>(), ["fast", "performance", "ultimate"]);
        let ultimate = &models[2];
        assert_eq!(ultimate.efforts, ["low", "high", "xhigh"]);
        assert_eq!((ultimate.context, ultimate.default_window, ultimate.always_thinks), (1_000_000, 200_000, true));
        assert_eq!(effort_for("medium", ultimate), (true, "high".to_owned()));
        assert_eq!(effort_for("none", ultimate), (true, "low".to_owned()), "a model that always thinks thinks at its lowest");
        assert_eq!(effort_for("", ultimate), (true, "high".to_owned()));
        let small = json!({"messages": [{"role": "user", "content": "hi"}]});
        assert_eq!(window_for(&small, ultimate, 1000), 200_000);
        assert_eq!(window_for(&small, ultimate, 300_000), 1_000_000, "a reply too large for the default asks a larger window");
    }

    #[test]
    fn a_chat_becomes_qoder_s_request_with_its_line_first() {
        let model = ModelInfo { key: "fast".into(), config: json!({"key": "fast"}), ..Default::default() };
        let chat = json!({"model": "fast", "tool_choice": "required", "messages": [
            {"role": "system", "content": "agent rules"}, {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "shot", "arguments": {}}}]},
            {"role": "tool", "tool_call_id": "c1", "content": [{"type": "text", "text": "done"}, {"type": "image_url", "image_url": {"url": "data:x"}}]},
            {"role": "user", "content": "next"}],
            "tools": [{"type": "function", "function": {"name": "shot"}}]});
        let body = qoder_body(&chat, &model);
        let system = body["system"][0]["text"].as_str().unwrap();
        assert!(system.starts_with(QODER_SYS) && system.contains("agent rules") && system.ends_with("You must call an available function in this response."));
        let turns = body["messages"].as_array().unwrap();
        assert_eq!(turns[0]["role"], "system");
        assert_eq!(turns[2]["tool_calls"][0]["function"]["arguments"], "{}");
        assert!(turns[3]["content"].as_str().unwrap().contains("follows in the next message"));
        assert_eq!(turns[4]["content"][0]["text"], "[From the result of shot (tool call c1):]", "the tool's image comes before the user's own words");
        assert_eq!(turns[4]["content"][2]["text"], "next");
        assert_eq!(body["model_config"]["key"], "fast");
    }

    #[test]
    fn text_tool_calls_are_lifted_out_and_a_broken_one_fails() {
        let envelope = |content: &str| format!("data: {}\n", json!({"body": json!({"choices": [{"delta": {"content": content}}]}).to_string()}));
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        decoder.feed(envelope("let me look <tool_").as_bytes(), &mut out);
        decoder.feed(envelope("call><function=read><parameter=path>a.txt</parameter><parameter=n>3</parameter></function></tool_call>").as_bytes(), &mut out);
        decoder.feed(envelope("`<tool_call>` is how").as_bytes(), &mut out);
        decoder.finish(&mut out);
        assert_eq!(out[0], Event::Text("let me look ".into()));
        assert!(matches!(&out[1], Event::ToolStart { name, .. } if name == "read"));
        assert!(matches!(&out[2], Event::ToolArgs { text, .. } if serde_json::from_str::<Value>(text).unwrap() == json!({"path": "a.txt", "n": 3})));
        assert!(out.contains(&Event::Finish("tool_calls")));

        let mut broken = Decoder::default();
        let mut out = Vec::new();
        broken.feed(envelope("<tool_call>{\"name\": \"x\", \"argum").as_bytes(), &mut out);
        broken.finish(&mut out);
        assert_eq!(out.last(), Some(&Event::Error(CUT_SHORT.into())));

        let mut busy = Decoder::default();
        let mut out = Vec::new();
        let queued = json!({"statusCodeValue": 503, "body": json!({"code": "10605", "message": "{\"isQueued\":false,\"modelKey\":\"efficient\",\"retryAfterSeconds\":30,\"serviceAvailable\":false}"}).to_string()});
        busy.feed(format!("data: {queued}\n").as_bytes(), &mut out);
        assert!(matches!(&out[0], Event::Error(message) if message.contains("busy") && message.contains("30s")));
    }

    #[test]
    fn vpc_instances_are_names_or_their_hosts() {
        assert_eq!(vpc_instance("Acme").as_deref(), Some("acme"));
        assert_eq!(vpc_instance("https://acme-openapi.vpc.qoder.com.cn/").as_deref(), Some("acme"));
        assert!(vpc_instance("http://acme").is_none() && vpc_instance("acme.example.com").is_none() && vpc_instance("10.0.0.1").is_none());
    }
}
