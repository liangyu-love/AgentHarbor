use crate::core::{
    api_client, models::ApiProxyConfigPayload, newapi, repository::Repository, router, secret_file,
};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::multipart;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Cursor, Read},
    net::{IpAddr, ToSocketAddrs},
    sync::Mutex,
    time::Duration,
};
use tauri::State;

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 120 * 1024 * 1024;
const THUMBNAIL_EDGE: u32 = 256;
const MAX_THUMBNAIL_DIMENSION: u32 = 8192;
const MAX_THUMBNAIL_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_THUMBNAIL_DECODE_BYTES: u64 = 128 * 1024 * 1024;
static THUMBNAIL_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static IMAGE_GENERATION_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "image.errors.timeout".into()
    } else {
        "image.errors.network".into()
    }
}

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
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                "image.errors.timeout"
            } else {
                "image.errors.network"
            }
        })?;
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

fn download_client_builder(
    proxy: &ApiProxyConfigPayload,
) -> Result<reqwest::blocking::ClientBuilder, String> {
    let builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    api_client::configure_blocking_client(builder, proxy).map_err(|_| "image.errors.network".into())
}

fn download_client(
    url: &reqwest::Url,
    proxy: &ApiProxyConfigPayload,
) -> Result<reqwest::blocking::Client, String> {
    let mut builder = download_client_builder(proxy)?;
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
        // Pin validated answers for direct connections. HTTP CONNECT / SOCKS5h
        // proxies may resolve the original hostname themselves and ignore this
        // override. Such explicitly configured or system proxies are trusted
        // network exits; the app cannot enforce their remote DNS answers. Local
        // URL and DNS checks still fail closed before sending to any proxy.
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder.build().map_err(|_| "image.errors.network".into())
}

fn image_to_data_url(value: &Value, proxy: &ApiProxyConfigPayload) -> Result<String, String> {
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
    // This separate client has no provider headers, cookies, or credentials.
    let client = download_client(&url, proxy)?;
    let response = client.get(url).send().map_err(network_error)?;
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
            .map_err(network_error)?
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
            .map_err(network_error)?
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
        images.push(image_to_data_url(item, &proxy)?);
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
    let permit = IMAGE_GENERATION_JOBS
        .try_acquire()
        .map_err(|_| "image.errors.busy")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        generate(request, &paths)
    })
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

fn decode_data_url(value: &str) -> Result<(Vec<u8>, String), String> {
    let value = value.trim();
    let (meta, data) = value
        .split_once(',')
        .ok_or_else(|| "image.errors.response".to_owned())?;
    if !meta.starts_with("data:") || !meta.contains(";base64") {
        return Err("image.errors.response".into());
    }
    let mime = meta
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return Err("image.errors.response".into());
    }
    let extension = match mime.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| "image.errors.response".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.responseTooLarge".into());
    }
    Ok((bytes, extension.to_owned()))
}

fn is_safe_image_extension(path: &std::path::Path) -> bool {
    is_gallery_image(path)
}

/// Resolve a user-chosen save path and reject empty / relative / extension-less targets.
fn resolve_save_path(path: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("image.errors.save".into());
    }
    let target = std::path::PathBuf::from(trimmed);
    if target.as_os_str().is_empty() || !is_safe_image_extension(&target) {
        return Err("image.errors.save".into());
    }
    // Reject parent-directory escapes in the literal path before touching the FS.
    if target
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("image.errors.save".into());
    }
    Ok(target)
}

fn ensure_inside_gallery(
    paths: &CodexPaths,
    target: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let directory = gallery_dir(paths);
    std::fs::create_dir_all(&directory).map_err(|_| "image.errors.save".to_owned())?;
    let canonical_dir = directory
        .canonicalize()
        .map_err(|_| "image.errors.save".to_owned())?;
    let canonical_target = target
        .canonicalize()
        .map_err(|_| "image.errors.notFound".to_owned())?;
    if !canonical_target.starts_with(&canonical_dir) || !canonical_target.is_file() {
        return Err("image.errors.notFound".into());
    }
    Ok(canonical_target)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedImagesPayload {
    pub directory: String,
    pub files: Vec<String>,
}

/// Write one generated image to an explicit path chosen by the user.
#[tauri::command]
pub fn save_image_file(path: String, data_url: String) -> Result<(), String> {
    let (bytes, _) = decode_data_url(&data_url)?;
    let target = resolve_save_path(&path)?;
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|_| "image.errors.save".to_owned())?;
        }
    }
    std::fs::write(&target, bytes).map_err(|_| "image.errors.save".to_owned())
}

