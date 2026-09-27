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
pub struct ClientOptions {
    pub access_mode: ClientAccessMode,
    pub provider_ids: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub provider_model_ids: HashMap<String, Vec<String>>,
    pub claude_models: ClaudeModels,
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
            super::super::protocol::Translation::between(ModelProtocol::AnthropicMessages, protocol)
                .map_or(true, |pair| pair.is_supported())
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

pub(super) fn native_base(provider: &Provider, with_version: bool) -> Result<String, String> {
    let base = super::super::newapi::base_url(&provider.base_url)?;
    if with_version {
        Ok(base
            .join("v1")
            .map_err(|_| "newapi.errors.url")?
            .to_string())
    } else {
        Ok(base.as_str().trim_end_matches('/').to_owned())
    }
}
