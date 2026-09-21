use super::models::ApiProxyConfigPayload;
use super::router;
use crate::platform::paths::CodexPaths;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{
    HeaderMap, HeaderValue, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE,
    COOKIE, EXPECT, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING,
    UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;
const CHATGPT_CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex/";
const OPENAI_API_BASE: &str = "https://api.openai.com/v1/";

pub(crate) type ResponseBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

struct ProxyHandle {
    port: u16,
    shutdown: tokio::sync::watch::Sender<bool>,
}

static HANDLE: OnceLock<Mutex<Option<ProxyHandle>>> = OnceLock::new();
static START_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn handle_slot() -> &'static Mutex<Option<ProxyHandle>> {
    HANDLE.get_or_init(|| Mutex::new(None))
}

fn start_lock() -> &'static tokio::sync::Mutex<()> {
    START_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn is_running() -> bool {
    handle_slot().lock().map(|guard| guard.is_some()).unwrap_or(false)
}

pub fn running_port() -> Option<u16> {
    handle_slot().lock().ok().and_then(|guard| guard.as_ref().map(|handle| handle.port))
}

pub fn stop() {
    if let Ok(mut guard) = handle_slot().lock() {
        if let Some(handle) = guard.take() {
            let _ = handle.shutdown.send(true);
        }
    }
}

/// Bind the router on localhost. Returns the actually bound port.
/// Called from async commands and app setup.
pub async fn ensure_running(port: u16, paths: CodexPaths, proxy: ApiProxyConfigPayload) -> Result<u16, String> {
    let _start_guard = start_lock().lock().await;
    {
        let guard = handle_slot().lock().map_err(|_| "newapi.errors.storage")?;
        if let Some(handle) = guard.as_ref() {
            if handle.port == port {
                return Ok(handle.port);
            }
        }
    }
    stop();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|e| -> String {
            if e.kind() == std::io::ErrorKind::AddrInUse { "newapi.errors.portBusy".to_owned() } else { "newapi.errors.network".to_owned() }
        })?;
    let bound_port = listener.local_addr().map_err(|_| "newapi.errors.network".to_owned())?.port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let context = std::sync::Arc::new(ArcContext {
        store_path: paths.app_data_dir.join("newapi-providers.dat"),
        app_data_dir: paths.app_data_dir.clone(),
        auth_path: paths.auth_path.clone(),
        outbound: outbound_client(&proxy)?,
        outbound_direct: outbound_client(&ApiProxyConfigPayload::default())?,
    });
    tokio::spawn(accept_loop(listener, shutdown_rx, context));
    *handle_slot().lock().map_err(|_| "newapi.errors.storage")? = Some(ProxyHandle { port: bound_port, shutdown: shutdown_tx });
    Ok(bound_port)
}

struct ArcContext {
    store_path: PathBuf,
    app_data_dir: PathBuf,
    auth_path: PathBuf,
    outbound: Client,
    /// 供应商网络模式为 direct 时使用的无代理客户端。
    outbound_direct: Client,
}

async fn accept_loop(listener: tokio::net::TcpListener, mut shutdown: tokio::sync::watch::Receiver<bool>, context: std::sync::Arc<ArcContext>) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let context = context.clone();
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service_fn(move |request| forward(request, context.clone())))
                        .await;
                });
            }
        }
    }
}

fn outbound_client(proxy: &ApiProxyConfigPayload) -> Result<Client, String> {
    let proxy = super::api_client::sanitize_proxy_config(proxy).map_err(|_| "newapi.errors.proxy")?;
    let mut builder = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url) = proxy.url {
        builder = builder.proxy(reqwest::Proxy::all(url).map_err(|_| "newapi.errors.proxy")?);
    }
    builder.build().map_err(|_| "newapi.errors.network".into())
}

fn full_body(text: String) -> ResponseBody {
    Full::new(Bytes::from(text)).map_err(|never| match never {}).boxed()
}

fn json_response(status: StatusCode, message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(full_body(json!({"error": {"message": message, "type": "agentharbor_local_router"}}).to_string()))
        .expect("static response parts")
}

async fn forward(request: Request<Incoming>, context: std::sync::Arc<ArcContext>) -> Result<Response<ResponseBody>, std::convert::Infallible> {
    Ok(handle(request, context).await.unwrap_or_else(|(status, message)| json_response(status, message)))
}

