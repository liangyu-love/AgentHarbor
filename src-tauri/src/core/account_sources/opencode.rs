//! OpenCode's gateways: Zen (pay-as-you-go, and its free models) and Go
//! (the subscription). An account is an OpenCode key, imported from what
//! `opencode auth login` keeps (its `auth.json`, read only), or pasted; Zen's
//! free tier takes the public key.
//!
//! Ported from magpie and its community `zen-free` plugin (MIT, see
//! LICENSES/THIRD_PARTY.md). Each model is served on one API of OpenCode's
//! gateway — chat completions, Responses or Anthropic's Messages, all under
//! `/v1` — as OpenCode's docs table names it (Gemini's own API, which the
//! router doesn't speak, is left out). Every request carries what OpenCode
//! itself sends there: its User-Agent and its session ids. Zen's free models
//! are served only to what looks like OpenCode: exactly OpenCode 1.19.0's
//! headers, a session id of OpenCode's own shape, a streamed request at the
//! inference API offering its core tools (stand-ins the agent doesn't have,
//! said not to be called); a call of a stand-in is left out of the reply.
use super::{store::{Quota, SourceModel}, wire, SourceKind};
use crate::core::router::ModelProtocol;
use futures_util::StreamExt;
use hyper::body::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

const OPENCODE_VERSION: &str = "1.19.0";
/// OpenCode's table of what each model is asked at.
const DOCS: &str = "https://raw.githubusercontent.com/anomalyco/opencode/dev/services/www/src/docs/content/console/models.mdx";
/// The free tier's public key.
pub const PUBLIC_KEY: &str = "public";
const CORE_TOOLS: [(&str, &str); 5] = [("bash", "command"), ("edit", "filePath"), ("glob", "pattern"), ("grep", "pattern"), ("read", "filePath")];
const STUB: &str = "Not available. Never call this tool.";
pub const ZEN_USAGE_URL: &str = "https://opencode.ai/workspace";

fn root() -> String {
    super::endpoint_override(SourceKind::OpencodeZen).or_else(|| super::endpoint_override(SourceKind::OpencodeGo))
        .unwrap_or_else(|| "https://opencode.ai".to_owned())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creds {
    pub key: String,
    /// Zen's free tier, asked as OpenCode asks it.
    pub free: bool,
}

/// Where a gateway's APIs are: Zen's, its free tier's (the inference API,
/// since Zen answers the free tier only there), or Go's.
pub fn base(kind: SourceKind, free: bool) -> String {
    match (kind, free) {
        (SourceKind::OpencodeGo, _) => format!("{}/zen/go/v1", root()),
        (_, true) => format!("{}/inference/v1", root()),
        _ => format!("{}/zen/v1", root()),
    }
}

/// OpenCode's own sign-ins: auth.json in its data folder,
/// `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode` (on Windows too).
fn auth_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir).join("opencode").join("auth.json"));
    }
    dirs::home_dir().map(|home| home.join(".local").join("share").join("opencode").join("auth.json"))
}

/// OpenCode's provider id for a gateway.
fn entry_id(kind: SourceKind) -> &'static str {
    if kind == SourceKind::OpencodeGo { "opencode-go" } else { "opencode" }
}

/// The key `opencode auth login` keeps for this gateway.
pub fn import(kind: SourceKind) -> Result<Creds, String> {
    let path = auth_path().ok_or("no home folder")?;
    let text = std::fs::read_to_string(&path).map_err(|_| "accountSources.errors.opencodeNotSignedIn".to_owned())?;
    let auth: Value = serde_json::from_str(&text).map_err(|_| format!("{} can't be read", path.display()))?;
    let entry = &auth[entry_id(kind)];
    match (entry["type"].as_str(), entry["key"].as_str().map(str::trim)) {
        (Some("api"), Some(key)) if !key.is_empty() => Ok(Creds { key: key.to_owned(), free: key == PUBLIC_KEY && kind == SourceKind::OpencodeZen }),
        _ => Err("accountSources.errors.opencodeNotSignedIn".into()),
    }
}

