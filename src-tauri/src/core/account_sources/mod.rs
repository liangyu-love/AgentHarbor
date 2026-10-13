//! Official account sources: subscriptions whose own sign-in serves models
//! (WorkBuddy, WorkBuddy AI, ChatGPT through Sign in with ChatGPT, GitHub
//! Copilot, Google's Gemini CLI and Antigravity, Kiro, Grok, Cursor). Each source
//! keeps its accounts here and, when shared with model inject, publishes one
//! router provider whose credentials are the active account's, fetched (and
//! refreshed) per request. Those providers are reachable only through the
//! local router: no client config ever receives an account token.
pub mod chatgpt_api;
mod cli;
pub mod codeassist;
pub mod copilot;
pub mod cursor;
pub mod google;
pub mod grok;
pub mod kiro;
mod loopback;
pub mod store;
pub mod wire;
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
    Copilot,
    GeminiCli,
    Antigravity,
    Kiro,
    Grok,
    Cursor,
}

impl SourceKind {
    pub const ALL: [SourceKind; 9] = [SourceKind::ChatgptApi, SourceKind::Workbuddy, SourceKind::WorkbuddyAi, SourceKind::Copilot,
        SourceKind::GeminiCli, SourceKind::Antigravity, SourceKind::Kiro, SourceKind::Grok, SourceKind::Cursor];

    pub fn provider_id(self) -> &'static str {
        match self {
            SourceKind::ChatgptApi => "account-chatgpt-api",
            SourceKind::Workbuddy => "account-workbuddy",
            SourceKind::WorkbuddyAi => "account-workbuddy-ai",
            SourceKind::Copilot => "account-copilot",
            SourceKind::GeminiCli => "account-gemini-cli",
            SourceKind::Antigravity => "account-antigravity",
            SourceKind::Kiro => "account-kiro",
            SourceKind::Grok => "account-grok",
            SourceKind::Cursor => "account-cursor",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SourceKind::ChatgptApi => "ChatGPT",
            SourceKind::Workbuddy => "WorkBuddy",
            SourceKind::WorkbuddyAi => "WorkBuddy AI",
            SourceKind::Copilot => "GitHub Copilot",
            SourceKind::GeminiCli => "Gemini CLI",
            SourceKind::Antigravity => "Antigravity",
            SourceKind::Kiro => "Kiro",
            SourceKind::Grok => "Grok",
            SourceKind::Cursor => "Cursor",
        }
    }

    /// The source's API; a model may name another (`SourceModel::protocol`).
    pub fn protocol(self) -> router::ModelProtocol {
        match self {
            SourceKind::ChatgptApi | SourceKind::Grok => router::ModelProtocol::Responses,
            // Google's, Kiro's and Cursor's own APIs are spoken through Chat.
            _ => router::ModelProtocol::ChatCompletions,
        }
    }

    fn base_url(self) -> String {
        match self {
            SourceKind::ChatgptApi => chatgpt_api::api_base(),
            SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::site(self).chat_base(),
            SourceKind::Copilot => copilot::api_base(),
            SourceKind::GeminiCli | SourceKind::Antigravity => google::base(google::app_of(self)),
            SourceKind::Kiro => kiro::runtime("us-east-1"),
            SourceKind::Grok => grok::api_base(),
            SourceKind::Cursor => cursor::API.into(),
        }
    }

    /// The source reports an allowance the Accounts page can show.
    fn has_quota(self) -> bool {
        !matches!(self, SourceKind::ChatgptApi)
    }

    /// Where the vendor shows what its API does not report.
    fn usage_url(self) -> Option<&'static str> {
        match self {
            SourceKind::ChatgptApi => Some(chatgpt_api::USAGE_URL),
            SourceKind::Copilot => Some(copilot::SETTINGS_URL),
            SourceKind::Kiro => Some(kiro::USAGE_URL),
            SourceKind::Cursor => Some(cursor::USAGE_URL),
            SourceKind::Workbuddy | SourceKind::WorkbuddyAi | SourceKind::GeminiCli | SourceKind::Antigravity | SourceKind::Grok => None,
        }
    }

    /// Its model list is fetched with an account, not built in.
    fn fetches_models(self) -> bool {
        !matches!(self, SourceKind::Workbuddy | SourceKind::WorkbuddyAi)
    }

    /// What the user should know before signing in.
    fn risk(self) -> Option<&'static str> {
        (self == SourceKind::Antigravity).then_some(google::ANTIGRAVITY_RISK)
    }
}

