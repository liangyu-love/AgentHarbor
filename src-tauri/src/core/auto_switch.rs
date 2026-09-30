//! In-app automatic account switching. While AgentHarbor runs, a background loop
//! refreshes every account's quota and, once the active account drops to a
//! configured threshold, switches to the account with the most headroom.

use super::accounts::{CodexAccountListPayload, CodexAccountSummary};
use super::models::RateLimitWindow;
use super::repository::AutoSwitchConfig;

/// Quota refreshes spawn app-servers per account, so never poll faster than this.
pub const MIN_CHECK_INTERVAL_SECS: u64 = 60;
/// Gives the new account's quota time to show up before another switch is considered.
pub const SWITCH_COOLDOWN_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The active account is fine, or there is nothing to judge.
    Stay,
    Switch {
        from: Option<String>,
        to: String,
        to_label: String,
    },
    /// The active account is low but no other account has headroom.
    NoCandidate,
}

fn remaining(window: Option<&RateLimitWindow>) -> Option<i32> {
    window.map(|window| window.remaining_percent)
}

/// True when any known window is at or below its threshold.
fn is_low(account: &CodexAccountSummary, config: &AutoSwitchConfig) -> bool {
    remaining(account.primary_window.as_ref()).is_some_and(|value| value <= config.threshold_5h_percent)
        || remaining(account.secondary_window.as_ref())
            .is_some_and(|value| value <= config.threshold_weekly_percent)
}

