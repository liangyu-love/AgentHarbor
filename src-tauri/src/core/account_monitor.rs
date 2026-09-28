use super::accounts::{AccountCredential, AccountUsageUpdate, CredentialRefresh};
use super::auth::{load_auth_file, make_auth_snapshot};
use super::models::{ApiProxyConfigPayload, RateLimitWindow};
use super::storage;
use crate::platform::paths::CodexPaths;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RPC_TIMEOUT: Duration = Duration::from_secs(20);

pub struct LoginArtifact {
    pub home_path: PathBuf,
    pub auth_path: PathBuf,
}

enum FetchFailure {
    Unauthorized,
    RateLimited,
    Unavailable(String),
}

enum CodexExecutable {
    Direct(PathBuf),
    WindowsScript(PathBuf),
}

fn candidate_executables() -> Vec<CodexExecutable> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let bin = PathBuf::from(local).join("OpenAI/Codex/bin");
            candidates.push(CodexExecutable::Direct(bin.join("codex.exe")));
            if let Ok(entries) = std::fs::read_dir(&bin) {
                let mut nested = entries
                    .flatten()
                    .map(|entry| entry.path().join("codex.exe"))
                    .collect::<Vec<_>>();
                nested.sort();
                nested.reverse();
                candidates.extend(nested.into_iter().map(CodexExecutable::Direct));
            }
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            candidates.push(CodexExecutable::WindowsScript(
                PathBuf::from(app_data).join("npm/codex.cmd"),
            ));
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            candidates.push(CodexExecutable::WindowsScript(
                PathBuf::from(profile).join("scoop/apps/nodejs-lts/current/bin/codex.cmd"),
            ));
        }
        candidates.push(CodexExecutable::Direct(PathBuf::from("codex.exe")));
        candidates.push(CodexExecutable::WindowsScript(PathBuf::from("codex.cmd")));
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(CodexExecutable::Direct(PathBuf::from(
            "/Applications/Codex.app/Contents/Resources/codex",
        )));
        candidates.push(CodexExecutable::Direct(PathBuf::from(
            "/Applications/ChatGPT.app/Contents/Resources/codex",
        )));
        candidates.push(CodexExecutable::Direct(PathBuf::from("codex")));
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    candidates.push(CodexExecutable::Direct(PathBuf::from("codex")));
    candidates
}

fn find_codex_executable() -> Option<CodexExecutable> {
    candidate_executables()
        .into_iter()
        .find(|candidate| match candidate {
            CodexExecutable::Direct(path) | CodexExecutable::WindowsScript(path) => {
                !path.is_absolute() || path.is_file()
            }
        })
}

#[cfg(target_os = "windows")]
fn quote_windows_arg(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:=\\-".contains(character))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn codex_command(args: &[&str]) -> Result<Command, String> {
    let executable = find_codex_executable()
        .ok_or_else(|| "未找到 Codex CLI，请先安装 Codex 或确认 codex 命令可用。".to_string())?;
    match executable {
        CodexExecutable::Direct(path) => {
            let mut command = Command::new(path);
            command.args(args);
            Ok(command)
        }
        CodexExecutable::WindowsScript(path) => {
            #[cfg(target_os = "windows")]
            {
                let mut parts = vec![quote_windows_arg(&path.display().to_string())];
                parts.extend(args.iter().map(|arg| quote_windows_arg(arg)));
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/s", "/c", &parts.join(" ")]);
                Ok(command)
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = path;
                Err("Unsupported Codex launcher".to_string())
            }
        }
    }
}

fn configure_network(command: &mut Command, proxy: &ApiProxyConfigPayload) -> Result<(), String> {
    for key in ["OPENAI_BASE_URL", "OPENAI_API_BASE"] {
        command.env_remove(key);
    }
    super::api_client::configure_codex_cli_network(command.as_std_mut(), proxy)
        .map_err(|error| error.to_string())
}