/// What a sign-in asks of the user: the vendor's page, and the code to type
/// there for a device sign-in.
pub struct SignInPrompt<'a> {
    pub url: &'a str,
    pub code: Option<&'a str>,
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
    /// The Google Cloud project a Gemini CLI or Antigravity account's requests go to.
    pub project: Option<String>,
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
    /// Models left out of the share with model inject.
    pub hidden_models: Vec<String>,
    pub accounts: Vec<AccountView>,
    /// Where the vendor shows an allowance it does not report through its API.
    pub usage_url: Option<&'static str>,
    /// Its accounts report an allowance (`AccountView::quota`).
    pub has_quota: bool,
    /// Its model list is fetched with an account (and can be refreshed).
    pub fetches_models: bool,
    /// A risk to know of before signing in.
    pub risk: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPayload {
    pub sources: Vec<SourceView>,
}

fn source_models(store: &store::Store, kind: SourceKind) -> Vec<SourceModel> {
    match kind {
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::models(kind),
        _ => store.models.get(&kind).cloned().unwrap_or_default(),
    }
}

/// The models a shared source offers: its list without those left out.
fn shared_models(store: &store::Store, kind: SourceKind) -> Vec<SourceModel> {
    let hidden = store.hidden_models.get(&kind);
    source_models(store, kind).into_iter()
        .filter(|model| hidden.is_none_or(|hidden| !hidden.contains(&model.id)))
        .collect()
}

/// Leave models out of a source's share (or bring them back). A shared
/// source keeps at least one.
pub fn set_hidden_models(paths: &CodexPaths, kind: SourceKind, hidden: Vec<String>) -> Result<(), String> {
    let exposed = router::load_store(paths)?.providers.iter().any(|provider| provider.account_source == Some(kind));
    store::mutate(&paths.app_data_dir, |store| {
        let all = source_models(store, kind);
        let mut kept: Vec<String> = all.iter().filter(|model| hidden.contains(&model.id)).map(|model| model.id.clone()).collect();
        kept.dedup();
        if exposed && !all.is_empty() && kept.len() == all.len() {
            return Err("accountSources.errors.noModelsSelected".into());
        }
        if kept.is_empty() {
            store.hidden_models.remove(&kind);
        } else {
            store.hidden_models.insert(kind, kept);
        }
        Ok(())
    })?;
    sync_provider(paths, kind)
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
        hidden_models: accounts.hidden_models.get(&kind).cloned().unwrap_or_default(),
        accounts: accounts.accounts.iter().filter(|account| account.kind == kind).map(|account| AccountView {
            id: account.id.clone(),
            name: account.name.clone(),
            plan: account.plan.clone(),
            project: match &account.credentials {
                Credentials::Google(creds) => Some(creds.project.clone()),
                _ => None,
            },
            active: accounts.active(kind).is_some_and(|active| active.id == account.id),
            lapsed: account.lapsed.clone(),
            quota: account.quota.clone(),
            added_at: account.added_at,
        }).collect(),
        usage_url: kind.usage_url(),
        has_quota: kind.has_quota(),
        fetches_models: kind.fetches_models(),
        risk: kind.risk(),
    }).collect();
    Ok(ListPayload { sources })
}

/// How often a WorkBuddy sign-in asks whether the browser is done.
const POLL_INTERVAL: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(10)
} else {
    std::time::Duration::from_secs(1)
};

