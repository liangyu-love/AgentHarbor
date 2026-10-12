//! Independent, recoverable ownership of the Windows user's proxy environment.
//! No process-global environment changes: the app's API proxy remains separate.
use super::{
    api_client,
    models::{ApiProxyConfigPayload, ApiProxyMode},
    secret_file,
};
use crate::platform::{
    paths::CodexPaths,
    terminal_environment::{Environment, EnvironmentValue},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

const NAMES: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
const MAX_BYPASS_BYTES: usize = 4_096;
// Older journals can exceed the input limit by the three appended loopback
// entries and their commas. Keep those readable so their leases can be restored.
const MAX_STORED_BYPASS_BYTES: usize = MAX_BYPASS_BYTES + 24;
// Twelve bounded raw values (four originals plus two owned generations) can
// expand to over 1 MiB when JSON encodes bytes as decimal array entries.
const MAX_JOURNAL_BYTES: u64 = 4 * 1_048_576;
fn guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "settings.terminalProxy.errors.storage".into())
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    url: Option<String>,
    no_proxy: String,
    lease: Option<Lease>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lease {
    phase: Phase,
    entries: Vec<Entry>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Applying,
    Active,
    Restoring,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    original: Option<EnvironmentValue>,
    owned: Vec<EnvironmentValue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProxyStatus {
    supported: bool,
    enabled: bool,
    applied: bool,
    url: Option<String>,
    no_proxy: String,
    recovery_required: bool,
    conflicts: Vec<String>,
    preserved_variables: Vec<String>,
    notification_sent: bool,
}

pub fn normalize_url(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.len() > 2_048 || text.chars().any(char::is_control) {
        return Err("settings.terminalProxy.errors.url".into());
    }
    let url = reqwest::Url::parse(text).map_err(|_| "settings.terminalProxy.errors.url")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("settings.terminalProxy.errors.scheme".into());
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err("settings.terminalProxy.errors.url".into());
    }
    Ok(url.to_string())
}

fn normalize_bypass(text: &str) -> Result<String, String> {
    normalize_bypass_with_limit(text, MAX_BYPASS_BYTES)
}

fn normalize_bypass_with_limit(text: &str, limit: usize) -> Result<String, String> {
    if text.len() > limit
        || text
            .chars()
            .any(|ch| ch.is_control() && !ch.is_whitespace())
    {
        return Err("settings.terminalProxy.errors.bypass".into());
    }
    let mut values = Vec::<String>::new();
    for value in text
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|value| !value.is_empty())
        .chain(LOOPBACK)
    {
        if !values
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(value))
        {
            values.push(value.to_owned());
        }
    }
    let normalized = values.join(",");
    if normalized.len() > limit {
        return Err("settings.terminalProxy.errors.bypass".into());
    }
    Ok(normalized)
}

fn load(path: &Path) -> Result<Settings, String> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_JOURNAL_BYTES) {
        return Err("settings.terminalProxy.errors.storage".into());
    }
    let settings: Settings =
        match secret_file::read(path).map_err(|_| "settings.terminalProxy.errors.storage")? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "settings.terminalProxy.errors.storage")?,
            None => Settings::default(),
        };
    // Corrupt ownership records must never name unrelated registry values.
    if settings
        .url
        .as_deref()
        .is_some_and(|url| normalize_url(url).is_err())
        || normalize_bypass_with_limit(&settings.no_proxy, MAX_STORED_BYPASS_BYTES).is_err()
        || settings.lease.as_ref().is_some_and(|lease| {
            lease.entries.len() != NAMES.len()
                || NAMES.iter().any(|name| {
                    lease
                        .entries
                        .iter()
                        .filter(|entry| entry.name == *name)
                        .count()
                        != 1
                })
                || lease.entries.iter().any(|entry| {
                    entry.owned.is_empty()
                        || entry.owned.len() > 2
                        || entry
                            .original
                            .iter()
                            .chain(entry.owned.iter())
                            .any(|value| value.bytes.len() > 65_536)
                })
        })
    {
        return Err("settings.terminalProxy.errors.storage".into());
    }
    Ok(settings)
}

fn persist(path: &Path, settings: &Settings) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(settings).map_err(|_| "settings.terminalProxy.errors.storage")?;
    secret_file::write(path, &bytes).map_err(|_| "settings.terminalProxy.errors.storage".into())
}

