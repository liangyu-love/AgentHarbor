use super::models::ApiProxyConfigPayload;
use super::{account_sources, protocol, relay_usage, request_diagnostics, route_trace, router};
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
use protocol::adapters::{AdapterRequest, InboundAdapter, WireAdapter};
#[cfg(test)]
use protocol::adapters::OutboundAdapter;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

mod aggregate;
mod aggregate_output;
mod aggregate_session;
mod codex_compat;
mod count_tokens;
#[cfg(test)]
#[path = "router_proxy/account_source_tests.rs"]
mod account_source_tests;
#[cfg(test)]
#[path = "router_proxy/gateway_tests.rs"]
mod gateway_tests;
#[cfg(test)]
mod aggregate_tests;
#[cfg(test)]
mod aggregate_live_tests;
#[cfg(test)]
#[path = "router_proxy/claude_cli_tests.rs"]
mod claude_cli_tests;
#[cfg(test)]
mod claude_continuation_tests;
#[cfg(test)]
#[path = "router_proxy/desktop_gateway_tests.rs"]
mod desktop_gateway_tests;
mod pipeline;
use pipeline::relay::*;
use pipeline::target::*;
use pipeline::translate::*;
mod hosted_search;
#[cfg(test)]
#[path = "router_proxy/hosted_search_tests.rs"]
mod hosted_search_tests;
mod native_sse;
#[cfg(test)]
#[path = "router_proxy/routing_safety_tests.rs"]
mod routing_safety_tests;
#[cfg(test)]
#[path = "router_proxy/codex_switch_tests.rs"]
mod codex_switch_tests;
#[cfg(test)]
#[path = "router_proxy/codex_compaction_tests.rs"]
mod codex_compaction_tests;
mod session_affinity;
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
    accept_task: tokio::task::JoinHandle<()>,
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
    handle_slot().lock().ok().and_then(|slot| {
        slot.as_ref()
            .filter(|handle| !handle.accept_task.is_finished())
            .map(|handle| handle.lease.clone())
    })
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
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|handle| !handle.accept_task.is_finished())
        })
        .unwrap_or(false)
}

pub fn running_port() -> Option<u16> {
    handle_slot()
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .filter(|handle| !handle.accept_task.is_finished())
                .map(|handle| handle.port)
        })
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
        let guard = handle_slot().lock().map_err(|_| "newapi.errors.storage")?;
        if let Some(handle) = guard.as_ref() {
            if handle.port == port && !handle.accept_task.is_finished() {
                *handle
                    .outbound
                    .write()
                    .map_err(|_| "newapi.errors.storage")? = next_outbound;
                return Ok(handle.port);
            }
        }
    }
    // Keep the old listener available if a requested new port cannot bind.
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
        #[cfg(test)]
        official_base: None,
    });
    let pins = listener_pins().lock().map_err(|_| "newapi.errors.storage")?;
    if pinned_port(&pins).is_some_and(|p| p != bound_port) {
        return Err("claudeDesktop.errors.listenerPinned".into());
    }
    let mut guard = handle_slot().lock().map_err(|_| "newapi.errors.storage")?;
    if let Some(handle) = guard.take() {
        let _ = handle.shutdown.send(true);
    }
    let accept_task = tokio::spawn(accept_loop(listener, shutdown_rx, context));
    *guard = Some(ProxyHandle {
        port: bound_port,
        lease,
        shutdown: shutdown_tx,
        outbound,
        accept_task,
    });
    Ok(bound_port)
}

#[derive(Clone)]
struct ArcContext {
    bound_port: u16,
    store_path: PathBuf,
    app_data_dir: PathBuf,
    codex_home: PathBuf,
    auth_path: PathBuf,
    outbound: Arc<RwLock<Client>>,
    /// 供应商网络模式为 direct 时使用的无代理客户端。
    outbound_direct: Client,
    /// A private loopback fixture endpoint; absent from production builds.
    #[cfg(test)]
    official_base: Option<String>,
}

impl ArcContext {
    fn official_base<'a>(&'a self, default: &'a str) -> &'a str {
        #[cfg(test)]
        if let Some(base) = &self.official_base {
            return base;
        }
        default
    }
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
    let mut accept_errors = 0u8;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = refresh.tick() => {
                if refreshing.as_ref().is_none_or(|task| task.is_finished()) {
                    let context = context.clone();
                    refreshing = Some(tokio::task::spawn_blocking(move || {
                        super::model_catalog::official(&context.codex_home, &context.app_data_dir);
                        let paths = CodexPaths::from_roots(
                            context.codex_home.clone(),
                            context.app_data_dir.clone(),
                        );
                        if let Ok(store) = router::load_store(&paths) {
                            if store.router.enabled && router::active_routing_config_is_ours(&paths) {
                                if let Err(error) = router::refresh_catalog_if_changed(&paths, &store)
                                {
                                    log::warn!("Codex account model catalog refresh failed: {error}");
                                }
                            }
                        }
                    }));
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(connection) => {
                        accept_errors = 0;
                        connection
                    }
                    Err(error) => {
                        accept_errors = accept_errors.saturating_add(1);
                        log::warn!("local router accept failed: {error}");
                        if accept_errors >= 3 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
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
        .read_timeout(Duration::from_secs(900))
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

fn desktop_count_tokens_response(body: &[u8]) -> Response<ResponseBody> {
    let (status, value) = match count_tokens::estimate_input_tokens(body) {
        Ok(input_tokens) => (StatusCode::OK, json!({"input_tokens": input_tokens})),
        Err(message) => (
            StatusCode::BAD_REQUEST,
            json!({"type":"error","error":{"type":"invalid_request_error","message":message}}),
        ),
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(full_body(value.to_string()))
        .expect("static count-tokens response")
}

fn count_tokens_estimate_on_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
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
        "/claude-desktop/v1/messages" | "/claude-desktop/v1/messages/count_tokens" => {
            *method == hyper::Method::POST
        }
        _ => {
            return Err((StatusCode::NOT_FOUND, "unsupported Desktop gateway endpoint",
            ))
        }
    };
    if allowed { Ok(()) } else { Err((StatusCode::METHOD_NOT_ALLOWED, "unsupported Desktop gateway method",
        )) }
}

fn desktop_authorized(headers: &HeaderMap, token: &str) -> bool {
    if token.is_empty() { return false; }
    headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))
        })
        .is_some_and(|candidate| token_matches(candidate, token))
}

fn token_matches(candidate: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
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

/// A context selection depends on capacity, never the member's wire protocol.
/// Keep the existing explicit opt-in for models without known capacity; a
/// conservative fallback is not evidence that an upstream only supports 200K.
fn validate_one_m_capacity(
    context: &ArcContext,
    store: &router::Store,
    destination: &Destination,
    catalog: &super::models_dev::Catalog,
) -> ForwardResult<()> {
    let Destination::Provider {
        provider,
        model_override,
        ..
    } = destination
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            "1M context requires a configured model",
        ));
    };
    let model = model_override.as_deref().unwrap_or(&provider.model);
    let official_snapshot = router::official_endpoint_applies(provider)
        .then(|| super::model_catalog::official(&context.codex_home, &context.app_data_dir));
    let official = official_snapshot
        .as_ref()
        .and_then(|snapshot| router::matched_official_model(&snapshot.models, model));
    let projection = router::model_projection(
        provider,
        model,
        official,
        catalog,
        &store.global_compression(),
    )
    .1;
    if !projection.context_is_fallback && projection.context_window < 1_000_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "selected model does not support 1M context",
        ));
    }
    Ok(())
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