/// The tightest remaining window, used to rank candidates. `None` when no quota is known.
fn headroom(account: &CodexAccountSummary) -> Option<i32> {
    [
        remaining(account.primary_window.as_ref()),
        remaining(account.secondary_window.as_ref()),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn is_candidate(account: &CodexAccountSummary, config: &AutoSwitchConfig) -> bool {
    // Only trust quotas fetched successfully in this round; stale or expired
    // accounts would just fail after the switch.
    account.usage_status == "healthy" && headroom(account).is_some() && !is_low(account, config)
}

pub fn decide(accounts: &CodexAccountListPayload, config: &AutoSwitchConfig) -> Decision {
    let active = accounts.items.iter().find(|item| item.is_active);
    let Some(active) = active else {
        return Decision::Stay;
    };
    // Judge the active account only on fresh data; an error keeps the current state.
    if active.usage_status != "healthy" || !is_low(active, config) {
        return Decision::Stay;
    }
    let best = accounts
        .items
        .iter()
        .filter(|item| !item.is_active && is_candidate(item, config))
        .max_by_key(|item| headroom(item).unwrap_or_default());
    match best {
        Some(target) => Decision::Switch {
            from: Some(active.account_key.clone()),
            to: target.account_key.clone(),
            to_label: target.display_name.clone(),
        },
        None => Decision::NoCandidate,
    }
}

/// Loop state that must survive between rounds but not restarts.
#[derive(Debug, Default)]
pub struct LoopState {
    last_switch_at: Option<i64>,
    /// The active account already reported as having no candidate, so the user is
    /// notified once per exhausted account rather than every round.
    notified_exhausted: Option<String>,
}

impl LoopState {
    pub fn in_cooldown(&self, now: i64) -> bool {
        self.last_switch_at
            .is_some_and(|at| now - at < SWITCH_COOLDOWN_SECS)
    }

    pub fn record_switch(&mut self, now: i64) {
        self.last_switch_at = Some(now);
        self.notified_exhausted = None;
    }

    /// Returns true the first time a given active account is found without a candidate.
    pub fn should_notify_exhausted(&mut self, active: Option<&str>) -> bool {
        let active = active.map(str::to_owned);
        if self.notified_exhausted == active {
            return false;
        }
        self.notified_exhausted = active;
        true
    }

    pub fn clear_exhausted(&mut self) {
        self.notified_exhausted = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(remaining_percent: i32) -> Option<RateLimitWindow> {
        Some(RateLimitWindow {
            used_percent: f64::from(100 - remaining_percent),
            remaining_percent,
            window_minutes: None,
            resets_at: None,
        })
    }

    fn account(key: &str, active: bool, five_hour: i32, weekly: i32) -> CodexAccountSummary {
        CodexAccountSummary {
            account_key: key.into(),
            email: format!("{key}@example.com"),
            display_name: key.into(),
            plan: "plus".into(),
            auth_mode: "chatgpt".into(),
            is_active: active,
            created_at: 0,
            last_used_at: None,
            last_usage_at: None,
            primary_window: window(five_hour),
            secondary_window: window(weekly),
            reset_credits: None,
            usage_status: "healthy".into(),
            usage_error: None,
        }
    }

    fn list(items: Vec<CodexAccountSummary>) -> CodexAccountListPayload {
        CodexAccountListPayload {
            active_account_key: items
                .iter()
                .find(|item| item.is_active)
                .map(|item| item.account_key.clone()),
            refreshed_at: 0,
            items,
        }
    }

    fn config() -> AutoSwitchConfig {
        AutoSwitchConfig {
            enabled: true,
            threshold_5h_percent: 15,
            threshold_weekly_percent: 10,
        }
    }

    #[test]
    fn stays_while_the_active_account_has_quota() {
        let accounts = list(vec![account("a", true, 16, 11), account("b", false, 90, 90)]);
        assert_eq!(decide(&accounts, &config()), Decision::Stay);
    }

    #[test]
    fn switches_to_the_account_with_the_most_headroom() {
        let accounts = list(vec![
            account("a", true, 15, 80),
            // High 5h but nearly out of weekly quota: its tightest window is 20.
            account("b", false, 95, 20),
            account("c", false, 60, 70),
        ]);
        assert_eq!(
            decide(&accounts, &config()),
            Decision::Switch {
                from: Some("a".into()),
                to: "c".into(),
                to_label: "c".into()
            }
        );
    }

    #[test]
    fn weekly_threshold_alone_triggers_a_switch() {
        let accounts = list(vec![account("a", true, 90, 10), account("b", false, 50, 50)]);
        assert!(matches!(decide(&accounts, &config()), Decision::Switch { to, .. } if to == "b"));
    }

    #[test]
    fn skips_low_unhealthy_and_unknown_candidates() {
        let mut expired = account("expired", false, 90, 90);
        expired.usage_status = "expired".into();
        let mut unknown = account("unknown", false, 0, 0);
        unknown.primary_window = None;
        unknown.secondary_window = None;
        let accounts = list(vec![
            account("a", true, 5, 50),
            account("low", false, 12, 90),
            expired,
            unknown,
        ]);
        assert_eq!(decide(&accounts, &config()), Decision::NoCandidate);
    }

    #[test]
    fn a_failed_refresh_of_the_active_account_never_switches() {
        let mut active = account("a", true, 0, 0);
        active.usage_status = "error".into();
        let accounts = list(vec![active, account("b", false, 90, 90)]);
        assert_eq!(decide(&accounts, &config()), Decision::Stay);
    }

    #[test]
    fn no_active_account_stays() {
        let accounts = list(vec![account("a", false, 0, 0), account("b", false, 90, 90)]);
        assert_eq!(decide(&accounts, &config()), Decision::Stay);
    }

    #[test]
    fn cooldown_and_exhausted_notice_are_tracked() {
        let mut state = LoopState::default();
        assert!(!state.in_cooldown(1_000));
        state.record_switch(1_000);
        assert!(state.in_cooldown(1_000 + SWITCH_COOLDOWN_SECS - 1));
        assert!(!state.in_cooldown(1_000 + SWITCH_COOLDOWN_SECS));

        assert!(state.should_notify_exhausted(Some("a")));
        assert!(!state.should_notify_exhausted(Some("a")));
        assert!(state.should_notify_exhausted(Some("b")));
        state.clear_exhausted();
        assert!(state.should_notify_exhausted(Some("b")));
    }
}
