//! Publishes the configured model catalog to Cursor.
use axum::{
    body::{Body, Bytes},
    extract::{Extension, State},
    http::{header, HeaderValue, Request, Response, StatusCode},
};
use bytes::{BufMut, BytesMut};
use prost::Message;

use crate::{
    api::cursor::proxy::{self, CursorProxy},
    cursor::{protocol::proto::agent::v1 as agent, transport::TransportRegistry},
    model::{format_token_count, parse_token_count, ModelConfig},
    Error, Result,
};

#[derive(Clone, PartialEq, Message)]
struct AvailableModelsAddition {
    #[prost(string, repeated, tag = "1")]
    model_names: Vec<String>,
    #[prost(message, repeated, tag = "2")]
    models: Vec<AvailableModel>,
}

#[derive(Clone, PartialEq, Message)]
struct AvailableModel {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(bool, tag = "2")]
    default_on: bool,
    #[prost(bool, optional, tag = "5")]
    supports_agent: Option<bool>,
    #[prost(int32, optional, tag = "6")]
    degradation_status: Option<i32>,
    #[prost(message, optional, tag = "8")]
    tooltip_data: Option<TooltipData>,
    #[prost(bool, optional, tag = "9")]
    supports_thinking: Option<bool>,
    #[prost(bool, optional, tag = "10")]
    supports_images: Option<bool>,
    #[prost(bool, optional, tag = "14")]
    supports_max_mode: Option<bool>,
    #[prost(string, optional, tag = "17")]
    client_display_name: Option<String>,
    #[prost(string, optional, tag = "18")]
    server_model_name: Option<String>,
    #[prost(bool, optional, tag = "19")]
    supports_non_max_mode: Option<bool>,
    #[prost(message, optional, tag = "20")]
    tooltip_data_for_max_mode: Option<TooltipData>,
    #[prost(bool, optional, tag = "21")]
    is_recommended_for_background_composer: Option<bool>,
    #[prost(bool, optional, tag = "22")]
    supports_plan_mode: Option<bool>,
    #[prost(string, optional, tag = "24")]
    inputbox_short_model_name: Option<String>,
    #[prost(bool, optional, tag = "25")]
    supports_sandboxing: Option<bool>,
    #[prost(bool, optional, tag = "26")]
    supports_cmd_k: Option<bool>,
    #[prost(message, repeated, tag = "29")]
    parameter_definitions: Vec<ModelParameterDefinition>,
    #[prost(message, repeated, tag = "30")]
    variants: Vec<ModelVariant>,
    #[prost(string, repeated, tag = "36")]
    legacy_slugs: Vec<String>,
    #[prost(int32, optional, tag = "38")]
    named_model_section_index: Option<i32>,
    #[prost(string, optional, tag = "41")]
    vendor_name: Option<String>,
    #[prost(message, optional, tag = "42")]
    vendor: Option<AvailableModelVendor>,
    #[prost(message, repeated, tag = "48")]
    model_picker_badges: Vec<ModelPickerBadge>,
}

#[derive(Clone, PartialEq, Message)]
struct TooltipData {
    #[prost(string, optional, tag = "7")]
    markdown_content: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct ModelParameterDefinition {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, optional, tag = "3")]
    markdown_tooltip: Option<String>,
    #[prost(message, optional, tag = "4")]
    parameter_type: Option<ModelParameterType>,
    #[prost(bool, optional, tag = "5")]
    is_cycleable_by_hotkey: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct ModelParameterType {
    #[prost(message, optional, tag = "1")]
    boolean_parameter: Option<BooleanParameter>,
    #[prost(message, optional, tag = "2")]
    enum_parameter: Option<EnumParameter>,
}

#[derive(Clone, PartialEq, Message)]
struct BooleanParameter {
    #[prost(message, repeated, tag = "1")]
    values: Vec<BooleanParameterValue>,
}

