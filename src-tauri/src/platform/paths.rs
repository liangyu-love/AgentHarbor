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
    pub auto_switch_log_path: PathBuf,
    pub app_data_dir: PathBuf,
    pub skill_backups_dir: PathBuf,
    pub quota_history_path: PathBuf,
    pub quota_store_path: PathBuf,
    pub settings_path: PathBuf,
    pub bootstrap_cache_path: PathBuf,
    pub auto_switch_pending_path: PathBuf,
    pub auto_switch_snooze_path: PathBuf,
    pub voice_workspace_path: PathBuf,
    pub voice_runtime_path: PathBuf,
    pub launch_agent_path: PathBuf,
    pub global_agents_path: PathBuf,
    pub custom_instructions_dir: PathBuf,
    pub custom_instruction_history_dir: PathBuf,
}

impl CodexPaths {
    pub fn new() -> Self {
        let codex_home = Self::resolve_codex_home();
        Self::from_home(codex_home)
    }

    pub fn from_home(codex_home: PathBuf) -> Self {
        let app_data_dir = codex_home.join("codexharbor");
        let accounts_dir = app_data_dir.join("accounts");
        let custom_instructions_dir = app_data_dir.join("custom-instructions");
        let launch_agent_path = Self::resolve_launch_agent_path();

        // Copy the previous build's state once. Keeping the source intact lets
        // CodexHarbor and an installed AiMaMi run independently.
        if !app_data_dir.exists() {
            let _ = migrate_previous_app_data(&codex_home, &app_data_dir);
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
            auto_switch_log_path: accounts_dir.join("auto-switch.log"),
            skill_backups_dir: app_data_dir.join("skill-backups"),
            quota_history_path: app_data_dir.join("quota-history.jsonl"),
            quota_store_path: app_data_dir.join("quota-store.json"),
            settings_path: app_data_dir.join("settings.json"),
            bootstrap_cache_path: app_data_dir.join("bootstrap-cache.json"),
            auto_switch_pending_path: app_data_dir.join("auto-switch-pending.json"),
            auto_switch_snooze_path: app_data_dir.join("auto-switch-snooze.json"),
            voice_workspace_path: app_data_dir.join("voice-workspace.json"),
            voice_runtime_path: app_data_dir.join("voice-runtime.json"),
            global_agents_path: codex_home.join("AGENTS.md"),
            custom_instruction_history_dir: custom_instructions_dir.join("history"),
            accounts_dir,
            app_data_dir,
            custom_instructions_dir,
            launch_agent_path,
            codex_home,
        }
    }

    fn resolve_codex_home() -> PathBuf {
        // App-only override allows development and smoke tests without changing
        // the Codex environment or touching the user's normal configuration.
        for name in ["CODEX_HARBOR_CODEX_HOME", "CODEX_HOME"] {
            if let Some(val) = std::env::var_os(name).filter(|v| !v.is_empty()) {
                return PathBuf::from(val);
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
    }

    #[cfg(target_os = "macos")]
    fn resolve_launch_agent_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/LaunchAgents/dev.codexharbor.auto-switch.plist")
    }

    #[cfg(not(target_os = "macos"))]
    fn resolve_launch_agent_path() -> PathBuf {
        PathBuf::new()
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

fn migrate_previous_app_data(codex_home: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    let previous = codex_home.join("codexmate");
    let previous_accounts = codex_home.join("accounts");
    if !previous.exists() && !previous_accounts.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(target)?;
    for name in [
        "newapi-providers.dat",
        "newapi-connection.dat",
        "settings.json",
        "bootstrap-cache.json",
        "quota-history.jsonl",
        "quota-store.json",
        "auto-switch-pending.json",
        "auto-switch-snooze.json",
        "voice-workspace.json",
        "voice-runtime.json",
    ] {
        let source = previous.join(name);
        if source.is_file() {
            std::fs::copy(source, target.join(name))?;
        }
    }
    for name in ["skill-backups", "custom-instructions"] {
        copy_directory_if_present(&previous.join(name), &target.join(name))?;
    }
    let previous_catalog = previous.join("relay").join("codex_router_catalog.json");
    if previous_catalog.is_file() {
        std::fs::copy(previous_catalog, target.join("previous-router-catalog.json"))?;
    }
    copy_directory_if_present(&previous_accounts, &target.join("accounts"))
}

fn copy_directory_if_present(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
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
        let previous = codex_home.join("codexmate");
        let previous_accounts = codex_home.join("accounts");
        std::fs::create_dir_all(previous.join("relay")).unwrap();
        std::fs::create_dir_all(&previous_accounts).unwrap();
        std::fs::write(previous.join("settings.json"), "previous-settings").unwrap();
        std::fs::write(previous.join("relay/codex_router_catalog.json"), "{}").unwrap();
        std::fs::write(previous_accounts.join("registry.json"), "previous-registry").unwrap();

        let paths = CodexPaths::from_home(codex_home.clone());
        assert_eq!(paths.app_data_dir, codex_home.join("codexharbor"));
        assert_eq!(paths.accounts_dir, codex_home.join("codexharbor/accounts"));
        assert_eq!(std::fs::read_to_string(&paths.settings_path).unwrap(), "previous-settings");
        assert_eq!(
            std::fs::read_to_string(&paths.registry_path).unwrap(),
            "previous-registry"
        );
        assert!(paths.app_data_dir.join("previous-router-catalog.json").exists());
        assert!(previous.join("settings.json").exists());

        std::fs::write(previous.join("settings.json"), "changed-later").unwrap();
        let paths = CodexPaths::from_home(codex_home);
        assert_eq!(std::fs::read_to_string(paths.settings_path).unwrap(), "previous-settings");
    }
}
