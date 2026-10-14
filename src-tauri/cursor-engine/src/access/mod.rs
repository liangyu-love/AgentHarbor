//! The one local port Cursor reaches AgentHarbor through.
//!
//! ```text
//! Cursor IDE ── http.proxy ──▶ CONNECT host:443
//!                                 ├ *.cursor.sh → TLS with a local certificate → HTTP/1.1 ─┐
//!                                 └ any other   → plain tunnel (through the old proxy)      │
//! cursor-agent ── CURSOR_API_ENDPOINT ──▶ HTTP/1.1 or h2c ───────────────────────────────────┤
//!                                                                                           ▼
//!                                  Cursor API router: answered here, or forwarded to Cursor
//! ```
//! A request read inside a CONNECT carries the address it was meant for, so
//! anything not answered here goes on to exactly that Cursor server.
pub mod ca;
mod tunnel;

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use axum::body::Body;
use hyper::{body::Incoming, header, Method, Request, Response, StatusCode};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::{
    api::cursor::proxy::{is_cursor_host, UPSTREAM_URL_HEADER},
    host::{Egress, Host},
    Result,
};

pub use ca::{Authority, TrustState};

struct State {
    router: axum::Router,
    authority: Option<Arc<Authority>>,
    host: Host,
    port: u16,
}

/// The running front; dropping it does not stop it, `stop` does.
pub struct Front {
    address: SocketAddr,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Front {
    /// Listens on loopback at `port`, or any free port when it is taken or 0.
    pub async fn bind(port: u16, router: axum::Router, authority: Option<Arc<Authority>>, host: Host) -> Result<Self> {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => listener,
            Err(error) if port != 0 => {
                log::warn!("Cursor port {port} is unavailable ({error}); using a free one");
                TcpListener::bind(("127.0.0.1", 0)).await?
            }
            Err(error) => return Err(error.into()),
        };
        let address = listener.local_addr()?;
        let state = Arc::new(State { router, authority, host, port: address.port() });
        let shutdown = CancellationToken::new();
        let stopping = shutdown.clone();
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    () = stopping.cancelled() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { continue };
                let state = state.clone();
                let stopping = stopping.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request| handle(state.clone(), request));
                    let builder = auto::Builder::new(TokioExecutor::new());
                    let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
                    tokio::pin!(connection);
                    tokio::select! {
                        _ = connection.as_mut() => {}
                        () = stopping.cancelled() => connection.as_mut().graceful_shutdown(),
                    }
                });
            }
        });
        Ok(Self { address, shutdown, task })
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// `http://127.0.0.1:<port>`: the proxy for the IDE and the endpoint for the CLI.
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub async fn stop(self) {
        self.shutdown.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.task).await;
    }
}

async fn handle(state: Arc<State>, request: Request<Incoming>) -> std::result::Result<Response<Body>, Infallible> {
    if request.method() == Method::CONNECT {
        return Ok(connect(state, request).await);
    }
    let names_self = request.uri().authority().is_some_and(|authority| is_self(authority.as_str(), state.port));
    if request.uri().scheme().is_some() && !names_self {
        // An absolute address elsewhere is a plain-HTTP proxy request;
        // Cursor's servers are HTTPS only, so this only ever passes through.
        return Ok(forward_plain(&state, request).await);
    }
    // A direct call (cursor-agent). A browser page must not drive it: it
    // sends an Origin, and a rebound DNS name arrives under a foreign Host.
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| request.uri().authority().map(|authority| authority.as_str().to_owned()));
    if request.headers().contains_key(header::ORIGIN) || !host.as_deref().is_some_and(|host| is_self(host, state.port)) {
        return Ok(status(StatusCode::FORBIDDEN));
    }
    Ok(call(&state.router, request.map(Body::new)).await)
}

