//! Credential-free network diagnostics; this does not fetch account quotas.
use super::models::{
    ApiProxyConfigPayload, ApiProxyDetectPayload, ApiProxyMode, ApiProxyTestPayload, CoreError,
};
use std::time::Duration;

pub fn sanitize_proxy_config(
    config: &ApiProxyConfigPayload,
) -> Result<ApiProxyConfigPayload, CoreError> {
    if config.mode != ApiProxyMode::Manual {
        return Ok(ApiProxyConfigPayload {
            mode: config.mode.clone(),
            url: None,
        });
    }
    let text = config
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::InvalidData("Manual proxy mode requires a proxy URL".into()))?;
    let url = reqwest::Url::parse(text)
        .map_err(|_| CoreError::InvalidData("Invalid proxy URL".into()))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(CoreError::InvalidData("Unsupported proxy scheme".into()));
    }
    if url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(CoreError::InvalidData("Invalid proxy URL".into()));
    }
    Ok(ApiProxyConfigPayload {
        mode: ApiProxyMode::Manual,
        url: Some(text.to_owned()),
    })
}

pub(crate) fn configure_async_client(
    builder: reqwest::ClientBuilder,
    config: &ApiProxyConfigPayload,
) -> Result<reqwest::ClientBuilder, CoreError> {
    let config = sanitize_proxy_config(config)?;
    // Relays and vendor APIs stay on HTTP/1.1; only Cursor's duplex stream
    // builds its own HTTP/2 client.
    let builder = builder.http1_only();
    Ok(match config.mode {
        ApiProxyMode::System => builder.no_proxy().proxy(system_proxy()),
        ApiProxyMode::Direct => builder.no_proxy(),
        ApiProxyMode::Manual => builder.no_proxy().proxy(
            reqwest::Proxy::all(config.url.as_deref().unwrap())
                .map_err(|_| CoreError::InvalidData("Invalid proxy URL".into()))?,
        ),
    })
}

pub(crate) fn configure_blocking_client(
    builder: reqwest::blocking::ClientBuilder,
    config: &ApiProxyConfigPayload,
) -> Result<reqwest::blocking::ClientBuilder, CoreError> {
    let config = sanitize_proxy_config(config)?;
    let builder = builder.http1_only();
    Ok(match config.mode {
        ApiProxyMode::System => builder.no_proxy().proxy(system_proxy()),
        ApiProxyMode::Direct => builder.no_proxy(),
        ApiProxyMode::Manual => builder.no_proxy().proxy(
            reqwest::Proxy::all(config.url.as_deref().unwrap())
                .map_err(|_| CoreError::InvalidData("Invalid proxy URL".into()))?,
        ),
    })
}

/// The system proxy, looked up again every few seconds rather than once when
/// a client is built: proxy software started, stopped or moved after a
/// long-lived client (the router's) was built takes effect without a restart.
pub(crate) fn system_proxy() -> reqwest::Proxy {
    reqwest::Proxy::custom(|url: &reqwest::Url| system_route(url))
}

fn system_route(url: &reqwest::Url) -> Option<reqwest::Url> {
    use hyper_util::client::proxy::matcher::Matcher;
    struct Routes {
        read: std::time::Instant,
        matcher: Matcher,
    }
    static ROUTES: std::sync::Mutex<Option<Routes>> = std::sync::Mutex::new(None);
    const FRESH: Duration = Duration::from_secs(2);
    let uri = url.as_str().parse::<hyper::Uri>().ok()?;
    let mut routes = ROUTES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if routes.as_ref().is_none_or(|routes| routes.read.elapsed() >= FRESH) {
        *routes = Some(Routes {
            read: std::time::Instant::now(),
            matcher: system_matcher(),
        });
    }
    proxy_url(&routes.as_ref()?.matcher.intercept(&uri)?)
}

/// Proxy variables first, then the registry, as reqwest's own system proxy
/// does; but `ProxyServer`'s per-protocol form (`http=…;https=…`), which
/// reqwest takes for one invalid address and silently ignores, is read too.
#[cfg(target_os = "windows")]
fn system_matcher() -> hyper_util::client::proxy::matcher::Matcher {
    let system = crate::platform::proxy::wininet_proxy().unwrap_or_default();
    matcher_from(|name| std::env::var(name).ok(), system)
}

