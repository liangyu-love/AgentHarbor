use crate::core::auth::current_timestamp;
use crate::core::models::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod activation;
mod source;
pub use source::{SkillPaths, SkillSource};
pub use activation::{load_skill_catalog, load_skill_catalog_report, load_skill_catalog_report_with_links, resolve_skill_conflict, set_skill_enabled};

#[derive(Default)]
pub struct ScanReport<T> {
    pub items: Vec<T>,
    pub warnings: Vec<CoreWarning>,
}

fn scan_warning(code: &str, directory: &Path) -> CoreWarning {
    CoreWarning { code: format!("skills.warnings.{code}"), message: directory.display().to_string() }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillBackupMetadata {
    #[serde(rename = "backupID")]
    backup_id: String,
    #[serde(rename = "skillID")]
    skill_id: String,
    name: String,
    title: Option<String>,
    relative_path: String,
    created_at: i64,
}

pub fn load_installed_skills(skills_dir: &Path) -> Result<Vec<InstalledSkillSummary>, CoreError> {
    Ok(scan_installed_skills(skills_dir)?.items)
}

pub fn scan_installed_skills(skills_dir: &Path) -> Result<ScanReport<InstalledSkillSummary>, CoreError> {
    scan_installed_skills_with_links(skills_dir, false)
}

fn scan_installed_skills_with_links(skills_dir: &Path, include_links: bool) -> Result<ScanReport<InstalledSkillSummary>, CoreError> {
    let mut report = ScanReport { items: Vec::new(), warnings: Vec::new() };
    match std::fs::symlink_metadata(skills_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    scan_skills_recursive(skills_dir, skills_dir, &mut report, include_links)?;
    report.items.sort_by(|a, b| {
        b.updated_at
            .unwrap_or(0)
            .cmp(&a.updated_at.unwrap_or(0))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(report)
}

fn scan_skills_recursive(dir: &Path, root: &Path, report: &mut ScanReport<InstalledSkillSummary>, include_links: bool) -> Result<(), CoreError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if dir != root && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if activation::is_link(&metadata) {
            if include_links && !entry.file_name().to_string_lossy().starts_with('.') {
                // CC Switch and other installers own these projections. Read a
                // direct SKILL.md only; never walk a link or mutate its target.
                match std::fs::read_to_string(path.join("SKILL.md")) {
                    Ok(text) => {
                        if let Some(mut summary) = skill_summary_from_text(&path.join("SKILL.md"), root, &text) {
                            summary.read_only = true;
                            report.items.push(summary);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::InvalidData => report.warnings.push(scan_warning("invalidEncoding", &path)),
                    Err(_) => report.warnings.push(scan_warning("invalidLinkedSkill", &path)),
                }
            }
            continue;
        }
        if path
            .file_name()
            .map_or(false, |n| n.to_str().map_or(false, |s| s.starts_with('.')))
        {
            continue;
        }
        if metadata.is_dir() {
            let skill_file = path.join("SKILL.md");
            match std::fs::read_to_string(&skill_file) {
                Ok(text) => {
                    if let Some(summary) = skill_summary_from_text(&skill_file, root, &text) {
                        report.items.push(summary);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => scan_skills_recursive(&path, root, report, include_links)?,
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => report.warnings.push(scan_warning("invalidEncoding", &path)),
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

fn load_skill_summary(skill_file: &Path, root: &Path) -> Option<InstalledSkillSummary> {
    let text = std::fs::read_to_string(skill_file).ok()?;
    skill_summary_from_text(skill_file, root, &text)
}

fn skill_summary_from_text(skill_file: &Path, root: &Path, text: &str) -> Option<InstalledSkillSummary> {
    let dir = skill_file.parent()?;
    let title = first_markdown_heading(&text);
    let summary = first_skill_summary_line(&text);
    let updated_at = std::fs::metadata(skill_file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let relative: PathBuf = dir
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(dir.file_name().unwrap_or_default()));
    let name = dir.file_name()?.to_str()?.to_string();
    let relative_path = relative.display().to_string();

    Some(InstalledSkillSummary {
        id: relative_path.clone(),
        enabled: true,
        conflict: false,
        conflict_directory_path: None,
        read_only: false,
        name,
        title,
        summary,
        relative_path,
        directory_path: dir.display().to_string(),
        skill_file_path: skill_file.display().to_string(),
        updated_at,
    })
}

pub fn load_skill_backups(backup_dir: &Path) -> Result<Vec<SkillBackupSummary>, CoreError> {
    Ok(scan_skill_backups(backup_dir)?.items)
}

pub fn scan_skill_backups(backup_dir: &Path) -> Result<ScanReport<SkillBackupSummary>, CoreError> {
    let mut report = ScanReport { items: Vec::new(), warnings: Vec::new() };
    match std::fs::symlink_metadata(backup_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    for entry in std::fs::read_dir(backup_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || activation::is_link(&metadata) || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let meta_path = path.join("metadata.json");
        let data = match std::fs::read_to_string(&meta_path) {
            Ok(data) => data,
            Err(_) => {
                report.warnings.push(scan_warning("unreadableMetadata", &path));
                continue;
            }
        };
        let meta: SkillBackupMetadata = match serde_json::from_str(&data) {
            Ok(meta) => meta,
            Err(_) => { report.warnings.push(scan_warning("corruptMetadata", &path)); continue; }
        };
        report.items.push(SkillBackupSummary {
            id: meta.backup_id,
            skill_id: meta.skill_id,
            name: meta.name,
            title: meta.title,
            relative_path: meta.relative_path,
            backup_path: path.join("skill").display().to_string(),
            created_at: meta.created_at,
        });
    }
    report.items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(report)
}

fn with_recovery_backup(error: CoreError, backup: Option<&SkillBackupSummary>) -> CoreError {
    match backup {
        Some(backup) => CoreError::InvalidData(format!("{error}; recovery backup {} retained at {}", backup.id, backup.backup_path)),
        None => error,
    }
}

pub fn import_skill(
    skills_dir: &Path,
    app_data_dir: &Path,
    source_path: &str,
) -> Result<SkillImportPayload, CoreError> {
    let _guard = activation::write_guard(app_data_dir)?;
    activation::prepare_roots(skills_dir, app_data_dir)?;
    let backup_dir = app_data_dir.join("skill-backups");
    std::fs::create_dir_all(&backup_dir)?;

    let source = resolve_skill_source(Path::new(source_path))?;
    let id = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CoreError::InvalidData("Invalid skill name".into()))?;
    if id.starts_with('.') {
        return Err(CoreError::InvalidData(
            "Hidden skills cannot be discovered by the client".into(),
        ));
    }
    let (root, target, enabled) = activation::target_for(skills_dir, app_data_dir, id)?;

    let source_canonical = std::fs::canonicalize(&source).unwrap_or_else(|_| source.clone());
    let target_canonical = if target.exists() {
        std::fs::canonicalize(&target).unwrap_or_else(|_| target.clone())
    } else {
        activation::resolved_location(&target)?
    };
    if target_canonical.starts_with(&source_canonical) && target_canonical != source_canonical {
        return Err(CoreError::InvalidData(
            "Cannot import a skill into its own subdirectory".into(),
        ));
    }
    if source_canonical == target_canonical {
        let mut skill = load_skill_summary(&target.join("SKILL.md"), &root)
            .ok_or_else(|| CoreError::InvalidData("Invalid skill source".into()))?;
        skill.enabled = enabled;
        return Ok(SkillImportPayload {
            skill,
            replaced_existing: false,
            backup: None,
        });
    }

    let mut skill = load_skill_summary(&source.join("SKILL.md"), &source)
        .ok_or_else(|| CoreError::InvalidData("Invalid skill after import".into()))?;
    let mut backup = None;
    let replaced = target.exists();
    if replaced {
        backup = Some(backup_skill_directory(
            &target,
            &root,
            &backup_dir,
            "replace",
        )?);
    }

    replace_skill_directory(&source, &target).map_err(|error| with_recovery_backup(error, backup.as_ref()))?;
    skill.id = id.to_string();
    skill.relative_path = id.to_string();
    skill.directory_path = target.display().to_string();
    skill.skill_file_path = target.join("SKILL.md").display().to_string();
    skill.enabled = enabled;

    Ok(SkillImportPayload {
        skill,
        replaced_existing: replaced,
        backup,
    })
}

pub fn remove_skill(
    skills_dir: &Path,
    app_data_dir: &Path,
    id: &str,
) -> Result<SkillRemovePayload, CoreError> {
    let _guard = activation::write_guard(app_data_dir)?;
    activation::prepare_roots(skills_dir, app_data_dir)?;
    let installed = activation::catalog(skills_dir, app_data_dir)?;
    installed
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::NotFound(format!("Skill not found: {id}")))?;

    let backup_dir = app_data_dir.join("skill-backups");
    let (root, dir, _) = activation::target_for(skills_dir, app_data_dir, id)?;
    let backup = backup_skill_directory(&dir, &root, &backup_dir, "remove")?;

    // The complete backup exists before the atomic retirement. A failed cleanup
    // leaves only a hidden tombstone; never partially delete a visible skill.
    activation::retire_directory(&dir).map_err(|error| with_recovery_backup(error, Some(&backup)))?;
    let remaining = (installed.len() - 1) as i32;

    Ok(SkillRemovePayload {
        removed_skill_id: id.to_string(),
        backup,
        remaining_installed_count: remaining,
    })
}

pub fn restore_skill_backup(
    skills_dir: &Path,
    app_data_dir: &Path,
    backup_id: &str,
) -> Result<SkillRestorePayload, CoreError> {
    let _guard = activation::write_guard(app_data_dir)?;
    activation::prepare_roots(skills_dir, app_data_dir)?;
    let backup_dir = app_data_dir.join("skill-backups");
    let backup_path = checked_child(&backup_dir, backup_id)?;
    if !backup_path.exists() {
        return Err(CoreError::NotFound(format!(
            "Backup not found: {backup_id}"
        )));
    }

    let meta_data = std::fs::read_to_string(backup_path.join("metadata.json"))?;
    let meta: SkillBackupMetadata = serde_json::from_str(&meta_data)?;
    let staged = backup_path.join("skill");
    if !staged.exists() {
        return Err(CoreError::InvalidData(format!(
            "Backup corrupted: {backup_id}"
        )));
    }

    let (root, target, enabled) =
        activation::target_for(skills_dir, app_data_dir, &meta.relative_path)?;
    std::fs::create_dir_all(target.parent().unwrap_or(&root))?;

    let mut restored = load_skill_summary(&staged.join("SKILL.md"), &staged)
        .ok_or_else(|| CoreError::InvalidData("Backup corrupted".into()))?;
    let mut rollback_backup = None;
    if target.exists() {
        rollback_backup = Some(backup_skill_directory(
            &target,
            &root,
            &backup_dir,
            "restore-rollback",
        )?);
    }

    replace_skill_directory(&staged, &target).map_err(|error| with_recovery_backup(error, rollback_backup.as_ref()))?;
    restored.id = meta.relative_path.clone();
    restored.name = meta.name.clone();
    restored.relative_path = meta.relative_path.clone();
    restored.directory_path = target.display().to_string();
    restored.skill_file_path = target.join("SKILL.md").display().to_string();
    restored.enabled = enabled;

    let backup_summary = SkillBackupSummary {
        id: meta.backup_id,
        skill_id: meta.skill_id,
        name: meta.name,
        title: meta.title,
        relative_path: meta.relative_path,
        backup_path: staged.display().to_string(),
        created_at: meta.created_at,
    };

    Ok(SkillRestorePayload {
        restored_skill: restored,
        backup: backup_summary,
        rollback_backup,
    })
}

pub fn delete_skill_backup(
    app_data_dir: &Path,
    backup_id: &str,
) -> Result<SkillDeleteBackupPayload, CoreError> {
    let _guard = activation::write_guard(app_data_dir)?;
    let backup_dir = app_data_dir.join("skill-backups");
    let path = checked_child(&backup_dir, backup_id)?;
    if !path.exists() {
        return Err(CoreError::NotFound(format!(
            "Backup not found: {backup_id}"
        )));
    }
    let remaining = load_skill_backups(&backup_dir)?.len().saturating_sub(1) as i32;
    activation::retire_directory(&path)?;
    Ok(SkillDeleteBackupPayload {
        deleted_backup_id: backup_id.to_string(),
        remaining_backup_count: remaining,
    })
}

fn backup_skill_directory(
    dir: &Path,
    skills_root: &Path,
    backup_dir: &Path,
    reason: &str,
) -> Result<SkillBackupSummary, CoreError> {
    let skill = load_skill_summary(&dir.join("SKILL.md"), skills_root)
        .ok_or_else(|| CoreError::InvalidData("Invalid skill source".into()))?;
    std::fs::create_dir_all(backup_dir)?;
    let ts = current_timestamp();
    let safe_path = skill.relative_path.replace(['/', '\\'], "__");
    let backup_id = format!(
        "{ts}-{safe_path}-{reason}-{}",
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let backup_path = backup_dir.join(&backup_id);
    let staged = backup_path.join("skill");
    std::fs::create_dir_all(&backup_path)?;
    copy_dir_all(dir, &staged)?;

    let meta = SkillBackupMetadata {
        backup_id: backup_id.clone(),
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        title: skill.title.clone(),
        relative_path: skill.relative_path.clone(),
        created_at: ts,
    };
    let meta_json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(backup_path.join("metadata.json"), meta_json)?;

    Ok(SkillBackupSummary {
        id: backup_id,
        skill_id: skill.id,
        name: skill.name,
        title: skill.title,
        relative_path: skill.relative_path,
        backup_path: staged.display().to_string(),
        created_at: ts,
    })
}

fn resolve_skill_source(path: &Path) -> Result<PathBuf, CoreError> {
    if !path.exists() {
        return Err(CoreError::NotFound(format!(
            "Path not found: {}",
            path.display()
        )));
    }
    if path.is_dir() {
        if path.join("SKILL.md").exists() {
            return Ok(path.to_path_buf());
        }
        return Err(CoreError::InvalidData(
            "Directory must contain SKILL.md".into(),
        ));
    }
    if path.file_name().map_or(false, |n| n == "SKILL.md") {
        return Ok(path.parent().unwrap_or(path).to_path_buf());
    }
    Err(CoreError::InvalidData(
        "Must be a directory with SKILL.md or a SKILL.md file".into(),
    ))
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), CoreError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        if activation::is_link(&std::fs::symlink_metadata(entry.path())?) {
            return Err(CoreError::InvalidData(
                "Symbolic links inside skills are not supported".into(),
            ));
        }
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn replace_skill_directory(source: &Path, target: &Path) -> Result<(), CoreError> {
    if std::fs::symlink_metadata(target).map_or(false, |meta| activation::is_link(&meta)) {
        return Err(CoreError::InvalidData(
            "Cannot replace a linked skill directory".into(),
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| CoreError::InvalidData("Missing skill parent directory".into()))?;
    let staged = parent.join(format!(".agentharbor-stage-{}", uuid::Uuid::new_v4()));
    let previous = parent.join(format!(".agentharbor-previous-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), CoreError> {
        copy_dir_all(source, &staged)?;
        std::fs::read_to_string(staged.join("SKILL.md"))?;
        let replacing = target.exists();
        if replacing {
            std::fs::rename(target, &previous)?;
        }
        if let Err(error) = std::fs::rename(&staged, target) {
            if replacing {
                if let Err(rollback) = std::fs::rename(&previous, target) {
                    return Err(CoreError::InvalidData(format!("Skill publish failed: {error}; rollback failed: {rollback}; original retained at {}", previous.display())));
                }
            }
            return Err(error.into());
        }
        if replacing { let _ = activation::retire_directory(&previous); }
        Ok(())
    })();
    if staged.exists() {
        let _ = std::fs::remove_dir_all(staged);
    }
    result
}

fn checked_child(root: &Path, relative: &str) -> Result<PathBuf, CoreError> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(CoreError::InvalidData(
            "Invalid skill or backup path".into(),
        ));
    }
    let target = root.join(relative);
    let mut component_path = target.as_path();
    while component_path != root {
        if std::fs::symlink_metadata(component_path).is_ok_and(|meta| activation::is_link(&meta)) {
            return Err(CoreError::InvalidData(
                "Linked skill or backup paths are not supported".into(),
            ));
        }
        component_path = component_path
            .parent()
            .ok_or_else(|| CoreError::InvalidData("Invalid skill path".into()))?;
    }
    let root_resolved = std::fs::canonicalize(root)?;
    let mut ancestor = target.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| CoreError::InvalidData("Invalid skill path".into()))?;
    }
    if !std::fs::canonicalize(ancestor)?.starts_with(root_resolved) {
        return Err(CoreError::InvalidData(
            "Skill path is outside the managed directory".into(),
        ));
    }
    Ok(target)
}

fn first_markdown_heading(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading: String = trimmed
                .chars()
                .skip_while(|c| *c == '#' || *c == ' ')
                .collect();
            if !heading.is_empty() {
                return Some(heading);
            }
        }
    }
    None
}

fn first_skill_summary_line(text: &str) -> Option<String> {
    let mut in_frontmatter = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter {
            continue;
        }
        if trimmed.starts_with('#')
            || trimmed.starts_with("```")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
        {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}
