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
}

pub trait RoutingTarget {
    fn target_id(&self) -> TargetId;
    fn config_path(&self) -> &Path;
    fn application_data_dir(&self) -> &Path;
}

pub trait ConversationSource {
    fn target_id(&self) -> TargetId;
    fn sessions_dir(&self) -> &Path;
    fn archived_sessions_dir(&self) -> &Path;
    fn state_database_path(&self) -> &Path;
}

pub trait McpConfigStore {
    fn target_id(&self) -> TargetId;
    fn config_path(&self) -> &Path;
}

pub trait SkillStore {
    fn target_id(&self) -> TargetId;
    fn skills_dir(&self) -> &Path;
}

pub trait InstructionStore {
    fn target_id(&self) -> TargetId;
    fn global_instructions_path(&self) -> &Path;
}

pub trait ProcessController {
    fn target_id(&self) -> TargetId;
    fn is_running(&self) -> bool;
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

impl RoutingTarget for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

    fn config_path(&self) -> &Path {
        &self.paths.config_path
    }

    fn application_data_dir(&self) -> &Path {
        &self.paths.app_data_dir
    }
}

impl ConversationSource for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

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

impl McpConfigStore for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

    fn config_path(&self) -> &Path {
        &self.paths.config_path
    }
}

impl SkillStore for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

    fn skills_dir(&self) -> &Path {
        &self.paths.skills_dir
    }
}

impl InstructionStore for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

    fn global_instructions_path(&self) -> &Path {
        &self.paths.global_agents_path
    }
}

impl ProcessController for CodexTarget {
    fn target_id(&self) -> TargetId {
        TargetId::Codex
    }

    fn is_running(&self) -> bool {
        platform::process::is_codex_app_running()
    }

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

    pub fn routing(&self, id: TargetId) -> Option<&dyn RoutingTarget> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }

    pub fn conversations(&self, id: TargetId) -> Option<&dyn ConversationSource> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }

    pub fn mcp(&self, id: TargetId) -> Option<&dyn McpConfigStore> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }

    pub fn skills(&self, id: TargetId) -> Option<&dyn SkillStore> {
        match id {
            TargetId::Codex => Some(&self.codex),
            _ => None,
        }
    }

    pub fn instructions(&self, id: TargetId) -> Option<&dyn InstructionStore> {
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
            assert!(registry.mcp(id).is_none());
            assert!(registry.skills(id).is_none());
            assert!(registry.instructions(id).is_none());
        }
    }

    #[test]
    fn codex_registers_each_supported_capability() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join(".codex"));
        let registry = TargetRegistry::new(paths.clone());

        assert_eq!(
            registry.routing(TargetId::Codex).unwrap().config_path(),
            paths.config_path
        );
        assert_eq!(
            registry
                .conversations(TargetId::Codex)
                .unwrap()
                .sessions_dir(),
            paths.sessions_dir
        );
        assert_eq!(
            registry.mcp(TargetId::Codex).unwrap().config_path(),
            paths.config_path
        );
        assert_eq!(
            registry.skills(TargetId::Codex).unwrap().skills_dir(),
            paths.skills_dir
        );
        assert_eq!(
            registry
                .instructions(TargetId::Codex)
                .unwrap()
                .global_instructions_path(),
            paths.global_agents_path
        );
        assert_eq!(
            registry.process(TargetId::Codex).unwrap().target_id(),
            TargetId::Codex
        );
    }
}
