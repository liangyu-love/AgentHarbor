//! Skills: one library copy of each skill, linked into the skills folder of
//! every agent it is on (`hub.rs`), with backups of what is replaced or removed.
use crate::core::auth::current_timestamp;
use crate::core::models::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod hub;
mod link;
pub use hub::*;

#[derive(Default)]
pub struct ScanReport<T> {
    pub items: Vec<T>,
    pub warnings: Vec<CoreWarning>,
}

fn scan_warning(code: &str, directory: &Path) -> CoreWarning {
    CoreWarning {
        code: format!("skills.warnings.{code}"),
        message: directory.display().to_string(),
    }
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

fn load_skill_summary(skill_file: &Path, root: &Path) -> Option<InstalledSkillSummary> {
    let text = std::fs::read_to_string(skill_file).ok()?;
    skill_summary_from_text(skill_file, root, &text)
}

fn skill_summary_from_text(
    skill_file: &Path,
    root: &Path,
    text: &str,
) -> Option<InstalledSkillSummary> {
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
    let mut report = ScanReport {
        items: Vec::new(),
        warnings: Vec::new(),
    };
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
        if !metadata.is_dir()
            || is_link(&metadata)
            || entry.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let meta_path = path.join("metadata.json");
        let data = match std::fs::read_to_string(&meta_path) {
            Ok(data) => data,
            Err(_) => {
                report
                    .warnings
                    .push(scan_warning("unreadableMetadata", &path));
                continue;
            }
        };
        let meta: SkillBackupMetadata = match serde_json::from_str(&data) {
            Ok(meta) => meta,
            Err(_) => {
                report.warnings.push(scan_warning("corruptMetadata", &path));
                continue;
            }
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
        if is_link(&std::fs::symlink_metadata(entry.path())?) {
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

/// A symlink, or on Windows any reparse point (a junction).
fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Refuses a tree holding links: removing it must not reach through them.
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

/// Removes a directory that is already backed up: renamed aside first, so an
/// interrupted cleanup leaves a hidden leftover, never a half-deleted skill.
fn retire_directory(source: &Path) -> Result<(), CoreError> {
    validate_tree(source)?;
    let previous = source
        .parent()
        .ok_or_else(|| CoreError::InvalidData("Invalid skill path".into()))?
        .join(format!(".agentharbor-retired-{}", uuid::Uuid::new_v4()));
    std::fs::rename(source, &previous)?;
    let _ = std::fs::remove_dir_all(previous);
    Ok(())
}
