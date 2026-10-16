//! Command Code (commandcode.ai) plans: Pro, GOAT, Max, Ultra, Go, Teams Pro.
//!
//! Ported from magpie's community plugin (MIT, see LICENSES/THIRD_PARTY.md).
//! An account is a Command Code API key, got as the CLI gets one (Studio
//! approves a key for this machine and posts it to a callback on
//! 127.0.0.1), taken from the CLI's own sign-in (`~/.commandcode/auth.json`,
//! read only, and read again for each request so `commandcode login` is
//! followed), or pasted. A key doesn't expire.
//!
//! Every plan but Go is served on the Provider API: chat completions and
//! Responses at `/provider/v1`, Anthropic's Messages at
//! `/provider/v1/messages`, the key in Authorization and `x-api-key`. Go's
//! key has no Provider API: it is taken only where the CLI asks,
//! `POST /alpha/generate`, in the CLI's own format, which this module writes
//! from a chat completion and reads back into one (a wire codec).
use super::{
    store::{Quota, SourceModel},
    wire::Event,
};
use crate::core::router::ModelProtocol;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use super::loopback::{page, read_incoming, respond};
use tokio::net::TcpListener;
use tokio::sync::watch;

const CLI_VERSION: &str = "1.72.2";
/// The CLI's `max_tokens` when none is asked.
const MAX_TOKENS: u64 = 64_000;
const ORIGINS: [&str; 2] = ["https://commandcode.ai", "https://staging.commandcode.ai"];
const SIGN_IN_WINDOW: Duration = Duration::from_secs(600);
pub const USAGE_URL: &str = "https://commandcode.ai/settings/usage";

fn api() -> String {
    super::endpoint_override(super::SourceKind::Commandcode).unwrap_or_else(|| "https://api.commandcode.ai".to_owned())
}

fn studio() -> String {
    super::endpoint_override(super::SourceKind::Commandcode).unwrap_or_else(|| "https://commandcode.ai".to_owned())
}

/// The Provider API.
pub fn provider_base() -> String {
    format!("{}/provider/v1", api())
}

/// Where Go's `/alpha/generate` is asked from.
pub fn api_root() -> String {
    api()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub key: String,
    /// The plan's name ("Go", "Pro", "No plan"…), as last read; empty when unread.
    pub plan: String,
    pub plan_id: String,
    /// Taken from the CLI's sign-in: its key is read again for each request.
    pub from_cli: bool,
}

fn cli_auth_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".commandcode").join("auth.json"))
}

/// The CLI's sign-in: its key and who it names.
fn read_cli() -> Result<Value, String> {
    let path = cli_auth_path().ok_or("no home folder")?;
    let text = std::fs::read_to_string(&path).map_err(|_| "accountSources.errors.commandcodeCli".to_owned())?;
    let value: Value = serde_json::from_str(&text).map_err(|_| format!("{} can't be read", path.display()))?;
    if value["apiKey"].as_str().is_none_or(|key| key.trim().is_empty()) {
        return Err("accountSources.errors.commandcodeCli".into());
    }
    Ok(value)
}

impl Creds {
    /// The key the account has now: one taken from the CLI's sign-in reads
    /// the CLI's again, keeping the one it was saved with while the CLI has none.
    pub fn live_key(&self) -> String {
        if self.from_cli {
            if let Ok(value) = read_cli() {
                if let Some(key) = value["apiKey"].as_str().map(str::trim).filter(|key| !key.is_empty()) {
                    return key.to_owned();
                }
            }
        }
        self.key.clone()
    }

    pub fn is_go(&self) -> bool {
        self.plan == "Go"
    }
}

/// The Provider API's headers: the key both ways Command Code takes it.
pub fn request_headers(key: &str) -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {key}")), ("x-api-key", key.to_owned())]
}

async fn account_json(client: &Client, path: &str, key: &str, timeout: Duration) -> Result<Value, u16> {
    let response = client.get(format!("{}{path}", api())).header("authorization", format!("Bearer {key}"))
        .header("accept", "application/json").timeout(timeout).send().await.map_err(|_| 0u16)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(status);
    }
    response.json().await.map_err(|_| 0)
}

/// Plan names by planId prefix, the longest first, with the dollars of
/// credits each gives a month (the CLI's getPlanInfo).
const PLAN_NAMES: [(&str, &str, f64); 8] = [
    ("individual-provider", "Provider", 15.0),
    ("individual-pro-v1", "Pro", 80.0),
    ("individual-goat", "GOAT", 70.0),
    ("individual-ultra", "Ultra", 300.0),
    ("individual-max", "Max", 150.0),
    ("individual-pro", "Pro", 30.0),
    ("individual-go", "Go", 10.0),
    ("teams-pro", "Teams Pro", 40.0),
];
const NO_PLAN: &str = "No plan";

fn plan_of(id: &str) -> Option<(&'static str, f64)> {
    let id = id.to_ascii_lowercase().replace('_', "-");
    PLAN_NAMES.iter().find(|(prefix, _, _)| id.starts_with(prefix)).map(|(_, name, monthly)| (*name, *monthly))
}

/// The account's plan as billing/subscriptions says: (id, name); `None`
/// when it can't be read.
async fn subscription(client: &Client, key: &str) -> Option<(String, String)> {
    let reply = account_json(client, "/alpha/billing/subscriptions", key, Duration::from_secs(30)).await.ok()?;
    // a 200 with success false when Command Code couldn't tell: unread, not no plan
    if reply["success"] == false {
        return None;
    }
    let data = &reply["data"];
    match data["planId"].as_str().filter(|id| !id.is_empty()) {
        Some(id) if !matches!(data["status"].as_str(), Some("canceled" | "incomplete_expired")) => {
            Some((id.to_owned(), plan_of(id).map_or_else(|| id.to_owned(), |(name, _)| name.to_owned())))
        }
        _ => Some((String::new(), NO_PLAN.to_owned())),
    }
}

