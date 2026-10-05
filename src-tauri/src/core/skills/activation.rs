//! Codex activation is derived from directory placement, never a UI-only flag.
use super::*;
use std::fs::{File, OpenOptions};

fn disabled_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("disabled-skills")
}

pub(super) fn write_guard(app_data_dir: &Path) -> Result<File, CoreError> {
    std::fs::create_dir_all(app_data_dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app_data_dir.join("skills.lock"))?;
    file.lock()?;
    Ok(file) // Closing the handle releases the cross-process lock.
}

pub(super) fn resolved_location(path: &Path) -> Result<PathBuf, CoreError> {
    if path.exists() {
        return Ok(std::fs::canonicalize(path)?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::InvalidData("Invalid skills root".into()))?;
    Ok(resolved_location(parent)?.join(path.file_name().unwrap_or_default()))
}

pub(super) fn prepare_roots(skills_dir: &Path, app_data_dir: &Path) -> Result<(), CoreError> {
    let client = resolved_location(skills_dir)?;
    let disabled = resolved_location(&disabled_root(app_data_dir))?;
    if client.starts_with(&disabled) || disabled.starts_with(&client) {
        return Err(CoreError::InvalidData(
            "Skills storage overlaps the client scan directory".into(),
        ));
    }
    std::fs::create_dir_all(skills_dir)?;
    std::fs::create_dir_all(disabled_root(app_data_dir))?;
    cleanup_staging(skills_dir)?;
    cleanup_staging(&disabled_root(app_data_dir))?;
    let backups = app_data_dir.join("skill-backups");
    if backups.is_dir() { cleanup_staging(&backups)?; }
    Ok(())
}

pub(super) fn catalog(
    skills_dir: &Path,
    app_data_dir: &Path,
) -> Result<Vec<InstalledSkillSummary>, CoreError> {
    Ok(catalog_report(skills_dir, app_data_dir)?.items)
}

fn catalog_report(skills_dir: &Path, app_data_dir: &Path) -> Result<ScanReport<InstalledSkillSummary>, CoreError> {
    let mut report = scan_installed_skills(skills_dir)?;
    let inactive_report = scan_installed_skills(&disabled_root(app_data_dir))?;
    report.warnings.extend(inactive_report.warnings);
    let items = &mut report.items;
    for active in items.iter_mut() {
        active.conflict =
            std::fs::symlink_metadata(disabled_root(app_data_dir).join(&active.relative_path))
                .is_ok();
        if active.conflict { active.conflict_directory_path = Some(disabled_root(app_data_dir).join(&active.relative_path).display().to_string()); }
    }
    for mut inactive in inactive_report.items {
        if let Some(active) = items.iter_mut().find(|item| item.id == inactive.id) {
            active.conflict = true;
        } else {
            inactive.enabled = false;
            inactive.conflict =
                std::fs::symlink_metadata(skills_dir.join(&inactive.relative_path)).is_ok();
            if inactive.conflict { inactive.conflict_directory_path = Some(skills_dir.join(&inactive.relative_path).display().to_string()); }
            items.push(inactive);
        }
    }
    items.sort_by(|a, b| {
        b.updated_at
            .unwrap_or(0)
            .cmp(&a.updated_at.unwrap_or(0))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    Ok(report)
}

pub fn load_skill_catalog(
    skills_dir: &Path,
    app_data_dir: &Path,
) -> Result<Vec<InstalledSkillSummary>, CoreError> {
    let _guard = write_guard(app_data_dir)?;
    catalog(skills_dir, app_data_dir)
}

pub fn load_skill_catalog_report(skills_dir: &Path, app_data_dir: &Path) -> Result<ScanReport<InstalledSkillSummary>, CoreError> {
    let _guard = write_guard(app_data_dir)?;
    catalog_report(skills_dir, app_data_dir)
}

/// Re-import and restore retain the current activation state.
pub(super) fn target_for(
    skills_dir: &Path,
    app_data_dir: &Path,
    id: &str,
) -> Result<(PathBuf, PathBuf, bool), CoreError> {
    if Path::new(id).components().any(|part| matches!(part, std::path::Component::Normal(name) if name.to_string_lossy().starts_with('.'))) {
        return Err(CoreError::InvalidData("Hidden skills cannot be discovered by Codex".into()));
    }
    let disabled = disabled_root(app_data_dir);
    let active_path = checked_child(skills_dir, id)?;
    let inactive_path = checked_child(&disabled, id)?;
    if std::fs::symlink_metadata(&active_path).is_ok()
        && std::fs::symlink_metadata(&inactive_path).is_ok()
    {
        return Err(CoreError::InvalidData(
            "Skill exists in both active and disabled directories; resolve the conflict first"
                .into(),
        ));
    }
    let result = if std::fs::symlink_metadata(&inactive_path).is_ok() {
        (disabled, inactive_path, false)
    } else {
        (skills_dir.to_path_buf(), active_path, true)
    };
    validate_destination(&result.0, &result.1)?;
    Ok(result)
}

fn validate_destination(root: &Path, target: &Path) -> Result<(), CoreError> {
    let mut ancestor = target.parent();
    while let Some(path) = ancestor.filter(|path| *path != root) {
        if path.join("SKILL.md").exists() {
            return Err(CoreError::InvalidData(
                "A parent directory is already a skill".into(),
            ));
        }
        ancestor = path.parent();
    }
    Ok(())
}

pub fn set_skill_enabled(
    skills_dir: &Path,
    app_data_dir: &Path,
    id: &str,
    enabled: bool,
) -> Result<InstalledSkillSummary, CoreError> {
    let _guard = write_guard(app_data_dir)?;
    prepare_roots(skills_dir, app_data_dir)?;
    let (_, source, was_enabled) = target_for(skills_dir, app_data_dir, id)?;
    let skill = catalog(skills_dir, app_data_dir)?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| CoreError::NotFound(format!("Skill not found: {id}")))?;
    if was_enabled == enabled {
        return Ok(skill);
    }
    validate_tree(&source)?;
    let root = if enabled {
        skills_dir.to_path_buf()
    } else {
        disabled_root(app_data_dir)
    };
    let target = checked_child(&root, id)?;
    // An ancestor SKILL.md would hide a restored nested skill from the scanner.
    validate_destination(&root, &target)?;
    std::fs::create_dir_all(target.parent().unwrap())?;
    let disabled = disabled_root(app_data_dir);
    let mut summary = load_skill_summary(&source.join("SKILL.md"), if was_enabled { skills_dir } else { &disabled })
        .ok_or_else(|| CoreError::InvalidData("Invalid skill after activation".into()))?;
    move_directory(&source, &target)?;
    summary.enabled = enabled;
    summary.directory_path = target.display().to_string();
    summary.skill_file_path = target.join("SKILL.md").display().to_string();
    Ok(summary)
}

pub(super) fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        } // junction/reparse point
    }
    metadata.file_type().is_symlink()
}

