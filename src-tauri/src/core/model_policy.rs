//! Compression policy: the single place that turns "auto compact at 90% of
//! the prompt budget" (or a fixed token count, or delegation to the client)
//! into a concrete threshold. Never invents values for unknown budgets.
use serde::{Deserialize, Serialize};

pub const DEFAULT_PERCENT: u8 = 90;
pub const MIN_PERCENT: u8 = 1;
pub const MAX_PERCENT: u8 = 99;
pub const MAX_TOKENS: u64 = 10_000_000;

/// One level of the model → provider → global inheritance chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum CompressionPolicy {
    /// Follow the next broader setting.
    #[default]
    Inherit,
    /// Percentage of the effective prompt budget, 1–99.
    Percent { percent: u8 },
    /// A user-fixed token threshold (the historical autoCompactLimit).
    Tokens { tokens: u64 },
    /// The client manages compaction itself; not the same as "disabled".
    ClientDefault,
}

impl CompressionPolicy {
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Percent { .. } => "percent",
            Self::Tokens { .. } => "tokens",
            Self::ClientDefault => "clientDefault",
        }
    }

    pub fn percent(percent: u8) -> Self {
        Self::Percent {
            percent: percent.clamp(MIN_PERCENT, MAX_PERCENT),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Inherit | Self::ClientDefault => Ok(()),
            Self::Percent { percent } => {
                if (MIN_PERCENT..=MAX_PERCENT).contains(percent) {
                    Ok(())
                } else {
                    Err("percent_out_of_range")
                }
            }
            Self::Tokens { tokens } => {
                if *tokens > 0 && *tokens <= MAX_TOKENS {
                    Ok(())
                } else {
                    Err("tokens_out_of_range")
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicySource {
    Model,
    Provider,
    Global,
}

impl PolicySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Provider => "provider",
            Self::Global => "global",
        }
    }
}

/// Model → provider → global, skipping `Inherit`.
pub fn resolve<'a>(
    model: Option<&'a CompressionPolicy>,
    provider: Option<&'a CompressionPolicy>,
    global: &'a CompressionPolicy,
) -> (&'a CompressionPolicy, PolicySource) {
    if let Some(policy) = model.filter(|policy| **policy != CompressionPolicy::Inherit) {
        return (policy, PolicySource::Model);
    }
    if let Some(policy) = provider.filter(|policy| **policy != CompressionPolicy::Inherit) {
        return (policy, PolicySource::Provider);
    }
    (global, PolicySource::Global)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionPlan {
    pub mode: String,
    pub percent: Option<u8>,
    pub tokens: Option<u64>,
    pub source: String,
    /// Effective prompt budget the threshold is derived from, when known.
    pub budget: Option<u64>,
    pub budget_source: String,
    /// The computed auto-compact threshold, when one should be applied.
    pub threshold: Option<u64>,
    /// computed | clientDefault | unknown | invalid
    pub state: String,
    pub note: Option<String>,
}

impl CompressionPlan {
    pub fn is_applied(&self) -> bool {
        self.threshold.is_some()
    }
}

/// Compute the threshold for one effective policy. `budget` is the effective
/// prompt budget (`limit.input` when present, otherwise `limit.context`), and
/// `budget_source` says where it came from for the UI.
pub fn plan(
    policy: &CompressionPolicy,
    source: PolicySource,
    budget: Option<u64>,
    budget_source: &str,
) -> CompressionPlan {
    let base = |mode: &str| CompressionPlan {
        mode: mode.into(),
        percent: None,
        tokens: None,
        source: source.as_str().into(),
        budget,
        budget_source: budget_source.into(),
        threshold: None,
        state: "unknown".into(),
        note: None,
    };
    if let Err(error) = policy.validate() {
        let mut plan = base(policy.mode());
        plan.state = "invalid".into();
        plan.note = Some(error.into());
        return plan;
    }
    match policy {
        CompressionPolicy::Inherit => {
            let mut plan = base("inherit");
            plan.state = "unresolved".into();
            plan.note = Some("inheritance was not resolved before planning".into());
            plan
        }
        CompressionPolicy::ClientDefault => {
            let mut plan = base("clientDefault");
            plan.state = "clientDefault".into();
            plan.note = Some("the client manages its own compaction".into());
            plan
        }
        CompressionPolicy::Percent { percent } => {
            let mut plan = base("percent");
            plan.percent = Some(*percent);
            let Some(budget) = budget else {
                plan.note = Some("prompt budget is unknown; threshold not derived".into());
                return plan;
            };
            let threshold = ((budget as u128) * (*percent as u128) / 100) as u64;
            if threshold == 0 || threshold >= budget {
                plan.state = "invalid".into();
                plan.note = Some("computed threshold falls outside the prompt budget".into());
                return plan;
            }
            plan.threshold = Some(threshold);
            plan.state = "computed".into();
            plan
        }
        CompressionPolicy::Tokens { tokens } => {
            let mut plan = base("tokens");
            plan.tokens = Some(*tokens);
            if let Some(budget) = budget {
                if *tokens >= budget {
                    plan.state = "invalid".into();
                    plan.note = Some("fixed threshold is not below the prompt budget".into());
                    return plan;
                }
            } else {
                plan.note = Some("prompt budget is unknown; fixed threshold is used as entered".into());
            }
            plan.threshold = Some(*tokens);
            plan.state = "computed".into();
            plan
        }
    }
}

/// Existing provider settings migrate to an explicit token policy; everything
/// else inherits the global default.
pub fn migrate_legacy_limit(limit: Option<u64>) -> CompressionPolicy {
    match limit.filter(|limit| *limit > 0) {
        Some(tokens) => CompressionPolicy::Tokens { tokens },
        None => CompressionPolicy::Inherit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ninety_percent_of_the_input_budget_is_the_default() {
        let policy = CompressionPolicy::percent(DEFAULT_PERCENT);
        let computed = plan(&policy, PolicySource::Global, Some(200_000), "catalog");
        assert_eq!(computed.threshold, Some(180_000));
        assert_eq!(computed.state, "computed");
        // Input budget beats the raw context: 272000 → 244800, never 360000.
        let computed = plan(&policy, PolicySource::Global, Some(272_000), "catalog");
        assert_eq!(computed.threshold, Some(244_800));
        let computed = plan(&policy, PolicySource::Global, Some(400_000), "catalog");
        assert_eq!(computed.threshold, Some(360_000));
        let computed = plan(
            &CompressionPolicy::percent(80),
            PolicySource::Model,
            Some(200_000),
            "catalog",
        );
        assert_eq!(computed.threshold, Some(160_000));
    }

    #[test]
    fn fixed_thresholds_win_but_never_silently_exceed_the_budget() {
        let tokens = CompressionPolicy::Tokens { tokens: 150_000 };
        let computed = plan(&tokens, PolicySource::Provider, Some(200_000), "catalog");
        assert_eq!(computed.threshold, Some(150_000));
        let computed = plan(&tokens, PolicySource::Provider, Some(100_000), "catalog");
        assert_eq!(computed.threshold, None);
        assert_eq!(computed.state, "invalid");
        // Without a known budget the user's number is still applied.
        let computed = plan(&tokens, PolicySource::Provider, None, "unknown");
        assert_eq!(computed.threshold, Some(150_000));
    }

    #[test]
    fn inheritance_resolves_from_model_to_provider_to_global() {
        let global = CompressionPolicy::percent(90);
        let provider = CompressionPolicy::Tokens { tokens: 100_000 };
        let model = CompressionPolicy::ClientDefault;
        assert_eq!(
            resolve(Some(&model), Some(&provider), &global).1,
            PolicySource::Model
        );
        assert_eq!(
            resolve(Some(&CompressionPolicy::Inherit), Some(&provider), &global).1,
            PolicySource::Provider
        );
        assert_eq!(resolve(None, None, &global).1, PolicySource::Global);
        assert_eq!(
            resolve(None, Some(&CompressionPolicy::Inherit), &global).1,
            PolicySource::Global
        );
    }

    #[test]
    fn client_default_and_invalid_percent_are_distinct_states() {
        let computed = plan(
            &CompressionPolicy::ClientDefault,
            PolicySource::Global,
            Some(200_000),
            "catalog",
        );
        assert_eq!(computed.state, "clientDefault");
        assert_eq!(computed.threshold, None);
        let computed = plan(
            &CompressionPolicy::Percent { percent: 0 },
            PolicySource::Global,
            Some(200_000),
            "catalog",
        );
        assert_eq!(computed.state, "invalid");
        let computed = plan(
            &CompressionPolicy::Tokens { tokens: 0 },
            PolicySource::Global,
            Some(200_000),
            "catalog",
        );
        assert_eq!(computed.state, "invalid");
    }

    #[test]
    fn legacy_limits_migrate_to_explicit_token_policies() {
        assert_eq!(
            migrate_legacy_limit(Some(120_000)),
            CompressionPolicy::Tokens { tokens: 120_000 }
        );
        assert_eq!(migrate_legacy_limit(None), CompressionPolicy::Inherit);
        assert_eq!(migrate_legacy_limit(Some(0)), CompressionPolicy::Inherit);
    }
}