pub struct SignedIn {
    pub creds: Creds,
    pub name: String,
    pub subject: String,
}

/// Name a key's account with whoami and read its plan. Only whoami turning
/// the key down fails: it has answered a key just made with a 500, so one
/// that errs is asked again a few times, then passed over for the name
/// Studio sent with the key.
async fn signed_in(client: &Client, key: &str, from_cli: bool, given: &Value) -> Result<SignedIn, String> {
    let mut me = Value::Null;
    for tries in 0.. {
        match account_json(client, "/alpha/whoami", key, Duration::from_secs(15)).await {
            Ok(reply) => {
                me = reply["user"].clone();
                break;
            }
            Err(401 | 403) => return Err("accountSources.errors.commandcodeKey".into()),
            Err(_) if tries >= 2 => break,
            Err(_) => tokio::time::sleep(Duration::from_millis(if cfg!(test) { 10 } else { 1000 })).await,
        }
    }
    let pick = |value: &Value, key: &str| value[key].as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned);
    let name = pick(&me, "userName").or_else(|| pick(given, "userName")).or_else(|| pick(&me, "email"))
        .or_else(|| pick(&me, "id")).or_else(|| pick(given, "userId")).unwrap_or_else(|| "Command Code".to_owned());
    let subject = pick(&me, "id").or_else(|| pick(given, "userId")).or_else(|| pick(&me, "email"))
        .unwrap_or_else(|| super::oauth::sha256_hex(key)[..16].to_owned());
    let (plan_id, plan) = subscription(client, key).await.unwrap_or_default();
    Ok(SignedIn { creds: Creds { key: key.to_owned(), plan, plan_id, from_cli }, name, subject })
}

/// A pasted key.
pub async fn sign_in_with_key(client: &Client, key: &str) -> Result<SignedIn, String> {
    signed_in(client, key.trim(), false, &Value::Null).await
}

/// The account `commandcode login` signed in to.
pub async fn sign_in_with_cli(client: &Client) -> Result<SignedIn, String> {
    let cli = read_cli()?;
    let key = cli["apiKey"].as_str().unwrap_or_default().trim().to_owned();
    signed_in(client, &key, true, &cli).await
}

pub struct BrowserSignIn {
    pub url: String,
    listener: TcpListener,
    state: String,
}

/// The CLI's browser sign-in: Studio asks the user to approve a key for
/// this machine, then posts it to a callback here.
pub async fn begin_browser_sign_in() -> Result<BrowserSignIn, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|_| "accounts.loginFailed".to_owned())?;
    let port = listener.local_addr().map_err(|_| "accounts.loginFailed".to_owned())?.port();
    let state = super::oauth::random_hex(24);
    let mut url = reqwest::Url::parse(&format!("{}/studio/auth/cli", studio())).map_err(|_| "accounts.loginFailed".to_owned())?;
    url.query_pairs_mut()
        .append_pair("callback", &format!("http://127.0.0.1:{port}/callback"))
        .append_pair("state", &state)
        .append_pair("mode", "redirect");
    Ok(BrowserSignIn { url: url.to_string(), listener, state })
}

/// Wait for Studio to post the approved key, then check it. The browser
/// comes back for its page after the key, and is answered while this waits.
pub async fn finish_browser_sign_in(client: &Client, sign_in: BrowserSignIn, mut cancel: watch::Receiver<bool>) -> Result<SignedIn, String> {
    let deadline = Instant::now() + SIGN_IN_WINDOW;
    let mut outcome: Option<Result<SignedIn, String>> = None;
    loop {
        if outcome.is_some() {
            // the page the browser is sent on to, a short while
            match tokio::time::timeout(Duration::from_secs(3), sign_in.listener.accept()).await {
                Ok(Ok((mut socket, _))) => {
                    if read_incoming(&mut socket).await.is_some() {
                        let body = match &outcome {
                            Some(Ok(done)) => page(true, "You're signed in", &format!("{} is signed in. You can close this tab.", done.name)),
                            _ => page(false, "Sign-in didn't finish", "Start it again in AgentHarbor."),
                        };
                        respond(&mut socket, "200 OK", &[("Content-Type", "text/html; charset=utf-8".into())], &body).await;
                        return outcome.expect("an outcome");
                    }
                    continue;
                }
                _ => return outcome.expect("an outcome"),
            }
        }
        let accept = tokio::select! {
            biased;
            _ = super::cancelled(&mut cancel) => return Err("accounts.loginCancelled".into()),
            accepted = tokio::time::timeout_at(deadline.into(), sign_in.listener.accept()) => accepted,
        };
        let Ok(Ok((mut socket, _))) = accept else { return Err("accounts.loginTimeout".into()) };
        let Some(request) = read_incoming(&mut socket).await else { continue };
        let origin = request.header("origin").filter(|origin| ORIGINS.contains(origin)).unwrap_or(ORIGINS[0]).to_owned();
        let mut cors = vec![
            ("Access-Control-Allow-Origin", origin),
            ("Access-Control-Allow-Methods", "GET, POST, OPTIONS".to_owned()),
            ("Access-Control-Allow-Headers", "Content-Type".to_owned()),
        ];
        if request.header("access-control-request-private-network") == Some("true") {
            cors.push(("Access-Control-Allow-Private-Network", "true".to_owned()));
        }
        match (request.method.as_str(), request.path.as_str()) {
            ("OPTIONS", _) => respond(&mut socket, "204 No Content", &cors, "").await,
            ("GET" | "POST", "/cancel") => {
                respond(&mut socket, "204 No Content", &cors, "").await;
                return Err("accounts.loginCancelled".into());
            }
            ("GET", "/callback/complete") => {
                respond(&mut socket, "200 OK", &[("Content-Type", "text/html; charset=utf-8".into())],
                    &page(false, "Sign-in didn't finish", "Start it again in AgentHarbor.")).await;
            }
            ("POST", "/callback") => {
                let json = request.header("content-type").is_some_and(|kind| kind.trim().to_ascii_lowercase().starts_with("application/json"));
                let got: Value = if json {
                    serde_json::from_slice(&request.body).unwrap_or(Value::Null)
                } else {
                    let pairs: serde_json::Map<String, Value> = reqwest::Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&request.body)))
                        .map(|url| url.query_pairs().map(|(key, value)| (key.into_owned(), Value::String(value.into_owned()))).collect())
                        .unwrap_or_default();
                    Value::Object(pairs)
                };
                if got["state"].as_str() != Some(sign_in.state.as_str()) {
                    respond(&mut socket, "403 Forbidden", &[("Content-Type", "text/html; charset=utf-8".into())],
                        &page(false, "This link isn't from this sign-in", "Start it again.")).await;
                    continue;
                }
                let result = if let Some(error) = got["error"].as_str().filter(|error| !error.is_empty()) {
                    Err(if error == "access_denied" { "accounts.loginCancelled".to_owned() } else {
                        format!("Command Code sign-in: {}", got["error_description"].as_str().unwrap_or(error))
                    })
                } else {
                    match got["apiKey"].as_str().map(str::trim).filter(|key| !key.is_empty()) {
                        Some(key) => signed_in(client, key, false, &got).await,
                        None => Err("Command Code sent back no key".into()),
                    }
                };
                if json {
                    let body = json!({"success": result.is_ok(), "error": result.as_ref().err()}).to_string();
                    let mut headers = cors.clone();
                    headers.push(("Content-Type", "application/json".into()));
                    respond(&mut socket, if result.is_ok() { "200 OK" } else { "400 Bad Request" }, &headers, &body).await;
                } else {
                    let location = format!("/callback/complete?state={}", super::oauth::encode_uri_component(&sign_in.state));
                    respond(&mut socket, "303 See Other", &[("Location", location), ("Cache-Control", "no-store".into())], "").await;
                }
                outcome = Some(result);
            }
            _ => respond(&mut socket, "404 Not Found", &[], "").await,
        }
    }
}