fn status(settings: &Settings, env: &mut impl Environment) -> Result<TerminalProxyStatus, String> {
    let mut conflicts = Vec::new();
    if let Some(lease) = &settings.lease {
        for entry in &lease.entries {
            if env.read(&entry.name)?.as_ref() != entry.owned.last() {
                conflicts.push(entry.name.clone());
            }
        }
    }
    let pending = settings
        .lease
        .as_ref()
        .is_some_and(|lease| lease.phase != Phase::Active);
    let inherited = env
        .read("NO_PROXY")?
        .and_then(|value| value.text())
        .or_else(|| std::env::var("NO_PROXY").ok())
        .unwrap_or_default();
    Ok(TerminalProxyStatus {
        supported: cfg!(windows),
        enabled: settings.lease.is_some(),
        applied: settings.lease.is_some() && !pending && conflicts.is_empty(),
        url: settings.url.clone(),
        no_proxy: normalize_bypass_with_limit(
            if settings.no_proxy.is_empty() {
                &inherited
            } else {
                &settings.no_proxy
            },
            MAX_STORED_BYPASS_BYTES,
        )?,
        recovery_required: pending,
        conflicts,
        preserved_variables: Vec::new(),
        notification_sent: true,
    })
}

fn set_with_environment(
    path: &Path,
    env: &mut impl Environment,
    enabled: bool,
    url: &str,
    bypass: &str,
) -> Result<TerminalProxyStatus, String> {
    let previous = load(path)?;
    if !enabled {
        let Some(mut lease) = previous.lease.clone() else {
            return status(&previous, env);
        };
        lease.phase = Phase::Restoring;
        let mut restoring = previous.clone();
        restoring.lease = Some(lease.clone());
        persist(path, &restoring)?;
        let restore_result = (|| {
            let mut preserved = Vec::new();
            for entry in &lease.entries {
                let current = env.read(&entry.name)?;
                if current == entry.original {
                    continue;
                }
                if current
                    .as_ref()
                    .is_some_and(|value| entry.owned.contains(value))
                {
                    env.write(&entry.name, entry.original.as_ref())?;
                } else {
                    preserved.push(entry.name.clone());
                }
            }
            restoring.lease = None;
            persist(path, &restoring)?;
            Ok::<_, String>(preserved)
        })();
        // Some fields may already have been restored when a later field or
        // journal write fails. Notify even on that recoverable error path.
        let notification_sent = env.notify();
        let preserved = restore_result?;
        let mut result = status(&restoring, env)?;
        result.notification_sent = notification_sent;
        result.preserved_variables = preserved;
        return Ok(result);
    }
    let url = normalize_url(url)?;
    let bypass = normalize_bypass(bypass)?;
    let before = NAMES
        .iter()
        .map(|name| env.read(name))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(lease) = &previous.lease {
        if lease.phase != Phase::Active {
            return Err("settings.terminalProxy.errors.recovery".into());
        }
        if lease.entries.iter().any(|entry| {
            before[NAMES.iter().position(|name| *name == entry.name).unwrap()].as_ref()
                != entry.owned.last()
        }) {
            return Err("settings.terminalProxy.errors.changed".into());
        }
    }
    let target: Vec<EnvironmentValue> = NAMES
        .iter()
        .map(|name| EnvironmentValue::string(if *name == "NO_PROXY" { &bypass } else { &url }))
        .collect();
    let entries = NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let old = previous
                .lease
                .as_ref()
                .and_then(|lease| lease.entries.iter().find(|entry| entry.name == *name));
            let mut owned: Vec<_> = old
                .and_then(|entry| entry.owned.last())
                .cloned()
                .into_iter()
                .collect();
            if !owned.contains(&target[index]) {
                owned.push(target[index].clone());
            }
            Entry {
                name: (*name).into(),
                original: old
                    .map(|entry| entry.original.clone())
                    .unwrap_or_else(|| before[index].clone()),
                owned,
            }
        })
        .collect();
    let mut applying = Settings {
        url: Some(url),
        no_proxy: bypass,
        lease: Some(Lease {
            phase: Phase::Applying,
            entries,
        }),
    };
    // Journal precedes every environment write, including first-time activation.
    persist(path, &applying)?;
    let write_result = (|| {
        for (index, name) in NAMES.iter().enumerate() {
            if env.read(name)? != before[index] {
                return Err("settings.terminalProxy.errors.changed".to_owned());
            }
            env.write(name, Some(&target[index]))?;
        }
        applying.lease.as_mut().unwrap().phase = Phase::Active;
        for (index, entry) in applying
            .lease
            .as_mut()
            .unwrap()
            .entries
            .iter_mut()
            .enumerate()
        {
            entry.owned = vec![target[index].clone()];
        }
        persist(path, &applying)
    })();
    if let Err(error) = write_result {
        let mut rolled_back = true;
        for (index, name) in NAMES.iter().enumerate() {
            match env.read(name) {
                Ok(current) if current == before[index] => {}
                Ok(current) if current.as_ref() == Some(&target[index]) => {
                    rolled_back &= env.write(name, before[index].as_ref()).is_ok();
                }
                _ => {
                    rolled_back = false;
                }
            }
        }
        if rolled_back {
            let _ = persist(path, &previous);
        }
        env.notify();
        return Err(error);
    }
    let notification_sent = env.notify();
    let mut result = status(&applying, env)?;
    result.notification_sent = notification_sent;
    Ok(result)
}

