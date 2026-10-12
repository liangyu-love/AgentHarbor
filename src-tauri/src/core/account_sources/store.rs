//! Signed-in accounts of the official account sources, kept encrypted beside
//! the relay store. Tokens leave this module only as request headers.
use super::{chatgpt_api, copilot, cursor, google, grok, kiro, workbuddy, SourceKind};
use crate::core::secret_file;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

const VERSION: u32 = 1;
pub(super) const STORAGE_ERROR: &str = "accountSources.errors.storage";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub version: u32,
    /// This machine's id for Sign in with ChatGPT (`urn:uuid:…`), made once.
    pub host_id: Option<String>,
    pub accounts: Vec<Account>,
    /// Model lists a source fetched for its accounts; WorkBuddy's are built in.
    pub models: BTreeMap<SourceKind, Vec<SourceModel>>,
    /// Models the user left out of a source's share; a model fetched later
    /// is shared until left out too.
    pub hidden_models: BTreeMap<SourceKind, Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    /// The vendor's stable identity (WorkBuddy uid, OpenAI subject): a second
    /// sign-in of the same account renews it instead of adding another.
    pub subject: String,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub active: bool,
    pub credentials: Credentials,
    pub added_at: i64,
    /// Set when the vendor refused a refresh: the account must sign in again.
    #[serde(default)]
    pub lapsed: Option<String>,
    #[serde(default)]
    pub quota: Option<Quota>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Credentials {
    WorkBuddy(workbuddy::Creds),
    ChatgptApi(chatgpt_api::Creds),
    Copilot(copilot::Creds),
    Google(google::Creds),
    Kiro(kiro::Creds),
    Grok(grok::Creds),
    Cursor(cursor::Creds),
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Quota {
    pub used: f64,
    pub limit: f64,
    pub unit: String,
    pub paid: Option<bool>,
    pub checked_at: i64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub reasoning: Vec<String>,
    /// The API this model is asked on, when it differs from its source's.
    pub protocol: Option<crate::core::router::ModelProtocol>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("account-sources.dat")
}

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn guard() -> Result<MutexGuard<'static, ()>, String> {
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| STORAGE_ERROR.to_owned())
}

fn read_unlocked(dir: &Path) -> Result<Store, String> {
    let Some(bytes) = secret_file::read(&path(dir))? else {
        return Ok(Store { version: VERSION, ..Default::default() });
    };
    let store: Store = serde_json::from_slice(&bytes).map_err(|_| STORAGE_ERROR)?;
    if store.version > VERSION {
        return Err(STORAGE_ERROR.into());
    }
    Ok(store)
}

pub fn load(dir: &Path) -> Result<Store, String> {
    let _guard = guard()?;
    read_unlocked(dir)
}

/// Read, change and write under one lock; a failed change writes nothing.
pub fn mutate<T>(dir: &Path, change: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let _guard = guard()?;
    let mut store = read_unlocked(dir)?;
    let result = change(&mut store)?;
    store.version = VERSION;
    let bytes = serde_json::to_vec(&store).map_err(|_| STORAGE_ERROR)?;
    secret_file::write(&path(dir), &bytes)?;
    Ok(result)
}

impl Store {
    pub fn active(&self, kind: SourceKind) -> Option<&Account> {
        self.accounts.iter().find(|account| account.kind == kind && account.active)
            .or_else(|| self.accounts.iter().find(|account| account.kind == kind))
    }

    /// Keep a fresh sign-in: the same vendor identity renews its entry, a new
    /// one is added and becomes active when its source has none yet.
    pub fn keep_sign_in(&mut self, mut account: Account) -> String {
        if let Some(existing) = self.accounts.iter_mut()
            .find(|existing| existing.kind == account.kind && existing.subject == account.subject)
        {
            existing.name = account.name;
            existing.plan = account.plan.or(existing.plan.take());
            existing.credentials = account.credentials;
            existing.lapsed = None;
            return existing.id.clone();
        }
        account.active = !self.accounts.iter().any(|existing| existing.kind == account.kind);
        let id = account.id.clone();
        self.accounts.push(account);
        id
    }

    pub fn activate(&mut self, id: &str) -> Result<(), String> {
        let kind = self.accounts.iter().find(|account| account.id == id)
            .ok_or("accountSources.errors.notFound")?.kind;
        for account in self.accounts.iter_mut().filter(|account| account.kind == kind) {
            account.active = account.id == id;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<Account, String> {
        let index = self.accounts.iter().position(|account| account.id == id)
            .ok_or("accountSources.errors.notFound")?;
        let removed = self.accounts.remove(index);
        if removed.active {
            if let Some(next) = self.accounts.iter_mut().find(|account| account.kind == removed.kind) {
                next.active = true;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(kind: SourceKind, id: &str, subject: &str) -> Account {
        Account {
            id: id.into(), kind, name: format!("{subject}-name"), subject: subject.into(), plan: None,
            active: false, credentials: Credentials::WorkBuddy(workbuddy::Creds::default()), added_at: 1,
            lapsed: Some("old".into()), quota: None,
        }
    }

    #[test]
    fn first_sign_in_of_a_source_is_active_and_the_same_subject_renews() {
        let mut store = Store::default();
        assert_eq!(store.keep_sign_in(account(SourceKind::Workbuddy, "a", "uid-1")), "a");
        assert_eq!(store.keep_sign_in(account(SourceKind::Workbuddy, "b", "uid-2")), "b");
        assert_eq!(store.keep_sign_in(account(SourceKind::WorkbuddyAi, "c", "uid-1")), "c");
        assert!(store.accounts[0].active && !store.accounts[1].active && store.accounts[2].active);
        let mut again = account(SourceKind::Workbuddy, "z", "uid-1");
        again.name = "renamed".into();
        assert_eq!(store.keep_sign_in(again), "a");
        assert_eq!(store.accounts.len(), 3);
        assert_eq!(store.accounts[0].name, "renamed");
        assert!(store.accounts[0].lapsed.is_none());
    }

    #[test]
    fn activation_stays_within_a_source_and_removal_promotes_the_next() {
        let mut store = Store::default();
        for (kind, id, subject) in [(SourceKind::Workbuddy, "a", "1"), (SourceKind::Workbuddy, "b", "2"), (SourceKind::WorkbuddyAi, "c", "3")] {
            store.keep_sign_in(account(kind, id, subject));
        }
        store.activate("b").unwrap();
        assert_eq!(store.active(SourceKind::Workbuddy).unwrap().id, "b");
        assert_eq!(store.active(SourceKind::WorkbuddyAi).unwrap().id, "c");
        store.remove("b").unwrap();
        assert_eq!(store.active(SourceKind::Workbuddy).unwrap().id, "a");
        assert!(store.activate("missing").is_err());
    }

    #[test]
    fn store_round_trips_through_the_protected_file() {
        let dir = std::env::temp_dir().join(format!("agentharbor-sources-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        mutate(&dir, |store| {
            store.keep_sign_in(account(SourceKind::Workbuddy, "a", "uid"));
            Ok(())
        }).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.version, VERSION);
        let raw = std::fs::read(path(&dir)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("uid-name") || cfg!(not(windows)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
