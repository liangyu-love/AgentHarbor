use crate::core::repository::Repository;
use crate::core::skills::{self, SkillApp, SkillContext, SkillImport, SkillLibraryView};
use std::sync::Mutex;
use tauri::State;

fn context(repo: &State<'_, Mutex<Repository>>) -> Result<SkillContext, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    Ok(SkillContext::discover(repo.paths()))
}

/// The Skills library, each agent's switch, and the skills not yet in it.
#[tauri::command]
pub async fn load_skill_library(
    repo: State<'_, Mutex<Repository>>,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::load_skills(&context)).await
}

/// Turn skills on or off for one agent, or for every installed agent when `app` is absent.
#[tauri::command]
pub async fn set_skills(
    repo: State<'_, Mutex<Repository>>,
    names: Vec<String>,
    app: Option<SkillApp>,
    on: bool,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::set_skills(&context, &names, app, on)).await
}

/// Add a skill folder (or a SKILL.md's folder) and turn it on for `apps`.
#[tauri::command]
pub async fn add_skill(
    repo: State<'_, Mutex<Repository>>,
    path: String,
    apps: Vec<SkillApp>,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::add_skill(&context, &path, &apps)).await
}

/// Bring skills the agents already have into the library.
#[tauri::command]
pub async fn import_skills(
    repo: State<'_, Mutex<Repository>>,
    items: Vec<SkillImport>,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::import_skills(&context, &items)).await
}

#[tauri::command]
pub async fn uninstall_skill(
    repo: State<'_, Mutex<Repository>>,
    name: String,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::uninstall_skill(&context, &name)).await
}

#[tauri::command]
pub async fn restore_skill(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::restore_skill(&context, &id)).await
}

#[tauri::command]
pub async fn delete_skill_backup(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<SkillLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || skills::delete_backup(&context, &id)).await
}
