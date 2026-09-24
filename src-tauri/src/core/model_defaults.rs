//! Conservative local metadata used when a relay's `/v1/models` response only
//! returns ids. Every value is editable in the provider model editor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDefaults {
    pub context_window: u64,
    pub max_output_tokens: u64,
}

pub fn for_model(model_id: &str) -> ModelDefaults {
    let id = model_id.trim().to_ascii_lowercase();
    if id.contains("gemini") {
        return defaults(1_048_576, 65_536);
    }
    if id.contains("gpt-4.1") {
        return defaults(1_047_576, 32_768);
    }
    if id.contains("gpt-6") || id.contains("gpt-5") {
        return defaults(400_000, 128_000);
    }
    if id.contains("claude") {
        return defaults(200_000, 64_000);
    }
    if id.contains("minimax") {
        return defaults(204_800, 16_384);
    }
    if id.contains("qwen3-coder") {
        return defaults(262_144, 65_536);
    }
    if id.contains("glm-") || id.starts_with("glm") {
        return defaults(202_752, 32_768);
    }
    if id.contains("qwen") {
        return defaults(131_072, 32_768);
    }
    if id.contains("deepseek") {
        return defaults(128_000, 8_192);
    }
    if id == "o3" || id.starts_with("o3-") || id == "o4-mini" || id.starts_with("o4-mini-") {
        return defaults(200_000, 100_000);
    }
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
        assert_eq!(for_model("Gemini-2.5-Pro").context_window, 1_048_576);
        assert_eq!(for_model("unknown-relay-model"), defaults(128_000, 8_192));
    }
}
