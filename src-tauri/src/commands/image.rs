use crate::core::{newapi, repository::Repository, router, secret_file};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::multipart;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    sync::Mutex,
    time::Duration,
};
use tauri::State;

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 120 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedImageKey {
    base_url: String,
    api_key: String,
}

fn key_path(paths: &CodexPaths) -> std::path::PathBuf {
    paths.app_data_dir.join("image-workbench-key.dat")
}

fn read_key(paths: &CodexPaths, base_url: &str) -> Result<Option<String>, String> {
    let Some(bytes) = secret_file::read(&key_path(paths)).map_err(|_| "image.errors.storage")?
    else {
        return Ok(None);
    };
    let saved: SavedImageKey =
        serde_json::from_slice(&bytes).map_err(|_| "image.errors.storage")?;
    let base_url = newapi::base_url(base_url).map_err(|_| "image.errors.config")?;
    Ok((saved.base_url == base_url.as_str()).then_some(saved.api_key))
}

fn write_key(paths: &CodexPaths, base_url: &str, api_key: &str) -> Result<(), String> {
    let base_url = newapi::base_url(base_url).map_err(|_| "image.errors.config")?;
    newapi::auth_header(api_key).map_err(|_| "image.errors.apiKey")?;
    let saved = SavedImageKey {
        base_url: base_url.to_string(),
        api_key: api_key.trim().to_owned(),
    };
    let bytes = serde_json::to_vec(&saved).map_err(|_| "image.errors.storage")?;
    secret_file::write(&key_path(paths), &bytes).map_err(|_| "image.errors.storage".to_owned())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationRequest {
    pub provider_id: Option<String>,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    pub prompt: String,
    #[serde(default = "default_size")]
    pub size: String,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_count")]
    pub count: u8,
    pub image_data: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationResponse {
    pub images: Vec<String>,
    pub model: String,
    pub revised_prompts: Vec<String>,
}

fn default_size() -> String {
    "1024x1024".into()
}

fn default_quality() -> String {
    "auto".into()
}

fn default_count() -> u8 {
    1
}

fn validate(request: &ImageGenerationRequest) -> Result<(), String> {
    let model = request.model.trim();
    if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
        return Err("image.errors.model".into());
    }
    let prompt = request.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err("image.errors.prompt".into());
    }
    if !matches!(
        request.size.as_str(),
        "auto" | "1024x1024" | "1536x1024" | "1024x1536" | "1792x1024" | "1024x1792"
    ) {
        return Err("image.errors.size".into());
    }
    if !matches!(request.quality.as_str(), "auto" | "low" | "medium" | "high") {
        return Err("image.errors.quality".into());
    }
    if !(1..=4).contains(&request.count) {
        return Err("image.errors.count".into());
    }
    if request
        .image_data
        .as_deref()
        .is_some_and(|value| value.len() > MAX_IMAGE_BYTES * 2)
    {
        return Err("image.errors.inputTooLarge".into());
    }
    Ok(())
}

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "image.errors.storage".to_owned())?
        .paths()
        .clone())
}

fn decode_image_data(value: &str) -> Result<(Vec<u8>, String, String), String> {
    let (mime, encoded) = if let Some((header, data)) = value.split_once(",") {
        let mime = header
            .strip_prefix("data:")
            .and_then(|header| header.split(';').next())
            .unwrap_or("image/png");
        (mime.to_owned(), data)
    } else {
        ("image/png".into(), value)
    };
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "image.errors.input".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.inputTooLarge".into());
    }
    let extension = match mime.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    Ok((bytes, mime, extension.into()))
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "image.errors.network")?;
    if bytes.len() > limit {
        return Err("image.errors.responseTooLarge".into());
    }
    Ok(bytes)
}

