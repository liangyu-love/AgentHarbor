//! Bounded ownership for conversations and response resources. Reservations
//! are made before sending a create, and committed before exposing an ID.
use crate::core::protocol::ModelProtocol;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

pub const MAX_BINDINGS: usize = 1024;
pub const MAX_RESOURCES: usize = 1024;
const IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_RESOURCE_ID: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub aggregate_id: String,
    pub member_id: String,
    pub provider_id: String,
    pub actual_model_id: String,
    /// Configured identity, separate from a per-request compatibility plan.
    pub connection_key: String,
    pub configured_protocol: ModelProtocol,
    pub effective_protocol: ModelProtocol,
    pub native_resource: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    Capacity,
    Unknown,
    Conflict,
    InvalidId,
}

struct Entry {
    owner: Owner,
    last_used: Instant,
    conflicted: bool,
}
#[derive(Default)]
struct State {
    bindings: HashMap<String, Entry>,
    resources: HashMap<(String, String), Entry>,
    binding_reservations: usize,
    resource_reservations: usize,
}

#[derive(Clone, Default)]
pub struct Registry {
    state: Arc<Mutex<State>>,
}
pub fn shared() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

/// Hash length-delimited observable identities. No prompt/model-name heuristics
/// are used to invent a client role that the request does not carry.
pub fn scope(fields: &[&str]) -> String {
    let mut hash = Sha256::new();
    for field in fields {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

type Decisions = HashMap<String, Weak<tokio::sync::Mutex<()>>>;
pub async fn decision_lock(key: &str) -> tokio::sync::OwnedMutexGuard<()> {
    static DECISIONS: OnceLock<Mutex<Decisions>> = OnceLock::new();
    let decision = {
        let mut locks = DECISIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        locks.get(key).and_then(Weak::upgrade).unwrap_or_else(|| {
            let decision = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key.to_owned(), Arc::downgrade(&decision));
            decision
        })
    };
    decision.lock_owned().await
}

impl Registry {
    pub fn binding(&self, key: &str) -> Option<Owner> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cleanup(&mut state);
        let entry = state.bindings.get_mut(key)?;
        entry.last_used = Instant::now();
        Some(entry.owner.clone())
    }

    pub fn reserve_binding(&self, key: &str) -> Result<BindingReservation, OwnershipError> {
        if key.len() != 64 {
            return Err(OwnershipError::InvalidId);
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cleanup(&mut state);
        let reserved = !state.bindings.contains_key(key);
        if reserved {
            if state.bindings.len() + state.binding_reservations >= MAX_BINDINGS {
                return Err(OwnershipError::Capacity);
            }
            state.binding_reservations += 1;
        }
        Ok(BindingReservation {
            state: self.state.clone(),
            key: key.into(),
            reserved,
        })
    }

    pub fn reserve_resource(&self, scope: &str) -> Result<ResourceReservation, OwnershipError> {
        if scope.len() != 64 {
            return Err(OwnershipError::InvalidId);
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cleanup(&mut state);
        if state.resources.len() + state.resource_reservations >= MAX_RESOURCES {
            return Err(OwnershipError::Capacity);
        }
        state.resource_reservations += 1;
        Ok(ResourceReservation {
            state: self.state.clone(),
            scope: scope.into(),
            id: None,
            reserved: true,
        })
    }

    pub fn resource(&self, scope: &str, id: &str) -> Result<Owner, OwnershipError> {
        validate_resource_id(id)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cleanup(&mut state);
        let entry = state
            .resources
            .get_mut(&(scope.into(), id.into()))
            .ok_or(OwnershipError::Unknown)?;
        if entry.conflicted {
            return Err(OwnershipError::Conflict);
        }
        entry.last_used = Instant::now();
        Ok(entry.owner.clone())
    }

    pub fn forget_aggregate(&self, id: &str) {
        // Removal does not make an old ID reusable. Keep a tombstone until TTL,
        // so any subsequent lookup still reaches validation of the old owner.
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .bindings
            .retain(|_, entry| entry.owner.aggregate_id != id);
        cleanup(&mut state);
    }
}

pub struct BindingReservation {
    state: Arc<Mutex<State>>,
    key: String,
    reserved: bool,
}
impl BindingReservation {
    pub fn commit(mut self, owner: Owner) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if self.reserved {
            state.binding_reservations = state.binding_reservations.saturating_sub(1);
        }
        self.reserved = false;
        state.bindings.insert(
            self.key.clone(),
            Entry {
                owner,
                last_used: Instant::now(),
                conflicted: false,
            },
        );
    }
}
impl Drop for BindingReservation {
    fn drop(&mut self) {
        if self.reserved {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.binding_reservations = state.binding_reservations.saturating_sub(1);
        }
    }
}

pub struct ResourceReservation {
    state: Arc<Mutex<State>>,
    scope: String,
    id: Option<String>,
    reserved: bool,
}
impl ResourceReservation {
    pub fn register(&mut self, id: &str, owner: &Owner) -> Result<(), OwnershipError> {
        validate_resource_id(id)?;
        if self.id.as_deref().is_some_and(|old| old != id) {
            return Err(OwnershipError::Conflict);
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let key = (self.scope.clone(), id.into());
        if let Some(entry) = state.resources.get_mut(&key) {
            if entry.owner != *owner {
                entry.conflicted = true;
                return Err(OwnershipError::Conflict);
            }
            if entry.conflicted {
                return Err(OwnershipError::Conflict);
            }
            entry.last_used = Instant::now();
        } else {
            if !self.reserved {
                return Err(OwnershipError::Capacity);
            }
            state.resources.insert(
                key,
                Entry {
                    owner: owner.clone(),
                    last_used: Instant::now(),
                    conflicted: false,
                },
            );
        }
        if self.reserved {
            state.resource_reservations = state.resource_reservations.saturating_sub(1);
        }
        self.reserved = false;
        self.id = Some(id.into());
        Ok(())
    }
}
impl Drop for ResourceReservation {
    fn drop(&mut self) {
        if self.reserved {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.resource_reservations = state.resource_reservations.saturating_sub(1);
        }
    }
}

fn validate_resource_id(id: &str) -> Result<(), OwnershipError> {
    if id.is_empty() || id.len() > MAX_RESOURCE_ID || id.chars().any(char::is_control) {
        Err(OwnershipError::InvalidId)
    } else {
        Ok(())
    }
}
fn cleanup(state: &mut State) {
    state
        .bindings
        .retain(|_, entry| entry.last_used.elapsed() < IDLE_TTL);
    state
        .resources
        .retain(|_, entry| entry.last_used.elapsed() < IDLE_TTL);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(member: &str) -> Owner {
        Owner {
            aggregate_id: "pool".into(),
            member_id: member.into(),
            provider_id: member.into(),
            actual_model_id: "wire".into(),
            connection_key: member.into(),
            configured_protocol: ModelProtocol::Responses,
            effective_protocol: ModelProtocol::Responses,
            native_resource: true,
        }
    }
    #[test]
    fn scope_separates_observable_clients_roles_and_targets() {
        let code = scope(&["store", "code", "pool", "session"]);
        assert_ne!(code, scope(&["store", "opencode", "pool", "session"]));
        assert_ne!(code, scope(&["store", "code", "other", "session"]));
        assert_ne!(scope(&["a", "bc"]), scope(&["ab", "c"]));
    }
    #[test]
    fn capacity_is_reserved_before_create_and_cancel_releases_it() {
        let registry = Registry::default();
        let scope = scope(&["resource"]);
        let mut reservations: Vec<_> = (0..MAX_RESOURCES)
            .map(|_| registry.reserve_resource(&scope).unwrap())
            .collect();
        assert!(matches!(
            registry.reserve_resource(&scope),
            Err(OwnershipError::Capacity)
        ));
        reservations.pop();
        let mut last = registry.reserve_resource(&scope).unwrap();
        last.register("resp_1", &owner("a")).unwrap();
        assert_eq!(registry.resource(&scope, "resp_1").unwrap().member_id, "a");
        drop(reservations);
        assert!(registry.reserve_resource(&scope).is_ok());
    }
    #[test]
    fn duplicate_resource_from_other_member_never_overwrites_owner() {
        let registry = Registry::default();
        let scope = scope(&["resource"]);
        registry
            .reserve_resource(&scope)
            .unwrap()
            .register("resp_1", &owner("a"))
            .unwrap();
        assert_eq!(
            registry
                .reserve_resource(&scope)
                .unwrap()
                .register("resp_1", &owner("b")),
            Err(OwnershipError::Conflict)
        );
        assert_eq!(
            registry.resource(&scope, "resp_1"),
            Err(OwnershipError::Conflict)
        );
        assert_eq!(
            Registry::default().resource(&scope, "resp_1"),
            Err(OwnershipError::Unknown)
        );
    }
    #[test]
    fn failed_precheck_does_not_publish_or_replace_binding() {
        let registry = Registry::default();
        let key = scope(&["session"]);
        registry.reserve_binding(&key).unwrap().commit(owner("a"));
        drop(registry.reserve_binding(&key).unwrap());
        assert_eq!(registry.binding(&key).unwrap().member_id, "a");
        for i in 1..MAX_BINDINGS {
            registry
                .reserve_binding(&scope(&[&i.to_string()]))
                .unwrap()
                .commit(owner("a"));
        }
        assert!(matches!(
            registry.reserve_binding(&scope(&["extra"])),
            Err(OwnershipError::Capacity)
        ));
        assert!(registry.reserve_binding(&key).is_ok());
    }
}
