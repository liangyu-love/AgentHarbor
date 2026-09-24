//! Conservative local metadata used when a relay's `/v1/models` response only
//! returns ids. Every value is editable in the provider model editor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDefaults {
    pub context_window: u64,
    pub max_output_tokens: u64,
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
        return defaults(1_047_576, 32_768);
    }
    if compact.contains("gpt6") || compact.contains("gpt5") {
        return defaults(400_000, 128_000);
    }
    if compact.contains("gpt4") || compact.contains("gpt3") || compact.contains("gpt") {
        return defaults(128_000, 16_384);
    }
    if compact.contains("claude") {
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
    // Unknown relay models are commonly modern long-context models.  Keep a
    // generous context fallback while preserving a conservative output cap.
    defaults(1_048_576, 8_192)
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
        assert_eq!(for_model("Gemini-2.5-Pro").context_window, 1_048_576);
        assert_eq!(for_model("unknown-relay-model"), defaults(1_048_576, 8_192));
        assert_eq!(for_model("vendor/gpt-4o:free").context_window, 128_000);
        assert_eq!(
            for_model("proxy__Claude-Sonnet-4-fast").context_window,
            200_000
        );
    }
}
