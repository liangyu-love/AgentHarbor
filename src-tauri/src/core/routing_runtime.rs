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
    if router::load_store(paths)?.router.enabled
        || client_config::has_active_configs(&paths.app_data_dir)?
    {
        ensure_available(paths).await
    } else {
        router_proxy::stop();
        Ok(())
    }
}