fn validate_tree(path: &Path) -> Result<(), CoreError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if is_link(&metadata) {
        return Err(CoreError::InvalidData(
            "Linked skills must be managed by their original installer".into(),
        ));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            validate_tree(&entry?.path())?;
        }
    }
    Ok(())
}

fn move_directory(source: &Path, target: &Path) -> Result<(), CoreError> {
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(CoreError::InvalidData(
            "Skill destination already exists".into(),
        ));
    }
    if std::fs::rename(source, target).is_ok() {
        return Ok(());
    }
    copy_then_retire(source, target)
}

fn copy_then_retire(source: &Path, target: &Path) -> Result<(), CoreError> {
    // Publish the full destination before retiring the source. An interruption
    // can leave a visible conflict, but never the only copy hidden in staging.
    let staged = target
        .parent()
        .unwrap()
        .join(format!(".agentharbor-stage-{}", uuid::Uuid::new_v4()));
    let previous = source
        .parent()
        .unwrap()
        .join(format!(".agentharbor-retired-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), CoreError> {
        copy_dir_all(source, &staged)?;
        std::fs::rename(&staged, target)?;
        if let Err(error) = std::fs::rename(source, &previous) {
            if let Err(rollback) = retire_directory(target) {
                return Err(CoreError::InvalidData(format!("Skill move failed: {error}; rollback failed: {rollback}; both copies retained at {} and {}. Resolve the conflict in Skills.", source.display(), target.display())));
            }
            return Err(error.into());
        }
        let _ = std::fs::remove_dir_all(&previous);
        Ok(())
    })();
    if staged.exists() {
        let _ = std::fs::remove_dir_all(staged);
    }
    result
}

