//! One committed protected State is the authority for routes and the local credential.
use super::{
    provider,
    routes::{self, ResolvedRoute},
    transaction::AppliedMode,
    types::{
        error_code, DesktopError, DesktopResult, DesktopRouteChoice, DesktopRouteView,
        DesktopRouterView,
    },
    Manager,
};
use crate::core::{models_dev, router::Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterConfig {
    pub routes: Vec<DesktopRouteChoice>,
    pub route_views: Vec<DesktopRouteView>,
    pub file_projection: String,
    pub route_revision: u64,
    pub gateway_token_version: u64,
    pub gateway_token: String,
    pub bound_port: u16,
}

impl std::fmt::Debug for RouterConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouterConfig")
            .field("routes", &self.routes)
            .field("route_views", &self.route_views)
            .field("file_projection", &self.file_projection)
            .field("route_revision", &self.route_revision)
            .field("gateway_token_version", &self.gateway_token_version)
            .field("bound_port", &self.bound_port)
            .finish_non_exhaustive()
    }
}

impl RouterConfig {
    pub fn view(&self) -> DesktopRouterView {
        DesktopRouterView {
            bound_port: self.bound_port,
            route_revision: self.route_revision,
            gateway_token_version: self.gateway_token_version,
            listener_ready: crate::core::router_proxy::current_lease()
                .is_some_and(|lease| lease.bound_port == self.bound_port),
        }
    }
}

#[derive(Clone)]
pub struct GatewaySnapshot {
    pub token: String,
    pub token_version: u64,
    pub bound_port: u16,
    pub route_revision: u64,
    pub paths_identity: String,
    pub routes: Vec<DesktopRouteChoice>,
}

impl std::fmt::Debug for GatewaySnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewaySnapshot")
            .field("token_version", &self.token_version)
            .field("bound_port", &self.bound_port)
            .field("route_revision", &self.route_revision)
            .field("paths_identity", &self.paths_identity)
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

/// The State file as last read: its size and modification time. A committed
/// State changes only by its file being replaced, which moves both.
type FileKey = Option<(u64, SystemTime)>;

