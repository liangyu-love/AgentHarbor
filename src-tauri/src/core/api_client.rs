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
    Ok(match config.mode {
        ApiProxyMode::System => builder,
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
    Ok(match config.mode {
        ApiProxyMode::System => builder,
        ApiProxyMode::Direct => builder.no_proxy(),
        ApiProxyMode::Manual => builder.no_proxy().proxy(
            reqwest::Proxy::all(config.url.as_deref().unwrap())
                .map_err(|_| CoreError::InvalidData("Invalid proxy URL".into()))?,
        ),
    })
}

/// Scope proxy changes to a child process. Loopback router and OAuth callback
/// requests must never travel through an external proxy.
pub(crate) fn configure_command_network(
    command: &mut std::process::Command,
    config: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    let config = sanitize_proxy_config(config)?;
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
        ApiProxyMode::Manual => "localhost,127.0.0.1,::1".to_owned(),
        ApiProxyMode::System => {
            let inherited = std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default();
            format!("{inherited},localhost,127.0.0.1,::1")
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
    config: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    configure_command_network(command, config)?;
    let config = sanitize_proxy_config(config)?;
    match config.mode {
        ApiProxyMode::Manual => {
            let url = config
                .url
                .as_deref()
                .unwrap()
                .replacen("socks5h://", "socks5://", 1);
            command.arg(format!("--proxy-server={url}"));
            command.arg("--proxy-bypass-list=localhost;127.0.0.1;[::1]");
        }
        ApiProxyMode::Direct => {
            command.arg("--no-proxy-server");
        }
        ApiProxyMode::System => {}
    }
    Ok(())
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
        configure_codex_desktop(&mut command, &manual).unwrap();
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
        configure_codex_desktop(&mut command, &direct).unwrap();
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
        configure_codex_desktop(&mut command, &system).unwrap();
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
            (ApiProxyMode::System, false, direct_url.as_str(), "system"),
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