#[derive(Clone, PartialEq, Message)]
struct BooleanParameterValue {
    #[prost(string, tag = "1")]
    value: String,
    #[prost(string, optional, tag = "2")]
    display_name: Option<String>,
    #[prost(bool, optional, tag = "3")]
    increases_model_cost: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct EnumParameter {
    #[prost(message, repeated, tag = "1")]
    values: Vec<EnumParameterValue>,
}

#[derive(Clone, PartialEq, Message)]
struct EnumParameterValue {
    #[prost(string, tag = "1")]
    value: String,
    #[prost(string, optional, tag = "2")]
    display_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct ModelVariant {
    #[prost(message, repeated, tag = "1")]
    parameter_values: Vec<ModelParameterValue>,
    #[prost(string, tag = "2")]
    display_name: String,
    #[prost(bool, tag = "3")]
    is_max_mode: bool,
    #[prost(bool, optional, tag = "4")]
    is_default_max_config: Option<bool>,
    #[prost(bool, optional, tag = "5")]
    is_default_non_max_config: Option<bool>,
    #[prost(message, optional, tag = "6")]
    tooltip_data: Option<TooltipData>,
    #[prost(string, optional, tag = "8")]
    display_name_outside_picker: Option<String>,
    #[prost(string, optional, tag = "9")]
    variant_string_representation: Option<String>,
    #[prost(string, optional, tag = "11")]
    legacy_slug: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct ModelParameterValue {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(string, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct ModelPickerBadge {
    #[prost(string, tag = "1")]
    label: String,
    #[prost(int32, tag = "2")]
    variant: i32,
    #[prost(bool, tag = "3")]
    dismiss_on_selection: bool,
}

#[derive(Clone, PartialEq, Message)]
struct AvailableModelVendor {
    #[prost(int32, tag = "1")]
    id: i32,
    #[prost(string, tag = "2")]
    display_name: String,
}

#[derive(Clone, PartialEq, Message)]
struct UsableModelsAddition {
    #[prost(message, repeated, tag = "1")]
    models: Vec<agent::ModelDetails>,
}

#[derive(Clone, PartialEq, Message)]
struct DefaultModelNudgeAddition {
    #[prost(string, repeated, tag = "3")]
    models_with_no_default_switch: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
struct DefaultModelNudgeDataResponse {
    #[prost(string, tag = "1")]
    nudge_date: String,
    #[prost(bool, tag = "2")]
    should_default_switch_on_new_chat: bool,
    #[prost(string, repeated, tag = "3")]
    models_with_no_default_switch: Vec<String>,
    #[prost(string, tag = "4")]
    conversion_model_override: String,
}

const CLI_LOCAL_MODEL_API_KEY: &str = "agentharbor-local";

const CONTEXTS: [(&str, &str); 5] = [
    ("200k", "200K"),
    ("356k", "356K"),
    ("500k", "500K"),
    ("800k", "800K"),
    ("1m", "1M"),
];
/// How Cursor's picker names a thinking level.
fn effort_name(effort: &str) -> String {
    match effort {
        "none" => "Off".into(),
        "minimal" => "Minimal".into(),
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra High".into(),
        "max" => "Max".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}
/// The context a model starts on when its own window is unknown.
const DEFAULT_CONTEXT: &str = "200k";

/// The context a model starts on: its own window, as the option that names
/// it, else the fallback.
fn default_context(contexts: &[(String, String)], context_window_tokens: Option<u64>) -> &str {
    context_window_tokens
        .and_then(|tokens| contexts.iter().find(|(value, _)| parse_token_count(value) == Some(tokens)))
        .map_or(DEFAULT_CONTEXT, |(value, _)| value.as_str())
}

/// The contexts a model offers: the presets its window holds, then the
/// window itself when no preset names it. An unknown window offers them all.
fn context_options(context_window_tokens: Option<u64>) -> Vec<(String, String)> {
    let mut contexts = CONTEXTS
        .into_iter()
        .filter(|(value, _)| {
            context_window_tokens.is_none_or(|tokens| parse_token_count(value).is_some_and(|preset| preset <= tokens))
        })
        .map(|(value, display_name)| (value.to_owned(), display_name.to_owned()))
        .collect::<Vec<_>>();
    if let Some(tokens) = context_window_tokens {
        let duplicate = contexts
            .iter()
            .any(|(existing, _)| parse_token_count(existing) == Some(tokens));
        if !duplicate {
            contexts.push((tokens.to_string(), format!("{} (Full)", format_token_count(tokens))));
        }
    }
    contexts
}

pub async fn available_models(
    State(registry): State<TransportRegistry>,
    Extension(proxy): Extension<CursorProxy>,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let models = registry.host().models();
    tracing::info!(model_count = models.len(), "appending AgentHarbor models to Cursor AvailableModels");
    let local = AvailableModelsAddition {
        model_names: models.iter().map(|model| model.model_hash.clone()).collect(),
        models: models.iter().map(available_model).collect(),
    }
    .encode_to_vec();
    if super::account::request_uses_local_cursor_token(request.headers()) {
        super::account::consume_body(request).await?;
        return Ok(local_response(local));
    }
    match proxy::forward_buffered(&proxy, request).await {
        Ok(upstream) => merge_response(upstream, local),
        Err(error) => {
            tracing::warn!(%error, "Cursor AvailableModels upstream unavailable; using local catalog");
            Ok(local_response(local))
        }
    }
}

pub async fn usable_models(
    State(registry): State<TransportRegistry>,
    Extension(proxy): Extension<CursorProxy>,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let models = registry.host().models();
    tracing::info!(model_count = models.len(), "appending AgentHarbor models to Cursor GetUsableModels");
    let local = UsableModelsAddition { models: models.iter().map(usable_model).collect() }.encode_to_vec();
    if super::account::request_uses_local_cursor_token(request.headers()) {
        super::account::consume_body(request).await?;
        return Ok(local_response(local));
    }
    match proxy::forward_buffered(&proxy, request).await {
        Ok(upstream) => merge_response(upstream, local),
        Err(error) => {
            tracing::warn!(%error, "Cursor GetUsableModels upstream unavailable; using local catalog");
            Ok(local_response(local))
        }
    }
}

/// Cursor's own nudge, with AgentHarbor's models kept from being switched
/// away from on a new chat.
pub async fn default_model_nudge(
    State(registry): State<TransportRegistry>,
    Extension(proxy): Extension<CursorProxy>,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let models = registry.host().models();
    if super::account::request_uses_local_cursor_token(request.headers()) {
        super::account::consume_body(request).await?;
        return Ok(local_response(DefaultModelNudgeDataResponse {
            nudge_date: "0".into(),
            should_default_switch_on_new_chat: false,
            models_with_no_default_switch: models.iter().map(|model| model.model_hash.clone()).collect(),
            conversion_model_override: String::new(),
        }.encode_to_vec()));
    }
    let local = DefaultModelNudgeAddition {
        models_with_no_default_switch: models.iter().map(|model| model.model_hash.clone()).collect(),
    }
    .encode_to_vec();
    match proxy::forward_buffered(&proxy, request).await {
        Ok(upstream) => merge_response(upstream, local),
        Err(error) => {
            tracing::warn!(%error, "Cursor GetDefaultModelNudgeData upstream unavailable");
            Ok(local_response(
                DefaultModelNudgeDataResponse {
                    nudge_date: "0".into(),
                    should_default_switch_on_new_chat: false,
                    models_with_no_default_switch: models.iter().map(|model| model.model_hash.clone()).collect(),
                    conversion_model_override: String::new(),
                }
                .encode_to_vec(),
            ))
        }
    }
}

fn merge_response(upstream: proxy::BufferedResponse, extra: Vec<u8>) -> Result<Response<Body>> {
    if !upstream.status.is_success() {
        tracing::warn!(status = %upstream.status, "Cursor model catalog upstream rejected request; using local catalog");
        return Ok(local_response(extra));
    }
    let (framed, payload) = unary_payload(&upstream.body)?;
    let body = if framed {
        let mut merged = BytesMut::with_capacity(5 + payload.len() + extra.len());
        merged.put_u8(0);
        merged.put_u32((payload.len() + extra.len()) as u32);
        merged.extend_from_slice(payload);
        merged.extend_from_slice(&extra);
        merged.freeze()
    } else {
        let mut merged = BytesMut::with_capacity(payload.len() + extra.len());
        merged.extend_from_slice(payload);
        merged.extend_from_slice(&extra);
        merged.freeze()
    };
    Ok(upstream.with_body(body))
}

pub(crate) fn local_response(body: Vec<u8>) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/proto"),
    );
    response
}

pub(crate) fn unary_payload(body: &Bytes) -> Result<(bool, &[u8])> {
    if body.len() < 5 {
        return Ok((false, body));
    }
    let flags = body[0];
    let length = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    if length != body.len() - 5 {
        return Ok((false, body));
    }
    if flags != 0 {
        return Err(Error::Protocol(format!(
            "cannot merge compressed or terminal model catalog frame: flags={flags}"
        )));
    }
    Ok((true, &body[5..]))
}

fn available_model(model: &ModelConfig) -> AvailableModel {
    let contexts = context_options(model.context_window_tokens);
    let tooltip = model_tooltip(model);
    let efforts = model
        .efforts
        .iter()
        .map(|effort| (effort.clone(), effort_name(effort)))
        .collect::<Vec<_>>();
    let default_effort = model.default_effort.as_deref();
    let default_context = default_context(&contexts, model.context_window_tokens);
    let variants = model_variants(
        &model.model_hash,
        &model.display_name,
        &tooltip,
        &contexts,
        default_context,
        &efforts,
        default_effort,
    );
    let legacy_slugs = variants
        .iter()
        .filter_map(|variant| variant.legacy_slug.clone())
        .collect();
    AvailableModel {
        name: model.model_hash.clone(),
        default_on: true,
        supports_agent: Some(true),
        degradation_status: Some(0),
        tooltip_data: Some(tooltip.clone()),
        supports_thinking: Some(!efforts.is_empty()),
        supports_images: Some(model.supports_images),
        supports_max_mode: Some(true),
        client_display_name: Some(model.display_name.clone()),
        server_model_name: Some(model.model_hash.clone()),
        supports_non_max_mode: Some(true),
        tooltip_data_for_max_mode: Some(tooltip),
        is_recommended_for_background_composer: Some(false),
        supports_plan_mode: Some(true),
        inputbox_short_model_name: Some(model.display_name.clone()),
        supports_sandboxing: Some(true),
        supports_cmd_k: Some(false),
        parameter_definitions: model_parameters(&contexts, &efforts),
        variants,
        legacy_slugs,
        named_model_section_index: Some(1),
        vendor_name: Some("cursor".into()),
        vendor: Some(AvailableModelVendor {
            id: 6,
            display_name: "Cursor".into(),
        }),
        model_picker_badges: vec![ModelPickerBadge {
            label: model
                .group_name
                .clone()
                .unwrap_or_else(|| provider_host(&model.base_url)),
            variant: 1,
            dismiss_on_selection: false,
        }],
    }
}

/// The badge when the model names no group: its endpoint's host.
fn provider_host(base_url: &str) -> String {
    reqwest::Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_else(|| base_url.trim().into())
}

fn model_parameters(
    contexts: &[(String, String)],
    efforts: &[(String, String)],
) -> Vec<ModelParameterDefinition> {
    let mut parameters = vec![ModelParameterDefinition {
        id: "context".into(),
        name: "Context".into(),
        markdown_tooltip: Some("Context size used to trigger conversation compaction.".into()),
        parameter_type: Some(ModelParameterType {
            boolean_parameter: None,
            enum_parameter: Some(EnumParameter {
                values: contexts
                    .iter()
                    .map(|(value, display_name)| EnumParameterValue {
                        value: value.clone(),
                        display_name: Some(display_name.clone()),
                    })
                    .collect(),
            }),
        }),
        is_cycleable_by_hotkey: Some(false),
    }];
    if !efforts.is_empty() {
        parameters.push(ModelParameterDefinition {
            id: "reasoning".into(),
            name: "Effort".into(),
            markdown_tooltip: Some("Effort the model uses to generate its response.".into()),
            parameter_type: Some(ModelParameterType {
                boolean_parameter: None,
                enum_parameter: Some(EnumParameter {
                    values: efforts
                        .iter()
                        .map(|(value, display_name)| EnumParameterValue {
                            value: value.clone(),
                            display_name: Some(display_name.clone()),
                        })
                        .collect(),
                }),
            }),
            is_cycleable_by_hotkey: Some(true),
        });
    }
    parameters.push(ModelParameterDefinition {
        id: "fast".into(),
        name: "Fast".into(),
        markdown_tooltip: Some("Significantly faster but consumes more usage".into()),
        parameter_type: Some(ModelParameterType {
            boolean_parameter: Some(BooleanParameter {
                values: vec![
                    BooleanParameterValue {
                        value: "false".into(),
                        display_name: None,
                        increases_model_cost: None,
                    },
                    BooleanParameterValue {
                        value: "true".into(),
                        display_name: Some("Fast".into()),
                        increases_model_cost: Some(true),
                    },
                ],
            }),
            enum_parameter: None,
        }),
        is_cycleable_by_hotkey: Some(false),
    });
    parameters
}

fn model_variants(
    name: &str,
    display_name: &str,
    tooltip: &TooltipData,
    contexts: &[(String, String)],
    default_context: &str,
    efforts: &[(String, String)],
    default_effort: Option<&str>,
) -> Vec<ModelVariant> {
    // A model without thinking levels has no Effort axis: Context × Fast only.
    let efforts: Vec<Option<(&str, &str)>> = if efforts.is_empty() {
        vec![None]
    } else {
        efforts.iter().map(|(value, name)| Some((value.as_str(), name.as_str()))).collect()
    };
    let default_effort = default_effort
        .filter(|wanted| efforts.iter().flatten().any(|(value, _)| value == wanted))
        .or_else(|| efforts.iter().flatten().find(|(value, _)| *value == "high").map(|(value, _)| *value))
        .or_else(|| efforts.iter().flatten().last().map(|(value, _)| *value));
    let mut variants = Vec::with_capacity(contexts.len() * efforts.len() * 2);
    for (context, context_name) in contexts {
        for effort in &efforts {
            for fast in [false, true] {
                variants.push(model_variant(
                    name,
                    display_name,
                    tooltip,
                    context,
                    context_name,
                    default_context,
                    *effort,
                    default_effort,
                    fast,
                ));
            }
        }
    }
    variants
}

fn model_variant(
    name: &str,
    display_name: &str,
    tooltip: &TooltipData,
    context: &str,
    context_name: &str,
    default_context: &str,
    effort: Option<(&str, &str)>,
    default_effort: Option<&str>,
    fast: bool,
) -> ModelVariant {
    let mut suffix = Vec::with_capacity(3);
    if context != default_context {
        suffix.push(context_name);
    }
    if let Some((_, effort_name)) = effort {
        suffix.push(effort_name);
    }
    if fast {
        suffix.push("Fast");
    }
    let suffix = suffix.join(" ");
    let display_name = if suffix.is_empty() {
        display_name.to_owned()
    } else {
        format!(
            "{display_name} <span style=\"color: var(--cursor-text-tertiary);\">{suffix}</span>"
        )
    };
    let is_default = context == default_context
        && !fast
        && effort.is_none_or(|(effort, _)| Some(effort) == default_effort);
    let mut parameter_values = vec![ModelParameterValue {
        id: "context".into(),
        value: context.into(),
    }];
    if let Some((effort, _)) = effort {
        parameter_values.push(ModelParameterValue {
            id: "reasoning".into(),
            value: effort.into(),
        });
    }
    parameter_values.push(ModelParameterValue {
        id: "fast".into(),
        value: fast.to_string(),
    });
    ModelVariant {
        parameter_values,
        display_name: display_name.clone(),
        is_max_mode: false,
        is_default_max_config: is_default.then_some(true),
        is_default_non_max_config: is_default.then_some(true),
        tooltip_data: Some(tooltip.clone()),
        display_name_outside_picker: Some(display_name),
        variant_string_representation: Some(match effort {
            Some((effort, _)) => {
                format!("{name}[context={context},reasoning={effort},fast={fast}]")
            }
            None => format!("{name}[context={context},fast={fast}]"),
        }),
        legacy_slug: Some(format!(
            "{name}-{context}{}{}",
            effort
                .map(|(effort, _)| format!("-{effort}"))
                .unwrap_or_default(),
            if fast { "-fast" } else { "" }
        )),
    }
}

fn model_tooltip(model: &ModelConfig) -> TooltipData {
    TooltipData {
        markdown_content: Some(model.tooltip_data.clone()),
    }
}

fn cli_local_model_credentials() -> agent::model_details::Credentials {
    agent::model_details::Credentials::ApiKeyCredentials(agent::ApiKeyCredentials {
        api_key: CLI_LOCAL_MODEL_API_KEY.into(),
        base_url: None,
    })
}

fn usable_model(model: &ModelConfig) -> agent::ModelDetails {
    agent::ModelDetails {
        model_id: model.model_hash.clone(),
        display_model_id: model.model_hash.clone(),
        display_name: model.display_name.clone(),
        display_name_short: model.display_name.clone(),
        thinking_details: Some(agent::ThinkingDetails::default()),
        credentials: Some(cli_local_model_credentials()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModelType, OPENAI_CHAT_ENDPOINT};

    fn model() -> ModelConfig {
        ModelConfig {
            model_hash: "local-model-hash".into(),
            sort_order: 0,
            display_name: "Local Model".into(),
            group_name: None,
            model_type: ModelType::OpenAi,
            base_url: "https://provider.example/v1/chat/completions".into(),
            use_full_url: true,
            api_key: "provider-secret".into(),
            tooltip_data: "Local Model".into(),
            model_id: "upstream-model".into(),
            reasoning_effort: None,
            openai_endpoint: OPENAI_CHAT_ENDPOINT.into(),
            openai_extra_params_enabled: false,
            openai_extra_params: serde_json::json!({}),
            custom_headers_enabled: false,
            custom_headers: serde_json::json!({}),
            anthropic_extra_params_enabled: false,
            anthropic_extra_params: serde_json::json!({}),
            context_window_tokens: None,
            max_completion_tokens: None,
            anthropic_max_tokens: None,
            anthropic_thinking_effort: None,
            thinking_budget_tokens: None,
            supports_images: true,
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("high".into()),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn a_model_starts_on_its_own_context_window() {
        let default_variant = |tokens: Option<u64>| {
            let model = ModelConfig { context_window_tokens: tokens, ..model() };
            let variants = available_model(&model).variants;
            let defaults: Vec<_> = variants.iter().filter(|variant| variant.is_default_non_max_config == Some(true)).collect();
            assert_eq!(defaults.len(), 1);
            let context = defaults[0].parameter_values.iter().find(|value| value.id == "context").unwrap().value.clone();
            (context, defaults[0].display_name.clone())
        };

        // A custom window (DeepSeek's 1048576) is the default, not 200K.
        let (context, name) = default_variant(Some(1_048_576));
        assert_eq!(context, "1048576");
        assert_eq!(name, "Local Model <span style=\"color: var(--cursor-text-tertiary);\">High</span>");
        // A window that matches a preset picks the preset.
        assert_eq!(default_variant(Some(1_000_000)).0, "1m");
        // An unknown window falls back to 200K.
        assert_eq!(default_variant(None).0, "200k");
    }

    #[test]
    fn contexts_stop_at_the_model_window() {
        let values = |tokens: Option<u64>| -> Vec<(String, String)> { context_options(tokens) };
        let names = |tokens| values(tokens).into_iter().map(|(_, name)| name).collect::<Vec<_>>();
        assert_eq!(names(Some(1_048_576)), ["200K", "356K", "500K", "800K", "1M", "1048576 (Full)"]);
        assert_eq!(names(Some(400_000)), ["200K", "356K", "400K (Full)"]);
        assert_eq!(names(Some(128_000)), ["128K (Full)"]);
        assert_eq!(names(Some(1_000_000)), ["200K", "356K", "500K", "800K", "1M"]);
        assert_eq!(names(None), ["200K", "356K", "500K", "800K", "1M"]);
    }

    #[test]
    fn cli_model_details_use_local_routing_credentials() {
        let details = usable_model(&model());
        assert_eq!(details.model_id, "local-model-hash");
        assert_eq!(details.display_name, "Local Model");
        let agent::model_details::Credentials::ApiKeyCredentials(credentials) =
            details.credentials.expect("API credentials")
        else {
            panic!("expected API key credentials");
        };
        assert_eq!(credentials.api_key, CLI_LOCAL_MODEL_API_KEY);
        assert_eq!(credentials.base_url, None);
        assert_ne!(credentials.api_key, "provider-secret");
    }
}