pub async fn login(
    paths: &CodexPaths,
    proxy: &ApiProxyConfigPayload,
) -> Result<LoginArtifact, String> {
    paths
        .ensure_directories()
        .map_err(|error| error.to_string())?;
    let home_path = paths
        .accounts_dir
        .join("login")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&home_path).map_err(|error| error.to_string())?;

    let mut command = codex_command(&["login"])?;
    command
        .env("CODEX_HOME", &home_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_network(&mut command, proxy)?;

    let output = match tokio::time::timeout(LOGIN_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = std::fs::remove_dir_all(&home_path);
            return Err(format!("无法启动 Codex 官方登录：{error}"));
        }
        Err(_) => {
            let _ = std::fs::remove_dir_all(&home_path);
            return Err("Codex 登录等待超时，请重新尝试。".to_string());
        }
    };
    let auth_path = home_path.join("auth.json");
    if !output.status.success() || !auth_path.is_file() {
        let _ = std::fs::remove_dir_all(&home_path);
        return Err("Codex 官方登录未完成，请在浏览器中完成授权后重试。".to_string());
    }
    if let Err(error) = load_auth_file(&auth_path) {
        // The temporary home can hold partial credentials; never leave it behind.
        let _ = std::fs::remove_dir_all(&home_path);
        return Err(error.to_string());
    }
    Ok(LoginArtifact {
        home_path,
        auth_path,
    })
}

fn http_client(proxy: &ApiProxyConfigPayload) -> Result<reqwest::Client, FetchFailure> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());
    let builder = super::api_client::configure_async_client(builder, proxy)
        .map_err(|error| FetchFailure::Unavailable(error.to_string()))?;
    builder
        .build()
        .map_err(|error| FetchFailure::Unavailable(error.to_string()))
}

async fn fetch_usage_http(
    credential: &AccountCredential,
    proxy: &ApiProxyConfigPayload,
) -> Result<Value, FetchFailure> {
    let bytes = credential_bytes(credential).map_err(FetchFailure::Unavailable)?;
    let auth: super::auth::AuthFile = serde_json::from_slice(&bytes)
        .map_err(|error| FetchFailure::Unavailable(error.to_string()))?;
    let access_token = auth
        .tokens
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or(FetchFailure::Unauthorized)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| FetchFailure::Unauthorized)?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("AgentHarbor/1.0"));
    if let Some(account_id) = auth
        .tokens
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            "chatgpt-account-id",
            HeaderValue::from_str(account_id).map_err(|_| FetchFailure::Unauthorized)?,
        );
    }
    let response = http_client(proxy)?
        .get(USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|error| FetchFailure::Unavailable(error.to_string()))?;
    match response.status().as_u16() {
        200..=299 => response
            .json::<Value>()
            .await
            .map_err(|error| FetchFailure::Unavailable(error.to_string())),
        401 | 403 => Err(FetchFailure::Unauthorized),
        429 => Err(FetchFailure::RateLimited),
        status => Err(FetchFailure::Unavailable(format!(
            "官方用量接口返回 HTTP {status}"
        ))),
    }
}

