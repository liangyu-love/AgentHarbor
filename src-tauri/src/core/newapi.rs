use super::models::ApiProxyConfigPayload;
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

pub(crate) fn base_url(text: &str) -> Result<Url, String> {
    let mut url = Url::parse(text.trim()).map_err(|_| "newapi.errors.url")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("newapi.errors.url".into());
    }
    // Accept the commonly pasted OpenAI base URL as well as the site root.
    let path = url
        .path()
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_owned();
    url.set_path(&format!("{path}/"));
    Ok(url)
}

pub(crate) fn validate_limits(config: &Connection) -> Result<(), String> {
    if config
        .context_window
        .is_some_and(|v| v == 0 || v > 100_000_000)
        || config
            .auto_compact_limit
            .is_some_and(|v| v == 0 || v > 100_000_000)
        || matches!((config.context_window, config.auto_compact_limit), (Some(c), Some(a)) if a >= c)
    {
        return Err("newapi.errors.context".into());
    }
    if config.model.len() > 200 || config.model.chars().any(char::is_control) {
        return Err("newapi.errors.model".into());
    }
    Ok(())
}

pub(crate) fn auth_header(token: &str) -> Result<HeaderValue, String> {
    let token = api_key_header(token)?;
    let mut value = HeaderValue::from_str(&format!(
        "Bearer {}",
        token.to_str().map_err(|_| "newapi.errors.token")?
    ))
    .map_err(|_| "newapi.errors.token")?;
    value.set_sensitive(true);
    Ok(value)
}

pub(crate) fn api_key_header(token: &str) -> Result<HeaderValue, String> {
    let token = token.trim().strip_prefix("Bearer ").unwrap_or(token.trim());
    if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_whitespace) {
        return Err("newapi.errors.token".into());
    }
    let mut value = HeaderValue::from_str(token).map_err(|_| "newapi.errors.token")?;
    value.set_sensitive(true);
    Ok(value)
}

pub(crate) fn client(proxy: &ApiProxyConfigPayload) -> Result<Client, String> {
    client_with_timeout(proxy, Duration::from_secs(20))
}

pub(crate) fn client_with_timeout(
    proxy: &ApiProxyConfigPayload,
    timeout: Duration,
) -> Result<Client, String> {
    let proxy =
        super::api_client::sanitize_proxy_config(proxy).map_err(|_| "newapi.errors.proxy")?;
    let mut builder = Client::builder()
        .no_proxy()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url) = proxy.url {
        builder = builder.proxy(reqwest::Proxy::all(url).map_err(|_| "newapi.errors.proxy")?);
    }
    builder.build().map_err(|_| "newapi.errors.network".into())
}

/// 按供应商网络模式计算实际代理配置：direct 直连（忽略全局代理），其余跟随全局设置。
pub(crate) fn effective_proxy(
    network_mode: &str,
    proxy: &ApiProxyConfigPayload,
) -> ApiProxyConfigPayload {
    if network_mode == "direct" {
        ApiProxyConfigPayload {
            mode: super::models::ApiProxyMode::Direct,
            url: None,
        }
    } else {
        proxy.clone()
    }
}

pub(crate) fn get(client: &Client, url: Url, headers: HeaderMap) -> Result<Value, String> {
    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(request_error)?;
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
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "newapi.errors.network")?;
    if body.len() > 2 * 1024 * 1024 {
        return Err("newapi.errors.response".into());
    }
    serde_json::from_slice(&body).map_err(|_| "newapi.errors.response".into())
}

pub(crate) fn endpoint(config: &Connection, path: &str) -> Result<Url, String> {
    base_url(&config.base_url)?
        .join(path)
        .map_err(|_| "newapi.errors.url".into())
}

fn data(value: &Value) -> Result<&Value, String> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err("newapi.errors.rejected".into());
    }
    value
        .get("data")
        .filter(|v| v.is_object())
        .ok_or_else(|| "newapi.errors.response".into())
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
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

