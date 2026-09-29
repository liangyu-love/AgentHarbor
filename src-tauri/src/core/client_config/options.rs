use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientAccessMode {
    #[default]
    Native,
    Router,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeAuthField {
    #[default]
    AuthToken,
    ApiKey,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeModels {
    pub sonnet: String,
    pub opus: String,
    pub fable: String,
    pub haiku: String,
    pub subagent: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeOneM {
    pub startup: bool,
    pub sonnet: bool,
    pub opus: bool,
    pub fable: bool,
    pub subagent: bool,
}

impl ClaudeOneM {
    pub fn is_empty(&self) -> bool {
        !self.startup && !self.sonnet && !self.opus && !self.fable && !self.subagent
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientOptions {
    pub access_mode: ClientAccessMode,
    pub provider_ids: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub provider_model_ids: HashMap<String, Vec<String>>,
    pub claude_models: ClaudeModels,
    #[serde(skip_serializing_if = "ClaudeOneM::is_empty")]
    pub claude_one_m: ClaudeOneM,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_model_ids: Option<Vec<String>>,
    pub auth_field: ClaudeAuthField,
}

impl ConfigRequest {
    pub fn needs_router(&self) -> bool {
        self.options.access_mode == ClientAccessMode::Router
    }

    pub(super) fn accepts_model(&self, provider: &Provider, model: &str) -> bool {
        if !provider.supports_model(model) {
            return false;
        }
        let protocol = provider.protocol_for_model(model);
        if self.target_id == TargetId::ClaudeCode && self.needs_router() {
            super::super::protocol::Translation::can_serve(
                ModelProtocol::AnthropicMessages,
                protocol,
            )
        } else {
            adapter(self.target_id).is_ok_and(|adapter| adapter.protocols().contains(&protocol))
        }
    }

    pub(super) fn models(&self, provider: &Provider) -> Vec<String> {
        let mut models = Vec::new();
        for id in std::iter::once(&provider.model).chain(
            provider
                .models
                .iter()
                .filter(|model| model.included)
                .map(|model| &model.id),
        ) {
            if !id.is_empty() && self.accepts_model(provider, id) && !models.contains(id) {
                models.push(id.clone());
            }
        }
        models
    }
}

pub(super) fn validate_model(model: &str) -> Result<(), String> {
    if model.trim().is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err("clients.errors.model".into());
    }
    Ok(())
}

/// Base URL and credential a client uses for `provider`: the local router's
/// per-provider path and bearer token in router mode, otherwise the provider itself.
/// `with_version` appends `/v1` for clients (OpenCode) whose SDKs expect it.
pub(super) fn client_endpoint<'a>(
    store: &'a Store,
    provider: &'a Provider,
    request: &ConfigRequest,
    with_version: bool,
) -> Result<(String, &'a str), String> {
    if request.needs_router() {
        let version = if with_version { "/v1" } else { "" };
        Ok((
            format!(
                "http://127.0.0.1:{}/p/{}{version}",
                store.router.port, provider.id
            ),
            &store.router.bearer_token,
        ))
    } else {
        Ok((native_base(provider, with_version)?, &provider.api_key))
    }
}

fn native_base(provider: &Provider, with_version: bool) -> Result<String, String> {
    let base = super::super::newapi::base_url(&provider.base_url)?;
    if with_version {
        Ok(
            super::super::newapi::provider_endpoint(&provider.base_url, "v1")
                .map_err(|_| "newapi.errors.url")?
                .to_string(),
        )
    } else {
        Ok(base.as_str().trim_end_matches('/').to_owned())
    }
}