async fn rpc_request(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut BufReader<tokio::process::ChildStdout>,
    id: i64,
    method: &str,
    params: Option<Value>,
) -> Result<Value, String> {
    let message = match params {
        Some(params) => json!({"method": method, "id": id, "params": params}),
        None => json!({"method": method, "id": id}),
    };
    stdin
        .write_all(format!("{message}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stdin.flush().await.map_err(|error| error.to_string())?;

    loop {
        let mut line = String::new();
        let read = tokio::time::timeout(RPC_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| format!("{method} timed out"))?
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("Codex app-server exited before replying".to_string());
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex app-server request failed")
                .to_string());
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

fn credential_source(credential: &AccountCredential) -> &Path {
    credential
        .active_auth_path
        .as_deref()
        .unwrap_or(&credential.snapshot_path)
}

fn credential_bytes(credential: &AccountCredential) -> Result<Vec<u8>, String> {
    let source = credential_source(credential);
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    let auth = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if make_auth_snapshot(&auth, source)
        .map_err(|error| error.to_string())?
        .account_key
        != credential.account_key
    {
        return Err("The active Codex account changed; refresh the account list".into());
    }
    Ok(bytes)
}

async fn fetch_usage_rpc(
    credential: &AccountCredential,
    proxy: &ApiProxyConfigPayload,
) -> (Result<Value, String>, Option<CredentialRefresh>) {
    let original = match credential_bytes(credential) {
        Ok(bytes) => bytes,
        Err(error) => return (Err(error), None),
    };
    let runtime_root = credential
        .snapshot_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".runtime")
        .join(uuid::Uuid::new_v4().to_string());
    let runtime_auth = runtime_root.join("auth.json");

    let result = async {
        std::fs::create_dir_all(&runtime_root).map_err(|error| error.to_string())?;
        storage::write_atomic(&runtime_auth, &original).map_err(|error| error.to_string())?;
        let mut command = codex_command(&["-s", "read-only", "-a", "never", "app-server"])?;
        command
            .env("CODEX_HOME", &runtime_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        configure_network(&mut command, proxy)?;
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or("Codex app-server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Codex app-server stdout unavailable")?;
        let mut reader = BufReader::new(stdout);

        rpc_request(
            &mut stdin,
            &mut reader,
            1,
            "initialize",
            Some(json!({
                "clientInfo": {
                    "name": "agent-harbor",
                    "title": "AgentHarbor",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        )
        .await?;
        stdin
            .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n")
            .await
            .map_err(|error| error.to_string())?;
        stdin.flush().await.map_err(|error| error.to_string())?;
        let limits = rpc_request(&mut stdin, &mut reader, 2, "account/rateLimits/read", None).await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        limits
    }
    .await;

    let credential_refresh = std::fs::read(&runtime_auth)
        .ok()
        .filter(|bytes| bytes != &original)
        .map(|refreshed| CredentialRefresh {
            source_path: credential_source(credential).to_path_buf(),
            original,
            refreshed,
        });
    let _ = std::fs::remove_dir_all(&runtime_root);
    (result, credential_refresh)
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

fn integer(value: Option<&Value>) -> Option<i64> {
    number(value).map(|value| value.round() as i64)
}

fn parse_window(value: Option<&Value>) -> Option<RateLimitWindow> {
    let value = value?.as_object()?;
    let used = number(
        value
            .get("usedPercent")
            .or_else(|| value.get("used_percent")),
    )?;
    let minutes = integer(
        value
            .get("windowDurationMins")
            .or_else(|| value.get("window_duration_mins")),
    )
    .map(|value| value as i32)
    .or_else(|| {
        integer(
            value
                .get("limitWindowSeconds")
                .or_else(|| value.get("limit_window_seconds")),
        )
        .map(|seconds| (seconds / 60) as i32)
    });
    let resets_at = integer(
        value
            .get("resetsAt")
            .or_else(|| value.get("resets_at"))
            .or_else(|| value.get("resetAt"))
            .or_else(|| value.get("reset_at")),
    );
    Some(RateLimitWindow {
        used_percent: used.clamp(0.0, 100.0),
        remaining_percent: (100.0 - used).round().clamp(0.0, 100.0) as i32,
        window_minutes: minutes,
        resets_at,
    })
}

fn rate_limit_snapshot(payload: &Value) -> Value {
    if let Some(snapshot) = payload
        .get("rateLimitsByLimitId")
        .or_else(|| payload.get("rate_limits_by_limit_id"))
        .and_then(|value| value.get("codex"))
    {
        return snapshot.clone();
    }
    payload
        .get("rateLimits")
        .or_else(|| payload.get("rate_limits"))
        .or_else(|| payload.get("rateLimit"))
        .or_else(|| payload.get("rate_limit"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn normalize_plan(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pro" => "pro20x".to_string(),
        "pro_lite" | "pro-lite" | "prolite" => "pro5x".to_string(),
        "teams" => "team".to_string(),
        value => value.to_string(),
    }
}

fn parse_usage(account_key: String, payload: &Value) -> Result<AccountUsageUpdate, String> {
    let snapshot = rate_limit_snapshot(payload);
    let primary = parse_window(
        snapshot
            .get("primary")
            .or_else(|| snapshot.get("primaryWindow"))
            .or_else(|| snapshot.get("primary_window")),
    );
    let secondary = parse_window(
        snapshot
            .get("secondary")
            .or_else(|| snapshot.get("secondaryWindow"))
            .or_else(|| snapshot.get("secondary_window")),
    );
    if primary.is_none() && secondary.is_none() {
        return Err("官方用量响应未包含额度窗口".to_string());
    }
    let plan = snapshot
        .get("planType")
        .or_else(|| snapshot.get("plan_type"))
        .or_else(|| payload.get("planType"))
        .or_else(|| payload.get("plan_type"))
        .and_then(Value::as_str)
        .map(normalize_plan);
    Ok(AccountUsageUpdate {
        account_key,
        primary_window: primary,
        secondary_window: secondary,
        plan,
        status: "healthy".to_string(),
        error: None,
        credential_refresh: None,
    })
}

pub async fn refresh_account(
    credential: AccountCredential,
    proxy: ApiProxyConfigPayload,
) -> AccountUsageUpdate {
    let mut unauthorized = false;
    let (payload, credential_refresh) = match fetch_usage_http(&credential, &proxy).await {
        Ok(payload) => (Ok(payload), None),
        Err(FetchFailure::RateLimited) => (Err("官方用量接口请求过于频繁".to_string()), None),
        Err(FetchFailure::Unauthorized) => {
            unauthorized = true;
            fetch_usage_rpc(&credential, &proxy).await
        }
        Err(FetchFailure::Unavailable(message)) => {
            let (result, refresh) = fetch_usage_rpc(&credential, &proxy).await;
            (
                result.map_err(|rpc_error| format!("{message}；Codex CLI 回退失败：{rpc_error}")),
                refresh,
            )
        }
    };
    let mut update =
        match payload.and_then(|payload| parse_usage(credential.account_key.clone(), &payload)) {
            Ok(update) => update,
            Err(error) => AccountUsageUpdate {
                account_key: credential.account_key,
                primary_window: None,
                secondary_window: None,
                plan: None,
                status: if unauthorized { "expired" } else { "error" }.to_string(),
                error: Some(error),
                credential_refresh: None,
            },
        };
    update.credential_refresh = credential_refresh;
    update
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_usage_reads_current_credentials_and_rejects_a_changed_identity() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot_path = temp.path().join("snapshot.json");
        let auth_path = temp.path().join("auth.json");
        let original = json!({"tokens": {"account_id": "account", "access_token": "old"}});
        std::fs::write(&snapshot_path, original.to_string()).unwrap();
        let key = make_auth_snapshot(&load_auth_file(&snapshot_path).unwrap(), &snapshot_path)
            .unwrap()
            .account_key;
        let credential = AccountCredential {
            account_key: key,
            snapshot_path,
            active_auth_path: Some(auth_path.clone()),
        };
        let current =
            json!({"tokens": {"account_id": "account", "access_token": "new"}}).to_string();
        std::fs::write(&auth_path, &current).unwrap();
        assert_eq!(credential_bytes(&credential).unwrap(), current.as_bytes());
        std::fs::write(
            &auth_path,
            json!({"tokens": {"account_id": "another", "access_token": "other"}}).to_string(),
        )
        .unwrap();
        assert!(credential_bytes(&credential).is_err());
        std::fs::remove_file(auth_path).unwrap();
        assert!(credential_bytes(&credential).is_err());
    }

    #[test]
    fn parses_wham_usage_windows() {
        let payload = json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {"used_percent": 4.0, "limit_window_seconds": 18000, "reset_at": 1789990000},
                "secondary_window": {"used_percent": 40.0, "limit_window_seconds": 604800, "reset_at": 1790000000}
            }
        });
        let update = parse_usage("account".into(), &payload).unwrap();
        assert_eq!(update.primary_window.unwrap().remaining_percent, 96);
        assert_eq!(update.secondary_window.unwrap().remaining_percent, 60);
        assert_eq!(update.plan.as_deref(), Some("plus"));
    }

    #[test]
    fn parses_app_server_rate_limit_response() {
        let payload = json!({
            "rateLimits": {
                "planType": "pro",
                "primary": {"usedPercent": 12.5, "windowDurationMins": 300},
                "secondary": {"usedPercent": 25, "windowDurationMins": 10080}
            }
        });
        let update = parse_usage("account".into(), &payload).unwrap();
        assert_eq!(update.primary_window.unwrap().remaining_percent, 88);
        assert_eq!(update.plan.as_deref(), Some("pro20x"));
    }
}
