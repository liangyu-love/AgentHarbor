use super::auth::{current_timestamp, load_auth_file, make_auth_snapshot};
use super::models::{ApiProxyConfigPayload, CoreError, RateLimitWindow};
use super::repository::{RegistryFile, RegistryItem, Repository};
use super::storage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountSummary {
    pub account_key: String,
    pub email: String,
    pub display_name: String,
    pub plan: String,
    pub auth_mode: String,
    pub is_active: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub last_usage_at: Option<i64>,
    pub primary_window: Option<RateLimitWindow>,
    pub secondary_window: Option<RateLimitWindow>,
    pub usage_status: String,
    pub usage_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountListPayload {
    pub active_account_key: Option<String>,
    pub refreshed_at: i64,
    pub items: Vec<CodexAccountSummary>,
}

#[derive(Debug, Clone)]
pub struct AccountCredential {
    pub account_key: String,
    pub snapshot_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AccountUsageUpdate {
    pub account_key: String,
    pub primary_window: Option<RateLimitWindow>,
    pub secondary_window: Option<RateLimitWindow>,
    pub plan: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

fn needs_registry_sync(repo: &Repository, registry: &RegistryFile) -> bool {
    if !repo.paths().registry_path.exists() {
        return true;
    }
    if registry
        .items
        .iter()
        .any(|item| !Path::new(&item.snapshot_path).is_file())
    {
        return true;
    }
    let Ok(auth) = load_auth_file(&repo.paths().auth_path) else {
        return false;
    };
    let Ok(snapshot) = make_auth_snapshot(&auth, &repo.paths().auth_path) else {
        return false;
    };
    registry.active_account_key.as_deref() != Some(snapshot.account_key.as_str())
        || !registry
            .items
            .iter()
            .any(|item| item.account_key == snapshot.account_key)
}

fn synced_registry(repo: &Repository) -> Result<RegistryFile, CoreError> {
    let registry = repo.load_registry_or_empty()?;
    if needs_registry_sync(repo, &registry) {
        repo.rebuild_registry_state()
    } else {
        Ok(registry)
    }
}

fn display_name(item: &RegistryItem) -> String {
    for value in [
        item.alias.as_str(),
        item.profile_name.as_deref().unwrap_or_default(),
        item.account_name.as_deref().unwrap_or_default(),
        item.workspace_name.as_deref().unwrap_or_default(),
        item.email.as_str(),
    ] {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    "Codex account".to_string()
}

fn payload(registry: RegistryFile) -> CodexAccountListPayload {
    let active = registry.active_account_key.clone();
    let mut items = registry
        .items
        .into_iter()
        .map(|item| CodexAccountSummary {
            is_active: active.as_deref() == Some(item.account_key.as_str()),
            display_name: display_name(&item),
            account_key: item.account_key,
            email: item.email,
            plan: item.plan,
            auth_mode: item.auth_mode,
            created_at: item.created_at,
            last_used_at: item.last_used_at,
            last_usage_at: item.last_usage_at,
            primary_window: item.cached_primary_window,
            secondary_window: item.cached_secondary_window,
            usage_status: item.usage_status,
            usage_error: item.last_usage_error,
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .is_active
            .cmp(&left.is_active)
            .then_with(|| left.email.to_lowercase().cmp(&right.email.to_lowercase()))
    });
    CodexAccountListPayload {
        active_account_key: active,
        refreshed_at: current_timestamp(),
        items,
    }
}

pub fn list(repo: &Repository) -> Result<CodexAccountListPayload, CoreError> {
    Ok(payload(synced_registry(repo)?))
}

pub fn credentials(
    repo: &Repository,
) -> Result<(Vec<AccountCredential>, ApiProxyConfigPayload), CoreError> {
    let registry = synced_registry(repo)?;
    let items = registry
        .items
        .iter()
        .filter(|item| Path::new(&item.snapshot_path).is_file())
        .map(|item| AccountCredential {
            account_key: item.account_key.clone(),
            snapshot_path: PathBuf::from(&item.snapshot_path),
        })
        .collect();
    let proxy = super::newapi::proxy_from_settings(&repo.paths().settings_path);
    Ok((items, proxy))
}

pub fn register_login(
    repo: &Repository,
    source_auth_path: &Path,
) -> Result<CodexAccountListPayload, CoreError> {
    let auth = load_auth_file(source_auth_path)?;
    let snapshot = make_auth_snapshot(&auth, source_auth_path)?;
    repo.paths().ensure_directories()?;

    let bytes = std::fs::read(source_auth_path)?;
    let snapshot_path = repo
        .paths()
        .snapshots_dir
        .join(format!("{}.json", snapshot.account_key));
    storage::write_atomic(&snapshot_path, &bytes)?;

    if !repo.paths().auth_path.exists() {
        storage::write_atomic(&repo.paths().auth_path, &bytes)?;
    }

    Ok(payload(repo.rebuild_registry_state()?))
}

pub fn switch(repo: &Repository, account_key: &str) -> Result<CodexAccountListPayload, CoreError> {
    let registry = synced_registry(repo)?;
    let target = registry
        .items
        .iter()
        .find(|item| item.account_key == account_key)
        .ok_or_else(|| CoreError::InvalidData("Account not found".into()))?;
    let target_bytes = std::fs::read(&target.snapshot_path)?;
    let target_auth: super::auth::AuthFile = serde_json::from_slice(&target_bytes)?;
    let verified = make_auth_snapshot(&target_auth, Path::new(&target.snapshot_path))?;
    if verified.account_key != account_key {
        return Err(CoreError::InvalidData(
            "Account snapshot identity does not match registry".into(),
        ));
    }

    repo.paths().ensure_directories()?;
    if repo.paths().auth_path.exists() {
        if let Ok(current_auth) = load_auth_file(&repo.paths().auth_path) {
            if let Ok(current) = make_auth_snapshot(&current_auth, &repo.paths().auth_path) {
                let current_snapshot = repo
                    .paths()
                    .snapshots_dir
                    .join(format!("{}.json", current.account_key));
                storage::write_atomic(&current_snapshot, &std::fs::read(&repo.paths().auth_path)?)?;
            }
        }
        let backup = repo.paths().auth_backups_dir.join(format!(
            "auth-{}-{}.json",
            current_timestamp(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::copy(&repo.paths().auth_path, backup);
    }
    storage::write_atomic(&repo.paths().auth_path, &target_bytes)?;
    Ok(payload(repo.rebuild_registry_state()?))
}

pub fn delete(repo: &Repository, account_key: &str) -> Result<CodexAccountListPayload, CoreError> {
    let mut registry = synced_registry(repo)?;
    if registry.active_account_key.as_deref() == Some(account_key) {
        return Err(CoreError::InvalidData(
            "The active Codex account cannot be deleted".into(),
        ));
    }
    let item = registry
        .items
        .iter()
        .find(|item| item.account_key == account_key)
        .cloned()
        .ok_or_else(|| CoreError::InvalidData("Account not found".into()))?;
    let snapshot_path = PathBuf::from(&item.snapshot_path);
    if snapshot_path.parent() == Some(repo.paths().snapshots_dir.as_path()) {
        match std::fs::remove_file(&snapshot_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    registry
        .items
        .retain(|item| item.account_key != account_key);
    registry.updated_at = current_timestamp();
    repo.save_registry(&registry)?;
    Ok(payload(registry))
}

pub fn apply_usage_updates(
    repo: &Repository,
    updates: Vec<AccountUsageUpdate>,
) -> Result<CodexAccountListPayload, CoreError> {
    let mut registry = synced_registry(repo)?;
    let checked_at = current_timestamp();
    for update in updates {
        let Some(item) = registry
            .items
            .iter_mut()
            .find(|item| item.account_key == update.account_key)
        else {
            continue;
        };
        item.cached_primary_window = update.primary_window;
        item.cached_secondary_window = update.secondary_window;
        item.last_usage_at = Some(checked_at);
        item.usage_status = update.status;
        item.last_usage_error = update.error;
        if let Some(plan) = update.plan.filter(|plan| !plan.trim().is_empty()) {
            item.plan = plan;
        }
    }
    registry.updated_at = checked_at;
    repo.save_registry(&registry)?;
    Ok(payload(registry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::paths::CodexPaths;

    fn write_auth(path: &Path, account_id: &str) {
        let auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access",
                "refresh_token": "refresh",
                "account_id": account_id
            }
        });
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&auth).unwrap()).unwrap();
    }

    #[test]
    fn register_switch_and_delete_preserve_the_active_account() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let first = temp.path().join("first.json");
        let second = temp.path().join("second.json");
        write_auth(&first, "first");
        write_auth(&second, "second");

        let initial = register_login(&repo, &first).unwrap();
        let first_key = initial.active_account_key.unwrap();
        let with_second = register_login(&repo, &second).unwrap();
        let second_key = with_second
            .items
            .iter()
            .find(|item| item.account_key != first_key)
            .unwrap()
            .account_key
            .clone();
        let switched = switch(&repo, &second_key).unwrap();
        assert_eq!(
            switched.active_account_key.as_deref(),
            Some(second_key.as_str())
        );
        assert!(delete(&repo, &second_key).is_err());
        let remaining = delete(&repo, &first_key).unwrap();
        assert_eq!(remaining.items.len(), 1);
    }
}
