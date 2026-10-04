//! Shared router lifecycle: a Codex toggle owns only the Codex config, not the server.
use super::{client_config, newapi, router, router_proxy};
use crate::platform::paths::CodexPaths;
use std::sync::OnceLock;

static MUTATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub async fn mutation_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MUTATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

pub async fn ensure_available(paths: &CodexPaths) -> Result<(), String> {
    let store = router::load_store(paths)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    router_proxy::ensure_running(store.router.port, paths.clone(), proxy)
        .await
        .map(|_| ())
}

/// Point every client that uses this process's router back at its own settings.
/// Used on exit, and at startup when the router cannot take its port: a client
/// left pointing at a port held by another process would send it the local token.
pub fn release_router_clients(paths: &CodexPaths) {
    release_router_clients_at(paths, client_config::ClientPaths::discover());
}

fn release_router_clients_at(paths: &CodexPaths, clients: client_config::ClientPaths) {
    let manager = client_config::ConfigManager::new(&paths.app_data_dir, clients)
        .with_official_home(&paths.codex_home);
    for (target, error) in manager.restore_router_configs_on_exit() {
        log::warn!("{target:?} router config cleanup failed: {error}");
    }
    if let Err(error) = router::stop_smart_routing_on_exit(paths) {
        log::warn!("Codex router config cleanup failed: {error}");
    }
}

/// Startup reconcile. If the router cannot bind its port, the clients configured
/// to use it are released instead of being left aimed at that port.
pub async fn reconcile_on_startup(paths: &CodexPaths) -> Result<(), String> {
    reconcile_on_startup_at(paths, client_config::ClientPaths::discover()).await
}

async fn reconcile_on_startup_at(
    paths: &CodexPaths,
    clients: client_config::ClientPaths,
) -> Result<(), String> {
    let result = reconcile(paths).await;
    if result.as_ref().err().map(String::as_str) == Some("newapi.errors.portBusy") {
        release_router_clients_at(paths, clients);
    }
    result
}

pub async fn reconcile(paths: &CodexPaths) -> Result<(), String> {
    let store = router::load_store(paths)?;
    if needs_server(
        &store,
        client_config::has_router_configs(&paths.app_data_dir)?,
        router::active_routing_config_is_ours(paths),
    ) {
        ensure_available(paths).await
    } else {
        router_proxy::stop();
        Ok(())
    }
}

fn needs_server(
    store: &router::Store,
    other_clients_active: bool,
    codex_route_active: bool,
) -> bool {
    other_clients_active
        || (store.router.enabled
            && codex_route_active
            && store.codex_access_mode == router::CodexAccessMode::Router)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a crash, the next start finds the client still aimed at the router.
    /// If another process holds that port, the client is restored instead.
    #[test]
    fn startup_releases_router_clients_when_the_port_is_taken() {
        let _router = router_proxy::global_router_test_lock();
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let clients = client_config::ClientPaths::from_home(&temp.path().join("home"));
            let squatter = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();

            let mut store = router::load_store(&paths).unwrap();
            store.router.port = squatter.local_addr().unwrap().port();
            store.providers.push(router::Provider {
                id: "relay".into(),
                name: "Relay".into(),
                base_url: "https://relay.example/v1".into(),
                enabled: true,
                model: "claude-test".into(),
                api_key: "upstream-secret".into(),
                protocol: router::ModelProtocol::AnthropicMessages,
                ..Default::default()
            });
            crate::core::secret_file::write(
                &paths.app_data_dir.join("newapi-providers.dat"),
                &serde_json::to_vec(&store).unwrap(),
            )
            .unwrap();
            let store = router::load_store(&paths).unwrap();

            let original = r#"{"env":{"CUSTOM":"kept"}}"#;
            crate::core::storage::write_atomic(&clients.claude, original.as_bytes()).unwrap();
            let manager = client_config::ConfigManager::new(&paths.app_data_dir, clients.clone())
                .with_official_home(&paths.codex_home);
            let request = client_config::ConfigRequest {
                target_id: crate::core::targets::TargetId::ClaudeCode,
                provider_id: "relay".into(),
                model: "claude-test".into(),
                context_window: Some(200000),
                max_output_tokens: Some(8192),
                options: client_config::ClientOptions {
                    access_mode: client_config::ClientAccessMode::Router,
                    ..Default::default()
                },
            };
            let preview = manager.preview(&store, &request).unwrap();
            manager.apply(&store, &request, &preview.revision).unwrap();
            assert!(std::fs::read_to_string(&clients.claude)
                .unwrap()
                .contains("127.0.0.1"));

            let result = reconcile_on_startup_at(&paths, clients.clone()).await;
            assert_eq!(result.unwrap_err(), "newapi.errors.portBusy");
            assert_eq!(std::fs::read_to_string(&clients.claude).unwrap(), original);
            assert!(!client_config::has_router_configs(&paths.app_data_dir).unwrap());
            drop(squatter);
        });
    }

    #[test]
    fn native_codex_does_not_require_the_server_but_other_clients_do() {
        let mut store = router::Store::default();
        assert!(!needs_server(&store, false, false));
        store.router.enabled = true;
        assert!(!needs_server(&store, false, false));
        assert!(needs_server(&store, false, true));
        store.codex_access_mode = router::CodexAccessMode::Native;
        assert!(!needs_server(&store, false, true));
        assert!(needs_server(&store, true, false));
        store.router.enabled = false;
        assert!(needs_server(&store, true, false));
    }
}
