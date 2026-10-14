//! Shared router lifecycle: a Codex toggle owns only the Codex config, not the server.
use super::{claude_desktop, client_config, newapi, router, router_proxy};
use crate::platform::paths::CodexPaths;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

static MUTATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
type PreviewReservations = HashMap<String, (String, Instant)>;
static PREVIEW_RESERVATIONS: OnceLock<Mutex<PreviewReservations>> = OnceLock::new();

fn reservations() -> &'static Mutex<PreviewReservations> {
    PREVIEW_RESERVATIONS.get_or_init(Default::default)
}

/// Retain a successful preview's listener until its ticket is consumed or expires.
pub fn retain_preview_listener(revision: String, reservation: String) {
    let expires =
        Instant::now() + Duration::from_secs(claude_desktop::constants::TICKET_TTL_SECONDS as u64);
    if let Some((old, _)) = reservations()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(revision, (reservation, expires))
    {
        router_proxy::release_listener_reservation(&old);
    }
}

pub fn release_preview_listener(revision: &str) {
    if let Some((id, _)) = reservations()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(revision)
    {
        router_proxy::release_listener_reservation(&id);
    }
}

pub fn release_all_preview_listeners() {
    for (_, (id, _)) in reservations()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
    {
        router_proxy::release_listener_reservation(&id);
    }
}

fn prune_preview_listeners() {
    let now = Instant::now();
    reservations()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, (id, expires)| {
            if *expires > now {
                true
            } else {
                router_proxy::release_listener_reservation(id);
                false
            }
        });
}

pub async fn mutation_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MUTATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

pub async fn ensure_available(paths: &CodexPaths) -> Result<(), String> {
    ensure_listener(paths).await.map(|_| ())
}

/// Bind before constructing a Desktop profile and retain the exact bind identity.
pub async fn ensure_listener(paths: &CodexPaths) -> Result<router_proxy::ListenerLease, String> {
    prune_preview_listeners();
    let store = router::load_store(paths)?;
    let desktop_port = claude_desktop::gateway::required_port(&paths.app_data_dir)
        .map_err(|error| error.message)?;
    router_proxy::set_desktop_pin(desktop_port)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    router_proxy::ensure_running(store.router.port, paths.clone(), proxy).await?;
    router_proxy::current_lease().ok_or_else(|| "claudeDesktop.errors.listenerChanged".into())
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
    super::cursor_inject::release_on_exit(&paths.app_data_dir);
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
    let manager = claude_desktop::Manager::new(&paths.app_data_dir);
    if let Err(error) = manager.cleanup_previews() {
        log::warn!("Claude Desktop preview cleanup failed: {}", error.message);
    }
    // Complete a committed journal or roll back an interrupted write while
    // Desktop is stopped. An unresolved journal keeps its endpoints closed.
    if let Err(error) = manager.recover_at_startup() {
        log::warn!(
            "Claude Desktop startup recovery required: {}",
            error.message
        );
    }
    // Cursor's settings and variable are pointed back at this process.
    let result = reconcile_with(paths, true).await;
    if result.as_ref().err().map(String::as_str) == Some("newapi.errors.portBusy") {
        release_router_clients_at(paths, clients);
    }
    router::normalize_threads_when_stopped(paths);
    result
}

pub async fn reconcile(paths: &CodexPaths) -> Result<(), String> {
    reconcile_with(paths, false).await
}

/// [`reconcile`], rewriting Cursor's settings and variable when
/// `cursor_leases` (at startup and when its switches change) and otherwise
/// leaving a user's own edit to them alone.
pub async fn reconcile_with(paths: &CodexPaths, cursor_leases: bool) -> Result<(), String> {
    prune_preview_listeners();
    let store = router::load_store(paths)?;
    super::aggregation::reconcile_runtime(&store);
    let manager = claude_desktop::Manager::new(&paths.app_data_dir);
    if let Err(error) = manager.cleanup_previews() {
        log::warn!("Claude Desktop preview cleanup failed: {}", error.message);
    }
    let desktop_port = claude_desktop::gateway::required_port(&paths.app_data_dir)
        .map_err(|error| error.message)?;
    router_proxy::set_desktop_pin(desktop_port)?;
    let cursor = super::cursor_inject::active(&paths.app_data_dir);
    let result = if needs_server(
        &store,
        client_config::has_router_configs(&paths.app_data_dir)? || cursor,
        router_proxy::desktop_listener_required(),
    ) {
        ensure_available(paths).await
    } else {
        router_proxy::stop();
        Ok(())
    };
    if let Err(error) = super::cursor_inject::reconcile(paths, cursor_leases).await {
        log::warn!("Cursor injection reconcile failed: {error}");
    }
    result
}

