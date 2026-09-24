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
        let base = format!("http://127.0.0.1:{}/p/{}", store.router.port, provider.id);
        let mut edits = vec![
            edit(&["env", "ANTHROPIC_BASE_URL"], json!(base)),
            edit(
                &["env", "ANTHROPIC_AUTH_TOKEN"],
                json!(store.router.bearer_token),
            ),
            // Avoid an existing API key taking precedence over the gateway credential.
            edit(&["env", "ANTHROPIC_API_KEY"], json!("")),
            edit(&["model"], json!(request.model)),
        ];
        for name in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        ] {
            edits.push(edit(&["env", name], json!(request.model)));
        }
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
            let model = json!(request.model);
            if !allowed.contains(&model) {
                allowed.push(model);
            }
            edits.push(edit(&["availableModels"], json!(allowed)));
        }
        Ok(edits)
    }
}
