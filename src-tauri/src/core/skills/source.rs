use super::*;
use crate::core::client_config::ClientPaths;
use crate::platform::paths::CodexPaths;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    #[default]
    Codex,
    ClaudeCode,
}

/// The source owns its scan root, disabled copies, backups, and writer lock.
/// Codex keeps its original layout, so upgrading never moves existing backups.
pub struct SkillPaths {
    pub skills_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub backups_dir: PathBuf,
}

impl SkillPaths {
    pub fn for_source(
        codex: &CodexPaths,
        clients: &ClientPaths,
        source: SkillSource,
    ) -> Result<Self, CoreError> {
        if source == SkillSource::Codex {
            return Ok(Self {
                skills_dir: codex.skills_dir.clone(),
                app_data_dir: codex.app_data_dir.clone(),
                backups_dir: codex.skill_backups_dir.clone(),
            });
        }
        // ClientPaths already applies the isolated test home before
        // CLAUDE_CONFIG_DIR, keeping client config and skills in the same home.
        let claude_dir = clients.claude.parent().ok_or_else(|| {
            CoreError::InvalidData("Invalid Claude Code configuration directory".into())
        })?;
        let skills_dir = absolute_path(&claude_dir.join("skills"))?;
        let app_data_dir = absolute_path(&codex.app_data_dir.join("skill-clients/claude-code"))?;
        let claude_root = activation::resolved_location(&skills_dir)?;
        for other in [
            &codex.skills_dir,
            &app_data_dir,
            &codex.app_data_dir.join("disabled-skills"),
            &codex.skill_backups_dir,
        ] {
            let other = activation::resolved_location(&absolute_path(other)?)?;
            if claude_root.starts_with(&other) || other.starts_with(&claude_root) {
                return Err(CoreError::InvalidData(
                    "Claude Code Skills storage overlaps another managed skills directory".into(),
                ));
            }
        }
        Ok(Self {
            skills_dir,
            backups_dir: app_data_dir.join("skill-backups"),
            app_data_dir,
        })
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, CoreError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, CodexPaths, ClientPaths) {
        let temp = tempfile::tempdir().unwrap();
        let codex = CodexPaths::from_home(temp.path().join("codex"));
        let clients = ClientPaths::from_home(&temp.path().join("clients"));
        (temp, codex, clients)
    }

    #[test]
    fn source_defaults_and_codex_layout_remain_compatible() {
        assert_eq!(SkillSource::default(), SkillSource::Codex);
        assert_eq!(
            serde_json::from_str::<SkillSource>("\"claudeCode\"").unwrap(),
            SkillSource::ClaudeCode
        );
        assert!(serde_json::from_str::<SkillSource>("\"desktop\"").is_err());
        let (_temp, codex, clients) = fixture();
        let old = SkillPaths::for_source(&codex, &clients, SkillSource::Codex).unwrap();
        assert_eq!(old.skills_dir, codex.skills_dir);
        assert_eq!(old.app_data_dir, codex.app_data_dir);
        assert_eq!(old.backups_dir, codex.skill_backups_dir);
    }

