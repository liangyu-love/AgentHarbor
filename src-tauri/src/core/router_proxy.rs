use super::models::ApiProxyConfigPayload;
use super::{protocol, relay_usage, router};
use crate::platform::paths::CodexPaths;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_LENGTH,
    CONTENT_TYPE, COOKIE, EXPECT, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;
const CHATGPT_CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex/";
const OPENAI_API_BASE: &str = "https://api.openai.com/v1/";

pub(crate) type ResponseBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

struct ProxyHandle {
    port: u16,
    shutdown: tokio::sync::watch::Sender<bool>,
    outbound: Arc<RwLock<Client>>,
}

static HANDLE: OnceLock<Mutex<Option<ProxyHandle>>> = OnceLock::new();
static START_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn handle_slot() -> &'static Mutex<Option<ProxyHandle>> {
    HANDLE.get_or_init(|| Mutex::new(None))
}

/// The router is a process-wide singleton; tests that start or stop it must not
/// run at the same time as each other.
#[cfg(test)]
pub(crate) fn global_router_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn start_lock() -> &'static tokio::sync::Mutex<()> {
    START_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn is_running() -> bool {
    handle_slot()
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

pub fn running_port() -> Option<u16> {
    handle_slot()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|handle| handle.port))
}

pub fn stop() {
    if let Ok(mut guard) = handle_slot().lock() {
        if let Some(handle) = guard.take() {
            let _ = handle.shutdown.send(true);
        }
    }
}

/// `stop()` only signals the accept loop, which drops its listener a moment later.
/// Retry briefly so an immediate restart on the same port is not reported as busy;
/// a port held by another process still fails once the window passes.
async fn bind_localhost(port: u16) -> Result<tokio::net::TcpListener, String> {
    const ATTEMPTS: u32 = 20;
    for attempt in 1..=ATTEMPTS {
        match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                if attempt == ATTEMPTS || port == 0 {
                    return Err("newapi.errors.portBusy".to_owned());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(_) => return Err("newapi.errors.network".to_owned()),
        }
    }
    unreachable!("the loop returns on its last attempt")
}

/// Replace the global outbound client used by an already-running router.
/// Provider routes configured as `direct` keep using their dedicated direct client.
pub fn update_proxy(proxy: &ApiProxyConfigPayload) -> Result<(), String> {
    let client = outbound_client(proxy)?;
    let outbound = handle_slot()
        .lock()
        .map_err(|_| "newapi.errors.storage")?
        .as_ref()
        .map(|handle| handle.outbound.clone());
    if let Some(outbound) = outbound {
        *outbound.write().map_err(|_| "newapi.errors.storage")? = client;
    }
    Ok(())
}

/// Bind the router on localhost. Returns the actually bound port.
/// Called from async commands and app setup.
pub async fn ensure_running(
    port: u16,
    paths: CodexPaths,
    proxy: ApiProxyConfigPayload,
) -> Result<u16, String> {
    let _start_guard = start_lock().lock().await;
    let next_outbound = outbound_client(&proxy)?;
    {
        let guard = handle_slot().lock().map_err(|_| "newapi.errors.storage")?;
        if let Some(handle) = guard.as_ref() {
            if handle.port == port {
                *handle
                    .outbound
                    .write()
                    .map_err(|_| "newapi.errors.storage")? = next_outbound;
                return Ok(handle.port);
            }
        }
    }
    stop();
    let listener = bind_localhost(port).await?;
    let bound_port = listener
        .local_addr()
        .map_err(|_| "newapi.errors.network".to_owned())?
        .port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let outbound = Arc::new(RwLock::new(next_outbound));
    let context = Arc::new(ArcContext {
        store_path: paths.app_data_dir.join("newapi-providers.dat"),
        app_data_dir: paths.app_data_dir.clone(),
        codex_home: paths.codex_home.clone(),
        auth_path: paths.auth_path.clone(),
        outbound: outbound.clone(),
        outbound_direct: outbound_client(&ApiProxyConfigPayload {
            mode: super::models::ApiProxyMode::Direct,
            url: None,
        })?,
    });
    tokio::spawn(accept_loop(listener, shutdown_rx, context));
    *handle_slot().lock().map_err(|_| "newapi.errors.storage")? = Some(ProxyHandle {
        port: bound_port,
        shutdown: shutdown_tx,
        outbound,
    });
    Ok(bound_port)
}

struct ArcContext {
    store_path: PathBuf,
    app_data_dir: PathBuf,
    codex_home: PathBuf,
    auth_path: PathBuf,
    outbound: Arc<RwLock<Client>>,
    /// 供应商网络模式为 direct 时使用的无代理客户端。
    outbound_direct: Client,
}

async fn accept_loop(
    listener: tokio::net::TcpListener,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    context: Arc<ArcContext>,
) {
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
    let builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // Idle timeout per read (resets on every chunk), so long SSE streams keep working
        // while a hung upstream no longer pins the connection forever. Generous because
        // reasoning models can stay silent for minutes before the first token.
        .read_timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none());
    let builder = super::api_client::configure_async_client(builder, proxy)
        .map_err(|_| "newapi.errors.proxy")?;
    builder.build().map_err(|_| "newapi.errors.network".into())
}

fn full_body(text: String) -> ResponseBody {
    Full::new(Bytes::from(text))
        .map_err(|never| match never {})
        .boxed()
}

fn json_response(status: StatusCode, message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(full_body(
            json!({"error": {"message": message, "type": "agentharbor_local_router"}}).to_string(),
        ))
        .expect("static response parts")
}

async fn forward(
    request: Request<Incoming>,
    context: Arc<ArcContext>,
) -> Result<Response<ResponseBody>, std::convert::Infallible> {
    Ok(handle(request, context)
        .await
        .unwrap_or_else(|(status, message)| json_response(status, message)))
}

type ForwardResult<T = Response<ResponseBody>> = Result<T, (StatusCode, &'static str)>;

fn is_unsafe_path_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    let decoded = lower.replace("%2e", ".");
    decoded == "." || decoded == ".." || lower.contains("%2f") || lower.contains("%5c") || segment.contains('\\')
}

/// Claude Code normally strips this itself; the router only handles clients that forward it.
const CLAUDE_ONE_M_MARKER: &str = "[1m]";
const CLAUDE_ONE_M_BETA: &str = "context-1m-2025-08-07";

fn strip_claude_one_m_marker(model: &str) -> (&str, bool) {
    let split = model.len().saturating_sub(CLAUDE_ONE_M_MARKER.len());
    match model.get(split..) {
        Some(tail) if tail.eq_ignore_ascii_case(CLAUDE_ONE_M_MARKER) => (&model[..split], true),
        _ => (model, false),
    }
}

fn append_anthropic_beta(headers: &mut HeaderMap) {
    let mut betas: Vec<String> = headers
        .get_all("anthropic-beta")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    if !betas.iter().any(|value| value == CLAUDE_ONE_M_BETA) {
        betas.push(CLAUDE_ONE_M_BETA.to_owned());
    }
    if let Ok(value) = HeaderValue::from_str(&betas.join(", ")) {
        headers.insert("anthropic-beta", value);
    }
}

enum Destination {
    /// Relay traffic: forward to this provider's base URL with its API key.
    Provider {
        provider: Box<router::Provider>,
        model_override: Option<String>,
        upstream_protocol: router::ModelProtocol,
        /// Set when the caller spoke a different protocol than the provider
        /// serves, so the body and the reply both need translating.
        translation: Option<protocol::Translation>,
    },
    /// Official OpenAI models: forward using the local Codex credentials.
    Official,
}

/// The protocol a destination's request body is being rewritten into.
fn translation_target(destination: &Destination) -> Option<router::ModelProtocol> {
    match destination {
        Destination::Provider {
            translation: Some(translation),
            ..
        } => Some(translation.to),
        _ => None,
    }
}

