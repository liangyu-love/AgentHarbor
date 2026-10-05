use super::models::ApiProxyConfigPayload;
use super::{protocol, relay_usage, request_diagnostics, route_trace, router};
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

#[cfg(test)]
#[path = "router_proxy/claude_cli_tests.rs"]
mod claude_cli_tests;
#[cfg(test)]
mod claude_continuation_tests;
#[cfg(test)]
#[path = "router_proxy/desktop_gateway_tests.rs"]
mod desktop_gateway_tests;
mod failover;
#[cfg(test)]
#[path = "router_proxy/failover_tests.rs"]
mod failover_tests;
mod session_compat;
mod tool_schema;

const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;
const CHATGPT_CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex/";
const OPENAI_API_BASE: &str = "https://api.openai.com/v1/";

pub(crate) type ResponseBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

struct ProxyHandle {
    port: u16,
    lease: ListenerLease,
    shutdown: tokio::sync::watch::Sender<bool>,
    outbound: Arc<RwLock<Client>>,
}

/// A lease identifies one successful bind in this backend process. A port alone
/// cannot distinguish a stopped/restarted listener or an old preview ticket.
pub use super::claude_desktop::types::ListenerLease;

#[derive(Default)]
struct ListenerPins {
    managed: Option<u16>,
    reservations: std::collections::HashMap<String, u16>,
}
static PINS: OnceLock<Mutex<ListenerPins>> = OnceLock::new();
fn listener_pins() -> &'static Mutex<ListenerPins> {
    PINS.get_or_init(Default::default)
}
fn pinned_port(pins: &ListenerPins) -> Option<u16> {
    pins.managed.or_else(|| pins.reservations.values().next().copied())
}
pub fn listener_pinned_port() -> Option<u16> {
    pinned_port(&listener_pins().lock().unwrap_or_else(|e| e.into_inner()))
}
pub fn desktop_listener_required() -> bool {
    listener_pinned_port().is_some()
}
pub fn current_lease() -> Option<ListenerLease> {
    handle_slot().lock().ok().and_then(|slot| slot.as_ref().map(|h| h.lease.clone()))
}
pub fn validate_lease(lease: &ListenerLease) -> bool {
    current_lease().as_ref() == Some(lease)
}
pub fn reserve_listener(lease: &ListenerLease) -> Result<String, String> {
    let mut pins = listener_pins().lock().map_err(|_| "newapi.errors.storage")?;
    if lease.bound_port == 0 || !validate_lease(lease) {
        return Err("claudeDesktop.errors.listenerChanged".into());
    }
    if pinned_port(&pins).is_some_and(|port| port != lease.bound_port) {
        return Err("claudeDesktop.errors.listenerPinned".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    pins.reservations.insert(id.clone(), lease.bound_port);
    Ok(id)
}
pub fn release_listener_reservation(id: &str) {
    listener_pins().lock().unwrap_or_else(|e| e.into_inner()).reservations.remove(id);
}
pub fn set_desktop_pin(port: Option<u16>) -> Result<(), String> {
    let mut pins = listener_pins().lock().map_err(|_| "newapi.errors.storage")?;
    if port == Some(0) || port.is_some_and(|port| pinned_port(&pins).is_some_and(|p| p != port)) {
        return Err("claudeDesktop.errors.listenerPinned".into());
    }
    pins.managed = port;
    Ok(())
}

static HANDLE: OnceLock<Mutex<Option<ProxyHandle>>> = OnceLock::new();
static START_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
type FailoverScope = Option<(PathBuf, Arc<std::sync::atomic::AtomicBool>)>;
static FAILOVER_SCOPE: OnceLock<Mutex<FailoverScope>> = OnceLock::new();

fn failover_seen_for(store_path: &std::path::Path) -> Arc<std::sync::atomic::AtomicBool> {
    let mut scope = FAILOVER_SCOPE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((path, seen)) = &*scope {
        if path == store_path {
            return seen.clone();
        }
    }
    let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
    *scope = Some((store_path.to_owned(), seen.clone()));
    seen
}

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
    let pins = listener_pins().lock().unwrap_or_else(|e| e.into_inner());
    if pinned_port(&pins).is_some() {
        return;
    }
    stop_unchecked();
}

/// Final process shutdown only. Normal client reconciliation must respect pins.
pub fn stop_unchecked() {
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
        // Reserve and replace use the same lock order. A ticket cannot acquire
        // the old lease between checking pins and removing that listener.
        let pins = listener_pins().lock().map_err(|_| "newapi.errors.storage")?;
        if pinned_port(&pins).is_some_and(|p| p != port || port == 0) {
            return Err("claudeDesktop.errors.listenerPinned".into());
        }
        let mut guard = handle_slot().lock().map_err(|_| "newapi.errors.storage")?;
        if let Some(handle) = guard.as_ref() {
            if handle.port == port {
                *handle
                    .outbound
                    .write()
                    .map_err(|_| "newapi.errors.storage")? = next_outbound;
                return Ok(handle.port);
            }
        }
        if let Some(handle) = guard.take() {
            let _ = handle.shutdown.send(true);
        }
    }
    let listener = bind_localhost(port).await?;
    let bound_port = listener
        .local_addr()
        .map_err(|_| "newapi.errors.network".to_owned())?
        .port();
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static EPOCH: OnceLock<String> = OnceLock::new();
    let lease = ListenerLease {
        bound_port,
        generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
        process_epoch: EPOCH.get_or_init(|| uuid::Uuid::new_v4().to_string()).clone(),
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let outbound = Arc::new(RwLock::new(next_outbound));
    let context = Arc::new(ArcContext {
        bound_port,
        store_path: paths.app_data_dir.join("newapi-providers.dat"),
        app_data_dir: paths.app_data_dir.clone(),
        codex_home: paths.codex_home.clone(),
        auth_path: paths.auth_path.clone(),
        outbound: outbound.clone(),
        outbound_direct: outbound_client(&ApiProxyConfigPayload {
            mode: super::models::ApiProxyMode::Direct,
            url: None,
        })?,
        failover_seen: failover_seen_for(&paths.app_data_dir.join("newapi-providers.dat")),
    });
    let pins = listener_pins().lock().map_err(|_| "newapi.errors.storage")?;
    if pinned_port(&pins).is_some_and(|p| p != bound_port) {
        return Err("claudeDesktop.errors.listenerPinned".into());
    }
    *handle_slot().lock().map_err(|_| "newapi.errors.storage")? = Some(ProxyHandle {
        port: bound_port,
        lease,
        shutdown: shutdown_tx,
        outbound,
    });
    tokio::spawn(accept_loop(listener, shutdown_rx, context));
    Ok(bound_port)
}

struct ArcContext {
    bound_port: u16,
    store_path: PathBuf,
    app_data_dir: PathBuf,
    codex_home: PathBuf,
    auth_path: PathBuf,
    outbound: Arc<RwLock<Client>>,
    /// 供应商网络模式为 direct 时使用的无代理客户端。
    outbound_direct: Client,
    /// Once this process has routed with failover, missing affinity is no longer
    /// evidence that a continuation is safe to send to its original channel.
    failover_seen: Arc<std::sync::atomic::AtomicBool>,
}

async fn accept_loop(
    listener: tokio::net::TcpListener,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    context: Arc<ArcContext>,
) {
    // Observe Codex's locally synced catalog in the background. No extra network
    // endpoint or credential is introduced, and failed refreshes keep last-good.
    let period = Duration::from_secs(30);
    let mut refresh = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut refreshing: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = refresh.tick() => {
                if refreshing.as_ref().is_none_or(|task| task.is_finished()) {
                    let context = context.clone();
                    refreshing = Some(tokio::task::spawn_blocking(move || {
                        super::model_catalog::official(&context.codex_home, &context.app_data_dir);
                    }));
                }
            }
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

fn validate_desktop_endpoint(method: &hyper::Method, path: &str) -> ForwardResult<()> {
    let allowed = match path {
        "/claude-desktop/v1/models" => *method == hyper::Method::GET,
        "/claude-desktop/v1/messages" => *method == hyper::Method::POST,
        _ => return Err((StatusCode::NOT_FOUND, "unsupported Desktop gateway endpoint")),
    };
    if allowed { Ok(()) } else { Err((StatusCode::METHOD_NOT_ALLOWED, "unsupported Desktop gateway method")) }
}

fn desktop_authorized(headers: &HeaderMap, token: &str) -> bool {
    if token.is_empty() { return false; }
    headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")))
        == Some(token)
}

fn is_unsafe_path_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    let decoded = lower.replace("%2e", ".");
    decoded == "."
        || decoded == ".."
        || lower.contains("%2f")
        || lower.contains("%5c")
        || segment.contains('\\')
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
        ["claude-desktop", "v1", "messages"] => Some(router::ModelProtocol::AnthropicMessages),
        ["codex", "v1", ..] | ["v1", "responses", ..] | ["p", _, "v1", "responses", ..] => {
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

/// This local token grants inference access, not access to arbitrary upstream
/// administration, file storage or billing APIs with a provider's credentials.
fn validate_endpoint(method: &hyper::Method, segments: &[&str]) -> ForwardResult<()> {
    let suffix = match segments {
        ["codex", "v1", suffix @ ..] | ["p", _, "v1", suffix @ ..] | ["v1", suffix @ ..] => suffix,
        _ => return Err((StatusCode::NOT_FOUND, "unknown router endpoint")),
    };
    let allowed = match suffix {
        ["models"] | ["models", _] => matches!(*method, hyper::Method::GET | hyper::Method::HEAD),
        ["responses"] | ["responses", "compact"] | ["messages"] | ["messages", "count_tokens"]
            | ["chat", "completions"] => *method == hyper::Method::POST,
        ["responses", _] => matches!(*method, hyper::Method::GET | hyper::Method::HEAD | hyper::Method::DELETE),
        ["responses", _, "cancel"] => *method == hyper::Method::POST,
        ["responses", _, "input_items"] => matches!(*method, hyper::Method::GET | hyper::Method::HEAD),
        _ => return Err((StatusCode::NOT_FOUND, "unsupported router endpoint")),
    };
    if allowed { Ok(()) } else { Err((StatusCode::METHOD_NOT_ALLOWED, "unsupported method for router endpoint")) }
}

fn is_inference_create(method: &hyper::Method, segments: &[&str]) -> bool {
    if *method != hyper::Method::POST {
        return false;
    }
    matches!(
        segments,
        ["claude-desktop", "v1", "messages"]
            |
        ["codex", "v1", "responses" | "messages"]
            | ["codex", "v1", "chat", "completions"]
            | ["p", _, "v1", "responses" | "messages"]
            | ["p", _, "v1", "chat", "completions"]
            | ["v1", "responses" | "messages"]
            | ["v1", "chat", "completions"]
    )
}

fn is_response_resource(segments: &[&str]) -> bool {
    let suffix = match segments {
        ["codex", "v1", suffix @ ..] | ["p", _, "v1", suffix @ ..] | ["v1", suffix @ ..] => suffix,
        _ => return false,
    };
    matches!(suffix, ["responses", id] if *id != "compact")
        || matches!(suffix, ["responses", _, "cancel" | "input_items"])
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
    if segments
        .iter()
        .any(|segment| is_unsafe_path_segment(segment))
    {
        return Err((StatusCode::BAD_REQUEST, "invalid path"));
    }
    // Select the namespace before authentication. Desktop's local token never
    // grants the existing /v1, /p or /codex endpoints, and vice versa.
    let desktop_namespace = segments.first() == Some(&"claude-desktop");
    let method = request.method().clone();
    let desktop = if desktop_namespace {
        validate_desktop_endpoint(&method, &path)?;
        let snapshot = super::claude_desktop::gateway::committed_snapshot(&context.app_data_dir)
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "Desktop gateway recovery required"))?
            .ok_or((StatusCode::UNAUTHORIZED, "Desktop gateway is not managed"))?;
        if !desktop_authorized(request.headers(), &snapshot.token) {
            return Err((StatusCode::UNAUTHORIZED, "invalid Desktop gateway token"));
        }
        if snapshot.bound_port != context.bound_port {
            return Err((StatusCode::SERVICE_UNAVAILABLE, "Desktop gateway listener changed"));
        }
        Some(snapshot)
    } else {
        None
    };
    let store = router::read_store(&context.store_path).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "router storage unavailable",
        )
    })?;
    if desktop.is_none() && !authorized(request.headers(), &store.router.bearer_token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid local router token"));
    }

    if desktop.is_none() {
        validate_endpoint(&method, &segments)?;
    }
    // A refresh can replace the shared catalog Arc while a body is being read
    // or converted. Desktop eligibility, conversion and pricing must all use
    // the catalog generation captured for this request.
    let desktop_catalog = desktop.as_ref().map(|_| super::models_dev::load(&context.app_data_dir));
    if path == "/claude-desktop/v1/models" {
        let value = super::claude_desktop::gateway::models(
            desktop.as_ref().expect("authenticated Desktop namespace"),
            &store,
            desktop_catalog.as_deref().expect("Desktop request catalog"),
        ).map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "no available Desktop routes"))?;
        return Ok(Response::builder().status(StatusCode::OK).header(CONTENT_TYPE, "application/json")
            .body(full_body(value.to_string())).expect("static response parts"));
    }
    let has_body = !matches!(method, hyper::Method::GET | hyper::Method::HEAD);
    let codex_auth = desktop.is_none().then(|| codex_request_auth(request.headers(), &store.router.bearer_token)).flatten();
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
    let desktop_one_m_beta = desktop.is_some() && headers.get_all("anthropic-beta").iter()
        .filter_map(|value| value.to_str().ok()).flat_map(|value| value.split(','))
        .any(|value| value.trim() == CLAUDE_ONE_M_BETA);
    let body: Option<Bytes> = if has_body {
        Some(read_limited_body(request.into_body()).await?)
    } else {
        read_limited_body(request.into_body()).await?;
        None
    };

    let required_protocol = request_protocol(&segments);
    let (requested_model, one_m_context) = requested_model(body.as_deref(), required_protocol);
    let inference_create = is_inference_create(&method, &segments);
    let response_resource = is_response_resource(&segments);
    let compact = method == hyper::Method::POST
        && matches!(
            segments.as_slice(),
            ["codex", "v1", "responses", "compact"]
                | ["p", _, "v1", "responses", "compact"]
                | ["v1", "responses", "compact"]
        );
    let same_model_failover = desktop.is_none() && store.router.same_model_failover;
    if (inference_create || compact) && same_model_failover {
        context
            .failover_seen
            .store(true, std::sync::atomic::Ordering::Release);
    }
    let failover_seen = desktop.is_none() && context
        .failover_seen
        .load(std::sync::atomic::Ordering::Acquire);

    // Codex model catalog for the model picker (`model_catalog_json` mirror).
    if matches!(segments.as_slice(), ["codex", "v1", "models"]) && !has_body {
        return Ok(codex_catalog_response(&context, &store));
    }
    let trace = route_trace::Trace::begin(
        requested_model.as_deref().unwrap_or_default(),
        body.as_deref(),
    );
    if let Some(snapshot) = &desktop {
        trace.identity(&request_diagnostics::RequestIdentity {
            client_source: Some("claudeDesktop".into()),
            route_revision: Some(snapshot.route_revision),
            ..Default::default()
        });
    }
    let result = async {
        // Identify provider routes without consulting the original channel's
        // availability: an existing session may already have switched away.
        let provider_route = match segments.as_slice() {
            ["codex", "v1", ..] => requested_model
                .as_deref()
                .is_some_and(|m| m.contains("::")),
            ["p", _, "v1", ..] | ["v1", ..] => true,
            _ => false,
        };
        let affinity_key = (provider_route && (inference_create || compact || response_resource))
            .then(|| {
                failover::key(
                    &context.store_path,
                    &headers,
                    requested_model.as_deref().unwrap_or_default(),
                )
            })
            .flatten();
        let _decision = if let Some(key) = &affinity_key {
            Some(failover::decision_lock(key).await)
        } else {
            None
        };
        let binding = affinity_key.as_deref().and_then(failover::bound);
        let continuation = compact || (inference_create && failover::continuation(body.as_deref()));
        let fresh_turn = inference_create && failover::fresh_turn(body.as_deref());
        if !same_model_failover && binding.as_ref().is_some_and(|binding| binding.requires_failover) {
            if continuation {
                return Err((StatusCode::CONFLICT,
                    "failover is disabled for this bound conversation; re-enable it or start a new conversation"));
            }
        }
        // A new turn while disabled selects the ordinary route. Replace the old
        // binding only after all path/body/tool checks succeed, immediately
        // before forwarding; rejected requests must not destroy affinity.
        let selected_binding = if !same_model_failover && fresh_turn { None } else { binding.as_ref() };
        let desktop_alias = desktop.as_ref().and_then(|_| raw_requested_model(body.as_deref()))
            .map(|alias| alias.trim().to_owned());
        let desktop_route = if let Some(snapshot) = &desktop {
            let alias = desktop_alias.as_deref()
                .ok_or((StatusCode::BAD_REQUEST, "Desktop model alias is required"))?;
            let with_marker = (desktop_one_m_beta && !strip_claude_one_m_marker(alias).1)
                .then(|| format!("{alias}[1m]"));
            Some(super::claude_desktop::gateway::resolve_message(snapshot, &store,
                desktop_catalog.as_deref().expect("Desktop request catalog"), with_marker.as_deref().unwrap_or(alias))
                .map_err(|_| (StatusCode::BAD_REQUEST, "Desktop route is unknown or unavailable"))?)
        } else { None };
        let one_m_context = desktop_route.as_ref().map(|route| route.view.one_m).unwrap_or(one_m_context);
        let identity = desktop_route.as_ref().map(|route| request_diagnostics::RequestIdentity {
            client_source: Some("claudeDesktop".into()),
            requested_model: desktop_alias.clone(),
            resolved_model: Some(route.choice.model_id.clone()),
            route_revision: desktop.as_ref().map(|s| s.route_revision),
        });
        if let Some(identity) = &identity { trace.identity(identity); }
        let (mut destination, mut selection_reason) = if let Some(route) = &desktop_route {
            let upstream_protocol = route.provider.protocol_for_model(&route.choice.model_id);
            let translation = protocol::Translation::between(router::ModelProtocol::AnthropicMessages, upstream_protocol);
            (Destination::Provider { provider: Box::new(route.provider.clone()),
                model_override: Some(route.choice.model_id.clone()), upstream_protocol, translation }, "desktop_role_mapping".into())
        } else { resolve_destination_explain(
            &store,
            &segments,
            required_protocol,
            requested_model.as_deref(),
            selected_binding,
        )? };
        if one_m_context
            && desktop.is_none()
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
        let stateless = same_model_failover
            && binding.is_none()
            && affinity_key.is_some()
            && failover::stateless(body.as_deref());
        if failover_seen
            && affinity_key.is_some()
            && binding.is_none()
            && continuation
        {
            return Err((
                StatusCode::CONFLICT,
                "conversation channel is unknown or expired; start a new conversation",
            ));
        }
        let can_retry = same_model_failover
            && binding.is_none()
            && affinity_key.is_some()
            && fresh_turn
            && stateless;
        trace.stage("build");
        let body = if required_protocol == Some(router::ModelProtocol::Responses) && !response_resource {
            session_compat::prepare_reasoning(body, || {
                requested_model.as_deref().and_then(|slug| {
                    router::catalog_entry_for_slug(
                        &context.app_data_dir,
                        &context.codex_home,
                        &store,
                        slug,
                    )
                })
            })?
        } else {
            body
        };
        let usage_request_id = trace.request_id().to_owned();
        for attempt in 0..2 {
            let attempt_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let suffix = upstream_suffix(&segments, &destination, required_protocol)?;
            let pricing = relay_pricing(&context, &destination, requested_model.as_deref(), desktop_catalog.as_deref());
            let attempt_metadata = route_trace::AttemptMetadata {
                attempt_id: attempt_id.clone(),
                catalog_revision: pricing.revision.clone(),
                match_kind: pricing.match_kind.clone(),
                pricing_status: pricing.status().into(),
                pricing_source: pricing.source.clone(),
                selection_reason: selection_reason.clone(),
                failover_reason: (attempt > 0).then(|| selection_reason.clone()),
            };
            let mut relay_request = relay_usage_request(
                &context,
                &destination,
                required_protocol,
                requested_model.as_deref(),
                pricing,
            ).map(|request| {
                let request = request.with_request_id(&usage_request_id).with_attempt_id(&attempt_id);
                if let Some(identity) = &identity { request.with_identity(identity.clone()) } else { request }
            });
            let (upstream_protocol, upstream_model) = match &destination {
                Destination::Provider { provider, model_override, upstream_protocol, .. } =>
                    (*upstream_protocol, model_override.clone().unwrap_or_else(|| provider.model.clone())),
                Destination::Official => (router::ModelProtocol::Responses, requested_model.clone().unwrap_or_default()),
            };
            let (url, mut attempt_headers) = match &destination {
                Destination::Provider {
                    provider,
                    model_override,
                    upstream_protocol,
                    translation,
                } => {
                    trace.attempt(
                        &provider.id,
                        &provider.name,
                        model_override.as_deref().unwrap_or(&provider.model),
                        match upstream_protocol {
                            router::ModelProtocol::Responses => "responses",
                            router::ModelProtocol::ChatCompletions => "chatCompletions",
                            router::ModelProtocol::AnthropicMessages => "anthropicMessages",
                        },
                        attempt_metadata.clone(),
                    );
                    provider_target(
                        provider,
                        *upstream_protocol,
                        *translation,
                        &suffix,
                        query.as_deref(),
                        headers.clone(),
                    )?
                }
                Destination::Official => {
                    trace.attempt(
                        "official",
                        "OpenAI",
                        requested_model.as_deref().unwrap_or_default(),
                        "responses",
                        attempt_metadata.clone(),
                    );
                    official_target(
                        &context,
                        codex_auth.clone(),
                        &suffix,
                        query.as_deref(),
                        headers.clone(),
                    )?
                }
            };
            if one_m_context && upstream_protocol == router::ModelProtocol::AnthropicMessages {
                append_anthropic_beta(&mut attempt_headers);
            }
            let desktop_history_scope = desktop_route.as_ref().zip(desktop.as_ref()).map(|(route, snapshot)| {
                protocol::message_history::ClientScope {
                    client_source: "claudeDesktop", config_identity: &snapshot.paths_identity,
                    model_alias: &route.choice.role_alias, route_revision: snapshot.route_revision,
                }
            });
            let message_context = match &destination {
                Destination::Provider { provider, translation: Some(pair), .. }
                    if pair.from == router::ModelProtocol::AnthropicMessages => {
                    body.as_deref().and_then(|bytes| protocol::message_history::Context::new_with_client_scope(
                        &[&context.store_path.to_string_lossy(), &provider.id, &url,
                          attempt_headers.get(AUTHORIZATION).or_else(|| attempt_headers.get("x-api-key")).and_then(|v| v.to_str().ok()).unwrap_or_default(),
                          &upstream_model, affinity_key.as_deref().unwrap_or_default()], pair.to, bytes, affinity_key.as_deref(), desktop_history_scope.as_ref()))
                }
                _ => None,
            };
            // Resource operations read the established channel but do not carry
            // a create request; preserve their body, including an empty cancel.
            // upstream_suffix already rejects translated Responses resources.
            let (mut sent, translated_input, tool_namespaces) = if response_resource {
                (body.clone(), Vec::new(), Default::default())
            } else {
                upstream_body(&destination, body.clone())?
            };
            if let (Some(history), Some(bytes)) = (&message_context, &sent) {
                let mut value: Value = serde_json::from_slice(bytes).map_err(|_| (StatusCode::BAD_REQUEST, "invalid translated request"))?;
                history.restore(&mut value);
                sent = Some(Bytes::from(value.to_string()));
            }
            let sent = if let Destination::Provider { provider, translation: Some(translation), .. } = &destination {
                if translation.from == router::ModelProtocol::AnthropicMessages {
                    let official_snapshot = router::official_endpoint_applies(provider)
                        .then(|| super::model_catalog::official(&context.codex_home, &context.app_data_dir));
                    let official = official_snapshot.as_ref()
                        .and_then(|snapshot| router::matched_official_model(&snapshot.models, &upstream_model));
                    let metadata = router::resolve_model_metadata(
                        provider, &upstream_model, official,
                        &desktop_catalog.clone().unwrap_or_else(|| super::models_dev::load(&context.app_data_dir)),
                    );
                    session_compat::prepare_message_translation(sent, translation.to, &metadata)?
                } else { sent }
            } else { sent };
            let (sent, tool_adapter) = if response_resource {
                (sent, Default::default())
            } else {
                tool_schema::prepare(sent, &upstream_model, upstream_protocol)?
            };
            let outbound_client = client_for(&context, &destination)?;
            let credential_kind = if matches!(destination, Destination::Official) { "codexLocal" } else { "providerApiKey" };
            let credential_header = attempt_headers.get(AUTHORIZATION)
                .or_else(|| attempt_headers.get("x-api-key"))
                .and_then(|value| value.to_str().ok());
            let account_id = attempt_headers.get("chatgpt-account-id").and_then(|value| value.to_str().ok());
            let provider_id = match &destination {
                Destination::Provider { provider, .. } => provider.id.as_str(),
                Destination::Official => "official",
            };
            let credential_scope = format!("{}|{}", provider_id, url.split('?').next().unwrap_or(&url));
            let credential = request_diagnostics::CredentialSnapshot::from_header(
                &context.app_data_dir, credential_kind, &credential_scope, credential_header, account_id,
            );
            relay_request = relay_request.map(|request| request.with_credential(credential.clone()));
            trace.sent(sent.as_deref(), credential);
            let mut outbound = outbound_client.request(method.clone(), url)
                .headers(attempt_headers);
            if let Some(sent) = sent {
                outbound = outbound.body(sent);
            }
            if inference_create && failover_seen && (fresh_turn || continuation) {
                if let Some(key) = &affinity_key {
                    if !failover::bind_for_mode(key, &destination, same_model_failover) {
                        return Err((StatusCode::SERVICE_UNAVAILABLE,
                            "conversation routing capacity reached; wait for idle sessions to expire or start a new conversation"));
                    }
                }
            }
            let alternative = (can_retry && attempt == 0)
                .then(|| failover::alternative(&store, &destination, required_protocol))
                .flatten();
            let response = match outbound.send().await {
                Ok(response) => response,
                Err(error) => {
                    if let Some(request) = relay_request {
                        request.failed(StatusCode::BAD_GATEWAY.as_u16(), "response_not_received");
                    }
                    trace.failed_attempt(
                        "connect",
                        if error.is_timeout() {
                            "timeout"
                        } else {
                            "transport_error"
                        },
                        started.elapsed().as_millis() as u64,
                    );
                    // A timeout or broken established connection may already have executed.
                    if error.is_connect() && !error.is_timeout() {
                        if let Some(next) = alternative {
                            destination = next;
                            selection_reason = "failover_connect_error".into();
                            if let Some(key) = &affinity_key {
                                failover::bind_for_mode(key, &destination, true);
                            }
                            continue;
                        }
                    }
                    trace.fail(
                        "connect",
                        if error.is_timeout() {
                            "timeout"
                        } else {
                            "transport_error"
                        },
                    );
                    return Err((StatusCode::BAD_GATEWAY, "cannot reach the provider"));
                }
            };
            trace.status(response.status().as_u16());
            // Only explicit admission rejection, never an ambiguous 5xx response.
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                if let Some(next) = alternative {
                    if let Some(request) = relay_request {
                        request.failed(429, "upstream_http_error");
                    }
                    trace.failed_attempt(
                        "headers",
                        "rate_limited",
                        started.elapsed().as_millis() as u64,
                    );
                    destination = next;
                    selection_reason = "failover_rate_limited".into();
                    if let Some(key) = &affinity_key {
                        failover::bind_for_mode(key, &destination, true);
                    }
                    continue;
                }
            }
            drop(_decision);
            let translation = match &destination {
                Destination::Provider { translation, .. } => *translation,
                Destination::Official => None,
            };
            return relay_response(
                response,
                translation,
                relay_request,
                translated_input,
                trace.clone(),
                upstream_protocol,
                upstream_model,
                attempt_id,
                message_context,
                tool_adapter,
                tool_namespaces,
            )
            .await;
        }
        unreachable!("at most one failover")
    }
    .await;
    if let Err((status, message)) = &result {
        trace.reject(status.as_u16(), message);
    }
    result
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
    let raw = raw_requested_model(body);
    raw.map_or((None, false), |model| {
        if required_protocol == Some(router::ModelProtocol::AnthropicMessages) {
            let (base, enabled) = strip_claude_one_m_marker(&model);
            (Some(base.to_owned()), enabled)
        } else {
            (Some(model), false)
        }
    })
}

