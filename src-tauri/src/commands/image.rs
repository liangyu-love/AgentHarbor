use crate::core::{newapi, repository::Repository, router};
use crate::platform::paths::CodexPaths;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::multipart;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use tauri::State;

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

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

fn response_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .bytes()
        .map_err(|_| "image.errors.network".to_owned())?;
    if body.len() > 25 * 1024 * 1024 {
        return Err("image.errors.responseTooLarge".into());
    }
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

fn image_to_data_url(client: &reqwest::blocking::Client, value: &Value) -> Result<String, String> {
    if let Some(encoded) = value.get("b64_json").and_then(Value::as_str) {
        return Ok(format!("data:image/png;base64,{encoded}"));
    }
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or("image.errors.response")?;
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
    let bytes = response
        .bytes()
        .map_err(|_| "image.errors.network".to_owned())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image.errors.responseTooLarge".into());
    }
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
        if request.base_url.trim().is_empty() || request.api_key.trim().is_empty() {
            return Err("image.errors.config".into());
        }
        (
            newapi::Connection {
                base_url: newapi::base_url(&request.base_url)?.to_string(),
                api_key: request.api_key.trim().to_owned(),
                model: request.model.clone(),
                ..Default::default()
            },
            "proxy".to_owned(),
        )
    };
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    let proxy = newapi::effective_proxy(&network_mode, &proxy);
    let client = newapi::client(&proxy)?;
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
    let mut images = Vec::with_capacity(data.len());
    let mut revised_prompts = Vec::with_capacity(data.len());
    for item in data {
        images.push(image_to_data_url(&client, item)?);
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
