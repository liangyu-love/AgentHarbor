use super::auth::{current_timestamp, load_auth_file, make_auth_snapshot};
use super::models::{ApiProxyConfigPayload, CoreError, RateLimitWindow};
use super::repository::{RegistryFile, RegistryItem, Repository};
use super::storage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const AUTH_BACKUP_LIMIT: usize = 20;

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
    pub active_auth_path: Option<PathBuf>,
}

// Never serialize or log authentication bytes. RPC refreshes are applied under
// the repository lock, after checking that their input has not been replaced.
#[derive(Clone)]
pub struct CredentialRefresh {
    pub source_path: PathBuf,
    pub original: Vec<u8>,
    pub refreshed: Vec<u8>,
}

#[derive(Clone)]
pub struct AccountUsageUpdate {
    pub account_key: String,
    pub primary_window: Option<RateLimitWindow>,
    pub secondary_window: Option<RateLimitWindow>,
    pub plan: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub credential_refresh: Option<CredentialRefresh>,
}

fn needs_registry_sync(repo: &Repository, registry: &RegistryFile) -> Result<bool, CoreError> {
    if !repo.paths().registry_path.exists() {
        return Ok(true);
    }
    if registry
        .items
        .iter()
        .any(|item| !Path::new(&item.snapshot_path).is_file())
    {
        return Ok(true);
    }
    let bytes = match std::fs::read(&repo.paths().auth_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(registry.active_account_key.is_some());
        }
        Err(error) => return Err(error.into()),
    };
    // A corrupt auth.json is treated like one that is not a saved account, so the
    // account list, switching and deletion keep working and can repair it.
    let Ok(auth) = serde_json::from_slice(&bytes) else {
        return Ok(registry.active_account_key.is_some());
    };
    let Ok(snapshot) = make_auth_snapshot(&auth, &repo.paths().auth_path) else {
        return Ok(registry.active_account_key.is_some());
    };
    let current = registry
        .items
        .iter()
        .find(|item| item.account_key == snapshot.account_key);
    Ok(
        registry.active_account_key.as_deref() != Some(snapshot.account_key.as_str())
            || current.map_or(true, |item| {
                std::fs::read(&item.snapshot_path).map_or(true, |stored| stored != bytes)
            }),
    )
}

fn synced_registry(repo: &Repository) -> Result<RegistryFile, CoreError> {
    let registry = repo.load_registry_or_empty()?;
    if needs_registry_sync(repo, &registry)? {
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
            active_auth_path: (registry.active_account_key.as_deref()
                == Some(item.account_key.as_str()))
            .then(|| repo.paths().auth_path.clone()),
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

    let replaces_active = load_auth_file(&repo.paths().auth_path)
        .and_then(|auth| make_auth_snapshot(&auth, &repo.paths().auth_path))
        .is_ok_and(|active| active.account_key == snapshot.account_key);
    if !repo.paths().auth_path.exists() || replaces_active {
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
        // Without a backup the current credentials could be lost, so abort the switch.
        std::fs::copy(&repo.paths().auth_path, backup)?;
        // Each backup holds live tokens; keep a short recovery window, not every switch.
        storage::prune_backups(&repo.paths().auth_backups_dir, AUTH_BACKUP_LIMIT, |path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("auth-") && name.ends_with(".json"))
        });
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
    mut updates: Vec<AccountUsageUpdate>,
) -> Result<CodexAccountListPayload, CoreError> {
    // A refreshed credential that cannot be stored only affects its own account;
    // the quota results fetched for every other account are still kept.
    for update in &mut updates {
        if let Some(refresh) = &update.credential_refresh {
            if let Err(error) = apply_credential_refresh(repo, &update.account_key, refresh) {
                update.status = "error".into();
                update.error = Some(error.to_string());
            }
        }
    }
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
    repo.save_registry_cache(&registry)?;
    Ok(payload(registry))
}