fn request_protocol(segments: &[&str]) -> Option<router::ModelProtocol> {
    match segments {
        ["aggregate", "v1", "responses", ..] => Some(router::ModelProtocol::Responses),
        ["aggregate", "v1", "messages", ..] => Some(router::ModelProtocol::AnthropicMessages),
        ["aggregate", "v1", "chat", "completions"] => Some(router::ModelProtocol::ChatCompletions),
        ["claude-desktop", "v1", "messages"]
        | ["claude-desktop", "v1", "messages", "count_tokens"] => {
            Some(router::ModelProtocol::AnthropicMessages)
        }
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
        ["aggregate", "v1", suffix @ ..] => suffix,
        ["codex", "v1", suffix @ ..] | ["p", _, "v1", suffix @ ..] | ["v1", suffix @ ..] => suffix,
        _ => return Err((StatusCode::NOT_FOUND, "unknown router endpoint")),
    };
    let allowed = match suffix {
        // A model id may hold `/` (`relay-a/vendor/m`), so any depth below `models`.
        ["models", ..] => matches!(*method, hyper::Method::GET | hyper::Method::HEAD),
        ["responses"] | ["responses", "compact"] | ["messages"] | ["messages", "count_tokens"]
            | ["chat", "completions"] => *method == hyper::Method::POST,
        ["responses", _] => matches!(*method, hyper::Method::GET | hyper::Method::HEAD | hyper::Method::DELETE),
        ["responses", _, "cancel"] => *method == hyper::Method::POST,
        ["responses", _, "input_items"] => {
            matches!(*method, hyper::Method::GET | hyper::Method::HEAD)
        }
        _ => return Err((StatusCode::NOT_FOUND, "unsupported router endpoint")),
    };
    if allowed { Ok(()) } else { Err((StatusCode::METHOD_NOT_ALLOWED, "unsupported method for router endpoint",
        )) }
}

fn is_inference_create(method: &hyper::Method, segments: &[&str]) -> bool {
    if *method != hyper::Method::POST {
        return false;
    }
    matches!(
        segments,
        ["aggregate", "v1", "responses" | "messages"]
            | ["aggregate", "v1", "chat", "completions"]
            | ["claude-desktop", "v1", "messages"]
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
        ["aggregate", "v1", suffix @ ..] => suffix,
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

// Managed client configurations use a source-scoped URL. The scope is only
// diagnostic; the existing local router token still controls access. Legacy
// /p and /aggregate URLs remain valid and retain an unknown/generic source.
fn client_request_path(path: &str) -> (String, Option<&'static str>) {
    for (prefix, source) in [
        ("/clients/claude-code/", "claudeCode"),
        ("/clients/opencode/", "opencode"),
        ("/clients/pi/", "pi"),
        ("/clients/harness/", "harness"),
        ("/clients/codex/", "codex"),
        ("/clients/cursor/", "cursor"),
    ] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return (format!("/{rest}"), Some(source));
        }
    }
    (path.to_owned(), None)
}

