use super::{models::ApiProxyConfigPayload, secret_file, storage};
use crate::platform::paths::CodexPaths;
use reqwest::{
    blocking::{Client, Response},
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, path::Path, time::Duration};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub base_url: String,
    pub user_id: Option<u64>,
    pub access_token: String,
    pub api_key: String,
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub base_url: String,
    pub user_id: Option<u64>,
    pub has_access_token: bool,
    pub has_api_key: bool,
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
}

impl Connection {
    pub fn view(&self) -> ConnectionView {
        ConnectionView { base_url: self.base_url.clone(), user_id: self.user_id,
            has_access_token: !self.access_token.is_empty(), has_api_key: !self.api_key.is_empty(),
            model: self.model.clone(), context_window: self.context_window, auto_compact_limit: self.auto_compact_limit }
    }
}

pub fn load(paths: &CodexPaths) -> Result<Connection, String> {
    match secret_file::read(&paths.app_data_dir.join("newapi-connection.dat"))? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage".into()),
        None => Ok(Connection::default()),
    }
}

pub(crate) fn base_url(text: &str) -> Result<Url, String> {
    let mut url = Url::parse(text.trim()).map_err(|_| "newapi.errors.url")?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("newapi.errors.url".into());
    }
    // Accept the commonly pasted OpenAI base URL as well as the site root.
    let path = url.path().trim_end_matches('/').trim_end_matches("/v1").to_owned();
    url.set_path(&format!("{path}/"));
    Ok(url)
}

pub(crate) fn validate_limits(config: &Connection) -> Result<(), String> {
    if config.context_window.is_some_and(|v| v == 0 || v > 100_000_000)
        || config.auto_compact_limit.is_some_and(|v| v == 0 || v > 100_000_000)
        || matches!((config.context_window, config.auto_compact_limit), (Some(c), Some(a)) if a >= c) {
        return Err("newapi.errors.context".into());
    }
    if config.model.len() > 200 || config.model.chars().any(char::is_control) {
        return Err("newapi.errors.model".into());
    }
    Ok(())
}

pub fn save(paths: &CodexPaths, mut next: Connection) -> Result<ConnectionView, String> {
    next.base_url = base_url(&next.base_url)?.to_string();
    if next.user_id == Some(0) || next.user_id.is_some_and(|v| v > 9_007_199_254_740_991) {
        return Err("newapi.errors.userId".into());
    }
    next.access_token = next.access_token.trim().to_owned();
    next.api_key = next.api_key.trim().to_owned();
    next.model = next.model.trim().to_owned();
    validate_limits(&next)?;
    let old = load(paths)?;
    // Never silently send credentials from one site/account to another.
    if old.base_url == next.base_url {
        if next.access_token.is_empty() && old.user_id == next.user_id { next.access_token = old.access_token; }
        if next.api_key.is_empty() { next.api_key = old.api_key; }
    }
    for token in [&next.access_token, &next.api_key] {
        if !token.is_empty() { auth_header(token)?; }
    }
    let bytes = serde_json::to_vec(&next).map_err(|_| "newapi.errors.storage")?;
    secret_file::write(&paths.app_data_dir.join("newapi-connection.dat"), &bytes)?;
    Ok(next.view())
}

pub fn forget(paths: &CodexPaths) -> Result<(), String> {
    match std::fs::remove_file(paths.app_data_dir.join("newapi-connection.dat")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("newapi.errors.storage".into()),
    }
}

pub(crate) fn auth_header(token: &str) -> Result<HeaderValue, String> {
    let token = token.trim().strip_prefix("Bearer ").unwrap_or(token.trim());
    if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_whitespace) {
        return Err("newapi.errors.token".into());
    }
    let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "newapi.errors.token")?;
    value.set_sensitive(true);
    Ok(value)
}

pub(crate) fn client(proxy: &ApiProxyConfigPayload) -> Result<Client, String> {
    let proxy = super::api_client::sanitize_proxy_config(proxy).map_err(|_| "newapi.errors.proxy")?;
    let mut builder = Client::builder().no_proxy().timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none());
    if let Some(url) = proxy.url { builder = builder.proxy(reqwest::Proxy::all(url).map_err(|_| "newapi.errors.proxy")?); }
    builder.build().map_err(|_| "newapi.errors.network".into())
}

/// 按供应商网络模式计算实际代理配置：direct 直连（忽略全局代理），其余跟随全局设置。
pub(crate) fn effective_proxy(network_mode: &str, proxy: &ApiProxyConfigPayload) -> ApiProxyConfigPayload {
    if network_mode == "direct" {
        ApiProxyConfigPayload { mode: super::models::ApiProxyMode::Direct, url: None }
    } else {
        proxy.clone()
    }
}