/// A key, checked by asking for the gateway's model list with it.
pub async fn check_key(client: &Client, kind: SourceKind, key: &str) -> Result<Creds, String> {
    let free = kind == SourceKind::OpencodeZen && key == PUBLIC_KEY;
    let response = client.get(format!("{}/models", base(kind, free))).header("authorization", format!("Bearer {key}"))
        .header("x-opencode-client", "cli").timeout(Duration::from_secs(15)).send().await.map_err(|_| "cannot reach OpenCode".to_owned())?;
    match response.status().as_u16() {
        401 | 403 => Err("accountSources.errors.opencodeKey".into()),
        _ => Ok(Creds { key: key.to_owned(), free }),
    }
}

/// The account's name: the key's last characters, as OpenCode shows a key.
pub fn name_of(kind: SourceKind, creds: &Creds) -> String {
    if creds.free {
        return "Zen Free".into();
    }
    let tail: String = creds.key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{} …{tail}", if kind == SourceKind::OpencodeGo { "Go" } else { "Zen" })
}

// ---- models ----------------------------------------------------------------------

/// What each free model is asked at when OpenCode's docs can't be read.
const BOOTSTRAP: [(&str, &str); 9] = [("big-pickle", "chat"), ("space-bunny-free", "chat"), ("cat-jam-free", "chat"), ("longcat-2.5-preview-free", "chat"),
    ("step-5-preview-free", "chat"), ("mimo-v2.6-flash-free", "chat"), ("nemotron-3.5-lightning-free", "chat"),
    ("muse-spark-1.3-contributor-free", "responses"), ("jev-1.13-free", "systemone")];

/// Zen's free models whose ids don't end in -free.
const FREE_IDS: [&str; 2] = ["big-pickle", "grok-code"];

fn is_free(id: &str) -> bool {
    id.ends_with("-free") || FREE_IDS.contains(&id)
}

/// OpenCode's docs table: `| Name | id | `…/v1/messages` |` rows.
fn parse_docs(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines().filter(|line| line.trim_start().starts_with('|')) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let id = cells[2].trim_matches('`').trim();
        let Some(endpoint) = cells[3].trim_matches('`').split("/v1/").nth(1) else { continue };
        let api = match endpoint.trim_end_matches('`').trim() {
            "chat/completions" => "chat",
            "responses" => "responses",
            "messages" => "anthropic",
            other if other.starts_with("models/") => "gemini",
            "systemone" => "systemone",
            _ => continue,
        };
        if !id.is_empty() && !id.contains(' ') {
            out.insert(id.to_owned(), api.to_owned());
        }
    }
    out
}

/// A model's API by its family, as OpenCode asks it: GPT, Grok and Muse
/// Spark on Responses, Claude and MiniMax on Messages, Gemini on its own
/// API, the rest on chat.
fn family_api(id: &str) -> &'static str {
    let id = id.to_ascii_lowercase();
    if id.starts_with("gpt-") || id.starts_with("grok-") && id != "grok-code" || id.starts_with("muse-spark") {
        "responses"
    } else if id.starts_with("claude-") || id.starts_with("minimax-") {
        "anthropic"
    } else if id.starts_with("gemini-") {
        "gemini"
    } else {
        "chat"
    }
}

fn protocol_of(api: &str) -> Option<Option<ModelProtocol>> {
    match api {
        "chat" => Some(None),
        "responses" => Some(Some(ModelProtocol::Responses)),
        "anthropic" => Some(Some(ModelProtocol::AnthropicMessages)),
        _ => None,
    }
}