// ---- models -------------------------------------------------------------------

/// The plan's models when its list can't be asked: Claude on Messages, the
/// rest on chat completions.
fn default_models() -> Vec<SourceModel> {
    [("claude-sonnet-5", "Claude Sonnet 5", 1_000_000), ("claude-opus-5-5", "Claude Opus 5.5", 1_000_000), ("gpt-6-sol", "GPT-6 Sol", 1_050_000),
     ("deepseek/deepseek-v4-pro", "DeepSeek V4 Pro", 1_000_000), ("deepseek/deepseek-v4-flash", "DeepSeek V4 Flash", 1_000_000),
     ("moonshotai/Kimi-K3", "Kimi K3", 1_000_000), ("zai-org/GLM-5.3", "GLM-5.3", 1_000_000), ("MiniMaxAI/MiniMax-M3", "MiniMax M3", 1_000_000)]
        .into_iter().map(|(id, name, context)| SourceModel {
            id: id.into(), name: name.into(), context_window: Some(context),
            protocol: is_claude(id).then_some(ModelProtocol::AnthropicMessages), ..Default::default()
        }).collect()
}

const EFF5: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const EFF_LHM: &[&str] = &["low", "high", "max"];
const EFF_HM: &[&str] = &["high", "max"];
const EFF_LMH: &[&str] = &["low", "medium", "high"];
const EFF_LMX: &[&str] = &["low", "medium", "xhigh"];

