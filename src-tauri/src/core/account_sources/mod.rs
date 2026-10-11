//! Official account sources: subscriptions whose own sign-in serves models
//! (WorkBuddy, WorkBuddy AI, ChatGPT through Sign in with ChatGPT). Each source
//! keeps its accounts here and, when shared with model inject, publishes one
//! router provider whose credentials are the active account's, fetched (and
//! refreshed) per request. Those providers are reachable only through the
//! local router: no client config ever receives an account token.
pub mod chatgpt_api;
pub mod store;
pub mod workbuddy;

use super::{newapi, router};
use crate::platform::paths::CodexPaths;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use store::{Account, Credentials, Quota, SourceModel};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    ChatgptApi,
    Workbuddy,
    WorkbuddyAi,
}

impl SourceKind {
    pub const ALL: [SourceKind; 3] = [SourceKind::ChatgptApi, SourceKind::Workbuddy, SourceKind::WorkbuddyAi];

    pub fn provider_id(self) -> &'static str {
        match self {
            SourceKind::ChatgptApi => "account-chatgpt-api",
            SourceKind::Workbuddy => "account-workbuddy",
            SourceKind::WorkbuddyAi => "account-workbuddy-ai",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SourceKind::ChatgptApi => "ChatGPT",
            SourceKind::Workbuddy => "WorkBuddy",
            SourceKind::WorkbuddyAi => "WorkBuddy AI",
        }
    }

    pub fn protocol(self) -> router::ModelProtocol {
        match self {
            SourceKind::ChatgptApi => router::ModelProtocol::Responses,
            SourceKind::Workbuddy | SourceKind::WorkbuddyAi => router::ModelProtocol::ChatCompletions,
        }
    }

    fn base_url(self) -> String {
        match self {
            SourceKind::ChatgptApi => chatgpt_api::api_base(),
            SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::site(self).chat_base(),
        }
    }
}

#[cfg(test)]
static ENDPOINT_OVERRIDES: std::sync::Mutex<Vec<(SourceKind, String)>> = std::sync::Mutex::new(Vec::new());

/// Tests point a source at a loopback upstream.
#[cfg(test)]
pub(crate) fn set_endpoint_override(kind: SourceKind, base: Option<String>) {
    let mut overrides = ENDPOINT_OVERRIDES.lock().unwrap();
    overrides.retain(|(existing, _)| *existing != kind);
    if let Some(base) = base {
        overrides.push((kind, base));
    }
}

#[cfg(test)]
fn endpoint_override(kind: SourceKind) -> Option<String> {
    ENDPOINT_OVERRIDES.lock().unwrap().iter().find(|(existing, _)| *existing == kind).map(|(_, base)| base.clone())
}

#[cfg(not(test))]
fn endpoint_override(_kind: SourceKind) -> Option<String> {
    None
}

/// Resolves once the sign-in is cancelled; never when the sender is dropped.
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub name: String,
    pub plan: Option<String>,
    pub active: bool,
    pub lapsed: Option<String>,
    pub quota: Option<Quota>,
    pub added_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub kind: SourceKind,
    pub name: &'static str,
    pub provider_id: &'static str,
    pub protocol: router::ModelProtocol,
    /// Shared with model inject: its router provider exists.
    pub exposed: bool,
    pub models: Vec<SourceModel>,
    pub accounts: Vec<AccountView>,
    /// Where the vendor shows an allowance it does not report through its API.
    pub usage_url: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPayload {
    pub sources: Vec<SourceView>,
}

fn source_models(store: &store::Store, kind: SourceKind) -> Vec<SourceModel> {
    match kind {
        SourceKind::ChatgptApi => store.models.get(&kind).cloned().unwrap_or_default(),
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::models(kind),
    }
}

pub fn list(paths: &CodexPaths) -> Result<ListPayload, String> {
    let accounts = store::load(&paths.app_data_dir)?;
    let routes = router::load_store(paths)?;
    let sources = SourceKind::ALL.into_iter().map(|kind| SourceView {
        kind,
        name: kind.display_name(),
        provider_id: kind.provider_id(),
        protocol: kind.protocol(),
        exposed: routes.providers.iter().any(|provider| provider.account_source == Some(kind)),
        models: source_models(&accounts, kind),
        accounts: accounts.accounts.iter().filter(|account| account.kind == kind).map(|account| AccountView {
            id: account.id.clone(),
            name: account.name.clone(),
            plan: account.plan.clone(),
            active: accounts.active(kind).is_some_and(|active| active.id == account.id),
            lapsed: account.lapsed.clone(),
            quota: account.quota.clone(),
            added_at: account.added_at,
        }).collect(),
        usage_url: (kind == SourceKind::ChatgptApi).then_some(chatgpt_api::USAGE_URL),
    }).collect();
    Ok(ListPayload { sources })
}