/// Skip message/tool bodies while observing only the model identity.
fn raw_requested_model(body: Option<&[u8]>) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct ModelOnly { model: Option<String> }
    body.and_then(|bytes| serde_json::from_slice::<ModelOnly>(bytes).ok()).and_then(|v| v.model)
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
#[cfg(test)]
fn resolve_destination(
    store: &router::Store,
    segments: &[&str],
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
    binding: Option<&failover::Binding>,
) -> ForwardResult<Destination> {
    resolve_destination_explain(store, segments, required_protocol, requested_model, binding)
        .map(|(destination, _)| destination)
}

fn resolve_destination_explain(
    store: &router::Store,
    segments: &[&str],
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
    binding: Option<&failover::Binding>,
) -> ForwardResult<(Destination, String)> {
    let prefixed_model = requested_model.and_then(|model| model.split_once("::"));
    let select_provider = |id: &str, protocol, model: Option<String>| {
        if let Some(binding) = binding {
            // Validate only the actual channel. Path/model namespace checks
            // below still apply before reaching this selection.
            let destination = provider_destination(
                store,
                &binding.provider_id,
                protocol,
                Some(binding.model.clone()),
            )?;
            if model.as_deref().is_some_and(|m| m != binding.model)
                || !failover::agrees(binding, &destination)
            {
                return Err((
                    StatusCode::CONFLICT,
                    "conversation channel changed; start a new conversation",
                ));
            }
            Ok(destination)
        } else {
            provider_destination(store, id, protocol, model)
        }
    };
    let reason = if binding.is_some() {
        "session_binding"
    } else {
        ""
    };
    let (destination, reason) = match segments {
        // Codex protocol: model slug `<provider id>::<model>` decides the route.
        ["codex", "v1", ..] => match prefixed_model {
            Some((provider_id, model_id)) => (
                select_provider(
                    provider_id,
                    Some(router::ModelProtocol::Responses),
                    Some(model_id.to_owned()),
                )?,
                if reason.is_empty() {
                    "codex_slug"
                } else {
                    reason
                },
            ),
            // Official model slugs (and anything unprefixed) go to OpenAI with local credentials.
            _ => (Destination::Official, "official"),
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
            (
                select_provider(id, required_protocol, model_override)?,
                if reason.is_empty() {
                    "path_provider"
                } else {
                    reason
                },
            )
        }
        ["v1", ..] => {
            if let Some((provider_id, model_id)) = prefixed_model {
                (
                    select_provider(provider_id, required_protocol, Some(model_id.to_owned()))?,
                    if reason.is_empty() {
                        "model_slug"
                    } else {
                        reason
                    },
                )
            } else if let Some(binding) = binding {
                (
                    select_provider(
                        &binding.provider_id,
                        required_protocol,
                        requested_model.map(str::to_owned),
                    )?,
                    "session_binding",
                )
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
                    Some(provider) => (
                        select_provider(
                            &provider.id,
                            required_protocol,
                            requested_model.map(str::to_owned),
                        )?,
                        if pinned.is_some() {
                            "active_provider"
                        } else {
                            "effective_active"
                        },
                    ),
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
    };
    Ok((destination, reason.into()))
}

/// The path below the upstream's `/v1/`, rewritten for a translated protocol.
fn upstream_suffix(
    segments: &[&str],
    destination: &Destination,
    required_protocol: Option<router::ModelProtocol>,
) -> ForwardResult<String> {
    let suffix = match segments {
        ["claude-desktop", "v1", rest @ ..] => rest.join("/"),
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

/// Price snapshot for the attempt about to be sent, fixed before the request
/// leaves so the ledger and diagnostics agree with what was actually used.
fn relay_pricing(
    context: &ArcContext,
    destination: &Destination,
    requested_model: Option<&str>,
    catalog: Option<&super::models_dev::Catalog>,
) -> relay_usage::PricingSnapshot {
    match destination {
        Destination::Provider {
            provider,
            model_override,
            ..
        } => {
            let model = model_override
                .clone()
                .or_else(|| requested_model.map(str::to_owned))
                .unwrap_or_else(|| provider.model.clone());
            match catalog {
                Some(catalog) => router::pricing_for(provider, &model, catalog),
                None => router::pricing_for_app_data(&context.app_data_dir, provider, &model),
            }
        }
        Destination::Official => {
            let revision = crate::core::models_dev::load(&context.app_data_dir)
                .snapshot
                .revision
                .clone();
            relay_usage::PricingSnapshot::unknown("official", &revision)
        }
    }
}

fn relay_usage_request(
    context: &ArcContext,
    destination: &Destination,
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
    pricing: relay_usage::PricingSnapshot,
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
        ) => {
            let model = model_override
                .clone()
                .or_else(|| requested_model.map(str::to_owned))
                .unwrap_or_else(|| provider.model.clone());
            Some(relay_usage::RelayUsageRequest::new(
                context.app_data_dir.clone(),
                provider.id.clone(),
                provider.name.clone(),
                model,
                match upstream_protocol {
                    router::ModelProtocol::Responses => "responses",
                    router::ModelProtocol::ChatCompletions => "chatCompletions",
                    router::ModelProtocol::AnthropicMessages => "anthropicMessages",
                }
                .to_owned(),
                pricing,
            ))
        }
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
            "{}{}",
            super::newapi::provider_endpoint(endpoint.base_url, &format!("v1/{suffix}"))
                .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid provider URL"))?,
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
) -> ForwardResult<(Option<Bytes>, Vec<Value>, protocol::tool_namespaces::ToolNamespaces)> {
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
        return Ok((body, Vec::new(), Default::default()));
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
                    match error.as_str() {
                        "protocol.errors.unknownPreviousResponse" =>
                            "previous_response_id is unknown or expired; resend the full conversation",
                        "protocol.errors.unsupportedMessageRole" =>
                            "cannot translate request: unsupported conversation role",
                        "protocol.errors.unsupportedToolDefinition" =>
                            "cannot translate request: unsupported hosted/custom tool, tool choice or function schema; select a provider using the caller's native protocol",
                        "protocol.errors.unsupportedHostedTool" =>
                            "cannot translate hosted tools; select a Responses provider or disable hosted web search in the caller",
                        "protocol.errors.unsupportedNamespacedTool" =>
                            "cannot translate namespaced tools; select a Responses provider or disable namespaced agent/MCP tools in the caller",
                        "protocol.errors.unsupportedCustomTool" =>
                            "cannot translate custom/freeform tools; select a Responses provider or use function tools in the caller",
                        "protocol.errors.invalidToolArguments" =>
                            "cannot translate request: tool arguments must be a JSON object",
                        "protocol.errors.unsupportedContent" =>
                            "cannot translate request without loss: unsupported content block, history item or output format; select a provider using the caller's protocol",
                        "protocol.errors.invalidRequest" =>
                            "cannot translate request: invalid request fields or message structure",
                        _ => "cannot translate request",
                    },
                )
            })?;
            if translation.from == router::ModelProtocol::Responses {
                // Check the expanded history too: a cached continuation may carry
                // items that the target protocol cannot represent without loss.
                session_compat::validate_translated_input(&rewritten.input)?;
            }
            let encoded = serde_json::to_vec(&rewritten.body).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cannot re-encode request",
                )
            })?;
            Ok((Some(Bytes::from(encoded)), rewritten.input, rewritten.namespaces))
        }
        None if model_override.is_some()
            || *upstream_protocol == router::ModelProtocol::ChatCompletions =>
        {
            let prepared =
                prepare_relay_body(bytes, model_override.as_deref(), *upstream_protocol)?;
            Ok((Some(prepared), Vec::new(), Default::default()))
        }
        None => Ok((body, Vec::new(), Default::default())),
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
    trace: route_trace::Trace,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    message_context: Option<protocol::message_history::Context>,
    tool_adapter: tool_schema::Adapter,
    tool_namespaces: protocol::tool_namespaces::ToolNamespaces,
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
        Some(translation) if response_status.is_success() && is_event_stream => translated_stream(
            response,
            translation,
            relay_request,
            translated_input,
            trace,
            upstream_protocol,
            upstream_model,
            attempt_id,
            message_context,
            tool_adapter,
            tool_namespaces,
        ),
        Some(translation) if response_status.is_success() => {
            let mut diagnostic = ResponseObservation::new(
                trace.clone(),
                relay_request,
                upstream_protocol,
                &upstream_model,
                &attempt_id,
                response_status.as_u16(),
                false,
            );
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    diagnostic.error();
                    diagnostic.finalize(false);
                    return Err((StatusCode::BAD_GATEWAY, "cannot read provider response"));
                }
            };
            diagnostic.push(&bytes);
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                diagnostic.translation_error("invalid_provider_response");
                (StatusCode::BAD_GATEWAY, "invalid provider response")
            })?;
            tool_adapter.response(&mut value, upstream_protocol).map_err(|code| {
                diagnostic.translation_error(code);
                (StatusCode::BAD_GATEWAY, "invalid Gemini tool arguments")
            })?;
            let mut converted = protocol::response_body(translation, &value).map_err(|_| {
                diagnostic.translation_error("invalid_upstream_response");
                (
                    StatusCode::BAD_GATEWAY,
                    "cannot translate provider response",
                )
            })?;
            tool_namespaces.restore_response(&mut converted);
            diagnostic.finish();
            if translation.from == router::ModelProtocol::Responses {
                protocol::history::remember(&converted, translated_input);
            } else if let Some(history) = message_context {
                if let Ok(canonical) = protocol::canonical_response(translation, &value) {
                    history.remember(&canonical);
                }
            }
            BodyExt::boxed(
                Full::new(Bytes::from(converted.to_string())).map_err(|never| match never {}),
            )
        }
        Some(translation) => {
            // An upstream error must reach the caller in the caller's own error shape,
            // otherwise SDKs report a parse failure instead of the real message.
            let bytes = response.bytes().await.unwrap_or_default();
            let mut diagnostic = ResponseObservation::new(
                trace.clone(),
                relay_request,
                upstream_protocol,
                &upstream_model,
                &attempt_id,
                response_status.as_u16(),
                false,
            );
            diagnostic.push(&bytes);
            diagnostic.finish();
            let body = translated_error_body(translation.from, response_status, &bytes);
            trace.fail("headers", "upstream_http_error");
            BodyExt::boxed(Full::new(Bytes::from(body)).map_err(|never| match never {}))
        }
        None if response_status.is_success() && !tool_adapter.is_empty() && !is_event_stream => {
            let mut diagnostic = ResponseObservation::new(trace, relay_request, upstream_protocol,
                &upstream_model, &attempt_id, response_status.as_u16(), false);
            let bytes = response.bytes().await.map_err(|_| {
                diagnostic.error();
                (StatusCode::BAD_GATEWAY, "cannot read provider response")
            })?;
            diagnostic.push(&bytes);
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                diagnostic.translation_error("invalid_provider_response");
                (StatusCode::BAD_GATEWAY, "invalid provider response")
            })?;
            tool_adapter.response(&mut value, upstream_protocol).map_err(|code| {
                diagnostic.translation_error(code);
                (StatusCode::BAD_GATEWAY, "invalid Gemini tool arguments")
            })?;
            diagnostic.finish();
            BodyExt::boxed(Full::new(Bytes::from(value.to_string())).map_err(|never| match never {}))
        }
        None if response_status.is_success() && !tool_adapter.is_empty() => {
            adapted_native_stream(response, trace, relay_request, upstream_protocol, upstream_model, attempt_id, tool_adapter)
        }
        None => {
            let stream = observed_stream(
                response,
                trace,
                relay_request,
                upstream_protocol,
                upstream_model,
                attempt_id,
                is_event_stream,
            )
            .map(move |chunk| {
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
    trace: route_trace::Trace,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    message_context: Option<protocol::message_history::Context>,
    tool_adapter: tool_schema::Adapter,
    tool_namespaces: protocol::tool_namespaces::ToolNamespaces,
) -> ResponseBody {
    let observer = ResponseObservation::new(
        trace,
        relay_request,
        upstream_protocol,
        &upstream_model,
        &attempt_id,
        response.status().as_u16(),
        true,
    );
    // One owner finalizes usage after translation. Dropping this stream also
    // drops the observer, marking an interrupted delivery without losing usage.
    let stream = futures_util::stream::unfold(
        (
            response.bytes_stream(),
            observer,
            protocol::stream_translator(translation),
            Some(translated_input),
            message_context,
            tool_adapter.stream(upstream_protocol),
            tool_namespaces,
            false,
        ),
        move |(mut source, mut observer, mut translator, mut recorded, history, mut tools, namespaces, done)| async move {
            if done {
                return None;
            }
            let (events, done) = match source.next().await {
                Some(Ok(bytes)) => {
                    observer.push(&bytes);
                    (match tools.push(&bytes) {
                        Ok(decoded) => translator.push(&decoded),
                        Err(code) => translator.fail(code),
                    }, false)
                }
                Some(Err(_)) => {
                    observer.error();
                    (translator.fail("upstream_stream_error"), true)
                }
                None => (match tools.finish() {
                    Ok(()) => translator.finish(),
                    Err(code) => translator.fail(code),
                }, true),
            };
            if let Some(code) = translator.failure() {
                observer.translation_error(code);
            }
            let done = done || translator.is_finished();
            // Commit before exposing terminal events: a follow-up may arrive
            // before the upstream connection reaches EOF.
            if let Some(response) = translator.completed_response() {
                if let Some(input) = recorded.take() {
                    if translation.from == router::ModelProtocol::Responses {
                        let mut response = response.clone();
                        namespaces.restore_response(&mut response);
                        protocol::history::remember(&response, input);
                    } else if let Some(history) = &history {
                        history.remember(response);
                    }
                }
            }
            if done {
                observer.finish();
            }
            let events: Vec<_> = events.into_iter().map(|event| namespaces.restore_event(event)).collect();
            let frames = futures_util::stream::iter(events.into_iter().map(|event| {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(Bytes::from(event)))
            }));
            Some((
                frames,
                (source, observer, translator, recorded, history, tools, namespaces, done),
            ))
        },
    )
    .flatten();
    BodyExt::boxed(StreamBody::new(stream))
}

fn adapted_native_stream(
    response: reqwest::Response,
    trace: route_trace::Trace,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    protocol: router::ModelProtocol,
    model: String,
    attempt_id: String,
    adapter: tool_schema::Adapter,
) -> ResponseBody {
    let observer = ResponseObservation::new(trace, relay_request, protocol, &model, &attempt_id, response.status().as_u16(), true);
    let stream = futures_util::stream::unfold(
        (response.bytes_stream(), observer, adapter.stream(protocol), false),
        |(mut source, mut observer, mut adapter, done)| async move {
            if done { return None; }
            let (result, done) = match source.next().await {
                Some(Ok(bytes)) => { observer.push(&bytes); (adapter.push(&bytes), false) }
                Some(Err(_)) => { observer.error(); (Err("upstream_stream_error"), true) }
                None => (adapter.finish().map(|()| Bytes::new()), true),
            };
            let (bytes, done) = match result {
                Ok(bytes) => {
                    let done = done || adapter.is_finished();
                    if done { observer.finish(); }
                    (bytes, done)
                }
                Err(code) => {
                    observer.translation_error(code);
                    observer.finalize(false);
                    // End the body with an error, never with an executable tool
                    // or a success terminal frame after invalid JSON arguments.
                    return Some((Err::<Frame<Bytes>, Box<dyn std::error::Error + Send + Sync>>(
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, code))),
                        (source, observer, adapter, true)));
                }
            };
            Some((Ok(Frame::data(bytes)), (source, observer, adapter, done)))
        },
    );
    BodyExt::boxed(StreamBody::new(stream))
}