fn file_key(path: &Path) -> FileKey {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// The last snapshot read with no journal pending, by data directory and
/// State file. Every Desktop request reads it: with the State unchanged it is
/// answered without the writer lock, which a Desktop restart holds for many
/// seconds, and without decrypting the State again.
static SNAPSHOT: Mutex<Option<(PathBuf, FileKey, Option<GatewaySnapshot>)>> = Mutex::new(None);

pub fn committed_snapshot(app_data_dir: &Path) -> DesktopResult<Option<GatewaySnapshot>> {
    let manager = Manager::new(app_data_dir);
    let state_path = manager.state_path();
    // A pending journal needs the full check below on every request.
    let settled = !manager.pending_path().exists();
    if settled {
        let key = file_key(&state_path);
        if let Some((_, _, snapshot)) = SNAPSHOT.lock().ok().and_then(|cached| cached.clone())
            .filter(|(dir, cached_key, _)| dir == app_data_dir && *cached_key == key)
        {
            return Ok(snapshot);
        }
    }
    let _guard = Manager::write_lock()?;
    // Read under the lock, which every writer holds while replacing the State.
    let key = file_key(&state_path);
    let snapshot = read_committed(&manager)?;
    if settled && !manager.pending_path().exists() {
        if let Ok(mut cached) = SNAPSHOT.lock() {
            *cached = Some((app_data_dir.to_path_buf(), key, snapshot.clone()));
        }
    }
    Ok(snapshot)
}

fn read_committed(manager: &Manager) -> DesktopResult<Option<GatewaySnapshot>> {
    let Some(state) = manager.load_state()? else {
        return Ok(None);
    };
    if let Some(pending) = manager.load_pending()? {
        let committed = state.transaction_id == pending.transaction_id
            && state.generation == pending.generation;
        if !committed
            || pending
                .files
                .iter()
                .any(|plan| !super::transaction::after_matches(plan).unwrap_or(false))
        {
            return Err(Box::new(DesktopError::new(
                error_code::RECOVERY_REQUIRED,
                "claudeDesktop.errors.pendingRecovery",
            )));
        }
    }
    if state.tombstone || state.data.applied_mode != AppliedMode::Router {
        return Ok(None);
    }
    let Some(router) = state.router else {
        return Err(super::transaction::storage_error());
    };
    if router.bound_port == 0 || router.gateway_token.is_empty() {
        return Err(super::transaction::storage_error());
    }
    Ok(Some(GatewaySnapshot {
        token: router.gateway_token,
        token_version: router.gateway_token_version,
        bound_port: router.bound_port,
        route_revision: router.route_revision,
        paths_identity: state.paths_identity,
        routes: router.routes,
    }))
}

pub fn resolve_message(
    snapshot: &GatewaySnapshot,
    store: &Store,
    catalog: &models_dev::Catalog,
    model: &str,
) -> DesktopResult<ResolvedRoute> {
    let input = model.trim();
    let (alias, one_m) = if input.to_ascii_lowercase().ends_with("[1m]") {
        (&input[..input.len() - 4], true)
    } else {
        (input, false)
    };
    let choice = snapshot
        .routes
        .iter()
        .find(|choice| choice.role_alias == alias)
        .or_else(|| role_for_alias(alias).and_then(|role| snapshot.routes.iter().find(|choice| choice.role == role)))
        .ok_or_else(|| {
            Box::new(DesktopError::new(
                error_code::MODEL_REJECTED,
                "claudeDesktop.errors.unknownRouteAlias",
            ))
        })?;
    // Already committed Fable was validated by the selected Desktop version at apply time.
    let mut result = routes::resolve_routes(
        store,
        std::slice::from_ref(choice),
        provider::DesktopVersionPolicy {
            fable_allowed: true,
        },
        catalog,
    )?
    .remove(0);
    if one_m && !result.view.one_m {
        return Err(Box::new(DesktopError::new(
            error_code::MODEL_REJECTED,
            "claudeDesktop.errors.route1mUnavailable",
        )));
    }
    // This flag describes this request, not a request to enable a menu variant.
    result.view.one_m = one_m;
    Ok(result)
}

/// Desktop's subagents use dated official names rather than profile aliases.
/// Match only validated Claude IDs and only an explicitly configured role.
fn role_for_alias(alias: &str) -> Option<&'static str> {
    if !provider::safe_model_id(alias, provider::DesktopVersionPolicy { fable_allowed: true }) { return None; }
    let rest = alias.strip_prefix("claude-")?;
    ["sonnet", "opus", "haiku", "fable"].into_iter()
        .find(|role| rest.strip_prefix(role).and_then(|tail| tail.strip_prefix('-')).is_some_and(|tail| {
            let parts: Vec<_> = tail.split('-').collect();
            // Official version aliases: major[-minor][-YYYYMMDD]. Arbitrary
            // provider prefixes/suffixes are not authorized by the role match.
            matches!(parts.len(), 1..=3) && parts.iter().enumerate().all(|(i, part)| {
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
                    && if part.len() == 8 { i > 0 && i == parts.len() - 1 } else { part.len() <= 2 }
            })
        }))
}

pub fn models(
    snapshot: &GatewaySnapshot,
    store: &Store,
    catalog: &models_dev::Catalog,
) -> DesktopResult<serde_json::Value> {
    let valid: Vec<_> = snapshot
        .routes
        .iter()
        .filter_map(|choice| {
            routes::resolve_routes(
                store,
                std::slice::from_ref(choice),
                provider::DesktopVersionPolicy {
                    fable_allowed: true,
                },
                catalog,
            )
            .ok()
            .and_then(|mut r| r.pop().map(|resolved| (choice, resolved)))
        })
        .collect();
    if valid.is_empty() {
        return Err(Box::new(DesktopError::new(
            error_code::MODEL_REJECTED,
            "claudeDesktop.errors.routeUnavailable",
        )));
    }
    let data: Vec<_> = valid.iter().map(|(committed, r)| {
        // The running Desktop profile still advertises its committed alias.
        // Canonicalization in resolve_routes applies only when reapplying it.
        let mut row = serde_json::json!({"type":"model", "id":committed.role_alias, "created_at":"1970-01-01T00:00:00Z"});
        if r.view.one_m { row["supports1m"] = serde_json::json!(true); }
        row
    }).collect();
    Ok(
        serde_json::json!({ "first_id":data.first().and_then(|r| r["id"].as_str()), "last_id":data.last().and_then(|r| r["id"].as_str()), "data":data, "has_more":false }),
    )
}

/// Keep server demand even when a pending operation currently blocks requests.
pub fn required_port(app_data_dir: &Path) -> DesktopResult<Option<u16>> {
    let manager = Manager::new(app_data_dir);
    Ok(manager
        .load_state()?
        .filter(|s| !s.tombstone && s.data.applied_mode == AppliedMode::Router)
        .and_then(|s| s.router)
        .map(|r| r.bound_port))
}