    #[test]
    fn same_skill_ids_keep_activation_and_backups_isolated() {
        let (temp, codex, clients) = fixture();
        let old = SkillPaths::for_source(&codex, &clients, SkillSource::Codex).unwrap();
        let claude = SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).unwrap();
        assert_eq!(
            claude.skills_dir,
            temp.path().join("clients/.claude/skills")
        );
        assert_eq!(
            claude.app_data_dir,
            codex.app_data_dir.join("skill-clients/claude-code")
        );
        for (root, text) in [
            (&old.skills_dir, "# Codex copy"),
            (&claude.skills_dir, "# Claude copy"),
        ] {
            std::fs::create_dir_all(root.join("same")).unwrap();
            std::fs::write(root.join("same/SKILL.md"), text).unwrap();
        }
        set_skill_enabled(&claude.skills_dir, &claude.app_data_dir, "same", false).unwrap();
        assert!(old.skills_dir.join("same/SKILL.md").is_file());
        assert!(!claude.skills_dir.join("same").exists());
        let backup = remove_skill(&claude.skills_dir, &claude.app_data_dir, "same")
            .unwrap()
            .backup;
        assert!(scan_skill_backups(&old.backups_dir)
            .unwrap()
            .items
            .is_empty());
        assert!(restore_skill_backup(&old.skills_dir, &old.app_data_dir, &backup.id).is_err());
        restore_skill_backup(&claude.skills_dir, &claude.app_data_dir, &backup.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(claude.skills_dir.join("same/SKILL.md")).unwrap(),
            "# Claude copy"
        );
        assert_eq!(
            std::fs::read_to_string(old.skills_dir.join("same/SKILL.md")).unwrap(),
            "# Codex copy"
        );
    }

    #[test]
    fn overlapping_client_roots_are_rejected_without_writes() {
        let (temp, codex, mut clients) = fixture();
        clients.claude = codex.codex_home.join("settings.json");
        assert!(SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).is_err());
        assert!(!codex.app_data_dir.exists());
        clients.claude = codex.app_data_dir.join("disabled-skills/settings.json");
        assert!(SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).is_err());
        clients.claude = temp.path().join("custom-claude/settings.json");
        assert_eq!(
            SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode)
                .unwrap()
                .skills_dir,
            temp.path().join("custom-claude/skills")
        );
    }

    #[test]
    fn import_replace_restore_and_backup_deletion_remain_source_scoped() {
        let (temp, codex, clients) = fixture();
        let old = SkillPaths::for_source(&codex, &clients, SkillSource::Codex).unwrap();
        let claude = SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).unwrap();
        let incoming = temp.path().join("incoming/same");
        std::fs::create_dir_all(&incoming).unwrap();
        std::fs::write(incoming.join("SKILL.md"), "# First").unwrap();
        for paths in [&old, &claude] {
            import_skill(
                &paths.skills_dir,
                &paths.app_data_dir,
                incoming.to_str().unwrap(),
            )
            .unwrap();
        }
        set_skill_enabled(&claude.skills_dir, &claude.app_data_dir, "same", false).unwrap();
        std::fs::write(incoming.join("SKILL.md"), "# Replacement").unwrap();
        let replacement = import_skill(
            &claude.skills_dir,
            &claude.app_data_dir,
            incoming.to_str().unwrap(),
        )
        .unwrap();
        assert!(!replacement.skill.enabled);
        let backup = replacement.backup.unwrap();
        assert!(backup
            .backup_path
            .starts_with(claude.backups_dir.to_str().unwrap()));
        let restored =
            restore_skill_backup(&claude.skills_dir, &claude.app_data_dir, &backup.id).unwrap();
        assert!(!restored.restored_skill.enabled);
        assert_eq!(
            std::fs::read_to_string(&restored.restored_skill.skill_file_path).unwrap(),
            "# First"
        );
        assert_eq!(
            std::fs::read_to_string(old.skills_dir.join("same/SKILL.md")).unwrap(),
            "# First"
        );
        assert!(load_skill_catalog(&old.skills_dir, &old.app_data_dir).unwrap()[0].enabled);
        assert!(scan_skill_backups(&old.backups_dir)
            .unwrap()
            .items
            .is_empty());
        delete_skill_backup(&claude.app_data_dir, &backup.id).unwrap();
        assert!(scan_skill_backups(&claude.backups_dir)
            .unwrap()
            .items
            .iter()
            .all(|item| item.id != backup.id));
        assert!(old.skills_dir.join("same/SKILL.md").exists());
    }

    #[cfg(any(unix, windows))]
    fn link_directory(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:SKILL_TEST_LINK -Target $env:SKILL_TEST_TARGET -ErrorAction Stop | Out-Null"])
                .env("SKILL_TEST_LINK", link)
                .env("SKILL_TEST_TARGET", target)
                .status().unwrap();
            assert!(status.success());
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn claude_linked_projection_is_read_only_and_never_recursively_scanned() {
        let (temp, codex, clients) = fixture();
        let claude = SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).unwrap();
        let external = temp.path().join("external/projected");
        std::fs::create_dir_all(external.join("nested")).unwrap();
        std::fs::write(external.join("SKILL.md"), "# Projected").unwrap();
        std::fs::write(external.join("nested/SKILL.md"), "# Never scan").unwrap();
        import_skill(
            &claude.skills_dir,
            &claude.app_data_dir,
            external.to_str().unwrap(),
        )
        .unwrap();
        let backup = remove_skill(&claude.skills_dir, &claude.app_data_dir, "projected")
            .unwrap()
            .backup;
        link_directory(&claude.skills_dir.join("projected"), &external);
        let report =
            load_skill_catalog_report_with_links(&claude.skills_dir, &claude.app_data_dir).unwrap();
        assert!(report.warnings.is_empty());
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].id, "projected");
        assert!(report.items[0].read_only);
        assert!(report.items[0].enabled);
        // The unchanged Codex scan deliberately omits projections.
        assert!(
            load_skill_catalog_report(&claude.skills_dir, &claude.app_data_dir)
                .unwrap()
                .items
                .is_empty()
        );
        for enabled in [true, false] {
            assert!(set_skill_enabled(
                &claude.skills_dir,
                &claude.app_data_dir,
                "projected",
                enabled
            )
            .is_err());
        }
        assert!(remove_skill(&claude.skills_dir, &claude.app_data_dir, "projected").is_err());
        assert!(import_skill(
            &claude.skills_dir,
            &claude.app_data_dir,
            external.to_str().unwrap()
        )
        .is_err());
        assert!(
            restore_skill_backup(&claude.skills_dir, &claude.app_data_dir, &backup.id).is_err()
        );
        let disabled = claude.app_data_dir.join("disabled-skills/projected");
        std::fs::create_dir_all(&disabled).unwrap();
        std::fs::write(disabled.join("SKILL.md"), "# Disabled copy").unwrap();
        let conflict =
            load_skill_catalog_report_with_links(&claude.skills_dir, &claude.app_data_dir).unwrap();
        assert!(conflict.items[0].conflict && conflict.items[0].read_only);
        for keep_enabled in [true, false] {
            assert!(resolve_skill_conflict(
                &claude.skills_dir,
                &claude.app_data_dir,
                "projected",
                keep_enabled
            )
            .is_err());
        }
        assert_eq!(
            std::fs::read_to_string(external.join("SKILL.md")).unwrap(),
            "# Projected"
        );
        assert!(external.join("nested/SKILL.md").exists());
        assert!(disabled.join("SKILL.md").exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn unavailable_link_only_warns_for_that_entry() {
        let (temp, codex, clients) = fixture();
        let claude = SkillPaths::for_source(&codex, &clients, SkillSource::ClaudeCode).unwrap();
        let external = temp.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::create_dir_all(claude.skills_dir.join("valid")).unwrap();
        std::fs::write(claude.skills_dir.join("valid/SKILL.md"), "# Valid").unwrap();
        link_directory(&claude.skills_dir.join("broken"), &external);
        let report =
            load_skill_catalog_report_with_links(&claude.skills_dir, &claude.app_data_dir).unwrap();
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].id, "valid");
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].code,
            "skills.warnings.invalidLinkedSkill"
        );
        assert!(report.warnings[0].message.ends_with("broken"));
    }
}
