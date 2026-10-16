//! Daily check-ins (签到): WorkBuddy, Trae CN, MiniMax Code and Qoder give
//! an account a few credits a day for pressing a button in their app. While
//! a source's auto check-in is on, AgentHarbor presses it for each of its
//! accounts once a Beijing day, as the vendor's own app does: the status
//! first, a claim only when today's isn't in yet.
//!
//! Ported from magpie's `checkin.go` (MIT, see LICENSES/THIRD_PARTY.md).
//! What came of a check-in is kept on the account (no token): an answer —
//! claimed, already in, not eligible, no event, the vendor's own app only —
//! holds for the rest of the Beijing day; a request that got no answer is
//! tried again after `RETRY`, or, when it never reached the vendor (a
//! machine just woken, its network not back yet), a minute later and twice
//! as long each time it still doesn't. Qoder's campaigns can roll over
//! before the Beijing day ends, so its accounts are asked again every
//! `EVERY` and when a claimed campaign's window closes.
use super::{store, SourceKind};
use crate::platform::paths::CodexPaths;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// How often the accounts are looked at.
pub const EVERY: Duration = Duration::from_secs(30 * 60);
/// How long a check-in that got no answer waits to be tried again.
const RETRY: Duration = Duration::from_secs(30 * 60);
/// How long one that never reached the vendor waits, doubling to `RETRY`.
const SOON: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Claimed by AgentHarbor.
    Claimed,
    /// In already: pressed in the vendor's app, or before.
    Done,
    /// The account can't take part.
    Ineligible,
    /// No event running, or it has ended.
    Inactive,
    /// No answer; tried again later that day.
    #[default]
    Failed,
    /// The vendor asked for a captcha: check in in its own app.
    Captcha,
    /// The vendor pays the check-in only to its own app.
    OwnApp,
}

/// What came of an account's check-in on `day` (Beijing): the credits it
/// gave and the streak it made, or why none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Checkin {
    pub day: String,
    /// Unix seconds.
    pub at: i64,
    pub outcome: Outcome,
    pub credit: f64,
    pub streak: u32,
    pub message: Option<String>,
    /// A campaign's end (unix seconds), when a claimed one is asked again
    /// even if the Beijing day hasn't changed.
    pub valid_until: Option<i64>,
    /// A failure that never reached the vendor, and how many before it the
    /// same day did the same.
    pub offline: bool,
    pub tries: u32,
}

impl Checkin {
    pub fn outcome(outcome: Outcome) -> Self {
        Checkin { outcome, ..Default::default() }
    }

    pub fn claimed(credit: f64, streak: u32) -> Self {
        Checkin { outcome: Outcome::Claimed, credit, streak, ..Default::default() }
    }

    pub fn done(credit: f64, streak: u32) -> Self {
        Checkin { outcome: Outcome::Done, credit, streak, ..Default::default() }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        let message = message.into();
        self.message = (!message.trim().is_empty()).then_some(message);
        self
    }

    /// A failure to try again later; `offline` when it got no reply at all.
    pub fn failed(message: impl Into<String>, offline: bool) -> Self {
        Checkin { offline, ..Checkin::outcome(Outcome::Failed).with_message(message) }
    }

    /// The day's credits are in, claimed now or before.
    pub fn checked(&self) -> bool {
        matches!(self.outcome, Outcome::Claimed | Outcome::Done)
    }

    /// How long a failure waits to be tried again.
    fn retry_after(&self) -> Duration {
        if !self.offline {
            return RETRY;
        }
        RETRY.min(SOON * (1u32 << self.tries.min(5)))
    }

    /// It answers for `day`: that day's, and not a failure due another try
    /// (at `now`; `soon`, at once).
    fn settled(&self, day: &str, now: i64, soon: bool) -> bool {
        if self.day != day {
            return false;
        }
        self.outcome != Outcome::Failed || !soon && now - self.at < self.retry_after().as_secs() as i64
    }
}

/// The Beijing day ("2006-01-02") a unix time falls on: the day the
/// vendors' events count in, UTC+8 all year.
pub fn day_of(unix: i64) -> String {
    let beijing = chrono::FixedOffset::east_opt(8 * 3600).expect("a fixed offset");
    chrono::DateTime::from_timestamp(unix, 0).unwrap_or_default().with_timezone(&beijing).format("%Y-%m-%d").to_string()
}

pub fn today() -> String {
    day_of(chrono::Utc::now().timestamp())
}

impl SourceKind {
    /// The source has a daily check-in.
    pub fn checks_in(self) -> bool {
        matches!(self, SourceKind::Workbuddy | SourceKind::MinimaxCode | SourceKind::MinimaxCodeGlobal | SourceKind::Qoder | SourceKind::QoderCn | SourceKind::TraeCn)
    }