/// The gateway's models with the API each is asked on; a free tier's only
/// its free ones.
pub async fn fetch_models(client: &Client, kind: SourceKind, creds: &Creds) -> Result<Vec<SourceModel>, String> {
    let response = client.get(format!("{}/models", base(kind, creds.free))).header("authorization", format!("Bearer {}", creds.key))
        .header("x-opencode-client", "cli").header("accept", "application/json").timeout(Duration::from_secs(15)).send().await
        .map_err(|_| "cannot reach OpenCode".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("OpenCode's models: HTTP {}", response.status().as_u16()));
    }
    let listed: Value = response.json().await.map_err(|_| "OpenCode's models: not JSON".to_owned())?;
    let docs_url = super::endpoint_override(kind).map_or_else(|| DOCS.to_owned(), |base| format!("{base}/docs/models.mdx"));
    let docs = match client.get(docs_url).timeout(Duration::from_secs(8)).send().await {
        Ok(reply) if reply.status().is_success() && kind == SourceKind::OpencodeZen => parse_docs(&reply.text().await.unwrap_or_default()),
        _ => HashMap::new(),
    };
    let mut out = Vec::new();
    for id in listed["data"].as_array().into_iter().flatten().filter_map(|model| model["id"].as_str()) {
        if creds.free && !is_free(id) {
            continue;
        }
        let api = docs.get(id).map(String::as_str)
            .or_else(|| creds.free.then(|| BOOTSTRAP.iter().find(|(known, _)| *known == id).map(|(_, api)| *api)).flatten())
            .unwrap_or_else(|| family_api(id));
        let Some(protocol) = protocol_of(api) else { continue };
        out.push(SourceModel { id: id.to_owned(), name: id.to_owned(), protocol, ..Default::default() });
    }
    if out.is_empty() {
        return Err("OpenCode listed no models it serves here".into());
    }
    Ok(out)
}

/// Go's allowance: the tightest of its rolling, weekly and monthly windows.
pub async fn quota(client: &Client, creds: &Creds) -> Quota {
    let mut quota = Quota { checked_at: chrono::Utc::now().timestamp(), unit: "percent".into(), limit: 100.0, ..Default::default() };
    let reply = client.get(format!("{}/zen/go/v1/usage", root())).header("authorization", format!("Bearer {}", creds.key))
        .timeout(Duration::from_secs(15)).send().await;
    match reply {
        Ok(response) if response.status().is_success() => {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            let number = |value: &Value| value.as_f64().or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()));
            quota.used = ["rolling", "weekly", "monthly"].iter().filter_map(|name| number(&body["usage"][name]["percent"])).fold(0.0, f64::max);
            quota.paid = Some(true);
        }
        Ok(response) => {
            quota.error = Some(format!("OpenCode Go's usage: HTTP {}", response.status().as_u16()));
            quota.limit = 0.0;
        }
        Err(_) => {
            quota.error = Some("cannot reach OpenCode".into());
            quota.limit = 0.0;
        }
    }
    quota
}

// ---- requests ------------------------------------------------------------------

/// The key, as each of the gateway's APIs takes it, and OpenCode's client.
pub fn request_headers(creds: &Creds) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", creds.key)),
        ("x-api-key", creds.key.clone()),
        ("anthropic-version", "2023-06-01".to_owned()),
        ("accept", "*/*".to_owned()),
        ("content-type", "application/json".to_owned()),
        ("x-opencode-client", "cli".to_owned()),
        ("x-opencode-project", "global".to_owned()),
    ]
}

/// What OpenCode 1.19.0 sends as its User-Agent: each API's ai-sdk package
/// bundles its own provider-utils.
fn user_agent(protocol: ModelProtocol) -> String {
    let utils = match protocol {
        ModelProtocol::ChatCompletions => "4.0.23",
        ModelProtocol::Responses => "4.0.40",
        ModelProtocol::AnthropicMessages => "4.0.46",
    };
    format!("opencode/{OPENCODE_VERSION} ai-sdk/provider-utils/{utils} runtime/bun/1.3.14")
}

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// An id as OpenCode makes one: 12 hex of the time in milliseconds times
/// 0x1000 plus a count of the ids made in that millisecond (bit-inverted
/// for a session, so newer sorts first), then 14 random base62 characters.
/// Zen's free tier checks a session id has this shape, its count small.
fn opencode_id(prefix: &str, descending: bool) -> String {
    static CLOCK: Mutex<(i64, u64)> = Mutex::new((0, 0));
    let now = chrono::Utc::now().timestamp_millis();
    let current = {
        let mut clock = CLOCK.lock().unwrap();
        if clock.0 != now {
            *clock = (now, 0);
        }
        clock.1 += 1;
        (now as u64).wrapping_mul(0x1000).wrapping_add(clock.1)
    };
    let value = if descending { !current } else { current } & 0xffff_ffff_ffff;
    let random: Vec<u8> = uuid::Uuid::new_v4().as_bytes().iter().take(14).copied().collect();
    let tail: String = random.iter().map(|byte| BASE62[*byte as usize % 62] as char).collect();
    format!("{prefix}_{value:012x}{tail}")
}