/// Persist generated images under the app gallery folder without a save dialog.
#[tauri::command]
pub fn save_images_to_gallery(
    repo: State<'_, Mutex<Repository>>,
    data_urls: Vec<String>,
) -> Result<SavedImagesPayload, String> {
    if data_urls.is_empty() {
        return Err("image.errors.save".into());
    }
    let paths = paths(&repo)?;
    let directory = gallery_dir(&paths);
    std::fs::create_dir_all(&directory).map_err(|_| "image.errors.save".to_owned())?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f");
    let nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    let mut files = Vec::with_capacity(data_urls.len());
    for (index, data_url) in data_urls.iter().enumerate() {
        let (bytes, extension) = decode_data_url(data_url)?;
        let file_name = format!("{}-{}-{:02}.{}", stamp, nonce, index + 1, extension);
        let target = directory.join(&file_name);
        std::fs::write(&target, bytes).map_err(|_| "image.errors.save".to_owned())?;
        files.push(target.to_string_lossy().into_owned());
    }
    Ok(SavedImagesPayload {
        directory: directory.to_string_lossy().into_owned(),
        files,
    })
}

fn gallery_dir(paths: &CodexPaths) -> std::path::PathBuf {
    paths.app_data_dir.join("image-gallery")
}

fn is_gallery_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("webp") | Some("gif")
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryListPayload {
    pub directory: String,
    pub items: Vec<GalleryItem>,
    pub has_more: bool,
}

/// List local gallery images newest-first (capped to keep the UI light).
#[tauri::command]
pub fn list_gallery_images(
    repo: State<'_, Mutex<Repository>>,
) -> Result<GalleryListPayload, String> {
    const MAX_GALLERY_ITEMS: usize = 200;
    let paths = paths(&repo)?;
    let directory = gallery_dir(&paths);
    if !directory.exists() {
        return Ok(GalleryListPayload {
            directory: directory.to_string_lossy().into_owned(),
            items: Vec::new(),
            has_more: false,
        });
    }
    let mut items = Vec::new();
    let entries = std::fs::read_dir(&directory).map_err(|_| "image.errors.save".to_owned())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_gallery_image(&path) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let modified_at = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        items.push(GalleryItem {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            size: meta.len(),
            modified_at,
        });
    }
    items.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| b.name.cmp(&a.name))
    });
    let has_more = items.len() > MAX_GALLERY_ITEMS;
    items.truncate(MAX_GALLERY_ITEMS);
    Ok(GalleryListPayload {
        directory: directory.to_string_lossy().into_owned(),
        items,
        has_more,
    })
}

/// Load one gallery image as a data URL for WebView display.
fn read_gallery_image(
    paths: &CodexPaths,
    target: &std::path::Path,
) -> Result<(std::path::PathBuf, Vec<u8>), String> {
    if !is_gallery_image(target) {
        return Err("image.errors.notFound".into());
    }
    let target = ensure_inside_gallery(paths, target)?;
    if !is_gallery_image(&target) {
        return Err("image.errors.notFound".into());
    }
    let file = std::fs::File::open(&target).map_err(|_| "image.errors.save")?;
    if file.metadata().map_err(|_| "image.errors.save")?.len() > MAX_IMAGE_BYTES as u64 {
        return Err("image.errors.responseTooLarge".into());
    }
    // The bounded read also protects against a file growing after metadata().
    let bytes = read_limited(file, MAX_IMAGE_BYTES)?;
    Ok((target, bytes))
}

fn thumbnail_error(error: image::ImageError) -> String {
    match error {
        image::ImageError::Limits(_) => "image.errors.responseTooLarge",
        _ => "image.errors.response",
    }
    .into()
}

fn gallery_thumbnail(paths: &CodexPaths, target: &std::path::Path) -> Result<String, String> {
    let (_, bytes) = read_gallery_image(paths, target)?;
    let format = image::guess_format(&bytes).map_err(thumbnail_error)?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif
    ) {
        return Err("image.errors.response".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_THUMBNAIL_DIMENSION);
    limits.max_image_height = Some(MAX_THUMBNAIL_DIMENSION);
    limits.max_alloc = Some(MAX_THUMBNAIL_DECODE_BYTES);
    let mut header = image::ImageReader::with_format(Cursor::new(&bytes), format);
    header.limits(limits.clone());
    let (width, height) = header.into_dimensions().map_err(thumbnail_error)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_THUMBNAIL_PIXELS {
        return Err("image.errors.responseTooLarge".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    reader.limits(limits);
    // ImageReader::decode reserves the output buffer against max_alloc; decoder
    // internal allocations are additionally limited on a best-effort basis.
    let decoded = reader.decode().map_err(thumbnail_error)?;
    let thumbnail = if width > THUMBNAIL_EDGE || height > THUMBNAIL_EDGE {
        decoded.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
    } else {
        decoded
    };
    let mut png = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(thumbnail_error)?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png.into_inner())
    ))
}

