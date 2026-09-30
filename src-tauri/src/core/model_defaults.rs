//! Model capabilities are keyed by model identity, independently of its channel.
//! Claude context retains the product's 1M default and manual opt-out.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDefaults {
    pub context_window: u64,
    pub max_output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

/// Remove routing namespaces and channel annotations only for capability lookup.
/// The original model ID is retained when sending requests upstream.
pub(super) fn model_key(model_id: &str) -> String {
    let id = model_id.trim().to_ascii_lowercase();
    let id = id.rsplit("::").next().unwrap_or(&id);
    let id = id.rsplit('/').next().unwrap_or(id);
    let id = id.rsplit("__").next().unwrap_or(id);
    let id = id.strip_prefix("anthropic.").unwrap_or(id);
    let id = id.split([':', '@', '[']).next().unwrap_or(id).trim();
    // Kiro identifies the channel, while numeric versions (including 5-5) and
    // unknown suffixes remain part of the model identity.
    id.strip_suffix("-kiro").unwrap_or(id).to_owned()
}

pub(super) fn channel_entry(model_id: &str) -> bool {
    model_id.contains("::") || model_id.starts_with("aimami_relay_")
}

/// Offline fallback for exact OpenAI IDs, from the unprefixed official Codex
/// cache observed on 2026-09-30. Live official metadata takes precedence.
/// Do not extrapolate these levels to unknown GPT variants.
pub(super) fn reasoning(model_id: &str) -> Option<(&'static [&'static str], &'static str)> {
    let full: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
    let standard: &[&str] = &["low", "medium", "high", "xhigh", "max"];
    match model_key(model_id).as_str() {
        "gpt-6.1-sol" | "gpt-6-astra" | "gpt-5.6-sol" => Some((full, "low")),
        "gpt-6-sol" | "gpt-5.6-terra" => Some((full, "medium")),
        "gpt-6-luna" | "gpt-5.6-luna" => Some((standard, "medium")),
        "gpt-5.5" => Some((&["low", "medium", "high", "xhigh"], "medium")),
        _ => None,
    }
}

/// Limits a signed-in Codex account has already synced for its own models.
///
/// `models_cache.json` is the most accurate local source for model metadata, so
/// it is consulted before the conservative guesses in [`for_model`]. A missing
/// or unreadable cache simply yields an empty catalog.
#[derive(Debug, Default)]
pub struct Catalog {
    entries: HashMap<String, CatalogEntry>,
}

impl Catalog {
    pub fn load(codex_home: &Path) -> Self {
        Self::from_models(
            &super::model_catalog::load(&codex_home.join("models_cache.json"), None).models,
        )
    }

    pub fn load_with_backup(codex_home: &Path, app_data: &Path) -> Self {
        Self::from_models(&super::model_catalog::official(codex_home, app_data).models)
    }

    pub(super) fn from_models(models: &[serde_json::Value]) -> Self {
        let mut entries = HashMap::new();
        for model in models {
            let Some(slug) = model.get("slug").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if channel_entry(slug) {
                continue;
            }
            let positive = |key: &str| {
                model
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .filter(|value| *value > 0)
            };
            let context_window = positive("context_window");
            let max_output_tokens = positive("max_output_tokens");
            // A positive context is required for output to be meaningful.
            if context_window.is_some() {
                entries.entry(model_key(slug)).or_insert(CatalogEntry {
                    context_window,
                    max_output_tokens: max_output_tokens
                        .filter(|output| Some(*output) <= context_window),
                });
            }
        }
        Self { entries }
    }

    pub fn get(&self, model_id: &str) -> Option<CatalogEntry> {
        self.entries.get(&model_key(model_id)).copied()
    }
}

pub fn for_model(model_id: &str) -> ModelDefaults {
    named_defaults(model_id).unwrap_or_else(|| defaults(128_000, 8_192))
}

pub(super) fn named_defaults(model_id: &str) -> Option<ModelDefaults> {
    let id = model_key(model_id);
    let compact: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if compact.contains("gemini") {
        return Some(defaults(1_048_576, 65_536));
    }
    if compact.contains("gpt41") {
        return Some(defaults(1_048_576, 32_768));
    }
    if compact.contains("gpt6") || compact.contains("gpt5") {
        return Some(defaults(400_000, 128_000));
    }
    if compact.contains("gpt4") || compact.contains("gpt3") || compact.contains("gpt") {
        return Some(defaults(128_000, 16_384));
    }
    if compact.contains("claude") {
        // All relay Claude models default to 1M, independently of output caps.
        let larger_output = [
            "opus5", "opus48", "opus47", "opus46", "sonnet5", "sonnet46", "fable5", "mythos5",
        ]
        .iter()
        .any(|family| compact.contains(family));
        return Some(defaults(
            1_000_000,
            if larger_output { 128_000 } else { 64_000 },
        ));
    }
    if compact.contains("minimax") {
        return Some(defaults(204_800, 16_384));
    }
    if compact.contains("qwen3coder") {
        return Some(defaults(262_144, 65_536));
    }
    if compact.contains("glm") {
        return Some(defaults(202_752, 32_768));
    }
    if compact.contains("qwen") {
        return Some(defaults(131_072, 32_768));
    }
    if compact.contains("deepseek") {
        return Some(defaults(128_000, 8_192));
    }
    if compact == "o3"
        || compact.starts_with("o3")
        || compact == "o4mini"
        || compact.starts_with("o4mini")
    {
        return Some(defaults(200_000, 100_000));
    }
    None
}

