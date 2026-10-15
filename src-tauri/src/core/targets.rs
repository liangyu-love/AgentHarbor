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
        let global = super::newapi::proxy_from_settings(&self.paths.settings_path);
        let store = super::router::load_store(&self.paths).map_err(CoreError::OperationFailed)?;
        let proxy = codex_process_proxy(&store, &global);
        platform::process::restart_codex_app(&proxy, &self.paths)
    }
}

fn codex_process_proxy(
    store: &super::router::Store,
    global: &super::models::ApiProxyConfigPayload,
) -> super::models::ApiProxyConfigPayload {
    if store.router.enabled && store.codex_access_mode == super::router::CodexAccessMode::Native {
        if let Some(provider) = super::router::effective_active(store) {
            return super::newapi::effective_proxy(provider.network_mode(), global);
        }
    }
    global.clone()
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
    fn native_process_uses_selected_provider_egress_while_router_preserves_global_egress() {
        use crate::core::{
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
            network_mode: "direct".into(),
            ..Default::default()
        });
        assert_eq!(
            codex_process_proxy(&store, &global).mode,
            ApiProxyMode::Direct
        );
        store.providers[0].network_mode = "proxy".into();
        assert_eq!(codex_process_proxy(&store, &global), global);
        store.providers[0].network_mode = "direct".into();
        store.codex_access_mode = CodexAccessMode::Router;
        assert_eq!(codex_process_proxy(&store, &global), global);
        store.codex_access_mode = CodexAccessMode::Native;
        store.router.enabled = false;
        assert_eq!(codex_process_proxy(&store, &global), global);
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
