use crate::core::{
    api_client, models::ApiProxyConfigPayload, newapi, repository::Repository, router, secret_file,
    storage,
};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::State;

pub(super) const MAX_PROMPT_CHARS: usize = 32_000;
/// A high-quality 8 MP PNG (3840x2160, or what `auto` picks) runs past 30 MB;
/// rejecting it would discard an image that was already paid for.
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Room for `count` base64 images plus the JSON around them.
fn max_response_bytes(count: u8) -> usize {
    usize::from(count.max(1)) * MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1024 * 1024
}
const THUMBNAIL_EDGE: u32 = 256;
const MAX_THUMBNAIL_DIMENSION: u32 = 8192;
const MAX_THUMBNAIL_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_THUMBNAIL_DECODE_BYTES: u64 = 128 * 1024 * 1024;
static THUMBNAIL_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static IMAGE_GENERATION_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static IMAGE_KEYS_LOCK: Mutex<()> = Mutex::new(());

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "image.errors.timeout".into()
    } else {
        "image.errors.network".into()
    }
}

/// Sites with a remembered key; the oldest is forgotten beyond this.
const MAX_SAVED_KEYS: usize = 20;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedImageKey {
    base_url: String,
    api_key: String,
}

/// Most recently used site first.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SavedImageKeys {
    sites: Vec<SavedImageKey>,
}

/// Earlier builds stored a single key object; it reads as a one-site list.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredImageKeys {
    Sites(SavedImageKeys),
    Single(SavedImageKey),
}

fn key_path(paths: &CodexPaths) -> std::path::PathBuf {
    paths.app_data_dir.join("image-workbench-key.dat")
}

fn read_keys(paths: &CodexPaths) -> Result<SavedImageKeys, String> {
    let Some(bytes) = secret_file::read(&key_path(paths)).map_err(|_| "image.errors.storage")?
    else {
        return Ok(SavedImageKeys::default());
    };
    Ok(
        match serde_json::from_slice(&bytes).map_err(|_| "image.errors.storage")? {
            StoredImageKeys::Sites(keys) => keys,
            StoredImageKeys::Single(key) => SavedImageKeys { sites: vec![key] },
        },
    )
}

fn write_keys(paths: &CodexPaths, keys: &SavedImageKeys) -> Result<(), String> {
    let bytes = serde_json::to_vec(keys).map_err(|_| "image.errors.storage")?;
    secret_file::write(&key_path(paths), &bytes).map_err(|_| "image.errors.storage".to_owned())
}

fn normalized_site(base_url: &str) -> Result<String, String> {
    Ok(newapi::base_url(base_url)
        .map_err(|_| "image.errors.config")?
        .to_string())
}

fn read_key(paths: &CodexPaths, base_url: &str) -> Result<Option<String>, String> {
    let base_url = normalized_site(base_url)?;
    Ok(read_keys(paths)?
        .sites
        .into_iter()
        .find(|site| site.base_url == base_url)
        .map(|site| site.api_key))
}

fn write_key(paths: &CodexPaths, base_url: &str, api_key: &str) -> Result<(), String> {
    let base_url = normalized_site(base_url)?;
    newapi::auth_header(api_key).map_err(|_| "image.errors.apiKey")?;
    let _guard = IMAGE_KEYS_LOCK.lock().map_err(|_| "image.errors.storage")?;
    let mut keys = read_keys(paths)?;
    keys.sites.retain(|site| site.base_url != base_url);
    keys.sites.insert(
        0,
        SavedImageKey {
            base_url,
            api_key: api_key.trim().to_owned(),
        },
    );
    keys.sites.truncate(MAX_SAVED_KEYS);
    write_keys(paths, &keys)
}

fn delete_key(paths: &CodexPaths, base_url: &str) -> Result<(), String> {
    let base_url = normalized_site(base_url)?;
    let _guard = IMAGE_KEYS_LOCK.lock().map_err(|_| "image.errors.storage")?;
    let mut keys = read_keys(paths)?;
    let before = keys.sites.len();
    keys.sites.retain(|site| site.base_url != base_url);
    if keys.sites.len() == before {
        return Ok(());
    }
    write_keys(paths, &keys)
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

/// GPT Image 2/2.5 accepts dimensions within these documented bounds.
fn valid_size(size: &str) -> bool {
    if size == "auto" {
        return true;
    }
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    if !width.bytes().all(|byte| byte.is_ascii_digit())
        || !height.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        return false;
    };
    width > 0
        && height > 0
        && width <= 3840
        && height <= 3840
        && width % 16 == 0
        && height % 16 == 0
        && width.max(height) <= width.min(height) * 3
        && (655_360..=8_294_400).contains(&(width * height))
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
    if !valid_size(&request.size) {
        return Err("image.errors.size".into());
    }
    if !matches!(
        request.quality.as_str(),
        "auto" | "low" | "medium" | "high" | "xhigh" | "max"
    ) {
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

pub(super) fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "image.errors.storage".to_owned())?
        .paths()
        .clone())
}

