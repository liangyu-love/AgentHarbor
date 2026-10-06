//! Bounded process-local scheduling. Selection and capacity reservation happen
//! atomically; leases follow an attempt through the complete response stream.
use super::{
    selector::Candidate,
    types::{Strategy, MAX_MEMBERS, MAX_PRIORITY, MAX_WEIGHT},
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const GLOBAL_CONCURRENT_LIMIT: usize = 128;
pub const TARGET_CONCURRENT_LIMIT: usize = 16;
pub const MAX_RUNTIME_TARGETS: usize = 4096;
pub const MAX_ROTATIONS: usize = 4096;
const IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_COOLDOWN: Duration = Duration::from_secs(60 * 60);
const MAX_CREDITS: usize = MAX_MEMBERS * 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    NoCandidates,
    Cooling { retry_after_ms: u64 },
    ConcurrencyLimit,
    StateCapacity,
    InvalidCandidates,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCandidates => "no_available_members",
            Self::Cooling { .. } => "members_cooling",
            Self::ConcurrencyLimit => "concurrency_limit",
            Self::StateCapacity => "routing_state_capacity",
            Self::InvalidCandidates => "invalid_candidates",
        })
    }
}
impl std::error::Error for SelectionError {}

#[derive(Default)]
struct Rotation {
    last_member: String,
    credits: HashMap<String, i64>,
    weights: HashMap<String, u16>,
    last_used: Option<Instant>,
}

struct TargetState {
    in_flight: usize,
    probe_in_flight: bool,
    cooldown_until: Option<Instant>,
    failures: u8,
    failure_generation: u64,
    last_used: Instant,
}

impl TargetState {
    fn new(now: Instant) -> Self {
        Self {
            in_flight: 0,
            probe_in_flight: false,
            cooldown_until: None,
            failures: 0,
            failure_generation: 0,
            last_used: now,
        }
    }
}

#[derive(Default)]
struct State {
    in_flight: usize,
    targets: HashMap<String, TargetState>,
    rotations: HashMap<(String, u16), Rotation>,
}

#[derive(Clone, Default)]
pub struct Runtime {
    state: Arc<Mutex<State>>,
}

pub fn shared() -> &'static Runtime {
    static SHARED: OnceLock<Runtime> = OnceLock::new();
    SHARED.get_or_init(Runtime::default)
}

pub struct Selection {
    pub candidate: Candidate,
    pub lease: Lease,
}

pub struct Lease {
    state: Arc<Mutex<State>>,
    target_key: String,
    probe: bool,
    failure_generation: u64,
}

