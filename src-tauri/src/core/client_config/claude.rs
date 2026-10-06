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

    /// Claude Code does not consume context/output limits in its settings;
    /// catalog-less models must stay injectable.
    fn needs_limits(&self) -> bool {
        false
    }

    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
        catalog: &super::models_dev::Catalog,
        official_models: &[Value],
        ctx: &mut EditContext<'_>,
    ) -> Result<Vec<Edit>, String> {
        let aggregate_mode = request.aggregate_target().is_some();
        let aggregate_models = if aggregate_mode {
            if request
                .options
                .model_selection
                .as_ref()
                .is_some_and(|selection| {
                    selection.available_targets.iter().any(|t| {
                        matches!(
                            t,
                            crate::core::model_targets::ModelTargetRef::Provider { .. }
                        )
                    })
                })
            {
                return Err("clients.errors.modelSelection".into());
            }
            aggregate_models::models(store, request, catalog)?
        } else {
            Vec::new()
        };
        let provider = if aggregate_mode {
            None
        } else {
            Some(selected_provider(store, request)?)
        };
        let (base, credential) = if let Some(provider) = provider {
            client_endpoint(store, provider, request, false)?
        } else {
            aggregate_models::endpoint(store, false)
        };
        let label_for = |model: &str| -> String {
            aggregate_models
                .iter()
                .find(|entry| entry.id == model)
                .map(|entry| entry.label.clone())
                .or_else(|| {
                    provider
                        .and_then(|p| p.models.iter().find(|entry| entry.id == model))
                        .map(|entry| entry.display_name.clone())
                        .filter(|name| !name.is_empty())
                })
                .unwrap_or_else(|| model.to_owned())
        };
        // Cloud-provider selectors override ANTHROPIC_BASE_URL, but an unset or
        // falsy one already lets the base URL win. Never create or rewrite these
        // keys: a truthy selector is the user's own Bedrock, Vertex or Foundry
        // setup, and it refuses the injection instead of being silently disabled
        // (direct configurations are persistent and not restored on exit).
        for name in [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ] {
            if document
                .get("env")
                .and_then(|env| env.get(name))
                .is_some_and(cloud_selector_truthy)
            {
                return Err("clients.errors.cloudMode".into());
            }
        }
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
        let mut available = provider.map(|p| request.models(p)).unwrap_or_else(|| {
            aggregate_models
                .iter()
                .map(|entry| entry.id.clone())
                .collect()
        });
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
                    && provider.is_some_and(|p| request.accepts_model(p, model))
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
        // so it works in native and router mode alike. It treats an unmarked model as
        // a 200K one and compacts early, while a mark on a model without the capacity
        // asks for a context the upstream must refuse — so the shared resolver decides:
        // only models whose known context reaches 1M are marked, and for models without
        // data the explicit opt-in decides. Context-window env vars are left to the user.
        let one_m = &request.options.claude_one_m;
        let known_small = |model: &str| {
            if let Some(entry) = aggregate_models.iter().find(|entry| entry.id == model) {
                return !entry.one_m;
            }
            let provider = provider.expect("ordinary model projection requires its provider");
            let official = super::super::router::matched_official_model(official_models, model)
                .filter(|_| super::super::router::official_endpoint_applies(provider));
            let projection = super::super::router::model_projection(
                provider,
                model,
                official,
                catalog,
                &store.global_compression(),
            )
            .1;
            !projection.context_is_fallback && projection.context_window < 1_000_000
        };
        let mut one_m_ignored = false;
        let mark = |model: &str, requested: bool, ignored: &mut bool| -> String {
            if requested && known_small(model) {
                *ignored = true;
                model.to_owned()
            } else {
                marked_model(model, requested)
            }
        };
        let startup_model = mark(&request.model, one_m.startup, &mut one_m_ignored);
        edits.push(edit(&["model"], json!(startup_model)));
        edits.push(edit(&["env", "ANTHROPIC_MODEL"], json!(startup_model)));
        let mut allowed_models = models.clone();
        let mut haiku_model = None;
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
                one_m.haiku,
            ),
            (
                "",
                "CLAUDE_CODE_SUBAGENT_MODEL",
                &mapping.subagent,
                one_m.subagent,
            ),
        ] {
            if role.is_empty() && explicit.is_empty() {
                // An unmapped subagent role must not force every subagent onto
                // the startup model: Claude Code's subagents ask for their own
                // tiers (haiku for small tasks), which one flat override would
                // silence. Drop the variable and let the role variables decide;
                // when all of them equal the startup model the two are
                // equivalent anyway.
                if document.get("env").and_then(|env| env.get(name)).is_some() {
                    edits.push(remove(&["env", name]));
                }
                continue;
            }
            let model = if !explicit.is_empty() {
                explicit
            } else {
                auto_role_model(&models, role, &request.model).unwrap_or(&request.model)
            };
            validate_model(model)?;
            if !models.iter().any(|id| id == model) {
                return Err("clients.errors.protocol".into());
            }
            let marked_value = mark(model, role_one_m, &mut one_m_ignored);
            edits.push(edit(&["env", name], json!(marked_value.clone())));
            if role == "haiku" {
                haiku_model = Some(marked_value);
            }
            // A previous gateway's display names must not describe a different model.
            if !role.is_empty() {
                let display_name = label_for(model);
                edits.push(edit(&["env", &format!("{name}_NAME")], json!(display_name)));
            }
            if !allowed_models.iter().any(|id| id == model) {
                allowed_models.push(model.to_owned());
            }
        }
        // Older Claude Code builds read the deprecated small-fast variable for
        // background tasks, so a leftover value from a previous gateway keeps
        // describing a different model. Overwrite it with the haiku role's
        // model, but never create the key: modern builds ignore it entirely.
        if let Some(haiku) = haiku_model {
            if document
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_SMALL_FAST_MODEL"))
                .is_some_and(|value| !value.is_null())
            {
                edits.push(edit(&["env", "ANTHROPIC_SMALL_FAST_MODEL"], json!(haiku)));
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
        // Older builds left their plain model/label rows behind when switching
        // relays. Without a journal and an ownership record nothing else
        // identifies them, so this narrow shape check is the one-time
        // migration; once a record exists it does the withdrawing. Restore
        // keeps the empty record so user rows, including minimal ones, are
        // never shape-guessed on subsequent applies.
        let mut stale_generated = Vec::new();
        if ctx.legacy_picker_cleanup {
            entries.retain(|entry| {
                let Some(object) = entry.as_object() else {
                    return true;
                };
                let Some(model) = object.get("model").and_then(Value::as_str) else {
                    return true;
                };
                let stale = object.len() == 2
                    && object.get("label").and_then(Value::as_str) == Some(model)
                    && !models.iter().any(|selected| selected == model);
                if stale {
                    stale_generated.push(model.to_owned());
                }
                !stale
            });
        }
        for model in &models {
            if entries
                .iter()
                .any(|entry| entry.get("model").and_then(Value::as_str) == Some(model.as_str()))
            {
                continue;
            }
            let label = label_for(model);
            entries.push(json!({"model": model, "label": label}));
        }
        edits.push(edit(&["modelPicker", "options"], json!(entries)));
        edits.push(edit(&["modelPicker", "replaceBuiltInOptions"], json!(true)));
        if let Some(allowed) = document.get("availableModels") {
            let mut allowed = allowed
                .as_array()
                .ok_or("clients.errors.invalidConfig")?
                .clone();
            allowed.retain(|value| {
                !value
                    .as_str()
                    .is_some_and(|id| stale_generated.iter().any(|stale| stale == id))
            });
            for model in allowed_models {
                let model = json!(model);
                if !allowed.contains(&model) {
                    allowed.push(model);
                }
            }
            edits.push(edit(&["availableModels"], json!(allowed)));
        }
        if one_m_ignored {
            ctx.warnings.push("clients.warnings.oneMIgnored".into());
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

/// Deterministic role auto-match: segment-boundary matches only, the startup
/// model wins when it fits the role, then the numerically newest version
/// (rolling aliases beat dated snapshots), then the provider's storage order.
/// Substring matching would drift with the provider's model order and could
/// pick an older alias of the same family.
fn auto_role_model<'a>(models: &'a [String], role: &str, startup: &'a str) -> Option<&'a str> {
    let matches = |id: &str| {
        id.split(['-', '_', '.'])
            .any(|segment| segment.eq_ignore_ascii_case(role))
    };
    if models.iter().any(|id| id == startup) && matches(startup) {
        return Some(startup);
    }
    let mut candidates: Vec<&str> = models
        .iter()
        .map(String::as_str)
        .filter(|id| matches(id))
        .collect();
    match candidates.len() {
        0 => None,
        1 => candidates.pop(),
        // min_by keeps the first of equals, so ties stay in storage order.
        _ => candidates.into_iter().min_by(|a, b| version_order(b, a)),
    }
}

/// Orders two model ids by version, `Ordering::Greater` meaning `a` is newer.
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let date_like = |id: &str| {
        id.split(['-', '_', '.'])
            .any(|segment| segment.len() >= 8 && segment.chars().all(|c| c.is_ascii_digit()))
    };
    match (date_like(a), date_like(b)) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let segments = |id: &str| -> Vec<String> {
        id.to_ascii_lowercase()
            .split(['-', '_', '.'])
            .map(str::to_owned)
            .collect()
    };
    let left = segments(a);
    let right = segments(b);
    for (a_segment, b_segment) in left.iter().zip(right.iter()) {
        if a_segment == b_segment {
            continue;
        }
        let numeric = |segment: &str| -> Option<u64> {
            (segment.len() < 8).then(|| segment.parse().ok()).flatten()
        };
        return match (numeric(a_segment), numeric(b_segment)) {
            (Some(a_number), Some(b_number)) => a_number.cmp(&b_number),
            _ => a_segment.cmp(b_segment),
        };
    }
    left.len().cmp(&right.len())
}

/// Mirrors Claude Code's own env-flag parsing: only 1/true/yes/on (case
/// insensitive, trimmed) enable a selector, so "0" and friends are falsy.
fn cloud_selector_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(enabled) => *enabled,
        Value::String(flag) => matches!(
            flag.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        _ => false,
    }
}