/// Map a caller-facing endpoint onto the one the target protocol serves.
///
/// Only the protocol's core endpoints are rewritten; sub-paths such as
/// `responses/{id}` keep their tail so follow-up calls still address the right
/// resource.
fn translate_endpoint(suffix: &str, target: router::ModelProtocol) -> String {
    let replacement = match target {
        router::ModelProtocol::Responses => "responses",
        router::ModelProtocol::ChatCompletions => "chat/completions",
        router::ModelProtocol::AnthropicMessages => "messages",
    };
    for source in ["responses", "chat/completions", "messages"] {
        if let Some(tail) = suffix.strip_prefix(source) {
            return format!("{replacement}{tail}");
        }
    }
    suffix.to_owned()
}

fn provider_query(query: Option<&str>, translation: Option<protocol::Translation>) -> String {
    let Some(query) = query else {
        return String::new();
    };
    let query =
        if translation.is_some_and(|pair| pair.from == router::ModelProtocol::AnthropicMessages) {
            // The Anthropic SDK's beta flag does not belong to OpenAI endpoints.
            query
                .split('&')
                .filter(|part| part.split('=').next() != Some("beta"))
                .collect::<Vec<_>>()
                .join("&")
        } else {
            query.to_owned()
        };
    if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    }
}

fn request_protocol(segments: &[&str]) -> Option<router::ModelProtocol> {
    match segments {
        ["codex", "v1", ..] | ["v1", "responses"] | ["p", _, "v1", "responses"] => {
            Some(router::ModelProtocol::Responses)
        }
        ["v1", "chat", "completions"] | ["p", _, "v1", "chat", "completions"] => {
            Some(router::ModelProtocol::ChatCompletions)
        }
        ["v1", "messages", ..] | ["p", _, "v1", "messages", ..] => {
            Some(router::ModelProtocol::AnthropicMessages)
        }
        _ => None,
    }
}

fn provider_destination(
    store: &router::Store,
    provider_id: &str,
    required_protocol: Option<router::ModelProtocol>,
    model_override: Option<String>,
) -> ForwardResult<Destination> {
    let Some(provider) = store
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
    else {
        return Err((StatusCode::NOT_FOUND, "unknown provider"));
    };
    let actual_model = model_override.as_deref().unwrap_or(&provider.model);
    let upstream_protocol = provider.protocol_for_model(actual_model);
    // A protocol mismatch is no longer fatal: the router translates Responses
    // traffic outward, so any provider can back Codex. Only a pair with no
    // translation is rejected, and identical protocols still pass through
    // byte-for-byte.
    let translation = match required_protocol {
        Some(spoken) => match protocol::Translation::between(spoken, upstream_protocol) {
            Some(needed) if !needed.is_supported() => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "provider protocol cannot be translated for this request",
                ))
            }
            other => other,
        },
        None => None,
    };
    if !provider.has_route_credentials() || actual_model.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "provider is not enabled for routing",
        ));
    }
    if !provider.supports_model(actual_model) {
        return Err((
            StatusCode::BAD_REQUEST,
            "model does not support provider protocol",
        ));
    }
    Ok(Destination::Provider {
        provider: Box::new(provider),
        model_override,
        upstream_protocol,
        translation,
    })
}

async fn handle(request: Request<Incoming>, context: Arc<ArcContext>) -> ForwardResult {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    if path == "/agentharbor/health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(full_body("ok".to_owned()))
            .expect("static body"));
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // The suffix is appended to upstream base URLs, which normalize dot segments;
    // reject them so a request cannot climb out of the `/v1` prefix with real credentials.
    if segments.iter().any(|segment| is_unsafe_path_segment(segment)) {
        return Err((StatusCode::BAD_REQUEST, "invalid path"));
    }
    let store = router::read_store(&context.store_path).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "router storage unavailable",
        )
    })?;
    if !authorized(request.headers(), &store.router.bearer_token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid local router token"));
    }

    let method = request.method().clone();
    let has_body = !matches!(method, hyper::Method::GET | hyper::Method::HEAD);
    let codex_auth = codex_request_auth(request.headers(), &store.router.bearer_token);
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BODY as u64)
    {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large"));
    }
    let headers = forwardable_headers(request.headers());
    let body: Option<Bytes> = if has_body {
        Some(read_limited_body(request.into_body()).await?)
    } else {
        read_limited_body(request.into_body()).await?;
        None
    };

    let required_protocol = request_protocol(&segments);
    let (requested_model, one_m_context) = requested_model(body.as_deref(), required_protocol);

    // Codex model catalog for the model picker (`model_catalog_json` mirror).
    if matches!(segments.as_slice(), ["codex", "v1", "models"]) && !has_body {
        return Ok(codex_catalog_response(&context, &store));
    }
    let destination = resolve_destination(
        &store,
        &segments,
        required_protocol,
        requested_model.as_deref(),
    )?;
    if one_m_context
        && !matches!(
            &destination,
            Destination::Provider {
                upstream_protocol: router::ModelProtocol::AnthropicMessages,
                ..
            }
        )
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "1M context requires an Anthropic Messages provider",
        ));
    }
    let suffix = upstream_suffix(&segments, &destination, required_protocol)?;
    let relay_request = relay_usage_request(
        &context,
        &destination,
        required_protocol,
        requested_model.as_deref(),
    );
    let (url, mut headers) = match &destination {
        Destination::Provider {
            provider,
            upstream_protocol,
            translation,
            ..
        } => provider_target(
            provider,
            *upstream_protocol,
            *translation,
            &suffix,
            query.as_deref(),
            headers,
        )?,
        Destination::Official => {
            official_target(&context, codex_auth, &suffix, query.as_deref(), headers)?
        }
    };
    if one_m_context {
        append_anthropic_beta(&mut headers);
    }
    let (body, translated_input) = upstream_body(&destination, body)?;

    let mut outbound = client_for(&context, &destination)?
        .request(method, url)
        .headers(headers);
    if let Some(body) = body {
        outbound = outbound.body(body);
    }
    let response = match outbound.send().await {
        Ok(response) => response,
        Err(_) => {
            if let Some(request) = relay_request {
                request.failed(StatusCode::BAD_GATEWAY.as_u16());
            }
            return Err((StatusCode::BAD_GATEWAY, "cannot reach the provider"));
        }
    };
    let translation = match &destination {
        Destination::Provider { translation, .. } => *translation,
        Destination::Official => None,
    };
    relay_response(response, translation, relay_request, translated_input).await
}

/// Inbound headers minus hop-by-hop, framing, and every local or client credential;
/// the upstream's own credential is added per destination.
fn forwardable_headers(inbound: &HeaderMap) -> HeaderMap {
    let mut headers = inbound.clone();
    remove_hop_by_hop_headers(&mut headers);
    for header in [
        AUTHORIZATION,
        ACCEPT_ENCODING,
        CONTENT_LENGTH,
        EXPECT,
        HOST,
        COOKIE,
    ] {
        headers.remove(header);
    }
    for header in ["x-api-key", "chatgpt-account-id", router::LOCAL_AUTH_HEADER] {
        headers.remove(header);
    }
    headers
}

/// The body's `model`, with Claude's `[1m]` marker stripped for Messages callers,
/// and whether that marker was present.
fn requested_model(
    body: Option<&[u8]>,
    required_protocol: Option<router::ModelProtocol>,
) -> (Option<String>, bool) {
    let raw = body
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    raw.map_or((None, false), |model| {
        if required_protocol == Some(router::ModelProtocol::AnthropicMessages) {
            let (base, enabled) = strip_claude_one_m_marker(&model);
            (Some(base.to_owned()), enabled)
        } else {
            (Some(model), false)
        }
    })
}

fn codex_catalog_response(context: &ArcContext, store: &router::Store) -> Response<ResponseBody> {
    let catalog = router::catalog_value(&context.app_data_dir, &context.codex_home, store);
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
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(full_body(
            json!({"object": "list", "data": entries}).to_string(),
        ))
        .expect("static response parts")
}