fn response_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let body = read_limited(response, MAX_RESPONSE_BYTES)?;
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "image.errors.auth",
            404 => "image.errors.notFound",
            429 => "image.errors.rateLimit",
            _ => "image.errors.server",
        }
        .into());
    }
    serde_json::from_slice(&body).map_err(|_| "image.errors.response".into())
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] == 0 || segments[1] == 0x0db8))
                && ip.to_ipv4_mapped().is_none()
        }
    }
}

fn download_client(url: &reqwest::Url) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    let host = url.host_str().ok_or("image.errors.response")?;
    let port = url.port_or_known_default().ok_or("image.errors.response")?;
    if let Ok(ip) = host.trim_matches(&['[', ']'][..]).parse::<IpAddr>() {
        if !public_ip(ip) {
            return Err("image.errors.response".into());
        }
    } else {
        let name = host.trim_end_matches('.').to_ascii_lowercase();
        if !name.contains('.')
            || [
                ".localhost",
                ".local",
                ".internal",
                ".test",
                ".invalid",
                ".example",
                ".onion",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            return Err("image.errors.response".into());
        }
        let addresses: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|_| "image.errors.network")?
            .collect();
        if addresses.is_empty() || addresses.iter().any(|addr| !public_ip(addr.ip())) {
            return Err("image.errors.response".into());
        }
        // Pin the validated DNS answer so the request cannot resolve to a local address later.
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder.build().map_err(|_| "image.errors.network".into())
}

fn image_to_data_url(value: &Value) -> Result<String, String> {
    if let Some(encoded) = value.get("b64_json").and_then(Value::as_str) {
        if encoded.len() > MAX_IMAGE_BYTES * 4 / 3 + 8 {
            return Err("image.errors.responseTooLarge".into());
        }
        return Ok(format!("data:image/png;base64,{encoded}"));
    }
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or("image.errors.response")?;
    let url = reqwest::Url::parse(url).map_err(|_| "image.errors.response")?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("image.errors.response".into());
    }
    let client = download_client(&url)?;
    let response = client
        .get(url)
        .send()
        .map_err(|_| "image.errors.network".to_owned())?;
    if !response.status().is_success() {
        return Err("image.errors.network".into());
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("image/"))
        .unwrap_or("image/png")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .to_owned();
    let bytes = read_limited(response, MAX_IMAGE_BYTES)?;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn generate(
    request: ImageGenerationRequest,
    paths: &CodexPaths,
) -> Result<ImageGenerationResponse, String> {
    validate(&request)?;
    let store = router::load_store(paths)?;
    let (connection, network_mode) = if let Some(provider_id) = request.provider_id.as_deref() {
        let provider = store
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or("image.errors.provider")?;
        if provider.api_key.trim().is_empty() {
            return Err("image.errors.apiKey".into());
        }
        (
            newapi::Connection {
                base_url: provider.base_url.clone(),
                api_key: provider.api_key.clone(),
                model: request.model.clone(),
                ..Default::default()
            },
            provider.network_mode().to_owned(),
        )
    } else {
        if request.base_url.trim().is_empty() {
            return Err("image.errors.config".into());
        }
        let api_key = if request.api_key.trim().is_empty() {
            read_key(paths, &request.base_url)?.ok_or("image.errors.apiKey")?
        } else {
            write_key(paths, &request.base_url, &request.api_key)?;
            request.api_key.trim().to_owned()
        };
        (
            newapi::Connection {
                base_url: newapi::base_url(&request.base_url)?.to_string(),
                api_key,
                model: request.model.clone(),
                ..Default::default()
            },
            "proxy".to_owned(),
        )
    };
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    let proxy = newapi::effective_proxy(&network_mode, &proxy);
    let client = newapi::client_with_timeout(&proxy, Duration::from_secs(300))?;
    let endpoint = newapi::endpoint(
        &connection,
        if request.image_data.is_some() {
            "v1/images/edits"
        } else {
            "v1/images/generations"
        },
    )?;
    let auth = newapi::auth_header(&connection.api_key)?;
    let response = if let Some(image_data) = request.image_data.as_deref() {
        let (bytes, mime, extension) = decode_image_data(image_data)?;
        let mut form = multipart::Form::new()
            .text("model", request.model.clone())
            .text("prompt", request.prompt.clone())
            .text("size", request.size.clone())
            .text("quality", request.quality.clone())
            .text("n", request.count.to_string());
        let part = multipart::Part::bytes(bytes)
            .file_name(format!("input.{extension}"))
            .mime_str(&mime)
            .map_err(|_| "image.errors.input".to_owned())?;
        form = form.part("image", part);
        client
            .post(endpoint)
            .header(AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .map_err(|_| "image.errors.network".to_owned())?
    } else {
        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": request.count,
            "size": request.size,
            "quality": request.quality,
        });
        if let Some(object) = body.as_object_mut() {
            object.retain(|_, value| {
                !value.is_string() || !value.as_str().is_some_and(|text| text.is_empty())
            });
        }
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .map_err(|_| "image.errors.network".to_owned())?
    };
    let value = response_json(response)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or("image.errors.response")?;
    if data.is_empty() || data.len() > request.count as usize {
        return Err("image.errors.response".into());
    }
    let mut images = Vec::with_capacity(data.len());
    let mut revised_prompts = Vec::with_capacity(data.len());
    for item in data {
        images.push(image_to_data_url(item)?);
        revised_prompts.push(
            item.get("revised_prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        );
    }
    if images.is_empty() {
        return Err("image.errors.response".into());
    }
    Ok(ImageGenerationResponse {
        images,
        model: request.model,
        revised_prompts,
    })
}

#[tauri::command]
pub async fn generate_images(
    repo: State<'_, Mutex<Repository>>,
    request: ImageGenerationRequest,
) -> Result<ImageGenerationResponse, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || generate(request, &paths))
        .await
        .map_err(|_| "image.errors.network".to_owned())?
}

