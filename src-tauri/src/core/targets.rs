use super::models::CoreError;
use crate::{platform, platform::paths::CodexPaths};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetId {
    Codex,
    #[serde(rename = "claudeCode")]
    ClaudeCode,
    OpenCode,
    Pi,
    /// DeepSeek Harness (`dsh`), configured through a profile's Cordis patch.
    Harness,
}

/// Paths for the Codex catalog-backed conversation store. Claude Code uses its
/// separate JSONL reader; it does not have an archive directory or SQLite catalog.
pub trait ConversationSource {
    fn sessions_dir(&self) -> &Path;
    fn archived_sessions_dir(&self) -> &Path;
    fn state_database_path(&self) -> &Path;
}

pub trait ProcessController {
    fn restart(&self) -> Result<(), CoreError>;
}

#[derive(Clone)]
pub struct CodexTarget {
    paths: CodexPaths,
}

impl CodexTarget {
    pub fn new(paths: CodexPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &CodexPaths {
        &self.paths
    }
}

impl ConversationSource for CodexTarget {
    fn sessions_dir(&self) -> &Path {
        &self.paths.sessions_dir
    }

    fn archived_sessions_dir(&self) -> &Path {
        &self.paths.archived_sessions_dir
    }

    fn state_database_path(&self) -> &Path {
        &self.paths.codex_state_db_path
    }
}

impl ProcessController for CodexTarget {
    fn restart(&self) -> Result<(), CoreError> {
        let store = super::router::load_store(&self.paths).map_err(CoreError::OperationFailed)?;
        let network = codex_process_network(&store, &self.global_proxy());
        platform::process::restart_codex_app(&network, &self.paths)
    }
}

impl CodexTarget {
    fn global_proxy(&self) -> super::models::ApiProxyConfigPayload {
        super::newapi::proxy_from_settings(&self.paths.settings_path)
    }

    /// Starts Codex on the network its connection calls for.
    pub fn launch(&self) -> Result<(), CoreError> {
        let global = self.global_proxy();
        let network = match super::router::load_store(&self.paths) {
            Ok(store) => codex_process_network(&store, &global),
            Err(error) => {
                log::warn!("Codex starts on the app proxy; the provider store is unreadable: {error}");
                global.into()
            }
        };
        super::codex_config::prepare_network_config(&self.paths, &network.proxy)?;
        platform::process::launch_codex_app(&network)
    }
}

/// Codex reaching a relay itself (native access) skips the proxy for a direct
/// card's relay alone; its other traffic (sign-in, MCP, plugins) keeps it.
/// Routed, Codex reaches only the local router, which picks each card's exit.
fn codex_process_network(
    store: &super::router::Store,
    global: &super::models::ApiProxyConfigPayload,
) -> super::api_client::ProcessNetwork {
    if store.router.enabled && store.codex_access_mode == super::router::CodexAccessMode::Native {
        if let Some(provider) = super::router::effective_active(store) {
            if provider.network_mode() == "direct" {
                return match provider.relay_host() {
                    Some(host) => super::api_client::ProcessNetwork::bypassing(global, vec![host]),
                    None => super::newapi::effective_proxy("direct", global).into(),
                };
            }
        }
    }
    global.clone().into()
}

pub struct TargetRegistry {
    codex: CodexTarget,
}

impl TargetRegistry {
    pub fn new(codex_paths: CodexPaths) -> Self {
        Self {
            codex: CodexTarget::new(codex_paths),
        }
    }

    pub fn codex(&self) -> &CodexTarget {
        &self.codex
    }

    pub fn conversations(&self, id: TargetId) -> Option<&dyn ConversationSource> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }

    pub fn process(&self, id: TargetId) -> Option<&dyn ProcessController> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_process_skips_the_proxy_for_a_direct_relay_only() {
        use crate::core::{
            api_client::ProcessNetwork,
            models::{ApiProxyConfigPayload, ApiProxyMode},
            router::{CodexAccessMode, Provider, Store},
        };
        let global = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("http://127.0.0.1:9000".into()),
        };
        let mut store = Store::default();
        store.router.enabled = true;
        store.codex_access_mode = CodexAccessMode::Native;
        store.providers.push(Provider {
            id: "selected".into(),
            enabled: true,
            api_key: "fixture-key".into(),
            model: "fixture-model".into(),
            base_url: "https://relay.example:8443/v1".into(),
            network_mode: "direct".into(),
            ..Default::default()
        });
        let network = codex_process_network(&store, &global);
        assert_eq!(network.proxy, global, "everything else keeps the proxy");
        assert_eq!(network.direct_hosts, ["relay.example"]);
        store.providers[0].network_mode = "proxy".into();
        assert_eq!(codex_process_network(&store, &global), ProcessNetwork::from(global.clone()));
        store.providers[0].network_mode = "direct".into();
        store.codex_access_mode = CodexAccessMode::Router;
        assert_eq!(codex_process_network(&store, &global), ProcessNetwork::from(global.clone()));
        store.codex_access_mode = CodexAccessMode::Native;
        store.router.enabled = false;
        assert_eq!(codex_process_network(&store, &global), ProcessNetwork::from(global.clone()));
        store.router.enabled = true;
        store.providers[0].base_url = "not a url".into();
        assert_eq!(codex_process_network(&store, &global).proxy.mode, ApiProxyMode::Direct);
    }

    #[test]
    fn configuration_only_clients_do_not_fall_back_to_codex_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TargetRegistry::new(CodexPaths::from_home(dir.path().join(".codex")));
        for id in [TargetId::ClaudeCode, TargetId::OpenCode] {
            assert!(registry.process(id).is_none());
            assert!(registry.conversations(id).is_none());
        }
    }

    #[test]
    fn codex_registers_each_supported_capability() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join(".codex"));
        let registry = TargetRegistry::new(paths.clone());

        assert_eq!(
            registry
                .conversations(TargetId::Codex)
                .unwrap()
                .sessions_dir(),
            paths.sessions_dir
        );
        assert!(registry.process(TargetId::Codex).is_some());
    }
}