/// Picks the upstream for a request path and its `model`.
fn resolve_destination(
    store: &router::Store,
    segments: &[&str],
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
) -> ForwardResult<Destination> {
    let prefixed_model = requested_model.and_then(|model| model.split_once("::"));
    Ok(match segments {
        // Codex protocol: model slug `<provider id>::<model>` decides the route.
        ["codex", "v1", ..] => match prefixed_model {
            Some((provider_id, model_id)) => provider_destination(
                store,
                provider_id,
                Some(router::ModelProtocol::Responses),
                Some(model_id.to_owned()),
            )?,
            // Official model slugs (and anything unprefixed) go to OpenAI with local credentials.
            _ => Destination::Official,
        },
        // Legacy direct routes.
        ["p", id, "v1", ..] => {
            let model_override = match prefixed_model {
                Some((provider_id, model_id)) if provider_id == *id => Some(model_id.to_owned()),
                Some(_) => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "model belongs to a different provider",
                    ))
                }
                None => requested_model.map(str::to_owned),
            };
            provider_destination(store, id, required_protocol, model_override)?
        }
        ["v1", ..] => {
            if let Some((provider_id, model_id)) = prefixed_model {
                provider_destination(
                    store,
                    provider_id,
                    required_protocol,
                    Some(model_id.to_owned()),
                )?
            } else {
                // The explicit model may override the pinned provider's default
                // protocol. Resolve it before falling back to another provider.
                let pinned = requested_model.and_then(|model| {
                    store.providers.iter().find(|provider| {
                        store.active_provider_id.as_deref() == Some(provider.id.as_str())
                            && provider.can_route_model(model)
                            && required_protocol.map_or(true, |from| {
                                protocol::Translation::can_serve(
                                    from,
                                    provider.protocol_for_model(model),
                                )
                            })
                    })
                });
                let provider = pinned.or_else(|| match required_protocol {
                    Some(protocol) => router::effective_active_for(store, protocol),
                    None => router::effective_active(store),
                });
                match provider {
                    Some(provider) => provider_destination(
                        store,
                        &provider.id,
                        required_protocol,
                        requested_model.map(str::to_owned),
                    )?,
                    None => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            "no enabled provider for this protocol",
                        ))
                    }
                }
            }
        }
        _ => return Err((StatusCode::NOT_FOUND, "unsupported path")),
    })
}

/// The path below the upstream's `/v1/`, rewritten for a translated protocol.
fn upstream_suffix(
    segments: &[&str],
    destination: &Destination,
    required_protocol: Option<router::ModelProtocol>,
) -> ForwardResult<String> {
    let suffix = match segments {
        ["p", _, "v1", rest @ ..] | ["codex", "v1", rest @ ..] | ["v1", rest @ ..] => {
            rest.join("/")
        }
        _ => String::new(),
    };
    // A translated request must reach the endpoint the provider's protocol
    // actually serves: `responses` exists only on a Responses provider, so the
    // path has to follow the protocol that the body was rewritten into.
    if required_protocol == Some(router::ModelProtocol::AnthropicMessages)
        && translation_target(destination).is_some()
        && suffix != "messages"
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "this endpoint requires a native Anthropic Messages provider",
        ));
    }
    // Only the core create endpoint is translated. Sub-resources such as
    // `responses/compact` or `responses/{id}` have no equivalent on another
    // protocol, and rewriting them would produce an endpoint that does not exist.
    if required_protocol == Some(router::ModelProtocol::Responses)
        && translation_target(destination).is_some()
        && suffix != "responses"
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "this endpoint requires a native Responses provider",
        ));
    }
    Ok(match translation_target(destination) {
        Some(protocol) => translate_endpoint(&suffix, protocol),
        None => suffix,
    })
}

fn relay_usage_request(
    context: &ArcContext,
    destination: &Destination,
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
) -> Option<relay_usage::RelayUsageRequest> {
    match (destination, required_protocol) {
        (
            Destination::Provider {
                provider,
                model_override,
                upstream_protocol,
                ..
            },
            Some(_),
        ) => Some(relay_usage::RelayUsageRequest::new(
            context.app_data_dir.clone(),
            provider.id.clone(),
            provider.name.clone(),
            model_override
                .clone()
                .or_else(|| requested_model.map(str::to_owned))
                .unwrap_or_else(|| provider.model.clone()),
            match upstream_protocol {
                router::ModelProtocol::Responses => "responses",
                router::ModelProtocol::ChatCompletions => "chatCompletions",
                router::ModelProtocol::AnthropicMessages => "anthropicMessages",
            }
            .to_owned(),
        )),
        _ => None,
    }
}

/// URL and headers for a relay provider, carrying its own API key.
fn provider_target(
    provider: &router::Provider,
    upstream_protocol: router::ModelProtocol,
    translation: Option<protocol::Translation>,
    suffix: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> ForwardResult<(String, HeaderMap)> {
    let endpoint = router::ModelEndpoint {
        base_url: &provider.base_url,
        protocol: upstream_protocol,
    };
    let credential = provider.model_credential();
    match endpoint.protocol {
        router::ModelProtocol::Responses | router::ModelProtocol::ChatCompletions => {
            headers.remove("anthropic-version");
            headers.remove("anthropic-beta");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", credential.api_key))
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid provider key"))?,
            );
        }
        router::ModelProtocol::AnthropicMessages => {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(credential.api_key)
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid provider key"))?,
            );
            if !headers.contains_key("anthropic-version") {
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
        }
    }
    Ok((
        format!(
            "{}v1/{}{}",
            endpoint.base_url,
            suffix,
            provider_query(query, translation)
        ),
        headers,
    ))
}

/// URL and headers for official OpenAI models, using Codex's own credentials.
fn official_target(
    context: &ArcContext,
    codex_auth: Option<(HeaderValue, Option<HeaderValue>)>,
    suffix: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> ForwardResult<(String, HeaderMap)> {
    let query = query.map(|q| format!("?{q}")).unwrap_or_default();
    if let Some((authorization, account_id)) = codex_auth {
        // Codex may have refreshed its token in memory or use a keyring.
        // Its current credentials take precedence over a stale auth.json.
        headers.insert(AUTHORIZATION, authorization);
        let is_chatgpt = account_id.is_some()
            || bearer_matches_chatgpt_auth(headers.get(AUTHORIZATION).unwrap(), &context.auth_path);
        if let Some(account_id) = account_id {
            headers.insert("chatgpt-account-id", account_id);
        }
        let base = if is_chatgpt {
            CHATGPT_CODEX_BASE
        } else {
            OPENAI_API_BASE
        };
        return Ok((format!("{base}{suffix}{query}"), headers));
    }
    let auth = crate::core::auth::load_auth_file(&context.auth_path).ok();
    let access_token = auth
        .as_ref()
        .and_then(|a| a.tokens.access_token.as_deref())
        .map(str::to_owned)
        .filter(|t| !t.trim().is_empty());
    if let Some(token) = access_token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid token"))?,
        );
        if let Some(account_id) = auth
            .as_ref()
            .and_then(|a| a.tokens.account_id.as_deref())
            .filter(|s| !s.is_empty())
        {
            if let Ok(value) = HeaderValue::from_str(account_id) {
                headers.insert("chatgpt-account-id", value);
            }
        }
        Ok((format!("{CHATGPT_CODEX_BASE}{suffix}{query}"), headers))
    } else if let Some(key) = auth
        .and_then(|a| a.openai_api_key)
        .filter(|k| !k.trim().is_empty())
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid key"))?,
        );
        Ok((format!("{OPENAI_API_BASE}{suffix}{query}"), headers))
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "no OpenAI credentials found for official models",
        ))
    }
}