pub fn load_status(paths: &CodexPaths) -> Result<TerminalProxyStatus, String> {
    let _guard = guard()?;
    #[cfg(windows)]
    {
        let mut env = crate::platform::terminal_environment::windows::UserEnvironment::open(false)?;
        return status(
            &load(&paths.app_data_dir.join("terminal-proxy.dat"))?,
            &mut env,
        );
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        Ok(TerminalProxyStatus {
            supported: false,
            enabled: false,
            applied: false,
            url: None,
            no_proxy: LOOPBACK.join(","),
            recovery_required: false,
            conflicts: Vec::new(),
            preserved_variables: Vec::new(),
            notification_sent: false,
        })
    }
}

/// The terminal proxy's address while new terminals carry it: a client set
/// up to reach its relay directly inherits it when started from one.
pub fn active_url(paths: &CodexPaths) -> Option<String> {
    load_status(paths)
        .ok()
        .filter(|status| status.applied)
        .and_then(|status| status.url)
}

pub fn set(
    paths: &CodexPaths,
    enabled: bool,
    url: &str,
    bypass: &str,
) -> Result<TerminalProxyStatus, String> {
    let _guard = guard()?;
    #[cfg(windows)]
    {
        let mut env = crate::platform::terminal_environment::windows::UserEnvironment::open(true)?;
        return set_with_environment(
            &paths.app_data_dir.join("terminal-proxy.dat"),
            &mut env,
            enabled,
            url,
            bypass,
        );
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, enabled, url, bypass);
        Err("settings.terminalProxy.errors.unsupported".into())
    }
}

pub fn open_terminal(paths: &CodexPaths) -> Result<(), String> {
    let _guard = guard()?;
    #[cfg(windows)]
    {
        let settings = load(&paths.app_data_dir.join("terminal-proxy.dat"))?;
        let mut env = crate::platform::terminal_environment::windows::UserEnvironment::open(false)?;
        if !status(&settings, &mut env)?.applied {
            return Err("settings.terminalProxy.errors.changed".into());
        }
        let values = settings
            .lease
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .map(|entry| {
                entry
                    .owned
                    .last()
                    .and_then(EnvironmentValue::text)
                    .map(|text| (entry.name.clone(), text))
                    .ok_or_else(|| "settings.terminalProxy.errors.storage".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return crate::platform::terminal_environment::windows::open_terminal(&values);
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        Err("settings.terminalProxy.errors.unsupported".into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyProbe {
    service: String,
    reachable: bool,
    status_code: Option<u16>,
}
pub fn test(url: &str) -> Result<Vec<ProxyProbe>, String> {
    let config = ApiProxyConfigPayload {
        mode: ApiProxyMode::Manual,
        url: Some(normalize_url(url)?),
    };
    let client =
        api_client::configure_blocking_client(reqwest::blocking::Client::builder(), &config)
            .map_err(|_| "settings.terminalProxy.errors.url")?
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "settings.terminalProxy.errors.url")?;
    Ok([
        ("Anthropic", "https://api.anthropic.com/"),
        ("OpenAI", "https://api.openai.com/v1/models"),
    ]
    .into_iter()
    .map(|(service, endpoint)| {
        let response = client.get(endpoint).send();
        let code = response.ok().map(|response| response.status().as_u16());
        ProxyProbe {
            service: service.into(),
            reachable: code.is_some_and(|code| matches!(code, 200..=299 | 401 | 404 | 405)),
            status_code: code,
        }
    })
    .collect())
}

#[cfg(test)]
#[path = "terminal_proxy_tests.rs"]
mod tests;
