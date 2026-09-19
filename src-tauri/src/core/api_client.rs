//! Credential-free network diagnostics; this does not fetch account quotas.
use super::models::{ApiProxyConfigPayload, ApiProxyDetectPayload, ApiProxyMode, ApiProxyTestPayload, CoreError};
use std::time::Duration;

pub fn sanitize_proxy_config(config: &ApiProxyConfigPayload) -> Result<ApiProxyConfigPayload, CoreError> {
    if config.mode == ApiProxyMode::Direct {
        return Ok(ApiProxyConfigPayload { mode: ApiProxyMode::Direct, url: None });
    }
    let text = config.url.as_deref().map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::InvalidData("Manual proxy mode requires a proxy URL".into()))?;
    let url = reqwest::Url::parse(text).map_err(|_| CoreError::InvalidData("Invalid proxy URL".into()))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(CoreError::InvalidData("Unsupported proxy scheme".into()));
    }
    if url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(CoreError::InvalidData("Invalid proxy URL".into()));
    }
    Ok(ApiProxyConfigPayload { mode: ApiProxyMode::Manual, url: Some(text.to_owned()) })
}

fn failed(code: &str, message: &str) -> ApiProxyTestPayload {
    ApiProxyTestPayload { code: code.into(), reachable: false, status_code: None, message: message.into() }
}

pub fn test_api_connectivity(config: &ApiProxyConfigPayload) -> ApiProxyTestPayload {
    let config = match sanitize_proxy_config(config) {
        Ok(config) => config,
        Err(_) => return failed("invalid_config", "Invalid proxy configuration"),
    };
    let mut builder = reqwest::blocking::Client::builder().no_proxy()
        .connect_timeout(Duration::from_secs(4)).timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url) = config.url {
        let proxy = match reqwest::Proxy::all(url) {
            Ok(proxy) => proxy,
            Err(_) => return failed("invalid_config", "Invalid proxy configuration"),
        };
        builder = builder.proxy(proxy);
    }
    let client = match builder.build() {
        Ok(client) => client,
        Err(_) => return failed("client_build_failed", "Could not initialize the network client"),
    };
    match client.get("https://api.openai.com/v1/models").send() {
        Ok(response) => {
            let status = response.status();
            // A 401 is expected: diagnostics intentionally send no account credentials.
            let reachable = status.is_success() || status == reqwest::StatusCode::UNAUTHORIZED;
            ApiProxyTestPayload {
                code: if reachable { "reachable" } else { "http_error" }.into(),
                reachable, status_code: Some(i32::from(status.as_u16())),
                message: if reachable { "API endpoint reachable; account authentication was not tested".into() }
                    else { format!("API endpoint returned HTTP {}", status.as_u16()) },
            }
        }
        Err(error) => failed("network_error", &error.without_url().to_string()),
    }
}

pub fn detect_api_proxy_config() -> ApiProxyDetectPayload {
    let mut candidates = Vec::new();
    for name in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "https_proxy", "http_proxy"] {
        if let Ok(url) = std::env::var(name) { if !candidates.contains(&url) { candidates.push(url); } }
    }
    candidates.extend(crate::platform::proxy::detect_system_proxy_candidates());
    for port in [10808, 7890, 1080] {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            let url = format!("http://127.0.0.1:{port}");
            if !candidates.contains(&url) { candidates.push(url); }
        }
    }
    let mut probe = failed("not_found", "No working local proxy found");
    for url in candidates.into_iter().take(5) {
        probe = test_api_connectivity(&ApiProxyConfigPayload { mode: ApiProxyMode::Manual, url: Some(url.clone()) });
        if probe.reachable {
            return ApiProxyDetectPayload { found: true, mode: Some(ApiProxyMode::Manual), url: Some(url), probe };
        }
    }
    ApiProxyDetectPayload { found: false, mode: None, url: None, probe }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_manual_urls_and_clears_direct_url() {
        for url in ["file:///tmp/proxy", "not a URL", "http://localhost/path", "http://localhost/?key=value"] {
            assert!(sanitize_proxy_config(&ApiProxyConfigPayload { mode: ApiProxyMode::Manual, url: Some(url.into()) }).is_err());
        }
        assert!(sanitize_proxy_config(&ApiProxyConfigPayload { mode: ApiProxyMode::Manual, url: Some("socks5://127.0.0.1:10808".into()) }).is_ok());
        assert_eq!(sanitize_proxy_config(&ApiProxyConfigPayload { mode: ApiProxyMode::Direct, url: Some("invalid".into()) }).unwrap().url, None);
    }
}