type ForwardResult = Result<Response<ResponseBody>, (StatusCode, &'static str)>;

enum Destination {
    /// Relay traffic: forward to this provider's base URL with its API key.
    Provider { provider: Box<router::Provider>, model_override: Option<String> },
    /// Official OpenAI models: forward using the local Codex credentials.
    Official,
}

async fn handle(request: Request<Incoming>, context: std::sync::Arc<ArcContext>) -> ForwardResult {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    if path == "/agentharbor/health" {
        return Ok(Response::builder().status(StatusCode::OK).body(full_body("ok".to_owned())).expect("static body"));
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let store = router::read_store(&context.store_path)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "router storage unavailable"))?;
    if !authorized(request.headers(), &store.router.bearer_token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid local router token"));
    }

    let method = request.method().clone();
    let has_body = !matches!(method, hyper::Method::GET | hyper::Method::HEAD | hyper::Method::DELETE);
    let mut headers = request.headers().clone();
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BODY as u64)
    {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large"));
    }
    for header in [
        CONNECTION,
        CONTENT_LENGTH,
        TRANSFER_ENCODING,
        UPGRADE,
        AUTHORIZATION,
        ACCEPT_ENCODING,
        EXPECT,
        HOST,
        COOKIE,
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
        TE,
        TRAILER,
    ] {
        headers.remove(header);
    }
    for header in ["x-api-key", "chatgpt-account-id"] {
        headers.remove(header);
    }
    let body: Option<Bytes> = if has_body {
        Some(read_limited_body(request.into_body()).await?)
    } else {
        read_limited_body(request.into_body()).await?;
        None
    };

    let destination = match segments.as_slice() {
        // Codex model catalog for the model picker (`model_catalog_json` mirror).
        ["codex", "v1", "models"] if !has_body => {
            let catalog = router::catalog_value(&context.app_data_dir, &store);
            let entries: Vec<Value> = catalog
                .get("models")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|entry| {
                            let mut entry = entry.clone();
                            if let Some(slug) = entry.get("slug").and_then(Value::as_str) {
                                entry["id"] = json!(slug);
                                entry["object"] = json!("model");
                            }
                            entry
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(full_body(json!({"object": "list", "data": entries}).to_string()))
                .expect("static response parts"));
        }
        // Codex protocol: model slug `<provider id>::<model>` decides the route.
        ["codex", "v1", ..] => {
            let model = body.as_deref().and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
                .and_then(|value| value.get("model").and_then(Value::as_str).map(str::to_owned));
            match model.as_deref().and_then(|model| model.split_once("::")) {
                Some((provider_id, model_id)) => {
                    let provider = store.providers.iter().find(|p| p.id == provider_id).cloned();
                    match provider {
                        Some(provider) if provider.is_routable() => {
                            Destination::Provider { provider: Box::new(provider), model_override: Some(model_id.to_owned()) }
                        }
                        Some(_) => return Err((StatusCode::SERVICE_UNAVAILABLE, "provider is not enabled for routing")),
                        None => return Err((StatusCode::NOT_FOUND, "unknown provider")),
                    }
                }
                // Official model slugs (and anything unprefixed) go to OpenAI with local credentials.
                _ => Destination::Official,
            }
        }
        // Legacy direct routes.
        ["p", id, "v1", ..] => {
            let provider = store.providers.iter().find(|p| p.id == *id).cloned();
            match provider {
                Some(provider) if provider.is_routable() => Destination::Provider { provider: Box::new(provider), model_override: None },
                Some(_) => return Err((StatusCode::SERVICE_UNAVAILABLE, "provider is not enabled for routing")),
                None => return Err((StatusCode::NOT_FOUND, "unknown provider")),
            }
        }
        ["v1", ..] => match router::effective_active(&store) {
            Some(provider) => Destination::Provider { provider: Box::new(provider.clone()), model_override: None },
            None => return Err((StatusCode::SERVICE_UNAVAILABLE, "no enabled provider for routing")),
        },
        _ => return Err((StatusCode::NOT_FOUND, "unsupported path")),
    };

    let suffix = match segments.as_slice() {
        ["p", _, "v1", rest @ ..] | ["codex", "v1", rest @ ..] | ["v1", rest @ ..] => rest.join("/"),
        _ => String::new(),
    };

    let (url, headers) = match &destination {
        Destination::Provider { provider, .. } => {
            headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", provider.api_key)).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid provider key"))?);
            (format!("{}v1/{}{}", provider.base_url, suffix, query.map(|q| format!("?{q}")).unwrap_or_default()), headers)
        }
        Destination::Official => {
            let auth = crate::core::auth::load_auth_file(&context.auth_path).ok();
            let access_token = auth.as_ref().and_then(|a| a.tokens.access_token.as_deref()).map(str::to_owned)
                .filter(|t| !t.trim().is_empty());
            if let Some(token) = access_token {
                headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid token"))?);
                if let Some(account_id) = auth.as_ref().and_then(|a| a.tokens.account_id.as_deref()).filter(|s| !s.is_empty()) {
                    if let Ok(value) = HeaderValue::from_str(account_id) {
                        headers.insert("chatgpt-account-id", value);
                    }
                }
                (format!("{CHATGPT_CODEX_BASE}{suffix}{}", query.map(|q| format!("?{q}")).unwrap_or_default()), headers)
            } else if let Some(key) = auth.and_then(|a| a.openai_api_key).filter(|k| !k.trim().is_empty()) {
                headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid key"))?);
                (format!("{OPENAI_API_BASE}{suffix}{}", query.map(|q| format!("?{q}")).unwrap_or_default()), headers)
            } else {
                return Err((StatusCode::UNAUTHORIZED, "no OpenAI credentials found for official models"));
            }
        }
    };

    let body = match (&destination, body) {
        (_, None) => None,
        (Destination::Provider { model_override: Some(model_id), .. }, Some(bytes)) => Some(rewrite_model(&bytes, model_id)?),
        (_, Some(bytes)) => Some(bytes),
    };

    let outbound_client = match &destination {
        Destination::Provider { provider, .. } if provider.network_mode() == "direct" => {
            &context.outbound_direct
        }
        _ => &context.outbound,
    };
    let mut outbound = outbound_client.request(method, url).headers(headers);
    if let Some(body) = body {
        outbound = outbound.body(body);
    }
    let response = outbound.send().await.map_err(|_| (StatusCode::BAD_GATEWAY, "cannot reach the provider"))?;

    let mut builder = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if matches!(*name, CONNECTION | CONTENT_LENGTH | TRANSFER_ENCODING | UPGRADE) {
            continue;
        }
        builder = builder.header(name, value);
    }
    let stream = response
        .bytes_stream()
        .map(|chunk| chunk.map(Frame::data).map_err(Into::into));
    builder.body(BodyExt::boxed(StreamBody::new(stream))).map_err(|_| (StatusCode::BAD_GATEWAY, "invalid upstream response"))
}