#[tauri::command]
pub fn image_key_status(
    repo: State<'_, Mutex<Repository>>,
    base_url: String,
) -> Result<bool, String> {
    if base_url.trim().is_empty() {
        return Ok(false);
    }
    Ok(read_key(&paths(&repo)?, &base_url)?.is_some())
}

#[tauri::command]
pub fn save_image_key(
    repo: State<'_, Mutex<Repository>>,
    base_url: String,
    api_key: String,
) -> Result<(), String> {
    write_key(&paths(&repo)?, &base_url, &api_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_key_is_bound_to_its_site() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        write_key(&paths, "https://example.org/v1", "sk-test").unwrap();
        assert_eq!(
            read_key(&paths, "https://example.org/").unwrap().as_deref(),
            Some("sk-test")
        );
        assert_eq!(
            read_key(&paths, "https://other.example.org/").unwrap(),
            None
        );
        let stored = std::fs::read(key_path(&paths)).unwrap();
        assert!(!stored.is_empty());
        #[cfg(windows)]
        assert!(!stored
            .windows(b"sk-test".len())
            .any(|part| part == b"sk-test"));
    }

    #[test]
    fn response_reader_rejects_oversize_without_reading_entire_stream() {
        assert_eq!(
            read_limited(&b"abc"[..], 2).unwrap_err(),
            "image.errors.responseTooLarge"
        );
        assert_eq!(read_limited(&b"ab"[..], 2).unwrap(), b"ab");
    }

    #[test]
    fn private_and_special_addresses_are_rejected() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.0.1",
            "100.64.0.1",
            "[::1]",
            "[fc00::1]",
        ] {
            let ip: IpAddr = address.trim_matches(&['[', ']'][..]).parse().unwrap();
            assert!(!public_ip(ip), "{address}");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
        for url in [
            "http://127.0.0.1/image.png",
            "http://192.168.1.4/image.png",
            "https://localhost/image.png",
        ] {
            let item = serde_json::json!({"url": url});
            assert_eq!(
                image_to_data_url(&item).unwrap_err(),
                "image.errors.response"
            );
        }
    }
}