fn parse_balance(user: &Value, status: Option<&Value>, user_id: u64) -> Result<Balance, String> {
    let user = data(user)?;
    if user.get("id").and_then(Value::as_u64) != Some(user_id) {
        return Err("newapi.errors.userMismatch".into());
    }
    let quota = number(user, "quota").ok_or("newapi.errors.response")?;
    let used = number(user, "used_quota");
    let status = status.and_then(|s| data(s).ok());
    let scale = status
        .and_then(|s| number(s, "quota_per_unit"))
        .filter(|n| *n > 0.0);
    let display = status
        .and_then(|s| s.get("quota_display_type"))
        .and_then(Value::as_str);
    let currency = match display {
        Some("USD") => Some(("USD".to_owned(), 1.0)),
        Some("CNY") => status
            .and_then(|s| number(s, "usd_exchange_rate"))
            .filter(|v| *v > 0.0)
            .map(|r| ("CNY".into(), r)),
        Some("CUSTOM") => status.and_then(|s| {
            let rate = number(s, "custom_currency_exchange_rate").filter(|v| *v > 0.0)?;
            let symbol = s
                .get("custom_currency_symbol")?
                .as_str()?
                .chars()
                .take(12)
                .collect();
            Some((symbol, rate))
        }),
        // Older New API versions use this boolean for the USD display.
        None if status
            .and_then(|s| s.get("display_in_currency"))
            .and_then(Value::as_bool)
            == Some(true) =>
        {
            Some(("USD".into(), 1.0))
        }
        _ => None,
    };
    let converted = scale.zip(currency);
    let (divisor, rate, unit, available) = match converted {
        Some((scale, (unit, rate))) => (scale, rate, unit, true),
        None => (1.0, 1.0, "quota".into(), false),
    };
    Ok(Balance {
        remaining: quota / divisor * rate,
        used: used.map(|v| v / divisor * rate),
        unit,
        raw_quota: quota,
        quota_per_unit: scale,
        request_count: user.get("request_count").and_then(Value::as_u64),
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: available,
    })
}

pub fn balance(config: &Connection, proxy: &ApiProxyConfigPayload) -> Result<Balance, String> {
    let user_id = config
        .user_id
        .filter(|id| *id > 0)
        .ok_or("newapi.errors.userId")?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.access_token)?);
    headers.insert(
        "New-Api-User",
        HeaderValue::from_str(&user_id.to_string()).map_err(|_| "newapi.errors.userId")?,
    );
    let client = client(proxy)?;
    let user = get(&client, endpoint(config, "api/user/self")?, headers)?;
    let status = get(&client, endpoint(config, "api/status")?, HeaderMap::new()).ok();
    parse_balance(&user, status.as_ref(), user_id)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub endpoint_types: Vec<String>,
    #[serde(default = "model_included_by_default")]
    pub included: bool,
    pub metadata_source: String,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            context_window: None,
            max_output_tokens: None,
            endpoint_types: Vec::new(),
            included: true,
            metadata_source: String::new(),
        }
    }
}

fn model_included_by_default() -> bool {
    true
}

impl Model {
    pub fn with_local_defaults(mut self) -> Self {
        let defaults = super::model_defaults::for_model(&self.id);
        // A locally edited value is authoritative; relay metadata and the local
        // catalog only fill what is still absent.
        if self.display_name.trim().is_empty() {
            self.display_name = self.id.clone();
        }
        if self.context_window.is_none() {
            self.context_window = Some(defaults.context_window);
        }
        if self.max_output_tokens.is_none() {
            self.max_output_tokens = Some(defaults.max_output_tokens);
        }
        if self.metadata_source.trim().is_empty() {
            self.metadata_source = "local".into();
        }
        self
    }