/// Run a source's sign-in; `prompt` shows the vendor's page (and a device code).
pub async fn sign_in(
    paths: &CodexPaths,
    client: &Client,
    kind: SourceKind,
    cancel: watch::Receiver<bool>,
    prompt: impl Fn(SignInPrompt<'_>) -> Result<(), String>,
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
            prompt(SignInPrompt { url: &pending.url, code: None })?;
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
            prompt(SignInPrompt { url: &pending.url, code: None })?;
            let done = chatgpt_api::finish_sign_in(client, pending, cancel).await?;
            let subject = if done.creds.subject.is_empty() { done.name.clone() } else { done.creds.subject.clone() };
            Ok(account(done.name, subject, done.plan, Credentials::ChatgptApi(done.creds)))
        }
        SourceKind::Copilot => {
            let code = copilot::begin_sign_in(client).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            prompt(SignInPrompt { url: &code.verification_uri, code: Some(&code.user_code) })?;
            let done = copilot::finish_sign_in(client, code, cancel, POLL_INTERVAL).await?;
            let login = done.creds.login.clone();
            Ok(account(login.clone(), login.to_ascii_lowercase(), done.plan, Credentials::Copilot(done.creds)))
        }
        SourceKind::GeminiCli | SourceKind::Antigravity => {
            let pending = google::begin_sign_in(google::app_of(kind)).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            prompt(SignInPrompt { url: &pending.url, code: None })?;
            let done = google::finish_sign_in(client, pending, cancel).await?;
            let email = done.creds.email.clone();
            let mut kept = account(email.clone(), email.to_ascii_lowercase(), done.plan, Credentials::Google(done.creds));
            // Kept without a project: a Standard account names one afterwards.
            kept.quota = done.note.map(|note| Quota { error: Some(note), checked_at: added_at, ..Default::default() });
            Ok(kept)
        }
        SourceKind::Cursor => {
            let pending = cursor::begin_sign_in(&paths.app_data_dir).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            if let Some(url) = &pending.url {
                prompt(SignInPrompt { url, code: None })?;
            }
            let done = cursor::finish_sign_in(&paths.app_data_dir, pending, cancel).await?;
            let email = done.creds.email.clone();
            Ok(account(email.clone(), email.to_ascii_lowercase(), done.plan, Credentials::Cursor(done.creds)))
        }
        SourceKind::Grok => {
            let pending = grok::begin_sign_in(&paths.app_data_dir).await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            prompt(SignInPrompt { url: &pending.url, code: None })?;
            let done = grok::finish_sign_in(pending, cancel).await?;
            Ok(account(done.email.clone(), done.email.to_ascii_lowercase(), None, Credentials::Grok(done.creds)))
        }
        SourceKind::Kiro => {
            let pending = kiro::begin_sign_in().await?;
            if *cancel.borrow() {
                return Err("accounts.loginCancelled".into());
            }
            prompt(SignInPrompt { url: &pending.url, code: None })?;
            let done = kiro::finish_sign_in(client, pending, cancel).await?;
            let subject = format!("{}|{}", done.creds.profile, done.email.to_ascii_lowercase());
            Ok(account(done.email, subject, done.plan, Credentials::Kiro(done.creds)))
        }
    }
}

/// Keep a finished sign-in. True when it is the first account of its source.
pub fn keep(paths: &CodexPaths, account: Account) -> Result<bool, String> {
    store::mutate(&paths.app_data_dir, |store| {
        if account.kind == SourceKind::Cursor {
            // The CLI keeps one sign-in for the machine: another account
            // signed in there replaces the one kept before.
            store.accounts.retain(|existing| existing.kind != SourceKind::Cursor || existing.subject == account.subject);
        }
        let first = !store.accounts.iter().any(|existing| existing.kind == account.kind);
        store.keep_sign_in(account);
        Ok(first)
    })
}