async fn handle(mut request: Request<Incoming>, context: Arc<ArcContext>) -> ForwardResult {
    // Codex's built-in provider (`openai_base_url`) cannot add our header, so
    // its URL carries the local token. It is checked below like the header.
    let builtin = codex_compat::builtin_entry(request.uri().path())
        .map(|(path, token)| (path, token.to_owned()));
    let (path, tagged_source) = match &builtin {
        Some((path, token)) => {
            let token = HeaderValue::from_str(token)
                .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid local router token"))?;
            request
                .headers_mut()
                .insert(HeaderName::from_static(router::LOCAL_AUTH_HEADER), token);
            (path.clone(), Some("codex"))
        }
        None => client_request_path(request.uri().path()),
    };
    let query = request.uri().query().map(str::to_owned);
    if path == "/agentharbor/health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(full_body("ok".to_owned()))
            .expect("static body"));
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let client_source = if segments.first() == Some(&"codex") {
        Some("codex")
    } else {
        tagged_source
    };
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
            .map_err(|_| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Desktop gateway recovery required",
                )
            })?
            .ok_or((StatusCode::UNAUTHORIZED, "Desktop gateway is not managed"))?;
        if !desktop_authorized(request.headers(), &snapshot.token) {
            return Err((StatusCode::UNAUTHORIZED, "invalid Desktop gateway token"));
        }
        if snapshot.bound_port != context.bound_port {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Desktop gateway listener changed",
            ));
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
    // The gateway admits its named keys only, and nothing else admits them.
    let gateway_key = if segments.first() == Some(&"gateway") {
        let bearer = request.headers().get(AUTHORIZATION).and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let api_key = request.headers().get("x-api-key").and_then(|value| value.to_str().ok());
        Some(super::gateway::authenticate(&store, bearer, api_key).map(|key| key.name.clone())
            .ok_or((StatusCode::UNAUTHORIZED, "invalid gateway key"))?)
    } else {
        None
    };
    if desktop.is_none() && gateway_key.is_none() && !authorized(request.headers(), &store.router.bearer_token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid local router token"));
    }
    // `/mix`: a managed client combining sources (Claude Code) names models
    // by their gateway ids, with the router's own token and every kind.
    let mix = desktop.is_none() && gateway_key.is_none() && segments.first() == Some(&"mix");
    // Past authentication a gateway request is an ordinary `/v1` one whose
    // model names a provider by short name, or an aggregate.
    let segments: Vec<&str> = if gateway_key.is_some() || mix { segments[1..].to_vec() } else { segments };
    let gateway_source = gateway_key.as_ref().map(|name| format!("gateway:{name}"));
    let client_source: Option<&str> = gateway_source.as_deref().or(client_source);
    // Responses-over-WebSocket is first-party only; 426 makes Codex use HTTP.
    if builtin.is_some() && codex_compat::websocket_upgrade(request.headers()) {
        return Err((StatusCode::UPGRADE_REQUIRED, "use HTTP Responses streaming"));
    }

    if desktop.is_none() {
        validate_endpoint(&method, &segments)?;
    }
    // A refresh can replace the shared catalog Arc while a body is being read
    // or converted. Desktop eligibility, conversion and pricing must all use
    // the catalog generation captured for this request.
    let desktop_catalog = desktop
        .as_ref()
        .map(|_| super::models_dev::load(&context.app_data_dir));
    if matches!(
        segments.as_slice(),
        ["aggregate", "v1", "models"] | ["aggregate", "v1", "models", _]
    ) {
        let catalog = super::models_dev::load(&context.app_data_dir);
        let anthropic = query
            .as_deref()
            .is_some_and(|query| query.split('&').any(|part| part == "format=anthropic"));
        let mut value = aggregate::catalog(&store, &catalog, anthropic);
        if let ["aggregate", "v1", "models", model] = segments.as_slice() {
            value = value["data"]
                .as_array()
                .and_then(|entries| entries.iter().find(|entry| entry["id"] == *model))
                .cloned()
                .ok_or((StatusCode::NOT_FOUND, "unknown aggregate model"))?;
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(full_body(if method == hyper::Method::HEAD {
                String::new()
            } else {
                value.to_string()
            }))
            .expect("static response parts"));
    }
    if gateway_key.is_some() && matches!(segments.as_slice(), ["v1", "models", ..]) {
        let catalog = super::models_dev::load(&context.app_data_dir);
        let anthropic = request.headers().contains_key("anthropic-version")
            || query.as_deref().is_some_and(|query| query.split('&').any(|part| part == "format=anthropic"));
        let mut models = super::gateway::models(&store, &catalog);
        // Gateway ids hold `/`: the rest of the path is the id, raw or escaped.
        let one = (segments.len() > 2).then(|| segments[2..].join("/").replace("%2F", "/").replace("%2f", "/"));
        if let Some(model) = &one {
            models.retain(|entry| entry.id == *model);
            if models.is_empty() {
                return Err((StatusCode::NOT_FOUND, "unknown gateway model"));
            }
        }
        let mut value = super::gateway::models_document(&models, anthropic);
        if one.is_some() {
            value = value["data"][0].clone();
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(full_body(if method == hyper::Method::HEAD { String::new() } else { value.to_string() }))
            .expect("static response parts"));
    }
    if path == "/claude-desktop/v1/models" {
        let value = super::claude_desktop::gateway::models(
            desktop.as_ref().expect("authenticated Desktop namespace"),
            &store,
            desktop_catalog.as_deref().expect("Desktop request catalog"),
        )
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "no available Desktop routes",
            )
        })?;
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(full_body(value.to_string()))
            .expect("static response parts"));
    }
    let has_body = !matches!(method, hyper::Method::GET | hyper::Method::HEAD);
    let codex_auth = desktop
        .is_none()
        .then(|| codex_request_auth(request.headers(), &store.router.bearer_token))
        .flatten();
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BODY as u64)
    {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large"));
    }
    let mut headers = forwardable_headers(request.headers());
    let one_m_beta = headers
        .get_all("anthropic-beta")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim() == CLAUDE_ONE_M_BETA);
    let body: Option<Bytes> = if has_body {
        let body = read_limited_body(request.into_body()).await?;
        Some(codex_compat::decode_body(&mut headers, body)?)
    } else {
        read_limited_body(request.into_body()).await?;
        None
    };
    let headers = headers;

    let desktop_count_body = (path == "/claude-desktop/v1/messages/count_tokens")
        .then(|| body.clone().unwrap_or_default());

    let required_protocol = request_protocol(&segments);
    let (requested_model, one_m_context) = requested_model(body.as_deref(), required_protocol);
    // A gateway model id becomes the provider slug routing already reads
    // (`<provider id>::<model>`), or the aggregate path.
    let gateway_model = (gateway_key.is_some() || mix).then(|| requested_model.clone()).flatten();
    let gateway_target = gateway_model.as_deref().and_then(|model| if mix {
        super::gateway::resolve_with(&store, model, router::ModelSources::default())
    } else {
        super::gateway::resolve(&store, model)
    });
    if gateway_model.is_some() && gateway_target.is_none() {
        return Err((StatusCode::NOT_FOUND, if mix {
            "unknown model; it may have been removed or turned off since the client was configured"
        } else {
            "unknown gateway model; GET /gateway/v1/models lists the models this key can use"
        }));
    }
    let requested_model = match &gateway_target {
        Some(super::gateway::Target::Provider { provider_id, model_id }) => Some(format!("{provider_id}::{model_id}")),
        Some(super::gateway::Target::Aggregate { public_id }) => Some(public_id.clone()),
        None => requested_model,
    };
    let segments: Vec<&str> = if matches!(gateway_target, Some(super::gateway::Target::Aggregate { .. })) {
        ["aggregate"].into_iter().chain(segments.iter().copied()).collect()
    } else {
        segments
    };
    // Claude Code normally strips the marker before sending the request and
    // expresses the selected context through the Messages beta header instead.
    let one_m_context = one_m_context
        || (required_protocol == Some(router::ModelProtocol::AnthropicMessages) && one_m_beta);
    let inference_create = is_inference_create(&method, &segments);
    let response_resource = is_response_resource(&segments);
    let compact = method == hyper::Method::POST
        && matches!(
            segments.as_slice(),
            ["codex", "v1", "responses", "compact"]
                | ["p", _, "v1", "responses", "compact"]
                | ["v1", "responses", "compact"]
        );
    // Remote compaction (V2) is an ordinary create carrying a trigger item.
    let codex_create = client_source == Some("codex") && inference_create && !response_resource;
    let remote_compaction = codex_create && codex_compat::compaction_trigger(body.as_ref());
    // Codex model catalog for the model picker (`model_catalog_json` mirror).
    if matches!(segments.as_slice(), ["codex", "v1", "models"]) && !has_body {
        return Ok(codex_catalog_response(&context, &store));
    }
    let trace = if desktop_count_body.is_some() {
        route_trace::Trace::untracked()
    } else {
        route_trace::Trace::begin(gateway_model.as_deref().or(requested_model.as_deref()).unwrap_or_default(), body.as_deref())
    };
    // Codex reaches every aggregate its menu offers by the public ID.
    let codex_aggregate = codex_create
        && router::codex_sources(&store).aggregate
        && requested_model.as_deref().is_some_and(|model| {
            store.aggregates.iter().any(|aggregate| aggregate.enabled && aggregate.public_model_id == model)
        });
    let segments: Vec<&str> = if codex_aggregate {
        ["aggregate", "v1"].into_iter().chain(segments[2..].iter().copied()).collect()
    } else {
        segments
    };
    if segments.first() == Some(&"aggregate") {
        let ingress =
            required_protocol.ok_or((StatusCode::NOT_FOUND, "unsupported aggregate endpoint"))?;
        let catalog = super::models_dev::load(&context.app_data_dir);
        let body = if codex_create {
            let converted = codex_compat::convert_official_compactions(
                &context, codex_auth.clone(), &headers, body.as_ref(),
            ).await;
            let body = if remote_compaction { codex_compat::summarization_request(body)? } else { body };
            codex_compat::prepare_relay(body, &converted)?
        } else {
            body
        };
        let result = aggregate::execute(
            &context,
            &store,
            &catalog,
            &method,
            &segments,
            query.as_deref(),
            &headers,
            body,
            ingress,
            requested_model.as_deref(),
            one_m_context,
            None,
            client_source,
            trace.clone(),
        )
        .await;
        let result = match result {
            Ok(response) if codex_create && remote_compaction => {
                codex_compat::compaction_response(response).await
            }
            other => other,
        };
        if let Err((status, message)) = &result {
            trace.reject(status.as_u16(), message);
        }
        return result;
    }
    if let Some(snapshot) = &desktop {
        trace.identity(&request_diagnostics::RequestIdentity {
            client_source: Some("claudeDesktop".into()),
            route_revision: Some(snapshot.route_revision),
            ..Default::default()
        });
    } else if let Some(source) = client_source {
        trace.identity(&request_diagnostics::RequestIdentity {
            client_source: Some(source.into()),
            ..Default::default()
        });
    }
    let result = async {
        // Identify ordinary provider routes for connection identity protection.
        // Codex always resends its complete history over HTTP, so each request
        // may pick a new destination by model; it needs no session binding.
        let provider_route = match segments.as_slice() {
            ["codex", "v1", ..] => false,
            ["p", _, "v1", ..] | ["v1", ..] => true,
            _ => false,
        };
        let affinity_key = (provider_route && (inference_create || compact || response_resource))
            .then(|| {
                session_affinity::key(
                    &context.store_path,
                    &headers,
                    requested_model.as_deref().unwrap_or_default(),
                )
            })
            .flatten();
        let _decision = if let Some(key) = &affinity_key {
            Some(session_affinity::decision_lock(key).await)
        } else {
            None
        };
        let binding = affinity_key.as_deref().and_then(session_affinity::bound);
        let continuation = compact || (inference_create && session_affinity::continuation(body.as_deref()));
        let fresh_turn = inference_create && session_affinity::fresh_turn(body.as_deref());
        // A fresh complete history may select the currently configured channel.
        // Continuations validate the established connection before any dispatch.
        let selected_binding = if fresh_turn { None } else { binding.as_ref() };
        let desktop_alias = desktop.as_ref().and_then(|_| raw_requested_model(body.as_deref()))
            .map(|alias| alias.trim().to_owned());
        let desktop_route = if let Some(snapshot) = &desktop {
            let alias = desktop_alias.as_deref()
                .ok_or((StatusCode::BAD_REQUEST, "Desktop model alias is required"))?;
            let with_marker = (one_m_beta && !strip_claude_one_m_marker(alias).1)
                .then(|| format!("{alias}[1m]"));
            Some(super::claude_desktop::gateway::resolve_message(snapshot, &store,
                desktop_catalog.as_deref().expect("Desktop request catalog"), with_marker.as_deref().unwrap_or(alias))
                .map_err(|_| (StatusCode::BAD_REQUEST, "Desktop route is unknown or unavailable"))?)
        } else { None };
        let one_m_context = desktop_route.as_ref().map(|route| route.view.one_m).unwrap_or(one_m_context);
        if let Some(route) = desktop_route.as_ref().filter(|route| route.aggregate_id.is_some()) {
            if let Some(count_body) = desktop_count_body.as_deref() {
                // An aggregate may select a different protocol/member for each
                // inference request, so a count from an auxiliary member would
                // not describe the route that handles the next turn.
                return Ok(desktop_count_tokens_response(count_body));
            }
            let snapshot = desktop.as_ref().expect("Desktop aggregate snapshot");
            return aggregate::execute(&context, &store,
                desktop_catalog.as_deref().expect("Desktop catalog"), &method, &segments,
                query.as_deref(), &headers, body.clone(), router::ModelProtocol::AnthropicMessages,
                Some(&route.choice.model_id), one_m_context,
                Some(aggregate::DesktopTarget { aggregate_id: route.aggregate_id.clone().expect("aggregate route"),
                    alias: route.choice.role_alias.clone(), config_identity: snapshot.paths_identity.clone(),
                    route_revision: snapshot.route_revision, one_m: one_m_context }),
                Some("claudeDesktop"), trace.clone()).await;
        }
        let identity = desktop_route.as_ref().map(|route| request_diagnostics::RequestIdentity {
            client_source: Some("claudeDesktop".into()),
            requested_model: desktop_alias.clone(),
            resolved_model: Some(route.choice.model_id.clone()),
            route_revision: desktop.as_ref().map(|s| s.route_revision),
        }).or_else(|| client_source.map(|source| request_diagnostics::RequestIdentity {
            client_source: Some(source.into()),
            requested_model: gateway_model.clone().or_else(|| requested_model.clone()),
            ..Default::default()
        }));
        if let Some(identity) = &identity { trace.identity(identity); }
        let (mut destination, selection_reason) = if let Some(route) = &desktop_route {
            let provider = route.provider.as_ref().expect("ordinary Desktop provider");
            let upstream_protocol = provider.protocol_for_model(&route.choice.model_id);
            let translation = protocol::Translation::between(router::ModelProtocol::AnthropicMessages, upstream_protocol);
            (Destination::Provider { provider: Box::new(provider.clone()),
                model_override: Some(route.choice.model_id.clone()), upstream_protocol, translation }, "desktop_role_mapping".into())
        } else { resolve_destination_explain(
            &store,
            &segments,
            required_protocol,
            requested_model.as_deref(),
            selected_binding,
        )? };
        if let Some(count_body) = desktop_count_body.as_deref() {
            let native_messages = matches!(
                &destination,
                Destination::Provider {
                    upstream_protocol: router::ModelProtocol::AnthropicMessages,
                    ..
                }
            );
            if !native_messages {
                return Ok(desktop_count_tokens_response(count_body));
            }
        }
        let command_selected = match &destination {
            Destination::Provider { provider, .. } => hosted_search::Plan::command_enabled(provider,&store),
            _ => false,
        };
        hosted_search::Plan::command_responses_search_to_chat(&mut destination, body.as_deref(), command_selected);
        let request_catalog = desktop_catalog.clone()
            .unwrap_or_else(|| super::models_dev::load(&context.app_data_dir));
        if one_m_context
            && desktop.is_none() {
            validate_one_m_capacity(&context, &store, &destination, &request_catalog)?;
        }
        trace.stage("build");
        // One Codex thread mixes official and relay turns. Give each destination
        // only the history it can read; relays get compaction emulated.
        let emulate_compaction =
            remote_compaction && matches!(destination, Destination::Provider { .. });
        let body = if !codex_create {
            body
        } else if matches!(destination, Destination::Official) {
            codex_compat::remember_official_model(&headers, requested_model.as_deref());
            codex_compat::prepare_official(body)?
        } else {
            let converted = codex_compat::convert_official_compactions(
                &context, codex_auth.clone(), &headers, body.as_ref(),
            ).await;
            let body = if emulate_compaction {
                codex_compat::summarization_request(body)?
            } else {
                body
            };
            codex_compat::prepare_relay(body, &converted)?
        };
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
        let attempt_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let suffix = upstream_suffix(&segments, &destination, required_protocol)?;
        if let Destination::Provider { upstream_protocol, .. } = &destination {
            let desktop_history_scope = desktop_route.as_ref().zip(desktop.as_ref()).map(|(route, snapshot)| {
                protocol::message_history::ClientScope {
                    client_source: "claudeDesktop", config_identity: &snapshot.paths_identity,
                    model_alias: &route.choice.role_alias, route_revision: snapshot.route_revision,
                }
            });
            // Resource operations read the established channel but do not carry
            // a create request; preserve their body, including an empty cancel.
            // upstream_suffix already rejects translated Responses resources.
            // Codex's sticky-routing token belongs to the official backend.
            let mut headers = headers.clone();
            headers.remove("x-codex-turn-state");
            let account = account_auth(&context, &destination).await?;
            let mut prepared = pipeline::request::prepare(pipeline::request::Input {
                context: &context, store: &store, catalog: &request_catalog,
                destination: &destination, suffix: &suffix, query: query.as_deref(), headers: &headers,
                body, ingress: required_protocol.unwrap_or(*upstream_protocol),
                resource: response_resource || desktop_count_body.is_some(), one_m: one_m_context, affinity_key: affinity_key.as_deref(),
                create: inference_create,
                client_scope: desktop_history_scope, identity: identity.unwrap_or_default(),
                trace: trace.clone(), selection_reason, retry_reason: None, attempt_id,
                account: account.as_ref(),
            }).await?;
            // GET /models remains diagnostic-only and does not count as inference.
            if required_protocol.is_none() { prepared.usage = None; }
            if desktop_count_body.is_none() {
                prepared.sent(&trace);
            }
            if inference_create && (fresh_turn || continuation) {
                if let Some(key) = &affinity_key {
                    if !session_affinity::bind(key, &destination) {
                        return Err((StatusCode::SERVICE_UNAVAILABLE,
                            "conversation routing capacity reached; wait for idle sessions to expire or start a new conversation"));
                    }
                }
            }
            if prepared.hosted.is_some() {
                drop(_decision);
                return prepared.search(trace.clone(), None).await;
            }
            let sent = match prepared.exchange.take() {
                // A source that answers the request itself (Cursor's Runs).
                Some(exchange) => Ok(exchange.run().await),
                None => {
                    let mut outbound = prepared.client.request(method.clone(), &prepared.url).headers(prepared.headers.clone());
                    if desktop_count_body.is_some() {
                        outbound = outbound.timeout(Duration::from_secs(2));
                    }
                    if let Some(sent) = &prepared.body { outbound = outbound.body(sent.clone()); }
                    outbound.send().await
                }
            };
            let response = match sent {
                Ok(response) => response,
                Err(error) => {
                    if let Some(count_body) = desktop_count_body.as_deref() {
                        return Ok(desktop_count_tokens_response(count_body));
                    }
                    if let Some(request) = prepared.usage {
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
            if desktop_count_body.is_some()
                && count_tokens_estimate_on_status(response.status())
            {
                return Ok(desktop_count_tokens_response(
                    desktop_count_body.as_deref().expect("count-token body"),
                ));
            }
            trace.status(response.status().as_u16());
            drop(_decision);
            let response = prepared.response(response, method.clone(), trace.clone(), None).await?;
            return if emulate_compaction {
                codex_compat::compaction_response(response).await
            } else {
                Ok(response)
            };
        }

        // The official endpoint keeps Codex's local account credentials and
        // original Responses body; provider attempt preparation does not own it.
        let upstream_protocol = router::ModelProtocol::Responses;
        let upstream_model = requested_model.clone().unwrap_or_default();
        let pricing = relay_usage::PricingSnapshot::unknown("official", &request_catalog.snapshot.revision);
        trace.attempt("official", "OpenAI", &upstream_model, "responses", route_trace::AttemptMetadata {
            attempt_id: attempt_id.clone(), catalog_revision: pricing.revision.clone(),
            match_kind: pricing.match_kind.clone(), pricing_status: pricing.status().into(),
            pricing_source: pricing.source.clone(), selection_reason, retry_reason: None,
        });
        let (url, attempt_headers) = official_target(
            &context, codex_auth, &suffix, query.as_deref(), headers,
        )?;
        let (sent, tool_adapter) = if response_resource { (body, Default::default()) }
            else { tool_schema::prepare(body, &upstream_model, upstream_protocol)? };
        let credential = request_diagnostics::CredentialSnapshot::from_header(
            &context.app_data_dir, "codexLocal", &format!("official|{}", url.split('?').next().unwrap_or(&url)),
            attempt_headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
            attempt_headers.get("chatgpt-account-id").and_then(|value| value.to_str().ok()),
        );
        trace.sent(sent.as_deref(), credential);
        let client = client_for(&context, &destination)?;
        let official = OfficialAttempt {
            client: &client, method: &method, url: &url, headers: &attempt_headers,
            trace: &trace, started,
        };
        let mut response = official.send(sent.clone()).await?;
        // Reasoning encrypted for another account or service cannot be replayed
        // (e.g. after an account switch). Retry once without it.
        if codex_create && response.status() == StatusCode::BAD_REQUEST {
            let status = response.status();
            let response_headers = response.headers().clone();
            let error = response.bytes().await.unwrap_or_default();
            let retry = sent.as_ref().and_then(codex_compat::without_reasoning)
                .filter(|_| codex_compat::rejected_reasoning(status, &error));
            response = match retry {
                Some(retry) => official.send(Some(retry)).await?,
                None => {
                    let mut rebuilt = hyper::Response::builder().status(status);
                    for (name, value) in &response_headers { rebuilt = rebuilt.header(name, value); }
                    reqwest::Response::from(rebuilt.body(error).expect("response parts"))
                }
            };
        }
        trace.status(response.status().as_u16());
        drop(_decision);
        relay_response(response, None, None, Vec::new(),
                trace.clone(),
                upstream_protocol,
                upstream_model,
                attempt_id, None, tool_adapter, Default::default(), Default::default(), None).await
    }
    .await;
    if let Err((status, message)) = &result {
        trace.reject(status.as_u16(), message);
    }
    result
}

struct OfficialAttempt<'a> {
    client: &'a Client,
    method: &'a hyper::Method,
    url: &'a str,
    headers: &'a HeaderMap,
    trace: &'a route_trace::Trace,
    started: std::time::Instant,
}

impl OfficialAttempt<'_> {
    async fn send(&self, body: Option<Bytes>) -> ForwardResult<reqwest::Response> {
        let mut outbound = self.client.request(self.method.clone(), self.url).headers(self.headers.clone());
        if let Some(body) = body {
            outbound = outbound.body(body);
        }
        outbound.send().await.map_err(|error| {
            let reason = if error.is_timeout() { "timeout" } else { "transport_error" };
            self.trace.failed_attempt("connect", reason, self.started.elapsed().as_millis() as u64);
            self.trace.fail("connect", reason);
            (StatusCode::BAD_GATEWAY, "cannot reach the provider")
        })
    }
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
    struct ModelOnly { model: Option<String>,
    }
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
    binding: Option<&session_affinity::Binding>,
) -> ForwardResult<Destination> {
    resolve_destination_explain(store, segments, required_protocol, requested_model, binding)
        .map(|(destination, _)| destination)
}