    /// Its campaigns can change before the Beijing day ends (Qoder's).
    fn rechecks(self) -> bool {
        matches!(self, SourceKind::Qoder | SourceKind::QoderCn)
    }
}

/// Keeps the loop's run and a press of "check in now" from asking for one
/// account at once.
static CHECKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Check in each account that isn't in for today yet: those of `only`, or
/// of every source whose auto check-in is on. `soon` (a press) tries a
/// failure again without waiting. Says how many it asked.
pub async fn run(dir: &Path, client: &Client, only: Option<SourceKind>, soon: bool) -> Result<usize, String> {
    let _checking = CHECKING.lock().await;
    let store = store::load(dir)?;
    let now = chrono::Utc::now().timestamp();
    let day = day_of(now);
    let mut asked = 0;
    for account in &store.accounts {
        let kind = account.kind;
        let chosen = match only {
            Some(only) => only == kind,
            None => store.checkin_on.contains(&kind),
        };
        if !kind.checks_in() || !chosen || account.checkin_off || account.lapsed.is_some() {
            continue;
        }
        let prev = account.checkin.as_ref();
        let mut settled = prev.is_some_and(|prev| prev.settled(&day, now, soon));
        if kind.rechecks() {
            if let Some(prev) = prev {
                settled = settled && !soon && now - prev.at < EVERY.as_secs() as i64;
                if prev.checked() {
                    settled = settled && prev.valid_until.is_none_or(|until| now < until);
                }
            }
        }
        if settled {
            continue;
        }
        let mut result = check_in(dir, client, account).await;
        result.day = day.clone();
        result.at = now;
        if result.offline {
            if let Some(prev) = prev.filter(|prev| prev.day == day && prev.offline) {
                result.tries = prev.tries + 1;
            }
        }
        log_result(kind, &account.name, &result);
        asked += 1;
        let id = account.id.clone();
        store::mutate(dir, |store| {
            if let Some(found) = store.accounts.iter_mut().find(|found| found.id == id) {
                found.checkin = Some(result);
            }
            Ok(())
        })?;
    }
    Ok(asked)
}

fn log_result(kind: SourceKind, name: &str, result: &Checkin) {
    let source = kind.display_name();
    match result.outcome {
        Outcome::Claimed if result.streak > 0 => log::info!("{source} check-in: {name} +{}, a {}-day streak", result.credit, result.streak),
        Outcome::Claimed => log::info!("{source} check-in: {name} +{}", result.credit),
        Outcome::Failed => log::warn!("{source} check-in: {name}: {}", result.message.as_deref().unwrap_or("failed")),
        outcome => log::info!("{source} check-in: {name}: {outcome:?}"),
    }
}

/// One account's check-in, with fresh credentials.
async fn check_in(dir: &Path, client: &Client, account: &store::Account) -> Checkin {
    match &account.credentials {
        store::Credentials::WorkBuddy(creds) => match super::fresh_workbuddy(dir, client, account, creds).await {
            Ok(creds) => super::workbuddy::check_in(client, &super::workbuddy::site(account.kind), &creds).await,
            Err((_, message)) => Checkin::failed(message, false),
        },
        store::Credentials::MiniMax(creds) => match super::fresh_minimax(dir, client, account, creds).await {
            Ok(creds) => super::minimax::check_in(client, &super::minimax::site(account.kind), &creds).await,
            Err((_, message)) => Checkin::failed(message, false),
        },
        store::Credentials::Trae(creds) => match super::fresh_trae(dir, client, account, creds).await {
            Ok(creds) => super::trae::check_in(client, &super::trae::site(account.kind), &creds).await,
            Err((_, message)) => Checkin::failed(message, false),
        },
        store::Credentials::Qoder(creds) => match super::fresh_qoder(dir, client, account, creds).await {
            Ok(creds) => match super::qoder::site_of(account.kind, &creds) {
                Ok(site) => super::qoder::check_in(client, &site, &creds).await,
                Err(error) => Checkin::failed(error, false),
            },
            Err((_, message)) => Checkin::failed(message, false),
        },
        _ => Checkin::outcome(Outcome::Inactive),
    }
}

/// Turn a source's auto check-in on or off.
pub fn set_auto(dir: &Path, kind: SourceKind, on: bool) -> Result<(), String> {
    if !kind.checks_in() {
        return Err("accountSources.errors.notFound".into());
    }
    store::mutate(dir, |store| {
        if on {
            store.checkin_on.insert(kind);
        } else {
            store.checkin_on.remove(&kind);
        }
        Ok(())
    })
}