#[cfg(not(target_os = "windows"))]
fn system_matcher() -> hyper_util::client::proxy::matcher::Matcher {
    hyper_util::client::proxy::matcher::Matcher::from_system()
}

#[cfg(any(target_os = "windows", test))]
fn matcher_from(
    env: impl Fn(&str) -> Option<String>,
    system: crate::platform::proxy::SystemProxy,
) -> hyper_util::client::proxy::matcher::Matcher {
    let env = |names: [&str; 2]| {
        names
            .into_iter()
            .find_map(|name| env(name).filter(|value| !value.trim().is_empty()))
    };
    hyper_util::client::proxy::matcher::Matcher::builder()
        .all(env(["ALL_PROXY", "all_proxy"]).unwrap_or_default())
        .http(env(["HTTP_PROXY", "http_proxy"]).or(system.http).unwrap_or_default())
        .https(env(["HTTPS_PROXY", "https_proxy"]).or(system.https).unwrap_or_default())
        .no(env(["NO_PROXY", "no_proxy"]).unwrap_or(system.bypass))
        .build()
}

/// A matched proxy as the address reqwest takes, credentials included.
fn proxy_url(
    intercept: &hyper_util::client::proxy::matcher::Intercept,
) -> Option<reqwest::Url> {
    use base64::Engine as _;
    let mut url = reqwest::Url::parse(&intercept.uri().to_string()).ok()?;
    let credentials = match intercept.raw_auth() {
        Some((user, password)) => Some((user.to_owned(), password.to_owned())),
        None => intercept
            .basic_auth()
            .and_then(|header| header.to_str().ok()?.strip_prefix("Basic ").map(str::to_owned))
            .and_then(|encoded| base64::engine::general_purpose::STANDARD.decode(encoded).ok())
            .and_then(|decoded| String::from_utf8(decoded).ok())
            .and_then(|pair| {
                pair.split_once(':')
                    .map(|(user, password)| (user.to_owned(), password.to_owned()))
            }),
    };
    if let Some((user, password)) = credentials {
        url.set_username(&user).ok()?;
        url.set_password(Some(password.as_str()).filter(|password| !password.is_empty()))
            .ok()?;
    }
    Some(url)
}

/// The network a launched process is handed: the app proxy, except for the
/// relay hosts a direct card reaches without it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessNetwork {
    pub proxy: ApiProxyConfigPayload,
    /// `NO_PROXY` entries reached without the proxy.
    pub direct_hosts: Vec<String>,
}

impl From<ApiProxyConfigPayload> for ProcessNetwork {
    fn from(proxy: ApiProxyConfigPayload) -> Self {
        Self {
            proxy,
            direct_hosts: Vec::new(),
        }
    }
}

impl ProcessNetwork {
    /// `global` for everything but `hosts`. A process can be told to skip
    /// hosts only with an explicit proxy, so a system proxy is handed over as
    /// the address it names now; with no proxy at all everything is direct.
    pub fn bypassing(global: &ApiProxyConfigPayload, hosts: Vec<String>) -> Self {
        let direct = || {
            Self::from(ApiProxyConfigPayload {
                mode: ApiProxyMode::Direct,
                url: None,
            })
        };
        let Ok(global) = sanitize_proxy_config(global) else {
            return direct();
        };
        if hosts.is_empty() {
            return Self::from(global);
        }
        match global.mode {
            ApiProxyMode::Direct => direct(),
            ApiProxyMode::Manual => Self {
                proxy: global,
                direct_hosts: hosts,
            },
            ApiProxyMode::System => {
                let Some(system) = crate::platform::proxy::system_proxy() else {
                    return direct();
                };
                let Some(url) = system.https.or(system.http) else {
                    return direct();
                };
                let mut direct_hosts = hosts;
                direct_hosts.extend(
                    system
                        .bypass
                        .split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(str::to_owned),
                );
                let proxy = ApiProxyConfigPayload {
                    mode: ApiProxyMode::Manual,
                    url: Some(url),
                };
                match sanitize_proxy_config(&proxy) {
                    Ok(proxy) => Self {
                        proxy,
                        direct_hosts,
                    },
                    Err(_) => direct(),
                }
            }
        }
    }

