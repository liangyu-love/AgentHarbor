//! Bounded process-local scheduling. Selection and capacity reservation happen
//! atomically; leases follow an attempt through the complete response stream.
use super::{
    selector::Candidate,
    types::{MemberLive, Strategy, MAX_MEMBERS, MAX_PRIORITY, MAX_WEIGHT},
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
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
    probe_generation: Option<u64>,
    cooldown_until: Option<Instant>,
    retry_until: Option<Instant>,
    tripped: bool,
    failures: u8,
    failure_generation: u64,
    last_used: Instant,
}

impl TargetState {
    fn new(now: Instant) -> Self {
        Self {
            in_flight: 0,
            probe_generation: None,
            cooldown_until: None,
            retry_until: None,
            tripped: false,
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
    preferred: HashMap<String, Preferred>,
}

struct Preferred {
    member_id: String,
    target_key: String,
    last_used: Instant,
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
    health: Health,
}

/// An actual inference attempt reports exactly one outcome. Dropping a lease
/// (client cancellation), preflight, counting and resource reads report none.
#[derive(Clone)]
pub struct Health {
    state: Arc<Mutex<State>>,
    target_key: String,
    failure_generation: u64,
    probe_generation: Option<u64>,
    threshold: u8,
    outcome: Arc<AtomicU8>,
}

impl Health {
    pub fn succeed(&self) {
        if self
            .outcome
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(target) = state.targets.get_mut(&self.target_key) {
            if target.failure_generation != self.failure_generation {
                return;
            }
            target.failures = 0;
            target.cooldown_until = None;
            target.retry_until = None;
            target.tripped = false;
            if target.probe_generation == self.probe_generation {
                target.probe_generation = None;
            }
            target.last_used = Instant::now();
        }
    }

    /// Return true only when this real failure reaches the switch threshold.
    /// Retry-After below the threshold pauses the preferred channel without
    /// allowing an alternative to bypass that pause.
    pub fn fail(&self, retry_after: Option<Duration>) -> bool {
        if self
            .outcome
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = state.targets.get_mut(&self.target_key) else {
            return false;
        };
        // A known failed probe must stop blocking replacement selection even
        // while its response body still owns the physical concurrency slot.
        if self.probe_generation.is_some() && target.probe_generation == self.probe_generation {
            target.probe_generation = None;
        }
        target.failures = target.failures.saturating_add(1);
        target.failure_generation = target.failure_generation.wrapping_add(1);
        target.last_used = now;
        let reached = target.failures >= self.threshold;
        if let Some(duration) = retry_after.filter(|d| !d.is_zero()) {
            let until = now + duration.min(MAX_COOLDOWN);
            target.retry_until = Some(target.retry_until.map_or(until, |old| old.max(until)));
        }
        if reached {
            target.tripped = true;
            let duration = retry_after
                .filter(|d| !d.is_zero())
                .unwrap_or_else(|| {
                    Duration::from_secs(
                        (5u64 << target.failures.saturating_sub(self.threshold).min(6)).min(300),
                    )
                })
                .min(MAX_COOLDOWN);
            let until = now + duration;
            target.cooldown_until = Some(target.cooldown_until.map_or(until, |old| old.max(until)));
        }
        reached
    }

    pub fn neutral(&self) {
        let _ = self
            .outcome
            .compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire);
    }

    /// Hosted tools can execute several actual model requests on one lease.
    /// Each round takes a fresh generation and reports its own outcome.
    pub fn next_attempt(&self) -> Self {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self {
            failure_generation: state
                .targets
                .get(&self.target_key)
                .map_or(0, |t| t.failure_generation),
            outcome: Arc::new(AtomicU8::new(0)),
            ..self.clone()
        }
    }
}

impl Lease {
    pub fn inference(&mut self, threshold: u8, enabled: bool) {
        self.health.threshold = threshold.clamp(2, 10);
        if !enabled {
            self.health.neutral();
        }
    }
    pub fn health(&self) -> Health {
        self.health.clone()
    }
    /// Recovery is acknowledged only after a complete response, never merely
    /// after 200 headers or a cancelled stream.
    pub fn succeed(&self) {
        self.health.succeed();
    }
    pub fn fail(&self, retry_after: Option<Duration>) -> bool {
        self.health.fail(retry_after)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(target) = state.targets.get_mut(&self.target_key) {
            target.in_flight = target.in_flight.saturating_sub(1);
            if self.health.probe_generation.is_some()
                && target.probe_generation == self.health.probe_generation
            {
                target.probe_generation = None;
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

    pub fn should_switch(&self, key: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .targets
            .get(key)
            .is_some_and(|target| target.tripped)
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
        self.select_inner(scope, strategy, candidates, excluded, now, true)
    }

    /// Whether a selection excluding `excluded` would find a member now, without
    /// reserving one or moving the preference.
    pub fn has_alternative(&self, candidates: &[Candidate], excluded: &HashSet<String>) -> bool {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.in_flight < GLOBAL_CONCURRENT_LIMIT
            && candidates.iter().any(|candidate| {
                !excluded.contains(&candidate.target_key)
                    && state.targets.get(&candidate.target_key).is_none_or(|target| {
                        target.cooldown_until.into_iter().chain(target.retry_until).all(|until| until <= now)
                            && target.in_flight < TARGET_CONCURRENT_LIMIT
                            && !(target.tripped && target.probe_generation.is_some())
                    })
            })
    }

    /// The member a scope's requests stick to (and how long since it was
    /// chosen), with each `(member_id, target_key)`'s health. Read only.
    pub fn snapshot(
        &self,
        scope: &str,
        members: &[(String, String)],
    ) -> (Option<String>, Option<u64>, Vec<MemberLive>) {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let preferred = state.preferred.get(scope).filter(|p| {
            members.iter().any(|(id, key)| *id == p.member_id && *key == p.target_key)
        });
        let live = members
            .iter()
            .map(|(member_id, key)| {
                let target = state.targets.get(key);
                MemberLive {
                    member_id: member_id.clone(),
                    in_flight: target.map_or(0, |t| t.in_flight),
                    failures: target.map_or(0, |t| t.failures),
                    cooling_ms: target
                        .and_then(|t| t.cooldown_until.into_iter().chain(t.retry_until).max())
                        .filter(|until| *until > now)
                        .map(|until| until.duration_since(now).as_millis().min(u64::MAX as u128) as u64),
                    tripped: target.is_some_and(|t| t.tripped),
                }
            })
            .collect();
        (
            preferred.map(|p| p.member_id.clone()),
            preferred.map(|p| now.saturating_duration_since(p.last_used).as_secs()),
            live,
        )
    }

    /// Auxiliary operations do not change the inference channel preference.
    pub fn select_auxiliary(&self, candidates: &[Candidate]) -> Result<Selection, SelectionError> {
        self.select_inner(
            "__auxiliary",
            Strategy::RoundRobin,
            candidates,
            &HashSet::new(),
            Instant::now(),
            false,
        )
    }

    fn select_inner(
        &self,
        scope: &str,
        strategy: Strategy,
        candidates: &[Candidate],
        excluded: &HashSet<String>,
        now: Instant,
        remember: bool,
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
        let preferred = remember
            .then(|| state.preferred.get(scope))
            .flatten()
            .and_then(|p| candidates.iter().find(|c| c.member_id == p.member_id));
        let mut seen = HashSet::new();
        // Deduplicate physical connections without dropping a preferred alias
        // in favour of an earlier member ID or a newly higher priority label.
        ordered.retain(|c| {
            preferred.is_none_or(|p| p.target_key != c.target_key || p.member_id == c.member_id)
                && seen.insert(&c.target_key)
        });
        if ordered.is_empty() {
            return Err(SelectionError::NoCandidates);
        }
        // Busy or rate-limited is not a reason to rotate a healthy preference.
        if let Some(candidate) = preferred.filter(|c| !excluded.contains(&c.target_key)) {
            if let Some(target) = state.targets.get(&candidate.target_key) {
                if target.probe_generation.is_some() {
                    return Err(SelectionError::ConcurrencyLimit);
                }
                if !target.tripped {
                    if let Some(until) = target.retry_until.filter(|until| *until > now) {
                        return Err(SelectionError::Cooling {
                            retry_after_ms: until.duration_since(now).as_millis() as u64,
                        });
                    }
                    if target.in_flight >= TARGET_CONCURRENT_LIMIT {
                        return Err(SelectionError::ConcurrencyLimit);
                    }
                }
            }
        }
        let mut earliest: Option<Duration> = None;
        let mut full = false;
        ordered.retain(|candidate| {
            if let Some(target) = state.targets.get(&candidate.target_key) {
                if let Some(until) = target
                    .cooldown_until
                    .into_iter()
                    .chain(target.retry_until)
                    .filter(|until| *until > now)
                    .max()
                {
                    let remaining = until.duration_since(now);
                    earliest = Some(earliest.map_or(remaining, |old| old.min(remaining)));
                    return false;
                }
                if target.in_flight >= TARGET_CONCURRENT_LIMIT {
                    full = true;
                    return false;
                }
                if target.tripped && target.probe_generation.is_some() {
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
        let stable = preferred.filter(|c| {
            ordered.iter().any(|o| o.target_key == c.target_key)
                && state.targets.get(&c.target_key).is_none_or(|t| !t.tripped)
        });
        // Once switched, do not reclaim an old higher-priority channel merely
        // because its cooldown expired. Priority orders replacement choices.
        if stable.is_none() && ordered.len() > 1 {
            if let Some(old) = preferred {
                ordered.retain(|c| c.target_key != old.target_key);
            }
        }
        let priority = stable.map_or(ordered[0].priority, |c| c.priority);
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
        if remember
            && stable.is_none()
            && !state.rotations.contains_key(&rotation_key)
            && state.rotations.len() >= MAX_ROTATIONS
        {
            return Err(SelectionError::StateCapacity);
        }
        if remember
            && !state.preferred.contains_key(scope)
            && state.preferred.len() >= MAX_ROTATIONS
        {
            return Err(SelectionError::StateCapacity);
        }
        let selected = if let Some(stable) = stable {
            if let Some(rotation) = state.rotations.get_mut(&rotation_key) {
                rotation.last_used = Some(now);
            }
            stable.clone()
        } else if !remember {
            ordered[0].clone()
        } else {
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
            selected
        };
        // A capability-filtered operation may temporarily use another member,
        // but must not replace a preferred member absent from this candidate set.
        if remember && (preferred.is_some() || !state.preferred.contains_key(scope)) {
            state.preferred.insert(
                scope.into(),
                Preferred {
                    member_id: selected.member_id.clone(),
                    target_key: selected.target_key.clone(),
                    last_used: now,
                },
            );
        } else if let Some(preferred) = state.preferred.get_mut(scope) {
            preferred.last_used = now;
        }
        let target = state
            .targets
            .entry(selected.target_key.clone())
            .or_insert_with(|| TargetState::new(now));
        let probe_generation = if target.tripped {
            // Each recovery reservation has a unique generation. A cancelled
            // or completed older lease cannot acknowledge/release a new probe.
            target.failure_generation = target.failure_generation.wrapping_add(1);
            target.probe_generation = Some(target.failure_generation);
            target.probe_generation
        } else {
            None
        };
        let failure_generation = target.failure_generation;
        target.in_flight += 1;
        target.last_used = now;
        state.in_flight += 1;
        Ok(Selection {
            lease: Lease {
                state: self.state.clone(),
                target_key: selected.target_key.clone(),
                health: Health {
                    state: self.state.clone(),
                    target_key: selected.target_key.clone(),
                    failure_generation,
                    probe_generation,
                    threshold: 3,
                    outcome: Arc::new(AtomicU8::new(0)),
                },
            },
            candidate: selected,
        })
    }

    /// Reserve a previously bound member without advancing the rotation. The
    /// caller must validate binding scope, capabilities and connection identity.
    pub fn reserve(&self, candidate: &Candidate) -> Result<Lease, SelectionError> {
        self.select_inner(
            "__bound_reservation",
            Strategy::RoundRobin,
            std::slice::from_ref(candidate),
            &HashSet::new(),
            Instant::now(),
            false,
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
        target.tripped = true;
        target.failure_generation = target.failure_generation.wrapping_add(1);
        target.last_used = now;
        Ok(())
    }

    /// Prune deleted targets and their historical cooldown state. Live leases
    /// always retain their counters until Drop, even after a configuration edit.
    pub fn retain_targets(&self, targets: &HashSet<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .targets
            .retain(|key, state| state.in_flight > 0 || targets.contains(key));
        state
            .preferred
            .retain(|_, p| targets.contains(&p.target_key));
    }

    pub fn forget_scope(&self, scope: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.rotations.retain(|(key, _), _| key != scope);
        state.preferred.remove(scope);
    }

    pub fn retain_scopes(&self, scopes: &HashSet<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.rotations.retain(|(key, _), _| scopes.contains(key));
        state.preferred.retain(|key, _| scopes.contains(key));
    }

    pub fn retain_members(&self, members: &HashMap<String, HashSet<String>>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .preferred
            .retain(|scope, p| {
                members
                    .get(scope)
                    .is_some_and(|ids| ids.contains(&p.member_id))
            });
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
    state
        .preferred
        .retain(|_, p| now.saturating_duration_since(p.last_used) < IDLE_TTL);
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
    fn snapshot_and_alternatives_read_without_changing_the_choice() {
        let runtime = Runtime::default();
        let all = vec![candidate("a", 0, 1), candidate("b", 1, 1)];
        let members: Vec<_> = all.iter().map(|c| (c.member_id.clone(), c.target_key.clone())).collect();
        let (preferred, idle, live) = runtime.snapshot("model", &members);
        assert_eq!((preferred, idle), (None, None));
        assert!(live.iter().all(|m| m.failures == 0 && m.cooling_ms.is_none() && !m.tripped));

        let mut first = runtime.select("model", Strategy::RoundRobin, &all, &HashSet::new()).unwrap();
        first.lease.inference(2, true);
        let (preferred, _, live) = runtime.snapshot("model", &members);
        assert_eq!(preferred.as_deref(), Some("a"));
        assert_eq!(live[0].in_flight, 1);
        let only_a = HashSet::from(["physical-b".to_string()]);
        assert!(runtime.has_alternative(&all, &only_a), "a is busy but not full");
        let excluded = HashSet::from(["physical-a".to_string()]);
        assert!(runtime.has_alternative(&all, &excluded));
        assert!(!runtime.has_alternative(&all[..1], &excluded), "nothing is left once a is excluded");

        assert!(!first.lease.fail(None));
        assert!(first.lease.health().next_attempt().fail(None), "the second failure reaches the threshold");
        drop(first);
        let (preferred, _, live) = runtime.snapshot("model", &members);
        assert_eq!(preferred.as_deref(), Some("a"), "reading never moves the preference");
        assert_eq!((live[0].failures, live[0].tripped, live[0].in_flight), (2, true, 0));
        assert!(live[0].cooling_ms.is_some_and(|ms| ms > 0 && ms <= 5_000));
        assert!(!runtime.has_alternative(&all[..1], &HashSet::new()), "a cooling member is no alternative");
        assert!(runtime.has_alternative(&all, &HashSet::new()));
        assert_eq!(runtime.snapshot("other", &members).0, None);
    }

    #[test]
    fn preferred_channel_survives_old_priority_recovery() {
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
        assert_eq!(next.candidate.member_id, "c");
    }

    #[test]
    fn weighted_selection_stays_on_the_hit_channel_and_switches_without_reclaiming() {
        let runtime = Runtime::default();
        let all = vec![candidate("a", 0, 3), candidate("b", 0, 1)];
        let mut count = [0, 0];
        for _ in 0..40 {
            let selection = runtime
                .select("model", Strategy::WeightedRoundRobin, &all, &HashSet::new())
                .unwrap();
            count[usize::from(selection.candidate.member_id == "b")] += 1;
        }
        assert_eq!(
            count,
            [40, 0],
            "weights choose a channel, not a channel per request"
        );
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
        drop(state);
        assert_eq!(
            runtime
                .select("model", Strategy::WeightedRoundRobin, &all, &HashSet::new())
                .unwrap()
                .candidate
                .member_id,
            "b"
        );
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
                &all[1..],
                &HashSet::new(),
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
    fn concurrent_selection_keeps_the_same_channel() {
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
        assert_eq!(counts.len(), 1);
        assert_eq!(counts["a"], 800);
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
        let newer = runtime.reserve(&candidate).unwrap();
        assert!(!newer.fail(Some(Duration::from_secs(30))));
        old.succeed();
        assert!(matches!(
            runtime.reserve(&candidate),
            Err(SelectionError::Cooling { .. })
        ));
        drop(old);
        drop(newer);
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn consecutive_failures_switch_only_at_threshold_and_success_resets() {
        let runtime = Runtime::default();
        let all = [candidate("a", 0, 1), candidate("b", 0, 1)];
        let select = || {
            runtime
                .select("model", Strategy::RoundRobin, &all, &HashSet::new())
                .unwrap()
        };
        for _ in 0..2 {
            let selection = select();
            assert_eq!(selection.candidate.member_id, "a");
            assert!(!selection.lease.fail(None));
            assert!(!selection.lease.fail(None), "an attempt cannot count twice");
        }
        let success = select();
        success.lease.succeed();
        success.lease.succeed();
        drop(success);
        for index in 1..=3 {
            let selection = select();
            assert_eq!(selection.candidate.member_id, "a");
            assert_eq!(selection.lease.fail(None), index == 3);
        }
        for _ in 0..8 {
            let selection = select();
            assert_eq!(selection.candidate.member_id, "b");
            selection.lease.succeed();
        }
        assert_eq!(
            runtime.state.lock().unwrap().targets["physical-a"].failures,
            3
        );
        assert_eq!(runtime.stats().in_flight, 0);
    }

    #[test]
    fn busy_preferred_does_not_rotate_and_auxiliary_operations_do_not_touch_health() {
        let runtime = Runtime::default();
        let all = [candidate("a", 0, 1), candidate("b", 0, 1)];
        let mut held = Vec::new();
        for _ in 0..TARGET_CONCURRENT_LIMIT {
            held.push(
                runtime
                    .select("model", Strategy::RoundRobin, &all, &HashSet::new())
                    .unwrap(),
            );
        }
        assert!(matches!(
            runtime.select("model", Strategy::RoundRobin, &all, &HashSet::new()),
            Err(SelectionError::ConcurrencyLimit)
        ));
        drop(held);
        let failed = runtime
            .select("model", Strategy::RoundRobin, &all, &HashSet::new())
            .unwrap();
        failed.lease.fail(None);
        drop(failed);
        let mut counting = runtime.select_auxiliary(&all[..1]).unwrap();
        counting.lease.inference(3, false);
        counting.lease.succeed();
        drop(counting);
        let mut auxiliary = runtime.select_auxiliary(&all[1..]).unwrap();
        auxiliary.lease.inference(3, false);
        assert!(!auxiliary.lease.fail(None));
        auxiliary.lease.succeed();
        drop(auxiliary);
        let selection = runtime
            .select("model", Strategy::RoundRobin, &all, &HashSet::new())
            .unwrap();
        assert_eq!(selection.candidate.member_id, "a");
        drop(selection); // cancellation must not count
        let state = runtime.state.lock().unwrap();
        assert_eq!(state.targets["physical-a"].failures, 1);
        assert_eq!(state.targets["physical-b"].failures, 0);
    }

    #[test]
    fn retry_after_pauses_preferred_without_counting_local_rejections() {
        let runtime = Runtime::default();
        let all = [candidate("a", 0, 1), candidate("b", 0, 1)];
        let selected = runtime
            .select("model", Strategy::RoundRobin, &all, &HashSet::new())
            .unwrap();
        assert!(!selected.lease.fail(Some(Duration::from_secs(30))));
        drop(selected);
        for _ in 0..8 {
            assert!(matches!(
                runtime.select("model", Strategy::RoundRobin, &all, &HashSet::new()),
                Err(SelectionError::Cooling { .. })
            ));
        }
        let now = Instant::now() + Duration::from_secs(31);
        let selected = runtime
            .select_at("model", Strategy::RoundRobin, &all, &HashSet::new(), now)
            .unwrap();
        assert_eq!(selected.candidate.member_id, "a");
        assert_eq!(
            runtime.state.lock().unwrap().targets["physical-a"].failures,
            1
        );
    }

    #[test]
    fn effective_protocol_changes_do_not_rotate_a_member_or_create_a_new_preference() {
        let runtime = Runtime::default();
        let all = [candidate("a", 0, 1), candidate("b", 0, 1)];
        drop(
            runtime
                .select("model", Strategy::RoundRobin, &all, &HashSet::new())
                .unwrap(),
        );
        let mut compatibility = all.clone();
        compatibility[0].target_key = "physical-a-chat-search".into();
        for _ in 0..4 {
            assert_eq!(
                runtime
                    .select(
                        "model",
                        Strategy::RoundRobin,
                        &compatibility,
                        &HashSet::new()
                    )
                    .unwrap()
                    .candidate
                    .member_id,
                "a"
            );
            assert_eq!(
                runtime
                    .select("model", Strategy::RoundRobin, &all, &HashSet::new())
                    .unwrap()
                    .candidate
                    .member_id,
                "a"
            );
        }
        assert_eq!(runtime.state.lock().unwrap().preferred.len(), 1);
    }

    #[test]
    fn lower_priority_hit_is_kept_until_failure_then_a_recovered_higher_group_is_selected() {
        for strategy in [Strategy::RoundRobin, Strategy::WeightedRoundRobin] {
            let runtime = Runtime::default();
            let all = [
                candidate("high", 0, 1),
                candidate("low", 1, 1),
                candidate("lower", 2, 1),
            ];
            let now = Instant::now();
            for index in 1..=3 {
                let selected = runtime
                    .select_at("model", strategy, &all, &HashSet::new(), now)
                    .unwrap();
                assert_eq!(selected.candidate.member_id, "high");
                assert_eq!(selected.lease.fail(None), index == 3);
            }
            let after = now + Duration::from_secs(1);
            let low = runtime
                .select_at("model", strategy, &all, &HashSet::new(), after)
                .unwrap();
            assert_eq!(low.candidate.member_id, "low");
            low.lease.succeed();
            drop(low);
            let recovered = now + Duration::from_secs(10);
            for _ in 0..10 {
                let selected = runtime
                    .select_at("model", strategy, &all, &HashSet::new(), recovered)
                    .unwrap();
                assert_eq!(
                    selected.candidate.member_id, "low",
                    "recovery cannot preempt the healthy hit"
                );
                selected.lease.succeed();
            }
            for index in 1..=3 {
                let selected = runtime
                    .select_at("model", strategy, &all, &HashSet::new(), recovered)
                    .unwrap();
                assert_eq!(selected.candidate.member_id, "low");
                assert_eq!(selected.lease.fail(None), index == 3);
            }
            let high = runtime
                .select_at("model", strategy, &all, &HashSet::new(), recovered)
                .unwrap();
            assert_eq!(
                high.candidate.member_id, "high",
                "replacement starts again at the highest available group"
            );
            assert!(
                matches!(
                    runtime.select_at("model", strategy, &all, &HashSet::new(), recovered),
                    Err(SelectionError::ConcurrencyLimit)
                ),
                "a pending recovery probe must not rotate the preference"
            );
            high.lease.succeed();
            drop(high);
            assert_eq!(
                runtime
                    .select_at("model", strategy, &all, &HashSet::new(), recovered)
                    .unwrap()
                    .candidate
                    .member_id,
                "high"
            );
        }
    }

    #[test]
    fn failed_recovery_probe_allows_replacement_before_its_response_is_dropped() {
        for strategy in [Strategy::RoundRobin, Strategy::WeightedRoundRobin] {
            let runtime = Runtime::default();
            let all = [candidate("high", 0, 1), candidate("low", 1, 1)];
            for _ in 0..3 {
                runtime
                    .select("model", strategy, &all, &HashSet::new())
                    .unwrap()
                    .lease
                    .fail(None);
            }
            let recovered = Instant::now() + Duration::from_secs(10);
            let failed_probe = runtime
                .select_at("model", strategy, &all[..1], &HashSet::new(), recovered)
                .unwrap();
            assert!(failed_probe.lease.fail(None));
            let replacement = runtime
                .select_at("model", strategy, &all, &HashSet::new(), recovered)
                .expect("an already failed probe cannot block a usable alternative");
            assert_eq!(replacement.candidate.member_id, "low");
            replacement.lease.succeed();
            drop(replacement);

            let later = Instant::now() + Duration::from_secs(20);
            let next_probe = runtime
                .select_at("other", strategy, &all[..1], &HashSet::new(), later)
                .unwrap();
            drop(failed_probe);
            assert!(
                matches!(
                    runtime.select_at("other", strategy, &all[..1], &HashSet::new(), later),
                    Err(SelectionError::ConcurrencyLimit)
                ),
                "dropping an older probe cannot release the newer probe's gate"
            );
            next_probe.lease.succeed();
            drop(next_probe);
            assert_eq!(runtime.stats().in_flight, 0);
        }
    }

    #[test]
    fn cancelling_a_probe_cannot_let_its_late_outcome_release_a_new_probe() {
        let runtime = Runtime::default();
        let all = [candidate("a", 0, 1), candidate("b", 1, 1)];
        runtime
            .cooldown(&all[0].target_key, Duration::ZERO)
            .unwrap();
        let old = runtime
            .select("model", Strategy::RoundRobin, &all[..1], &HashSet::new())
            .unwrap();
        let late_health = old.lease.health();
        drop(old);
        let current = runtime
            .select("model", Strategy::RoundRobin, &all[..1], &HashSet::new())
            .unwrap();
        late_health.succeed();
        assert!(matches!(
            runtime.select("model", Strategy::RoundRobin, &all[..1], &HashSet::new()),
            Err(SelectionError::ConcurrencyLimit)
        ));
        current.lease.succeed();
    }

    #[test]
    fn physical_alias_deduplication_keeps_the_preferred_member_and_its_priority() {
        let runtime = Runtime::default();
        let a = candidate("a", 0, 1);
        let mut alias = candidate("alias", 1, 1);
        alias.target_key = a.target_key.clone();
        drop(
            runtime
                .select(
                    "model",
                    Strategy::RoundRobin,
                    std::slice::from_ref(&alias),
                    &HashSet::new(),
                )
                .unwrap(),
        );
        for _ in 0..5 {
            let selected = runtime
                .select(
                    "model",
                    Strategy::RoundRobin,
                    &[a.clone(), alias.clone()],
                    &HashSet::new(),
                )
                .unwrap();
            assert_eq!(selected.candidate.member_id, "alias");
            selected.lease.succeed();
        }
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