/// A session id OpenCode itself could have made.
fn real_session(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("ses_") else { return false };
    if rest.len() != 26 || !rest[..12].bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || !rest[12..].bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return false;
    }
    u16::from_str_radix(&rest[9..12], 16).is_ok_and(|low| 0xfff - low < 16)
}

/// One session per conversation, named by its first user message, so its
/// turns keep one session (the last thousand conversations' are kept).
fn session_of(body: &Value) -> String {
    static SESSIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let given = body["metadata"]["session_id"].as_str().unwrap_or_default();
    if real_session(given) {
        return given.to_owned();
    }
    let first = body["messages"].as_array().or(body["input"].as_array())
        .and_then(|messages| messages.iter().find(|message| message["role"] == "user")).map(|message| message["content"].to_string())
        .or_else(|| body["input"].as_str().map(str::to_owned));
    let Some(first) = first.filter(|_| given.is_empty()).or_else(|| (!given.is_empty()).then(|| given.to_owned())) else {
        return opencode_id("ses", true);
    };
    let seed = super::oauth::sha256_hex(&format!("ses\0{first}"));
    let mut sessions = SESSIONS.lock().unwrap();
    if let Some(index) = sessions.iter().position(|(kept, _)| *kept == seed) {
        let entry = sessions.remove(index);
        let id = entry.1.clone();
        sessions.push(entry);
        return id;
    }
    let id = opencode_id("ses", true);
    if sessions.len() >= 1000 {
        sessions.remove(0);
    }
    sessions.push((seed, id.clone()));
    id
}

/// The headers that name this request's conversation and turn, and the
/// User-Agent of the API it is asked on.
pub fn body_headers(protocol: ModelProtocol, body: &[u8]) -> Vec<(&'static str, String)> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let session = session_of(&value);
    vec![
        ("user-agent", user_agent(protocol)),
        ("x-opencode-session", session.clone()),
        ("x-opencode-session-id", session),
        ("x-opencode-request", opencode_id("msg", false)),
    ]
}

fn tool_name(protocol: ModelProtocol, tool: &Value) -> Option<String> {
    match protocol {
        ModelProtocol::ChatCompletions => tool["function"]["name"].as_str(),
        _ => tool["name"].as_str(),
    }.map(str::to_owned)
}

/// The tools the agent declared, by name.
fn declared(protocol: ModelProtocol, body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body["tools"].as_array().into_iter().flatten().filter_map(|tool| tool_name(protocol, tool)).collect();
    if protocol == ModelProtocol::ChatCompletions {
        names.extend(body["functions"].as_array().into_iter().flatten().filter_map(|function| function["name"].as_str().map(str::to_owned)));
    }
    names
}

/// A free-tier request as OpenCode sends it: streamed (a chat that didn't
/// stream is collapsed again on the way back), its usage asked for, and
/// OpenCode's core tools offered as stand-ins the agent doesn't declare.
pub fn shape_free(protocol: ModelProtocol, body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let streamed = value["stream"] == true;
    if !streamed && protocol != ModelProtocol::ChatCompletions {
        // only a chat reply can be collapsed here; others go as asked
        return None;
    }
    value["stream"] = json!(true);
    let present = declared(protocol, &value);
    if !value["tools"].is_array() {
        value["tools"] = json!([]);
    }
    let tools = value["tools"].as_array_mut()?;
    for (name, parameter) in CORE_TOOLS {
        if present.iter().any(|known| known == name) {
            continue;
        }
        let parameters = json!({"type": "object", "properties": {parameter: {"type": "string"}}, "additionalProperties": false});
        tools.push(match protocol {
            ModelProtocol::ChatCompletions => json!({"type": "function", "function": {"name": name, "description": STUB, "parameters": parameters}}),
            ModelProtocol::AnthropicMessages => json!({"name": name, "description": STUB, "input_schema": parameters}),
            ModelProtocol::Responses => json!({"type": "function", "name": name, "description": STUB, "parameters": parameters}),
        });
    }
    if protocol == ModelProtocol::ChatCompletions {
        value["stream_options"]["include_usage"] = json!(true);
    }
    serde_json::to_vec(&value).ok()
}