fn authorized(headers: &HeaderMap, bearer_token: &str) -> bool {
    if bearer_token.is_empty() {
        return false;
    }
    let Ok(expected) = HeaderValue::from_str(&format!("Bearer {bearer_token}")) else {
        return false;
    };
    headers.get(AUTHORIZATION) == Some(&expected)
}

async fn read_limited_body(mut body: Incoming) -> Result<Bytes, (StatusCode, &'static str)> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| (StatusCode::BAD_REQUEST, "unreadable request body"))?;
        if let Some(data) = frame.data_ref() {
            if bytes.len().saturating_add(data.len()) > MAX_REQUEST_BODY {
                return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large"));
            }
            bytes.extend_from_slice(data);
        }
    }
    Ok(Bytes::from(bytes))
}

fn rewrite_model(body: &Bytes, model_id: &str) -> Result<Bytes, (StatusCode, &'static str)> {
    let mut value: Value = serde_json::from_slice(body).map_err(|_| (StatusCode::BAD_REQUEST, "request body is not JSON"))?;
    value
        .as_object_mut()
        .ok_or((StatusCode::BAD_REQUEST, "request body is not an object"))?
        .insert("model".to_owned(), json!(model_id));
    let bytes = serde_json::to_vec(&value).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "cannot re-encode request"))?;
    Ok(Bytes::from(bytes))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::router::{Provider, ProviderInput, RouterSettings, Store};
    use crate::platform::paths::CodexPaths;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn proxy_serves_the_codex_catalog_and_routes_prefixed_models() {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let upstream_port = upstream.local_addr().unwrap().port();
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = upstream.accept().await else { break };
                    let mut buffer = vec![0u8; 16384];
                    let Ok(count) = socket.read(&mut buffer).await else { break };
                    let request = String::from_utf8_lossy(&buffer[..count]).to_lowercase();
                    assert!(request.contains("authorization: bearer sk-upstream"), "upstream must see the provider key");
                    assert!(!request.contains("sk-codex-dummy"), "the inbound dummy key must be replaced");
                    assert!(request.contains("/v1/"), "path must be rewritten to the provider root");
                    assert!(!request.contains("agentharbor_fixture"), "the model slug prefix must be stripped");
                    socket.write_all(b"HTTP/1.1 200 OK
content-type: application/json
content-length: 22

{\"object\":\"list\",\"n\":1}").await.unwrap();
                }
            });

            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().to_owned());
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            let provider = Provider {
                id: "agentharbor_fixture".into(),
                name: "Mock".into(),
                base_url: format!("http://127.0.0.1:{upstream_port}/"),
                api_key: "sk-upstream".into(),
                model: "mock-model".into(),
                enabled: true,
                test_status: "ok".into(),
                models: vec![super::super::newapi::Model { id: "mock-model".into(), context_window: Some(128000), max_output_tokens: None, endpoint_types: vec![] }],
                ..Default::default()
            };
            let store = Store {
                version: 1,
                providers: vec![provider],
                active_provider_id: Some("agentharbor_fixture".into()),
                router: RouterSettings { bearer_token: "router-test-token".into(), ..Default::default() },
            };
            crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &serde_json::to_vec(&store).unwrap()).unwrap();

            let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
            assert!(port > 0);
            let client = Client::new();
            let base = format!("http://127.0.0.1:{port}");

            // The catalog endpoint lists the relay model under its prefixed slug.
            let unauthorized = client.get(format!("{base}/codex/v1/models")).send().await.unwrap();
            assert_eq!(unauthorized.status(), 401);
            let wrong_token = client
                .get(format!("{base}/codex/v1/models"))
                .header(AUTHORIZATION, "Bearer wrong")
                .send()
                .await
                .unwrap();
            assert_eq!(wrong_token.status(), 401);

            let catalog: Value = client
                .get(format!("{base}/codex/v1/models"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let entries = catalog.get("data").and_then(Value::as_array).unwrap();
            assert_eq!(entries[0].get("id").and_then(Value::as_str), Some("agentharbor_fixture::mock-model"));

            // Codex-style POST with a prefixed model is routed and the prefix stripped.
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .json(&json!({"model": "agentharbor_fixture::mock-model", "input": "hi"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);

            // An official (unprefixed) model without local credentials fails clearly.
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .json(&json!({"model": "gpt-6-astra", "input": "hi"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401);

            // Legacy per-provider and default routes keep working.
            let response = client
                .get(format!("{base}/p/agentharbor_fixture/v1/models"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let response = client
                .get(format!("{base}/v1/models"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);

            let mut oversized = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            oversized
                .write_all(
                    format!(
                        "POST /codex/v1/responses HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer router-test-token\r\nContent-Length: {}\r\n\r\n",
                        MAX_REQUEST_BODY + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = [0u8; 1024];
            let count = oversized.read(&mut response).await.unwrap();
            assert!(String::from_utf8_lossy(&response[..count]).starts_with("HTTP/1.1 413"));

            let health = client.get(format!("{base}/agentharbor/health")).send().await.unwrap();
            assert_eq!(health.status(), 200);
            stop();
        });
    }

    #[test]
    #[ignore = "Requires an explicitly provided local encrypted test connection"]
    fn live_smart_routing_injection() {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let source_home = std::path::PathBuf::from(
                std::env::var("AGENT_HARBOR_NEWAPI_TEST_HOME").expect("Set isolated test home"),
            );
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            std::fs::copy(
                source_home.join("codexmate/newapi-connection.dat"),
                paths.app_data_dir.join("newapi-connection.dat"),
            )
            .unwrap();
            let source_settings = source_home.join("codexmate/settings.json");
            if source_settings.exists() {
                std::fs::copy(source_settings, &paths.settings_path).unwrap();
            }

            let proxy = crate::core::newapi::proxy_from_settings(&paths.settings_path);
            let initial = router::load_store(&paths).unwrap();
            let provider = initial.providers.first().cloned().expect("test relay connection");
            let models = {
                let paths = paths.clone();
                let provider_id = provider.id.clone();
                let base_url = provider.base_url.clone();
                let proxy = proxy.clone();
                tokio::task::spawn_blocking(move || {
                    router::fetch_models(&paths, Some(&provider_id), &base_url, "", &proxy)
                })
                .await
                .unwrap()
                .unwrap()
            };
            let test_model = models
                .iter()
                .find(|model| model.id == provider.model)
                .or_else(|| models.first())
                .expect("at least one relay model")
                .id
                .clone();
            let tested = {
                let paths = paths.clone();
                let provider_id = provider.id.clone();
                let proxy = proxy.clone();
                let test_model = test_model.clone();
                tokio::task::spawn_blocking(move || {
                    router::test_provider(&paths, &provider_id, &test_model, &proxy)
                })
                .await
                .unwrap()
                .unwrap()
            };
            assert_eq!(tested.test_status, "ok");
            let balance = {
                let paths = paths.clone();
                let provider_id = provider.id.clone();
                let proxy = proxy.clone();
                tokio::task::spawn_blocking(move || {
                    router::query_provider_balance(&paths, &provider_id, &proxy)
                })
                .await
                .unwrap()
                .unwrap()
            };
            assert!(balance.balance.is_some());
            let model = tested
                .models
                .iter()
                .find(|model| model.id == "gpt-5.6-luna")
                .or_else(|| tested.models.first())
                .expect("at least one relay model")
                .id
                .clone();
            router::save_provider(
                &paths,
                ProviderInput {
                    id: Some(provider.id.clone()),
                    name: provider.name,
                    base_url: provider.base_url,
                    user_id: provider.user_id,
                    access_token: provider.access_token,
                    api_key: provider.api_key,
                    model,
                    context_window: provider.context_window,
                    auto_compact_limit: provider.auto_compact_limit,
                    network_mode: provider.network_mode,
                },
            )
            .unwrap();
            router::set_enabled(&paths, &provider.id, true).unwrap();

            let port = {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
                listener.local_addr().unwrap().port()
            };
            let mut store = router::load_store(&paths).unwrap();
            store.router.port = port;
            crate::core::secret_file::write(
                &paths.app_data_dir.join("newapi-providers.dat"),
                &serde_json::to_vec(&store).unwrap(),
            )
            .unwrap();
            assert!(
                !paths.auth_path.exists(),
                "isolated Codex home must not contain auth.json"
            );
            router::set_accountless(&paths, true).unwrap();
            router::set_smart_routing(&paths, true).unwrap();
            let store = router::load_store(&paths).unwrap();
            ensure_running(port, paths.clone(), proxy).await.unwrap();

            let config: toml::Value = std::fs::read_to_string(&paths.config_path)
                .unwrap()
                .parse()
                .unwrap();
            let injected = &config["model_providers"][router::CATALOG_PROVIDER_KEY];
            assert_eq!(
                injected["base_url"].as_str(),
                Some(format!("http://127.0.0.1:{port}/codex/v1").as_str())
            );
            assert_eq!(injected["wire_api"].as_str(), Some("responses"));
            assert_eq!(injected["requires_openai_auth"].as_bool(), Some(false));
            assert_eq!(
                injected["experimental_bearer_token"].as_str(),
                Some(store.router.bearer_token.as_str())
            );
            assert_eq!(config["preferred_auth_method"].as_str(), Some("apikey"));
            assert!(
                !paths.auth_path.exists(),
                "accountless routing must not create auth.json"
            );

            let client = Client::builder().timeout(Duration::from_secs(90)).build().unwrap();
            let base = format!("http://127.0.0.1:{port}");
            let catalog: Value = client
                .get(format!("{base}/codex/v1/models"))
                .bearer_auth(&store.router.bearer_token)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let slug = catalog["data"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|entry| entry["id"].as_str().filter(|id| id.starts_with(&provider.id)))
                .expect("prefixed relay model")
                .to_owned();
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .bearer_auth(&store.router.bearer_token)
                .json(&json!({"model": slug, "input": "Reply with exactly OK.", "stream": true}))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success(), "relay returned HTTP {}", response.status());
            let body = response.text().await.unwrap();
            assert!(body.contains("response.completed"), "response stream did not complete");
            stop();
        });
    }
}