const fn defaults(context_window: u64, max_output_tokens: u64) -> ModelDefaults {
    ModelDefaults {
        context_window,
        max_output_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_identity_ignores_channel_names_and_annotations() {
        for id in [
            "gpt-6.1-sol",
            "OpenAI/GPT-6.1-SOL",
            "relay::gpt-6.1-sol",
            "channel__gpt-6.1-sol:free",
            "gpt-6.1-sol@fast",
        ] {
            assert_eq!(model_key(id), "gpt-6.1-sol");
            assert_eq!(for_model(id), for_model("gpt-6.1-sol"));
            assert_eq!(reasoning(id), reasoning("gpt-6.1-sol"));
        }
        assert_ne!(model_key("gpt-6.1-sol-preview"), model_key("gpt-6.1-sol"));
        assert_eq!(model_key("Claude-Opus-5-5-KIRO"), "claude-opus-5-5");
        assert_eq!(
            model_key("anthropic/claude-opus-5-5-kiro@fast"),
            "claude-opus-5-5"
        );
        assert_ne!(
            model_key("claude-opus-5-5-kiro"),
            model_key("claude-opus-5")
        );
        assert_ne!(
            model_key("claude-opus-5-5-preview"),
            model_key("claude-opus-5-5")
        );
        let catalog = Catalog::from_models(&[
            serde_json::json!({"slug":"gpt-6.1-sol","context_window":900000,"max_output_tokens":64000}),
            serde_json::json!({"slug":"relay::gpt-6.1-sol","context_window":32000,"max_output_tokens":1000}),
        ]);
        assert_eq!(
            catalog.get("OpenAI/GPT-6.1-SOL@fast"),
            Some(CatalogEntry {
                context_window: Some(900_000),
                max_output_tokens: Some(64_000),
            })
        );
    }

    #[test]
    fn known_families_and_unknown_models_have_editable_local_defaults() {
        assert_eq!(for_model("claude-sonnet-4"), defaults(1_000_000, 64_000));
        assert_eq!(for_model("claude-opus-4-8").context_window, 1_000_000);
        assert_eq!(for_model("claude-sonnet-5").context_window, 1_000_000);
        assert_eq!(for_model("claude-haiku-4-5"), defaults(1_000_000, 64_000));
        assert_eq!(for_model("Gemini-2.5-Pro").context_window, 1_048_576);
        assert_eq!(for_model("unknown-relay-model"), defaults(128_000, 8_192));
        assert_eq!(for_model("vendor/gpt-4o:free").context_window, 128_000);
        assert_eq!(
            for_model("proxy__Claude-Sonnet-4-fast").context_window,
            1_000_000
        );
    }

    #[test]
    fn catalog_prefers_the_signed_in_account_synced_limits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("models_cache.json"),
            r#"{"models":[
                {"slug":"gpt-6-luna","context_window":900000,"max_output_tokens":64000},
                {"slug":"no-context","max_output_tokens":4096},
                {"slug":"output-bigger-than-context","context_window":8192,"max_output_tokens":99999}
            ]}"#,
        )
        .unwrap();
        let catalog = Catalog::load(dir.path());

        assert_eq!(
            catalog.get("gpt-6-luna").unwrap().context_window,
            Some(900_000)
        );
        assert_eq!(
            catalog.get("gpt-6-luna").unwrap().max_output_tokens,
            Some(64_000)
        );
        // Without a context window the entry carries no usable limits.
        assert!(catalog.get("no-context").is_none());
        // An output cap above the context window is not used.
        let entry = catalog.get("output-bigger-than-context").unwrap();
        assert_eq!(entry.context_window, Some(8_192));
        assert_eq!(entry.max_output_tokens, None);
    }

    #[test]
    fn catalog_absent_or_malformed_yields_no_entries() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Catalog::load(dir.path()).get("anything").is_none());

        std::fs::write(dir.path().join("models_cache.json"), "not json").unwrap();
        assert!(Catalog::load(dir.path()).get("anything").is_none());
    }
}
