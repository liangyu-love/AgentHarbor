//! Conservative local metadata used when a relay's `/v1/models` response only
//! returns ids. Every value is editable in the provider model editor.

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
        let Ok(bytes) = std::fs::read(codex_home.join("models_cache.json")) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Self::default();
        };
        let Some(models) = value.get("models").and_then(serde_json::Value::as_array) else {
            return Self::default();
        };
        let mut entries = HashMap::new();
        for model in models {
            let Some(slug) = model.get("slug").and_then(serde_json::Value::as_str) else {
                continue;
            };
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
                entries.insert(
                    slug.to_owned(),
                    CatalogEntry {
                        context_window,
                        max_output_tokens: max_output_tokens
                            .filter(|output| Some(*output) <= context_window),
                    },
                );
            }
        }
        Self { entries }
    }

    pub fn get(&self, model_id: &str) -> Option<CatalogEntry> {
        self.entries.get(model_id).copied()
    }
}

pub fn for_model(model_id: &str) -> ModelDefaults {
    // Relays frequently decorate model ids with a provider prefix/suffix
    // (for example `relay/gpt-4o:free` or `claude-sonnet-4@fast`).  Matching
    // against a compact form keeps the local catalog useful without needing
    // a per-relay alias table.
    let id = model_id.trim().to_ascii_lowercase();
    let compact: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if compact.contains("gemini") {
        return defaults(1_048_576, 65_536);
    }
    if compact.contains("gpt41") {
        return defaults(1_048_576, 32_768);
    }
    if compact.contains("gpt6") || compact.contains("gpt5") {
        return defaults(400_000, 128_000);
    }
    if compact.contains("gpt4") || compact.contains("gpt3") || compact.contains("gpt") {
        return defaults(128_000, 16_384);
    }
    if compact.contains("claude") {
        // Current Opus/Sonnet/Fable/Mythos generations have a 1M window.
        // Older models and Haiku still need their smaller documented limit.
        let one_m = [
            "opus5", "opus48", "opus47", "opus46", "sonnet5", "sonnet46", "fable5", "mythos5",
        ]
        .iter()
        .any(|family| compact.contains(family));
        if one_m {
            return defaults(1_000_000, 128_000);
        }
        return defaults(200_000, 64_000);
    }
    if compact.contains("minimax") {
        return defaults(204_800, 16_384);
    }
    if compact.contains("qwen3coder") {
        return defaults(262_144, 65_536);
    }
    if compact.contains("glm") {
        return defaults(202_752, 32_768);
    }
    if compact.contains("qwen") {
        return defaults(131_072, 32_768);
    }
    if compact.contains("deepseek") {
        return defaults(128_000, 8_192);
    }
    if compact == "o3"
        || compact.starts_with("o3")
        || compact == "o4mini"
        || compact.starts_with("o4mini")
    {
        return defaults(200_000, 100_000);
    }
    // An unknown model's actual limit is unknown. Use a conservative editable fallback.
    defaults(128_000, 8_192)
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
    fn known_families_and_unknown_models_have_editable_local_defaults() {
        assert_eq!(for_model("claude-sonnet-4").context_window, 200_000);
        assert_eq!(for_model("claude-opus-4-8").context_window, 1_000_000);
        assert_eq!(for_model("claude-sonnet-5").context_window, 1_000_000);
        assert_eq!(for_model("claude-haiku-4-5").context_window, 200_000);
        assert_eq!(for_model("Gemini-2.5-Pro").context_window, 1_048_576);
        assert_eq!(for_model("unknown-relay-model"), defaults(128_000, 8_192));
        assert_eq!(for_model("vendor/gpt-4o:free").context_window, 128_000);
        assert_eq!(
            for_model("proxy__Claude-Sonnet-4-fast").context_window,
            200_000
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