/// The models the Go plan is let use, as the CLI's own table has it: they
/// give Command Code's list (which says neither) reasoning levels and
/// windows, and stand in when that list can't be had.
const GO_MODELS: &[(&str, &str, u64, &[&str])] = &[
    ("gpt-6-luna", "GPT-6 Luna", 1_050_000, EFF5), ("gpt-5.6-luna", "GPT-5.6 Luna", 1_050_000, EFF5),
    ("deepseek/deepseek-v4-pro", "DeepSeek V4 Pro", 1_000_000, EFF_HM), ("deepseek/deepseek-v4-flash", "DeepSeek V4 Flash", 1_000_000, EFF_HM),
    ("deepseek/deepseek-v4-flash-vision-exp", "DeepSeek V4 Flash Vision (exp)", 1_000_000, EFF_HM),
    ("deepseek/deepseek-v4-flash-fast", "DeepSeek V4 Flash Fast", 1_000_000, EFF_LHM),
    ("deepseek/deepseek-v4.1-flash", "DeepSeek V4.1 Flash", 1_000_000, EFF_LHM), ("deepseek/deepseek-v4.1-flash-fast", "DeepSeek V4.1 Flash Fast", 1_000_000, EFF_LHM),
    ("moonshotai/Kimi-K3", "Kimi K3", 1_000_000, EFF_LHM), ("moonshotai/Kimi-K2.7-Code", "Kimi K2.7 Code", 256_000, &[]),
    ("moonshotai/Kimi-K2.7-Code-Highspeed", "Kimi K2.7 Code HighSpeed", 262_000, &[]), ("moonshotai/Kimi-K2.6", "Kimi K2.6", 256_000, &[]),
    ("moonshotai/Kimi-K2.5", "Kimi K2.5", 256_000, &[]), ("z-ai/glm-5.3-flash", "GLM-5.3 Flash", 1_048_576, EFF_LHM),
    ("z-ai/glm-5.3-flashx", "GLM-5.3 FlashX", 1_000_000, EFF_LHM), ("zai-org/GLM-5.3", "GLM-5.3", 1_000_000, EFF_LHM),
    ("zai-org/GLM-5.2", "GLM-5.2", 1_000_000, EFF_HM), ("zai-org/GLM-5.2-Fast", "GLM-5.2 Fast", 1_000_000, &[]),
    ("zai-org/GLM-5.1", "GLM-5.1", 200_000, &[]), ("zai-org/GLM-5", "GLM-5", 200_000, &[]),
    ("MiniMaxAI/MiniMax-M3", "MiniMax M3", 1_000_000, EFF_LMH), ("MiniMaxAI/MiniMax-M2.7", "MiniMax M2.7", 200_000, &[]),
    ("MiniMaxAI/MiniMax-M2.5", "MiniMax M2.5", 200_000, &[]), ("xiaomi/mimo-v2.6-pro", "MiMo V2.6 Pro", 1_048_576, &[]),
    ("xiaomi/mimo-v2.6-flash", "MiMo V2.6 Flash", 1_048_576, &[]), ("xiaomi/mimo-v2.5-pro", "MiMo V2.5 Pro", 1_000_000, &[]),
    ("xiaomi/mimo-v2.5", "MiMo V2.5", 1_000_000, &[]), ("Qwen/Qwen3.8-Omni-Flash", "Qwen 3.8 Omni Flash", 1_000_000, EFF_LMX),
    ("Qwen/Qwen3.8-Max-0902", "Qwen 3.8 Max 0902", 1_000_000, EFF_LMX), ("Qwen/Qwen3.8-Max", "Qwen 3.8 Max", 1_000_000, EFF_LMX),
    ("Qwen/Qwen3.8-27B", "Qwen 3.8 27B", 262_144, EFF_LMX), ("Qwen/Qwen3.8-Flash", "Qwen 3.8 Flash", 1_000_000, EFF_LMX),
    ("Qwen/Qwen3.7-Max", "Qwen 3.7 Max", 1_000_000, &[]), ("Qwen/Qwen3.7-Plus", "Qwen 3.7 Plus", 1_000_000, &[]),
    ("Qwen/Qwen3.7-Flash", "Qwen 3.7 Flash", 1_000_000, &[]), ("Qwen/Qwen3.6-Max-Preview", "Qwen 3.6 Max Preview", 200_000, &[]),
    ("Qwen/Qwen3.6-Plus", "Qwen 3.6 Plus", 200_000, &[]), ("meituan/LongCat-2.0", "LongCat 2.0", 1_048_576, &[]),
    ("stepfun/Step-5-Preview", "Step 5 Preview", 1_000_000, EFF_LMH), ("stepfun/Step-3.7-Flash", "Step 3.7 Flash", 256_000, &[]),
    ("stepfun/Step-3.5-Flash", "Step 3.5 Flash", 262_144, &[]), ("tencent/hy3-paid", "Tencent Hy3", 262_144, &[]),
    ("tencent/hy4-preview", "Tencent Hy4 Preview", 1_048_576, EFF_LMH), ("google/gemini-3.6-flash", "Gemini 3.6 Flash", 1_000_000, EFF_LMH),
    ("google/gemini-3.5-flash-lite", "Gemini 3.5 Flash Lite", 1_000_000, EFF_LMH), ("nvidia/nemotron-3-ultra-550b-a55b", "Nemotron 3 Ultra", 1_000_000, &[]),
    ("thinkingmachines/inkling", "Inkling", 256_000, &[]), ("thinkingmachines/inkling-small", "Inkling Small", 1_000_000, &[]),
    ("stealth/space-bunny-alpha", "Space Bunny Alpha", 1_000_000, EFF_LMH), ("stealth/pixel-canary", "Pixel Canary", 262_144, EFF_LMX),
    ("poolside/laguna-s-2.1-free", "Laguna S 2.1", 256_000, &[]), ("inclusionai/ling-3.0-flash-free", "Ling 3.0 Flash", 256_000, &[]),
    ("inclusionai/ling-3.0-flash-sante:free", "Ling 3.0 Flash Sante", 262_144, &[]), ("inclusionai/ling-3.1-flash:free", "Ling 3.1 Flash", 262_144, EFF_LMH),
    ("meta/muse-spark-1.2-contributor", "Muse Spark 1.2 Contributor", 1_048_576, &["low", "medium", "high", "xhigh"]),
    ("meta/muse-spark-1.3-contributor", "Muse Spark 1.3 Contributor", 1_048_576, &["low", "medium", "high", "xhigh"]),
    ("xai/grok-4.5", "Grok 4.5", 500_000, EFF_LMH),
];

/// Models a plan that has the open models alone (Go, GOAT) may not use.
const PREMIUM: &[&str] = &["claude-sonnet-5", "claude-sonnet-4-6", "claude-fable-5-1", "claude-fable-5", "claude-opus-5-5", "claude-opus-5",
    "claude-opus-4-8", "claude-opus-4-7", "claude-haiku-4-5-20251001", "gpt-5.6-terra", "gpt-6-astra", "gpt-6.1-sol", "gpt-6-sol", "gpt-5.5",
    "gpt-5.4", "gpt-5.3-codex", "gpt-5.4-mini", "google/gemini-3.5-flash", "google/gemini-3.1-flash-lite", "sakana/fugu-ultra", "meta/muse-spark-1.1"];
const GO_BLOCKED: &[&str] = &["meta/muse-spark-1.2", "meta/muse-spark-1.3", "xai/grok-4.6", "xai/grok-4.7", "xiaomi/mimo-v2.6-pro-ultraspeed",
    "google/gemini-3.7-flash", "google/gemini-3.8-flash", "gpt-5.6-sol", "claude-sonnet-5-5", "claude-haiku-5-5"];
const PRO_BLOCKED: &[&str] = &["claude-fable-5-1", "claude-fable-5", "claude-opus-5-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-4-7",
    "claude-opus-4-6", "claude-opus-4-5-20251101", "gpt-6-astra", "gpt-6.1-sol", "sakana/fugu-ultra"];