fn decode_image_data(value: &str) -> Result<(Vec<u8>, String, String), String> {
    let encoded = if let Some((header, data)) = value.split_once(",") {
        if !header.starts_with("data:image/") || !header.ends_with(";base64") {
            return Err("image.errors.input".into());
        }
        data
    } else {
        value
    };
    if encoded.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("image.errors.inputTooLarge".into());
    }
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "image.errors.input".to_owned())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.inputTooLarge".into());
    }
    let (format, _) = decode_bounded_image(&bytes).map_err(|error| {
        if error == "image.errors.responseTooLarge" {
            "image.errors.inputTooLarge"
        } else {
            "image.errors.input"
        }
        .to_owned()
    })?;
    let (mime, extension) = image_format_names(format);
    Ok((bytes, mime.into(), extension.into()))
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

/// Streams the body so an oversize response is rejected without buffering all of it.
async fn read_limited_response(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > limit {
            return Err("image.errors.responseTooLarge".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn response_json(response: reqwest::Response, count: u8) -> Result<Value, String> {
    let status = response.status();
    let body = read_limited_response(response, max_response_bytes(count)).await?;
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
) -> Result<reqwest::ClientBuilder, String> {
    let builder = reqwest::Client::builder()
        // A large original over a slow link takes well over half a minute.
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    api_client::configure_async_client(builder, proxy).map_err(|_| "image.errors.network".into())
}

async fn download_client(
    url: &reqwest::Url,
    proxy: &ApiProxyConfigPayload,
) -> Result<reqwest::Client, String> {
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
        let addresses: Vec<_> = tokio::net::lookup_host((host, port))
            .await
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

async fn image_to_data_url(value: &Value, proxy: &ApiProxyConfigPayload) -> Result<String, String> {
    if let Some(encoded) = value.get("b64_json").and_then(Value::as_str) {
        if encoded.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
            return Err("image.errors.responseTooLarge".into());
        }
        let encoded = encoded.to_owned();
        return super::blocking(move || {
            let bytes = STANDARD
                .decode(encoded.trim())
                .map_err(|_| "image.errors.response")?;
            validated_image_url(bytes)
        })
        .await;
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
    let client = download_client(&url, proxy).await?;
    let response = client.get(url).send().await.map_err(network_error)?;
    if !response.status().is_success() {
        return Err("image.errors.network".into());
    }
    let bytes = read_limited_response(response, MAX_IMAGE_BYTES).await?;
    super::blocking(move || validated_image_url(bytes)).await
}

fn image_format_names(format: image::ImageFormat) -> (&'static str, &'static str) {
    match format {
        image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        image::ImageFormat::WebP => ("image/webp", "webp"),
        image::ImageFormat::Gif => ("image/gif", "gif"),
        _ => ("image/png", "png"),
    }
}

fn validated_image_url(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.responseTooLarge".into());
    }
    let (format, _) = decode_bounded_image(&bytes)?;
    let (mime, _) = image_format_names(format);
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

async fn generate(
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
    let client = api_client::configure_async_client(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none()),
        &proxy,
    )
    .map_err(|_| "newapi.errors.proxy")?
    .build()
    .map_err(|_| "newapi.errors.network")?;
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
        let image_data = image_data.to_owned();
        let (bytes, mime, extension) =
            super::blocking(move || decode_image_data(&image_data)).await?;
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
        form = form.part("image[]", part);
        client
            .post(endpoint)
            .header(AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .await
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
            .await
            .map_err(network_error)?
    };
    let value = response_json(response, request.count).await?;
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
        images.push(image_to_data_url(item, &proxy).await?);
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

/// Cancel handles of running generations, keyed by the caller's job id.
fn generation_jobs() -> &'static Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>> =
        OnceLock::new();
    JOBS.get_or_init(Default::default)
}

/// Removes the job's cancel handle however the generation ends.
struct JobRegistration(Option<String>);

impl Drop for JobRegistration {
    fn drop(&mut self) {
        if let Some(id) = self.0.take() {
            if let Ok(mut jobs) = generation_jobs().lock() {
                jobs.remove(&id);
            }
        }
    }
}

async fn run_generation(
    job_id: Option<String>,
    request: ImageGenerationRequest,
    paths: CodexPaths,
) -> Result<ImageGenerationResponse, String> {
    let _permit = IMAGE_GENERATION_JOBS
        .try_acquire()
        .map_err(|_| "image.errors.busy")?;
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let _registration = match job_id {
        Some(id) if !id.is_empty() && id.len() <= 64 => {
            let mut jobs = generation_jobs()
                .lock()
                .map_err(|_| "image.errors.storage")?;
            if jobs.contains_key(&id) {
                return Err("image.errors.busy".into());
            }
            jobs.insert(id.clone(), cancel_tx);
            JobRegistration(Some(id))
        }
        _ => JobRegistration(None),
    };
    // Dropping the losing future closes its connections, so a cancelled job stops
    // uploading/downloading at once and frees its slot.
    tokio::select! {
        result = generate(request, &paths) => result,
        Ok(()) = cancel_rx => Err("image.errors.cancelled".into()),
    }
}

#[tauri::command]
pub async fn generate_images(
    repo: State<'_, Mutex<Repository>>,
    request: ImageGenerationRequest,
    job_id: Option<String>,
) -> Result<ImageGenerationResponse, String> {
    let paths = paths(&repo)?;
    run_generation(job_id, request, paths).await
}

/// Returns false when the job already finished or never existed.
#[tauri::command]
pub fn cancel_image_generation(job_id: String) -> bool {
    let sender = generation_jobs()
        .lock()
        .ok()
        .and_then(|mut jobs| jobs.remove(&job_id));
    sender.is_some_and(|sender| sender.send(()).is_ok())
}

/// Sites with a saved key, most recently used first. Keys themselves never leave Rust.
#[tauri::command]
pub fn list_image_key_sites(repo: State<'_, Mutex<Repository>>) -> Result<Vec<String>, String> {
    Ok(read_keys(&paths(&repo)?)?
        .sites
        .into_iter()
        .map(|site| site.base_url)
        .collect())
}

#[tauri::command]
pub fn delete_image_key(
    repo: State<'_, Mutex<Repository>>,
    base_url: String,
) -> Result<(), String> {
    delete_key(&paths(&repo)?, &base_url)
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
    if !meta.starts_with("data:image/") || !meta.ends_with(";base64") {
        return Err("image.errors.response".into());
    }
    if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("image.errors.responseTooLarge".into());
    }
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| "image.errors.response".to_owned())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.responseTooLarge".into());
    }
    let (format, _) = decode_bounded_image(&bytes)?;
    Ok((bytes, image_format_names(format).1.to_owned()))
}

