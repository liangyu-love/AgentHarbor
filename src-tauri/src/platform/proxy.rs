use reqwest::blocking::Client;
use std::fs;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::Duration;

pub fn detect_system_proxy_candidates() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        return detect_macos_system_proxy_candidates();
    }

    #[cfg(target_os = "windows")]
    {
        return detect_windows_system_proxy_candidates();
    }

    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(target_os = "macos")]
fn detect_macos_system_proxy_candidates() -> Vec<String> {
    let output = Command::new("scutil").arg("--proxy").output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pairs = parse_scutil_proxy_pairs(&stdout);
    let mut candidates = Vec::new();

    if is_enabled(&pairs, "HTTPEnable") {
        push_url(
            &mut candidates,
            "http",
            pairs.get("HTTPProxy").map(String::as_str),
            pairs.get("HTTPPort").map(String::as_str),
        );
    }

    if is_enabled(&pairs, "HTTPSEnable") {
        push_url(
            &mut candidates,
            "http",
            pairs.get("HTTPSProxy").map(String::as_str),
            pairs.get("HTTPSPort").map(String::as_str),
        );
    }

    if is_enabled(&pairs, "SOCKSEnable") {
        push_url(
            &mut candidates,
            "socks5",
            pairs.get("SOCKSProxy").map(String::as_str),
            pairs.get("SOCKSPort").map(String::as_str),
        );
    }

    if is_enabled(&pairs, "ProxyAutoConfigEnable") {
        if let Some(url) = pairs.get("ProxyAutoConfigURLString") {
            candidates.extend(parse_pac_candidates(url));
        }
    }

    dedupe(candidates)
}

#[cfg(target_os = "macos")]
fn parse_scutil_proxy_pairs(stdout: &str) -> std::collections::HashMap<String, String> {
    let mut pairs = std::collections::HashMap::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('<') || trimmed == "{" || trimmed == "}" || trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        pairs.insert(key.trim().to_string(), value.trim().to_string());
    }
    pairs
}

#[cfg(target_os = "macos")]
fn is_enabled(pairs: &std::collections::HashMap<String, String>, key: &str) -> bool {
    matches!(pairs.get(key).map(String::as_str), Some("1"))
}

#[cfg(target_os = "windows")]
const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The proxies system settings name for plain and TLS traffic, and the hosts
/// reached without them as a `NO_PROXY` list.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SystemProxy {
    pub http: Option<String>,
    pub https: Option<String>,
    pub bypass: String,
}

/// The WinINet proxy, which proxy software turns on as "the system proxy".
/// Read natively, so it can be read again for every lookup.
#[cfg(target_os = "windows")]
pub(crate) fn wininet_proxy() -> Option<SystemProxy> {
    use crate::platform::windows::{user_registry_dword, user_registry_text};
    if user_registry_dword(INTERNET_SETTINGS, "ProxyEnable")? == 0 {
        return None;
    }
    let server = parse_proxy_server(&user_registry_text(INTERNET_SETTINGS, "ProxyServer")?);
    let proxy = SystemProxy {
        http: server.http.or_else(|| server.socks.clone()),
        https: server.https.or(server.socks),
        bypass: bypass_list(&user_registry_text(INTERNET_SETTINGS, "ProxyOverride").unwrap_or_default()),
    };
    (proxy.http.is_some() || proxy.https.is_some()).then_some(proxy)
}

/// The system proxy as a child process can be handed it: WinINet's, else the
/// first address a PAC file or WinHTTP names.
pub(crate) fn system_proxy() -> Option<SystemProxy> {
    #[cfg(target_os = "windows")]
    if let Some(proxy) = wininet_proxy() {
        return Some(proxy);
    }
    let url = detect_system_proxy_candidates().into_iter().next()?;
    Some(SystemProxy {
        http: Some(url.clone()),
        https: Some(url),
        bypass: LOOPBACK.join(","),
    })
}