/// The plan refuses the model, as the CLI's table has it; a plan it doesn't
/// know refuses nothing.
fn refuses(plan: &str, id: &str) -> bool {
    match plan {
        "Go" => PREMIUM.contains(&id) || GO_BLOCKED.contains(&id),
        "GOAT" => PREMIUM.contains(&id),
        "Pro" => PRO_BLOCKED.contains(&id),
        "Provider" => id == "gpt-6.1-sol",
        _ => false,
    }
}

fn is_claude(id: &str) -> bool {
    id.to_ascii_lowercase().starts_with("claude-") || id.starts_with("anthropic/")
}

/// Not a chat model: one that draws, embeds or speaks.
fn not_chat(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    ["image", "imagen", "imagine", "dall-e", "flux", "seedream", "cogview", "stable-diffusion", "sdxl", "wanx", "kolors", "hidream"].iter().any(|word| lower.contains(word))
        || ["embed", "-tts", "audio", "-live", "robotics", "computer-use", "deep-research", "transcribe", "realtime", "moderation", "whisper", "sora"].iter().any(|word| id.contains(word))
}

/// The account has credits beyond its plan's, which let it every model.
async fn has_extra(client: &Client, key: &str) -> bool {
    let Ok(credits) = account_json(client, "/alpha/billing/credits", key, Duration::from_secs(8)).await else { return false };
    let number = |value: &Value| value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok())).unwrap_or(0.0);
    number(&credits["credits"]["purchasedCredits"]) > 0.0 || number(&credits["credits"]["freeCredits"]) > 0.0
}

/// The plan's models: the Provider API's list with the API each model is
/// served on, less what the plan refuses; Go's list is the same one asked
/// without the key, every model on chat completions (sent on to
/// /alpha/generate), with the CLI's reasoning levels.
pub async fn fetch_models(client: &Client, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let key = creds.live_key();
    let extra = has_extra(client, &key).await;
    let keep = |id: &str| extra || !refuses(&creds.plan, id);
    let mut request = client.get(format!("{}/models", provider_base())).header("accept", "application/json").timeout(Duration::from_secs(10));
    if !creds.is_go() {
        request = request.header("authorization", format!("Bearer {key}")).header("x-api-key", &key);
    }
    let listed = match request.send().await {
        Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
        _ => None,
    };
    let rows: Vec<Value> = listed.as_ref().and_then(|body| body["data"].as_array().or(body["models"].as_array())).cloned().unwrap_or_default();
    let table = |id: &str| GO_MODELS.iter().find(|(known, ..)| *known == id || id.rsplit('/').next() == Some(known));
    let mut out = Vec::new();
    for row in &rows {
        let Some(id) = row["id"].as_str().or(row["name"].as_str()).filter(|id| !id.is_empty()) else { continue };
        if not_chat(id) || !keep(id) {
            continue;
        }
        let known = table(id);
        let context = row["context_length"].as_u64().filter(|tokens| *tokens > 0).or(known.map(|(_, _, context, _)| *context));
        let named = row["display_name"].as_str().or(row["name"].as_str().filter(|name| *name != id));
        let mut model = SourceModel {
            id: id.to_owned(),
            name: named.or(known.map(|(_, name, ..)| *name)).unwrap_or(id).to_owned(),
            context_window: context,
            ..Default::default()
        };
        if creds.is_go() {
            model.reasoning = known.map(|(.., efforts)| efforts.iter().map(|effort| (*effort).to_owned()).collect()).unwrap_or_default();
        } else {
            let endpoints: Vec<String> = row["supported_endpoints"].as_array().into_iter().flatten().filter_map(Value::as_str)
                .map(|endpoint| endpoint.trim_end_matches('/').trim_start_matches("/v1").to_owned()).collect();
            model.protocol = if endpoints.is_empty() {
                is_claude(id).then_some(ModelProtocol::AnthropicMessages)
            } else if endpoints.iter().any(|endpoint| endpoint == "/chat/completions") {
                None
            } else if endpoints.iter().any(|endpoint| endpoint == "/responses") {
                Some(ModelProtocol::Responses)
            } else if endpoints.iter().any(|endpoint| endpoint == "/messages") {
                Some(ModelProtocol::AnthropicMessages)
            } else {
                None
            };
        }
        out.push(model);
    }
    if !out.is_empty() {
        return Ok(out);
    }
    Ok(if creds.is_go() {
        GO_MODELS.iter().filter(|(id, ..)| keep(id)).map(|(id, name, context, efforts)| SourceModel {
            id: (*id).into(), name: (*name).into(), context_window: Some(*context),
            reasoning: efforts.iter().map(|effort| (*effort).to_owned()).collect(), ..Default::default()
        }).collect()
    } else {
        default_models().into_iter().filter(|model| keep(&model.id)).collect()
    })
}

// ---- the allowance ------------------------------------------------------------