fn observed_stream(
    response: reqwest::Response,
    trace: route_trace::Trace,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    sse: bool,
) -> impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send {
    let observer = ResponseObservation::new(
        trace,
        relay_request,
        upstream_protocol,
        &upstream_model,
        &attempt_id,
        response.status().as_u16(),
        sse,
    );
    futures_util::stream::unfold(
        (response.bytes_stream(), observer),
        |(mut stream, mut observer)| async move {
            match stream.next().await {
                Some(chunk) => {
                    match &chunk {
                        Ok(bytes) => observer.push(bytes),
                        Err(_) => observer.error(),
                    }
                    Some((chunk, (stream, observer)))
                }
                None => {
                    observer.finish();
                    None
                }
            }
        },
    )
}

/// One raw-byte observation per attempt. The same bounded model result is
/// delivered to the memory trace and the provider ledger before either closes.
struct ResponseObservation {
    trace: route_trace::Trace,
    diagnostic: route_trace::Observation,
    usage: Option<relay_usage::RelayUsageObserver>,
    served: request_diagnostics::ServedModelObserver,
    attempt_id: String,
    finalized: bool,
    delivered: bool,
}

impl ResponseObservation {
    fn new(
        trace: route_trace::Trace,
        request: Option<relay_usage::RelayUsageRequest>,
        protocol: router::ModelProtocol,
        model: &str,
        attempt_id: &str,
        status: u16,
        sse: bool,
    ) -> Self {
        Self {
            diagnostic: route_trace::Observation::new(trace.clone(), sse, status),
            trace,
            usage: request.map(|request| request.observe(status, sse)),
            served: request_diagnostics::ServedModelObserver::new(protocol, model, status, sse),
            attempt_id: attempt_id.into(),
            finalized: false,
            delivered: false,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        self.diagnostic.push(bytes);
        self.served.push(bytes);
        if let Some(usage) = self.usage.as_mut() {
            usage.observe(bytes);
        }
    }
    fn error(&mut self) {
        self.diagnostic.error();
        self.served.error();
        if let Some(usage) = self.usage.as_mut() {
            usage.fail_delivery();
        }
    }
    fn translation_error(&mut self, code: &'static str) {
        self.trace.fail("translation", code);
        if let Some(usage) = self.usage.as_mut() {
            usage.fail_delivery();
        }
    }
    fn finish(&mut self) {
        self.delivered = true;
        self.finalize(true);
        self.diagnostic.finish();
    }
    fn finalize(&mut self, eof: bool) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let observation = self.served.finish(eof);
        self.trace.observed(&self.attempt_id, observation.clone());
        if let Some(usage) = self.usage.as_mut() {
            usage.set_model_observation(observation);
        }
    }
}

