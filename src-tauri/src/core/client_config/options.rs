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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeOneM {
    pub startup: bool,
    pub sonnet: bool,
    pub opus: bool,
    pub fable: bool,
    pub haiku: bool,
    pub subagent: bool,
}

impl Default for ClaudeOneM {
    fn default() -> Self {
        Self {
            startup: true,
            sonnet: true,
            opus: true,
            fable: true,
            haiku: true,
            subagent: true,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientOptions {
    pub access_mode: ClientAccessMode,
    /// Additive typed selection; old provider lists/journals retain their semantics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection: Option<crate::core::model_targets::ClientModelSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_ingress_protocol: Option<ModelProtocol>,
    pub provider_ids: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub provider_model_ids: HashMap<String, Vec<String>>,
    pub claude_models: ClaudeModels,
    // Explicit false values must survive saving now that missing values mean on.
    pub claude_one_m: ClaudeOneM,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_model_ids: Option<Vec<String>>,
    pub auth_field: ClaudeAuthField,
    /// The DeepSeek Harness profile (`$DSH_HOME/profiles/<name>`) written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_profile: Option<String>,
    /// Whether Harness's default model for new sessions is managed too;
    /// absent means yes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_default_model: Option<bool>,
}

impl ConfigRequest {
    pub fn needs_router(&self) -> bool {
        self.options.access_mode == ClientAccessMode::Router
    }

    pub fn aggregate_target(&self) -> Option<&str> {
        match self
            .options
            .model_selection
            .as_ref()?
            .default_target
            .as_ref()?
        {
            crate::core::model_targets::ModelTargetRef::Aggregate { aggregate_id } => {
                Some(aggregate_id)
            }
            _ => None,
        }
    }

    pub(super) fn aggregate_protocol(&self) -> ModelProtocol {
        if self.target_id == TargetId::ClaudeCode {
            ModelProtocol::AnthropicMessages
        } else {
            self.options
                .aggregate_ingress_protocol
                .unwrap_or(ModelProtocol::Responses)
        }
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
        } else if self.target_id == TargetId::Harness {
            super::harness::PROTOCOLS.contains(&protocol)
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
    client_endpoint_for_protocol(
        store,
        provider,
        request,
        with_version,
        provider.protocol_for_model(&request.model),
    )
}

/// A mixed-protocol client registers each SDK group at that group's upstream
/// root; its currently selected model must not change another group's URL.
pub(super) fn client_endpoint_for_protocol<'a>(
    store: &'a Store,
    provider: &'a Provider,
    request: &ConfigRequest,
    with_version: bool,
    protocol: ModelProtocol,
) -> Result<(String, &'a str), String> {
    if request.needs_router() {
        let version = if with_version { "/v1" } else { "" };
        let source = router_source_prefix(request.target_id);
        Ok((
            format!(
                "http://127.0.0.1:{}{source}/p/{}{version}",
                store.router.port, provider.id,
            ),
            &store.router.bearer_token,
        ))
    } else if provider.router_only() {
        Err("accountSources.errors.routerOnly".into())
    } else {
        Ok((
            native_base(provider, with_version, protocol)?,
            native_key(&provider.api_key),
        ))
    }
}

/// The card's key as a client sends it itself: a pasted `Bearer ` prefix is
/// dropped, as the router and Claude Desktop drop it, or the client would
/// send `Bearer Bearer …`.
fn native_key(key: &str) -> &str {
    let key = key.trim();
    key.strip_prefix("Bearer ").or_else(|| key.strip_prefix("bearer ")).map(str::trim).filter(|key| !key.is_empty()).unwrap_or(key)
}

pub(super) fn router_source_prefix(target: TargetId) -> &'static str {
    match target {
        TargetId::ClaudeCode => "/clients/claude-code",
        TargetId::OpenCode => "/clients/opencode",
        TargetId::Codex => "/clients/codex",
        TargetId::Pi => "/clients/pi",
        TargetId::Harness => "/clients/harness",
    }
}

fn native_base(
    provider: &Provider,
    with_version: bool,
    protocol: ModelProtocol,
) -> Result<String, String> {
    let base = super::super::newapi::protocol_base_url(&provider.base_url, protocol)?;
    if with_version {
        Ok(super::super::newapi::provider_endpoint(base.as_str(), "v1")
            .map_err(|_| "newapi.errors.url")?
            .to_string())
    } else {
        Ok(base.as_str().trim_end_matches('/').to_owned())
    }
}