pub(super) fn retire_directory(source: &Path) -> Result<(), CoreError> {
    validate_tree(source)?;
    let previous = source.parent().unwrap().join(format!(".agentharbor-retired-{}", uuid::Uuid::new_v4()));
    std::fs::rename(source, &previous)?;
    // The backup is already durable. Cleanup is retryable, not a failed removal.
    let _ = std::fs::remove_dir_all(previous);
    Ok(())
}

/// These names only contain disposable copies with a separate visible source or
/// backup. Run under the writer lock; do not traverse links or unknown dot dirs.
fn cleanup_staging(root: &Path) -> Result<(), CoreError> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let owned = [".agentharbor-stage-", ".agentharbor-retired-"].iter()
            .any(|prefix| name.strip_prefix(prefix).is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()));
        let meta = std::fs::symlink_metadata(&path)?;
        if is_link(&meta) { continue; }
        if !owned {
            if meta.is_dir() && !name.starts_with('.') && !path.join("SKILL.md").exists() { cleanup_staging(&path)?; }
            continue;
        }
        let _ = std::fs::remove_dir_all(path);
    }
    Ok(())
}

/// Keep the chosen placement and archive the other complete copy before hiding
/// it. Neither conflicting copy is overwritten or merged by guesswork.
pub fn resolve_skill_conflict(skills_dir: &Path, app_data_dir: &Path, id: &str, keep_enabled: bool) -> Result<InstalledSkillSummary, CoreError> {
    let _guard = write_guard(app_data_dir)?;
    prepare_roots(skills_dir, app_data_dir)?;
    let inactive_root = disabled_root(app_data_dir);
    let active = checked_child(skills_dir, id)?;
    let inactive = checked_child(&inactive_root, id)?;
    let (keep, discard, keep_root, discard_root) = if keep_enabled {
        (&active, &inactive, skills_dir, inactive_root.as_path())
    } else { (&inactive, &active, inactive_root.as_path(), skills_dir) };
    validate_tree(keep)?;
    validate_tree(discard)?;
    let mut result = load_skill_summary(&keep.join("SKILL.md"), keep_root)
        .ok_or_else(|| CoreError::InvalidData("Invalid chosen skill".into()))?;
    let backup = backup_skill_directory(discard, discard_root, &app_data_dir.join("skill-backups"), "conflict")?;
    retire_directory(discard).map_err(|error| with_recovery_backup(error, Some(&backup)))?;
    result.enabled = keep_enabled;
    result.conflict = false;
    result.conflict_directory_path = None;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_volume_copy_path_preserves_the_complete_skill() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir_all(source.join(".hidden")).unwrap();
        std::fs::write(source.join("SKILL.md"), "# Sample").unwrap();
        std::fs::write(source.join(".hidden/data"), [0, 255]).unwrap();
        copy_then_retire(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(target.join(".hidden/data")).unwrap(),
            [0, 255]
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn failed_retirement_keeps_the_original_complete() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "# Sample").unwrap();
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000)
            .open(&source)
            .unwrap();
        assert!(copy_then_retire(&source, &target).is_err());
        assert_eq!(
            std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
            "# Sample"
        );
        assert!(!target.exists());
    }
}
