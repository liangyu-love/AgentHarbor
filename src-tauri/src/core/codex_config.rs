use super::{models::CoreError, storage};
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join(".codex"));
        (dir, paths)
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