/// Fill in what a new sign-in can tell: a fetched model list, an allowance;
/// a source's first account is shared with model inject.
pub async fn after_sign_in(paths: &CodexPaths, client: &Client, kind: SourceKind, first: bool) -> Result<(), String> {
    if kind.fetches_models() {
        refresh_models(paths, client, kind).await?;
    }
    if kind.has_quota() {
        if let Some(id) = store::load(&paths.app_data_dir)?.active(kind).map(|account| account.id.clone()) {
            refresh_quota(paths, client, &id).await?;
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
    match &removed.credentials {
        // Signed out here, signed out at OpenAI too; a failure leaves nothing behind locally.
        Credentials::ChatgptApi(creds) => chatgpt_api::revoke(client, creds).await,
        // The Grok home made for it goes with it; the user's own is never touched.
        Credentials::Grok(creds) => grok::forget(&paths.app_data_dir, creds),
        _ => {}
    }
    Ok(())
}

fn to_model(model: &SourceModel) -> newapi::Model {
    newapi::Model {
        id: model.id.clone(),
        display_name: model.name.clone(),
        protocol: model.protocol,
        context_window: model.context_window,
        max_output_tokens: model.max_output_tokens,
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
        // Every model names its API: a source's Claude model is asked on the
        // source's own API, not taken for Anthropic's by its name.
        models: models.iter().map(|model| newapi::Model { protocol: model.protocol.or(Some(kind.protocol())), ..to_model(model) }).collect(),
        catalog_providers: match kind {
            SourceKind::ChatgptApi => vec!["openai".into()],
            SourceKind::Copilot => vec!["github-copilot".into()],
            SourceKind::GeminiCli => vec!["google".into()],
            SourceKind::Antigravity => vec!["google".into(), "anthropic".into()],
            SourceKind::Kiro => vec!["anthropic".into()],
            SourceKind::Grok => vec!["xai".into()],
            SourceKind::Cursor => Vec::new(),
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
        let models = shared_models(&store::load(&paths.app_data_dir)?, kind);
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

/// Fetch a source's model list with its active account (WorkBuddy's are built in).
pub async fn refresh_models(paths: &CodexPaths, client: &Client, kind: SourceKind) -> Result<(), String> {
    if !kind.fetches_models() {
        return sync_provider(paths, kind);
    }
    let auth = credentials(&paths.app_data_dir, client, kind).await.map_err(|(_, message)| message.to_owned())?;
    let models = match &auth.credentials {
        Credentials::ChatgptApi(creds) => {
            let official = super::model_catalog::official(&paths.codex_home, &paths.app_data_dir);
            chatgpt_api::fetch_models(client, creds, &official.models).await?
        }
        Credentials::Copilot(creds) => copilot::fetch_models(client, creds).await?,
        Credentials::Google(creds) => google::fetch_models(client, google::app_of(kind), creds).await?,
        Credentials::Kiro(creds) => kiro::fetch_models(client, creds).await?,
        Credentials::Grok(creds) => {
            let token = grok::token(&paths.app_data_dir, creds).await.map_err(|_| "accountSources.errors.signedOut")?;
            grok::fetch_models(client, &token).await?
        }
        Credentials::Cursor(_) => cursor::fetch_models(&paths.app_data_dir).await?,
        Credentials::WorkBuddy(_) => return Err("accountSources.errors.notFound".into()),
    };
    store::mutate(&paths.app_data_dir, |store| {
        store.models.insert(kind, models);
        Ok(())
    })?;
    sync_provider(paths, kind)
}

/// Read an account's allowance: WorkBuddy's credits, Copilot's premium
/// requests (ChatGPT reports none through its API).
pub async fn refresh_quota(paths: &CodexPaths, client: &Client, id: &str) -> Result<(), String> {
    let account = store::load(&paths.app_data_dir)?.accounts.into_iter().find(|account| account.id == id)
        .ok_or("accountSources.errors.notFound")?;
    let (quota, plan) = match &account.credentials {
        Credentials::WorkBuddy(creds) => {
            let site = workbuddy::site(account.kind);
            let creds = fresh_workbuddy(&paths.app_data_dir, client, &account, creds).await
                .map_err(|(_, message)| message.to_owned())?;
            let quota = workbuddy::quota(client, &site, &creds).await;
            let plan = quota.paid.map(|paid| if paid { "Pro".to_owned() } else { "Free".to_owned() });
            (quota, plan)
        }
        Credentials::Copilot(creds) => (copilot::quota(client, creds).await, None),
        Credentials::Google(creds) => {
            let creds = fresh_google(&paths.app_data_dir, client, &account, creds).await.map_err(|(_, message)| message.to_owned())?;
            (google::quota(client, google::app_of(account.kind), &creds).await, None)
        }
        Credentials::Kiro(creds) => {
            let creds = fresh_kiro(&paths.app_data_dir, client, &account, creds).await.map_err(|(_, message)| message.to_owned())?;
            kiro::quota(client, &creds).await
        }
        Credentials::Grok(creds) => match grok::token(&paths.app_data_dir, creds).await {
            Ok(token) => (grok::quota(client, &token).await, None),
            Err(_) => (Quota { checked_at: chrono::Utc::now().timestamp(), error: Some("Grok is not signed in".into()), ..Default::default() }, None),
        },
        Credentials::Cursor(_) => match cursor::token(&paths.app_data_dir).await {
            Ok(token) => (cursor::quota(client, &token).await, None),
            Err(_) => (Quota { checked_at: chrono::Utc::now().timestamp(), error: Some("Cursor is not signed in".into()), ..Default::default() }, None),
        },
        Credentials::ChatgptApi(_) => return Ok(()),
    };
    store::mutate(&paths.app_data_dir, |store| {
        if let Some(account) = store.accounts.iter_mut().find(|account| account.id == id) {
            account.plan = plan.or(account.plan.take());
            account.quota = Some(quota);
        }
        Ok(())
    })
}

/// Name the Google Cloud project a Gemini CLI (or Antigravity) account's
/// requests go to, set the account up with it and read its models again.
pub async fn set_project(paths: &CodexPaths, client: &Client, id: &str, project: &str) -> Result<(), String> {
    let project = project.trim();
    if project.is_empty() || project.len() > 64 || !project.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == ':' || ch == '.') {
        return Err("accountSources.errors.googleProject".into());
    }
    let account = store::load(&paths.app_data_dir)?.accounts.into_iter().find(|account| account.id == id)
        .ok_or("accountSources.errors.notFound")?;
    let Credentials::Google(creds) = &account.credentials else { return Err("accountSources.errors.notFound".into()) };
    let mut creds = fresh_google(&paths.app_data_dir, client, &account, creds).await.map_err(|(_, message)| message.to_owned())?;
    creds.project = project.to_owned();
    let plan = google::setup(client, google::app_of(account.kind), &mut creds).await?;
    store::mutate(&paths.app_data_dir, |store| {
        if let Some(found) = store.accounts.iter_mut().find(|found| found.id == id) {
            found.credentials = Credentials::Google(creds);
            found.plan = plan.or(found.plan.take());
            found.quota = None;
        }
        Ok(())
    })?;
    if store::load(&paths.app_data_dir)?.active(account.kind).is_some_and(|active| active.id == id) {
        refresh_models(paths, client, account.kind).await?;
    }
    refresh_quota(paths, client, id).await
}

pub type RouteError = (hyper::StatusCode, &'static str);

const NO_ACCOUNT: RouteError = (hyper::StatusCode::UNAUTHORIZED,
    "no signed-in account for this account source; sign in on the Accounts page");
const SIGNED_OUT: RouteError = (hyper::StatusCode::UNAUTHORIZED,
    "the account source's sign-in has expired; sign in again on the Accounts page");
const REFRESH_FAILED: RouteError = (hyper::StatusCode::BAD_GATEWAY,
    "cannot refresh the account source's token; try again");
const STORAGE: RouteError = (hyper::StatusCode::INTERNAL_SERVER_ERROR, "account source storage is unavailable");
const NO_PROJECT: RouteError = (hyper::StatusCode::FAILED_DEPENDENCY,
    "this Google account has no Google Cloud project for Code Assist yet; set one on the Accounts page");

/// One request's upstream for an account source: where to send it and the
/// headers that sign it as the active account.
#[derive(Clone)]
pub struct RequestAuth {
    pub kind: SourceKind,
    pub base: String,
    pub headers: Vec<(&'static str, String)>,
    credentials: Credentials,
    /// A source whose own API is spoken through Chat.
    wire: Option<Wire>,
}

/// What encoding a request on a vendor wire needs to know of the account.
#[derive(Clone)]
enum Wire {
    CodeAssist { app: codeassist::App, project: String, flags: google::Flags },
    /// Cursor's Runs, answered by the source itself (`RequestAuth::exchange`).
    Cursor { token: String, proxy: crate::core::models::ApiProxyConfigPayload },
    Kiro { profile: String, windows: std::collections::HashMap<String, u64> },
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

async fn fresh_google(dir: &Path, client: &Client, account: &Account, creds: &google::Creds) -> Result<google::Creds, RouteError> {
    if !creds.near_expiry() {
        return Ok(creds.clone());
    }
    let _refreshing = REFRESHING.lock().await;
    let current = match reload(dir, &account.id)?.map(|account| account.credentials) {
        Some(Credentials::Google(current)) => current,
        _ => return Err(NO_ACCOUNT),
    };
    if !current.near_expiry() {
        return Ok(current);
    }
    match google::refresh(client, google::app_of(account.kind), &current).await {
        Ok(next) => {
            save_credentials(dir, &account.id, Credentials::Google(next.clone()));
            Ok(next)
        }
        Err(google::RefreshError::SignedOut(reason)) => {
            mark_lapsed(dir, &account.id, &reason);
            Err(SIGNED_OUT)
        }
        Err(google::RefreshError::Transient) => Err(REFRESH_FAILED),
    }
}

async fn fresh_kiro(dir: &Path, client: &Client, account: &Account, creds: &kiro::Creds) -> Result<kiro::Creds, RouteError> {
    if !creds.near_expiry() {
        return Ok(creds.clone());
    }
    let _refreshing = REFRESHING.lock().await;
    let current = match reload(dir, &account.id)?.map(|account| account.credentials) {
        Some(Credentials::Kiro(current)) => current,
        _ => return Err(NO_ACCOUNT),
    };
    if !current.near_expiry() {
        return Ok(current);
    }
    match kiro::refresh(client, &current).await {
        Ok(next) => {
            save_credentials(dir, &account.id, Credentials::Kiro(next.clone()));
            Ok(next)
        }
        Err(kiro::RefreshError::SignedOut(reason)) => {
            mark_lapsed(dir, &account.id, &reason);
            Err(SIGNED_OUT)
        }
        // The token in hand may still be accepted.
        Err(kiro::RefreshError::Transient) if current.expires_at > chrono::Utc::now().timestamp() => Ok(current),
        Err(kiro::RefreshError::Transient) => Err(REFRESH_FAILED),
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
                credentials: Credentials::WorkBuddy(creds), wire: None })
        }
        Credentials::ChatgptApi(creds) => {
            let creds = fresh_chatgpt(dir, client, &account, creds).await?;
            Ok(RequestAuth { kind, base: chatgpt_api::api_base(),
                headers: vec![("authorization", format!("Bearer {}", creds.access))],
                credentials: Credentials::ChatgptApi(creds), wire: None })
        }
        Credentials::Copilot(creds) => match copilot::session(client, creds).await {
            Ok(session) => Ok(RequestAuth { kind, base: session.api.clone(), headers: copilot::request_headers(&session),
                credentials: Credentials::Copilot(creds.clone()), wire: None }),
            Err(copilot::SessionError::SignedOut(reason)) => {
                mark_lapsed(dir, &account.id, &reason);
                Err(SIGNED_OUT)
            }
            Err(copilot::SessionError::Transient) => Err(REFRESH_FAILED),
        },
        Credentials::Google(creds) => {
            let creds = fresh_google(dir, client, &account, creds).await?;
            if creds.project.is_empty() {
                return Err(NO_PROJECT);
            }
            let app = google::app_of(kind);
            let mut headers = vec![("authorization", format!("Bearer {}", creds.access))];
            if app == codeassist::App::Antigravity {
                headers.push(("user-agent", google::user_agent(client, app, "").await));
            }
            let flags = if app == codeassist::App::GeminiCli { google::flags(client, &creds).await } else { google::Flags::default() };
            Ok(RequestAuth { kind, base: google::base(app), headers,
                wire: Some(Wire::CodeAssist { app, project: creds.project.clone(), flags }), credentials: Credentials::Google(creds) })
        }
        Credentials::Cursor(creds) => match cursor::token(dir).await {
            Ok(token) => {
                let proxy = newapi::proxy_from_settings(&dir.join("settings.json"));
                let base = cursor::agent_url_with(&proxy, &token).await;
                Ok(RequestAuth { kind, base, headers: Vec::new(), credentials: Credentials::Cursor(creds.clone()),
                    wire: Some(Wire::Cursor { token, proxy }) })
            }
            Err(cursor::TokenError::SignedOut(reason)) => {
                mark_lapsed(dir, &account.id, &reason);
                Err(SIGNED_OUT)
            }
            Err(cursor::TokenError::Transient) => Err(REFRESH_FAILED),
        },
        Credentials::Grok(creds) => match grok::token(dir, creds).await {
            Ok(token) => Ok(RequestAuth { kind, base: grok::api_base(), headers: grok::request_headers(&token).await,
                credentials: Credentials::Grok(creds.clone()), wire: None }),
            Err(grok::TokenError::SignedOut(reason)) => {
                mark_lapsed(dir, &account.id, &reason);
                Err(SIGNED_OUT)
            }
            Err(grok::TokenError::Transient) => Err(REFRESH_FAILED),
        },
        Credentials::Kiro(creds) => {
            let creds = fresh_kiro(dir, client, &account, creds).await?;
            let windows = store.models.get(&kind).into_iter().flatten()
                .filter_map(|model| model.context_window.map(|window| (model.id.clone(), window))).collect();
            Ok(RequestAuth { kind, base: kiro::runtime(&creds.api_region()), headers: kiro::request_headers(&creds),
                wire: Some(Wire::Kiro { profile: creds.profile.clone(), windows }), credentials: Credentials::Kiro(creds) })
        }
    }
}

impl RequestAuth {
    /// Whether this source answers `suffix` on `protocol`: inference, and
    /// ChatGPT's model list.
    pub fn serves(&self, protocol: router::ModelProtocol, suffix: &str) -> bool {
        use router::ModelProtocol::*;
        match (self.kind, protocol) {
            (SourceKind::ChatgptApi, Responses) => suffix == "responses" || suffix == "models",
            (SourceKind::ChatgptApi, _) => false,
            (_, ChatCompletions) => suffix == "chat/completions",
            (SourceKind::Copilot, Responses) => suffix == "responses",
            (SourceKind::Copilot, AnthropicMessages) => suffix == "messages",
            (SourceKind::Grok, Responses) => suffix == "responses",
            _ => false,
        }
    }

    /// The path below `base` that `suffix` is asked at.
    pub fn path(&self, suffix: &str) -> String {
        match (self.kind, suffix) {
            (SourceKind::Copilot, "messages") => "v1/messages".into(),
            _ => suffix.to_owned(),
        }
    }

    /// A Chat request in the vendor's own shape, for a source spoken through
    /// Chat (`wire`); `None` for any other source.
    pub fn encode(&self, protocol: router::ModelProtocol, body: &[u8], model: &str) -> Option<Result<wire::Encoded, RouteError>> {
        let wire = self.wire.as_ref().filter(|_| protocol == router::ModelProtocol::ChatCompletions)?;
        if matches!(wire, Wire::Cursor { .. }) {
            return None;
        }
        let Ok(chat) = serde_json::from_slice::<serde_json::Value>(body) else {
            return Some(Err((hyper::StatusCode::BAD_REQUEST, "invalid chat request")));
        };
        let model = chat["model"].as_str().filter(|model| !model.is_empty()).unwrap_or(model).to_owned();
        let stream = chat["stream"].as_bool().unwrap_or(false);
        Some(Ok(match wire {
            Wire::CodeAssist { app, project, flags } => {
                let sent = if *app == codeassist::App::GeminiCli { flags.wire(&model) } else { model.clone() };
                let request = codeassist::gemini_request(&chat, &sent, *app);
                let mut headers = vec![("content-type", "application/json".to_owned()), ("accept", "text/event-stream".to_owned())];
                if *app == codeassist::App::GeminiCli {
                    let (os, arch) = (if cfg!(windows) { "win32" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" },
                        if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" });
                    headers.push(("user-agent", format!("GeminiCLI/0.61.0/{model} ({os}; {arch}; terminal)")));
                }
                wire::Encoded {
                    body: codeassist::envelope(request, &sent, project, *app).to_string().into_bytes(),
                    path: "v1internal:streamGenerateContent?alt=sse".into(),
                    headers,
                    decoder: wire::Decoder::new(wire::Codec::CodeAssist(codeassist::Decoder::default()), &model, stream),
                }
            }
            Wire::Cursor { .. } => unreachable!("Cursor answers through its exchange"),
            Wire::Kiro { profile, windows } => {
                let (body, names, thinking) = kiro::kiro_request(&chat, &model, profile);
                wire::Encoded {
                    body: body.to_string().into_bytes(),
                    path: "generateAssistantResponse".into(),
                    headers: vec![("content-type", "application/json".to_owned())],
                    decoder: wire::Decoder::new(wire::Codec::Kiro(kiro::Decoder::new(names, thinking, windows.get(&model).copied())), &model, stream),
                }
            }
        }))
    }

    /// A Chat request a source answers itself, over a stream both ways (Cursor).
    pub fn exchange(&self, protocol: router::ModelProtocol, body: &[u8], model: &str) -> Option<Result<cursor::Exchange, RouteError>> {
        match &self.wire {
            Some(Wire::Cursor { token, proxy }) if protocol == router::ModelProtocol::ChatCompletions => {
                Some(cursor::exchange(proxy, token, &self.base, body, model))
            }
            _ => None,
        }
    }

    /// Headers that depend on the request body.
    pub fn body_headers(&self, protocol: router::ModelProtocol, body: &[u8]) -> Vec<(&'static str, String)> {
        match self.kind {
            SourceKind::Copilot => copilot::body_headers(protocol, body),
            SourceKind::Grok if protocol == router::ModelProtocol::Responses => grok::body_headers(body),
            _ => Vec::new(),
        }
    }

    /// The request body as the source takes it, `None` when unchanged.
    pub fn shape_body(&self, protocol: router::ModelProtocol, body: &[u8]) -> Option<Vec<u8>> {
        shape_body_for_source(self.kind,protocol,body)
    }

    /// What reads the reply of this request, given before it was shaped.
    pub fn reply_fix(&self, protocol: router::ModelProtocol, body: &[u8]) -> Option<ReplyFix> {
        (self.kind == SourceKind::Grok && protocol == router::ModelProtocol::Responses)
            .then(|| ReplyFix::Grok(grok::reply_fix(body)))
    }
}

/// Pure source contract checks do not require loading or refreshing credentials.
pub(crate) fn shape_body_for_source(kind: SourceKind, protocol: router::ModelProtocol, body: &[u8]) -> Option<Vec<u8>> {
    match (kind,protocol) {
        (SourceKind::Workbuddy | SourceKind::WorkbuddyAi, router::ModelProtocol::ChatCompletions) => workbuddy::shape_chat_body(body),
        (SourceKind::ChatgptApi, router::ModelProtocol::Responses) => chatgpt_api::shape_body(body),
        (SourceKind::Grok, router::ModelProtocol::Responses) => grok::shape_body(body),
        _ => None,
    }
}

/// A source's reply turned into what the client takes.
pub enum ReplyFix {
    Grok(grok::ReplyFix),
}

impl ReplyFix {
    pub async fn apply(self, response: reqwest::Response) -> reqwest::Response {
        match self {
            ReplyFix::Grok(fix) => fix.apply(response).await,
        }
    }
}

/// What the user can do about an upstream refusal from this source.
pub fn explain(kind: SourceKind, status: u16, body: &[u8]) -> Option<String> {
    match kind {
        SourceKind::Workbuddy | SourceKind::WorkbuddyAi => workbuddy::refused(status, body).then(|| workbuddy::REFUSED_HINT.to_owned()),
        SourceKind::ChatgptApi => (status >= 400).then(|| chatgpt_api::explain(status, body)).flatten(),
        SourceKind::Copilot => copilot::explain(status, body),
        SourceKind::GeminiCli | SourceKind::Antigravity => codeassist::explain(google::app_of(kind), status, body),
        SourceKind::Kiro => kiro::explain(status, body),
        SourceKind::Grok => grok::explain(status, body),
        SourceKind::Cursor => cursor::explain(status, body),
    }
}

#[cfg(test)]
pub(crate) mod tests;
