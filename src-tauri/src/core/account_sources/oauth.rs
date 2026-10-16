//! What the vendors' own sign-ins share: PKCE, a JWT's claims, OAuth form
//! posts, and the encodings their clients sign requests with.
use base64::Engine;
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

/// A PKCE verifier (43 URL-safe characters) and its S256 challenge.
pub fn pkce() -> (String, String) {
    let raw: Vec<u8> = (0..2).flat_map(|_| *uuid::Uuid::new_v4().as_bytes()).collect();
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Random lowercase hex of `bytes` bytes.
pub fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        out.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    out.truncate(bytes * 2);
    out
}

/// A JWT's claims, unchecked: only what the vendor said of its own token.
pub fn jwt_claims(token: &str) -> Value {
    token.split('.').nth(1)
        .and_then(|part| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

pub fn md5_hex(text: &str) -> String {
    use md5::Digest as _;
    md5::Md5::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// JavaScript's `encodeURIComponent`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// An OAuth reply: its HTTP status and JSON body (`Null` when it wasn't JSON).
pub struct FormReply {
    pub status: u16,
    pub body: Value,
}

impl FormReply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status) && self.error().is_none()
    }

    pub fn error(&self) -> Option<&str> {
        self.body["error"].as_str().filter(|error| !error.is_empty())
    }
}

/// Post an OAuth form; `Err` only when no reply came at all.
pub async fn post_form(client: &Client, url: &str, fields: &[(&str, &str)]) -> Result<FormReply, String> {
    let response = client.post(url)
        .header("accept", "application/json")
        .form(fields)
        .timeout(Duration::from_secs(30))
        .send().await.map_err(|error| if error.is_timeout() { "timed out".to_owned() } else { "no connection".to_owned() })?;
    let status = response.status().as_u16();
    let body = response.json().await.unwrap_or(Value::Null);
    Ok(FormReply { status, body })
}

/// This machine's OS as Node's `process.platform` names it.
pub fn node_platform() -> &'static str {
    if cfg!(windows) { "win32" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" }
}

/// The OS release as Node's `os.release()` gives it: `10.0.26200` on
/// Windows, the kernel's release elsewhere.
pub fn os_release() -> &'static str {
    static RELEASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    RELEASE.get_or_init(|| release().unwrap_or_default())
}

#[cfg(windows)]
fn release() -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let dword = |name: &str| -> Option<u32> {
        let name = wide(name);
        let mut value = 0u32;
        let mut size = 4u32;
        // SAFETY: the buffers outlive the call and `size` says how big `value` is.
        let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut value as *mut u32).cast(), &mut size) };
        (status == 0).then_some(value)
    };
    let text = |name: &str| -> Option<String> {
        let name = wide(name);
        let mut buffer = [0u16; 64];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: as above; the value is written into `buffer`, at most `size` bytes.
        let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
        (status == 0).then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]))
    };
    Some(format!("{}.{}.{}", dword("CurrentMajorVersionNumber")?, dword("CurrentMinorVersionNumber")?, text("CurrentBuild")?))
}

#[cfg(target_os = "linux")]
fn release() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|text| text.trim().to_owned())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn release() -> Option<String> {
    let output = std::process::Command::new("uname").arg("-r").output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The machine's architecture as Node's `os.arch()` names it.
pub fn node_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

/// The local UTC offset in seconds, as `-new Date().getTimezoneOffset() * 60`.
pub fn utc_offset_seconds() -> i32 {
    chrono::Local::now().offset().local_minus_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_match_their_javascript_counterparts() {
        assert_eq!(encode_uri_component("/v1/api?a=1&b=x y"), "%2Fv1%2Fapi%3Fa%3D1%26b%3Dx%20y");
        assert_eq!(encode_uri_component("a-_.!~*'()"), "a-_.!~*'()");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        let (verifier, challenge) = pkce();
        assert_eq!(verifier.len(), 43);
        assert_eq!(challenge.len(), 43);
        assert_eq!(random_hex(16).len(), 32);
        if cfg!(windows) {
            assert_eq!(os_release().split('.').count(), 3, "{}", os_release());
        }
        let token = format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"u1"}"#));
        assert_eq!(jwt_claims(&token)["sub"], "u1");
    }
}