/// What reads a free-tier reply: the agent's tool names, and whether the
/// client wanted one JSON body rather than a stream.
pub struct FreeReply {
    protocol: ModelProtocol,
    declared: Vec<String>,
    collapse: bool,
    model: String,
}

pub fn free_reply(protocol: ModelProtocol, body: &[u8]) -> FreeReply {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    FreeReply {
        protocol,
        declared: declared(protocol, &value),
        collapse: protocol == ModelProtocol::ChatCompletions && value["stream"] != true,
        model: value["model"].as_str().unwrap_or_default().to_owned(),
    }
}

/// The agent's tool a model's call names: its own name, or the one name
/// that differs from it only in case, or in case and `_`/`-`. `None` when
/// the agent has none such (a stand-in's call).
fn agent_tool(declared: &[String], name: &str) -> Option<String> {
    if declared.iter().any(|known| known == name) {
        return Some(name.to_owned());
    }
    let unique = |found: Vec<&String>| (found.len() == 1).then(|| found[0].clone());
    let fold = |text: &str| text.to_ascii_lowercase().replace(['_', '-'], "");
    unique(declared.iter().filter(|known| known.eq_ignore_ascii_case(name)).collect())
        .or_else(|| unique(declared.iter().filter(|known| fold(known) == fold(name)).collect()))
}

/// The guard's state over one reply.
#[derive(Default)]
struct Guard {
    /// Chat and Messages: upstream index → the index sent on, `None` dropped.
    indices: HashMap<u64, Option<u64>>,
    kept: u64,
    /// Responses: dropped items by id.
    dropped_items: Vec<String>,
    left_out: Vec<String>,
}

impl FreeReply {
    /// One SSE data object with stand-ins' calls left out and the rest
    /// named as the agent named its tools; `None` drops the event.
    fn event(&self, guard: &mut Guard, mut value: Value) -> Option<Value> {
        match self.protocol {
            ModelProtocol::ChatCompletions => {
                for choice in value["choices"].as_array_mut().into_iter().flatten() {
                    if let Some(calls) = choice["delta"]["tool_calls"].as_array_mut() {
                        calls.retain_mut(|call| {
                            let index = call["index"].as_u64().unwrap_or(0);
                            if !guard.indices.contains_key(&index) {
                                let name = call["function"]["name"].as_str().unwrap_or_default().to_owned();
                                match agent_tool(&self.declared, &name) {
                                    Some(ours) => {
                                        call["function"]["name"] = json!(ours);
                                        guard.indices.insert(index, Some(guard.kept));
                                        guard.kept += 1;
                                    }
                                    None => {
                                        guard.left_out.push(name);
                                        guard.indices.insert(index, None);
                                    }
                                }
                            }
                            match guard.indices[&index] {
                                Some(sent) => {
                                    call["index"] = json!(sent);
                                    true
                                }
                                None => false,
                            }
                        });
                        if calls.is_empty() {
                            choice["delta"].as_object_mut()?.remove("tool_calls");
                        }
                    }
                    if choice["finish_reason"] == "tool_calls" && guard.kept == 0 {
                        choice["finish_reason"] = json!("stop");
                        if !guard.left_out.is_empty() {
                            choice["delta"]["content"] = json!(format!("\n\n(Left out a call of `{}`, a tool this session doesn't have.)", guard.left_out.join("`, `")));
                        }
                    }
                }
                Some(value)
            }
            ModelProtocol::AnthropicMessages => {
                let index = value["index"].as_u64();
                match value["type"].as_str().unwrap_or_default() {
                    "content_block_start" => {
                        let index = index.unwrap_or(0);
                        if value["content_block"]["type"] == "tool_use" {
                            let name = value["content_block"]["name"].as_str().unwrap_or_default().to_owned();
                            match agent_tool(&self.declared, &name) {
                                Some(ours) => value["content_block"]["name"] = json!(ours),
                                None => {
                                    guard.left_out.push(name);
                                    guard.indices.insert(index, None);
                                    return None;
                                }
                            }
                        }
                        guard.indices.insert(index, Some(guard.kept));
                        value["index"] = json!(guard.kept);
                        guard.kept += 1;
                        Some(value)
                    }
                    "content_block_delta" | "content_block_stop" => match guard.indices.get(&index.unwrap_or(0)).copied().flatten() {
                        Some(sent) => {
                            value["index"] = json!(sent);
                            Some(value)
                        }
                        None if guard.indices.contains_key(&index.unwrap_or(0)) => None,
                        None => Some(value),
                    },
                    "message_delta" => {
                        let tools_kept = guard.indices.values().flatten().count() as u64;
                        if value["delta"]["stop_reason"] == "tool_use" && !guard.left_out.is_empty() && tools_kept == 0 {
                            value["delta"]["stop_reason"] = json!("end_turn");
                        }
                        Some(value)
                    }
                    _ => Some(value),
                }
            }
            ModelProtocol::Responses => {
                let item = if value["item"].is_object() { Some(value["item"].clone()) } else { None };
                if let Some(item) = item.filter(|item| item["type"] == "function_call") {
                    let id = item["id"].as_str().unwrap_or_default().to_owned();
                    let name = item["name"].as_str().unwrap_or_default().to_owned();
                    match agent_tool(&self.declared, &name) {
                        Some(ours) => value["item"]["name"] = json!(ours),
                        None => {
                            if !guard.dropped_items.contains(&id) {
                                guard.left_out.push(name);
                                guard.dropped_items.push(id);
                            }
                            return None;
                        }
                    }
                }
                if value["item_id"].as_str().is_some_and(|id| guard.dropped_items.iter().any(|dropped| dropped == id)) {
                    return None;
                }
                if let Some(output) = value["response"]["output"].as_array_mut() {
                    output.retain_mut(|item| {
                        if item["type"] != "function_call" {
                            return true;
                        }
                        match agent_tool(&self.declared, item["name"].as_str().unwrap_or_default()) {
                            Some(ours) => {
                                item["name"] = json!(ours);
                                true
                            }
                            None => false,
                        }
                    });
                }
                Some(value)
            }
        }
    }

