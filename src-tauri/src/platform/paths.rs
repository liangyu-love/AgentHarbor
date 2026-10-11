use std::path::PathBuf;

#[derive(Clone)]
pub struct CodexPaths {
    pub codex_home: PathBuf,
    pub auth_path: PathBuf,
    pub config_path: PathBuf,
    pub session_index_path: PathBuf,
    /// Codex 客户端真正的 session 索引数据库（SQLite）。
    ///
    /// 历史上我们以为 `session_index.jsonl` 是 codex 列表来源，但实测
    /// codex 启动后会**反向覆盖**这个 jsonl 文件，且其线程列表来自
    /// `state_5.sqlite` 的 `threads` 表 —— 我们之前往 jsonl 写恢复记录
    /// 完全无效。所有"恢复 / 是否被 codex 看到" 的判定都必须以这个文件为准。
    pub codex_state_db_path: PathBuf,
    pub sessions_dir: PathBuf,
    pub archived_sessions_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub accounts_dir: PathBuf,
    pub registry_path: PathBuf,
    pub snapshots_dir: PathBuf,
    pub auth_backups_dir: PathBuf,
    pub registry_backups_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub skill_backups_dir: PathBuf,
    pub quota_history_path: PathBuf,
    pub quota_store_path: PathBuf,
    pub settings_path: PathBuf,
    pub global_agents_path: PathBuf,
    pub custom_instructions_dir: PathBuf,
    pub custom_instruction_history_dir: PathBuf,
}

impl CodexPaths {
    pub fn new() -> Self {
        let codex_home = Self::resolve_codex_home();
        let app_data_dir = Self::resolve_app_data_dir(&codex_home);
        Self::from_roots(codex_home, app_data_dir)
    }

    /// Construct an isolated Codex target for tests and smoke runs.
    /// Application data intentionally stays below the supplied home.
    pub fn from_home(codex_home: PathBuf) -> Self {
        let app_data_dir = codex_home.join("agentharbor");
        Self::from_roots(codex_home, app_data_dir)
    }

    pub fn from_roots(codex_home: PathBuf, app_data_dir: PathBuf) -> Self {
        let accounts_dir = app_data_dir.join("accounts");
        let custom_instructions_dir = app_data_dir.join("custom-instructions");

        // Copy the previous build's state once and leave the source intact.
        if !app_data_dir.exists() {
            if let Err(error) = migrate_previous_app_data(&codex_home, &app_data_dir) {
                eprintln!(
                    "[AgentHarbor] previous app data migration failed; source was preserved: {error}"
                );
            }
        }

        Self {
            auth_path: codex_home.join("auth.json"),
            config_path: codex_home.join("config.toml"),
            session_index_path: codex_home.join("session_index.jsonl"),
            codex_state_db_path: codex_home.join("state_5.sqlite"),
            sessions_dir: codex_home.join("sessions"),
            archived_sessions_dir: codex_home.join("archived_sessions"),
            skills_dir: codex_home.join("skills"),
            registry_path: accounts_dir.join("registry.json"),
            snapshots_dir: accounts_dir.join("snapshots"),
            auth_backups_dir: accounts_dir.join("backups"),
            registry_backups_dir: accounts_dir.join("registry-backups"),
            skill_backups_dir: app_data_dir.join("skill-backups"),
            quota_history_path: app_data_dir.join("quota-history.jsonl"),
            quota_store_path: app_data_dir.join("quota-store.json"),
            settings_path: app_data_dir.join("settings.json"),
            global_agents_path: codex_home.join("AGENTS.md"),
            custom_instruction_history_dir: custom_instructions_dir.join("history"),
            accounts_dir,
            app_data_dir,
            custom_instructions_dir,
            codex_home,
        }
    }

