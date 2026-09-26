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