/// Leave an account out of its source's check-in, or bring it back.
pub fn set_account_off(dir: &Path, id: &str, off: bool) -> Result<(), String> {
    store::mutate(dir, |store| {
        let account = store.accounts.iter_mut().find(|account| account.id == id).ok_or("accountSources.errors.notFound")?;
        account.checkin_off = off;
        Ok(())
    })
}

/// When the loop last looked at the accounts, and when a failure is due
/// to be tried again if before its next look.
#[derive(Default)]
struct Loop {
    last: Option<i64>,
    again: Option<i64>,
}

impl Loop {
    /// It is time to look: the first tick, `EVERY` after the last look, as
    /// the Beijing day turns, or as a failure comes due again.
    fn due(&self, now: i64) -> bool {
        let Some(last) = self.last else { return true };
        now - last >= EVERY.as_secs() as i64 || day_of(now) != day_of(last) || self.again.is_some_and(|again| now >= again)
    }

    /// The soonest failure's next try, from the accounts as they stand.
    fn next_try(dir: &Path) -> Option<i64> {
        let day = today();
        store::load(dir).ok()?.accounts.iter()
            .filter_map(|account| account.checkin.as_ref())
            .filter(|checkin| checkin.day == day && checkin.outcome == Outcome::Failed)
            .map(|checkin| checkin.at + checkin.retry_after().as_secs() as i64)
            .min()
    }
}

/// Check the accounts in each day while their sources' auto check-in is on:
/// two minutes after the app starts, then every `EVERY`, as the Beijing day
/// turns, and as a failure comes due. The clock is looked at every minute:
/// a machine that slept through the day's turn notices on waking.
pub fn spawn(paths: CodexPaths) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        let mut state = Loop::default();
        loop {
            let now = chrono::Utc::now().timestamp();
            let wanted = store::load(&paths.app_data_dir).is_ok_and(|store| !store.checkin_on.is_empty());
            if wanted && state.due(now) {
                state.last = Some(now);
                match client(&paths) {
                    Ok(client) => {
                        let run = tokio::time::timeout(Duration::from_secs(300), run(&paths.app_data_dir, &client, None, false));
                        if let Ok(Err(error)) = run.await {
                            log::warn!("account check-in: {error}");
                        }
                    }
                    Err(error) => log::warn!("account check-in: {error}"),
                }
                state.again = Loop::next_try(&paths.app_data_dir);
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

/// A client through the app's proxy, as the Accounts page's requests go.
pub fn client(paths: &CodexPaths) -> Result<Client, String> {
    let builder = Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60));
    crate::core::api_client::configure_async_client(builder, &crate::core::newapi::proxy_from_settings(&paths.settings_path))
        .map_err(|_| "newapi.errors.proxy".to_owned())?
        .build()
        .map_err(|_| "newapi.errors.network".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_is_beijing_s() {
        // 2026-10-15 16:30 UTC is 00:30 on the 16th in Beijing.
        assert_eq!(day_of(1_792_081_800), "2026-10-16");
        assert_eq!(day_of(1_792_081_800 - 3600), "2026-10-15");
    }

    #[test]
    fn an_answer_holds_for_the_day_and_a_failure_waits_its_turn() {
        let at = 1_792_000_000;
        let day = day_of(at);
        let claimed = Checkin { day: day.clone(), at, ..Checkin::claimed(5.0, 2) };
        assert!(claimed.settled(&day, at + 10_000, true), "an answer holds even for a press");
        assert!(!claimed.settled("2026-10-17", at, false), "another day is asked again");
        let failed = Checkin { day: day.clone(), at, ..Checkin::failed("502", false) };
        assert!(failed.settled(&day, at + 60, false));
        assert!(!failed.settled(&day, at + 60, true), "a press tries a failure again at once");
        assert!(!failed.settled(&day, at + 31 * 60, false));
        let offline = Checkin { day: day.clone(), at, tries: 1, ..Checkin::failed("no route", true) };
        assert_eq!(offline.retry_after(), Duration::from_secs(120));
        assert!(!offline.settled(&day, at + 121, false));
        let many = Checkin { tries: 9, ..offline };
        assert_eq!(many.retry_after(), RETRY, "doubling stops at the usual wait");
    }

    #[test]
    fn the_loop_looks_on_its_turn_the_day_s_turn_and_a_retry() {
        let start = 1_792_000_000;
        let mut state = Loop::default();
        assert!(state.due(start));
        state.last = Some(start);
        assert!(!state.due(start + 60));
        assert!(state.due(start + EVERY.as_secs() as i64));
        state.again = Some(start + 120);
        assert!(state.due(start + 120));
        let before_midnight = 1_792_079_940; // 23:59 Beijing
        let state = Loop { last: Some(before_midnight), again: None };
        assert!(state.due(before_midnight + 120), "the day's turn is a look of its own");
    }
}
