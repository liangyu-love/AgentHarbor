use super::{
    models::{ApiProxyConfigPayload, ApiProxyMode, CoreError},
    storage,
};
use crate::platform::paths::CodexPaths;
use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};

static CONFIG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn config_guard() -> Result<MutexGuard<'static, ()>, CoreError> {
    CONFIG_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| CoreError::OperationFailed("Codex config lock is poisoned".into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWriteResult {
    pub path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub changed: bool,
}

/// The only in-process read/modify/write boundary for Codex `config.toml`.
///
/// The process-wide lock prevents AgentHarbor features from overwriting each
/// other. Re-reading the file immediately before the atomic replace also
/// detects edits made by Codex or another process while a mutation is being
/// prepared.
#[derive(Debug, Clone)]
pub struct CodexConfigRepository {
    config_path: PathBuf,
    backup_dir: PathBuf,
}

impl CodexConfigRepository {
    pub fn new(paths: &CodexPaths) -> Self {
        Self {
            config_path: paths.config_path.clone(),
            backup_dir: paths.app_data_dir.join("config-backups"),
        }
    }

    pub fn read_text(&self) -> Result<String, CoreError> {
        Ok(self.read_optional_text()?.unwrap_or_default())
    }

    pub fn read_optional_text(&self) -> Result<Option<String>, CoreError> {
        let _guard = config_guard()?;
        decode_config(read_bytes(&self.config_path)?)
    }

    pub fn update(
        &self,
        mutate: impl FnOnce(&str) -> Result<String, CoreError>,
    ) -> Result<ConfigWriteResult, CoreError> {
        let _guard = config_guard()?;
        let original_bytes = read_bytes(&self.config_path)?;
        let original_text = decode_config(original_bytes.clone())?.unwrap_or_default();
        let updated = normalize_config(mutate(&original_text)?);

        // Validate the complete document before replacing a user's config.
        let _: toml::Value = updated.parse()?;

        if original_bytes.as_deref() == Some(updated.as_bytes())
            || (original_bytes.is_none() && updated.is_empty())
        {
            return Ok(ConfigWriteResult {
                path: self.config_path.clone(),
                backup_path: None,
                changed: false,
            });
        }

        if read_bytes(&self.config_path)? != original_bytes {
            return Err(CoreError::OperationFailed(
                "Codex config changed while AgentHarbor was updating it".into(),
            ));
        }

        let backup_path = if let Some(original) = original_bytes {
            let path = self
                .backup_dir
                .join(format!("{}.toml", uuid::Uuid::new_v4()));
            storage::write_atomic(&path, &original)?;
            Some(path)
        } else {
            None
        };

        storage::write_atomic(&self.config_path, updated.as_bytes())?;
        Ok(ConfigWriteResult {
            path: self.config_path.clone(),
            backup_path,
            changed: true,
        })
    }
}

fn read_bytes(path: &std::path::Path) -> Result<Option<Vec<u8>>, CoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn decode_config(bytes: Option<Vec<u8>>) -> Result<Option<String>, CoreError> {
    bytes
        .map(|bytes| {
            String::from_utf8(bytes)
                .map_err(|_| CoreError::InvalidData("Codex config is not valid UTF-8".into()))
        })
        .transpose()
}

fn normalize_config(mut text: String) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

const NETWORK_FEATURES: [&str; 2] = ["respect_system_proxy", "system_proxy_fallback"];

#[derive(serde::Serialize, serde::Deserialize)]
struct NetworkFeatureSnapshot {
    profile: Option<String>,
    key: String,
    original: Option<bool>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct NetworkSnapshot {
    config_path: PathBuf,
    features: Vec<NetworkFeatureSnapshot>,
}

/// Called after Codex exits and before its desktop process starts. The desktop
/// does not forward CLI -c flags to app-server, so explicit network modes need
/// two local config overrides. System mode restores only our unchanged fields.
pub fn prepare_network_config(
    paths: &CodexPaths,
    proxy: &ApiProxyConfigPayload,
) -> Result<(), CoreError> {
    super::api_client::sanitize_proxy_config(proxy)?;
    let snapshot_path = paths.app_data_dir.join("codex-network-overrides.json");
    let mut snapshot = match read_bytes(&snapshot_path)? {
        Some(bytes) => serde_json::from_slice::<NetworkSnapshot>(&bytes)?,
        None => NetworkSnapshot {
            config_path: paths.config_path.clone(),
            features: Vec::new(),
        },
    };
    if snapshot.config_path != paths.config_path {
        return Err(CoreError::InvalidData(
            "Codex network snapshot belongs to another config".into(),
        ));
    }
    if proxy.mode == ApiProxyMode::System && snapshot.features.is_empty() {
        return Ok(());
    }
    CodexConfigRepository::new(paths).update(|original| {
        let mut document = original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
        if proxy.mode == ApiProxyMode::System {
            for saved in &snapshot.features {
                if !NETWORK_FEATURES.contains(&saved.key.as_str()) {
                    return Err(CoreError::InvalidData(
                        "Invalid Codex network snapshot".into(),
                    ));
                }
                let features = match &saved.profile {
                    None => document.get_mut("features"),
                    Some(name) => document
                        .get_mut("profiles")
                        .and_then(|p| p.get_mut(name))
                        .and_then(|p| p.get_mut("features")),
                };
                if let Some(features) = features.and_then(toml_edit::Item::as_table_like_mut) {
                    // Preserve a field the user changed while the override was active.
                    if features.get(&saved.key).and_then(toml_edit::Item::as_bool) == Some(false) {
                        if let Some(value) = saved.original {
                            set_network_flag(features, &saved.key, value);
                        } else {
                            features.remove(&saved.key);
                        }
                    }
                }
            }
        } else {
            let mut scopes = vec![None];
            if let Some(profiles) = document
                .get("profiles")
                .and_then(toml_edit::Item::as_table_like)
            {
                for (name, profile) in profiles.iter() {
                    if profile.get("features").is_some_and(|features| {
                        NETWORK_FEATURES
                            .iter()
                            .any(|key| features.get(key).is_some())
                    }) {
                        scopes.push(Some(name.to_owned()));
                    }
                }
            }
            for profile in scopes {
                let item = match &profile {
                    None => &mut document["features"],
                    Some(name) => &mut document["profiles"][name]["features"],
                };
                if item.is_none() {
                    *item = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let features = item.as_table_like_mut().ok_or_else(|| {
                    CoreError::InvalidData("Codex features must be a TOML table".into())
                })?;
                for key in NETWORK_FEATURES {
                    if profile.is_some() && features.get(key).is_none() {
                        continue;
                    }
                    let original = match features.get(key) {
                        Some(value) => Some(value.as_bool().ok_or_else(|| {
                            CoreError::InvalidData(format!("Codex {key} must be a boolean"))
                        })?),
                        None => None,
                    };
                    if !snapshot
                        .features
                        .iter()
                        .any(|saved| saved.profile == profile && saved.key == key)
                    {
                        snapshot.features.push(NetworkFeatureSnapshot {
                            profile: profile.clone(),
                            key: key.to_owned(),
                            original,
                        });
                    }
                    set_network_flag(features, key, false);
                }
            }
            // Save only these nonsecret flags before installing their overrides.
            // The repository separately backs up the complete original config.
            storage::write_atomic(&snapshot_path, &serde_json::to_vec(&snapshot)?)?;
        }
        Ok(document.to_string())
    })?;
    if proxy.mode == ApiProxyMode::System {
        match std::fs::remove_file(snapshot_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn set_network_flag(features: &mut dyn toml_edit::TableLike, key: &str, enabled: bool) {
    let decor = features
        .get(key)
        .and_then(toml_edit::Item::as_value)
        .map(|value| value.decor().clone());
    let mut value = toml_edit::Value::from(enabled);
    if let Some(decor) = decor {
        *value.decor_mut() = decor;
    }
    features.insert(key, toml_edit::Item::Value(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join(".codex"));
        (dir, paths)
    }

    #[test]
    fn network_overrides_restore_prior_features_and_preserve_user_edits() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, "model = 'fixture'\n[features]\nrespect_system_proxy = true # keep this comment\nuser_feature = true\n[profiles.office.features]\nsystem_proxy_fallback = true\n").unwrap();
        let manual = ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("http://127.0.0.1:10808".into()),
        };
        prepare_network_config(&paths, &manual).unwrap();
        let installed = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = installed.parse().unwrap();
        assert_eq!(
            parsed["features"]["respect_system_proxy"].as_bool(),
            Some(false)
        );
        assert_eq!(
            parsed["features"]["system_proxy_fallback"].as_bool(),
            Some(false)
        );
        assert_eq!(
            parsed["profiles"]["office"]["features"]["system_proxy_fallback"].as_bool(),
            Some(false)
        );
        prepare_network_config(&paths, &manual).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            installed
        );
        std::fs::write(
            &paths.config_path,
            installed.replace("user_feature = true", "user_feature = false"),
        )
        .unwrap();
        prepare_network_config(&paths, &ApiProxyConfigPayload::default()).unwrap();
        let restored = std::fs::read_to_string(&paths.config_path).unwrap();
        let parsed: toml::Value = restored.parse().unwrap();
        assert_eq!(
            parsed["features"]["respect_system_proxy"].as_bool(),
            Some(true)
        );
        assert!(parsed["features"].get("system_proxy_fallback").is_none());
        assert_eq!(
            parsed["profiles"]["office"]["features"]["system_proxy_fallback"].as_bool(),
            Some(true)
        );
        assert_eq!(parsed["features"]["user_feature"].as_bool(), Some(false));
        assert!(restored.contains("# keep this comment"));
        assert!(!paths
            .app_data_dir
            .join("codex-network-overrides.json")
            .exists());
    }

    #[test]
    fn update_backs_up_and_atomically_replaces_valid_toml() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, "theme = 'dark'\n").unwrap();
        let repository = CodexConfigRepository::new(&paths);

        let result = repository
            .update(|original| Ok(format!("{original}model = 'gpt-test'\n")))
            .unwrap();

        assert!(result.changed);
        assert_eq!(
            std::fs::read_to_string(result.backup_path.unwrap()).unwrap(),
            "theme = 'dark'\n"
        );
        let updated = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(updated.contains("theme = 'dark'"));
        assert!(updated.contains("model = 'gpt-test'"));
    }

    #[test]
    fn update_rejects_an_external_edit_instead_of_overwriting_it() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, "version = 1\n").unwrap();
        let repository = CodexConfigRepository::new(&paths);

        let error = repository
            .update(|_| {
                std::fs::write(&paths.config_path, "version = 2\n")?;
                Ok("version = 3\n".into())
            })
            .unwrap_err();

        assert!(matches!(error, CoreError::OperationFailed(_)));
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            "version = 2\n"
        );
    }
}