#[cfg(target_os = "windows")]
fn detect_windows_system_proxy_candidates() -> Vec<String> {
    use crate::platform::windows::{user_registry_dword, user_registry_text};
    let mut candidates = Vec::new();
    let mut push_server = |value: &str| {
        let server = parse_proxy_server(value);
        candidates.extend([server.http, server.https, server.socks].into_iter().flatten());
    };
    if user_registry_dword(INTERNET_SETTINGS, "ProxyEnable").is_some_and(|enabled| enabled != 0) {
        if let Some(server) = user_registry_text(INTERNET_SETTINGS, "ProxyServer") {
            push_server(&server);
        }
    }
    if let Some(winhttp_proxy) = read_windows_winhttp_proxy_server() {
        push_server(&winhttp_proxy);
    }
    if let Some(pac) = user_registry_text(INTERNET_SETTINGS, "AutoConfigURL").filter(|url| !url.trim().is_empty()) {
        candidates.extend(parse_pac_candidates(&pac));
    }
    dedupe(candidates)
}

#[cfg(target_os = "windows")]
fn read_windows_winhttp_proxy_server() -> Option<String> {
    let output = crate::platform::windows::background_command("netsh")
        .args(["winhttp", "show", "proxy"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with("Proxy Server") {
            return None;
        }
        trimmed
            .split_once(':')
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty() && *value != "(none)")
            .map(|value| value.to_string())
    })
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Default, PartialEq)]
struct ProxyServer {
    http: Option<String>,
    https: Option<String>,
    socks: Option<String>,
}

/// `ProxyServer` as proxy software writes it: one `host:port` for every
/// protocol, or `http=…;https=…;socks=…` per protocol.
#[cfg(any(target_os = "windows", test))]
fn parse_proxy_server(value: &str) -> ProxyServer {
    let url = |address: &str, scheme: &str| {
        let address = address.trim();
        (!address.is_empty()).then(|| match address.contains("://") {
            true => address.to_owned(),
            false => format!("{scheme}://{address}"),
        })
    };
    let value = value.trim();
    if !value.contains('=') {
        let all = url(value, "http");
        return ProxyServer { http: all.clone(), https: all, socks: None };
    }
    let mut server = ProxyServer::default();
    for (scheme, address) in value.split(';').filter_map(|part| part.split_once('=')) {
        match scheme.trim().to_ascii_lowercase().as_str() {
            "http" => server.http = url(address, "http"),
            // The proxy for TLS traffic, itself spoken to in plain HTTP.
            "https" => server.https = url(address, "http"),
            "socks" | "socks5" => server.socks = url(address, "socks5"),
            _ => {}
        }
    }
    server
}

/// `ProxyOverride` (`;`-separated, `*` wildcards, `<local>`) as a `NO_PROXY`
/// list: `*.example.com` covers the domain, `10.*` the address range.
/// Loopback always bypasses.
#[cfg(any(target_os = "windows", test))]
fn bypass_list(value: &str) -> String {
    let mut entries: Vec<String> = Vec::new();
    for entry in value.split(';').map(str::trim) {
        let entry = entry.strip_prefix("*.").unwrap_or(entry);
        let entry = match ipv4_range(entry) {
            Some(range) => range,
            // `<local>` (dotless intranet names) has no `NO_PROXY` form.
            None if entry.is_empty() || entry.starts_with('<') => continue,
            None if entry != "*" && entry.contains('*') => continue,
            None => entry.to_owned(),
        };
        if !entries.iter().any(|known| known.eq_ignore_ascii_case(&entry)) {
            entries.push(entry);
        }
    }
    for host in LOOPBACK {
        if !entries.iter().any(|known| known == host) {
            entries.push(host.to_owned());
        }
    }
    entries.join(",")
}