fn apply_credential_refresh(
    repo: &Repository,
    account_key: &str,
    refresh: &CredentialRefresh,
) -> Result<(), CoreError> {
    let registry = repo.load_registry_or_empty()?;
    let Some(item) = registry
        .items
        .iter()
        .find(|item| item.account_key == account_key)
    else {
        return Ok(()); // The account was removed while the request was running.
    };
    let snapshot_path = Path::new(&item.snapshot_path);
    if refresh.source_path != repo.paths().auth_path && refresh.source_path != snapshot_path {
        return Err(CoreError::InvalidData(
            "Unexpected credential refresh path".into(),
        ));
    }
    // The caller holds the repository lock, serializing application commands.
    // This comparison also rejects observed external changes, but it is not a
    // cross-process compare-and-swap with Codex writing the same auth file.
    if std::fs::read(&refresh.source_path).ok().as_deref() != Some(refresh.original.as_slice()) {
        return Ok(());
    }
    let auth = serde_json::from_slice(&refresh.refreshed)?;
    if make_auth_snapshot(&auth, &refresh.source_path)?.account_key != account_key {
        return Err(CoreError::InvalidData(
            "Refreshed account identity changed".into(),
        ));
    }
    if refresh.source_path == repo.paths().auth_path {
        storage::write_atomic(&refresh.source_path, &refresh.refreshed)?;
    } else {
        // If this account became active meanwhile, update the live credential
        // only when it still has exactly the input used for this refresh.
        let active = std::fs::read(&repo.paths().auth_path).ok();
        if active.as_deref() == Some(refresh.original.as_slice()) {
            storage::write_atomic(&repo.paths().auth_path, &refresh.refreshed)?;
        } else if active.as_deref().is_some_and(|bytes| {
            serde_json::from_slice(bytes)
                .ok()
                .and_then(|auth| make_auth_snapshot(&auth, &repo.paths().auth_path).ok())
                .is_some_and(|auth| auth.account_key == account_key)
        }) {
            return Ok(());
        }
    }
    // Never recreate a snapshot deleted or re-imported during the request.
    if std::fs::read(snapshot_path).ok().as_deref() == Some(refresh.original.as_slice()) {
        storage::write_atomic(snapshot_path, &refresh.refreshed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::paths::CodexPaths;

    fn write_auth(path: &Path, account_id: &str) {
        write_auth_token(path, account_id, "access");
    }

    fn write_auth_token(path: &Path, account_id: &str, access_token: &str) {
        let auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "refresh_token": "refresh",
                "account_id": account_id
            }
        });
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&auth).unwrap()).unwrap();
    }

    fn usage_update(account_key: &str, refresh: Option<CredentialRefresh>) -> AccountUsageUpdate {
        AccountUsageUpdate {
            account_key: account_key.into(),
            primary_window: None,
            secondary_window: None,
            plan: Some("plus".into()),
            status: "healthy".into(),
            error: None,
            credential_refresh: refresh,
        }
    }

    #[test]
    fn one_rejected_credential_refresh_keeps_other_accounts_updates() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let first = temp.path().join("first.json");
        let second = temp.path().join("second.json");
        write_auth(&first, "first");
        write_auth(&second, "second");
        register_login(&repo, &first).unwrap();
        register_login(&repo, &second).unwrap();
        let (credentials, _) = credentials(&repo).unwrap();
        let bad = &credentials[0];
        let original = std::fs::read(&bad.snapshot_path).unwrap();
        // The "refreshed" credential belongs to another account: rejected.
        let other = temp.path().join("other.json");
        write_auth(&other, "someone-else");
        let refresh = CredentialRefresh {
            source_path: bad.snapshot_path.clone(),
            original,
            refreshed: std::fs::read(&other).unwrap(),
        };
        let listed = apply_usage_updates(
            &repo,
            vec![
                usage_update(&bad.account_key, Some(refresh)),
                usage_update(&credentials[1].account_key, None),
            ],
        )
        .unwrap();
        let status = |key: &str| {
            listed.items.iter().find(|item| item.account_key == key).unwrap().usage_status.clone()
        };
        assert_eq!(status(&bad.account_key), "error");
        assert_eq!(status(&credentials[1].account_key), "healthy");
    }

    #[test]
    fn corrupt_auth_json_keeps_the_account_list_usable() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth_token(&source, "account", "token");
        register_login(&repo, &source).unwrap();
        std::fs::write(&repo.paths().auth_path, b"{ not json").unwrap();
        let listed = list(&repo).unwrap();
        assert_eq!(listed.items.len(), 1);
    }

    #[test]
    fn clean_removes_only_owned_backup_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        repo.paths().ensure_directories().unwrap();
        let dir = &repo.paths().auth_backups_dir;
        std::fs::write(dir.join("auth-1-x.json"), b"{}").unwrap();
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        repo.clean().unwrap();
        assert!(!dir.join("auth-1-x.json").exists());
        assert!(dir.join("notes.txt").exists());
        assert!(dir.join("nested").is_dir());
    }

    #[test]
    fn relogin_active_account_keeps_the_new_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth_token(&source, "account", "old-token");
        let key = register_login(&repo, &source)
            .unwrap()
            .active_account_key
            .unwrap();
        write_auth_token(&source, "account", "new-token");
        let expected = std::fs::read(&source).unwrap();
        register_login(&repo, &source).unwrap();
        let (credentials, _) = credentials(&repo).unwrap();
        assert_eq!(std::fs::read(&repo.paths().auth_path).unwrap(), expected);
        assert_eq!(
            std::fs::read(&credentials[0].snapshot_path).unwrap(),
            expected
        );
        assert_eq!(credentials[0].account_key, key);
    }

    #[test]
    fn rotated_active_credentials_sync_and_logout_clears_the_active_account() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth(&source, "account");
        let key = register_login(&repo, &source)
            .unwrap()
            .active_account_key
            .unwrap();
        apply_usage_updates(&repo, vec![usage_update(&key, None)]).unwrap();
        write_auth_token(&repo.paths().auth_path, "account", "rotated-token");
        let (credentials, _) = credentials(&repo).unwrap();
        assert_eq!(
            credentials[0].active_auth_path.as_ref(),
            Some(&repo.paths().auth_path)
        );
        assert_eq!(
            std::fs::read(&credentials[0].snapshot_path).unwrap(),
            std::fs::read(&repo.paths().auth_path).unwrap()
        );
        assert_eq!(list(&repo).unwrap().items[0].usage_status, "healthy");
        std::fs::remove_file(&repo.paths().auth_path).unwrap();
        let logged_out = list(&repo).unwrap();
        assert!(logged_out.active_account_key.is_none());
        assert!(!logged_out.items[0].is_active);
        assert!(delete(&repo, &key).unwrap().items.is_empty());
    }

    #[test]
    fn rpc_refresh_updates_active_auth_and_snapshot_together() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth(&source, "account");
        let key = register_login(&repo, &source)
            .unwrap()
            .active_account_key
            .unwrap();
        let original = std::fs::read(&repo.paths().auth_path).unwrap();
        write_auth_token(&source, "account", "rpc-rotated");
        let refreshed = std::fs::read(&source).unwrap();
        let refresh = CredentialRefresh {
            source_path: repo.paths().auth_path.clone(),
            original,
            refreshed: refreshed.clone(),
        };
        apply_usage_updates(&repo, vec![usage_update(&key, Some(refresh))]).unwrap();
        let (credentials, _) = credentials(&repo).unwrap();
        assert_eq!(std::fs::read(&repo.paths().auth_path).unwrap(), refreshed);
        assert_eq!(
            std::fs::read(&credentials[0].snapshot_path).unwrap(),
            refreshed
        );
    }

    #[test]
    fn late_rpc_refresh_cannot_overwrite_relogin_logout_or_account_switch() {
        for action in ["relogin", "logout", "switch", "delete"] {
            let temp = tempfile::tempdir().unwrap();
            let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
            let source = temp.path().join("source.json");
            write_auth(&source, "account");
            let key = register_login(&repo, &source)
                .unwrap()
                .active_account_key
                .unwrap();
            let original = std::fs::read(&repo.paths().auth_path).unwrap();
            write_auth_token(&source, "account", "stale-rpc-result");
            let refresh = CredentialRefresh {
                source_path: repo.paths().auth_path.clone(),
                original,
                refreshed: std::fs::read(&source).unwrap(),
            };
            match action {
                "relogin" => {
                    write_auth_token(&source, "account", "new-login-wins");
                    register_login(&repo, &source).unwrap();
                }
                "switch" => {
                    write_auth(&source, "other-account");
                    let accounts = register_login(&repo, &source).unwrap();
                    let other = accounts
                        .items
                        .iter()
                        .find(|item| item.account_key != key)
                        .unwrap();
                    switch(&repo, &other.account_key).unwrap();
                }
                _ => {
                    std::fs::remove_file(&repo.paths().auth_path).unwrap();
                    if action == "delete" {
                        delete(&repo, &key).unwrap();
                    }
                }
            }
            let expected = std::fs::read(&repo.paths().auth_path).ok();
            apply_usage_updates(&repo, vec![usage_update(&key, Some(refresh))]).unwrap();
            assert_eq!(
                std::fs::read(&repo.paths().auth_path).ok(),
                expected,
                "{action}"
            );
            if action == "delete" {
                assert!(list(&repo).unwrap().items.is_empty());
            }
        }
    }

    #[test]
    fn inactive_rpc_refresh_follows_an_unchanged_account_that_became_active() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth(&source, "first");
        register_login(&repo, &source).unwrap();
        write_auth(&source, "second");
        register_login(&repo, &source).unwrap();
        let (credentials, _) = credentials(&repo).unwrap();
        let inactive = credentials
            .iter()
            .find(|item| item.active_auth_path.is_none())
            .unwrap();
        let original = std::fs::read(&inactive.snapshot_path).unwrap();
        write_auth_token(&source, "second", "new-second-token");
        let refreshed = std::fs::read(&source).unwrap();
        let refresh = CredentialRefresh {
            source_path: inactive.snapshot_path.clone(),
            original,
            refreshed: refreshed.clone(),
        };
        switch(&repo, &inactive.account_key).unwrap();
        apply_usage_updates(
            &repo,
            vec![usage_update(&inactive.account_key, Some(refresh))],
        )
        .unwrap();
        assert_eq!(std::fs::read(&repo.paths().auth_path).unwrap(), refreshed);
    }

    #[test]
    fn quota_refreshes_do_not_create_registry_backups() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::from_paths(CodexPaths::from_home(temp.path().join("codex")));
        let source = temp.path().join("source.json");
        write_auth(&source, "account");
        let key = register_login(&repo, &source)
            .unwrap()
            .active_account_key
            .unwrap();
        let before = std::fs::read_dir(&repo.paths().registry_backups_dir)
            .unwrap()
            .count();
        for _ in 0..3 {
            apply_usage_updates(&repo, vec![usage_update(&key, None)]).unwrap();
        }
        assert_eq!(
            std::fs::read_dir(&repo.paths().registry_backups_dir)
                .unwrap()
                .count(),
            before
        );
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