/// The plan and how much of it is used: the tightest of the 5-hour and
/// weekly windows, else the month's credits used of the plan's pool, else
/// a balance. Also gives the plan read, to keep with the key.
pub async fn quota(client: &Client, creds: &Creds) -> (Quota, Option<(String, String)>) {
    let key = creds.live_key();
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), unit: "usd".into(), ..Default::default() };
    let (read, credits) = tokio::join!(subscription(client, &key), account_json(client, "/alpha/billing/credits", &key, Duration::from_secs(12)));
    let plan_id = read.as_ref().map_or(creds.plan_id.as_str(), |(id, _)| id.as_str()).to_owned();
    match credits {
        Err(status) => quota.error = Some(if status == 0 { "cannot reach Command Code".into() } else { format!("Command Code's credits: HTTP {status}") }),
        Ok(reply) => {
            let number = |value: &Value| value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()));
            let mut tightest: Option<f64> = None;
            for window in [&reply["windowLimits"]["fiveHour"], &reply["windowLimits"]["weekly"]] {
                if let (Some(used), Some(cap)) = (number(&window["used"]), number(&window["cap"]).filter(|cap| *cap > 0.0)) {
                    let share = (100.0 * used.max(0.0) / cap).min(100.0);
                    tightest = Some(tightest.map_or(share, |was: f64| was.max(share)));
                }
            }
            let parts = [&reply["credits"]["monthlyCredits"], &reply["credits"]["purchasedCredits"], &reply["credits"]["freeCredits"]];
            let amounts: Vec<Option<f64>> = parts.iter().map(|value| number(value).map(|amount| amount.max(0.0))).collect();
            let left: f64 = amounts.iter().flatten().sum();
            let known = amounts.iter().any(Option::is_some);
            let monthly_plan = plan_of(reply["credits"]["planId"].as_str().filter(|id| !id.is_empty()).unwrap_or(&plan_id)).map_or(0.0, |(_, monthly)| monthly);
            if known {
                quota.remaining = Some(left);
            }
            if let Some(share) = tightest {
                quota.unit = "percent".into();
                quota.used = share;
                quota.limit = 100.0;
            } else if known && monthly_plan > 0.0 {
                let monthly = amounts[0].unwrap_or(0.0);
                let pool = monthly_plan.max(monthly) + left - monthly;
                quota.used = pool - left;
                quota.limit = pool;
            }
        }
    }
    quota.paid = read.as_ref().map(|(_, plan)| plan != NO_PLAN);
    (quota, read)
}

// ---- the Go plan: /alpha/generate ----------------------------------------------

/// One `x-session-id` for as long as this runs, as one CLI session has.
fn session() -> &'static str {
    static SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// The CLI's own headers (buildCommandAuthHeaders).
pub fn generate_headers(key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("content-type", "application/json".to_owned()),
        ("user-agent", "cli".to_owned()),
        ("x-command-code-version", CLI_VERSION.to_owned()),
        ("x-cli-environment", "production".to_owned()),
        ("x-project-slug", "agentharbor".to_owned()),
        ("x-taste-learning", "false".to_owned()),
        ("x-session-id", session().to_owned()),
        ("authorization", format!("Bearer {key}")),
    ]
}

const EFFORT_RANK: [&str; 8] = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// The level the model has nearest the one asked, the higher on a tie.
fn fit_effort(want: &str, levels: &[&str]) -> String {
    let want = if want == "ultra" && !levels.contains(&"ultra") { "max" } else { want };
    if levels.is_empty() || levels.contains(&want) {
        return want.to_owned();
    }
    let Some(at) = EFFORT_RANK.iter().position(|level| *level == want) else { return want.to_owned() };
    let mut best = (want, usize::MAX);
    for level in levels.iter().filter(|level| **level != "none") {
        let Some(index) = EFFORT_RANK.iter().position(|known| known == level) else { continue };
        let distance = index.abs_diff(at);
        if distance < best.1 || distance == best.1 && index > at {
            best = (level, distance);
        }
    }
    best.0.to_owned()
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect(),
        _ => String::new(),
    }
}

/// A chat completion request as the CLI asks /alpha/generate.
pub fn generate_body(chat: &Value) -> Value {
    let mut system = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut names = std::collections::HashMap::<String, String>::new();
    let mut results: Vec<Value> = Vec::new();
    let flush = |results: &mut Vec<Value>, messages: &mut Vec<Value>| {
        if !results.is_empty() {
            messages.push(json!({"role": "tool", "content": std::mem::take(results)}));
        }
    };
    for message in chat["messages"].as_array().into_iter().flatten() {
        let role = message["role"].as_str().unwrap_or_default();
        if role == "system" || role == "developer" {
            system.push(text_of(&message["content"]));
            continue;
        }
        if role == "tool" {
            let id = message["tool_call_id"].as_str().unwrap_or_default();
            results.push(json!({"type": "tool-result", "toolCallId": id, "toolName": names.get(id).map_or("unknown", String::as_str),
                "output": {"type": "text", "value": text_of(&message["content"])}}));
            continue;
        }
        flush(&mut results, &mut messages);
        let mut parts = Vec::new();
        if role == "assistant" {
            if let Some(thought) = message["reasoning_content"].as_str().or(message["reasoning"].as_str()).filter(|text| !text.is_empty()) {
                parts.push(json!({"type": "reasoning", "text": thought}));
            }
            let text = text_of(&message["content"]);
            if !text.is_empty() {
                parts.push(json!({"type": "text", "text": text}));
            }
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                let id = call["id"].as_str().unwrap_or_default();
                let name = call["function"]["name"].as_str().unwrap_or_default();
                names.insert(id.to_owned(), name.to_owned());
                let input = match &call["function"]["arguments"] {
                    Value::String(text) => serde_json::from_str::<Value>(if text.is_empty() { "{}" } else { text }).ok().filter(Value::is_object).unwrap_or(json!({})),
                    Value::Object(object) => Value::Object(object.clone()),
                    _ => json!({}),
                };
                parts.push(json!({"type": "tool-call", "toolCallId": id, "toolName": name, "input": input}));
            }
            if !parts.is_empty() {
                messages.push(json!({"role": "assistant", "content": parts}));
            }
            continue;
        }
        match &message["content"] {
            Value::String(text) if !text.is_empty() => parts.push(json!({"type": "text", "text": text})),
            Value::Array(items) => for part in items {
                match part["type"].as_str() {
                    Some("text") if part["text"].as_str().is_some_and(|text| !text.is_empty()) => parts.push(json!({"type": "text", "text": part["text"]})),
                    Some("image_url") => {
                        let url = part["image_url"].as_str().or(part["image_url"]["url"].as_str()).unwrap_or_default();
                        if url.is_empty() {
                            continue;
                        }
                        let mime = url.strip_prefix("data:").and_then(|rest| rest.split([';', ',']).next());
                        parts.push(match mime {
                            Some(mime) => json!({"type": "image", "image": url, "mimeType": mime}),
                            None => json!({"type": "image", "image": url}),
                        });
                    }
                    _ => {}
                }
            },
            _ => {}
        }
        if !parts.is_empty() {
            messages.push(json!({"role": "user", "content": parts}));
        }
    }
    flush(&mut results, &mut messages);
    let tools: Vec<Value> = chat["tools"].as_array().into_iter().flatten()
        .filter(|tool| tool["type"] == "function" && tool["function"]["name"].as_str().is_some())
        .map(|tool| json!({"name": tool["function"]["name"], "description": tool["function"]["description"].as_str().unwrap_or_default(),
            "input_schema": if tool["function"]["parameters"].is_object() { tool["function"]["parameters"].clone() } else { json!({"type": "object", "properties": {}}) }}))
        .collect();
    let model = chat["model"].as_str().unwrap_or_default();
    let max = chat["max_completion_tokens"].as_u64().or(chat["max_tokens"].as_u64()).unwrap_or(MAX_TOKENS);
    let mut params = json!({"model": model, "messages": messages, "tools": tools, "system": system.join("\n\n"), "max_tokens": max, "stream": true});
    if let Some(temperature) = chat["temperature"].as_f64() {
        params["temperature"] = json!(temperature);
    }
    if let Some(effort) = chat["reasoning_effort"].as_str().filter(|effort| *effort != "none") {
        let levels = GO_MODELS.iter().find(|(id, ..)| *id == model).map(|(.., efforts)| *efforts).unwrap_or(&[]);
        params["reasoning_effort"] = json!(fit_effort(effort, levels));
    }
    let home = dirs::home_dir().map(|home| home.display().to_string()).unwrap_or_default();
    json!({
        "config": {"workingDir": home, "date": chrono::Local::now().format("%Y-%m-%d").to_string(), "environment": super::oauth::node_platform(),
            "structure": [], "isGitRepo": false, "currentBranch": "", "mainBranch": "", "gitStatus": "", "recentCommits": []},
        "memory": null, "taste": null, "skills": null, "permissionMode": "standard", "params": params,
    })
}