/// `10.*` or `192.168.*` as the CIDR range it covers.
#[cfg(any(target_os = "windows", test))]
fn ipv4_range(entry: &str) -> Option<String> {
    let octets = entry
        .strip_suffix(".*")?
        .split('.')
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .filter(|octets| (1..=3).contains(&octets.len()))?;
    let mut address = octets.clone();
    address.resize(4, 0);
    Some(format!(
        "{}.{}.{}.{}/{}",
        address[0],
        address[1],
        address[2],
        address[3],
        octets.len() * 8
    ))
}

fn push_url(candidates: &mut Vec<String>, scheme: &str, host: Option<&str>, port: Option<&str>) {
    let Some(host) = host.map(str::trim).filter(|host| !host.is_empty()) else {
        return;
    };

    let url = if host.contains("://") {
        host.to_string()
    } else if let Some(port) = port.map(str::trim).filter(|port| !port.is_empty()) {
        format!("{scheme}://{host}:{port}")
    } else {
        format!("{scheme}://{host}")
    };

    candidates.push(url);
}

fn parse_pac_candidates(pac_url: &str) -> Vec<String> {
    let Some(contents) = load_pac_contents(pac_url) else {
        return Vec::new();
    };

    let uppercase = contents.to_ascii_uppercase();
    let mut candidates = Vec::new();
    for (token, scheme) in [
        ("SOCKS5 ", "socks5"),
        ("SOCKS ", "socks5"),
        ("HTTPS ", "http"),
        ("PROXY ", "http"),
    ] {
        let mut search_from = 0;
        while let Some(relative_index) = uppercase[search_from..].find(token) {
            let start = search_from + relative_index + token.len();
            let rest_original = &contents[start..];
            let mut end = 0;
            for ch in rest_original.chars() {
                if ch == ';' || ch == '"' || ch == '\'' || ch.is_whitespace() {
                    break;
                }
                end += ch.len_utf8();
            }
            let address = rest_original[..end].trim();
            if !address.is_empty() {
                push_url(&mut candidates, scheme, Some(address), None);
            }
            search_from = start;
        }
    }

    dedupe(candidates)
}

fn load_pac_contents(pac_url: &str) -> Option<String> {
    let trimmed = pac_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(path) = trimmed.strip_prefix("file://") {
        let decoded = percent_decode_path(path);
        return fs::read_to_string(decoded).ok();
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok()?;
        return client.get(trimmed).send().ok()?.text().ok();
    }

    None
}

fn percent_decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &path[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();

    for value in values {
        if seen.insert(value.clone()) {
            deduped.push(value);
        }
    }

    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_server_reads_one_address_or_one_per_protocol() {
        let one = parse_proxy_server(" 127.0.0.1:10808 ");
        assert_eq!(one.http.as_deref(), Some("http://127.0.0.1:10808"));
        assert_eq!(one.https, one.http);
        let split = parse_proxy_server("http=127.0.0.1:7890;https=127.0.0.1:7891;socks=127.0.0.1:7892");
        assert_eq!(split.http.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(split.https.as_deref(), Some("http://127.0.0.1:7891"));
        assert_eq!(split.socks.as_deref(), Some("socks5://127.0.0.1:7892"));
        let socks_only = parse_proxy_server("socks=127.0.0.1:1080");
        assert_eq!((socks_only.http, socks_only.https), (None, None));
    }

    #[test]
    fn proxy_override_becomes_a_no_proxy_list() {
        assert_eq!(
            bypass_list("<local>;localhost;127.*;172.16.*;192.168.1.*;*.example.com;relay.top;10.70.0.223;bad*glob;"),
            "localhost,127.0.0.0/8,172.16.0.0/16,192.168.1.0/24,example.com,relay.top,10.70.0.223,127.0.0.1,::1"
        );
        assert_eq!(bypass_list(""), "localhost,127.0.0.1,::1");
        assert_eq!(bypass_list("*"), "*,localhost,127.0.0.1,::1");
    }
}