/// Resolve a user-chosen save path and reject empty / relative / extension-less targets.
fn resolve_save_path(path: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("image.errors.save".into());
    }
    let target = std::path::PathBuf::from(trimmed);
    if !target.is_absolute() || !is_gallery_image(&target) {
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
pub async fn save_image_file(path: String, data_url: String) -> Result<(), String> {
    super::blocking(move || {
        let (bytes, _) = decode_data_url(&data_url)?;
        let target = resolve_save_path(&path)?;
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|_| "image.errors.save".to_owned())?;
            }
        }
        std::fs::write(&target, bytes).map_err(|_| "image.errors.save".to_owned())
    })
    .await
}

/// How an image was made, kept beside it in a same-named `.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryMeta {
    pub prompt: String,
    /// The prompt as the image API rewrote it, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub quality: String,
}

/// What a generation hands over with its images; each image may have its own revised prompt.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationMeta {
    pub prompt: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub revised_prompts: Vec<String>,
}

/// The sidecar file. The marker tells it apart from a user's own JSON in a
/// gallery that points at one of their folders, so deletes never touch theirs.
#[derive(Serialize, Deserialize)]
struct GallerySidecar {
    agentharbor: u32,
    #[serde(flatten)]
    meta: GalleryMeta,
}

const SIDECAR_VERSION: u32 = 1;
const MAX_SIDECAR_BYTES: u64 = 256 * 1024;

fn sidecar_path(image: &std::path::Path) -> std::path::PathBuf {
    image.with_extension("json")
}