pub(crate) fn get(client: &Client, url: Url, headers: HeaderMap) -> Result<Value, String> {
    let response = client.get(url).headers(headers).send().map_err(request_error)?;
    response_json(response)
}

fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "newapi.errors.timeout"
    } else {
        "newapi.errors.network"
    }
    .into()
}

fn status_error(status: StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => "newapi.errors.auth",
        404 => "newapi.errors.notFound",
        429 => "newapi.errors.rateLimit",
        300..=399 => "newapi.errors.redirect",
        _ => "newapi.errors.server",
    }
    .into()
}

fn response_json(response: Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(status));
    }
    let mut body = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut body).map_err(|_| "newapi.errors.network")?;
    if body.len() > 2 * 1024 * 1024 { return Err("newapi.errors.response".into()); }
    serde_json::from_slice(&body).map_err(|_| "newapi.errors.response".into())
}

pub(crate) fn endpoint(config: &Connection, path: &str) -> Result<Url, String> {
    base_url(&config.base_url)?.join(path).map_err(|_| "newapi.errors.url".into())
}

fn data(value: &Value) -> Result<&Value, String> {
    if value.get("success").and_then(Value::as_bool) != Some(true) { return Err("newapi.errors.rejected".into()); }
    value.get("data").filter(|v| v.is_object()).ok_or_else(|| "newapi.errors.response".into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Balance {
    pub remaining: f64,
    pub used: Option<f64>,
    pub unit: String,
    pub raw_quota: f64,
    pub quota_per_unit: Option<f64>,
    pub request_count: Option<u64>,
    pub checked_at: i64,
    pub conversion_available: bool,
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn parse_balance(user: &Value, status: Option<&Value>, user_id: u64) -> Result<Balance, String> {
    let user = data(user)?;
    if user.get("id").and_then(Value::as_u64) != Some(user_id) { return Err("newapi.errors.userMismatch".into()); }
    let quota = number(user, "quota").ok_or("newapi.errors.response")?;
    let used = number(user, "used_quota");
    let status = status.and_then(|s| data(s).ok());
    let scale = status.and_then(|s| number(s, "quota_per_unit")).filter(|n| *n > 0.0);
    let display = status.and_then(|s| s.get("quota_display_type")).and_then(Value::as_str);
    let currency = match display {
        Some("USD") => Some(("USD".to_owned(), 1.0)),
        Some("CNY") => status.and_then(|s| number(s, "usd_exchange_rate")).filter(|v| *v > 0.0).map(|r| ("CNY".into(), r)),
        Some("CUSTOM") => status.and_then(|s| {
            let rate = number(s, "custom_currency_exchange_rate").filter(|v| *v > 0.0)?;
            let symbol = s.get("custom_currency_symbol")?.as_str()?.chars().take(12).collect();
            Some((symbol, rate))
        }),
        // Older New API versions use this boolean for the USD display.
        None if status.and_then(|s| s.get("display_in_currency")).and_then(Value::as_bool) == Some(true) => Some(("USD".into(), 1.0)),
        _ => None,
    };
    let converted = scale.zip(currency);
    let (divisor, rate, unit, available) = match converted {
        Some((scale, (unit, rate))) => (scale, rate, unit, true),
        None => (1.0, 1.0, "quota".into(), false),
    };
    Ok(Balance { remaining: quota / divisor * rate, used: used.map(|v| v / divisor * rate), unit,
        raw_quota: quota, quota_per_unit: scale, request_count: user.get("request_count").and_then(Value::as_u64),
        checked_at: chrono::Utc::now().timestamp(), conversion_available: available })
}

pub fn balance(config: &Connection, proxy: &ApiProxyConfigPayload) -> Result<Balance, String> {
    let user_id = config.user_id.filter(|id| *id > 0).ok_or("newapi.errors.userId")?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.access_token)?);
    headers.insert("New-Api-User", HeaderValue::from_str(&user_id.to_string()).map_err(|_| "newapi.errors.userId")?);
    let client = client(proxy)?;
    let user = get(&client, endpoint(config, "api/user/self")?, headers)?;
    let status = get(&client, endpoint(config, "api/status")?, HeaderMap::new()).ok();
    parse_balance(&user, status.as_ref(), user_id)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub id: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub endpoint_types: Vec<String>,
}

fn parse_models(value: &Value) -> Result<Vec<Model>, String> {
    let items = value.get("data").and_then(Value::as_array).ok_or("newapi.errors.response")?;
    let mut models = Vec::new();
    for item in items {
        let id = item.get("id").and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.len() <= 200)
            .ok_or("newapi.errors.response")?;
        if models.iter().any(|m: &Model| m.id == id) { continue; }
        let positive = |key| item.get(key).and_then(Value::as_u64).filter(|v| *v > 0);
        models.push(Model { id: id.into(), context_window: positive("context_window").or_else(|| positive("context_length")),
            max_output_tokens: positive("max_output_tokens"), endpoint_types: item.get("supported_endpoint_types").and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default() });
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

pub fn models(config: &Connection, proxy: &ApiProxyConfigPayload) -> Result<Vec<Model>, String> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.api_key)?);
    parse_models(&get(&client(proxy)?, endpoint(config, "v1/models")?, headers)?)
}