    /// The reply with the guard applied, line by line; a chat the client
    /// didn't stream collapsed into one completion.
    pub async fn apply(self, response: reqwest::Response) -> reqwest::Response {
        if !response.status().is_success() {
            return response;
        }
        let status = response.status();
        let streamed = response.headers().get(hyper::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("event-stream"));
        if !streamed {
            return response;
        }
        if self.collapse {
            let bytes = response.bytes().await.unwrap_or_default();
            let mut guard = Guard::default();
            let mut events = Vec::new();
            let mut ids: HashMap<u64, String> = HashMap::new();
            for line in String::from_utf8_lossy(&bytes).lines() {
                let Some(data) = line.strip_prefix("data:").map(str::trim).filter(|data| *data != "[DONE]") else { continue };
                let Some(value) = serde_json::from_str::<Value>(data).ok().and_then(|value| self.event(&mut guard, value)) else { continue };
                chat_events(&value, &mut ids, &mut events);
            }
            let body = wire::Writer::new(&self.model, false).collect(events);
            return reqwest::Response::from(hyper::Response::builder().status(status)
                .header(hyper::header::CONTENT_TYPE, "application/json").body(body.to_string()).expect("static response parts"));
        }
        let state = (response.bytes_stream(), self, Guard::default(), String::new());
        let stream = futures_util::stream::unfold(Some(state), |state| async move {
            let (mut upstream, reply, mut guard, mut pending) = state?;
            loop {
                match upstream.next().await {
                    Some(Ok(chunk)) => {
                        pending.push_str(&String::from_utf8_lossy(&chunk));
                        let mut out = String::new();
                        while let Some(end) = pending.find("\n\n") {
                            let event: String = pending.drain(..end + 2).collect();
                            out.push_str(&reply.frame(&mut guard, &event));
                        }
                        if !out.is_empty() {
                            return Some((Ok::<_, std::io::Error>(Bytes::from(out)), Some((upstream, reply, guard, pending))));
                        }
                    }
                    Some(Err(error)) => return Some((Err(std::io::Error::other(error.to_string())), None)),
                    None => {
                        let rest = if pending.trim().is_empty() { String::new() } else { reply.frame(&mut guard, &pending) };
                        return (!rest.is_empty()).then(|| (Ok(Bytes::from(rest)), None));
                    }
                }
            }
        });
        reqwest::Response::from(hyper::Response::builder().status(status)
            .header(hyper::header::CONTENT_TYPE, "text/event-stream")
            .body(reqwest::Body::wrap_stream(stream)).expect("static response parts"))
    }