    fn resolve_app_data_dir(codex_home: &std::path::Path) -> PathBuf {
        if let Some(value) = std::env::var_os("AGENT_HARBOR_DATA_DIR").filter(|v| !v.is_empty()) {
            return PathBuf::from(value);
        }
        if std::env::var_os("AGENT_HARBOR_CODEX_HOME").is_some_and(|value| !value.is_empty()) {
            return codex_home.join("agentharbor");
        }
        dirs::data_local_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/share")
            })
            .join("AgentHarbor")
    }

    fn resolve_codex_home() -> PathBuf {
        // App-only override allows development and smoke tests without changing
        // the Codex environment or touching the user's normal configuration.
        for name in ["AGENT_HARBOR_CODEX_HOME", "CODEX_HOME"] {
            if let Some(val) = std::env::var_os(name).filter(|v| !v.is_empty()) {
                return PathBuf::from(val);
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
    }

    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.accounts_dir)?;
        std::fs::create_dir_all(&self.snapshots_dir)?;
        std::fs::create_dir_all(&self.auth_backups_dir)?;
        std::fs::create_dir_all(&self.registry_backups_dir)?;
        std::fs::create_dir_all(&self.app_data_dir)?;
        std::fs::create_dir_all(&self.skill_backups_dir)?;
        std::fs::create_dir_all(&self.custom_instructions_dir)?;
        std::fs::create_dir_all(&self.custom_instruction_history_dir)?;
        Ok(())
    }
}

fn migrate_previous_app_data(
    codex_home: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    if target.exists() {
        return Ok(());
    }

    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "application data directory has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let staged = parent.join(format!(".agentharbor-migration-{}", uuid::Uuid::new_v4()));

    // Current releases before the split stored application state below
    // `.codex/agentharbor`. Copy it first when moving to the OS app-data root.
    let current_legacy = codex_home.join("agentharbor");
    if current_legacy != target && current_legacy.is_dir() {
        return stage_complete_directory(&current_legacy, &staged, target);
    }

    // Prefer the immediately preceding CodexHarbor build so local changes made
    // during the rename are preserved. The source remains untouched.
    let previous_harbor = codex_home.join("codexharbor");
    if previous_harbor.is_dir() {
        return stage_complete_directory(&previous_harbor, &staged, target);
    }

    Ok(())
}

fn stage_complete_directory(
    source: &std::path::Path,
    staged: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    if let Err(error) = copy_directory_if_present(source, staged) {
        let _ = std::fs::remove_dir_all(staged);
        return Err(error);
    }
    publish_migrated_directory(staged, target)
}

fn publish_migrated_directory(
    staged: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    match std::fs::rename(staged, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => {
            std::fs::remove_dir_all(staged)?;
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(staged);
            Err(error)
        }
    }
}

fn copy_directory_if_present(
    source: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_if_present(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_is_isolated_and_previous_state_is_copied_once() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join(".codex");
        let previous = codex_home.join("codexharbor");
        std::fs::create_dir_all(previous.join("accounts")).unwrap();
        std::fs::write(previous.join("settings.json"), "previous-settings").unwrap();
        std::fs::write(previous.join("accounts/registry.json"), "previous-registry").unwrap();

        let paths = CodexPaths::from_home(codex_home.clone());
        assert_eq!(paths.app_data_dir, codex_home.join("agentharbor"));
        assert_eq!(paths.accounts_dir, codex_home.join("agentharbor/accounts"));
        assert_eq!(std::fs::read_to_string(&paths.settings_path).unwrap(), "previous-settings");
        assert_eq!(std::fs::read_to_string(&paths.registry_path).unwrap(), "previous-registry");
        assert!(previous.join("settings.json").exists(), "the source is left in place");

        std::fs::write(previous.join("settings.json"), "changed-later").unwrap();
        let paths = CodexPaths::from_home(codex_home);
        assert_eq!(std::fs::read_to_string(paths.settings_path).unwrap(), "previous-settings");
    }

    #[test]
    fn separate_app_root_migrates_current_legacy_state_once() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join(".codex");
        let legacy = codex_home.join("agentharbor");
        let app_data = temp.path().join("app-data");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("settings.json"), "legacy-settings").unwrap();

        let paths = CodexPaths::from_roots(codex_home.clone(), app_data.clone());
        assert_eq!(paths.codex_home, codex_home);
        assert_eq!(paths.app_data_dir, app_data);
        assert_eq!(
            std::fs::read_to_string(&paths.settings_path).unwrap(),
            "legacy-settings"
        );
        assert!(legacy.join("settings.json").exists());
    }
}