/// 真实连通性测试：向用户选择的模型发送一条 "hi" 聊天请求。
/// 仅 2xx 且返回有效 Chat Completions choices 才算通过。
pub fn test_chat(config: &Connection, model: &str, proxy: &ApiProxyConfigPayload) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
        return Err("newapi.errors.modelRequired".into());
    }
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.api_key)?);
    headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "hi" }],
        "max_tokens": 1,
        "stream": false,
    });
    let value = response_json(
        client(proxy)?
        .post(endpoint(config, "v1/chat/completions")?)
        .headers(headers)
        .json(&body)
        .send()
        .map_err(request_error)?,
    )?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .filter(|choices| !choices.is_empty())
        .map(|_| ())
        .ok_or_else(|| "newapi.errors.response".into())
}

pub fn config_snippet(config: &Connection) -> Result<String, String> {
    validate_limits(config)?;
    if config.model.is_empty() { return Err("newapi.errors.model".into()); }
    let mut document = toml_edit::DocumentMut::new();
    update_document(&mut document, config)?;
    Ok(document.to_string())
}

fn update_document(document: &mut toml_edit::DocumentMut, config: &Connection) -> Result<(), String> {
    use toml_edit::value;
    document["model"] = value(&config.model);
    document["model_provider"] = value("agentharbor_newapi");
    for (key, setting) in [("model_context_window", config.context_window), ("model_auto_compact_token_limit", config.auto_compact_limit)] {
        if let Some(setting) = setting { document[key] = value(setting as i64); } else { document.remove(key); }
    }
    if document.get("model_providers").is_none() {
        let mut providers = toml_edit::Table::new();
        providers.set_implicit(true);
        document["model_providers"] = toml_edit::Item::Table(providers);
    }
    let provider = &mut document["model_providers"]["agentharbor_newapi"];
    *provider = toml_edit::Item::Table(toml_edit::Table::new());
    provider["name"] = value("AgentHarbor New API");
    provider["base_url"] = value(endpoint(config, "v1")?.as_str());
    provider["wire_api"] = value("responses");
    provider["env_key"] = value("AGENT_HARBOR_NEWAPI_KEY");
    provider["requires_openai_auth"] = value(false);
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedConfig { pub path: String, pub backup_path: Option<String> }

// Environment authentication is the default. Embedding a key is an explicit UI choice.
pub fn apply_config(paths: &CodexPaths, config: &Connection, write_api_key: bool) -> Result<AppliedConfig, String> {
    config_snippet(config)?;
    let original = match std::fs::read_to_string(&paths.config_path) {
        Ok(text) => Some(text), Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("newapi.errors.configRead".into()),
    };
    let mut document = original.as_deref().unwrap_or("").parse::<toml_edit::DocumentMut>().map_err(|_| "newapi.errors.configParse")?;
    // Reject scalar/array shapes instead of letting index_mut panic or erase data.
    if document.get("model_providers").is_some_and(|i| !i.is_table()) { return Err("newapi.errors.configParse".into()); }
    update_document(&mut document, config)?;
    if write_api_key {
        auth_header(&config.api_key)?;
        let provider = document["model_providers"]["agentharbor_newapi"].as_table_mut().ok_or("newapi.errors.configParse")?;
        provider.remove("env_key");
        provider["experimental_bearer_token"] = toml_edit::value(config.api_key.trim().strip_prefix("Bearer ").unwrap_or(config.api_key.trim()));
    }
    let backup = if let Some(original) = original {
        let path = paths.app_data_dir.join("config-backups").join(format!("{}.toml", uuid::Uuid::new_v4()));
        storage::write_atomic(&path, original.as_bytes()).map_err(|_| "newapi.errors.storage")?;
        Some(path.display().to_string())
    } else { None };
    storage::write_atomic(&paths.config_path, document.to_string().as_bytes()).map_err(|_| "newapi.errors.storage")?;
    Ok(AppliedConfig { path: paths.config_path.display().to_string(), backup_path: backup })
}

pub fn proxy_from_settings(path: &Path) -> ApiProxyConfigPayload {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<super::repository::CodexMateSettings>(&b).ok())
        .map(|s| s.api_proxy).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};

    fn connection() -> Connection {
        Connection { base_url: "https://example.com/".into(), user_id: Some(42), access_token: "fixture-user-token".into(),
            api_key: "sk-fixture-model-key".into(), model: "fixture-model".into(), ..Default::default() }
    }

    fn request_is_complete(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let Some(header_end) = text.find("\r\n\r\n") else { return false };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + content_length
    }

    fn chat_server(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 { break }
                request.extend_from_slice(&chunk[..count]);
                if request_is_complete(&request) { break }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/"), server)
    }

    #[test]
    fn balance_obeys_site_units_and_does_not_invent_a_conversion() {
        let user = json!({"success":true,"data":{"id":42,"quota":750000,"used_quota":250000,"request_count":9}});
        let status = json!({"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"CNY","usd_exchange_rate":7.3}});
        let b = parse_balance(&user, Some(&status), 42).unwrap();
        assert!((b.remaining - 10.95).abs() < 0.00001);
        assert_eq!(b.unit, "CNY");
        let raw = parse_balance(&user, None, 42).unwrap();
        assert_eq!(raw.remaining, 750000.0);
        assert!(!raw.conversion_available);
        assert!(parse_balance(&user, None, 7).is_err());
        assert!(parse_balance(&json!({"success":false,"message":"secret server text"}), None, 42).is_err());
        let zero = json!({"success":true,"data":{"id":42,"quota":0}});
        assert_eq!(parse_balance(&zero, None, 42).unwrap().remaining, 0.0);
    }

    #[test]
    fn models_preserve_unknown_limits_and_optional_metadata() {
        let result = parse_models(&json!({"data":[{"id":"a"},{"id":"b","context_length":128000,"max_output_tokens":8192,"supported_endpoint_types":["openai-response"]},{"id":"a"}]})).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].context_window, None);
        assert_eq!(result[1].context_window, Some(128000));
        assert!(parse_models(&json!({"error":"not authorized"})).is_err());
    }

    #[test]
    fn provider_network_mode_either_preserves_or_bypasses_the_global_proxy() {
        let global = ApiProxyConfigPayload {
            mode: super::super::models::ApiProxyMode::Manual,
            url: Some("socks5://127.0.0.1:10808".into()),
        };
        assert_eq!(effective_proxy("proxy", &global), global);
        assert_eq!(
            effective_proxy("direct", &global),
            ApiProxyConfigPayload::default()
        );
    }

    #[test]
    fn chat_test_posts_hi_to_the_selected_model() {
        let (base_url, server) = chat_server(
            "200 OK",
            r#"{"id":"chatcmpl-test","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
        );
        let mut connection = connection();
        connection.base_url = base_url;
        test_chat(&connection, "selected-model", &ApiProxyConfigPayload::default()).unwrap();

        let request = server.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(headers.to_ascii_lowercase().contains("authorization: bearer sk-fixture-model-key"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body.get("model").and_then(Value::as_str), Some("selected-model"));
        assert_eq!(body.pointer("/messages/0/content").and_then(Value::as_str), Some("hi"));
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn chat_test_rejects_error_json_even_with_a_success_status() {
        let (base_url, server) = chat_server("200 OK", r#"{"error":{"message":"model unavailable"}}"#);
        let mut connection = connection();
        connection.base_url = base_url;
        assert_eq!(
            test_chat(&connection, "selected-model", &ApiProxyConfigPayload::default()).unwrap_err(),
            "newapi.errors.response"
        );
        server.join().unwrap();
    }

    #[test]
    fn saved_secrets_are_not_returned_and_do_not_follow_a_new_site() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_owned());
        let c = connection();
        let view = save(&paths, c.clone()).unwrap();
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("fixture-user-token"));
        assert!(!encoded.contains("sk-fixture-model-key"));
        assert_eq!(load(&paths).unwrap().access_token, c.access_token);
        #[cfg(windows)]
        assert!(!String::from_utf8_lossy(&std::fs::read(paths.app_data_dir.join("newapi-connection.dat")).unwrap()).contains("fixture-user-token"));
        let mut edit = c.clone(); edit.access_token.clear(); edit.api_key.clear();
        save(&paths, edit.clone()).unwrap();
        assert_eq!(load(&paths).unwrap().api_key, c.api_key);
        edit.base_url = "https://other.example.com".into();
        save(&paths, edit).unwrap();
        assert!(load(&paths).unwrap().api_key.is_empty());
        assert!(load(&paths).unwrap().access_token.is_empty());
        forget(&paths).unwrap();
        assert!(load(&paths).unwrap().base_url.is_empty());
    }

    #[test]
    fn rejects_bad_urls_tokens_and_context_settings() {
        for url in ["file:///tmp/key", "https://user:secret@example.com", "https://example.com/?key=secret", "https://example.com/#token"] { assert!(base_url(url).is_err()); }
        assert_eq!(base_url("https://example.com/sub/v1/").unwrap().as_str(), "https://example.com/sub/");
        assert!(auth_header("token\r\nX-Secret: bad").is_err());
        let mut c = connection(); c.context_window = Some(100); c.auto_compact_limit = Some(100);
        assert!(config_snippet(&c).is_err());
    }

    #[test]
    fn configuration_is_backed_up_and_preserves_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_owned());
        let original = "# keep this comment\nmodel = 'old'\nmodel_context_window = 12345\n[model_providers.other]\nname='Other'\n[mcp_servers.demo]\ncommand='node'\n";
        std::fs::write(&paths.config_path, original).unwrap();
        let mut c = connection(); c.context_window = Some(128000); c.auto_compact_limit = Some(100000);
        let preview = config_snippet(&c).unwrap();
        assert!(!preview.contains(&c.api_key));
        let applied = apply_config(&paths, &c, false).unwrap();
        assert_eq!(std::fs::read_to_string(applied.backup_path.unwrap()).unwrap(), original);
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(text.contains("# keep this comment"));
        let parsed = text.parse::<toml::Value>().unwrap();
        assert_eq!(parsed["mcp_servers"]["demo"]["command"].as_str(), Some("node"));
        assert_eq!(parsed["model_providers"]["other"]["name"].as_str(), Some("Other"));
        assert_eq!(parsed["model_context_window"].as_integer(), Some(128000));
        assert_eq!(parsed["model_providers"]["agentharbor_newapi"]["env_key"].as_str(), Some("AGENT_HARBOR_NEWAPI_KEY"));
        c.context_window = None; c.auto_compact_limit = None;
        apply_config(&paths, &c, true).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains("model_context_window"));
        assert!(text.contains("experimental_bearer_token"));
        for broken in ["[invalid", "model_providers = 3", "model_providers = { other = { name = 'Keep' } }"] {
            std::fs::write(&paths.config_path, broken).unwrap();
            assert!(apply_config(&paths, &c, false).is_err());
            assert_eq!(std::fs::read_to_string(&paths.config_path).unwrap(), broken);
        }
    }

    #[test]
    fn preview_and_fresh_config_include_the_provider_table() {
        let c = connection();
        let preview = config_snippet(&c).unwrap();
        let parsed = preview.parse::<toml::Value>().unwrap();
        assert_eq!(parsed["model_providers"]["agentharbor_newapi"]["base_url"].as_str(), Some("https://example.com/v1"));
        assert_eq!(parsed["model_providers"]["agentharbor_newapi"]["wire_api"].as_str(), Some("responses"));
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_owned());
        let applied = apply_config(&paths, &c, false).unwrap();
        assert!(applied.backup_path.is_none());
        assert_eq!(std::fs::read_to_string(&paths.config_path).unwrap(), preview);
    }

    #[test]
    fn sends_correct_headers_and_never_follows_redirects() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut bytes = [0u8; 8192];
            let count = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_lowercase();
            assert!(request.contains("get /api/user/self "));
            assert!(request.contains("authorization: bearer fixture-user-token"));
            assert!(request.contains("new-api-user: 42"));
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://example.net/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut c = connection(); c.base_url = format!("http://{address}/");
        assert_eq!(balance(&c, &ApiProxyConfigPayload::default()).unwrap_err(), "newapi.errors.redirect");
        server.join().unwrap();
    }

    #[test]
    #[ignore = "Requires an explicitly provided local encrypted test connection"]
    fn live_newapi_smoke() {
        let home = std::env::var("AGENT_HARBOR_NEWAPI_TEST_HOME").expect("Set isolated test home");
        let paths = CodexPaths::from_home(home.into());
        let c = load(&paths).unwrap();
        let proxy = proxy_from_settings(&paths.settings_path);
        let b = balance(&c, &proxy).unwrap();
        let m = models(&c, &proxy).unwrap();
        assert!(b.conversion_available);
        assert!(!m.is_empty());
        println!("Live balance query and model listing passed; {} models, unit {}", m.len(), b.unit);
    }
}