/// What ends a streamed reply Command Code stopped sending before its finish.
const CUT_SHORT_NOTE: &str = "(The reply was cut short: Command Code closed the stream before it finished. Ask again to go on.)";

/// Reads /alpha/generate's reply, a JSON object a line, into chat events.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    tools: usize,
    said: bool,
    wrote: bool,
    cache_write: u64,
    done: bool,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            self.line(&line, out);
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Event>) {
        let rest = std::mem::take(&mut self.buffer);
        self.line(&rest, out);
        if self.done {
            return;
        }
        self.done = true;
        if self.said {
            // the client has its start: a shorter turn that says so, not a lost one
            out.push(Event::Text(format!("{}{CUT_SHORT_NOTE}", if self.wrote { "\n\n" } else { "" })));
            out.push(Event::Finish(if self.tools > 0 { "tool_calls" } else { "stop" }));
        } else {
            out.push(Event::Error("Command Code's reply ended before it was complete".into()));
        }
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<Event>) {
        if self.done {
            return;
        }
        let Ok(event) = serde_json::from_slice::<Value>(line.trim_ascii()) else { return };
        match event["type"].as_str().unwrap_or_default() {
            "text-delta" => if let Some(text) = event["text"].as_str().filter(|text| !text.is_empty()) {
                self.said = true;
                self.wrote = true;
                out.push(Event::Text(text.to_owned()));
            },
            "reasoning-delta" => if let Some(text) = event["text"].as_str().filter(|text| !text.is_empty()) {
                self.said = true;
                out.push(Event::Reasoning(text.to_owned()));
            },
            "tool-call" if event["providerExecuted"] != true => {
                let input = match event.get("input").or(event.get("args")) {
                    Some(Value::String(text)) => serde_json::from_str::<Value>(text).unwrap_or(json!({})),
                    Some(value) if !value.is_null() => value.clone(),
                    _ => json!({}),
                };
                let id = event["toolCallId"].as_str().filter(|id| !id.is_empty()).map_or_else(|| format!("call_{}", super::oauth::random_hex(12)), str::to_owned);
                self.said = true;
                self.tools += 1;
                out.push(Event::ToolStart { id: id.clone(), name: event["toolName"].as_str().unwrap_or_default().to_owned() });
                out.push(Event::ToolArgs { id, text: input.to_string() });
            }
            "cache-write-tokens" => self.cache_write = self.cache_write.max(event["cacheWriteTokens"].as_u64().unwrap_or(0)),
            "finish" => {
                self.done = true;
                let usage = &event["totalUsage"];
                let reason = match event["finishReason"].as_str() {
                    Some("tool-calls") => "tool_calls",
                    Some("length") => "length",
                    Some("content-filter") => "content_filter",
                    _ if self.tools > 0 => "tool_calls",
                    _ => "stop",
                };
                out.push(Event::Finish(reason));
                out.push(Event::Usage {
                    input: usage["inputTokens"].as_u64().unwrap_or(0),
                    output: usage["outputTokens"].as_u64().unwrap_or(0),
                    cached: usage["inputTokenDetails"]["cacheReadTokens"].as_u64().unwrap_or(0),
                    reasoning: usage["outputTokenDetails"]["reasoningTokens"].as_u64().unwrap_or(0),
                });
            }
            "abort" => {
                self.done = true;
                out.push(Event::Finish("stop"));
            }
            "error" => {
                self.done = true;
                let error = &event["error"];
                let message = error.as_str().or(error["message"].as_str()).unwrap_or("Stream error");
                out.push(Event::Error(failure_message(message)));
            }
            _ => {}
        }
    }
}