    /// One SSE frame (its `event:` line kept) with the guard applied.
    fn frame(&self, guard: &mut Guard, frame: &str) -> String {
        let mut out = String::new();
        let mut dropped = false;
        for line in frame.split_inclusive('\n') {
            let bare = line.trim_end_matches(['\r', '\n']);
            match bare.strip_prefix("data:").map(str::trim) {
                Some(data) if data != "[DONE]" => match serde_json::from_str::<Value>(data) {
                    Ok(value) => match self.event(guard, value) {
                        Some(value) => out.push_str(&format!("data: {value}\n")),
                        None => dropped = true,
                    },
                    Err(_) => out.push_str(line),
                },
                _ => out.push_str(line),
            }
        }
        if dropped { String::new() } else { out }
    }
}

/// A chat chunk's events, for collapsing a streamed chat.
fn chat_events(value: &Value, ids: &mut HashMap<u64, String>, events: &mut Vec<wire::Event>) {
    for choice in value["choices"].as_array().into_iter().flatten() {
        let delta = &choice["delta"];
        if let Some(text) = delta["reasoning_content"].as_str().or(delta["reasoning"].as_str()) {
            events.push(wire::Event::Reasoning(text.to_owned()));
        }
        if let Some(text) = delta["content"].as_str() {
            events.push(wire::Event::Text(text.to_owned()));
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0);
            if let Some(name) = call["function"]["name"].as_str() {
                let id = call["id"].as_str().map_or_else(|| format!("call_{index}"), str::to_owned);
                ids.insert(index, id.clone());
                events.push(wire::Event::ToolStart { id, name: name.to_owned() });
            }
            if let (Some(text), Some(id)) = (call["function"]["arguments"].as_str(), ids.get(&index)) {
                events.push(wire::Event::ToolArgs { id: id.clone(), text: text.to_owned() });
            }
        }
        match choice["finish_reason"].as_str() {
            Some("length") => events.push(wire::Event::Finish("length")),
            Some("tool_calls") => events.push(wire::Event::Finish("tool_calls")),
            Some("content_filter") => events.push(wire::Event::Finish("content_filter")),
            Some(_) => events.push(wire::Event::Finish("stop")),
            None => {}
        }
    }
    let usage = &value["usage"];
    if usage.is_object() {
        events.push(wire::Event::Usage {
            input: usage["prompt_tokens"].as_u64().unwrap_or(0),
            output: usage["completion_tokens"].as_u64().unwrap_or(0),
            cached: usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0),
            reasoning: usage["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0),
        });
    }
}