/// The body to send upstream, and for a translated request the canonical
/// Responses input (excluding request-scoped instructions) to remember afterwards.
fn upstream_body(
    destination: &Destination,
    body: Option<Bytes>,
) -> ForwardResult<(Option<Bytes>, Vec<Value>)> {
    let (
        Destination::Provider {
            model_override,
            upstream_protocol,
            translation,
            ..
        },
        Some(bytes),
    ) = (destination, &body)
    else {
        return Ok((body, Vec::new()));
    };
    match translation {
        // A translated request is rebuilt for the provider's protocol,
        // with the slug prefix replaced by the provider's real model.
        Some(translation) => {
            let mut value: Value = serde_json::from_slice(bytes)
                .map_err(|_| (StatusCode::BAD_REQUEST, "request body is not JSON"))?;
            if let Some(model) = model_override.as_deref() {
                value["model"] = json!(model);
            }
            let rewritten = protocol::request_body(*translation, &value).map_err(|error| {
                (
                    StatusCode::BAD_REQUEST,
                    if error == "protocol.errors.unknownPreviousResponse" {
                        "previous_response_id is unknown or expired; resend the full conversation"
                    } else {
                        "cannot translate request"
                    },
                )
            })?;
            let encoded = serde_json::to_vec(&rewritten.body).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cannot re-encode request",
                )
            })?;
            Ok((Some(Bytes::from(encoded)), rewritten.input))
        }
        None if model_override.is_some()
            || *upstream_protocol == router::ModelProtocol::ChatCompletions =>
        {
            let prepared =
                prepare_relay_body(bytes, model_override.as_deref(), *upstream_protocol)?;
            Ok((Some(prepared), Vec::new()))
        }
        None => Ok((body, Vec::new())),
    }
}

/// Direct-mode providers bypass the configured proxy; everything else uses it.
fn client_for(context: &ArcContext, destination: &Destination) -> ForwardResult<Client> {
    Ok(match destination {
        Destination::Provider { provider, .. } if provider.network_mode() == "direct" => {
            context.outbound_direct.clone()
        }
        _ => context
            .outbound
            .read()
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "router network state unavailable",
                )
            })?
            .clone(),
    })
}

/// Streams the upstream response back, translating it into the caller's protocol
/// when needed and recording relay usage along the way.
async fn relay_response(
    response: reqwest::Response,
    translation: Option<protocol::Translation>,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    translated_input: Vec<Value>,
) -> ForwardResult {
    let response_status = response.status();
    let is_event_stream = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    let mut response_headers = response.headers().clone();
    remove_hop_by_hop_headers(&mut response_headers);
    response_headers.remove(CONTENT_LENGTH);

    // A translated stream is rebuilt as Responses events, so its length and any
    // upstream content type no longer describe the bytes being sent.
    let mut builder = Response::builder().status(response_status);
    for (name, value) in &response_headers {
        if translation.is_some() && name == CONTENT_TYPE {
            continue;
        }
        builder = builder.header(name, value);
    }
    if translation.is_some() {
        // A translated stream is rebuilt as Responses events; a translated
        // single body (including an error) is JSON in the caller's shape.
        builder = builder.header(
            CONTENT_TYPE,
            if is_event_stream && response_status.is_success() {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
    }

    let response_body = match translation {
        Some(translation) if response_status.is_success() && is_event_stream => {
            translated_stream(response, translation, relay_request, translated_input)
        }
        Some(translation) if response_status.is_success() => {
            let bytes = response
                .bytes()
                .await
                .map_err(|_| (StatusCode::BAD_GATEWAY, "cannot read provider response"))?;
            if let Some(request) = relay_request {
                request
                    .observe(response_status.as_u16(), false)
                    .observe(&bytes);
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid provider response"))?;
            let converted = protocol::response_body(translation, &value).map_err(|_| {
                (
                    StatusCode::BAD_GATEWAY,
                    "cannot translate provider response",
                )
            })?;
            if translation.from == router::ModelProtocol::Responses {
                protocol::history::remember(&converted, translated_input);
            }
            BodyExt::boxed(
                Full::new(Bytes::from(converted.to_string())).map_err(|never| match never {}),
            )
        }
        Some(translation) => {
            // An upstream error must reach the caller in the caller's own error shape,
            // otherwise SDKs report a parse failure instead of the real message.
            let bytes = response.bytes().await.unwrap_or_default();
            if let Some(request) = relay_request {
                request.observe(response_status.as_u16(), false).observe(&bytes);
            }
            let body = translated_error_body(translation.from, response_status, &bytes);
            BodyExt::boxed(Full::new(Bytes::from(body)).map_err(|never| match never {}))
        }
        None => {
            let mut observer =
                relay_request.map(|request| request.observe(response_status.as_u16(), is_event_stream));
            let stream = response.bytes_stream().map(move |chunk| {
                if let (Some(observer), Ok(bytes)) = (observer.as_mut(), &chunk) {
                    observer.observe(bytes);
                }
                chunk
                    .map(Frame::data)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
            });
            BodyExt::boxed(StreamBody::new(stream))
        }
    };
    builder
        .body(response_body)
        .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid upstream response"))
}

/// Translate an upstream event stream frame by frame; the translator emits a
/// terminal event if the upstream stops without one, so Codex never waits forever.
fn translated_stream(
    response: reqwest::Response,
    translation: protocol::Translation,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    translated_input: Vec<Value>,
) -> ResponseBody {
    let mut observer =
        relay_request.map(|request| request.observe(response.status().as_u16(), true));
    // The turn is recorded once it completes, so the next follow-up that
    // quotes this response id can be rebuilt from it.
    let mut recorded = Some(translated_input);
    // Shared so the same translator can still emit its closing events after
    // the body stream itself has been moved into the response.
    let translator = Arc::new(Mutex::new(protocol::stream_translator(translation)));
    let body_translator = Arc::clone(&translator);
    let stream = response.bytes_stream().flat_map(move |chunk| {
        let mut frames: Vec<Result<Frame<Bytes>, Box<dyn std::error::Error + Send + Sync>>> =
            Vec::new();
        match chunk {
            Ok(bytes) => {
                if let Some(observer) = observer.as_mut() {
                    observer.observe(&bytes);
                }
                if let Ok(mut translator) = body_translator.lock() {
                    let events = translator.push(&bytes);
                    // Record before exposing response.completed: a follow-up
                    // can arrive before the upstream socket reaches EOF.
                    if let Some(response) = translator.completed_response() {
                        if let Some(input) = recorded.take() {
                            protocol::history::remember(response, input);
                        }
                    }
                    for event in events {
                        frames.push(Ok(Frame::data(Bytes::from(event))));
                    }
                }
            }
            Err(_) => {
                if let Ok(mut translator) = body_translator.lock() {
                    for event in translator.fail("upstream_stream_error") {
                        frames.push(Ok(Frame::data(Bytes::from(event))));
                    }
                }
            }
        }
        futures_util::stream::iter(frames)
    });
    // The closing events are emitted once, after the upstream stream ends.
    let tail = futures_util::stream::once(async move {
        let events = match translator.lock() {
            Ok(mut translator) => translator.finish(),
            Err(_) => Vec::new(),
        };
        futures_util::stream::iter(
            events
                .into_iter()
                .map(|event| Ok(Frame::data(Bytes::from(event)))),
        )
    })
    .flatten();
    BodyExt::boxed(StreamBody::new(stream.chain(tail)))
}

/// Re-shape an upstream error body into the caller's protocol.
fn translated_error_body(
    caller: router::ModelProtocol,
    status: StatusCode,
    bytes: &[u8],
) -> String {
    let value: Option<Value> = serde_json::from_slice(bytes).ok();
    let error = value.as_ref().and_then(|value| value.get("error"));
    let message = error
        .and_then(|error| error.get("message").or(Some(error)).and_then(Value::as_str))
        .or_else(|| value.as_ref().and_then(|value| value.get("message")).and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text.is_empty() {
                status.canonical_reason().unwrap_or("upstream error").to_owned()
            } else {
                text.chars().take(2000).collect()
            }
        });
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| match status.as_u16() {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            429 => "rate_limit_error",
            529 => "overloaded_error",
            _ => "api_error",
        }
        .to_owned());
    match caller {
        router::ModelProtocol::AnthropicMessages => {
            json!({"type": "error", "error": {"type": kind, "message": message}})
        }
        _ => json!({"error": {"message": message, "type": kind, "code": status.as_u16()}}),
    }
    .to_string()
}

fn bearer_matches_chatgpt_auth(authorization: &HeaderValue, auth_path: &std::path::Path) -> bool {
    crate::core::auth::load_auth_file(auth_path)
        .ok()
        .and_then(|auth| auth.tokens.access_token)
        .filter(|token| !token.trim().is_empty())
        .is_some_and(|token| {
            authorization.to_str().ok() == Some(format!("Bearer {token}").as_str())
        })
}

fn codex_request_auth(
    headers: &HeaderMap,
    local_token: &str,
) -> Option<(HeaderValue, Option<HeaderValue>)> {
    if local_token.is_empty()
        || headers.get(router::LOCAL_AUTH_HEADER)?.to_str().ok()? != local_token
    {
        return None;
    }
    let authorization = headers.get(AUTHORIZATION)?;
    let token = authorization.to_str().ok()?.strip_prefix("Bearer ")?;
    if token.trim().is_empty() || token == local_token {
        return None;
    }
    let account_id = headers
        .get("chatgpt-account-id")
        .filter(|value| value.to_str().ok().is_some_and(|id| !id.trim().is_empty()))
        .cloned();
    Some((authorization.clone(), account_id))
}

fn authorized(headers: &HeaderMap, bearer_token: &str) -> bool {
    if bearer_token.is_empty() {
        return false;
    }
    // A supplied router header must be valid; never let an OpenAI bearer
    // credential substitute for authentication to the local server.
    if let Some(token) = headers.get(router::LOCAL_AUTH_HEADER) {
        return token.to_str().ok() == Some(bearer_token);
    }
    let Ok(expected) = HeaderValue::from_str(&format!("Bearer {bearer_token}")) else {
        return false;
    };
    headers.get(AUTHORIZATION) == Some(&expected)
        || headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            == Some(bearer_token)
}

fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    let nominated: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        CONNECTION,
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
    ] {
        headers.remove(name);
    }
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