fn clip(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

fn meta_for_image(meta: &GenerationMeta, index: usize) -> GalleryMeta {
    let revised = meta
        .revised_prompts
        .get(index)
        .map(|text| clip(text, MAX_PROMPT_CHARS))
        .filter(|text| !text.is_empty() && *text != meta.prompt.trim());
    GalleryMeta {
        prompt: clip(&meta.prompt, MAX_PROMPT_CHARS),
        revised_prompt: revised,
        model: clip(&meta.model, 200),
        size: clip(&meta.size, 32),
        quality: clip(&meta.quality, 32),
    }
}

fn write_sidecar(image: &std::path::Path, meta: GalleryMeta) -> Result<(), String> {
    let sidecar = GallerySidecar { agentharbor: SIDECAR_VERSION, meta };
    let contents = serde_json::to_vec_pretty(&sidecar).map_err(|_| "image.errors.save".to_owned())?;
    std::fs::write(sidecar_path(image), contents).map_err(|_| "image.errors.save".to_owned())
}

/// The record beside `image`, if there is one of ours.
fn read_sidecar(image: &std::path::Path) -> Option<GalleryMeta> {
    let path = sidecar_path(image);
    if std::fs::metadata(&path).ok()?.len() > MAX_SIDECAR_BYTES {
        return None;
    }
    let sidecar: GallerySidecar = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (sidecar.agentharbor >= 1 && !sidecar.meta.prompt.is_empty()).then_some(sidecar.meta)
}

fn save_to_gallery(
    paths: &CodexPaths,
    data_urls: &[String],
    meta: Option<&GenerationMeta>,
) -> Result<SavedImagesPayload, String> {
    if data_urls.is_empty() || data_urls.len() > 4 {
        return Err("image.errors.save".into());
    }
    // Validate the entire batch before creating any gallery file.
    let images = data_urls
        .iter()
        .map(|data_url| decode_data_url(data_url))
        .collect::<Result<Vec<_>, _>>()?;
    let directory = gallery_dir(paths);
    std::fs::create_dir_all(&directory).map_err(|_| "image.errors.save".to_owned())?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f");
    let nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    let mut files = Vec::with_capacity(data_urls.len());
    for (index, (bytes, extension)) in images.iter().enumerate() {
        let file_name = format!("{}-{}-{:02}.{}", stamp, nonce, index + 1, extension);
        let target = directory.join(&file_name);
        std::fs::write(&target, bytes).map_err(|_| "image.errors.save".to_owned())?;
        // The image is what matters; a missing record only loses its prompt.
        if let Some(meta) = meta.filter(|meta| !meta.prompt.trim().is_empty()) {
            if let Err(error) = write_sidecar(&target, meta_for_image(meta, index)) {
                log::warn!("gallery prompt record not written for {file_name}: {error}");
            }
        }
        files.push(target.to_string_lossy().into_owned());
    }
    Ok(SavedImagesPayload {
        directory: directory.to_string_lossy().into_owned(),
        files,
    })
}

/// Persist generated images under the app gallery folder without a save dialog,
/// each with a record of the prompt and parameters that made it.
#[tauri::command]
pub async fn save_images_to_gallery(
    repo: State<'_, Mutex<Repository>>,
    data_urls: Vec<String>,
    meta: Option<GenerationMeta>,
) -> Result<SavedImagesPayload, String> {
    let paths = paths(&repo)?;
    super::blocking(move || save_to_gallery(&paths, &data_urls, meta.as_ref())).await
}

#[tauri::command]
pub async fn set_gallery_directory(
    repo: State<'_, Mutex<Repository>>,
    directory: String,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    super::blocking(move || persist_gallery_directory(&paths, std::path::Path::new(&directory)))
        .await
}

fn persist_gallery_directory(
    paths: &CodexPaths,
    directory: &std::path::Path,
) -> Result<String, String> {
    let canonical = directory
        .canonicalize()
        .map_err(|_| "image.errors.save".to_owned())?;
    if !canonical.is_dir() {
        return Err("image.errors.save".into());
    }
    let setting = paths.app_data_dir.join("image-gallery-directory.json");
    let contents = serde_json::to_vec(&canonical.to_string_lossy().to_string())
        .map_err(|_| "image.errors.save".to_owned())?;
    storage::write_atomic(&setting, &contents).map_err(|_| "image.errors.save".to_owned())?;
    Ok(canonical.to_string_lossy().into_owned())
}

fn gallery_dir(paths: &CodexPaths) -> std::path::PathBuf {
    let setting = paths.app_data_dir.join("image-gallery-directory.json");
    std::fs::read(setting)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| paths.app_data_dir.join("image-gallery"))
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
    /// How the image was made; none for images saved before records were kept.
    pub meta: Option<GalleryMeta>,
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
pub async fn list_gallery_images(
    repo: State<'_, Mutex<Repository>>,
) -> Result<GalleryListPayload, String> {
    let paths = paths(&repo)?;
    super::blocking(move || list_gallery(&paths)).await
}

fn list_gallery(paths: &CodexPaths) -> Result<GalleryListPayload, String> {
    const MAX_GALLERY_ITEMS: usize = 200;
    let directory = gallery_dir(paths);
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
            meta: None,
        });
    }
    items.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| b.name.cmp(&a.name))
    });
    let has_more = items.len() > MAX_GALLERY_ITEMS;
    items.truncate(MAX_GALLERY_ITEMS);
    // Records are read only for the images shown.
    for item in &mut items {
        item.meta = read_sidecar(std::path::Path::new(&item.path));
    }
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