/// How often a WorkBuddy sign-in asks whether the browser is done.
const POLL_INTERVAL: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(10)
} else {
    std::time::Duration::from_secs(1)
};

/// Run a source's browser sign-in; `open` shows the vendor's page.
pub async fn sign_in(
    paths: &CodexPaths,
    client: &Client,
    kind: SourceKind,
    cancel: watch::Receiver<bool>,
    open: impl Fn(&str) -> Result<(), String>,
) -> Result<Account, String> {
    let added_at = chrono::Utc::now().timestamp();
    let account = |name: String, subject: String, plan: Option<String>, credentials: Credentials| Account {
        id: uuid::Uuid::new_v4().to_string(), kind, name, subject, plan, active: false, credentials,
        added_at, lapsed: None, quota: None,
    };
    match kind {
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => {
            let site = workbuddy::site(kind);
            let pending = workbuddy::begin_sign_in(client, &site).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            open(&pending.url)?;
            let done = workbuddy::finish_sign_in(client, &site, pending, cancel, POLL_INTERVAL).await?;
            Ok(account(done.name, done.creds.uid.clone(), None, Credentials::WorkBuddy(done.creds)))
        }
        SourceKind::ChatgptApi => {
            let host_id = store::mutate(&paths.app_data_dir, |store| {
                Ok(store.host_id.get_or_insert_with(chatgpt_api::new_host_id).clone())
            })?;
            let pending = chatgpt_api::begin_sign_in(&host_id).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            open(&pending.url)?;
            let done = chatgpt_api::finish_sign_in(client, pending, cancel).await?;
            let subject = if done.creds.subject.is_empty() { done.name.clone() } else { done.creds.subject.clone() };
            Ok(account(done.name, subject, done.plan, Credentials::ChatgptApi(done.creds)))
        }
    }
}

/// Keep a finished sign-in. True when it is the first account of its source.
pub fn keep(paths: &CodexPaths, account: Account) -> Result<bool, String> {
    store::mutate(&paths.app_data_dir, |store| {
        let first = !store.accounts.iter().any(|existing| existing.kind == account.kind);
        store.keep_sign_in(account);
        Ok(first)
    })
}

/// Fill in what a new sign-in can tell: ChatGPT's model list, WorkBuddy's
/// credits; a source's first account is shared with model inject.
pub async fn after_sign_in(paths: &CodexPaths, client: &Client, kind: SourceKind, first: bool) -> Result<(), String> {
    match kind {
        SourceKind::ChatgptApi => refresh_models(paths, client, kind).await?,
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => {
            if let Some(id) = store::load(&paths.app_data_dir)?.active(kind).map(|account| account.id.clone()) {
                refresh_quota(paths, client, &id).await?;
            }
        }
    }
    if first {
        set_exposed(paths, kind, true)?;
    } else {
        sync_provider(paths, kind)?;
    }
    Ok(())
}

pub fn activate(paths: &CodexPaths, id: &str) -> Result<(), String> {
    store::mutate(&paths.app_data_dir, |store| store.activate(id))
}

pub async fn remove(paths: &CodexPaths, client: &Client, id: &str) -> Result<(), String> {
    let removed = store::mutate(&paths.app_data_dir, |store| store.remove(id))?;
    if let Credentials::ChatgptApi(creds) = &removed.credentials {
        // Signed out here, signed out at OpenAI too; a failure leaves nothing behind locally.
        chatgpt_api::revoke(client, creds).await;
    }
    Ok(())
}

fn to_model(model: &SourceModel) -> newapi::Model {
    newapi::Model {
        id: model.id.clone(),
        display_name: model.name.clone(),
        context_window: model.context_window,
        included: true,
        metadata_source: "upstream".into(),
        supported_reasoning_levels: (!model.reasoning.is_empty()).then(|| model.reasoning.iter()
            .map(|effort| newapi::ReasoningLevel { effort: effort.clone(), description: String::new() }).collect()),
        ..Default::default()
    }
}

fn provider_for(kind: SourceKind, models: &[SourceModel]) -> Result<router::Provider, String> {
    let first = models.first().ok_or("accountSources.errors.noModels")?;
    Ok(router::Provider {
        id: kind.provider_id().into(),
        name: kind.display_name().into(),
        base_url: kind.base_url(),
        protocol: kind.protocol(),
        model: first.id.clone(),
        models: models.iter().map(to_model).collect(),
        catalog_providers: match kind {
            SourceKind::ChatgptApi => vec!["openai".into()],
            SourceKind::Workbuddy | SourceKind::WorkbuddyAi => Vec::new(),
        },
        enabled: true,
        network_mode: "proxy".into(),
        test_status: "untested".into(),
        account_source: Some(kind),
        ..Default::default()
    })
}

