//! What this app wrote into the client's element-keyed arrays, so a later
//! apply can withdraw exactly those elements instead of guessing from row
//! shapes. Element-keyed paths are `modelPicker.options` (keyed by the row's
//! `model` id) and `availableModels` on Claude Code, and OpenCode's
//! `enabled_providers` and `disabled_providers` (keyed by the provider key;
//! from the latter this app takes keys out rather than adding them, and
//! withdrawing puts them back). The record lives beside
//! the session journal, survives `release` (the file keeps our rows, so a
//! later apply must still withdraw them) and retains an empty record on
//! `restore`. The record's presence also marks the legacy picker migration as
//! complete, so restored user rows stay untouched on a later apply.

use super::*;

const PICKER_OPTIONS: [&str; 2] = ["modelPicker", "options"];
const AVAILABLE_MODELS: [&str; 1] = ["availableModels"];
const ENABLED_PROVIDERS: [&str; 1] = ["enabled_providers"];
const DISABLED_PROVIDERS: [&str; 1] = ["disabled_providers"];

/// Every element-keyed path, so callers handle them all alike.
pub(super) const ELEMENT_PATHS: [&[&str]; 4] = [
    &PICKER_OPTIONS,
    &AVAILABLE_MODELS,
    &ENABLED_PROVIDERS,
    &DISABLED_PROVIDERS,
];

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct OwnedRecord {
    /// Model ids this app last added to `modelPicker.options`. Kept so records
    /// written by older versions can still be read; new withdrawal decisions
    /// use the complete row snapshots below.
    pub picker_models: Vec<String>,
    /// Exact rows this app last added. If a user edits one in place, it no
    /// longer matches this snapshot and is adopted instead of withdrawn.
    pub picker_entries: Vec<Value>,
    /// Values this app last added to `availableModels`.
    pub available_models: Vec<String>,
    /// Provider keys this app last added to `enabled_providers`.
    pub enabled_providers: Vec<String>,
    /// Provider keys this app last took out of `disabled_providers`.
    pub disabled_providers: Vec<String>,
}

pub(super) fn is_element_array_path(path: &[String]) -> bool {
    ELEMENT_PATHS.iter().any(|expected| is_path(path, expected))
}

fn is_path(path: &[String], expected: &[&str]) -> bool {
    path.len() == expected.len() && path.iter().zip(expected).all(|(left, right)| left == right)
}

fn element_keys(path: &[String], array: &[Value]) -> Vec<String> {
    if is_path(path, &PICKER_OPTIONS) {
        array
            .iter()
            .filter_map(|entry| entry.get("model").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    } else {
        array
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }
}

impl OwnedRecord {
    /// The elements one apply added beyond what it found: exactly what a
    /// later apply must withdraw. Rows the user already had are not owned,
    /// so their labels and metadata are never rewritten.
    pub(super) fn from_changes(changes: &[FieldChange]) -> Self {
        let mut record = Self::default();
        for change in changes {
            if !is_element_array_path(&change.path) {
                continue;
            }
            let Some(after) = change.after.as_ref().and_then(Value::as_array) else {
                continue;
            };
            let added = element_keys(&change.path, after);
            let existing = change
                .before
                .as_ref()
                .and_then(Value::as_array)
                .map(|before| element_keys(&change.path, before))
                .unwrap_or_default();
            if is_path(&change.path, &DISABLED_PROVIDERS) {
                record
                    .disabled_providers
                    .extend(existing.into_iter().filter(|key| !added.contains(key)));
                continue;
            }
            for key in added {
                if !existing.contains(&key) {
                    if is_path(&change.path, &PICKER_OPTIONS) {
                        if let Some(entry) = after.iter().find(|entry| {
                            entry.get("model").and_then(Value::as_str) == Some(key.as_str())
                        }) {
                            record.picker_models.push(key);
                            record.picker_entries.push(entry.clone());
                        }
                    } else if is_path(&change.path, &ENABLED_PROVIDERS) {
                        record.enabled_providers.push(key);
                    } else {
                        record.available_models.push(key);
                    }
                }
            }
        }
        record
    }

    pub(super) fn seed_from_session(session: &Session) -> Self {
        Self::from_changes(&session.changes)
    }

    /// Upgrade records written by older versions from their session journal:
    /// key-only picker records, and records from before `disabled_providers`
    /// was tracked. A released legacy session has nothing to recover, so its
    /// rows are conservatively treated as user data rather than risking edits
    /// being lost.
    pub(super) fn recover_from_session(&mut self, session: Option<&Session>) {
        let Some(session) = session else {
            return;
        };
        let recovered = Self::seed_from_session(session);
        if self.picker_entries.is_empty() {
            self.picker_entries = recovered.picker_entries;
        }
        if self.disabled_providers.is_empty() {
            self.disabled_providers = recovered.disabled_providers;
        }
    }

    /// A copy of the array without the elements this app owns (with the keys
    /// it took out of `disabled_providers` back in); everything the
    /// user added, edited or kept — including rows whose shape we no longer
    /// recognize — stays exactly as it is.
    pub(super) fn withdraw(&self, path: &[String], array: &[Value]) -> Vec<Value> {
        if is_path(path, &PICKER_OPTIONS) {
            return array
                .iter()
                .filter(|entry| !self.picker_entries.contains(entry))
                .cloned()
                .collect();
        }
        if is_path(path, &DISABLED_PROVIDERS) {
            let mut restored = array.to_vec();
            for key in &self.disabled_providers {
                if !element_keys(path, array).contains(key) {
                    restored.push(json!(key));
                }
            }
            return restored;
        }
        let owned = if is_path(path, &ENABLED_PROVIDERS) {
            &self.enabled_providers
        } else {
            &self.available_models
        };
        array
            .iter()
            .filter(|entry| {
                !element_keys(path, std::slice::from_ref(entry))
                    .iter()
                    .any(|key| owned.contains(key))
            })
            .cloned()
            .collect()
    }
}

pub(super) fn path(data_dir: &Path, id: TargetId) -> PathBuf {
    data_dir.join(match id {
        TargetId::ClaudeCode => "claude-code.owned.dat",
        TargetId::OpenCode => "opencode.owned.dat",
        TargetId::Pi => "pi.owned.dat",
        TargetId::Harness => "harness.owned.dat",
        TargetId::Codex => "codex.owned.dat",
    })
}

pub(super) fn load(data_dir: &Path, id: TargetId) -> OwnedRecord {
    secret_file::read(&path(data_dir, id))
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Best-effort: a lost or corrupt record degrades to the session-derived seed
/// and to keeping existing rows — never to deleting user rows.
pub(super) fn save(data_dir: &Path, id: TargetId, record: &OwnedRecord) {
    if let Ok(bytes) = serde_json::to_vec(record) {
        if let Err(error) = secret_file::write(&path(data_dir, id), &bytes) {
            log::warn!("could not save the {id:?} ownership record: {error}");
        }
    }
}