    /// Prefer the signed-in Codex catalog over our heuristic limits.
    ///
    /// Codex keeps `models_cache.json` freshly synced with the signed-in account,
    /// so it is the most accurate local source. Values a relay reported, and any
    /// value the user edited, always win over this.
    pub(crate) fn with_catalog_defaults(
        mut self,
        catalog: &super::model_defaults::Catalog,
    ) -> Self {
        if let Some(entry) = catalog.get(&self.id) {
            let guess = super::model_defaults::for_model(&self.id);
            let replace_context = self.context_window.is_none()
                || self.metadata_source == "catalog"
                || (self.metadata_source == "local"
                    && self.context_window == Some(guess.context_window));
            if replace_context {
                self.context_window = entry.context_window;
                if self.max_output_tokens.is_none()
                    || (matches!(self.metadata_source.as_str(), "local" | "catalog")
                        && self.max_output_tokens == Some(guess.max_output_tokens))
                {
                    self.max_output_tokens = entry.max_output_tokens.or_else(|| {
                        entry
                            .context_window
                            .map(|limit| guess.max_output_tokens.min(limit))
                    });
                }
                if let (Some(context), Some(output)) = (self.context_window, self.max_output_tokens)
                {
                    if output > context {
                        self.max_output_tokens = Some(context);
                    }
                }
                self.metadata_source = "catalog".into();
            } else if self.max_output_tokens.is_none() {
                self.max_output_tokens = entry.max_output_tokens;
            }
        }
        self
    }
}

fn parse_models(value: &Value) -> Result<Vec<Model>, String> {
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or("newapi.errors.response")?;
    let mut models = Vec::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty() && s.len() <= 200)
            .ok_or("newapi.errors.response")?;
        if models.iter().any(|m: &Model| m.id == id) {
            continue;
        }
        let positive = |key| item.get(key).and_then(Value::as_u64).filter(|v| *v > 0);
        let upstream_context = positive("context_window").or_else(|| positive("context_length"));
        let upstream_output = positive("max_output_tokens");
        models.push(
            Model {
                id: id.into(),
                // The menu starts with a stable provider/model label. A user can
                // replace it with an alias in the model editor.
                display_name: id.to_owned(),
                context_window: upstream_context,
                max_output_tokens: upstream_output,
                endpoint_types: item
                    .get("supported_endpoint_types")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                included: true,
                metadata_source: if upstream_context.is_some() {
                    "upstream".into()
                } else {
                    "local".into()
                },
            }
            .with_local_defaults(),
        );
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

pub fn models(config: &Connection, proxy: &ApiProxyConfigPayload) -> Result<Vec<Model>, String> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.api_key)?);
    parse_models(&get(
        &client(proxy)?,
        endpoint(config, "v1/models")?,
        headers,
    )?)
}

pub fn anthropic_models(
    config: &Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<Vec<Model>, String> {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", api_key_header(&config.api_key)?);
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    parse_models(&get(
        &client(proxy)?,
        endpoint(config, "v1/models")?,
        headers,
    )?)
}

/// 真实连通性测试：向用户选择的模型发送一条 "hi" 聊天请求。
/// 仅 2xx 且返回有效 Chat Completions choices 才算通过。
pub fn test_chat(
    config: &Connection,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), String> {
    let model = validated_test_model(model)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.api_key)?);
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
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

pub fn test_responses(
    config: &Connection,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), String> {
    let model = validated_test_model(model)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth_header(&config.api_key)?);
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let value = response_json(
        client(proxy)?
            .post(endpoint(config, "v1/responses")?)
            .headers(headers)
            .json(&serde_json::json!({
                "model": model,
                "input": "hi",
                "max_output_tokens": 1,
                "stream": false,
            }))
            .send()
            .map_err(request_error)?,
    )?;
    (value.get("object").and_then(Value::as_str) == Some("response"))
        .then_some(())
        .ok_or_else(|| "newapi.errors.response".into())
}

