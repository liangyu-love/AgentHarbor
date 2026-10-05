use crate::core::auth::current_timestamp;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::skills;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub async fn load_installed_skills(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<SkillListPayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let items = skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir)
            .map_err(|e| e.to_string())?;
        let payload = SkillListPayload {
            total: items.len() as i32,
            root_path: paths.skills_dir.display().to_string(),
            last_scan_at: current_timestamp(),
            items,
        };
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}

#[tauri::command]
pub async fn set_skill_enabled(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    enabled: bool,
) -> Result<CoreEnvelope<InstalledSkillSummary>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let skill = skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, enabled)
            .map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(skill))
    })
    .await
}

#[tauri::command]
pub async fn load_skill_backups(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<SkillBackupListPayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let items =
            skills::load_skill_backups(&paths.skill_backups_dir).map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(SkillBackupListPayload {
            total: items.len() as i32,
            root_path: paths.skill_backups_dir.display().to_string(),
            last_scan_at: current_timestamp(),
            items,
        }))
    })
    .await
}

#[tauri::command]
pub async fn import_skill(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<CoreEnvelope<SkillImportPayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
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
) -> Result<CoreEnvelope<SkillRemovePayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
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
) -> Result<CoreEnvelope<SkillRestorePayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
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
) -> Result<CoreEnvelope<SkillDeleteBackupPayload>, String> {
    let paths = repo.lock().map_err(|e| e.to_string())?.paths().clone();
    super::blocking(move || {
        let payload =
            skills::delete_skill_backup(&paths.app_data_dir, &id).map_err(|e| e.to_string())?;
        Ok(CoreEnvelope::ok(payload))
    })
    .await
}
