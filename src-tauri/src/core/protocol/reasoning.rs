//! Map reasoning intent to Messages parameters, independently of history conversion.
//! Budgets are application policy, not a promise about a model's actual reasoning.

use serde_json::{json, Map, Value};

pub(super) fn anthropic(out: &mut Map<String, Value>, model: &str, effort: &str) -> bool {
    let budget: u64 = match effort {
        "minimal" => 1024,
        "low" => 4096,
        "medium" => 10_000,
        "high" => 24_000,
        "xhigh" | "max" => 32_000,
        // In particular, `none` must not turn thinking on.
        _ => return false,
    };
    if adaptive(model) {
        out.insert("thinking".into(), json!({"type":"adaptive"}));
        out.insert(
            "output_config".into(),
            json!({"effort":if effort == "minimal" { "low" } else { effort }}),
        );
    } else {
        let cap = out.get("max_tokens").and_then(Value::as_u64).unwrap_or(0);
        let forced_tool = matches!(
            out.get("tool_choice")
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str),
            Some("any" | "tool")
        );
        if cap <= 1024 || forced_tool {
            return false;
        }
        // Honour the caller's output cap. Reserve answer space where possible;
        // never send a budget below the protocol minimum or >= max_tokens.
        let budget = budget.min(cap.saturating_sub(1024).max(1024));
        out.insert(
            "thinking".into(),
            json!({"type":"enabled","budget_tokens":budget}),
        );
    }
    true
}

fn adaptive(model: &str) -> bool {
    let identity = super::super::model_defaults::model_key(model);
    let Some(name) = identity.strip_prefix("claude-") else {
        return false;
    };
    // Known adaptive families only: don't extrapolate to future generations,
    // Haiku, or arbitrary providers that also speak the Messages protocol.
    [
        "opus-4-6",
        "opus-4-7",
        "opus-4-8",
        "opus-5",
        "sonnet-4-6",
        "sonnet-5",
        "fable-5",
        "mythos-5",
    ]
    .iter()
    .any(|family| {
        name.strip_prefix(family)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(['-', '.', ':', '@', '[']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_names_do_not_match_older_or_unrelated_models() {
        for name in [
            "claude-opus-4-6",
            "vendor/claude-sonnet-4-6-20260217",
            "anthropic.claude-opus-4-7-v1:0",
            "claude-fable-5-1",
            "claude-mythos-5",
            "claude-opus-5-5-kiro",
            "channel__Claude-Opus-5-5-kiro:free",
        ] {
            assert!(adaptive(name), "{name}");
        }
        for name in [
            "claude-opus-4-5",
            "claude-haiku-4-5",
            "claude-opus-4-60",
            "notclaude-opus-4-6",
            "claude-sonnet-6",
            "glm-5",
        ] {
            assert!(!adaptive(name), "{name}");
        }
    }

    #[test]
    fn budget_respects_cap_and_forced_tools_without_changing_it() {
        for cap in [0, 1024, 1025, 2048, 8192, 32000] {
            let mut out = Map::from_iter([("max_tokens".into(), json!(cap))]);
            assert_eq!(anthropic(&mut out, "claude-haiku-4-5", "high"), cap > 1024);
            assert_eq!(out["max_tokens"], cap);
            if cap > 1024 {
                let budget = out["thinking"]["budget_tokens"].as_u64().unwrap();
                assert!((1024..cap).contains(&budget));
            }
        }
        for choice in ["tool", "any"] {
            let mut out = json!({"max_tokens":32000,"tool_choice":{"type":choice}})
                .as_object()
                .unwrap()
                .clone();
            assert!(!anthropic(&mut out, "claude-haiku-4-5", "high"));
            assert!(out.get("thinking").is_none());
        }
    }

    #[test]
    fn unknown_effort_does_not_enable_thinking() {
        let mut out = Map::new();
        assert!(!anthropic(&mut out, "claude-opus-4-6", "future-effort"));
        assert!(out.is_empty());
    }
}