/// Whether `authority` (`host[:port]`) is this front on loopback.
fn is_self(authority: &str, port: u16) -> bool {
    let (host, given) = match authority.rsplit_once(':') {
        Some((host, given)) if !host.ends_with(']') || authority.starts_with('[') => (host, given.parse::<u16>().ok()),
        _ => (authority, None),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "127.0.0.1" | "localhost" | "::1") && given.is_none_or(|given| given == port)
}

async fn call(router: &axum::Router, request: Request<Body>) -> Response<Body> {
    match router.clone().oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

async fn connect(state: Arc<State>, request: Request<Incoming>) -> Response<Body> {
    let Some(authority) = request.uri().authority().cloned() else {
        return status(StatusCode::BAD_REQUEST);
    };
    let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_owned();
    let port = authority.port_u16().unwrap_or(443);
    let read = port == 443 && is_cursor_host(&host) && state.authority.is_some();
    if read {
        let config = match state.authority.as_ref().expect("checked").server_config(&host) {
            Ok(config) => config,
            Err(error) => {
                log::warn!("no TLS certificate for {host}: {error}");
                return status(StatusCode::BAD_GATEWAY);
            }
        };
        tokio::spawn(async move {
            let Ok(upgraded) = hyper::upgrade::on(request).await else { return };
            let acceptor = tokio_rustls::TlsAcceptor::from(config);
            let stream = match acceptor.accept(TokioIo::new(upgraded)).await {
                Ok(stream) => stream,
                Err(error) => {
                    // Usually Cursor not trusting the certificate yet.
                    log::warn!("TLS with Cursor for {host} failed: {error}");
                    return;
                }
            };
            let origin = host.clone();
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let state = state.clone();
                let origin = origin.clone();
                async move { Ok::<_, Infallible>(inside(&state, &origin, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .keep_alive(true)
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
        return status(StatusCode::OK);
    }
    let egress = upstream_egress(&state);
    let target = match tunnel::open(&host, port, &egress).await {
        Ok(target) => target,
        Err(error) => {
            log::debug!("tunnel to {host}:{port} failed: {error}");
            return status(StatusCode::BAD_GATEWAY);
        }
    };
    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(request).await else { return };
        let mut client = TokioIo::new(upgraded);
        let mut target = target;
        let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
    });
    status(StatusCode::OK)
}

/// A request read inside a CONNECT to `origin`.
async fn inside(state: &State, origin: &str, request: Request<Incoming>) -> Response<Body> {
    let mut request = request.map(Body::new);
    let target = format!(
        "https://{origin}{}",
        request.uri().path_and_query().map_or("/", |path| path.as_str())
    );
    if let Ok(value) = target.parse() {
        request.headers_mut().insert(UPSTREAM_URL_HEADER, value);
    }
    call(&state.router, request).await
}

async fn forward_plain(state: &State, request: Request<Incoming>) -> Response<Body> {
    let client = match crate::network::client_builder(&upstream_egress(state)).and_then(|builder| Ok(builder.build()?)) {
        Ok(client) => client,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    headers.remove(header::HOST);
    headers.remove("proxy-connection");
    headers.remove(header::PROXY_AUTHORIZATION);
    let upstream = client
        .request(parts.method, parts.uri.to_string())
        .headers(headers)
        .body(reqwest::Body::wrap_stream(http_body_util::BodyDataStream::new(body)))
        .send()
        .await;
    match upstream {
        Ok(upstream) => {
            let code = upstream.status();
            let headers = upstream.headers().clone();
            let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
            *response.status_mut() = code;
            *response.headers_mut() = headers;
            response.headers_mut().remove(header::TRANSFER_ENCODING);
            response
        }
        Err(_) => status(StatusCode::BAD_GATEWAY),
    }
}

/// How tunnels leave this machine; never back into the front itself.
fn upstream_egress(state: &State) -> Egress {
    match state.host.egress() {
        Egress::Proxy(url) if points_at(&url, state.port) => Egress::Direct,
        egress => egress,
    }
}

fn points_at(proxy: &str, port: u16) -> bool {
    url::Url::parse(proxy).is_ok_and(|url| {
        url.port_or_known_default() == Some(port)
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]" | "::1"))
    })
}

fn status(code: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = code;
    response
}

#[cfg(test)]
mod tests {
    use axum::{http::HeaderMap, routing::get};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn echo_router() -> axum::Router {
        axum::Router::new().route(
            "/ping",
            get(|headers: HeaderMap| async move {
                headers.get(UPSTREAM_URL_HEADER).and_then(|value| value.to_str().ok()).unwrap_or("direct").to_owned()
            }),
        )
    }

    async fn front(directory: &std::path::Path) -> (Front, Arc<Authority>) {
        let authority = Arc::new(Authority::load_or_create(directory).unwrap());
        let front = Front::bind(0, echo_router(), Some(authority.clone()), Host::new()).await.unwrap();
        (front, authority)
    }

    #[tokio::test]
    async fn cursor_hosts_are_read_with_a_certificate_cursor_trusts() {
        let directory = tempfile::tempdir().unwrap();
        let (front, authority) = front(directory.path()).await;
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(front.url()).unwrap())
            .add_root_certificate(reqwest::Certificate::from_pem(authority.certificate_pem().as_bytes()).unwrap())
            .build()
            .unwrap();
        for host in ["api2.cursor.sh", "agentn.global.api5.cursor.sh"] {
            let body = client.get(format!("https://{host}/ping?x=1")).send().await.unwrap().text().await.unwrap();
            assert_eq!(body, format!("https://{host}/ping?x=1"));
        }
        front.stop().await;
    }

    #[tokio::test]
    async fn the_cli_talks_to_the_port_directly() {
        let directory = tempfile::tempdir().unwrap();
        let (front, _) = front(directory.path()).await;
        let body = reqwest::Client::builder().no_proxy().build().unwrap()
            .get(format!("{}/ping", front.url())).send().await.unwrap().text().await.unwrap();
        assert_eq!(body, "direct");
        front.stop().await;
    }

    #[tokio::test]
    async fn other_hosts_pass_through_a_plain_tunnel() {
        let directory = tempfile::tempdir().unwrap();
        let (front, _) = front(directory.path()).await;
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut buffer = [0u8; 5];
            stream.read_exact(&mut buffer).await.unwrap();
            stream.write_all(&buffer).await.unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", front.port())).await.unwrap();
        client
            .write_all(format!("CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
        client.write_all(b"hello").await.unwrap();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"hello");
        front.stop().await;
    }

    #[tokio::test]
    async fn browser_pages_cannot_drive_the_port() {
        let directory = tempfile::tempdir().unwrap();
        let (front, _) = front(directory.path()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let from_page = client.get(format!("{}/ping", front.url())).header("origin", "https://evil.example").send().await.unwrap();
        assert_eq!(from_page.status(), 403);
        let rebound = client.get(format!("{}/ping", front.url())).header("host", "evil.example").send().await.unwrap();
        assert_eq!(rebound.status(), 403);
        let local = client.get(format!("http://localhost:{}/ping", front.port())).send().await.unwrap();
        assert_eq!(local.status(), 200);
        front.stop().await;
    }

    #[test]
    fn the_front_knows_its_own_address() {
        assert!(is_self("127.0.0.1:4567", 4567));
        assert!(is_self("localhost:4567", 4567));
        assert!(is_self("[::1]:4567", 4567));
        assert!(is_self("127.0.0.1", 4567));
        assert!(!is_self("127.0.0.1:80", 4567));
        assert!(!is_self("evil.example:4567", 4567));
    }

    #[test]
    fn a_proxy_that_is_the_front_itself_is_never_used() {
        assert!(points_at("http://127.0.0.1:4567", 4567));
        assert!(points_at("http://localhost:4567/", 4567));
        assert!(!points_at("http://127.0.0.1:7890", 4567));
    }
}
