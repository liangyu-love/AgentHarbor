//! What this app wrote into the client's element-keyed arrays, so a later
//! apply can withdraw exactly those elements instead of guessing from row
//! shapes. Two paths are element-keyed on Claude Code: `modelPicker.options`
//! (keyed by the row's `model` id) and `availableModels` (keyed by the value
//! itself). The record lives beside the session journal, survives `release`
//! (the file keeps our rows, so a later apply must still withdraw them) and
//! retains an empty record on `restore`. The record's presence also marks the
//! legacy picker migration as complete, so restored user rows stay untouched
//! on a later apply.

use super::*;

const PICKER_OPTIONS: [&str; 2] = ["modelPicker", "options"];
const AVAILABLE_MODELS: [&str; 1] = ["availableModels"];

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
}

pub(super) fn is_element_array_path(path: &[String]) -> bool {
    is_path(path, &PICKER_OPTIONS) || is_path(path, &AVAILABLE_MODELS)
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
            for key in added {
                if !existing.contains(&key) {
                    if is_path(&change.path, &PICKER_OPTIONS) {
                        if let Some(entry) = after.iter().find(|entry| {
                            entry.get("model").and_then(Value::as_str) == Some(key.as_str())
                        }) {
                            record.picker_models.push(key);
                            record.picker_entries.push(entry.clone());
                        }
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

    /// Upgrade older key-only ownership records from their session journal.
    /// A released legacy session has no snapshots to recover, so its rows are
    /// conservatively treated as user data rather than risking edits being lost.
    pub(super) fn recover_picker_entries(&mut self, session: Option<&Session>) {
        if !self.picker_entries.is_empty() {
            return;
        }
        let Some(session) = session else {
            return;
        };
        let recovered = Self::seed_from_session(session);
        self.picker_entries = recovered.picker_entries;
    }

    /// A copy of the array without the elements this app owns; everything the
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
        let owned = &self.available_models;
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