    /// The hosts reached without the proxy, loopback first.
    fn bypass(&self) -> Vec<&str> {
        let mut hosts: Vec<&str> = vec!["localhost", "127.0.0.1", "::1"];
        for host in &self.direct_hosts {
            if !hosts.iter().any(|known| known.eq_ignore_ascii_case(host)) {
                hosts.push(host);
            }
        }
        hosts
    }
}

/// Scope proxy changes to a child process. Loopback router and OAuth callback
/// requests must never travel through an external proxy.
pub(crate) fn configure_command_network(
    command: &mut std::process::Command,
    config: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    configure_process_network(command, &ProcessNetwork::from(config.clone()))
}

/// [`configure_command_network`] with a direct card's relay hosts kept off
/// the proxy.
pub(crate) fn configure_process_network(
    command: &mut std::process::Command,
    network: &ProcessNetwork,
) -> Result<(), CoreError> {
    let config = sanitize_proxy_config(&network.proxy)?;
    if config.mode != ApiProxyMode::System {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(key);
            if let Some(url) = config.url.as_deref() {
                command.env(key, url);
            }
        }
    }
    let bypass = match config.mode {
        ApiProxyMode::Direct => "*".to_owned(),
        ApiProxyMode::Manual => network.bypass().join(","),
        ApiProxyMode::System => {
            let inherited = std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default();
            std::iter::once(inherited.as_str())
                .filter(|inherited| !inherited.is_empty())
                .chain(network.bypass())
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    command.env("NO_PROXY", &bypass).env("no_proxy", &bypass);
    Ok(())
}

/// Codex can otherwise override the environment or fall back to system proxy
/// discovery after a failed OAuth request, defeating explicit Manual/Direct.
pub(crate) fn configure_codex_cli_network(
    command: &mut std::process::Command,
    config: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    configure_command_network(command, config)?;
    if config.mode != ApiProxyMode::System {
        command.args([
            "-c",
            "features.respect_system_proxy=false",
            "-c",
            "features.system_proxy_fallback=false",
        ]);
    }
    Ok(())
}

/// Electron's own HTTP stack needs a proxy flag as well as the environment
/// inherited by the Codex app-server child.
pub(crate) fn configure_codex_desktop(
    command: &mut std::process::Command,
    network: &ProcessNetwork,
) -> Result<(), CoreError> {
    configure_process_network(command, network)?;
    let config = sanitize_proxy_config(&network.proxy)?;
    match config.mode {
        ApiProxyMode::Manual => {
            let url = config
                .url
                .as_deref()
                .unwrap()
                .replacen("socks5h://", "socks5://", 1);
            command.arg(format!("--proxy-server={url}"));
            let bypass = network
                .bypass()
                .into_iter()
                .map(|host| match host.contains(':') && !host.contains('/') {
                    true => format!("[{host}]"),
                    false => host.to_owned(),
                })
                .collect::<Vec<_>>();
            command.arg(format!("--proxy-bypass-list={}", bypass.join(";")));
        }
        ApiProxyMode::Direct => {
            command.arg("--no-proxy-server");
        }
        ApiProxyMode::System => {}
    }
    Ok(())
}

/// Where a request that got no response failed, read from its cause chain:
/// a stopped proxy, a blocked handshake and a slow relay look alike as a
/// plain "cannot reach the provider".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportFailure {
    Timeout,
    Dns,
    Proxy,
    Tls,
    Refused,
    Reset,
    Connect,
    Other,
}