impl Drop for ResponseObservation {
    fn drop(&mut self) {
        if !self.delivered {
            self.trace.fail("stream", "delivery_interrupted");
            if let Some(usage) = self.usage.as_mut() {
                usage.fail_delivery();
            }
        }
        self.finalize(false);
    }
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
        .or_else(|| {
            value
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text.is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("upstream error")
                    .to_owned()
            } else {
                text.chars().take(2000).collect()
            }
        });
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            match status.as_u16() {
                400 => "invalid_request_error",
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                529 => "overloaded_error",
                _ => "api_error",
            }
            .to_owned()
        });
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
            assert_eq!(
                bind_localhost(port).await.unwrap_err(),
                "newapi.errors.portBusy"
            );

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
    fn responses_subroutes_keep_the_protocol_and_reject_lossy_translation() {
        let store = Store {
            providers: vec![Provider {
                id: "chat".into(),
                api_key: "synthetic-key".into(),
                model: "model".into(),
                enabled: true,
                protocol: router::ModelProtocol::ChatCompletions,
                ..Default::default()
            }],
            ..Default::default()
        };
        for segments in [
            vec!["codex", "v1", "responses", "compact"],
            vec!["v1", "responses", "compact"],
            vec!["p", "chat", "v1", "responses", "compact"],
        ] {
            let required = request_protocol(&segments);
            assert_eq!(required, Some(router::ModelProtocol::Responses));
            let destination =
                resolve_destination(&store, &segments, required, Some("chat::model"), None)
                    .unwrap();
            assert_eq!(
                upstream_suffix(&segments, &destination, required)
                    .unwrap_err()
                    .0,
                StatusCode::NOT_IMPLEMENTED
            );
        }
        let destination = provider_destination(
            &store,
            "chat",
            Some(router::ModelProtocol::Responses),
            Some("model".into()),
        )
        .unwrap();
        for item in [
            json!({"type":"compaction","encrypted_content":"opaque"}),
            json!({"type":"reasoning","encrypted_content":"opaque","summary":[]}),
            json!({"type":"item_reference","id":"provider-item"}),
        ] {
            let body = Bytes::from(
                json!({"model":"chat::model","input":[item,{"role":"user","content":"continue"}]})
                    .to_string(),
            );
            let error = upstream_body(&destination, Some(body)).unwrap_err();
            assert_eq!(error.0, StatusCode::BAD_REQUEST);
            assert!(error.1.contains("without loss"));
        }
        let body = Bytes::from(json!({"model":"chat::model","previous_response_id":"unknown-official-id","input":"continue"}).to_string());
        assert_eq!(
            upstream_body(&destination, Some(body)).unwrap_err(),
            (
                StatusCode::BAD_REQUEST,
                "previous_response_id is unknown or expired; resend the full conversation"
            )
        );

        // Cached output must not bypass the history-loss check on the next turn.
        let response_id = format!("compat-{}", uuid::Uuid::new_v4());
        protocol::history::remember(
            &json!({
                "id": response_id, "status":"completed", "output":[
                    {"type":"reasoning","encrypted_content":"opaque","summary":[]}
                ]
            }),
            vec![],
        );
        let body = Bytes::from(
            json!({"model":"chat::model","previous_response_id":response_id,"input":"continue"})
                .to_string(),
        );
        assert!(upstream_body(&destination, Some(body))
            .unwrap_err()
            .1
            .contains("without loss"));
    }

    #[test]
    fn local_token_only_allows_inference_models_and_supported_response_resources() {
        use hyper::Method;
        for prefix in [vec!["v1"], vec!["codex", "v1"], vec!["p", "fixture", "v1"]] {
            for (method, suffix) in [(Method::GET, vec!["models"]), (Method::GET, vec!["models", "m"]),
                (Method::POST, vec!["responses"]), (Method::POST, vec!["responses", "compact"]),
                (Method::GET, vec!["responses", "r"]), (Method::DELETE, vec!["responses", "r"]),
                (Method::POST, vec!["responses", "r", "cancel"]), (Method::GET, vec!["responses", "r", "input_items"]),
                (Method::POST, vec!["messages", "count_tokens"]), (Method::POST, vec!["messages"]),
                (Method::POST, vec!["chat", "completions"])] {
                let mut path = prefix.clone(); path.extend(suffix);
                assert!(validate_endpoint(&method, &path).is_ok());
            }
            for suffix in [vec!["files"], vec!["organization", "admin_keys"], vec!["fine_tuning", "jobs"], vec!["responses", "r", "unknown"]] {
                let mut path = prefix.clone(); path.extend(suffix);
                assert_eq!(validate_endpoint(&Method::POST, &path).unwrap_err().0, StatusCode::NOT_FOUND);
            }
            let mut path = prefix.clone(); path.push("responses");
            assert_eq!(validate_endpoint(&Method::PUT, &path).unwrap_err().0, StatusCode::METHOD_NOT_ALLOWED);
        }
    }

    #[test]
    fn translated_continuations_preserve_plain_reasoning_summaries() {
        for target in [
            router::ModelProtocol::ChatCompletions,
            router::ModelProtocol::AnthropicMessages,
        ] {
            let response_id = format!("summary-{}", uuid::Uuid::new_v4());
            protocol::history::remember(
                &json!({"id":response_id,"status":"completed","output":[
                    {"type":"reasoning","summary":[{"type":"summary_text","text":"Preserve this conclusion"}]},
                    {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Earlier answer"}]}
                ]}),
                vec![json!({"role":"user","content":"Original question"})],
            );
            let store = Store {
                providers: vec![Provider {
                    id: "relay".into(),
                    api_key: "fixture-key".into(),
                    model: "model".into(),
                    enabled: true,
                    protocol: target,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let destination = provider_destination(
                &store,
                "relay",
                Some(router::ModelProtocol::Responses),
                Some("model".into()),
            )
            .unwrap();
            let body = Bytes::from(json!({"model":"relay::model","previous_response_id":response_id,"input":"Follow-up"}).to_string());
            let (body, _, _) = upstream_body(&destination, Some(body)).unwrap();
            let translated: Value = serde_json::from_slice(&body.unwrap()).unwrap();
            let messages = translated["messages"].to_string();
            for text in [
                "Original question",
                "Preserve this conclusion",
                "Earlier answer",
                "Follow-up",
            ] {
                assert!(messages.contains(text), "{target:?}: {messages}");
            }
        }
    }

    #[test]
    fn official_requests_keep_provider_specific_history_and_the_official_route() {
        let store = Store::default();
        for endpoint in ["responses", "responses/compact"] {
            let path = format!("codex/v1/{endpoint}");
            let segments: Vec<_> = path.split('/').collect();
            let required = request_protocol(&segments);
            let destination =
                resolve_destination(&store, &segments, required, Some("official-model"), None)
                    .unwrap();
            assert!(matches!(destination, Destination::Official));
            assert_eq!(
                upstream_suffix(&segments, &destination, required).unwrap(),
                endpoint
            );
            let body = Bytes::from_static(br#"{ "model":"official-model", "previous_response_id":"relay-response", "input":[{"type":"compaction","encrypted_content":"relay-opaque"}] }"#);
            assert_eq!(
                upstream_body(&destination, Some(body.clone()))
                    .unwrap()
                    .0
                    .unwrap(),
                body
            );
        }
    }

    async fn echo_relay(label: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), service_fn(move |request: Request<Incoming>| async move {
                        let (parts, body) = request.into_parts();
                        let headers: serde_json::Map<String, Value> = parts.headers.iter().map(|(name, value)| {
                            (name.to_string(), json!(value.to_str().unwrap()))
                        }).collect();
                        let bytes = body.collect().await.unwrap().to_bytes();
                        let body: Value = serde_json::from_slice(&bytes).unwrap();
                        Ok::<_, std::convert::Infallible>(Response::builder()
                            .header(CONTENT_TYPE, "application/json")
                            .body(full_body(json!({"relay":label,"path":parts.uri.to_string(),"headers":headers,"body":body}).to_string())).unwrap())
                    })).await;
                });
            }
        });
        (port, task)
    }

    #[test]
    fn http_model_switch_and_compaction_route_to_matching_relays_without_official_credentials() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = tempfile::tempdir().unwrap();
            let app_data_dir = fixture.path().join("app");
            std::fs::create_dir_all(&app_data_dir).unwrap();
            let auth_path = fixture.path().join("auth.json");
            let auth = br#"{"tokens":{"access_token":"fixture-official-file-token","account_id":"fixture-file-account"},"OPENAI_API_KEY":"fixture-official-api-key"}"#;
            std::fs::write(&auth_path, auth).unwrap();
            // Synthetic exact model metadata exercises both valid `none` and a
            // target that rejects it, independently of family-name heuristics.
            // The relays declare their own levels; the account cache below must
            // not override a relay that does not target the official endpoint.
            std::fs::write(fixture.path().join("models_cache.json"), json!({"models":[
                {"slug":"model-a","supported_reasoning_levels":[{"effort":"none","description":"Off"}]},
                {"slug":"model-b","supported_reasoning_levels":[{"effort":"high","description":"High"}]}
            ]}).to_string()).unwrap();
            let (port_a, relay_a) = echo_relay("relay-a").await;
            let (port_b, relay_b) = echo_relay("relay-b").await;
            let mut store = Store {
                codex_access_mode: router::CodexAccessMode::Router,
                active_provider_id: Some("relay-a".into()),
                router: RouterSettings { bearer_token:"fixture-local-secret".into(), ..Default::default() },
                ..Default::default()
            };
            for (id, port, model, key, levels) in [
                ("relay-a", port_a, "model-a", "fixture-relay-a-key", json!([{"effort":"none","description":"Off"}])),
                ("relay-b", port_b, "model-b", "fixture-relay-b-key", json!([{"effort":"high","description":"High"}])),
            ] {
                store.providers.push(Provider {
                    id:id.into(), name:id.into(), base_url:format!("http://127.0.0.1:{port}"),
                    api_key:key.into(), model:model.into(), enabled:true,
                    models: vec![crate::core::newapi::Model {
                        id: model.into(),
                        display_name: model.into(),
                        included: true,
                        metadata_source: "upstream".into(),
                        supported_reasoning_levels: serde_json::from_value(levels).unwrap(),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            }
            let store_path = app_data_dir.join("newapi-providers.dat");
            crate::core::secret_file::write(&store_path, &serde_json::to_vec(&store).unwrap()).unwrap();
            let client = Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
            let context = Arc::new(ArcContext {
                bound_port: 0,
                failover_seen: failover_seen_for(&store_path),
                store_path, app_data_dir, codex_home:fixture.path().to_owned(), auth_path:auth_path.clone(),
                outbound:Arc::new(RwLock::new(client.clone())), outbound_direct:client.clone()
            });
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (shutdown, receiver) = tokio::sync::watch::channel(false);
            let router = tokio::spawn(accept_loop(listener, receiver, context));
            let history = json!([
                {"role":"user","content":"Original question"},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Original answer"}]},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"Retained summary"}],"encrypted_content":"fixture-opaque-reasoning"},
                {"type":"compaction","encrypted_content":"fixture-opaque-compaction"},
                {"type":"function_call","call_id":"call_fixture","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_fixture","output":"Retained tool result"},
                {"role":"user","content":"Continue the same conversation"}
            ]);
            // Switch A -> B -> A on one thread, including both native and legacy
            // compaction URLs. The pinned provider remains A throughout.
            for (prefix, endpoint, id, model, key, effort) in [
                ("codex/v1", "responses", "relay-a", "model-a", "fixture-relay-a-key", "none"),
                ("codex/v1", "responses", "relay-b", "model-b", "fixture-relay-b-key", "none"),
                ("codex/v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "high"),
                ("v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "none"),
                ("p/relay-b/v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "high"),
                ("codex/v1", "responses", "relay-a", "model-a", "fixture-relay-a-key", "none"),
            ] {
                let mut body = json!({
                    "model":format!("{id}::{model}"), "input":history, "previous_response_id":"fixture-provider-response",
                    "reasoning":{"effort":effort,"summary":"auto"}, "store":false
                });
                let response = client.post(format!("http://127.0.0.1:{port}/{prefix}/{endpoint}?fixture=1"))
                    .header(router::LOCAL_AUTH_HEADER, "fixture-local-secret")
                    .bearer_auth("fixture-current-official-token")
                    .header("chatgpt-account-id", "fixture-current-official-account")
                    .header("x-api-key", "fixture-client-api-key")
                    .header(COOKIE, "fixture-official-cookie")
                    .header(PROXY_AUTHORIZATION, "fixture-proxy-credential")
                    .header("x-codex-turn-metadata", "fixture-same-conversation")
                    .json(&body).send().await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let observed: Value = response.json().await.unwrap();
                assert_eq!(observed["relay"], id);
                assert_eq!(observed["path"], format!("/v1/{endpoint}?fixture=1"));
                assert_eq!(observed["headers"]["authorization"], format!("Bearer {key}"));
                assert_eq!(observed["headers"]["x-codex-turn-metadata"], "fixture-same-conversation");
                for header in ["chatgpt-account-id", "x-api-key", "cookie", "proxy-authorization", router::LOCAL_AUTH_HEADER] {
                    assert!(observed["headers"].get(header).is_none(), "leaked {header}");
                }
                body["model"] = json!(model);
                if model == "model-b" && effort == "none" {
                    body["reasoning"].as_object_mut().unwrap().remove("effort");
                }
                assert_eq!(observed["body"], body, "history and continuation id must survive model switches");
            }
            assert_eq!(std::fs::read(&auth_path).unwrap(), auth);
            shutdown.send(true).unwrap();
            router.await.unwrap();
            relay_a.abort();
            relay_b.abort();
        });
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
                        let messages = if request.contains(" /v1/responses ") { &payload["input"] } else { &payload["messages"] };
                        assert_eq!(messages[0]["role"], "user");
                        assert_eq!(messages[1]["role"], "system");
                        if request.contains(" /v1/responses ") {
                            assert_eq!(payload["reasoning"]["effort"], "high");
                        } else {
                            assert_eq!(payload["reasoning_effort"], "high");
                        }
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
                compression: Default::default(),
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
                model: "claude-mock".into(), context_window: Some(200000), max_output_tokens: Some(8192),
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
                        .json(&json!({"model":model,"max_tokens":32,"stream":stream,
                            "thinking":{"type":"adaptive"},"output_config":{"effort":"high"},
                            "messages":[{"role":"user","content":"hi"},
                                {"role":"system","content":[{"type":"text","text":"Dynamic system"}]}]}))
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
                    catalog_providers: None,
                    price_catalog_provider: None,
                    compression: None,
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
