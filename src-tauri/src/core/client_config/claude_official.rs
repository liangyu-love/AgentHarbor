//! Switching to subscription login is different from restoring the takeover
//! baseline: that baseline may itself point at a third-party gateway.
use super::*;

const ENV_FIELDS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_PROFILE",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_REASONING_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_AWS_BASE_URL",
    "ANTHROPIC_AWS_API_KEY",
    "ANTHROPIC_AWS_WORKSPACE_ID",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeOfficialPreview {
    pub path: String,
    pub revision: String,
    /// Field names only. No credential values or original document are exposed.
    pub removed_fields: Vec<String>,
    pub warnings: Vec<String>,
    pub has_changes: bool,
}

struct OfficialPlan {
    path: PathBuf,
    current: Option<String>,
    next: Value,
    document_changed: bool,
    preview: ClaudeOfficialPreview,
}

impl ConfigManager {
    pub fn preview_claude_official(&self) -> Result<ClaudeOfficialPreview, String> {
        Ok(self.prepare_claude_official()?.preview)
    }

    pub fn switch_claude_official(&self, revision: &str) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        let plan = self.prepare_claude_official()?;
        if plan.preview.revision != revision {
            return Err("clients.errors.changed".into());
        }
        if plan.document_changed {
            let next = format!(
                "{}\n",
                serde_json::to_string_pretty(&plan.next).map_err(|_| "clients.errors.storage")?
            );
            return self.commit(
                TargetId::ClaudeCode,
                &plan.path,
                plan.current,
                Some(next),
                None,
                plan.preview.warnings,
            );
        }
        if plan.preview.has_changes {
            // The user may already have cleared the fields themselves. Release
            // the remaining journal without rewriting their exact file bytes.
            if read_text(&plan.path)? != plan.current {
                return Err("clients.errors.changed".into());
            }
            let journal = self.session_path(TargetId::ClaudeCode);
            if journal.exists() {
                storage::remove_file_durable(&journal).map_err(|_| "clients.errors.storage")?;
            }
            owned::save(
                &self.data_dir,
                TargetId::ClaudeCode,
                &owned::OwnedRecord::default(),
            );
        }
        Ok(ConfigApplied {
            path: plan.preview.path,
            backup_path: String::new(),
            warnings: plan.preview.warnings,
        })
    }

    fn prepare_claude_official(&self) -> Result<OfficialPlan, String> {
        let id = TargetId::ClaudeCode;
        let session_bytes = secret_file::read(&self.session_path(id))?;
        let session = self.session(id)?;
        let path = session
            .as_ref()
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| self.paths.claude.clone());
        let current = read_text(&path)?;
        let mut next = parse(current.as_deref(), id)?;
        if next.get("env").is_some_and(|value| !value.is_object())
            || next
                .get("modelPicker")
                .is_some_and(|value| !value.is_object())
        {
            return Err("clients.errors.invalidConfig".into());
        }
        let before = next.clone();
        let owned_bytes = secret_file::read(&owned::path(&self.data_dir, id))?;
        // Older journals can supply ownership if the separate record was lost.
        let mut owned = owned_bytes
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<owned::OwnedRecord>(bytes).ok())
            .unwrap_or_else(|| {
                session
                    .as_ref()
                    .map(owned::OwnedRecord::seed_from_session)
                    .unwrap_or_default()
            });
        owned.recover_picker_entries(session.as_ref());
        let mut removed = Vec::new();
        for name in ENV_FIELDS {
            clean_field(&mut next, &["env", name], &mut removed)?;
        }
        for name in ["model", "apiKeyHelper", "availableModels"] {
            clean_field(&mut next, &[name], &mut removed)?;
        }
        if let Some(options) = next.pointer("/modelPicker/options") {
            let options = options.as_array().ok_or("clients.errors.invalidConfig")?;
            let remaining = owned.withdraw(&["modelPicker".into(), "options".into()], options);
            if remaining != *options {
                set_field(
                    &mut next,
                    &["modelPicker".into(), "options".into()],
                    (!remaining.is_empty()).then(|| json!(remaining)),
                )?;
                removed.push("modelPicker.options".into());
            }
        }
        clean_field(
            &mut next,
            &["modelPicker", "replaceBuiltInOptions"],
            &mut removed,
        )?;
        removed.sort();
        let document_changed = before != next;
        let has_changes =
            document_changed || session.is_some() || owned != owned::OwnedRecord::default();
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_vec(&(
                "claudeOfficial",
                &path,
                &current,
                session_bytes,
                owned_bytes,
            ))
            .map_err(|_| "clients.errors.storage")?,
        );
        let warnings = official_warnings();
        let preview = ClaudeOfficialPreview {
            path: path.to_string_lossy().into(),
            revision: format!("{:x}", hasher.finalize()),
            removed_fields: removed,
            warnings,
            has_changes,
        };
        Ok(OfficialPlan {
            path,
            current,
            next,
            document_changed,
            preview,
        })
    }
}

fn clean_field(
    document: &mut Value,
    path: &[&str],
    removed: &mut Vec<String>,
) -> Result<(), String> {
    let path: Vec<String> = path.iter().map(|key| (*key).into()).collect();
    if field(document, &path).is_some() {
        set_field(document, &path, None)?;
        removed.push(path.join("."));
    }
    Ok(())
}

fn official_warnings() -> Vec<String> {
    // Higher-priority enterprise settings are not ours to remove. Read only
    // their shape; never return or log an administrator's credential values.
    let managed = takeover::managed_settings_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if managed.as_ref().is_some_and(managed_overrides_official) {
        vec!["clients.warnings.officialManagedSettings".into()]
    } else {
        Vec::new()
    }
}

fn managed_overrides_official(value: &Value) -> bool {
    value
        .get("apiKeyHelper")
        .is_some_and(|helper| !helper.is_null() && !helper.as_str().is_some_and(str::is_empty))
        || value
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| {
                ENV_FIELDS.iter().any(|key| {
                    env.get(*key).is_some_and(|setting| {
                        if key.starts_with("CLAUDE_CODE_USE_") {
                            return claude::cloud_selector_truthy(setting);
                        }
                        // Cloud gateway fields are inactive without the
                        // selectors checked above; don't warn about old,
                        // disabled cloud connections or skip-auth flags.
                        if key.starts_with("CLAUDE_CODE_SKIP_")
                            || [
                                "ANTHROPIC_BEDROCK_",
                                "ANTHROPIC_VERTEX_",
                                "ANTHROPIC_FOUNDRY_",
                                "ANTHROPIC_AWS_",
                            ]
                            .iter()
                            .any(|prefix| key.starts_with(prefix))
                        {
                            return false;
                        }
                        !setting.is_null() && !setting.as_str().is_some_and(str::is_empty)
                    })
                })
            })
}

#[cfg(test)]
#[path = "claude_official_tests.rs"]
mod tests;
