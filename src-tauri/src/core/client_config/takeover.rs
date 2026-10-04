//! Analysis of the configuration that already exists before this apply runs:
//! what a first injection will overwrite, and what a higher-priority
//! managed-settings file would override anyway. Everything here only warns;
//! it never blocks an apply.

use super::*;

fn edit_after<'a>(edits: &'a [Edit], path: &[&str]) -> Option<&'a Value> {
    edits
        .iter()
        .find(|edit| {
            edit.path.len() == path.len()
                && edit
                    .path
                    .iter()
                    .zip(path)
                    .all(|(left, right)| left == right)
        })
        .and_then(|edit| edit.value.as_ref())
}

fn occupied<'a>(document: &'a Value, key: &str) -> Option<&'a Value> {
    document
        .get("env")
        .and_then(|env| env.get(key))
        .filter(|value| !value.is_null() && !value.as_str().is_some_and(str::is_empty))
}

/// Warnings for a first injection into an existing file: the overwritten
/// fields may belong to another switcher (cc-switch and friends) or to the
/// user's own setup. Only non-empty current values that actually change are
/// reported, and only when no session exists — a re-apply under a session is
/// already guarded by the field-level conflict check.
pub(super) fn takeover_warnings(document: &Value, edits: &[Edit], warnings: &mut Vec<String>) {
    let overwritten = |key: &str| -> bool {
        let Some(current) = occupied(document, key) else {
            return false;
        };
        match edit_after(edits, &["env", key]) {
            Some(next) => next != current,
            // The key is withdrawn instead of written (the unused auth field).
            None => true,
        }
    };
    if overwritten("ANTHROPIC_BASE_URL") {
        warnings.push("clients.warnings.takeoverEndpoint".into());
    }
    if overwritten("ANTHROPIC_AUTH_TOKEN") || overwritten("ANTHROPIC_API_KEY") {
        warnings.push("clients.warnings.takeoverCredential".into());
    }
    let model_next = edit_after(edits, &["model"]).and_then(Value::as_str);
    let model_taken = document
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|current| !current.is_empty() && Some(current) != model_next);
    let picker_taken = document
        .get("modelPicker")
        .and_then(|picker| picker.get("options"))
        .and_then(Value::as_array)
        .is_some_and(|entries| !entries.is_empty());
    let allowlist_taken = document
        .get("availableModels")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty());
    if model_taken || picker_taken || allowlist_taken {
        warnings.push("clients.warnings.takeoverModelConfig".into());
    }
}

/// The administrator's managed-settings.json outranks the user file, so its
/// env block silently overrides this injection while the user file looks
/// correct (the same check magpie runs before trusting its own wiring). The
/// lookup path is a static so tests can point it at a fixture.
pub(super) fn managed_settings_conflicts(edits: &[Edit], warnings: &mut Vec<String>) {
    let Some(path) = managed_settings_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(managed) = value.get("env").and_then(Value::as_object) else {
        return;
    };
    let conflicts = managed
        .iter()
        .filter(|(key, value)| {
            !value.is_null()
                && edit_after(edits, &["env", key.as_str()]).is_some_and(|next| next != *value)
        })
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>();
    if !conflicts.is_empty() {
        log::warn!(
            "managed settings at {} would override the injection: {conflicts:?}",
            path.display()
        );
        warnings.push("clients.warnings.managedSettings".into());
    }
}

static MANAGED_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(windows)]
fn default_managed_path() -> Option<PathBuf> {
    Some(PathBuf::from(
        r"C:\ProgramData\ClaudeCode\managed-settings.json",
    ))
}

#[cfg(target_os = "macos")]
fn default_managed_path() -> Option<PathBuf> {
    Some(PathBuf::from(
        "/Library/Application Support/ClaudeCode/managed-settings.json",
    ))
}

#[cfg(target_os = "linux")]
fn default_managed_path() -> Option<PathBuf> {
    Some(PathBuf::from("/etc/claude-code/managed-settings.json"))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn default_managed_path() -> Option<PathBuf> {
    None
}

fn managed_settings_path() -> Option<PathBuf> {
    MANAGED_PATH
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .or_else(default_managed_path)
}

#[cfg(test)]
pub(super) fn set_managed_settings_path(path: Option<PathBuf>) {
    *MANAGED_PATH.lock().unwrap() = path;
}