fn decode_bounded_image(bytes: &[u8]) -> Result<(image::ImageFormat, image::DynamicImage), String> {
    let format = image::guess_format(bytes).map_err(thumbnail_error)?;
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
    let mut header = image::ImageReader::with_format(Cursor::new(bytes), format);
    header.limits(limits.clone());
    let (width, height) = header.into_dimensions().map_err(thumbnail_error)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_THUMBNAIL_PIXELS {
        return Err("image.errors.responseTooLarge".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    // ImageReader::decode reserves the output buffer against max_alloc; decoder
    // internal allocations are additionally limited on a best-effort basis.
    let decoded = reader.decode().map_err(thumbnail_error)?;
    Ok((format, decoded))
}

fn gallery_thumbnail(paths: &CodexPaths, target: &std::path::Path) -> Result<String, String> {
    let (_, bytes) = read_gallery_image(paths, target)?;
    let (_, decoded) = decode_bounded_image(&bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
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
pub async fn load_gallery_image(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    super::blocking(move || {
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
    })
    .await
}

/// Delete one gallery image.
#[tauri::command]
pub async fn delete_gallery_image(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    super::blocking(move || delete_from_gallery(&paths, std::path::Path::new(&path))).await
}

fn delete_from_gallery(paths: &CodexPaths, path: &std::path::Path) -> Result<(), String> {
    let target = ensure_inside_gallery(paths, path)?;
    // The gallery can point at any user directory; never delete non-image files there.
    if !is_gallery_image(&target) {
        return Err("image.errors.notFound".into());
    }
    let record = read_sidecar(&target).is_some();
    std::fs::remove_file(&target).map_err(|_| "image.errors.save".to_owned())?;
    // Only a record of ours goes with it; a user's own JSON of the same name stays.
    if record {
        let _ = std::fs::remove_file(sidecar_path(&target));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ApiProxyMode;
    use std::io::Write;

    #[test]
    fn current_image_api_parameters_validate_with_bounded_custom_dimensions() {
        let mut request = ImageGenerationRequest {
            provider_id: None,
            base_url: String::new(),
            api_key: String::new(),
            model: "gpt-image-2.5-sunburst".into(),
            prompt: "画".repeat(32_000),
            size: default_size(),
            quality: "max".into(),
            count: 1,
            image_data: None,
        };
        for size in [
            "auto",
            "1024x1024",
            "1536x864",
            "864x1536",
            "2048x2048",
            "640x1024",
            "3840x1280",
            "3840x2160",
            "2160x3840",
            "1792x1024",
        ] {
            request.size = size.into();
            assert!(validate(&request).is_ok(), "size {size}");
        }
        for size in [
            "0x1024",
            "1536x865",
            "624x1024",
            "3840x1264",
            "3856x2160",
            "3840x3840",
            "4294967296x1024",
            "+1024x1024",
            "1024X1024",
            "1024x1024x1",
        ] {
            request.size = size.into();
            assert_eq!(
                validate(&request).unwrap_err(),
                "image.errors.size",
                "size {size}"
            );
        }
        request.size = default_size();
        for quality in ["auto", "low", "medium", "high", "xhigh", "max"] {
            request.quality = quality.into();
            assert!(validate(&request).is_ok(), "quality {quality}");
        }
        request.quality = "unknown".into();
        assert_eq!(validate(&request).unwrap_err(), "image.errors.quality");
        request.quality = default_quality();
        request.prompt.push('画');
        assert_eq!(validate(&request).unwrap_err(), "image.errors.prompt");
    }

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
        responses: Vec<Vec<u8>>,
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
                requests.push(format!("{headers}{}", String::from_utf8_lossy(&body)));
                stream.write_all(&response).unwrap();
            }
            requests
        });
        (url, worker)
    }

    fn image_response(body: impl AsRef<[u8]>) -> Vec<u8> {
        let body = body.as_ref();
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        response
    }

    #[test]
    fn generation_downloads_through_the_effective_proxy_without_provider_auth() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let image = std::fs::read(image_fixture(&paths, 2, 2, image::ImageFormat::Png)).unwrap();
        let generated =
            serde_json::json!({"data":[{"url":"http://8.8.8.8/generated.png"}]}).to_string();
        let (proxy_url, server) =
            local_image_server(vec![image_response(&generated), image_response(&image)]);
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(
            &paths.settings_path,
            serde_json::json!({"apiProxy":{"mode":"manual","url":proxy_url}}).to_string(),
        )
        .unwrap();
        let result = tauri::async_runtime::block_on(generate(
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
        ))
        .unwrap();
        assert_eq!(
            result.images,
            vec![format!("data:image/png;base64,{}", STANDARD.encode(image))]
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
    fn official_image_generation_and_edit_contracts_keep_new_parameters_and_decode_images() {
        for edit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(dir.path().join("codex"));
            let image =
                std::fs::read(image_fixture(&paths, 2, 2, image::ImageFormat::Png)).unwrap();
            let encoded = STANDARD.encode(image);
            let data_url = format!("data:image/png;base64,{encoded}");
            let response = serde_json::json!({
                "created": 1792022400_u64, "output_format": "png", "size": "3840x2160",
                "quality": "max", "data": [{"b64_json": encoded}],
                "usage": {"total_tokens": 100, "input_tokens": 50, "output_tokens": 50}
            });
            let (base_url, server) = local_image_server(vec![image_response(response.to_string())]);
            std::fs::write(
                &paths.settings_path,
                serde_json::json!({"apiProxy":{"mode":"direct"}}).to_string(),
            )
            .unwrap();
            let result = tauri::async_runtime::block_on(generate(
                ImageGenerationRequest {
                    provider_id: None,
                    base_url,
                    api_key: "sk-local-image-fixture".into(),
                    model: "gpt-image-2.5-flare".into(),
                    prompt: "A local contract test".into(),
                    size: "3840x2160".into(),
                    quality: "max".into(),
                    count: 1,
                    image_data: edit.then(|| data_url.clone()),
                },
                &paths,
            ))
            .unwrap();
            assert_eq!(result.images, vec![data_url]);
            let requests = server.join().unwrap();
            let (headers, body) = requests[0].split_once("\r\n\r\n").unwrap();
            if edit {
                assert!(headers.starts_with("POST /v1/images/edits "));
                assert!(headers
                    .to_ascii_lowercase()
                    .contains("multipart/form-data; boundary="));
                assert!(body.contains("name=\"image[]\"; filename=\"input.png\""));
                for (field, value) in [
                    ("model", "gpt-image-2.5-flare"),
                    ("quality", "max"),
                    ("size", "3840x2160"),
                ] {
                    assert!(body.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n")));
                }
            } else {
                assert!(headers.starts_with("POST /v1/images/generations "));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["model"], "gpt-image-2.5-flare");
                assert_eq!(body["quality"], "max");
                assert_eq!(body["size"], "3840x2160");
                assert_eq!(body["n"], 1);
            }
        }
    }

    #[test]
    fn generated_images_validate_bytes_and_use_the_actual_format() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let proxy = ApiProxyConfigPayload {
            mode: ApiProxyMode::Direct,
            url: None,
        };
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
            image::ImageFormat::Gif,
        ] {
            let bytes = std::fs::read(image_fixture(&paths, 2, 2, format)).unwrap();
            let encoded = STANDARD.encode(&bytes);
            let result = tauri::async_runtime::block_on(image_to_data_url(
                &serde_json::json!({"b64_json": encoded}),
                &proxy,
            ))
            .unwrap();
            assert_eq!(
                result,
                format!("data:{};base64,{encoded}", image_format_names(format).0)
            );
            // An incorrect declared MIME must not change the multipart format.
            let (input, mime, extension) =
                decode_image_data(&format!("data:image/png;base64,{encoded}")).unwrap();
            assert_eq!(input, bytes);
            assert_eq!(
                (mime.as_str(), extension.as_str()),
                image_format_names(format)
            );
        }
        for encoded in [
            String::new(),
            "not base64!".into(),
            STANDARD.encode(b"<html>not an image</html>"),
        ] {
            assert_eq!(
                tauri::async_runtime::block_on(image_to_data_url(
                    &serde_json::json!({"b64_json":encoded}),
                    &proxy
                ))
                .unwrap_err(),
                "image.errors.response"
            );
        }
        let png = std::fs::read(image_fixture(&paths, 2, 2, image::ImageFormat::Png)).unwrap();
        assert!(validated_image_url(png[..24].to_vec()).is_err());
        assert_eq!(
            decode_image_data("data:text/html;base64,aGVsbG8=").unwrap_err(),
            "image.errors.input"
        );
        assert_eq!(
            decode_image_data("data:image/png,raw").unwrap_err(),
            "image.errors.input"
        );
    }

    #[test]
    fn generated_download_rejects_empty_or_fake_images_even_with_an_image_header() {
        let (proxy_url, server) = local_image_server(vec![
            image_response(""),
            image_response("<html>upstream error</html>"),
        ]);
        let proxy = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some(proxy_url),
        };
        for _ in 0..2 {
            assert_eq!(
                tauri::async_runtime::block_on(image_to_data_url(
                    &serde_json::json!({"url":"http://8.8.8.8/image.png"}),
                    &proxy
                ))
                .unwrap_err(),
                "image.errors.response"
            );
        }
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn concurrent_key_updates_preserve_all_sites() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            for index in 0..8 {
                let barrier = barrier.clone();
                let paths = &paths;
                scope.spawn(move || {
                    barrier.wait();
                    write_key(
                        paths,
                        &format!("https://s{index}.example.org"),
                        "sk-fixture",
                    )
                    .unwrap();
                });
            }
        });
        assert_eq!(read_keys(&paths).unwrap().sites.len(), 8);
    }

    #[test]
    fn image_download_redirects_are_not_followed() {
        let (proxy_url, server) = local_image_server(vec![b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()]);
        let config = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some(proxy_url),
        };
        assert_eq!(
            tauri::async_runtime::block_on(image_to_data_url(
                &serde_json::json!({"url":"http://8.8.8.8/image.png"}),
                &config
            ))
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
        let text = tauri::async_runtime::block_on(async {
            download_client_builder(&config)
                .unwrap()
                .build()
                .unwrap()
                .get(url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        assert_eq!(text, "direct-fixture");
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
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let png = std::fs::read(image_fixture(&paths, 2, 2, image::ImageFormat::Png)).unwrap();
        let ok =
            decode_data_url(&format!("data:image/png;base64,{}", STANDARD.encode(png))).unwrap();
        assert_eq!(ok.1, "png");
        assert_eq!(
            decode_data_url("data:image/png;base64,aGVsbG8=").unwrap_err(),
            "image.errors.response"
        );
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
        let absolute = std::env::temp_dir().join("ok.png");
        assert!(resolve_save_path(absolute.to_str().unwrap()).is_ok());
        assert_eq!(
            resolve_save_path("ok.png").unwrap_err(),
            "image.errors.save"
        );
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

    fn png_data_url() -> String {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([24, 96, 160])));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", STANDARD.encode(bytes.into_inner()))
    }

    #[test]
    fn gallery_images_keep_their_prompt_beside_them() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let meta = GenerationMeta {
            prompt: " 雨夜橘猫 ".into(),
            model: "gpt-image-2".into(),
            size: "1024x1024".into(),
            quality: "high".into(),
            revised_prompts: vec!["雨夜橘猫".into(), "雨夜里的一只橘猫，电影感".into()],
        };
        let saved = save_to_gallery(&paths, &[png_data_url(), png_data_url()], Some(&meta)).unwrap();
        let listed = list_gallery(&paths).unwrap();
        assert_eq!(listed.items.len(), 2, "records are not listed as images");
        let first = listed.items.iter().find(|item| item.path == saved.files[0]).unwrap();
        let second = listed.items.iter().find(|item| item.path == saved.files[1]).unwrap();
        let first_meta = first.meta.as_ref().unwrap();
        assert_eq!(first_meta.prompt, "雨夜橘猫");
        assert_eq!(first_meta.revised_prompt, None, "an unchanged prompt is not repeated");
        assert_eq!((first_meta.model.as_str(), first_meta.size.as_str(), first_meta.quality.as_str()), ("gpt-image-2", "1024x1024", "high"));
        assert_eq!(second.meta.as_ref().unwrap().revised_prompt.as_deref(), Some("雨夜里的一只橘猫，电影感"));

        delete_from_gallery(&paths, std::path::Path::new(&saved.files[0])).unwrap();
        assert!(!sidecar_path(std::path::Path::new(&saved.files[0])).exists());

        // Images saved without a record list without one.
        let plain = save_to_gallery(&paths, &[png_data_url()], None).unwrap();
        let listed = list_gallery(&paths).unwrap();
        assert!(listed.items.iter().find(|item| item.path == plain.files[0]).unwrap().meta.is_none());
    }

    #[test]
    fn deleting_an_image_leaves_a_users_own_json_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let gallery = gallery_dir(&paths);
        std::fs::create_dir_all(&gallery).unwrap();
        let image = gallery.join("photo.png");
        std::fs::write(&image, b"png").unwrap();
        std::fs::write(gallery.join("photo.json"), br#"{"prompt":"not ours"}"#).unwrap();
        assert!(read_sidecar(&image).is_none());
        delete_from_gallery(&paths, &image).unwrap();
        assert!(!image.exists());
        assert!(gallery.join("photo.json").exists());
    }

    #[test]
    fn gallery_directory_can_be_changed_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let custom = dir.path().join("my-image-library");
        std::fs::create_dir_all(&custom).unwrap();
        let selected = persist_gallery_directory(&paths, &custom).unwrap();
        let canonical = custom.canonicalize().unwrap();
        assert_eq!(gallery_dir(&paths), canonical);
        assert_eq!(selected, canonical.to_string_lossy());

        let image = gallery_dir(&paths).join("saved.png");
        std::fs::write(&image, b"png").unwrap();
        assert!(ensure_inside_gallery(&paths, &image).is_ok());
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
    fn keys_for_several_sites_coexist_and_can_be_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        write_key(&paths, "https://a.example.org", "sk-a").unwrap();
        write_key(&paths, "https://b.example.org", "sk-b").unwrap();
        assert_eq!(
            read_key(&paths, "https://a.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-a")
        );
        assert_eq!(
            read_key(&paths, "https://b.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-b")
        );
        // Re-saving moves a site to the front and replaces its key.
        write_key(&paths, "https://a.example.org/", "sk-a2").unwrap();
        let sites: Vec<_> = read_keys(&paths)
            .unwrap()
            .sites
            .into_iter()
            .map(|s| s.base_url)
            .collect();
        assert_eq!(sites, ["https://a.example.org/", "https://b.example.org/"]);
        assert_eq!(
            read_key(&paths, "https://a.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-a2")
        );

        delete_key(&paths, "https://a.example.org").unwrap();
        assert_eq!(read_key(&paths, "https://a.example.org").unwrap(), None);
        assert_eq!(
            read_key(&paths, "https://b.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-b")
        );
        delete_key(&paths, "https://missing.example.org").unwrap();
    }

    #[test]
    fn single_key_files_from_older_builds_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let legacy = serde_json::json!({"baseUrl":"https://old.example.org/","apiKey":"sk-old"});
        secret_file::write(&key_path(&paths), legacy.to_string().as_bytes()).unwrap();
        assert_eq!(
            read_key(&paths, "https://old.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-old")
        );
        write_key(&paths, "https://new.example.org", "sk-new").unwrap();
        assert_eq!(
            read_key(&paths, "https://old.example.org")
                .unwrap()
                .as_deref(),
            Some("sk-old")
        );
    }

    #[test]
    fn saved_sites_are_capped_dropping_the_least_recent() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        for index in 0..=MAX_SAVED_KEYS {
            write_key(&paths, &format!("https://s{index}.example.org"), "sk-x").unwrap();
        }
        assert_eq!(read_keys(&paths).unwrap().sites.len(), MAX_SAVED_KEYS);
        assert_eq!(read_key(&paths, "https://s0.example.org").unwrap(), None);
        assert!(
            read_key(&paths, &format!("https://s{MAX_SAVED_KEYS}.example.org"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn cancelling_stops_a_hanging_generation_and_frees_its_slot() {
        // Accepts the request and never answers. Accept polls with a deadline so the
        // test cannot hang if the connection never arrives.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}/", listener.local_addr().unwrap());
        let (connected_tx, connected_rx) = std::sync::mpsc::channel();
        let hold = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < deadline {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = connected_tx.send(());
                    std::thread::sleep(Duration::from_secs(5));
                    drop(stream);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        // Never route the fixture request through a system proxy.
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(
            &paths.settings_path,
            serde_json::json!({"apiProxy":{"mode":"direct"}}).to_string(),
        )
        .unwrap();
        let request = || ImageGenerationRequest {
            provider_id: None,
            base_url: base_url.clone(),
            api_key: "sk-cancel-fixture".into(),
            model: "fixture-image".into(),
            prompt: "fixture".into(),
            size: default_size(),
            quality: default_quality(),
            count: 1,
            image_data: None,
        };
        let started = std::time::Instant::now();
        let result = tauri::async_runtime::block_on(async {
            let job = tokio::spawn(run_generation(
                Some("job-cancel".into()),
                request(),
                paths.clone(),
            ));
            // Cancel only once the request is actually in flight upstream.
            tokio::task::spawn_blocking(move || connected_rx.recv_timeout(Duration::from_secs(10)))
                .await
                .unwrap()
                .expect("the generation request reached the fixture upstream");
            assert!(cancel_image_generation("job-cancel".into()));
            tokio::time::timeout(Duration::from_secs(10), job)
                .await
                .expect("cancelled generation settles")
                .unwrap()
        });
        assert_eq!(result.unwrap_err(), "image.errors.cancelled");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "cancel must not wait for the upstream"
        );
        assert!(!generation_jobs().lock().unwrap().contains_key("job-cancel"));
        assert!(!cancel_image_generation("job-cancel".into()));
        // Both generation slots are free again.
        assert_eq!(IMAGE_GENERATION_JOBS.available_permits(), 2);
        hold.join().unwrap();
    }

    #[test]
    fn limits_fit_the_largest_generated_image_even_uncompressed() {
        // 3840x2160 RGBA with no compression gain at all, as base64.
        let raw = 3840 * 2160 * 4;
        assert!(MAX_IMAGE_BYTES > raw);
        assert!(max_response_bytes(4) > 4 * raw.div_ceil(3) * 4);
        assert!(max_response_bytes(0) >= max_response_bytes(1));
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
                tauri::async_runtime::block_on(image_to_data_url(
                    &item,
                    &ApiProxyConfigPayload::default()
                ))
                .unwrap_err(),
                "image.errors.response"
            );
        }
    }
}