/// Decode gallery previews off the runtime thread, with bounded concurrency.
#[tauri::command]
pub async fn load_gallery_thumbnail(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    let permit = THUMBNAIL_JOBS
        .acquire()
        .await
        .map_err(|_| "image.errors.response")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        gallery_thumbnail(&paths, std::path::Path::new(&path))
    })
    .await
    .map_err(|_| "image.errors.response".to_owned())?
}

/// Load the original image only for preview or export.
#[tauri::command]
pub fn load_gallery_image(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    let (target, bytes) = read_gallery_image(&paths, std::path::Path::new(&path))?;
    let extension = target
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "png".to_owned());
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    };
    Ok(format!("data:{};base64,{}", mime, STANDARD.encode(bytes)))
}

/// Delete one gallery image.
#[tauri::command]
pub fn delete_gallery_image(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    let target = std::path::PathBuf::from(&path);
    let target = ensure_inside_gallery(&paths, &target)?;
    std::fs::remove_file(&target).map_err(|_| "image.errors.save".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ApiProxyMode;
    use std::io::Write;

    fn image_fixture(
        paths: &CodexPaths,
        width: u32,
        height: u32,
        format: image::ImageFormat,
    ) -> std::path::PathBuf {
        let gallery = gallery_dir(paths);
        std::fs::create_dir_all(&gallery).unwrap();
        let path = gallery.join(format!("fixture.{}", format.extensions_str()[0]));
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([24, 96, 160]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        std::fs::write(&path, bytes.into_inner()).unwrap();
        path
    }

    #[test]
    fn gallery_thumbnails_support_all_gallery_formats_and_do_not_enlarge() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
            image::ImageFormat::Gif,
        ] {
            let path = image_fixture(&paths, 32, 16, format);
            let preview = gallery_thumbnail(&paths, &path).unwrap();
            let encoded = preview.strip_prefix("data:image/png;base64,").unwrap();
            let thumbnail = image::load_from_memory(&STANDARD.decode(encoded).unwrap()).unwrap();
            assert_eq!(
                (thumbnail.width(), thumbnail.height()),
                (32, 16),
                "{format:?}"
            );
        }
        let path = image_fixture(&paths, 640, 320, image::ImageFormat::Png);
        let preview = gallery_thumbnail(&paths, &path).unwrap();
        let thumbnail =
            image::load_from_memory(&STANDARD.decode(preview.split_once(',').unwrap().1).unwrap())
                .unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (256, 128));
    }

    #[test]
    fn gallery_thumbnails_reject_oversize_corrupt_and_external_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let path = image_fixture(&paths, 1, 1, image::ImageFormat::Png);
        let outside = dir.path().join("outside.png");
        std::fs::copy(&path, &outside).unwrap();
        assert_eq!(
            gallery_thumbnail(&paths, &outside).unwrap_err(),
            "image.errors.notFound"
        );
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_IMAGE_BYTES as u64 + 1).unwrap();
        drop(file);
        assert_eq!(
            gallery_thumbnail(&paths, &path).unwrap_err(),
            "image.errors.responseTooLarge"
        );
        std::fs::write(&path, b"invalid image").unwrap();
        assert_eq!(
            gallery_thumbnail(&paths, &path).unwrap_err(),
            "image.errors.response"
        );

        let wide = image_fixture(
            &paths,
            MAX_THUMBNAIL_DIMENSION + 1,
            1,
            image::ImageFormat::Png,
        );
        assert_eq!(
            gallery_thumbnail(&paths, &wide).unwrap_err(),
            "image.errors.responseTooLarge"
        );
        // A tiny GIF can advertise a huge canvas without allocating it in the fixture.
        let gif = image_fixture(&paths, 1, 1, image::ImageFormat::Gif);
        let mut bytes = std::fs::read(&gif).unwrap();
        bytes[6..8].copy_from_slice(&4097_u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&4096_u16.to_le_bytes());
        std::fs::write(&gif, bytes).unwrap();
        assert_eq!(
            gallery_thumbnail(&paths, &gif).unwrap_err(),
            "image.errors.responseTooLarge"
        );
    }

    fn local_image_server(
        responses: Vec<String>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut requests = Vec::new();
            for response in responses {
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fixture request timed out"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    assert!(bytes.len() < 16 * 1024);
                }
                let headers = String::from_utf8(bytes).unwrap();
                let size = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(size < 64 * 1024);
                let mut body = vec![0; size];
                stream.read_exact(&mut body).unwrap();
                requests.push(headers);
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, worker)
    }

    fn image_response(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    #[test]
    fn generation_downloads_through_the_effective_proxy_without_provider_auth() {
        let generated =
            serde_json::json!({"data":[{"url":"http://8.8.8.8/generated.png"}]}).to_string();
        let (proxy_url, server) = local_image_server(vec![
            image_response(&generated),
            image_response("image-fixture"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(
            &paths.settings_path,
            serde_json::json!({"apiProxy":{"mode":"manual","url":proxy_url}}).to_string(),
        )
        .unwrap();
        let result = generate(
            ImageGenerationRequest {
                provider_id: None,
                base_url: "http://image-provider.invalid/".into(),
                api_key: "sk-download-fixture".into(),
                model: "fixture-image".into(),
                prompt: "fixture".into(),
                size: default_size(),
                quality: default_quality(),
                count: 1,
                image_data: None,
            },
            &paths,
        )
        .unwrap();
        assert_eq!(
            result.images,
            vec![format!(
                "data:image/png;base64,{}",
                STANDARD.encode(b"image-fixture")
            )]
        );
        let requests = server.join().unwrap();
        assert!(
            requests[0].starts_with("POST http://image-provider.invalid/v1/images/generations ")
        );
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-download-fixture"));
        assert!(requests[1].starts_with("GET http://8.8.8.8/generated.png "));
        assert!(!requests[1].to_ascii_lowercase().contains("authorization:"));
        assert!(!requests[1].contains("sk-download-fixture"));
    }

    #[test]
    fn image_download_redirects_are_not_followed() {
        let (proxy_url, server) = local_image_server(vec!["HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()]);
        let config = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some(proxy_url),
        };
        assert_eq!(
            image_to_data_url(
                &serde_json::json!({"url":"http://8.8.8.8/image.png"}),
                &config
            )
            .unwrap_err(),
            "image.errors.network"
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn isolated_direct_image_download_probe() {
        let Ok(url) = std::env::var("AGENTHARBOR_TEST_DIRECT_IMAGE_URL") else {
            return;
        };
        let config = newapi::effective_proxy(
            "direct",
            &ApiProxyConfigPayload {
                mode: ApiProxyMode::Manual,
                url: Some("http://127.0.0.1:1".into()),
            },
        );
        // URL/DNS validation rejects local production downloads; this isolated
        // transport test uses a local origin to prove inherited proxies are off.
        let response = download_client_builder(&config)
            .unwrap()
            .build()
            .unwrap()
            .get(url)
            .send()
            .unwrap();
        assert_eq!(response.text().unwrap(), "direct-fixture");
    }

    #[test]
    fn direct_image_transport_ignores_manual_and_environment_proxy() {
        let (origin_url, server) = local_image_server(vec![image_response("direct-fixture")]);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "commands::image::tests::isolated_direct_image_download_probe",
            "--nocapture",
        ]);
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            child.env(name, "http://127.0.0.1:1");
        }
        child.env("NO_PROXY", "").env("no_proxy", "").env(
            "AGENTHARBOR_TEST_DIRECT_IMAGE_URL",
            format!("{origin_url}/image.png"),
        );
        let outcome = child.output().unwrap();
        assert!(
            outcome.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&outcome.stdout),
            String::from_utf8_lossy(&outcome.stderr)
        );
        assert!(server.join().unwrap()[0].starts_with("GET /image.png "));
    }

    #[test]
    fn decode_data_url_requires_base64_and_image_mime() {
        let ok = decode_data_url("data:image/png;base64,aGVsbG8=").unwrap();
        assert_eq!(ok.1, "png");
        assert_eq!(
            decode_data_url("data:text/plain;base64,aGVsbG8=").unwrap_err(),
            "image.errors.response"
        );
        assert_eq!(
            decode_data_url("data:image/png,rawbytes").unwrap_err(),
            "image.errors.response"
        );
        assert_eq!(
            decode_data_url("not-a-data-url").unwrap_err(),
            "image.errors.response"
        );
    }

    #[test]
    fn save_path_rejects_traversal_and_unknown_extension() {
        assert!(resolve_save_path("/tmp/ok.png").is_ok());
        assert_eq!(
            resolve_save_path("../escape.png").unwrap_err(),
            "image.errors.save"
        );
        assert_eq!(
            resolve_save_path("C:\\tmp\\x.exe").unwrap_err(),
            "image.errors.save"
        );
        assert_eq!(resolve_save_path("").unwrap_err(), "image.errors.save");
    }

    #[test]
    fn gallery_reads_and_deletes_stay_inside_gallery() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let gallery = gallery_dir(&paths);
        std::fs::create_dir_all(&gallery).unwrap();
        let inside = gallery.join("ok.png");
        std::fs::write(&inside, b"png").unwrap();
        assert!(ensure_inside_gallery(&paths, &inside).is_ok());

        let outside = dir.path().join("outside.png");
        std::fs::write(&outside, b"png").unwrap();
        assert_eq!(
            ensure_inside_gallery(&paths, &outside).unwrap_err(),
            "image.errors.notFound"
        );
    }

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
                image_to_data_url(&item, &ApiProxyConfigPayload::default()).unwrap_err(),
                "image.errors.response"
            );
        }
    }
}
