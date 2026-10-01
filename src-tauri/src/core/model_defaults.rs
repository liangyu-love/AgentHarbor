//! Model identity for routing: the same model behind different channel names
//! keeps one identity, and the original upstream ID is always preserved.
//!
//! Capability values no longer live here. Every effective value comes from the
//! shared resolver in `model_metadata`, which reads user overrides, provider
//! settings, real upstream declarations and the models.dev catalog.

/// Remove routing namespaces and channel annotations only for capability
/// lookup. The original model ID is retained when sending requests upstream.
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
    }
}