impl Lease {
    /// Recovery is acknowledged only after a complete response, never merely
    /// after 200 headers or a cancelled stream.
    pub fn succeed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(target) = state.targets.get_mut(&self.target_key) {
            if target.failure_generation != self.failure_generation {
                return;
            }
            target.failures = 0;
            target.cooldown_until = None;
            target.last_used = Instant::now();
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(target) = state.targets.get_mut(&self.target_key) {
            target.in_flight = target.in_flight.saturating_sub(1);
            if self.probe {
                target.probe_in_flight = false;
            }
            target.last_used = Instant::now();
        }
        state.in_flight = state.in_flight.saturating_sub(1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeStats {
    pub in_flight: usize,
    pub targets: usize,
    pub rotations: usize,
}

impl Runtime {
    pub fn stats(&self) -> RuntimeStats {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        RuntimeStats {
            in_flight: state.in_flight,
            targets: state.targets.len(),
            rotations: state.rotations.len(),
        }
    }

    pub fn select(
        &self,
        scope: &str,
        strategy: Strategy,
        candidates: &[Candidate],
        excluded: &HashSet<String>,
    ) -> Result<Selection, SelectionError> {
        self.select_at(scope, strategy, candidates, excluded, Instant::now())
    }

    fn select_at(
        &self,
        scope: &str,
        strategy: Strategy,
        candidates: &[Candidate],
        excluded: &HashSet<String>,
        now: Instant,
    ) -> Result<Selection, SelectionError> {
        if scope.is_empty()
            || scope.len() > 1024
            || candidates.len() > MAX_MEMBERS
            || candidates.iter().any(|c| {
                c.member_id.is_empty()
                    || c.member_id.len() > 128
                    || c.target_key.is_empty()
                    || c.target_key.len() > 256
                    || c.priority > MAX_PRIORITY
                    || !(1..=MAX_WEIGHT).contains(&c.weight)
            })
        {
            return Err(SelectionError::InvalidCandidates);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        cleanup(&mut state, now);
        if state.in_flight >= GLOBAL_CONCURRENT_LIMIT {
            return Err(SelectionError::ConcurrencyLimit);
        }
        let mut ordered: Vec<_> = candidates
            .iter()
            .filter(|c| !excluded.contains(&c.target_key))
            .collect();
        ordered.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.member_id.cmp(&b.member_id))
        });
        let mut seen = HashSet::new();
        ordered.retain(|c| seen.insert(&c.target_key));
        if ordered.is_empty() {
            return Err(SelectionError::NoCandidates);
        }
        let mut earliest: Option<Duration> = None;
        let mut full = false;
        ordered.retain(|candidate| {
            if let Some(target) = state.targets.get(&candidate.target_key) {
                if let Some(until) = target.cooldown_until.filter(|until| *until > now) {
                    let remaining = until.duration_since(now);
                    earliest = Some(earliest.map_or(remaining, |old| old.min(remaining)));
                    return false;
                }
                if target.in_flight >= TARGET_CONCURRENT_LIMIT {
                    full = true;
                    return false;
                }
                if target.failures > 0 && target.probe_in_flight {
                    full = true;
                    return false;
                }
            }
            true
        });
        if ordered.is_empty() {
            return Err(if full {
                SelectionError::ConcurrencyLimit
            } else if let Some(duration) = earliest {
                SelectionError::Cooling {
                    retry_after_ms: duration.as_millis().min(u64::MAX as u128) as u64,
                }
            } else {
                SelectionError::NoCandidates
            });
        }
        let priority = ordered[0].priority;
        ordered.retain(|candidate| candidate.priority == priority);
        // A full bounded state table is an admission failure, not grounds for
        // evicting live cooldowns or silently forgetting a capacity reservation.
        ordered.retain(|candidate| {
            state.targets.contains_key(&candidate.target_key)
                || state.targets.len() < MAX_RUNTIME_TARGETS
        });
        if ordered.is_empty() {
            return Err(SelectionError::StateCapacity);
        }
        let rotation_key = (scope.to_owned(), priority);
        if !state.rotations.contains_key(&rotation_key) && state.rotations.len() >= MAX_ROTATIONS {
            return Err(SelectionError::StateCapacity);
        }
        let rotation = state.rotations.entry(rotation_key).or_default();
        rotation.last_used = Some(now);
        let selected = match strategy {
            Strategy::RoundRobin => {
                let index = ordered
                    .iter()
                    .position(|c| c.member_id > rotation.last_member)
                    .unwrap_or(0);
                ordered[index]
            }
            Strategy::WeightedRoundRobin => weighted(rotation, &ordered),
        }
        .clone();
        rotation.last_member = selected.member_id.clone();
        let target = state
            .targets
            .entry(selected.target_key.clone())
            .or_insert_with(|| TargetState::new(now));
        let probe = target.failures > 0;
        let failure_generation = target.failure_generation;
        if probe {
            target.probe_in_flight = true;
        }
        target.in_flight += 1;
        target.last_used = now;
        state.in_flight += 1;
        Ok(Selection {
            lease: Lease {
                state: self.state.clone(),
                target_key: selected.target_key.clone(),
                probe,
                failure_generation,
            },
            candidate: selected,
        })
    }

    /// Reserve a previously bound member without advancing the rotation. The
    /// caller must validate binding scope, capabilities and connection identity.
    pub fn reserve(&self, candidate: &Candidate) -> Result<Lease, SelectionError> {
        self.select(
            "__bound_reservation",
            Strategy::RoundRobin,
            std::slice::from_ref(candidate),
            &HashSet::new(),
        )
        .map(|selection| selection.lease)
    }

    pub fn cooldown(&self, key: &str, duration: Duration) -> Result<(), SelectionError> {
        self.cooldown_at(key, duration, Instant::now())
    }

    fn cooldown_at(
        &self,
        key: &str,
        duration: Duration,
        now: Instant,
    ) -> Result<(), SelectionError> {
        if key.is_empty() || key.len() > 256 {
            return Err(SelectionError::InvalidCandidates);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        cleanup(&mut state, now);
        if !state.targets.contains_key(key) && state.targets.len() >= MAX_RUNTIME_TARGETS {
            return Err(SelectionError::StateCapacity);
        }
        let target = state
            .targets
            .entry(key.to_owned())
            .or_insert_with(|| TargetState::new(now));
        let until = now + duration.min(MAX_COOLDOWN);
        if target.cooldown_until.is_none_or(|old| old < until) {
            target.cooldown_until = Some(until);
        }
        target.failures = target.failures.max(1);
        target.failure_generation = target.failure_generation.wrapping_add(1);
        target.last_used = now;
        Ok(())
    }

    pub fn record_failure(
        &self,
        key: &str,
        retry_after: Option<Duration>,
    ) -> Result<(), SelectionError> {
        let now = Instant::now();
        if key.is_empty() || key.len() > 256 {
            return Err(SelectionError::InvalidCandidates);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        cleanup(&mut state, now);
        if !state.targets.contains_key(key) && state.targets.len() >= MAX_RUNTIME_TARGETS {
            return Err(SelectionError::StateCapacity);
        }
        let target = state
            .targets
            .entry(key.to_owned())
            .or_insert_with(|| TargetState::new(now));
        let duration = retry_after
            .filter(|duration| !duration.is_zero())
            .unwrap_or_else(|| Duration::from_secs((5u64 << target.failures.min(6)).min(300)))
            .min(MAX_COOLDOWN);
        let until = now + duration;
        if target.cooldown_until.is_none_or(|old| old < until) {
            target.cooldown_until = Some(until);
        }
        target.failures = target.failures.saturating_add(1);
        target.failure_generation = target.failure_generation.wrapping_add(1);
        target.last_used = now;
        Ok(())
    }

    /// Prune deleted targets and their historical cooldown state. Live leases
    /// always retain their counters until Drop, even after a configuration edit.
    pub fn retain_targets(&self, targets: &HashSet<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .targets
            .retain(|key, state| state.in_flight > 0 || targets.contains(key));
    }

    pub fn forget_scope(&self, scope: &str) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rotations
            .retain(|(key, _), _| key != scope);
    }

    pub fn retain_scopes(&self, scopes: &HashSet<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rotations
            .retain(|(key, _), _| key == "__bound_reservation" || scopes.contains(key));
    }
}

fn cleanup(state: &mut State, now: Instant) {
    state.targets.retain(|_, target| {
        target.in_flight > 0
            || target.cooldown_until.is_some_and(|until| until > now)
            || now.saturating_duration_since(target.last_used) < IDLE_TTL
    });
    state.rotations.retain(|_, rotation| {
        rotation
            .last_used
            .is_some_and(|used| now.saturating_duration_since(used) < IDLE_TTL)
    });
}

fn weighted<'a>(rotation: &mut Rotation, candidates: &[&'a Candidate]) -> &'a Candidate {
    if candidates.iter().any(|c| {
        rotation
            .weights
            .get(&c.member_id)
            .is_some_and(|old| *old != c.weight)
    }) {
        rotation.credits.clear();
    }
    let new_members = candidates
        .iter()
        .filter(|c| !rotation.weights.contains_key(&c.member_id))
        .count();
    if rotation.weights.len().saturating_add(new_members) > MAX_CREDITS {
        rotation
            .weights
            .retain(|id, _| candidates.iter().any(|c| &c.member_id == id));
        rotation
            .credits
            .retain(|id, _| candidates.iter().any(|c| &c.member_id == id));
    }
    let mut total = 0i64;
    let mut selected = candidates[0];
    let mut best = i64::MIN;
    for candidate in candidates {
        rotation
            .weights
            .insert(candidate.member_id.clone(), candidate.weight);
        let credit = rotation
            .credits
            .entry(candidate.member_id.clone())
            .or_default();
        *credit = credit.saturating_add(candidate.weight as i64);
        total += candidate.weight as i64;
        if *credit > best {
            selected = candidate;
            best = *credit;
        }
    }
    if let Some(credit) = rotation.credits.get_mut(&selected.member_id) {
        *credit = credit.saturating_sub(total);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(id: &str, priority: u16, weight: u16) -> Candidate {
        Candidate {
            member_id: id.into(),
            provider_id: id.into(),
            actual_model_id: "wire".into(),
            target_key: format!("physical-{id}"),
            priority,
            weight,
        }
    }

    #[test]
    fn rotation_survives_filter_and_priority_recovery() {
        let runtime = Runtime::default();
        let all = vec![
            candidate("a", 0, 1),
            candidate("b", 0, 1),
            candidate("c", 1, 1),
        ];
        let now = Instant::now();
        let empty = HashSet::new();
        let first = runtime
            .select_at("model", Strategy::RoundRobin, &all, &empty, now)
            .unwrap();
        assert_eq!(first.candidate.member_id, "a");
        drop(first);
        runtime
            .cooldown_at("physical-a", Duration::from_secs(10), now)
            .unwrap();
        let second = runtime
            .select_at("model", Strategy::RoundRobin, &all, &empty, now)
            .unwrap();
        assert_eq!(second.candidate.member_id, "b");
        drop(second);
        runtime
            .cooldown_at("physical-b", Duration::from_secs(10), now)
            .unwrap();
        let third = runtime
            .select_at("model", Strategy::RoundRobin, &all, &empty, now)
            .unwrap();
        assert_eq!(third.candidate.member_id, "c");
        drop(third);
        let next = runtime
            .select_at(
                "model",
                Strategy::RoundRobin,
                &all,
                &empty,
                now + Duration::from_secs(11),
            )
            .unwrap();
        assert_eq!(next.candidate.member_id, "a");
    }

    #[test]
    fn weighted_ratio_and_transient_exclusions_preserve_credits() {
        let runtime = Runtime::default();
        let all = vec![candidate("a", 0, 3), candidate("b", 0, 1)];
        let mut count = [0, 0];
        for _ in 0..40 {
            let selection = runtime
                .select("model", Strategy::WeightedRoundRobin, &all, &HashSet::new())
                .unwrap();
            count[usize::from(selection.candidate.member_id == "b")] += 1;
        }
        assert_eq!(count, [30, 10]);
        for _ in 0..10 {
            let selection = runtime
                .select(
                    "model",
                    Strategy::WeightedRoundRobin,
                    &all,
                    &HashSet::from(["physical-a".into()]),
                )
                .unwrap();
            assert_eq!(selection.candidate.member_id, "b");
        }
        let state = runtime.state.lock().unwrap();
        let rotation = &state.rotations[&("model".into(), 0)];
        assert!(rotation.credits.contains_key("a"));
    }

    #[test]
    fn transient_subset_does_not_reset_nonzero_weighted_credits() {
        let runtime = Runtime::default();
        let all = vec![candidate("a", 0, 3), candidate("b", 0, 1)];
        let first = runtime
            .select("model", Strategy::WeightedRoundRobin, &all, &HashSet::new())
            .unwrap();
        assert_eq!(first.candidate.member_id, "a");
        drop(first);
        let before = runtime.state.lock().unwrap().rotations[&("model".into(), 0)]
            .credits
            .clone();
        let second = runtime
            .select(
                "model",
                Strategy::WeightedRoundRobin,
                &all,
                &HashSet::from(["physical-a".into()]),
            )
            .unwrap();
        assert_eq!(second.candidate.member_id, "b");
        drop(second);
        let after = runtime.state.lock().unwrap().rotations[&("model".into(), 0)]
            .credits
            .clone();
        assert_eq!(before, after);
        assert_eq!(after.get("a"), Some(&-1));
        assert_eq!(after.get("b"), Some(&1));
    }

    #[test]
    fn physical_capacity_is_shared_and_drop_releases() {
        let runtime = Runtime::default();
        let candidate = candidate("a", 0, 1);
        let mut held = vec![];
        for i in 0..TARGET_CONCURRENT_LIMIT {
            held.push(
                runtime
                    .select(
                        &format!("aggregate-{i}"),
                        Strategy::RoundRobin,
                        std::slice::from_ref(&candidate),
                        &HashSet::new(),
                    )
                    .unwrap(),
            );
        }
        assert!(matches!(
            runtime.select(
                "another",
                Strategy::RoundRobin,
                std::slice::from_ref(&candidate),
                &HashSet::new()
            ),
            Err(SelectionError::ConcurrencyLimit)
        ));
        held.pop();
        let last = runtime.reserve(&candidate).unwrap();
        assert_eq!(runtime.stats().in_flight, TARGET_CONCURRENT_LIMIT);
        drop(last);
        drop(held);
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn exclusion_uses_physical_identity_and_cooldown_is_monotonic() {
        let runtime = Runtime::default();
        let a = candidate("a", 0, 1);
        let mut alias = candidate("alias", 0, 1);
        alias.target_key = a.target_key.clone();
        assert!(matches!(
            runtime.select(
                "model",
                Strategy::RoundRobin,
                &[a.clone(), alias],
                &HashSet::from([a.target_key.clone()])
            ),
            Err(SelectionError::NoCandidates)
        ));
        let now = Instant::now();
        runtime
            .cooldown_at(&a.target_key, Duration::from_secs(10), now)
            .unwrap();
        runtime
            .cooldown_at(&a.target_key, Duration::from_secs(1), now)
            .unwrap();
        assert!(matches!(
            runtime.select_at("model", Strategy::RoundRobin, &[a], &HashSet::new(), now),
            Err(SelectionError::Cooling {
                retry_after_ms: 10000
            })
        ));
    }

    #[test]
    fn concurrent_selection_keeps_round_robin_balanced() {
        let runtime = Runtime::default();
        let candidates = Arc::new(vec![
            candidate("a", 0, 1),
            candidate("b", 0, 1),
            candidate("c", 0, 1),
            candidate("d", 0, 1),
        ]);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let runtime = runtime.clone();
                let candidates = candidates.clone();
                std::thread::spawn(move || {
                    (0..100)
                        .map(|_| {
                            runtime
                                .select("model", Strategy::RoundRobin, &candidates, &HashSet::new())
                                .unwrap()
                                .candidate
                                .member_id
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut counts = HashMap::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                *counts.entry(id).or_insert(0) += 1;
            }
        }
        assert_eq!(counts.len(), 4);
        assert!(counts.values().all(|count| *count == 200));
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn cooldown_recovery_allows_one_probe_until_it_finishes() {
        let runtime = Runtime::default();
        let candidate = candidate("a", 0, 1);
        let now = Instant::now();
        runtime
            .cooldown_at(&candidate.target_key, Duration::from_secs(1), now)
            .unwrap();
        let recovered = now + Duration::from_secs(2);
        let probe = runtime
            .select_at(
                "model",
                Strategy::RoundRobin,
                std::slice::from_ref(&candidate),
                &HashSet::new(),
                recovered,
            )
            .unwrap();
        assert!(matches!(
            runtime.select_at(
                "other",
                Strategy::RoundRobin,
                std::slice::from_ref(&candidate),
                &HashSet::new(),
                recovered
            ),
            Err(SelectionError::ConcurrencyLimit)
        ));
        probe.lease.succeed();
        let next = runtime
            .select_at(
                "other",
                Strategy::RoundRobin,
                std::slice::from_ref(&candidate),
                &HashSet::new(),
                recovered,
            )
            .unwrap();
        drop(next);
        drop(probe);
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn older_in_flight_success_cannot_clear_a_newer_rate_limit() {
        let runtime = Runtime::default();
        let candidate = candidate("a", 0, 1);
        let old = runtime.reserve(&candidate).unwrap();
        runtime
            .record_failure(&candidate.target_key, Some(Duration::from_secs(30)))
            .unwrap();
        old.succeed();
        assert!(matches!(
            runtime.reserve(&candidate),
            Err(SelectionError::Cooling { .. })
        ));
        drop(old);
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn global_limit_and_runtime_state_limits_are_admission_boundaries() {
        let runtime = Runtime::default();
        let mut held = vec![];
        for index in 0..GLOBAL_CONCURRENT_LIMIT {
            held.push(
                runtime
                    .select(
                        "model",
                        Strategy::RoundRobin,
                        &[candidate(&format!("t-{index}"), 0, 1)],
                        &HashSet::new(),
                    )
                    .unwrap(),
            );
        }
        assert!(matches!(
            runtime.select(
                "model",
                Strategy::RoundRobin,
                &[candidate("another", 0, 1)],
                &HashSet::new()
            ),
            Err(SelectionError::ConcurrencyLimit)
        ));
        drop(held);
        let now = Instant::now();
        {
            let mut state = runtime.state.lock().unwrap();
            state.targets.clear();
            for index in 0..MAX_RUNTIME_TARGETS {
                state
                    .targets
                    .insert(format!("target-{index}"), TargetState::new(now));
            }
        }
        assert!(matches!(
            runtime.select_at(
                "model",
                Strategy::RoundRobin,
                &[candidate("new", 0, 1)],
                &HashSet::new(),
                now
            ),
            Err(SelectionError::StateCapacity)
        ));
        runtime.retain_targets(&HashSet::new());
        assert_eq!(runtime.stats().targets, 0);
        {
            let mut state = runtime.state.lock().unwrap();
            for index in 0..MAX_ROTATIONS {
                state.rotations.insert(
                    (format!("scope-{index}"), 0),
                    Rotation {
                        last_used: Some(now),
                        ..Default::default()
                    },
                );
            }
        }
        assert!(matches!(
            runtime.select_at(
                "new-scope",
                Strategy::RoundRobin,
                &[candidate("a", 0, 1)],
                &HashSet::new(),
                now
            ),
            Err(SelectionError::StateCapacity)
        ));
        assert!(runtime
            .select_at(
                "new-scope",
                Strategy::RoundRobin,
                &[candidate("a", 0, 1)],
                &HashSet::new(),
                now + IDLE_TTL + Duration::from_secs(1)
            )
            .is_ok());
    }
}