fn resolve_destination_explain(
    store: &router::Store,
    segments: &[&str],
    required_protocol: Option<router::ModelProtocol>,
    requested_model: Option<&str>,
    binding: Option<&session_affinity::Binding>,
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
                || !session_affinity::agrees(binding, &destination)
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
            // A kind turned off for Codex is not reached by an old selection either.
            Some((provider_id, _))
                if store.providers.iter().any(|provider| {
                    provider.id == provider_id && !router::codex_sources(store).admits_provider(provider)
                }) =>
            {
                return Err((
                    StatusCode::FORBIDDEN,
                    "this kind of model is turned off for Codex; select another model",
                ));
            }
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
            _ if store.router.accountless => {
                return Err((
                    StatusCode::FORBIDDEN,
                    "official models are disabled in accountless mode; select a relay or aggregate model",
                ));
            }
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

fn codex_request_auth(
    headers: &HeaderMap,
    local_token: &str,
) -> Option<(HeaderValue, Option<HeaderValue>)> {
    if local_token.is_empty()
        || !token_matches(headers.get(router::LOCAL_AUTH_HEADER)?.to_str().ok()?, local_token,
        )
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
        return token.to_str().ok().is_some_and(|candidate| token_matches(candidate, bearer_token));
    }
    headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|candidate| token_matches(candidate, bearer_token))
        || headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|candidate| token_matches(candidate, bearer_token))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::router::{Provider, ProviderInput, RouterSettings, Store};
    use crate::platform::paths::CodexPaths;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn upstream_errors_are_reshaped_for_the_caller() {
        let sse: Value = serde_json::from_str(&translated_error_body(router::ModelProtocol::AnthropicMessages, StatusCode::BAD_REQUEST,
            b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"bad search\"}}\n\n")).unwrap();
        assert_eq!(sse["error"]["message"], "bad search");
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
    fn all_stream_paths_report_idle_timeout_without_a_success_terminal() {
        let _lock = global_router_test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        let service = service_fn(|_: Request<Incoming>| async {
                            let first = futures_util::stream::once(async {
                                Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(
                                    b": heartbeat\n\n",
                                )))
                            });
                            Ok::<_, std::convert::Infallible>(
                                Response::builder()
                                    .header(CONTENT_TYPE, "text/event-stream")
                                    .body(StreamBody::new(
                                        first.chain(futures_util::stream::pending()),
                                    ))
                                    .unwrap(),
                            )
                        });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(socket), service)
                            .await;
                    });
                }
            });
            for protocol in [
                router::ModelProtocol::Responses,
                router::ModelProtocol::ChatCompletions,
                router::ModelProtocol::AnthropicMessages,
            ] {
                for path in 0..3 {
                    let runtime = crate::core::aggregation::runtime::Runtime::default();
                    let candidate = crate::core::aggregation::selector::Candidate {
                        member_id: "timeout".into(),
                        provider_id: "fixture".into(),
                        actual_model_id: "fixture".into(),
                        target_key: "timeout-target".into(),
                        priority: 0,
                        weight: 1,
                    };
                    for failure_index in 1..=3 {
                        let lease = runtime.reserve(&candidate).unwrap();
                        let source = Client::builder()
                            .no_proxy()
                            .read_timeout(Duration::from_millis(50))
                            .build()
                            .unwrap()
                            .get(format!("http://{address}"))
                            .send()
                            .await
                            .unwrap();
                        let trace = route_trace::Trace::begin("timeout-fixture", None);
                        let bytes = match path {
                            0 => {
                                let stream = observed_stream(
                                    source,
                                    trace,
                                    None,
                                    protocol,
                                    "fixture".into(),
                                    "fixture-attempt".into(),
                                    true,
                                    Some(lease.health()),
                                )
                                .map(|b| b.map(Frame::data));
                                let frames: Vec<_> = tokio::time::timeout(
                                    Duration::from_secs(2),
                                    sse_keepalive(stream, Duration::from_millis(10)).collect(),
                                )
                                .await
                                .unwrap();
                                let bytes = frames
                                    .into_iter()
                                    .map(|f| f.unwrap().into_data().unwrap())
                                    .collect::<Vec<_>>()
                                    .concat();
                                assert!(String::from_utf8_lossy(&bytes).contains(": keepalive"));
                                bytes
                            }
                            1 => adapted_native_stream(
                                source,
                                trace,
                                None,
                                protocol,
                                "fixture".into(),
                                "fixture-attempt".into(),
                                Default::default(),
                                Some(lease.health()),
                            )
                            .collect()
                            .await
                            .unwrap()
                            .to_bytes()
                            .to_vec(),
                            _ => {
                                let from = if protocol == router::ModelProtocol::AnthropicMessages {
                                    router::ModelProtocol::Responses
                                } else {
                                    router::ModelProtocol::AnthropicMessages
                                };
                                translated_stream(
                                    source,
                                    protocol::Translation { from, to: protocol },
                                    None,
                                    Vec::new(),
                                    trace,
                                    protocol,
                                    "fixture".into(),
                                    "fixture-attempt".into(),
                                    None,
                                    Default::default(),
                                    Default::default(),
                                    Default::default(),
                                    Some(lease.health()),
                                )
                                .unwrap()
                                .collect()
                                .await
                                .unwrap()
                                .to_bytes()
                                .to_vec()
                            }
                        };
                        let text = String::from_utf8(bytes).unwrap();
                        let failure_event =
                            if path == 2 && protocol == router::ModelProtocol::AnthropicMessages {
                                "event: response.failed"
                            } else if path != 2 && protocol == router::ModelProtocol::ChatCompletions {
                                "\"error\""
                            } else {
                                "event: error"
                            };
                        assert!(
                            text.contains(failure_event) && text.contains("upstream_stream_timeout"),
                            "protocol={protocol:?}, path={path}, body={text}"
                        );
                        assert!(
                            !text.contains("message_stop")
                                && !text.contains("[DONE]")
                                && !text.contains("event: response.completed")
                        );
                        assert_eq!(
                            runtime.should_switch(&candidate.target_key),
                            failure_index == 3,
                            "all forwarding paths count each actual timeout once"
                        );
                    }
                }
            }
            server.abort();
        });
    }

    #[test]
    fn native_eof_requires_real_terminal_and_separates_error_frames() {
        let _lock = global_router_test_lock();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::ChatCompletions, router::ModelProtocol::AnthropicMessages] {
                let start = match protocol {
                    router::ModelProtocol::Responses => "data: {\"type\":\"response.created\",\"response\":{\"model\":\"fixture\"}}",
                    router::ModelProtocol::ChatCompletions => "data: {\"model\":\"fixture\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}",
                    router::ModelProtocol::AnthropicMessages => "data: {\"type\":\"message_start\",\"message\":{\"model\":\"fixture\"}}",
                };
                let terminal = match protocol {
                    router::ModelProtocol::Responses => "data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}",
                    router::ModelProtocol::ChatCompletions => "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]",
                    router::ModelProtocol::AnthropicMessages => "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\ndata: {\"type\":\"message_stop\"}",
                };
                let large_content = "x".repeat(300 * 1024);
                let large_value = match protocol {
                    router::ModelProtocol::Responses => json!({"type":"response.output_text.delta","delta":large_content}),
                    router::ModelProtocol::ChatCompletions => json!({"choices":[{"delta":{"content":large_content},"finish_reason":null}]}),
                    router::ModelProtocol::AnthropicMessages => json!({"type":"content_block_delta","delta":{"type":"text_delta","text":large_content}}),
                };
                let large_start = format!("data: {large_value}\n\n");
                let mut fixtures: Vec<Bytes> = vec![
                    start.to_owned(),
                    format!("{start}\n\n"),
                    format!("{start}\n\n{terminal}"),
                    format!("{start}\n\ndata: [DONE]\n\n"),
                    large_start.clone(),
                    format!("{large_start}{terminal}\n\n"),
                    "\u{feff}data: [DONE]\n\n".to_owned(),
                    format!("\u{feff}{terminal}\n\n"),
                ].into_iter().map(Bytes::from).collect();
                fixtures.push(Bytes::from_static(b": \xff\n\ndata: [DONE]\n\n"));
                let fixtures = Arc::new(fixtures);
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let address = listener.local_addr().unwrap();
                let server = tokio::spawn(async move {
                    while let Ok((socket, _)) = listener.accept().await {
                        let fixtures = fixtures.clone();
                        tokio::spawn(async move {
                            let service = service_fn(move |request: Request<Incoming>| {
                                let index: usize = request.uri().path().trim_start_matches('/').parse().unwrap();
                                let body = fixtures[index].clone();
                                async move {
                                    Ok::<_, std::convert::Infallible>(Response::builder()
                                        .header(CONTENT_TYPE, "text/event-stream")
                                        .body(Full::new(Bytes::from(body))).unwrap())
                                }
                            });
                            let _ = http1::Builder::new().serve_connection(TokioIo::new(socket), service).await;
                        });
                    }
                });
                for index in 0..9 {
                    for path in 0..2 {
                        let source = Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap()
                            .get(format!("http://{address}/{index}")).send().await.unwrap();
                        let trace = route_trace::Trace::begin("eof-fixture", None);
                        let bytes = if path == 0 {
                            observed_stream(source, trace, None, protocol, "fixture".into(), "eof-attempt".into(), true, None)
                                .collect::<Vec<_>>().await.into_iter().map(Result::unwrap).collect::<Vec<_>>().concat()
                        } else {
                            adapted_native_stream(source, trace, None, protocol, "fixture".into(), "eof-attempt".into(), Default::default(), None)
                                .collect().await.unwrap().to_bytes().to_vec()
                        };
                        let text = String::from_utf8(bytes).unwrap();
                        let errors: Vec<Value> = text.split("\n\n").filter_map(|frame| {
                            frame.lines().find_map(|line| line.strip_prefix("data: "))
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                        }).filter(|value| !value["error"].is_null() || value["type"] == "error").collect();
                        if matches!(index, 2 | 5 | 7) {
                            assert!(errors.is_empty(), "valid incomplete response: {protocol:?}/{path}/{index}");
                        } else {
                            assert_eq!(errors.len(), 1, "{protocol:?}/{path}/{index}");
                            let code = if index == 8 { "invalid_stream_frame" } else { "upstream_stream_truncated" };
                            assert!(text.contains(code), "{protocol:?}/{path}/{index}");
                            if matches!(index, 3 | 6 | 8) {
                                assert!(!text.contains("[DONE]"), "an SDK must see the error before any terminal marker: {text}");
                            }
                        }
                    }
                }
                server.abort();
            }
        });
    }

    #[test]
    fn official_messages_target_uses_gateway_auth_and_selected_protocol_root() {
        for (base, expected, bearer) in [
            ("https://api.novita.ai/v3/openai", "https://api.novita.ai/anthropic/v1/messages", true,
            ),
            ("https://api.xiaomimimo.com/v1", "https://api.xiaomimimo.com/anthropic/v1/messages", true,
            ),
            ("https://api.z.ai/api/coding/paas/v4", "https://api.z.ai/api/anthropic/v1/messages", true,
            ),
            ("https://openrouter.ai/api/v1", "https://openrouter.ai/api/v1/messages", true,
            ),
            ("https://api.siliconflow.cn/v1", "https://api.siliconflow.cn/v1/messages", true,
            ),
            ("https://relay.invalid/custom/v1", "https://relay.invalid/custom/v1/messages", false,
            ),
        ] {
            let provider = router::Provider { base_url: base.into(), api_key: "fixture-upstream-key".into(), ..Default::default() };
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer local-key"));
            headers.insert("x-api-key", HeaderValue::from_static("local-key"));
            headers.insert("anthropic-beta", HeaderValue::from_static("fixture-beta"));
            let (url, headers) = provider_target(&provider, router::ModelProtocol::AnthropicMessages, None, "messages", None, headers,
            ).unwrap();
            assert_eq!(url, expected);
            if bearer {
                assert_eq!(headers[AUTHORIZATION], "Bearer fixture-upstream-key");
                assert!(!headers.contains_key("x-api-key"));
            } else {
                assert_eq!(headers["x-api-key"], "fixture-upstream-key");
                assert!(!headers.contains_key(AUTHORIZATION));
            }
            assert_eq!(headers["anthropic-version"], "2023-06-01");
            assert_eq!(headers["anthropic-beta"], "fixture-beta");
        }
    }

    #[test]
    fn sse_keepalive_preserves_data_and_ends_at_source_eof() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let source = futures_util::stream::once(async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(b"data: final\n\n",
                )))
            });
            let frames: Vec<_> = tokio::time::timeout(Duration::from_secs(2), sse_keepalive(source, Duration::from_millis(5)).collect(),
            ).await.unwrap();
            assert!(frames.len() > 1);
            let bytes: Vec<_> = frames.into_iter().map(|f| f.unwrap().into_data().unwrap()).collect();
            assert_eq!(bytes.last().unwrap(), b"data: final\n\n".as_slice());
            assert!(bytes[..bytes.len()-1].iter().all(|b| b.as_ref() == b": keepalive\n\n"));
        });
    }

    #[test]
    fn delayed_native_bom_after_keepalive_does_not_hide_the_first_event() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let wire = Bytes::from_static(b"\xef\xbb\xbfdata: {\"type\":\"response.completed\"}\n\n");
            let release = Arc::new(tokio::sync::Notify::new());
            let upstream_release = release.clone();
            let upstream_wire = wire.clone();
            let source = futures_util::stream::once(async move {
                upstream_release.notified().await;
                Ok::<_, std::convert::Infallible>(Frame::data(upstream_wire))
            });
            let mut stream = Box::pin(sse_keepalive(source, Duration::from_millis(1)));
            let heartbeat = tokio::time::timeout(Duration::from_secs(2), stream.next()).await.unwrap().unwrap().unwrap().into_data().unwrap();
            assert_eq!(heartbeat, b": keepalive\n\n".as_slice());
            release.notify_one();
            let event = tokio::time::timeout(Duration::from_secs(2), stream.next()).await.unwrap().unwrap().unwrap().into_data().unwrap();
            assert_eq!(event, wire.slice(3..));
            assert!(stream.next().await.is_none());

            let source = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(Frame::data(wire.clone()))
            });
            let mut stream = Box::pin(sse_keepalive(source, Duration::from_secs(1)));
            let event = stream.next().await.unwrap().unwrap().into_data().unwrap();
            assert!(event.starts_with(b"\xef\xbb\xbf"));
            assert!(stream.next().await.is_none());
        });
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
    fn failed_port_change_keeps_the_existing_listener() {
        let _lock = global_router_test_lock();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                stop_unchecked();
            }
        }
        let _reset = Reset;
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let first = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default())
                .await
                .unwrap();
            let lease = current_lease().unwrap();
            let occupied = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let second = occupied.local_addr().unwrap().port();
            assert_eq!(
                ensure_running(second, paths, ApiProxyConfigPayload::default())
                    .await
                    .unwrap_err(),
                "newapi.errors.portBusy"
            );
            assert_eq!(running_port(), Some(first));
            assert_eq!(current_lease(), Some(lease));
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, first))
                .await
                .unwrap();
        });
    }

    #[test]
    fn dead_accept_task_is_not_reported_online_and_can_rebind() {
        let _lock = global_router_test_lock();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                stop_unchecked();
            }
        }
        let _reset = Reset;
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default())
                .await
                .unwrap();
            let first = current_lease().unwrap();
            handle_slot()
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .accept_task
                .abort();
            tokio::time::timeout(Duration::from_secs(2), async {
                while is_running() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(running_port(), None);
            assert_eq!(current_lease(), None);
            assert_eq!(
                ensure_running(port, paths, ApiProxyConfigPayload::default())
                    .await
                    .unwrap(),
                port
            );
            assert!(is_running());
            assert_ne!(current_lease(), Some(first));
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
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
    fn native_provider_preparation_keeps_original_bytes_without_edits() {
        let bytes = Bytes::from_static(
            br#"{
  "model": "fixture", "stream": false, "opaque": { "signed": "unchanged" }
}"#,
        );
        for wire in [
            router::ModelProtocol::Responses,
            router::ModelProtocol::ChatCompletions,
            router::ModelProtocol::AnthropicMessages,
        ] {
            let destination = Destination::Provider {
                provider: Box::new(Provider::default()),
                model_override: Some("fixture".into()),
                upstream_protocol: wire,
                translation: None,
            };
            assert_eq!(
                upstream_body(&destination, Some(bytes.clone()))
                    .unwrap()
                    .body
                    .unwrap(),
                bytes
            );
        }
        let stream = Bytes::from_static(
            br#"{ "model": "fixture", "stream": true,
 "stream_options": { "include_usage": true, "opaque": 1 } }"#,
        );
        assert_eq!(
            prepare_relay_body(
                &stream,
                Some("fixture"),
                router::ModelProtocol::ChatCompletions
            )
            .unwrap(),
            stream
        );
        let changed =
            prepare_relay_body(&bytes, Some("wire-model"), router::ModelProtocol::Responses)
                .unwrap();
        let changed: Value = serde_json::from_slice(&changed).unwrap();
        assert_eq!(changed["model"], "wire-model");
        assert_eq!(changed["opaque"]["signed"], "unchanged");
    }

    #[test]
    fn ordinary_one_m_admission_uses_capacity_instead_of_egress_protocol() {
        let fixture = tempfile::tempdir().unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let context = ArcContext {
            bound_port: 0,
            store_path: fixture.path().join("providers.dat"),
            app_data_dir: fixture.path().to_owned(),
            codex_home: fixture.path().join("codex"),
            auth_path: fixture.path().join("auth.json"),
            outbound: Arc::new(RwLock::new(client.clone())),
            outbound_direct: client,
            official_base: None,
        };
        let catalog = super::super::models_dev::load(fixture.path());
        for wire in [
            router::ModelProtocol::Responses,
            router::ModelProtocol::ChatCompletions,
            router::ModelProtocol::AnthropicMessages,
        ] {
            let mut provider = Provider {
                context_window: Some(1_000_000),
                model: "fixture-model".into(),
                protocol: wire,
                ..Default::default()
            };
            let destination = |provider| Destination::Provider {
                provider: Box::new(provider),
                model_override: Some("fixture-model".into()),
                upstream_protocol: wire,
                translation: protocol::Translation::between(
                    router::ModelProtocol::AnthropicMessages,
                    wire,
                ),
            };
            assert!(validate_one_m_capacity(
                &context,
                &Store::default(),
                &destination(provider.clone()),
                &catalog
            )
            .is_ok());
            provider.context_window = Some(128_000);
            let error = validate_one_m_capacity(
                &context,
                &Store::default(),
                &destination(provider.clone()),
                &catalog,
            )
            .unwrap_err();
            assert_eq!(
                error,
                (
                    StatusCode::BAD_REQUEST,
                    "selected model does not support 1M context"
                )
            );
            // Existing user opt-in remains available when no capacity is known.
            provider.context_window = None;
            assert!(validate_one_m_capacity(
                &context,
                &Store::default(),
                &destination(provider),
                &catalog
            )
            .is_ok());
        }
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
        assert!(reverse.is_supported());
        let request = protocol::request_body(
            reverse,
            &json!({"model":"chat-model","messages":[{"role":"user","content":"hello"}],"max_tokens":32}),
        ).unwrap();
        assert_eq!(request.body["input"][0]["role"], "user");
        assert_eq!(request.body["max_output_tokens"], 32);
        assert!(request.body.get("messages").is_none());
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
                (Method::POST, vec!["chat", "completions"]),
            ] {
                let mut path = prefix.clone(); path.extend(suffix);
                assert!(validate_endpoint(&method, &path).is_ok());
            }
            for suffix in [vec!["files"], vec!["organization", "admin_keys"], vec!["fine_tuning", "jobs"], vec!["responses", "r", "unknown"],
            ] {
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
            let body = upstream_body(&destination, Some(body)).unwrap().body;
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
    fn codex_refuses_a_model_of_a_kind_turned_off_for_it() {
        let mut store = Store {
            providers: vec![Provider {
                id: "relay".into(),
                api_key: "synthetic-key".into(),
                model: "model".into(),
                enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let segments = ["codex", "v1", "responses"];
        let required = request_protocol(&segments);
        assert!(resolve_destination(&store, &segments, required, Some("relay::model"), None).is_ok());
        store.router.codex_sources = Some(router::ModelSources { relay: false, ..Default::default() });
        assert_eq!(
            resolve_destination(&store, &segments, required, Some("relay::model"), None).err().map(|error| error.0),
            Some(StatusCode::FORBIDDEN)
        );
        // Other namespaces keep their own rules.
        assert!(resolve_destination(&store, &["v1", "responses"], required, Some("relay::model"), None).is_ok());
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
                    .body
                    .unwrap(),
                body
            );
        }
    }

    #[test]
    fn accountless_rejects_official_create_compact_and_resource_routes() {
        let mut store = Store::default();
        store.router.accountless = true;
        for path in [
            "codex/v1/responses",
            "codex/v1/responses/compact",
            "codex/v1/responses/old-response",
            "codex/v1/responses/old-response/cancel",
            "codex/v1/responses/old-response/input_items",
        ] {
            let segments: Vec<_> = path.split('/').collect();
            for model in [None, Some("official-plus-model")] {
                let error =
                    resolve_destination(&store, &segments, request_protocol(&segments), model, None)
                        .err()
                        .expect("official route must be rejected");
                assert_eq!(error.0, StatusCode::FORBIDDEN, "{path}, {model:?}");
                assert!(error.1.contains("accountless"));
            }
        }
        store.router.accountless = false;
        assert!(matches!(
            resolve_destination(
                &store,
                &["codex", "v1", "responses"],
                Some(router::ModelProtocol::Responses),
                Some("official-plus-model"),
                None
            )
            .unwrap(),
            Destination::Official
        ));
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
            let (port_official, official_task) = echo_relay("official").await;
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
                store_path, app_data_dir, codex_home:fixture.path().to_owned(), auth_path:auth_path.clone(),
                outbound:Arc::new(RwLock::new(client.clone())), outbound_direct:client.clone(),
                official_base:Some(format!("http://127.0.0.1:{port_official}/v1/"))
            });
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (shutdown, receiver) = tokio::sync::watch::channel(false);
            let router = tokio::spawn(accept_loop(listener, receiver, context.clone()));
            let history = json!([
                {"role":"user","content":"Original question"},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Original answer"}]},
                {"type":"reasoning","summary":[{"type":"summary_text","text":"Retained summary"}],"encrypted_content":"fixture-opaque-reasoning"},
                {"type":"compaction","encrypted_content":"fixture-opaque-compaction"},
                {"type":"function_call","call_id":"call_fixture","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_fixture","output":"Retained tool result"},
                {"role":"user","content":"Continue the same conversation"}
            ]);
            // Switch official -> A -> B -> official -> A on one thread, including
            // native and legacy compaction URLs.
            // The pinned provider remains A throughout.
            for (prefix, endpoint, id, model, key, effort) in [
                ("codex/v1", "responses", "official", "model-a", "fixture-current-official-token", "none"),
                ("codex/v1", "responses", "relay-a", "model-a", "fixture-relay-a-key", "none"),
                ("codex/v1", "responses", "relay-b", "model-b", "fixture-relay-b-key", "none"),
                ("codex/v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "high"),
                ("v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "none"),
                ("p/relay-b/v1", "responses/compact", "relay-b", "model-b", "fixture-relay-b-key", "high"),
                ("codex/v1", "responses", "official", "model-a", "fixture-current-official-token", "none"),
                ("codex/v1", "responses", "relay-a", "model-a", "fixture-relay-a-key", "none"),
            ] {
                let mut body = json!({
                    "model":if id == "official" { model.to_owned() } else { format!("{id}::{model}") }, "input":history, "previous_response_id":"fixture-provider-response",
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
                if id == "official" {
                    assert_eq!(observed["headers"]["chatgpt-account-id"], "fixture-current-official-account");
                } else {
                    assert!(observed["headers"].get("chatgpt-account-id").is_none());
                }
                for header in ["x-api-key", "cookie", "proxy-authorization", router::LOCAL_AUTH_HEADER] {
                    assert!(observed["headers"].get(header).is_none(), "leaked {header}");
                }
                body["model"] = json!(model);
                if model == "model-b" && effort == "none" {
                    body["reasoning"].as_object_mut().unwrap().remove("effort");
                }
                // A relay gets every visible item; official-only encrypted state
                // is dropped or replaced by a readable notice.
                if prefix == "codex/v1" && endpoint == "responses" && id != "official" {
                    let mut portable = history.as_array().unwrap().clone();
                    portable.retain(|item| item["type"] != "reasoning");
                    portable[2] = json!({"type":"message","role":"user","content":[
                        {"type":"input_text","text":codex_compat::UNREADABLE_COMPACTION}]});
                    body["input"] = Value::Array(portable);
                }
                assert_eq!(observed["body"], body, "visible history and continuation id must survive model switches");
            }
            // Retained file credentials and live client credentials must both
            // stay unused when this same router is switched to accountless.
            store.router.accountless = true;
            crate::core::secret_file::write(&context.store_path, &serde_json::to_vec(&store).unwrap()).unwrap();
            let menu: Value = client.get(format!("http://127.0.0.1:{port}/codex/v1/models"))
                .header(router::LOCAL_AUTH_HEADER, "fixture-local-secret")
                .send().await.unwrap().json().await.unwrap();
            let slugs: Vec<_> = menu["data"].as_array().unwrap().iter().map(|entry| entry["id"].as_str().unwrap()).collect();
            assert_eq!(slugs, ["relay-a::model-a", "relay-b::model-b"]);
            for (method, suffix) in [
                (hyper::Method::POST, "responses"),
                (hyper::Method::POST, "responses/compact"),
                (hyper::Method::GET, "responses/old-official-response"),
                (hyper::Method::POST, "responses/old-official-response/cancel"),
                (hyper::Method::GET, "responses/old-official-response/input_items"),
            ] {
                let response = client.request(method, format!("http://127.0.0.1:{port}/codex/v1/{suffix}"))
                    .header(router::LOCAL_AUTH_HEADER, "fixture-local-secret")
                    .bearer_auth("fixture-current-official-token")
                    .header("chatgpt-account-id", "fixture-current-official-account")
                    .json(&json!({"model":"model-a","input":"Retained official selection"}))
                    .send().await.unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{suffix}");
                assert!(response.text().await.unwrap().contains("accountless mode"));
            }
            let response: Value = client.post(format!("http://127.0.0.1:{port}/codex/v1/responses"))
                .bearer_auth("fixture-local-secret")
                .json(&json!({"model":"relay-b::model-b","input":"Accountless relay request"}))
                .send().await.unwrap().json().await.unwrap();
            assert_eq!(response["relay"], "relay-b");
            assert_eq!(response["headers"]["authorization"], "Bearer fixture-relay-b-key");
            assert!(response["headers"].get("chatgpt-account-id").is_none());
            store.router.accountless = false;
            crate::core::secret_file::write(&context.store_path, &serde_json::to_vec(&store).unwrap()).unwrap();
            let response: Value = client.post(format!("http://127.0.0.1:{port}/codex/v1/responses"))
                .header(router::LOCAL_AUTH_HEADER, "fixture-local-secret")
                .bearer_auth("fixture-current-official-token")
                .header("chatgpt-account-id", "fixture-current-official-account")
                .json(&json!({"model":"model-a","input":"Official access restored"}))
                .send().await.unwrap().json().await.unwrap();
            assert_eq!(response["relay"], "official");
            assert_eq!(response["headers"]["authorization"], "Bearer fixture-current-official-token");
            assert_eq!(std::fs::read(&auth_path).unwrap(), auth);
            shutdown.send(true).unwrap();
            router.await.unwrap();
            relay_a.abort();
            relay_b.abort();
            official_task.abort();
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
            crate::core::model_catalog::seed_test_account_catalog(&paths.codex_home, &paths.app_data_dir,
                &serde_json::to_vec(&json!({"models":[{
                    "slug":"gpt-official-fixture", "display_name":"Official Fixture", "context_window":272000
                }]})).unwrap());
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
                codex_subagents: None,
                providers: vec![provider, chat_provider, anthropic_provider],
                aggregates: Vec::new(),
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
            let codex_trace = route_trace::recent().into_iter().find(|route|
                route.requested_model == "agentharbor_fixture::mock-model"
                    && route.client_source.as_deref() == Some("codex")
                    && route.status == Some(200)).expect("Codex request has a source-tagged trace");
            let codex_usage = relay_usage::load(&paths.app_data_dir).unwrap();
            assert!(codex_usage.recent.iter().any(|record|
                record.request_id == codex_trace.id
                    && record.identity.client_source.as_deref() == Some("codex")
                    && record.model == "mock-model"));

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
            config_request.target_id = TargetId::Pi;
            let preview = manager.preview(&store, &config_request).unwrap();
            manager.apply(&store, &config_request, &preview.revision).unwrap();
            manager.restore(TargetId::OpenCode).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(is_running(), "Pi independently keeps its local router alive");
            let response = client.post(format!("{base}/clients/pi/p/chat_fixture/v1/chat/completions"))
                .bearer_auth("router-test-token")
                .json(&json!({"model":"chat-model","messages":[{"role":"user","content":"Pi source test"}]}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200);
            response.bytes().await.unwrap();
            let pi_trace = route_trace::recent().into_iter().find(|route| {
                route.client_source.as_deref() == Some("pi") && route.status == Some(200)
            }).expect("Pi request has an independent source-tagged trace");
            assert!(relay_usage::load(&paths.app_data_dir).unwrap().recent.iter().any(|record| {
                record.request_id == pi_trace.id && record.identity.client_source.as_deref() == Some("pi")
                    && record.model == "chat-model"
            }));
            manager.restore(TargetId::Pi).unwrap();
            routing_runtime::reconcile(&paths).await.unwrap();
            assert!(!is_running(), "the router stops only when all clients release it");
            config_request.target_id = TargetId::OpenCode;
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
                source_home.join("newapi-connection.dat"),
                paths.app_data_dir.join("newapi-connection.dat"),
            )
            .unwrap();
            let source_settings = source_home.join("settings.json");
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
                    chat_search: None,
                    gateway_slug: None,
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
                    price_multiplier: None,
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
