use crate::core::auth::current_timestamp;
use crate::core::client_config::ClientPaths;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::skills;
use crate::core::skills::{SkillPaths, SkillSource};
use crate::platform::paths::CodexPaths;
use std::sync::Mutex;
use tauri::State;

fn source_paths(paths: &CodexPaths, source: Option<SkillSource>) -> Result<SkillPaths, String> {
    SkillPaths::for_source(paths, &ClientPaths::discover(), source.unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_installed_skills(
    repo: State<'_, Mutex<Repository>>,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillListPayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let report = if source == Some(SkillSource::ClaudeCode) {
            skills::load_skill_catalog_report_with_links(&paths.skills_dir, &paths.app_data_dir)
        } else {
            skills::load_skill_catalog_report(&paths.skills_dir, &paths.app_data_dir)
        }
        .map_err(|e| e.to_string())?;
        let payload = SkillListPayload {
            total: report.items.len() as i32,
            root_path: paths.skills_dir.display().to_string(),
            last_scan_at: current_timestamp(),
            items: report.items,
        };
        Ok(CoreEnvelope::ok_with_warnings(payload, report.warnings))
    })
    .await
}

#[tauri::command]
pub async fn set_skill_enabled(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    enabled: bool,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<InstalledSkillSummary>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let skill = skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, enabled)
            .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(skill))
    })
    .await
}

#[tauri::command]
pub async fn resolve_skill_conflict(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    keep_enabled: bool,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<InstalledSkillSummary>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let item = skills::resolve_skill_conflict(
            &paths.skills_dir,
            &paths.app_data_dir,
            &id,
            keep_enabled,
        )
        .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(item))
    })
    .await
}

#[tauri::command]
pub async fn load_skill_backups(
    repo: State<'_, Mutex<Repository>>,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillBackupListPayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let report = skills::scan_skill_backups(&paths.backups_dir).map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok_with_warnings(
            SkillBackupListPayload {
                total: report.items.len() as i32,
                root_path: paths.backups_dir.display().to_string(),
                last_scan_at: current_timestamp(),
                items: report.items,
            },
            report.warnings,
        ))
    })
    .await
}

#[tauri::command]
pub async fn import_skill(
    repo: State<'_, Mutex<Repository>>,
    path: String,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillImportPayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let payload = skills::import_skill(&paths.skills_dir, &paths.app_data_dir, &path)
            .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}

#[tauri::command]
pub async fn remove_skill(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillRemovePayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let payload = skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &id)
            .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}

#[tauri::command]
pub async fn restore_skill_backup(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillRestorePayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let payload = skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &id)
            .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}

#[tauri::command]
pub async fn delete_skill_backup(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    source: Option<SkillSource>,
) -> Result<CoreEnvelope<SkillDeleteBackupPayload>, String> {
    let codex = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let paths = source_paths(&codex, source)?;
        let payload =
            skills::delete_skill_backup(&paths.app_data_dir, &id).map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}