fn needs_server(
    store: &router::Store,
    other_clients_active: bool,
    desktop_required: bool,
) -> bool {
    desktop_required
        || store.router.aggregate_external_enabled
        || other_clients_active
        // Existing Codex threads can still carry this listener even if another
        // tool replaces the current config.toml provider. Keep it available
        // until the user disables routing, without reclaiming that config.
        || (store.router.enabled && store.codex_access_mode == router::CodexAccessMode::Router)
        // A running Codex keeps its previous connection until it restarts.
        || router::codex_restart_pending()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            release_all_preview_listeners();
            router_proxy::set_desktop_pin(None).unwrap();
            router_proxy::stop_unchecked();
        }
    }

    fn save_store(paths: &CodexPaths, store: &router::Store) {
        crate::core::secret_file::write(
            &paths.app_data_dir.join("newapi-providers.dat"),
            &serde_json::to_vec(store).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn preview_reservations_expire_without_stopping_another_client() {
        let _router = router_proxy::global_router_test_lock();
        let _stop = Stop;
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let mut store = router::load_store(&paths).unwrap();
            store.router.port = std::net::TcpListener::bind(("127.0.0.1", 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            store.providers.push(router::Provider {
                id: "fixture".into(),
                name: "Fixture".into(),
                enabled: true,
                api_key: "synthetic".into(),
                base_url: "https://fixture.example".into(),
                model: "claude-fixture".into(),
                protocol: router::ModelProtocol::AnthropicMessages,
                ..Default::default()
            });
            save_store(&paths, &store);
            let clients = client_config::ClientPaths::from_home(&temp.path().join("clients"));
            let manager = client_config::ConfigManager::new(&paths.app_data_dir, clients)
                .with_official_home(&paths.codex_home);
            let request = client_config::ConfigRequest {
                target_id: crate::core::targets::TargetId::ClaudeCode,
                provider_id: "fixture".into(),
                model: "claude-fixture".into(),
                context_window: None,
                max_output_tokens: None,
                options: client_config::ClientOptions {
                    access_mode: client_config::ClientAccessMode::Router,
                    ..Default::default()
                },
            };
            let preview = manager.preview(&store, &request).unwrap();
            manager.apply(&store, &request, &preview.revision).unwrap();
            let lease = ensure_listener(&paths).await.unwrap();
            let reservation = router_proxy::reserve_listener(&lease).unwrap();
            retain_preview_listener("expired".into(), reservation);
            reservations().lock().unwrap().get_mut("expired").unwrap().1 =
                Instant::now() - Duration::from_secs(1);
            reconcile(&paths).await.unwrap();
            assert!(!router_proxy::desktop_listener_required());
            assert_eq!(
                router_proxy::current_lease(),
                Some(lease.clone()),
                "CLI still owns the same listener"
            );
            assert!(manager.restore_router_configs_on_exit().is_empty());
            let reservation = router_proxy::reserve_listener(&lease).unwrap();
            retain_preview_listener("cancelled".into(), reservation);
            reconcile(&paths).await.unwrap();
            assert_eq!(router_proxy::current_lease(), Some(lease));
            release_preview_listener("cancelled");
            reconcile(&paths).await.unwrap();
            assert!(!router_proxy::is_running());
        });
    }

    #[test]
    fn managed_desktop_survives_codex_native_and_corrupt_preview_cleanup() {
        let _router = router_proxy::global_router_test_lock();
        let _stop = Stop;
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let mut store = router::load_store(&paths).unwrap();
            store.router.port = std::net::TcpListener::bind(("127.0.0.1", 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            store.router.enabled = false;
            store.codex_access_mode = router::CodexAccessMode::Native;
            save_store(&paths, &store);
            let roots = claude_desktop::paths::DesktopPaths::from_roots(
                temp.path().join("Claude"),
                temp.path().join("Claude-3p"),
            )
            .unwrap();
            let mut state = claude_desktop::transaction::next_state(
                &[],
                &[],
                &roots,
                1,
                "committed",
                "",
                "",
                None,
                &[],
                claude_desktop::transaction::AppliedMode::Router,
            )
            .unwrap();
            state.router = Some(claude_desktop::gateway::RouterConfig {
                routes: Vec::new(),
                route_views: Vec::new(),
                file_projection: String::new(),
                route_revision: 1,
                gateway_token_version: 1,
                gateway_token: "synthetic-token".into(),
                bound_port: store.router.port,
            });
            crate::core::secret_file::write(
                &paths.app_data_dir.join("claude-desktop/state.dat"),
                &serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
            crate::core::secret_file::write(
                &paths.app_data_dir.join("claude-desktop/previews.dat"),
                b"broken-ticket-json",
            )
            .unwrap();
            reconcile_on_startup_at(
                &paths,
                client_config::ClientPaths::from_home(&temp.path().join("clients")),
            )
            .await
            .unwrap();
            let lease = router_proxy::current_lease().unwrap();
            assert_eq!(lease.bound_port, store.router.port);
            assert!(router_proxy::desktop_listener_required());
            store.router.port += 1;
            save_store(&paths, &store);
            assert_eq!(
                reconcile(&paths).await.unwrap_err(),
                "claudeDesktop.errors.listenerPinned"
            );
            assert_eq!(router_proxy::current_lease(), Some(lease));
        });
    }

    /// After a crash, the next start finds the client still aimed at the router.
    /// If another process holds that port, the client is restored instead.
    #[test]
    fn startup_releases_router_clients_when_the_port_is_taken() {
        let _router = router_proxy::global_router_test_lock();
        let _stop = Stop;
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

            // A crashed Desktop gateway keeps its fixed address, token and all
            // four files while existing CLI router clients recover separately.
            let desktop_paths = claude_desktop::paths::DesktopPaths::from_roots(
                temp.path().join("Claude"),
                temp.path().join("Claude-3p"),
            )
            .unwrap();
            for file in [
                &desktop_paths.normal_config,
                &desktop_paths.third_party_config,
                &desktop_paths.meta_path,
                &desktop_paths.profile_path,
            ] {
                crate::core::storage::write_atomic(file, b"{\"external\":\"preserved\"}").unwrap();
            }
            let desktop_before = claude_desktop::transaction::snapshot(&desktop_paths).unwrap();
            let mut desktop_state = claude_desktop::transaction::next_state(
                &[],
                &[],
                &desktop_paths,
                1,
                "crash-fixture",
                "",
                "",
                None,
                &[],
                claude_desktop::transaction::AppliedMode::Router,
            )
            .unwrap();
            desktop_state.router = Some(claude_desktop::gateway::RouterConfig {
                routes: Vec::new(),
                route_views: Vec::new(),
                file_projection: String::new(),
                route_revision: 1,
                gateway_token_version: 1,
                gateway_token: "stable-synthetic-gateway-token".into(),
                bound_port: store.router.port,
            });
            crate::core::secret_file::write(
                &paths.app_data_dir.join("claude-desktop/state.dat"),
                &serde_json::to_vec(&desktop_state).unwrap(),
            )
            .unwrap();

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
            assert!(!router_proxy::is_running());
            for file in &desktop_before {
                assert_eq!(
                    std::fs::read(&file.path).ok(),
                    file.text.as_ref().map(|text| text.as_bytes().to_vec()),
                    "port conflicts must not write Desktop configuration"
                );
            }
            drop(squatter);
            reconcile(&paths).await.unwrap();
            let gateway = claude_desktop::gateway::committed_snapshot(&paths.app_data_dir)
                .unwrap()
                .unwrap();
            assert_eq!(gateway.token, "stable-synthetic-gateway-token");
            assert_eq!(gateway.bound_port, store.router.port);
            assert_eq!(
                router_proxy::current_lease().unwrap().bound_port,
                store.router.port
            );
        });
    }

    #[test]
    fn native_codex_does_not_require_the_server_but_other_clients_do() {
        let mut store = router::Store::default();
        assert!(!needs_server(&store, false, false));
        store.router.enabled = true;
        assert!(needs_server(&store, false, false));
        store.codex_access_mode = router::CodexAccessMode::Native;
        assert!(!needs_server(&store, false, false));
        assert!(needs_server(&store, true, false));
        store.router.enabled = false;
        assert!(needs_server(&store, true, false));
        assert!(needs_server(&store, false, true));
    }

    #[test]
    fn enabled_codex_route_keeps_listening_after_external_config_takeover() {
        let _router = router_proxy::global_router_test_lock();
        let _stop = Stop;
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            let mut store = router::load_store(&paths).unwrap();
            store.providers.push(router::Provider {
                id: "fixture".into(),
                name: "Fixture".into(),
                enabled: true,
                api_key: "synthetic".into(),
                base_url: "https://fixture.example".into(),
                model: "gpt-fixture".into(),
                ..Default::default()
            });
            store.active_provider_id = Some("fixture".into());
            store.router.enabled = true;
            store.router.port = std::net::TcpListener::bind(("127.0.0.1", 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            save_store(&paths, &store);
            std::fs::write(&paths.config_path, "model_provider = 'external'\n").unwrap();
            reconcile(&paths).await.unwrap();
            assert_eq!(router_proxy::running_port(), Some(store.router.port));
            assert_eq!(
                std::fs::read_to_string(&paths.config_path).unwrap(),
                "model_provider = 'external'\n"
            );
        });
    }
}