/// Share a source with model inject (publish its router provider) or stop.
pub fn set_exposed(paths: &CodexPaths, kind: SourceKind, exposed: bool) -> Result<(), String> {
    if exposed {
        let models = source_models(&store::load(&paths.app_data_dir)?, kind);
        router::upsert_account_provider(paths, provider_for(kind, &models)?)?;
    } else if router::load_store(paths)?.providers.iter().any(|provider| provider.id == kind.provider_id()) {
        router::delete_provider(paths, kind.provider_id())?;
    }
    Ok(())
}

/// Bring a shared source's provider up to date with its model list.
fn sync_provider(paths: &CodexPaths, kind: SourceKind) -> Result<(), String> {
    if router::load_store(paths)?.providers.iter().any(|provider| provider.account_source == Some(kind)) {
        set_exposed(paths, kind, true)?;
    }
    Ok(())
}

/// Fetch a source's model list with its active account (ChatGPT only; WorkBuddy's are built in).
pub async fn refresh_models(paths: &CodexPaths, client: &Client, kind: SourceKind) -> Result<(), String> {
    if kind != SourceKind::ChatgptApi {
        return sync_provider(paths, kind);
    }
    let auth = credentials(&paths.app_data_dir, client, kind).await.map_err(|(_, message)| message.to_owned())?;
    let Credentials::ChatgptApi(creds) = auth.credentials else { return Err("accountSources.errors.notFound".into()) };
    let official = super::model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let models = chatgpt_api::fetch_models(client, &creds, &official.models).await?;
    store::mutate(&paths.app_data_dir, |store| {
        store.models.insert(kind, models);
        Ok(())
    })?;
    sync_provider(paths, kind)
}

/// Read a WorkBuddy account's credits (ChatGPT reports none through its API).
pub async fn refresh_quota(paths: &CodexPaths, client: &Client, id: &str) -> Result<(), String> {
    let account = store::load(&paths.app_data_dir)?.accounts.into_iter().find(|account| account.id == id)
        .ok_or("accountSources.errors.notFound")?;
    let Credentials::WorkBuddy(creds) = &account.credentials else { return Ok(()) };
    let site = workbuddy::site(account.kind);
    let creds = fresh_workbuddy(&paths.app_data_dir, client, &account, creds).await
        .map_err(|(_, message)| message.to_owned())?;
    let quota = workbuddy::quota(client, &site, &creds).await;
    store::mutate(&paths.app_data_dir, |store| {
        if let Some(account) = store.accounts.iter_mut().find(|account| account.id == id) {
            account.plan = quota.paid.map(|paid| if paid { "Pro".into() } else { "Free".into() }).or(account.plan.take());
            account.quota = Some(quota);
        }
        Ok(())
    })
}