impl TransportFailure {
    pub fn of(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            return Self::Timeout;
        }
        // The error itself names the URL, whose host may read like a cause.
        let mut text = String::new();
        let mut kind = None;
        let mut source = std::error::Error::source(error);
        while let Some(cause) = source {
            text.push_str(&cause.to_string().to_ascii_lowercase());
            text.push('\n');
            if let Some(io) = cause.downcast_ref::<std::io::Error>() {
                kind = kind.or(Some(io.kind()));
            }
            source = cause.source();
        }
        Self::classify(&text, kind, error.is_connect())
    }

    fn classify(text: &str, kind: Option<std::io::ErrorKind>, connect: bool) -> Self {
        use std::io::ErrorKind;
        let any = |words: &[&str]| words.iter().any(|word| text.contains(word));
        if kind == Some(ErrorKind::TimedOut) {
            Self::Timeout
        } else if any(&["dns error", "failed to lookup address"]) {
            Self::Dns
        } else if any(&["tunnel", "proxy", "socks"]) {
            Self::Proxy
        } else if any(&["certificate", "handshake", "tls"]) {
            Self::Tls
        } else if kind == Some(ErrorKind::ConnectionRefused) {
            Self::Refused
        } else if matches!(
            kind,
            Some(ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::UnexpectedEof)
        ) {
            Self::Reset
        } else if connect {
            Self::Connect
        } else {
            Self::Other
        }
    }

    /// The code a route trace records.
    pub fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Dns => "dns_error",
            Self::Proxy => "proxy_error",
            Self::Tls => "tls_error",
            Self::Refused => "connection_refused",
            Self::Reset => "connection_reset",
            Self::Connect => "connect_error",
            Self::Other => "transport_error",
        }
    }

    /// What the client is told.
    pub fn message(self) -> &'static str {
        match self {
            Self::Timeout => "cannot reach the provider: timed out",
            Self::Dns => "cannot reach the provider: DNS lookup failed",
            Self::Proxy => "cannot reach the provider: the proxy failed the connection",
            Self::Tls => "cannot reach the provider: TLS handshake failed",
            Self::Refused => "cannot reach the provider: connection refused (by the provider, or by a proxy that is not running)",
            Self::Reset => "cannot reach the provider: connection reset",
            Self::Connect => "cannot reach the provider: connection failed",
            Self::Other => "cannot reach the provider",
        }
    }

    /// The error key the app shows.
    pub fn error_key(self) -> &'static str {
        match self {
            Self::Timeout => "newapi.errors.timeout",
            Self::Dns => "newapi.errors.dns",
            Self::Proxy => "newapi.errors.proxyConnect",
            Self::Tls => "newapi.errors.tls",
            Self::Refused => "newapi.errors.refused",
            Self::Reset => "newapi.errors.reset",
            Self::Connect | Self::Other => "newapi.errors.network",
        }
    }

    /// Whether `key` is one of these: a failure worth one retry.
    pub fn is_error_key(key: &str) -> bool {
        [
            Self::Timeout,
            Self::Dns,
            Self::Proxy,
            Self::Tls,
            Self::Refused,
            Self::Reset,
            Self::Other,
        ]
        .into_iter()
        .any(|failure| failure.error_key() == key)
    }
}

fn failed(code: &str, message: &str) -> ApiProxyTestPayload {
    ApiProxyTestPayload {
        code: code.into(),
        reachable: false,
        status_code: None,
        message: message.into(),
    }
}

pub fn test_api_connectivity(config: &ApiProxyConfigPayload) -> ApiProxyTestPayload {
    let config = match sanitize_proxy_config(config) {
        Ok(config) => config,
        Err(_) => return failed("invalid_config", "Invalid proxy configuration"),
    };
    let builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    let builder = match configure_blocking_client(builder, &config) {
        Ok(builder) => builder,
        Err(_) => return failed("invalid_config", "Invalid proxy configuration"),
    };
    let client = match builder.build() {
        Ok(client) => client,
        Err(_) => {
            return failed(
                "client_build_failed",
                "Could not initialize the network client",
            )
        }
    };
    match client.get("https://api.openai.com/v1/models").send() {
        Ok(response) => {
            let status = response.status();
            // A 401 is expected: diagnostics intentionally send no account credentials.
            let reachable = status.is_success() || status == reqwest::StatusCode::UNAUTHORIZED;
            ApiProxyTestPayload {
                code: if reachable { "reachable" } else { "http_error" }.into(),
                reachable,
                status_code: Some(i32::from(status.as_u16())),
                message: if reachable {
                    "API endpoint reachable; account authentication was not tested".into()
                } else {
                    format!("API endpoint returned HTTP {}", status.as_u16())
                },
            }
        }
        Err(error) => failed("network_error", &error.without_url().to_string()),
    }
}