fn prepare_relay_body(
    body: &Bytes,
    model_id: Option<&str>,
    protocol: router::ModelProtocol,
) -> Result<Bytes, (StatusCode, &'static str)> {
    let mut value: Value = serde_json::from_slice(body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "request body is not JSON"))?;
    let object = value
        .as_object_mut()
        .ok_or((StatusCode::BAD_REQUEST, "request body is not an object"))?;
    if let Some(model_id) = model_id {
        object.insert("model".to_owned(), json!(model_id));
    }
    if protocol == router::ModelProtocol::ChatCompletions
        && object.get("stream") == Some(&Value::Bool(true))
    {
        let options = object.entry("stream_options").or_insert_with(|| json!({}));
        if options.is_null() {
            *options = json!({});
        }
        options
            .as_object_mut()
            .ok_or((StatusCode::BAD_REQUEST, "stream_options is not an object"))?
            .insert("include_usage".into(), json!(true));
    }
    let bytes = serde_json::to_vec(&value).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot re-encode request",
        )
    })?;
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::router::{Provider, ProviderInput, RouterSettings, Store};
    use crate::platform::paths::CodexPaths;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn upstream_errors_are_reshaped_for_the_caller() {
        let openai = br#"{"error":{"message":"bad model","type":"invalid_request_error","code":"model_not_found"}}"#;
        let anthropic: Value = serde_json::from_str(&translated_error_body(
            router::ModelProtocol::AnthropicMessages,
            StatusCode::BAD_REQUEST,
            openai,
        ))
        .unwrap();
        assert_eq!(anthropic["type"], "error");
        assert_eq!(anthropic["error"]["type"], "invalid_request_error");
        assert_eq!(anthropic["error"]["message"], "bad model");

        let claude = br#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#;
        let responses: Value = serde_json::from_str(&translated_error_body(
            router::ModelProtocol::Responses,
            StatusCode::from_u16(529).unwrap(),
            claude,
        ))
        .unwrap();
        assert_eq!(responses["error"]["message"], "busy");
        assert_eq!(responses["error"]["type"], "overloaded_error");

        let html: Value = serde_json::from_str(&translated_error_body(
            router::ModelProtocol::Responses,
            StatusCode::BAD_GATEWAY,
            b"<html>Bad Gateway</html>",
        ))
        .unwrap();
        assert_eq!(html["error"]["message"], "<html>Bad Gateway</html>");
        assert_eq!(html["error"]["type"], "api_error");
    }

    #[test]
    fn bind_waits_for_a_released_port_but_reports_a_held_one() {
        tauri::async_runtime::block_on(async {
            let held = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let port = held.local_addr().unwrap().port();
            assert_eq!(bind_localhost(port).await.unwrap_err(), "newapi.errors.portBusy");

            let release = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                drop(held);
            });
            let rebound = bind_localhost(port).await.unwrap();
            assert_eq!(rebound.local_addr().unwrap().port(), port);
            release.await.unwrap();
        });
    }

    #[test]
    fn dot_and_encoded_separator_segments_are_rejected() {
        for segment in ["..", ".", "%2e%2E", ".%2e", "a%2Fb", "a%5cb", "a\\b"] {
            assert!(is_unsafe_path_segment(segment), "{segment}");
        }
        for segment in ["v1", "responses", "chat", "claude-opus-4.8", "..x"] {
            assert!(!is_unsafe_path_segment(segment), "{segment}");
        }
    }

    #[test]
    fn one_m_marker_is_stripped_only_at_the_end_and_beta_is_preserved() {
        assert_eq!(strip_claude_one_m_marker("model[1M]"), ("model", true));
        assert_eq!(strip_claude_one_m_marker("model[1m]"), ("model", true));
        assert_eq!(strip_claude_one_m_marker("模型"), ("模型", false));
        assert_eq!(
            strip_claude_one_m_marker("model[1M]suffix"),
            ("model[1M]suffix", false)
        );
        let mut headers = HeaderMap::new();
        headers.insert("anthropic-beta", HeaderValue::from_static("existing-beta"));
        append_anthropic_beta(&mut headers);
        append_anthropic_beta(&mut headers);
        assert_eq!(
            headers["anthropic-beta"],
            "existing-beta, context-1m-2025-08-07"
        );
    }

    #[test]
    fn codex_auth_requires_a_local_token_and_preserves_current_credentials() {
        let fixture = tempfile::tempdir().unwrap();
        let auth_path = fixture.path().join("auth.json");
        std::fs::write(
            &auth_path,
            br#"{"tokens":{"access_token":"current-chatgpt-token"}}"#,
        )
        .unwrap();
        assert!(bearer_matches_chatgpt_auth(
            &HeaderValue::from_static("Bearer current-chatgpt-token"),
            &auth_path
        ));
        assert!(!bearer_matches_chatgpt_auth(
            &HeaderValue::from_static("Bearer sk-current-api-key"),
            &auth_path
        ));
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer current-chatgpt-token"),
        );
        headers.insert(
            "chatgpt-account-id",
            HeaderValue::from_static("current-account"),
        );
        assert!(!authorized(&headers, "local-secret"));
        assert!(codex_request_auth(&headers, "local-secret").is_none());
        headers.insert(
            router::LOCAL_AUTH_HEADER,
            HeaderValue::from_static("local-secret"),
        );
        assert!(authorized(&headers, "local-secret"));
        let (authorization, account) = codex_request_auth(&headers, "local-secret").unwrap();
        assert_eq!(authorization, "Bearer current-chatgpt-token");
        assert_eq!(account.unwrap(), "current-account");
        headers.remove("chatgpt-account-id");
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-current-api-key"),
        );
        assert!(codex_request_auth(&headers, "local-secret")
            .unwrap()
            .1
            .is_none());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer local-secret"),
        );
        assert!(
            codex_request_auth(&headers, "local-secret").is_none(),
            "never forward the local secret as an OpenAI credential"
        );
        headers.insert(router::LOCAL_AUTH_HEADER, HeaderValue::from_static("wrong"));
        assert!(!authorized(&headers, "local-secret"));
    }

    #[test]
    fn hop_by_hop_headers_include_connection_nominations() {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive, x-remove"));
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert("x-remove", HeaderValue::from_static("secret"));
        headers.insert("x-keep", HeaderValue::from_static("ok"));

        remove_hop_by_hop_headers(&mut headers);

        assert!(!headers.contains_key(CONNECTION));
        assert!(!headers.contains_key("keep-alive"));
        assert!(!headers.contains_key("x-remove"));
        assert_eq!(headers.get("x-keep"), Some(&HeaderValue::from_static("ok")));
    }

    #[test]
    fn fallback_provider_keeps_the_required_translation() {
        let store = Store {
            providers: vec![Provider {
                id: "chat-only".into(),
                api_key: "sk-test".into(),
                model: "chat-model".into(),
                enabled: true,
                protocol: router::ModelProtocol::ChatCompletions,
                ..Default::default()
            }],
            ..Default::default()
        };
        let provider =
            router::effective_active_for(&store, router::ModelProtocol::Responses).unwrap();
        let destination = provider_destination(
            &store,
            &provider.id,
            Some(router::ModelProtocol::Responses),
            None,
        )
        .unwrap();
        assert!(matches!(
            destination,
            Destination::Provider {
                translation: Some(protocol::Translation {
                    from: router::ModelProtocol::Responses,
                    to: router::ModelProtocol::ChatCompletions,
                }),
                ..
            }
        ));
        let reverse = protocol::Translation {
            from: router::ModelProtocol::ChatCompletions,
            to: router::ModelProtocol::Responses,
        };
        assert!(!reverse.is_supported());
    }

    #[test]
    fn proxy_serves_the_codex_catalog_and_routes_prefixed_models() {
        let _router = global_router_test_lock();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let upstream_port = upstream.local_addr().unwrap().port();
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = upstream.accept().await else { break };
                    let mut buffer = Vec::new();
                    loop {
                        let mut chunk = [0u8; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert!(count > 0, "upstream request must be complete");
                        buffer.extend_from_slice(&chunk[..count]);
                        if let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
                            let length = headers.lines().find_map(|line| line.strip_prefix("content-length:")).map(|value| value.trim().parse::<usize>().unwrap()).unwrap_or(0);
                            if buffer.len() >= header_end + 4 + length { break; }
                        }
                    }
                    let request = String::from_utf8_lossy(&buffer).to_lowercase();
                    let payload: Value = request.split_once("\r\n\r\n").and_then(|(_, body)| serde_json::from_str(body).ok()).unwrap_or(Value::Null);
                    let model = payload["model"].as_str().unwrap_or("");
                    let expected_key = match model {
                        "chat-model" => "sk-chat",
                        "claude-mock" => "sk-anthropic",
                        _ => "sk-upstream",
                    };
                    if request.contains(" /v1/messages ") {
                        assert!(request.contains(&format!("x-api-key: {expected_key}")));
                        assert!(request.contains("anthropic-version: 2023-06-01"));
                        assert!(!request.contains("authorization:"));
                        if request.contains("x-fixture-claude-one-m: true") {
                            assert_eq!(payload["model"], "claude-mock");
                            assert!(request.contains("anthropic-beta: existing-beta, context-1m-2025-08-07"));
                        }
                    } else if request.contains(" /v1/chat/completions ") {
                        assert!(request.contains(&format!("authorization: bearer {expected_key}")));
                        assert!(!request.contains("x-api-key:"));
                    } else {
                        assert!(request.contains("authorization: bearer sk-upstream"), "upstream must see the provider key");
                    }
                    match model {
                        "mixed-chat" => assert!(request.contains(" /v1/chat/completions ")),
                        "mixed-messages" => assert!(request.contains(" /v1/messages ")),
                        "mock-model" => assert!(request.contains(" /v1/responses ")),
                        _ => {}
                    }
                    assert!(!request.contains("router-test-token"), "the inbound local token must be replaced");
                    assert!(!request.contains("chatgpt-private-token"));
                    assert!(!request.contains("private-account-id"));
                    assert!(!request.contains(router::LOCAL_AUTH_HEADER));
                    if request.contains("x-fixture-claude: true") {
                        assert!(!request.contains("anthropic-version:"));
                        assert!(!request.contains("anthropic-beta:"));
                        assert!(!request.lines().next().unwrap().contains("beta=true"));
                    }
                    assert!(request.contains("/v1/"), "path must be rewritten to the provider root");
                    for prefix in ["agentharbor_fixture", "chat_fixture", "anthropic_fixture"] {
                        assert!(!request.contains(prefix), "the model slug prefix must be stripped");
                    }
                    let streaming = request.contains("\"stream\":true");
                    let (content_type, body) = if request.contains("x-fixture-claude: true") && request.contains(" /v1/responses ") {
                        assert!(payload["input"].is_array());
                        assert_eq!(payload["max_output_tokens"], 32);
                        let response = json!({"object":"response","id":"resp-claude-fixture","status":"completed","model":model,
                            "output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hello from responses"}]}],
                            "usage":{"input_tokens":20,"output_tokens":5,"total_tokens":25}});
                        if streaming {
                            let frames = [
                                json!({"type":"response.created","response":{"model":model}}),
                                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"hello from responses"}),
                                json!({"type":"response.completed","response":response}),
                            ];
                            ("text/event-stream", frames.iter().map(|value| format!("data: {value}\n\n")).collect::<String>())
                        } else { ("application/json", response.to_string()) }
                    } else if streaming {
                        assert!(request.contains("\"include_usage\":true"));
                        ("text/event-stream", "data: {\"model\":\"chat-model\",\"choices\":[{\"delta\":{\"content\":\"hello stream\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":5,\"total_tokens\":25}}\n\ndata: [DONE]\n\n".to_owned())
                    } else if request.contains(" /v1/chat/completions ") {
                        ("application/json", json!({"id": "chatcmpl-1", "model": model, "choices": [{"finish_reason":"stop","message": {"role": "assistant", "content": "hello from chat"}}], "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}}).to_string())
                    } else if request.contains(" /v1/messages ") {
                        ("application/json", json!({"id": "msg_1", "model": model, "content": [{"type": "text", "text": "hello from anthropic"}], "usage": {"input_tokens": 20, "output_tokens": 5}}).to_string())
                    } else {
                        ("application/json", "{\"object\":\"list\",\"n\":1,\"usage\":{\"input_tokens\":20,\"output_tokens\":5,\"total_tokens\":25}}".to_owned())
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }
            });

            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().to_owned());
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            std::fs::write(
                paths.codex_home.join("models_cache.json"),
                serde_json::to_vec(&json!({"models": [{
                    "slug": "gpt-official-fixture",
                    "display_name": "Official Fixture",
                    "context_window": 272000
                }]}))
                .unwrap(),
            )
            .unwrap();
            let provider = Provider {
                id: "agentharbor_fixture".into(),
                name: "Mock".into(),
                base_url: format!("http://127.0.0.1:{upstream_port}/"),
                api_key: "sk-upstream".into(),
                model: "mock-model".into(),
                enabled: true,
                test_status: "ok".into(),
                models: vec![
                    super::super::newapi::Model { id: "mock-model".into(), context_window: Some(128000), ..Default::default() },
                    super::super::newapi::Model { id: "mixed-chat".into(), protocol: Some(router::ModelProtocol::ChatCompletions), endpoint_types: vec!["openai-chat".into()], ..Default::default() },
                    super::super::newapi::Model { id: "mixed-messages".into(), protocol: Some(router::ModelProtocol::AnthropicMessages), endpoint_types: vec!["anthropic-messages".into()], ..Default::default() },
                ],
                ..Default::default()
            };
            let chat_provider = Provider {
                id: "chat_fixture".into(),
                name: "Chat Mock".into(),
                base_url: format!("http://127.0.0.1:{upstream_port}/"),
                api_key: "sk-chat".into(),
                protocol: router::ModelProtocol::ChatCompletions,
                model: "chat-model".into(),
                enabled: true,
                test_status: "ok".into(),
                models: vec![super::super::newapi::Model {
                    id: "chat-model".into(),
                    context_window: None,
                    max_output_tokens: None,
                    endpoint_types: vec!["openai-chat".into()],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let anthropic_provider = Provider {
                id: "anthropic_fixture".into(),
                name: "Anthropic Mock".into(),
                base_url: format!("http://127.0.0.1:{upstream_port}/"),
                api_key: "sk-anthropic".into(),
                protocol: router::ModelProtocol::AnthropicMessages,
                model: "claude-mock".into(),
                enabled: true,
                test_status: "ok".into(),
                models: vec![super::super::newapi::Model {
                    id: "claude-mock".into(),
                    context_window: None,
                    max_output_tokens: None,
                    endpoint_types: vec!["anthropic-messages".into()],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let mut store = Store {
                version: 3,
                codex_access_mode: router::CodexAccessMode::Router,
                providers: vec![provider, chat_provider, anthropic_provider],
                active_provider_id: Some("agentharbor_fixture".into()),
                router: RouterSettings { bearer_token: "router-test-token".into(), ..Default::default() },
            };
            crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &serde_json::to_vec(&store).unwrap()).unwrap();

            let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
            assert!(port > 0);
            let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
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
            let wrong_local_header = client
                .get(format!("{base}/codex/v1/models"))
                .header(router::LOCAL_AUTH_HEADER, "wrong")
                .bearer_auth("router-test-token")
                .send().await.unwrap();
            assert_eq!(wrong_local_header.status(), 401);

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
            assert_eq!(entries.len(), 6, "Codex catalog must merge official and all routable relay models");
            assert!(entries.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str) == Some("gpt-official-fixture")
            }));
            assert!(entries.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str)
                    == Some("agentharbor_fixture::mock-model")
            }));
            assert!(entries.iter().any(|entry| entry["id"] == "chat_fixture::chat-model"));
            assert!(entries.iter().any(|entry| entry["id"] == "anthropic_fixture::claude-mock"));
            assert!(entries.iter().any(|entry| entry["id"] == "agentharbor_fixture::mixed-chat"));
            assert!(entries.iter().any(|entry| entry["id"] == "agentharbor_fixture::mixed-messages"));

            // Codex-style POST with a prefixed model is routed and the prefix stripped.
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .header(router::LOCAL_AUTH_HEADER, "router-test-token")
                .bearer_auth("chatgpt-private-token")
                .header("chatgpt-account-id", "private-account-id")
                .json(&json!({"model": "agentharbor_fixture::mock-model", "input": "hi"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let _ = response.bytes().await.unwrap();

            // Codex must receive Responses JSON even when the provider speaks
            // Chat Completions or Anthropic Messages.
            for (slug, answer) in [
                ("chat_fixture::chat-model", "hello from chat"),
                ("anthropic_fixture::claude-mock", "hello from anthropic"),
                ("agentharbor_fixture::mixed-chat", "hello from chat"),
                ("agentharbor_fixture::mixed-messages", "hello from anthropic"),
            ] {
                let response = client
                    .post(format!("{base}/codex/v1/responses"))
                    .header(AUTHORIZATION, "Bearer router-test-token")
                    .json(&json!({"model": slug, "input": "hi", "stream": false}))
                    .send().await.unwrap();
                assert_eq!(response.status(), 200);
                let response: Value = response.json().await.unwrap();
                assert_eq!(response["object"], "response");
                assert_eq!(response["output"][0]["content"][0]["text"], answer);
            }
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .json(&json!({"model": "chat_fixture::chat-model", "input": "hi", "stream": true}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            let stream = response.text().await.unwrap();
            assert!(stream.contains("response.created"));
            assert!(stream.contains("response.output_text.delta"));
            assert!(stream.contains("hello stream"));
            assert!(stream.contains("response.completed"));

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

            // OpenAI Chat Completions uses Bearer auth and stays in its own wire protocol.
            let response = client
                .post(format!("{base}/v1/chat/completions"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .json(&json!({"model": "chat_fixture::chat-model", "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);

            // Anthropic clients may authenticate to the local router with x-api-key.
            // The proxy replaces it with the provider key and supplies the standard version.
            let response = client
                .post(format!("{base}/v1/messages"))
                .header("x-api-key", "router-test-token")
                .json(&json!({"model": "anthropic_fixture::claude-mock", "max_tokens": 32, "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);

            let response = client
                .post(format!("{base}/v1/messages"))
                .header("x-api-key", "router-test-token")
                .header("anthropic-beta", "existing-beta")
                .header("x-fixture-claude-one-m", "true")
                .json(&json!({"model": "anthropic_fixture::claude-mock[1M]", "max_tokens": 32, "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);

            let response = client
                .post(format!("{base}/v1/chat/completions"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .json(&json!({"model": "agentharbor_fixture::mock-model", "messages": []}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 400);

            // A single provider can expose all three protocols. Plain model IDs
            // on pinned and per-provider routes must retain their own endpoint.
            for (path, model) in [
                ("/p/agentharbor_fixture/v1/chat/completions", "mixed-chat"),
                ("/p/agentharbor_fixture/v1/messages", "mixed-messages"),
                ("/v1/chat/completions", "mixed-chat"),
                ("/v1/messages", "mixed-messages"),
            ] {
                let response = client.post(format!("{base}{path}"))
                    .bearer_auth("router-test-token")
                    .json(&json!({"model": model, "max_tokens": 32, "messages": [{"role": "user", "content": "hi"}]}))
                    .send().await.unwrap();
                assert_eq!(response.status(), 200, "{path}");
                let response: Value = response.json().await.unwrap();
                assert_eq!(response["model"], model);
            }
            let response = client.post(format!("{base}/v1/responses"))
                .bearer_auth("router-test-token")
                .json(&json!({"model": "mixed-chat", "input": "hi", "stream": false}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            let response: Value = response.json().await.unwrap();
            assert_eq!(response["object"], "response");
            assert_eq!(response["output"][0]["content"][0]["text"], "hello from chat");
            let response = client.post(format!("{base}/p/agentharbor_fixture/v1/chat/completions"))
                .bearer_auth("router-test-token")
                .json(&json!({"model": "chat_fixture::chat-model", "messages": []}))
                .send().await.unwrap();
            assert_eq!(response.status(), 400, "path provider cannot be overridden by a model prefix");

            // Both official auth modes use the configured model egress, even
            // with no auth.json (Codex can keep credentials in the keyring).
            // The mock rejects CONNECT so no traffic reaches an external host.
            let official_proxy = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let official_proxy_port = official_proxy.local_addr().unwrap().port();
            let observed = tokio::spawn(async move {
                let mut targets = Vec::new();
                for _ in 0..2 {
                    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), official_proxy.accept()).await.unwrap().unwrap();
                    let mut buffer = [0u8; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    targets.push(String::from_utf8_lossy(&buffer[..count]).lines().next().unwrap().to_owned());
                    socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                }
                targets
            });
            update_proxy(&ApiProxyConfigPayload {
                mode: super::super::models::ApiProxyMode::Manual,
                url: Some(format!("http://127.0.0.1:{official_proxy_port}")),
            }).unwrap();
            for account in [Some("fixture-account"), None] {
                let mut request = client.post(format!("{base}/codex/v1/responses"))
                    .header(router::LOCAL_AUTH_HEADER, "router-test-token")
                    .bearer_auth("current-official-fixture-token")
                    .json(&json!({"model": "gpt-official-fixture", "input": "hi"}));
                if let Some(account) = account { request = request.header("chatgpt-account-id", account); }
                assert_eq!(request.send().await.unwrap().status(), 502);
            }
            let targets = observed.await.unwrap();
            assert!(targets[0].starts_with("CONNECT chatgpt.com:443 "));
            assert!(targets[1].starts_with("CONNECT api.openai.com:443 "));

            // Changing the global proxy updates the running router without rebinding it.
            let unavailable_proxy = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let unavailable_proxy_port = unavailable_proxy.local_addr().unwrap().port();
            drop(unavailable_proxy);
            update_proxy(&ApiProxyConfigPayload {
                mode: super::super::models::ApiProxyMode::Manual,
                url: Some(format!("http://127.0.0.1:{unavailable_proxy_port}")),
            })
            .unwrap();
            assert_eq!(running_port(), Some(port));
            let response = client
                .get(format!("{base}/v1/models"))
                .header(AUTHORIZATION, "Bearer router-test-token")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 502);

            update_proxy(&ApiProxyConfigPayload::default()).unwrap();
            assert_eq!(running_port(), Some(port));
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
            let usage = crate::core::relay_usage::load(&paths.app_data_dir).unwrap();
            assert_eq!(usage.all_time.request_count, 14, "only inference requests are tracked");
            assert!(usage
                .by_provider
                .iter()
                .any(|entry| entry.provider_id == "agentharbor_fixture"));

            // Exercise generated client configs through the real proxy, independently of Codex.
            use crate::core::client_config::{ClientPaths, ConfigManager, ConfigRequest};
            use crate::core::targets::TargetId;
            use crate::core::routing_runtime;
            store.router.port = port;
            crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &serde_json::to_vec(&store).unwrap()).unwrap();
            let client_paths = ClientPaths::from_home(&dir.path().join("clients"));
            let claude_path = client_paths.claude.clone();
            let opencode_path = client_paths.opencode.clone();
            let manager = ConfigManager::new(&paths.app_data_dir, client_paths);
            let mut config_request = ConfigRequest {
                target_id: TargetId::ClaudeCode, provider_id: "anthropic_fixture".into(),
                model: "claude-mock".into(), context_window: 200000, max_output_tokens: 8192,
                options: crate::core::client_config::ClientOptions {
                    access_mode: crate::core::client_config::ClientAccessMode::Router,
                    ..Default::default()
                },
            };
            let preview = manager.preview(&store, &config_request).unwrap();
            manager.apply(&store, &config_request, &preview.revision).unwrap();
            config_request.target_id = TargetId::OpenCode;
            config_request.provider_id = "chat_fixture".into();
            config_request.model = "chat-model".into();
            let preview = manager.preview(&store, &config_request).unwrap();
            manager.apply(&store, &config_request, &preview.revision).unwrap();

            router::set_smart_routing(&paths, true).unwrap();
            router::set_smart_routing(&paths, false).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(is_running(), "disabling Codex must not stop other clients");

            stop();
            tokio::time::sleep(Duration::from_millis(30)).await;
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(is_running(), "client config alone restarts the router");

            let claude: Value = serde_json::from_slice(&std::fs::read(claude_path).unwrap()).unwrap();
            let response = client.post(format!("{}/v1/messages", claude["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap()))
                .bearer_auth(claude["env"]["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap())
                .json(&json!({"model": claude["model"], "max_tokens": 32, "messages": [{"role":"user", "content":"hi"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            response.bytes().await.unwrap();
            manager.restore(TargetId::ClaudeCode).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(is_running(), "OpenCode still depends on the router");

            let opencode: Value = serde_json::from_slice(&std::fs::read(opencode_path).unwrap()).unwrap();
            let options = &opencode["provider"]["agentharbor_chat_fixture"]["options"];
            let response = client.post(format!("{}/chat/completions", options["baseURL"].as_str().unwrap()))
                .bearer_auth(options["apiKey"].as_str().unwrap())
                .json(&json!({"model": "chat-model", "stream": true, "messages": [{"role":"user", "content":"hi"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            let body = response.text().await.unwrap();
            assert!(body.contains("data: [DONE]"), "SSE is forwarded unchanged");
            let usage = crate::core::relay_usage::load(&paths.app_data_dir).unwrap();
            assert_eq!(usage.all_time.request_count, 16);
            assert_eq!(usage.all_time.total_tokens, 400);
            // Claude's separate provider route remains available with Codex disabled.
            for (provider_id, model) in [("chat_fixture", "chat-model"), ("agentharbor_fixture", "mock-model")] {
                for stream in [false, true] {
                    let response = client.post(format!("{base}/p/{provider_id}/v1/messages?beta=true"))
                        .bearer_auth("router-test-token")
                        .header("anthropic-version", "2023-06-01")
                        .header("anthropic-beta", "fixture-beta")
                        .header("x-fixture-claude", "true")
                        .json(&json!({"model":model,"max_tokens":32,"stream":stream,"messages":[{"role":"user","content":"hi"}]}))
                        .send().await.unwrap();
                    assert_eq!(response.status(),200,"{provider_id}, stream={stream}");
                    if stream {
                        let body = response.text().await.unwrap();
                        assert!(body.contains("message_start"));
                        assert!(body.contains("text_delta"));
                        assert!(body.contains("message_stop"));
                        assert!(!body.contains("response.completed"));
                        assert!(!body.contains("event: error"));
                    } else {
                        let body: Value = response.json().await.unwrap();
                        assert_eq!(body["type"],"message");
                        assert_eq!(body["stop_reason"],"end_turn");
                        assert_eq!(body["usage"]["output_tokens"],5);
                    }
                }
            }
            let response = client.post(format!("{base}/p/chat_fixture/v1/messages/count_tokens"))
                .bearer_auth("router-test-token")
                .json(&json!({"model":"chat-model","messages":[{"role":"user","content":"hi"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(),501,"unsupported token counting must not be fabricated");
            let usage = crate::core::relay_usage::load(&paths.app_data_dir).unwrap();
            assert_eq!(usage.all_time.request_count,20);
            assert_eq!(usage.all_time.total_tokens,500);
            manager.restore(TargetId::OpenCode).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(!is_running(), "the router stops only when all clients release it");
            config_request.options.access_mode = crate::core::client_config::ClientAccessMode::Native;
            let preview = manager.preview(&store, &config_request).unwrap();
            manager.apply(&store, &config_request, &preview.revision).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(!is_running(), "a native client config must not restart the router");
            manager.restore(TargetId::OpenCode).unwrap();
            stop();
        });
    }

    #[test]
    #[ignore = "Requires an explicitly provided local encrypted test connection"]
    fn live_smart_routing_injection() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
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
            let provider = initial
                .providers
                .first()
                .cloned()
                .expect("test relay connection");
            let models = {
                let paths = paths.clone();
                let provider_id = provider.id.clone();
                let base_url = provider.base_url.clone();
                let proxy = proxy.clone();
                tokio::task::spawn_blocking(move || {
                    router::fetch_models(
                        &paths,
                        Some(&provider_id),
                        &base_url,
                        "",
                        router::ModelProtocol::Responses,
                        &proxy,
                    )
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
                    protocol: provider.protocol,
                    model,
                    models: tested.models,
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

            let client = Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .unwrap();
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
                .find_map(|entry| {
                    entry["id"]
                        .as_str()
                        .filter(|id| id.starts_with(&provider.id))
                })
                .expect("prefixed relay model")
                .to_owned();
            let response = client
                .post(format!("{base}/codex/v1/responses"))
                .bearer_auth(&store.router.bearer_token)
                .json(&json!({"model": slug, "input": "Reply with exactly OK.", "stream": true}))
                .send()
                .await
                .unwrap();
            assert!(
                response.status().is_success(),
                "relay returned HTTP {}",
                response.status()
            );
            let body = response.text().await.unwrap();
            assert!(
                body.contains("response.completed"),
                "response stream did not complete"
            );
            stop();
        });
    }
}
