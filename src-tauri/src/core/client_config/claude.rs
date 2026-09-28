use super::*;

pub(super) struct ClaudeCode;

impl ClientAdapter for ClaudeCode {
    fn id(&self) -> TargetId {
        TargetId::ClaudeCode
    }
    fn name(&self) -> &'static str {
        "Claude Code"
    }
    fn protocols(&self) -> &'static [ModelProtocol] {
        &[ModelProtocol::AnthropicMessages]
    }
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path {
        &paths.claude
    }

    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
    ) -> Result<Vec<Edit>, String> {
        let provider = selected_provider(store, request)?;
        let (base, credential) = if request.needs_router() {
            (
                format!("http://127.0.0.1:{}/p/{}", store.router.port, provider.id),
                &store.router.bearer_token,
            )
        } else {
            (native_base(provider, false)?, &provider.api_key)
        };
        let (auth, other_auth) = match request.options.auth_field {
            ClaudeAuthField::AuthToken => ("ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"),
            ClaudeAuthField::ApiKey => ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"),
        };
        let mut edits = vec![
            edit(&["env", "ANTHROPIC_BASE_URL"], json!(base)),
            edit(&["env", auth], json!(credential)),
            remove(&["env", other_auth]),
        ];
        let mapping = &request.options.claude_models;
        let mut available = request.models(provider);
        // Older requests can explicitly name models before a catalog has been fetched.
        // Keep that behavior when no selection list was supplied; explicit lists stay strict.
        if request.options.claude_model_ids.is_none() {
            for model in [
                &request.model,
                &mapping.sonnet,
                &mapping.opus,
                &mapping.fable,
                &mapping.haiku,
                &mapping.subagent,
            ] {
                if !model.is_empty()
                    && request.accepts_model(provider, model)
                    && !available.contains(model)
                {
                    validate_model(model)?;
                    available.push(model.clone());
                }
            }
        }
        let models = if let Some(selected) = &request.options.claude_model_ids {
            let mut models = Vec::new();
            for model in selected {
                validate_model(model)?;
                if !available.contains(model) {
                    return Err("clients.errors.modelSelection".into());
                }
                if !models.contains(model) {
                    models.push(model.clone());
                }
            }
            models
        } else {
            available
        };
        if !models.contains(&request.model) {
            return Err("clients.errors.modelSelection".into());
        }
        // Claude Code reads the [1m] suffix per variable and strips it before sending,
        // so it works in native and router mode alike. Context-window env vars are left
        // to the user: they are ignored for [1m] IDs and only affect unrecognized models.
        let one_m = &request.options.claude_one_m;
        let startup_model = marked_model(&request.model, one_m.startup);
        edits.push(edit(&["model"], json!(startup_model)));
        edits.push(edit(&["env", "ANTHROPIC_MODEL"], json!(startup_model)));
        let mut allowed_models = models.clone();
        for (role, name, explicit, role_one_m) in [
            (
                "sonnet",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                &mapping.sonnet,
                one_m.sonnet,
            ),
            (
                "opus",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                &mapping.opus,
                one_m.opus,
            ),
            (
                "fable",
                "ANTHROPIC_DEFAULT_FABLE_MODEL",
                &mapping.fable,
                one_m.fable,
            ),
            (
                "haiku",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                &mapping.haiku,
                false,
            ),
            (
                "",
                "CLAUDE_CODE_SUBAGENT_MODEL",
                &mapping.subagent,
                one_m.subagent,
            ),
        ] {
            let model = if !explicit.is_empty() {
                explicit
            } else {
                models
                    .iter()
                    .find(|id| !role.is_empty() && id.to_ascii_lowercase().contains(role))
                    .unwrap_or(&request.model)
            };
            validate_model(model)?;
            if !models.contains(model) {
                return Err("clients.errors.protocol".into());
            }
            edits.push(edit(&["env", name], json!(marked_model(model, role_one_m))));
            // A previous gateway's display names must not describe a different model.
            if !role.is_empty() {
                let display_name = provider
                    .models
                    .iter()
                    .find(|entry| entry.id == *model)
                    .map(|entry| entry.display_name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(model);
                edits.push(edit(&["env", &format!("{name}_NAME")], json!(display_name)));
            }
            if !allowed_models.contains(model) {
                allowed_models.push(model.clone());
            }
        }
        // Import every selected version, including multiple versions of one family.
        // Preserve manually configured rows and their labels/capability metadata.
        let picker = document.get("modelPicker");
        if picker.is_some_and(|value| !value.is_object()) {
            return Err("clients.errors.invalidConfig".into());
        }
        let mut entries = match picker.and_then(|value| value.get("options")) {
            Some(value) => value
                .as_array()
                .ok_or("clients.errors.invalidConfig")?
                .clone(),
            None => Vec::new(),
        };
        for model in &models {
            if entries
                .iter()
                .any(|entry| entry.get("model").and_then(Value::as_str) == Some(model.as_str()))
            {
                continue;
            }
            let label = provider
                .models
                .iter()
                .find(|entry| entry.id == *model)
                .map(|entry| entry.display_name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(model);
            entries.push(json!({"model": model, "label": label}));
        }
        edits.push(edit(&["modelPicker", "options"], json!(entries)));
        edits.push(edit(&["modelPicker", "replaceBuiltInOptions"], json!(true)));
        // Cloud-provider selectors override ANTHROPIC_BASE_URL; save and restore them as owned fields.
        for name in [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ] {
            edits.push(edit(&["env", name], json!("0")));
        }
        if let Some(allowed) = document.get("availableModels") {
            let mut allowed = allowed
                .as_array()
                .ok_or("clients.errors.invalidConfig")?
                .clone();
            for model in allowed_models {
                let model = json!(model);
                if !allowed.contains(&model) {
                    allowed.push(model);
                }
            }
            edits.push(edit(&["availableModels"], json!(allowed)));
        }
        Ok(edits)
    }
}

fn marked_model(model: &str, one_m: bool) -> String {
    if one_m {
        format!("{model}[1m]")
    } else {
        model.to_owned()
    }
}