/// A failure's message as the plugin words it: a model the plan hasn't, out
/// of credits, a window's limit.
fn failure_message(text: &str) -> String {
    let message = serde_json::from_str::<Value>(text).ok()
        .and_then(|value| value["error"]["message"].as_str().or(value["message"].as_str()).map(str::to_owned))
        .unwrap_or_else(|| text.trim().to_owned());
    if let Some(rest) = message.split_once("MODEL_NOT_IN_PLAN:").map(|(_, rest)| rest.trim()) {
        return format!("Model not in plan: {rest}");
    }
    message.replace("PREMIUM_CREDITS_EXHAUSTED:", "out of credits:").trim().to_owned()
}

/// What the user can do about a Command Code refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status < 400 {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    let lower = text.to_lowercase();
    if text.contains("Go plan doesn") && text.contains("API access") {
        return Some("this is a Go plan key, which Command Code serves only through its CLI's endpoint; refresh the account's allowance on the Accounts page so AgentHarbor reads the plan and switches to it".into());
    }
    if lower.contains("model_not_in_plan") {
        return Some("your Command Code plan doesn't include this model; pick another or add credits".into());
    }
    if lower.contains("premium_credits_exhausted") || lower.contains("insufficient credits") {
        return Some("this Command Code account is out of credits".into());
    }
    if text.contains("RATE_LIMITED") || lower.contains("usage limit") || lower.contains("window_limit") {
        return Some("a Command Code usage window is full; it resets on its own".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_refuse_what_the_cli_s_table_says() {
        assert!(refuses("Go", "claude-sonnet-5") && refuses("Go", "gpt-5.6-sol"));
        assert!(refuses("GOAT", "claude-sonnet-5") && !refuses("GOAT", "gpt-5.6-sol"));
        assert!(refuses("Pro", "claude-opus-5-5") && !refuses("Pro", "claude-sonnet-5"));
        assert!(!refuses("Max", "claude-opus-5-5") && !refuses("", "claude-opus-5-5"), "an unread plan keeps the whole list");
        assert_eq!(plan_of("individual_pro_v1_monthly"), Some(("Pro", 80.0)));
        assert_eq!(plan_of("individual-go-monthly"), Some(("Go", 10.0)));
    }

    #[test]
    fn a_chat_becomes_the_cli_s_generate_body() {
        let chat = json!({"model": "zai-org/GLM-5.3", "max_tokens": 1000, "reasoning_effort": "medium", "messages": [
            {"role": "system", "content": "sys"},
            {"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AA"}}]},
            {"role": "assistant", "content": "ok", "tool_calls": [{"id": "c1", "function": {"name": "bash", "arguments": "{\"cmd\":\"ls\"}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "a.txt"},
        ], "tools": [{"type": "function", "function": {"name": "bash", "parameters": {"type": "object"}}}]});
        let body = generate_body(&chat);
        let params = &body["params"];
        assert_eq!(params["system"], "sys");
        assert_eq!(params["max_tokens"], 1000);
        assert_eq!(params["reasoning_effort"], "high", "medium fits GLM-5.3's nearest level, the higher on a tie");
        assert_eq!(params["messages"][0]["content"][1], json!({"type": "image", "image": "data:image/png;base64,AA", "mimeType": "image/png"}));
        assert_eq!(params["messages"][1]["content"][1]["input"], json!({"cmd": "ls"}));
        assert_eq!(params["messages"][2]["content"][0]["toolName"], "bash");
        assert_eq!(params["tools"][0]["input_schema"], json!({"type": "object"}));
        assert_eq!(body["permissionMode"], "standard");
    }

    #[test]
    fn the_line_reply_becomes_chat_events() {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        decoder.feed(b"{\"type\":\"reasoning-delta\",\"text\":\"t\"}\n{\"type\":\"text-delta\",\"text\":\"hel", &mut out);
        decoder.feed(b"lo\"}\n{\"type\":\"tool-call\",\"toolCallId\":\"c\",\"toolName\":\"bash\",\"input\":{\"a\":1}}\n", &mut out);
        decoder.feed(b"{\"type\":\"finish\",\"finishReason\":\"stop\",\"totalUsage\":{\"inputTokens\":5,\"outputTokens\":2}}\n", &mut out);
        decoder.finish(&mut out);
        assert_eq!(out[0], Event::Reasoning("t".into()));
        assert_eq!(out[1], Event::Text("hello".into()));
        assert_eq!(out[2], Event::ToolStart { id: "c".into(), name: "bash".into() });
        assert_eq!(out[3], Event::ToolArgs { id: "c".into(), text: "{\"a\":1}".into() });
        assert_eq!(out[4], Event::Finish("tool_calls"));
        assert_eq!(out.len(), 6, "nothing after the finish");

        let mut cut = Decoder::default();
        let mut out = Vec::new();
        cut.feed(b"{\"type\":\"text-delta\",\"text\":\"half\"}\n", &mut out);
        cut.finish(&mut out);
        assert!(matches!(&out[1], Event::Text(note) if note.contains("cut short")));
        let mut failed = Decoder::default();
        let mut out = Vec::new();
        failed.feed(b"{\"type\":\"error\",\"error\":{\"message\":\"MODEL_NOT_IN_PLAN: Claude Opus 5.5 available in Provider\"}}\n", &mut out);
        assert_eq!(out[0], Event::Error("Model not in plan: Claude Opus 5.5 available in Provider".into()));
    }

    #[test]
    fn efforts_fit_the_nearest_level() {
        assert_eq!(fit_effort("medium", &["low", "high", "max"]), "high");
        assert_eq!(fit_effort("ultra", &["low", "max"]), "max");
        assert_eq!(fit_effort("low", &[]), "low");
    }
}