/// What the user can do about an OpenCode refusal.
pub fn explain(status: u16, body: &[u8]) -> Option<String> {
    if status < 400 {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    if text.contains("FreeTierError") || text.contains("free tier can only be used from within OpenCode") {
        return Some("OpenCode Zen's free tier turned the request away as not OpenCode's own; refresh the model list, or use a paid key".into());
    }
    if text.contains("does not support this protocol") {
        return Some("OpenCode serves this model on another API; refresh the account source's model list".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_have_opencode_s_shape_and_follow_the_conversation() {
        let id = opencode_id("ses", true);
        assert!(real_session(&id), "{id}");
        assert!(!real_session(&opencode_id("ses", false)), "an ascending id's count isn't small once inverted");
        let first = json!({"messages": [{"role": "user", "content": "hi"}]});
        let later = json!({"messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "x"}, {"role": "user", "content": "go"}]});
        assert_eq!(session_of(&first), session_of(&later));
        assert_ne!(session_of(&first), session_of(&json!({"messages": [{"role": "user", "content": "other"}]})));
    }

    #[test]
    fn the_free_tier_offers_opencode_s_core_tools_streamed() {
        let body = json!({"model": "big-pickle", "messages": [], "tools": [{"type": "function", "function": {"name": "Bash"}}]});
        let shaped: Value = serde_json::from_slice(&shape_free(ModelProtocol::ChatCompletions, body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(shaped["stream"], true);
        assert_eq!(shaped["stream_options"]["include_usage"], true);
        let names: Vec<&str> = shaped["tools"].as_array().unwrap().iter().map(|tool| tool["function"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Bash", "bash", "edit", "glob", "grep", "read"]);
        assert_eq!(shaped["tools"][1]["function"]["parameters"]["properties"]["command"]["type"], "string");
        let responses = json!({"model": "m", "input": "x", "stream": false});
        assert!(shape_free(ModelProtocol::Responses, responses.to_string().as_bytes()).is_none(), "only a chat can be collapsed again");
    }

    #[test]
    fn calls_of_stand_ins_are_left_out_and_the_rest_named_as_the_agent_named_them() {
        let reply = FreeReply { protocol: ModelProtocol::ChatCompletions, declared: vec!["Bash".into(), "todo_write".into()], collapse: false, model: "m".into() };
        let mut guard = Guard::default();
        let call = |index: u64, name: Option<&str>, args: &str| json!({"choices": [{"delta": {"tool_calls": [match name {
            Some(name) => json!({"index": index, "id": format!("c{index}"), "function": {"name": name, "arguments": args}}),
            None => json!({"index": index, "function": {"arguments": args}}),
        }]}}]});
        let renamed = reply.event(&mut guard, call(0, Some("bash"), "{")).unwrap();
        assert_eq!(renamed["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "Bash");
        let dropped = reply.event(&mut guard, call(1, Some("read"), "{")).unwrap();
        assert!(dropped["choices"][0]["delta"].get("tool_calls").is_none());
        assert!(reply.event(&mut guard, call(1, None, "}")).unwrap()["choices"][0]["delta"].get("tool_calls").is_none());
        let next = reply.event(&mut guard, call(2, Some("todowrite"), "{")).unwrap();
        assert_eq!(next["choices"][0]["delta"]["tool_calls"][0]["index"], 1, "renumbered past the one left out");
        assert_eq!(next["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "todo_write");

        let only = FreeReply { protocol: ModelProtocol::ChatCompletions, declared: vec![], collapse: false, model: "m".into() };
        let mut guard = Guard::default();
        only.event(&mut guard, call(0, Some("bash"), "{}"));
        let finish = only.event(&mut guard, json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]})).unwrap();
        assert_eq!(finish["choices"][0]["finish_reason"], "stop", "a turn whose every call was left out is an answer");
        assert!(finish["choices"][0]["delta"]["content"].as_str().unwrap().contains("bash"));

        let messages = FreeReply { protocol: ModelProtocol::AnthropicMessages, declared: vec!["Read".into()], collapse: false, model: "m".into() };
        let mut guard = Guard::default();
        assert!(messages.event(&mut guard, json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "name": "bash"}})).is_none());
        assert!(messages.event(&mut guard, json!({"type": "content_block_delta", "index": 0})).is_none());
        let read = messages.event(&mut guard, json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "name": "read"}})).unwrap();
        assert_eq!((read["index"].as_u64(), read["content_block"]["name"].as_str()), (Some(0), Some("Read")));
    }

    #[test]
    fn docs_rows_and_families_name_each_model_s_api() {
        let docs = "| Model | Model ID | Endpoint |\n|---|---|---|\n| GPT 5.5 | gpt-5.5 | `https://opencode.ai/zen/v1/responses` |\n| Qwen | qwen3.7-max | `https://opencode.ai/zen/v1/messages` |\n| Gemini | gemini-3-pro | `https://opencode.ai/zen/v1/models/gemini-3-pro` |";
        let parsed = parse_docs(docs);
        assert_eq!(parsed["gpt-5.5"], "responses");
        assert_eq!(parsed["qwen3.7-max"], "anthropic");
        assert_eq!(parsed["gemini-3-pro"], "gemini");
        assert_eq!(family_api("claude-sonnet-5"), "anthropic");
        assert_eq!(family_api("grok-code"), "chat");
        assert_eq!(family_api("glm-5.3"), "chat");
        assert!(is_free("big-pickle") && is_free("glm-5-free") && !is_free("glm-5"));
    }
}