pub fn detect_api_proxy_config() -> ApiProxyDetectPayload {
    let mut candidates = Vec::new();
    for name in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
    ] {
        if let Ok(url) = std::env::var(name) {
            if !candidates.contains(&url) {
                candidates.push(url);
            }
        }
    }
    candidates.extend(crate::platform::proxy::detect_system_proxy_candidates());
    for port in [10808, 7890, 1080] {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            let url = format!("http://127.0.0.1:{port}");
            if !candidates.contains(&url) {
                candidates.push(url);
            }
        }
    }
    let mut probe = failed("not_found", "No working local proxy found");
    for url in candidates.into_iter().take(5) {
        probe = test_api_connectivity(&ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some(url.clone()),
        });
        if probe.reachable {
            return ApiProxyDetectPayload {
                found: true,
                mode: Some(ApiProxyMode::Manual),
                url: Some(url),
                probe,
            };
        }
    }
    ApiProxyDetectPayload {
        found: false,
        mode: None,
        url: None,
        probe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_network_configuration_is_scoped_and_bypasses_loopback() {
        let mut command = std::process::Command::new("codex-fixture");
        let manual = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("http://127.0.0.1:10808".into()),
        };
        configure_codex_desktop(&mut command, &manual.clone().into()).unwrap();
        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_ascii_uppercase(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            assert_eq!(env[key].as_deref(), Some("http://127.0.0.1:10808"));
        }
        assert_eq!(env["NO_PROXY"].as_deref(), Some("localhost,127.0.0.1,::1"));
        assert!(command
            .get_args()
            .any(|arg| arg == "--proxy-server=http://127.0.0.1:10808"));
        let mut cli = std::process::Command::new("codex-fixture");
        configure_codex_cli_network(&mut cli, &manual).unwrap();
        assert!(cli
            .get_args()
            .any(|arg| arg == "features.respect_system_proxy=false"));
        assert!(cli
            .get_args()
            .any(|arg| arg == "features.system_proxy_fallback=false"));

        let direct = ApiProxyConfigPayload {
            mode: ApiProxyMode::Direct,
            url: None,
        };
        let mut command = std::process::Command::new("codex-fixture");
        configure_codex_desktop(&mut command, &direct.into()).unwrap();
        assert!(command.get_args().any(|arg| arg == "--no-proxy-server"));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "HTTP_PROXY" && value.is_none()));
        assert!(command.get_envs().any(|(key, value)| key
            .to_string_lossy()
            .eq_ignore_ascii_case("NO_PROXY")
            && value.is_some_and(|v| v == "*")));

        let system: ApiProxyConfigPayload = serde_json::from_str("{}").unwrap();
        assert_eq!(system.mode, ApiProxyMode::System);
        let mut command = std::process::Command::new("codex-fixture");
        command.env("HTTPS_PROXY", "http://inherited.invalid:7890");
        configure_codex_desktop(&mut command, &system.clone().into()).unwrap();
        assert_eq!(command.get_args().count(), 0);
        assert!(command.get_envs().any(|(key, value)| key == "HTTPS_PROXY"
            && value.is_some_and(|v| v == "http://inherited.invalid:7890")));
        assert_eq!(
            sanitize_proxy_config(&ApiProxyConfigPayload {
                mode: ApiProxyMode::System,
                url: Some("stale-url".into())
            })
            .unwrap()
            .url,
            None
        );
    }

    // Run this helper in a child test process so proxy environment changes never
    // race with the rest of the parallel test suite. Both HTTP client variants
    // must select the same network exit.
    #[test]
    fn isolated_proxy_probe() {
        let Ok(probe) = std::env::var("AGENTHARBOR_TEST_PROXY_PROBE") else {
            return;
        };
        let probe: serde_json::Value = serde_json::from_str(&probe).unwrap();
        let config: ApiProxyConfigPayload =
            serde_json::from_value(probe["config"].clone()).unwrap();
        let url = probe["url"].as_str().unwrap();
        let expected = probe["expected"].as_str().unwrap();
        let client = configure_blocking_client(
            reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)),
            &config,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(client.get(url).send().unwrap().text().unwrap(), expected);
        drop(client);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let client = configure_async_client(
                reqwest::Client::builder().timeout(Duration::from_secs(5)),
                &config,
            )
            .unwrap()
            .build()
            .unwrap();
            assert_eq!(
                client.get(url).send().await.unwrap().text().await.unwrap(),
                expected
            );
        });
    }

    #[test]
    fn system_manual_direct_and_codex_child_use_the_expected_network_exit() {
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let done = Arc::new(AtomicBool::new(false));
        let mut servers = Vec::new();
        let mut addresses = Vec::new();
        for name in ["direct", "system", "manual"] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            addresses.push(format!("http://{}", listener.local_addr().unwrap()));
            listener.set_nonblocking(true).unwrap();
            let done = done.clone();
            servers.push(std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                while !done.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                    if let Ok((mut stream, _)) = listener.accept() {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let mut bytes = [0; 4096];
                        let count = stream.read(&mut bytes).unwrap();
                        assert!(count > 0);
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{name}", name.len()).unwrap();
                    } else { std::thread::sleep(Duration::from_millis(5)); }
                }
            }));
        }
        let direct_url = format!("{}/probe", addresses[0]);
        let mut outcomes = Vec::new();
        for (mode, child_network, url, expected) in [
            // The registry's bypass list keeps loopback direct in system mode.
            (
                ApiProxyMode::System,
                false,
                "http://agentharbor-probe.invalid/system",
                "system",
            ),
            (ApiProxyMode::Manual, false, direct_url.as_str(), "manual"),
            (ApiProxyMode::Direct, false, direct_url.as_str(), "direct"),
            (ApiProxyMode::Manual, true, direct_url.as_str(), "direct"),
            (
                ApiProxyMode::Manual,
                true,
                "http://agentharbor-probe.invalid/account",
                "manual",
            ),
        ] {
            let config = ApiProxyConfigPayload {
                mode,
                url: Some(addresses[2].clone()),
            };
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "core::api_client::tests::isolated_proxy_probe",
                "--nocapture",
            ]);
            for key in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                child.env(key, &addresses[1]);
            }
            child.env("NO_PROXY", "").env("no_proxy", "");
            if child_network {
                configure_command_network(&mut child, &config).unwrap();
            }
            child.env(
                "AGENTHARBOR_TEST_PROXY_PROBE",
                serde_json::json!({
                    "config": if child_network { ApiProxyConfigPayload::default() } else { config },
                    "url": url, "expected": expected
                })
                .to_string(),
            );
            outcomes.push(child.output().unwrap());
        }
        done.store(true, Ordering::Relaxed);
        for server in servers {
            server.join().unwrap();
        }
        for output in outcomes {
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn the_system_proxy_reads_per_protocol_entries_and_their_bypass_list() {
        let system = crate::platform::proxy::SystemProxy {
            http: Some("http://127.0.0.1:7890".into()),
            https: Some("http://127.0.0.1:7891".into()),
            bypass: "relay.top,127.0.0.0/8,localhost".into(),
        };
        let route = |env: &[(&str, &str)], url: &str| {
            let env: std::collections::HashMap<String, String> = env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect();
            matcher_from(|name| env.get(name).cloned(), system.clone())
                .intercept(&url.parse().unwrap())
                .and_then(|intercept| proxy_url(&intercept))
                .map(|url| url.to_string())
        };
        assert_eq!(route(&[], "https://api.openai.com/v1").as_deref(), Some("http://127.0.0.1:7891/"));
        assert_eq!(route(&[], "http://example.com/").as_deref(), Some("http://127.0.0.1:7890/"));
        assert_eq!(route(&[], "https://api.relay.top/v1"), None);
        assert_eq!(route(&[], "http://127.0.0.1:8080/"), None);
        // Proxy variables come first, and their bypass list replaces the registry's.
        let variables = [
            ("HTTPS_PROXY", "http://user:p%40ss@10.0.0.1:3128"),
            ("NO_PROXY", "other.top"),
        ];
        assert_eq!(
            route(&variables, "https://api.relay.top/").as_deref(),
            Some("http://user:p%40ss@10.0.0.1:3128/")
        );
        assert_eq!(route(&variables, "https://x.other.top/"), None);
        assert_eq!(route(&[("HTTPS_PROXY", " ")], "https://api.openai.com/").as_deref(), Some("http://127.0.0.1:7891/"));
    }

    #[test]
    fn a_direct_cards_relay_host_alone_skips_the_proxy() {
        let manual = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("http://127.0.0.1:10808".into()),
        };
        let network = ProcessNetwork::bypassing(&manual, vec!["relay.example".into()]);
        assert_eq!(network.proxy, manual);
        let mut command = std::process::Command::new("codex-fixture");
        configure_codex_desktop(&mut command, &network).unwrap();
        let env = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
        };
        assert_eq!(env("HTTPS_PROXY").as_deref(), Some("http://127.0.0.1:10808"));
        assert_eq!(env("NO_PROXY").as_deref(), Some("localhost,127.0.0.1,::1,relay.example"));
        assert!(command
            .get_args()
            .any(|arg| arg == "--proxy-bypass-list=localhost;127.0.0.1;[::1];relay.example"));

        let direct = ApiProxyConfigPayload {
            mode: ApiProxyMode::Direct,
            url: None,
        };
        assert_eq!(
            ProcessNetwork::bypassing(&direct, vec!["relay.example".into()]),
            ProcessNetwork::from(direct)
        );
        assert_eq!(ProcessNetwork::bypassing(&manual, Vec::new()), ProcessNetwork::from(manual));
    }

    #[test]
    fn a_failed_request_names_where_it_failed() {
        use std::io::ErrorKind;
        let classify = TransportFailure::classify;
        assert_eq!(classify("client error (connect)\ndns error\n", None, true), TransportFailure::Dns);
        assert_eq!(classify("unsuccessful tunnel\n", None, true), TransportFailure::Proxy);
        assert_eq!(classify("invalid peer certificate: unknownissuer\n", None, true), TransportFailure::Tls);
        assert_eq!(classify("tcp connect error\n", Some(ErrorKind::ConnectionRefused), true), TransportFailure::Refused);
        assert_eq!(classify("connection reset\n", Some(ErrorKind::ConnectionReset), false), TransportFailure::Reset);
        assert_eq!(classify("os error 10060\n", Some(ErrorKind::TimedOut), true), TransportFailure::Timeout);
        assert_eq!(classify("", None, false), TransportFailure::Other);
        assert!(TransportFailure::is_error_key("newapi.errors.network"));
        assert!(TransportFailure::is_error_key("newapi.errors.proxyConnect"));
        assert!(!TransportFailure::is_error_key("newapi.errors.auth"));

        // A refused loopback port, end to end through reqwest's cause chain.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let error = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/proxy-in-the-path"))
            .send()
            .unwrap_err();
        assert_eq!(TransportFailure::of(&error), TransportFailure::Refused);
    }

    #[test]
    fn validates_manual_urls_and_clears_direct_url() {
        for url in [
            "file:///tmp/proxy",
            "not a URL",
            "http://localhost/path",
            "http://localhost/?key=value",
        ] {
            assert!(sanitize_proxy_config(&ApiProxyConfigPayload {
                mode: ApiProxyMode::Manual,
                url: Some(url.into())
            })
            .is_err());
        }
        assert!(sanitize_proxy_config(&ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("socks5://127.0.0.1:10808".into())
        })
        .is_ok());
        assert_eq!(
            sanitize_proxy_config(&ApiProxyConfigPayload {
                mode: ApiProxyMode::Direct,
                url: Some("invalid".into())
            })
            .unwrap()
            .url,
            None
        );
    }
}