pub fn test_anthropic_messages(
    config: &Connection,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), String> {
    let model = validated_test_model(model)?;
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", api_key_header(&config.api_key)?);
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let value = response_json(
        client(proxy)?
            .post(endpoint(config, "v1/messages")?)
            .headers(headers)
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "hi" }],
                "stream": false,
            }))
            .send()
            .map_err(request_error)?,
    )?;
    (value.get("type").and_then(Value::as_str) == Some("message"))
        .then_some(())
        .ok_or_else(|| "newapi.errors.response".into())
}

fn validated_test_model(model: &str) -> Result<&str, String> {
    let model = model.trim();
    if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
        return Err("newapi.errors.modelRequired".into());
    }
    Ok(model)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedConfig {
    pub path: String,
    pub backup_path: Option<String>,
}

pub fn proxy_from_settings(path: &Path) -> ApiProxyConfigPayload {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<super::repository::AgentHarborSettings>(&b).ok())
        .map(|s| s.api_proxy)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};

    fn connection() -> Connection {
        Connection {
            base_url: "https://example.com/".into(),
            user_id: Some(42),
            access_token: "fixture-user-token".into(),
            api_key: "sk-fixture-model-key".into(),
            model: "fixture-model".into(),
            ..Default::default()
        }
    }

    fn request_is_complete(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let Some(header_end) = text.find("\r\n\r\n") else {
            return false;
        };
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
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request_is_complete(&request) {
                    break;
                }
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
        assert!(parse_balance(
            &json!({"success":false,"message":"secret server text"}),
            None,
            42
        )
        .is_err());
        let zero = json!({"success":true,"data":{"id":42,"quota":0}});
        assert_eq!(parse_balance(&zero, None, 42).unwrap().remaining, 0.0);
    }

    #[test]
    fn models_fill_missing_limits_from_editable_local_defaults() {
        let result = parse_models(&json!({"data":[{"id":"a"},{"id":"b","context_length":128000,"max_output_tokens":8192,"supported_endpoint_types":["openai-response"]},{"id":"a"}]})).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].context_window, Some(128_000));
        assert_eq!(result[0].max_output_tokens, Some(8192));
        assert_eq!(result[0].display_name, "a");
        assert_eq!(result[0].metadata_source, "local");
        assert_eq!(result[1].context_window, Some(128000));
        assert_eq!(result[1].metadata_source, "upstream");
        assert!(parse_models(&json!({"error":"not authorized"})).is_err());
    }

    #[test]
    fn saved_context_limit_is_not_replaced_by_local_defaults() {
        let model = Model {
            id: "claude-custom".into(),
            context_window: Some(128_000),
            metadata_source: "local".into(),
            ..Default::default()
        }
        .with_local_defaults();
        assert_eq!(model.context_window, Some(128_000));
    }

    #[test]
    fn synced_catalog_replaces_guesses_but_not_manual_limits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-6-luna","context_window":272000}]}"#,
        )
        .unwrap();
        let catalog = super::super::model_defaults::Catalog::load(dir.path());
        let guessed = Model {
            id: "gpt-6-luna".into(),
            context_window: Some(400_000),
            max_output_tokens: Some(128_000),
            metadata_source: "local".into(),
            ..Default::default()
        }
        .with_catalog_defaults(&catalog);
        assert_eq!(guessed.context_window, Some(272_000));
        assert_eq!(guessed.metadata_source, "catalog");
        let manual = Model {
            context_window: Some(400_000),
            metadata_source: "manual".into(),
            ..guessed.clone()
        }
        .with_catalog_defaults(&catalog);
        assert_eq!(manual.context_window, Some(400_000));
        let upstream = Model {
            context_window: Some(400_000),
            metadata_source: "upstream".into(),
            ..guessed
        }
        .with_catalog_defaults(&catalog);
        assert_eq!(upstream.context_window, Some(400_000));
    }

    #[test]
    fn anthropic_model_listing_uses_api_key_and_version_headers() {
        let (base_url, server) = chat_server(
            "200 OK",
            r#"{"data":[{"id":"claude-test","type":"model"}]}"#,
        );
        let mut connection = connection();
        connection.base_url = base_url;

        let models = anthropic_models(&connection, &ApiProxyConfigPayload::default()).unwrap();

        assert_eq!(models[0].id, "claude-test");
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models http/1.1"));
        assert!(request.contains("x-api-key: sk-fixture-model-key"));
        assert!(request.contains("anthropic-version: 2023-06-01"));
        assert!(!request.contains("authorization: bearer"));
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
        test_chat(
            &connection,
            "selected-model",
            &ApiProxyConfigPayload::default(),
        )
        .unwrap();

        let request = server.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-fixture-model-key"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("selected-model")
        );
        assert_eq!(
            body.pointer("/messages/0/content").and_then(Value::as_str),
            Some("hi")
        );
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn responses_test_uses_responses_endpoint_and_shape() {
        let (base_url, server) = chat_server(
            "200 OK",
            r#"{"id":"resp_test","object":"response","output":[]}"#,
        );
        let mut connection = connection();
        connection.base_url = base_url;
        test_responses(
            &connection,
            "selected-model",
            &ApiProxyConfigPayload::default(),
        )
        .unwrap();

        let request = server.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-fixture-model-key"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("selected-model")
        );
        assert_eq!(body.get("input").and_then(Value::as_str), Some("hi"));
    }

    #[test]
    fn anthropic_test_uses_messages_headers_and_shape() {
        let (base_url, server) = chat_server(
            "200 OK",
            r#"{"id":"msg_test","type":"message","content":[]}"#,
        );
        let mut connection = connection();
        connection.base_url = base_url;
        test_anthropic_messages(
            &connection,
            "claude-test",
            &ApiProxyConfigPayload::default(),
        )
        .unwrap();

        let request = server.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        let headers = headers.to_ascii_lowercase();
        assert!(headers.starts_with("post /v1/messages http/1.1"));
        assert!(headers.contains("x-api-key: sk-fixture-model-key"));
        assert!(headers.contains("anthropic-version: 2023-06-01"));
        assert!(!headers.contains("authorization: bearer"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("claude-test")
        );
    }

    #[test]
    fn chat_test_rejects_error_json_even_with_a_success_status() {
        let (base_url, server) =
            chat_server("200 OK", r#"{"error":{"message":"model unavailable"}}"#);
        let mut connection = connection();
        connection.base_url = base_url;
        assert_eq!(
            test_chat(
                &connection,
                "selected-model",
                &ApiProxyConfigPayload::default()
            )
            .unwrap_err(),
            "newapi.errors.response"
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_bad_urls_tokens_and_context_settings() {
        for url in [
            "file:///tmp/key",
            "https://user:secret@example.com",
            "https://example.com/?key=secret",
            "https://example.com/#token",
        ] {
            assert!(base_url(url).is_err());
        }
        assert_eq!(
            base_url("https://example.com/sub/v1/").unwrap().as_str(),
            "https://example.com/sub/"
        );
        assert!(auth_header("token\r\nX-Secret: bad").is_err());
        let mut c = connection();
        c.context_window = Some(100);
        c.auto_compact_limit = Some(100);
        assert!(validate_limits(&c).is_err());
    }

    #[test]
    fn sends_correct_headers_and_never_follows_redirects() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0u8; 8192];
            let count = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_lowercase();
            assert!(request.contains("get /api/user/self "));
            assert!(request.contains("authorization: bearer fixture-user-token"));
            assert!(request.contains("new-api-user: 42"));
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://example.net/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut c = connection();
        c.base_url = format!("http://{address}/");
        assert_eq!(
            balance(&c, &ApiProxyConfigPayload::default()).unwrap_err(),
            "newapi.errors.redirect"
        );
        server.join().unwrap();
    }
}
