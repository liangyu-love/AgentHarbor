use crate::core::custom_instructions;
use crate::core::models::{
    CoreEnvelope, CustomInstructionPreviewPayload, CustomInstructionStatePayload,
};
use crate::core::repository::Repository;
use crate::core::{client_config::ClientPaths, targets::TargetId};
use std::sync::Mutex;
use tauri::State;

fn instruction_error(error: crate::core::models::CoreError) -> String {
    match error {
        crate::core::models::CoreError::OperationFailed(message)
            if message == "customInstructions.fileChanged" =>
        {
            message
        }
        error => error.to_string(),
    }
}

#[tauri::command]
pub fn load_custom_instruction_state(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
) -> Result<CoreEnvelope<CustomInstructionStatePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = custom_instructions::InstructionPaths::for_target(
        repo.paths(),
        &ClientPaths::discover(),
        target_id.unwrap_or(TargetId::Codex),
    )
    .map_err(instruction_error)?;
    let payload = custom_instructions::load_state(&paths).map_err(instruction_error)?;
    Ok(CoreEnvelope::ok(payload))
}

#[tauri::command]
pub fn preview_custom_instruction_apply(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    content: String,
) -> Result<CoreEnvelope<CustomInstructionPreviewPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = custom_instructions::InstructionPaths::for_target(
        repo.paths(),
        &ClientPaths::discover(),
        target_id.unwrap_or(TargetId::Codex),
    )
    .map_err(instruction_error)?;
    let payload =
        custom_instructions::preview_apply(&paths, &content).map_err(instruction_error)?;
    Ok(CoreEnvelope::ok(payload))
}

#[tauri::command]
pub fn apply_custom_instruction(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    content: String,
    template_code: Option<String>,
    template_title: Option<String>,
    source: Option<String>,
    expected_content: Option<String>,
) -> Result<CoreEnvelope<CustomInstructionStatePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = custom_instructions::InstructionPaths::for_target(
        repo.paths(),
        &ClientPaths::discover(),
        target_id.unwrap_or(TargetId::Codex),
    )
    .map_err(instruction_error)?;
    if let Some(expected) = expected_content {
        if custom_instructions::load_state(&paths)
            .map_err(instruction_error)?
            .current
            .managed_content
            != expected
        {
            return Err("customInstructions.fileChanged".into());
        }
    }
    let payload = custom_instructions::apply_managed_content(
        &paths,
        &content,
        template_code,
        template_title,
        source,
    )
    .map_err(instruction_error)?;
    Ok(CoreEnvelope::ok(payload))
}

#[tauri::command]
pub fn clear_custom_instruction_block(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
) -> Result<CoreEnvelope<CustomInstructionStatePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = custom_instructions::InstructionPaths::for_target(
        repo.paths(),
        &ClientPaths::discover(),
        target_id.unwrap_or(TargetId::Codex),
    )
    .map_err(instruction_error)?;
    let payload = custom_instructions::clear_managed_block(&paths).map_err(instruction_error)?;
    Ok(CoreEnvelope::ok(payload))
}

#[tauri::command]
pub fn rollback_custom_instruction(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    history_id: String,
) -> Result<CoreEnvelope<CustomInstructionStatePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = custom_instructions::InstructionPaths::for_target(
        repo.paths(),
        &ClientPaths::discover(),
        target_id.unwrap_or(TargetId::Codex),
    )
    .map_err(instruction_error)?;
    let payload =
        custom_instructions::rollback_history(&paths, &history_id).map_err(instruction_error)?;
    Ok(CoreEnvelope::ok(payload))
}