pub type RouteError = (hyper::StatusCode, &'static str);

const NO_ACCOUNT: RouteError = (hyper::StatusCode::UNAUTHORIZED,
    "no signed-in account for this account source; sign in on the Accounts page");
const SIGNED_OUT: RouteError = (hyper::StatusCode::UNAUTHORIZED,
    "the account source's sign-in has expired; sign in again on the Accounts page");
const REFRESH_FAILED: RouteError = (hyper::StatusCode::BAD_GATEWAY,
    "cannot refresh the account source's token; try again");
const STORAGE: RouteError = (hyper::StatusCode::INTERNAL_SERVER_ERROR, "account source storage is unavailable");

/// One request's upstream for an account source: where to send it and the
/// headers that sign it as the active account.
pub struct RequestAuth {
    pub kind: SourceKind,
    pub base: String,
    pub headers: Vec<(&'static str, String)>,
    credentials: Credentials,
}

/// Serialises refreshes: tokens rotate, so two concurrent refreshes would
/// spend one refresh token twice.
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn mark_lapsed(dir: &Path, id: &str, reason: &str) {
    let _ = store::mutate(dir, |store| {
        if let Some(account) = store.accounts.iter_mut().find(|account| account.id == id) {
            account.lapsed = Some(reason.to_owned());
        }
        Ok(())
    });
}

fn save_credentials(dir: &Path, id: &str, credentials: Credentials) {
    let _ = store::mutate(dir, |store| {
        if let Some(account) = store.accounts.iter_mut().find(|account| account.id == id) {
            account.credentials = credentials;
            account.lapsed = None;
        }
        Ok(())
    });
}

/// The stored account again, after waiting for any refresh in flight.
fn reload(dir: &Path, id: &str) -> Result<Option<Account>, RouteError> {
    Ok(store::load(dir).map_err(|_| STORAGE)?.accounts.into_iter().find(|account| account.id == id))
}

async fn fresh_workbuddy(dir: &Path, client: &Client, account: &Account, creds: &workbuddy::Creds) -> Result<workbuddy::Creds, RouteError> {
    if !workbuddy::near_expiry(creds) {
        return Ok(creds.clone());
    }
    let _refreshing = REFRESHING.lock().await;
    let current = match reload(dir, &account.id)?.map(|account| account.credentials) {
        Some(Credentials::WorkBuddy(current)) => current,
        _ => return Err(NO_ACCOUNT),
    };
    if !workbuddy::near_expiry(&current) {
        return Ok(current);
    }
    match workbuddy::refresh(client, &workbuddy::site(account.kind), &current).await {
        Ok(next) => {
            save_credentials(dir, &account.id, Credentials::WorkBuddy(next.clone()));
            Ok(next)
        }
        Err(workbuddy::RefreshError::SignedOut) => {
            mark_lapsed(dir, &account.id, "signed out");
            Err(SIGNED_OUT)
        }
        // The token in hand may still be accepted.
        Err(workbuddy::RefreshError::Transient(_)) if !current.access.is_empty() => Ok(current),
        Err(workbuddy::RefreshError::Transient(_)) => Err(REFRESH_FAILED),
    }
}

async fn fresh_chatgpt(dir: &Path, client: &Client, account: &Account, creds: &chatgpt_api::Creds) -> Result<chatgpt_api::Creds, RouteError> {
    if !creds.near_expiry() {
        return Ok(creds.clone());
    }
    let _refreshing = REFRESHING.lock().await;
    let current = match reload(dir, &account.id)?.map(|account| account.credentials) {
        Some(Credentials::ChatgptApi(current)) => current,
        _ => return Err(NO_ACCOUNT),
    };
    if !current.near_expiry() {
        return Ok(current);
    }
    match chatgpt_api::refresh(client, &current).await {
        Ok(next) => {
            save_credentials(dir, &account.id, Credentials::ChatgptApi(next.clone()));
            Ok(next)
        }
        Err(chatgpt_api::RefreshError::SignedOut(reason)) => {
            mark_lapsed(dir, &account.id, &reason);
            Err(SIGNED_OUT)
        }
        Err(chatgpt_api::RefreshError::Transient(_)) if current.expires_at > chrono::Utc::now().timestamp() => Ok(current),
        Err(chatgpt_api::RefreshError::Transient(_)) => Err(REFRESH_FAILED),
    }
}

/// The active account's fresh credentials, refreshed when about to lapse.
pub async fn credentials(dir: &Path, client: &Client, kind: SourceKind) -> Result<RequestAuth, RouteError> {
    let store = store::load(dir).map_err(|_| STORAGE)?;
    let account = store.active(kind).cloned().ok_or(NO_ACCOUNT)?;
    if account.lapsed.is_some() {
        return Err(SIGNED_OUT);
    }
    match &account.credentials {
        Credentials::WorkBuddy(creds) => {
            let creds = fresh_workbuddy(dir, client, &account, creds).await?;
            let site = workbuddy::site(kind);
            Ok(RequestAuth { kind, base: site.chat_base(), headers: workbuddy::auth_headers(&site, &creds),
                credentials: Credentials::WorkBuddy(creds) })
        }
        Credentials::ChatgptApi(creds) => {
            let creds = fresh_chatgpt(dir, client, &account, creds).await?;
            Ok(RequestAuth { kind, base: chatgpt_api::api_base(),
                headers: vec![("authorization", format!("Bearer {}", creds.access))],
                credentials: Credentials::ChatgptApi(creds) })
        }
    }
}

impl RequestAuth {
    /// The request body as the source takes it, `None` when unchanged.
    pub fn shape_body(&self, protocol: router::ModelProtocol, body: &[u8]) -> Option<Vec<u8>> {
        match (self.kind, protocol) {
            (SourceKind::Workbuddy | SourceKind::WorkbuddyAi, router::ModelProtocol::ChatCompletions) => workbuddy::shape_chat_body(body),
            (SourceKind::ChatgptApi, router::ModelProtocol::Responses) => chatgpt_api::shape_body(body),
            _ => None,
        }
    }
}

/// What the user can do about an upstream refusal from this source.
pub fn explain(kind: SourceKind, status: u16, body: &[u8]) -> Option<String> {
    match kind {
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::refused(status, body).then(|| workbuddy::REFUSED_HINT.to_owned()),
        SourceKind::ChatgptApi => (status >= 400).then(|| chatgpt_api::explain(status, body)).flatten(),
    }
}

#[cfg(test)]
pub(crate) mod tests;
