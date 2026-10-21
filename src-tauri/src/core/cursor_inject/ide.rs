//! Cursor IDE: its `settings.json` sends Cursor's traffic through the local
//! front. The file is edited in place (comments and layout kept), and only the
//! keys AgentHarbor wrote are put back, to what they were, when it lets go.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use jsonc_parser::{
    cst::{CstInputValue, CstObject, CstRootNode},
    ParseOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The values AgentHarbor owns while Cursor is injected.
fn wanted(proxy_url: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("http.proxy".to_owned(), Value::String(proxy_url.to_owned())),
        // Extensions and the agent use the proxy too.
        ("http.proxySupport".to_owned(), Value::String("on".to_owned())),
        // The front serves Cursor's Agent over HTTP/1.1.
        ("cursor.general.disableHttp2".to_owned(), Value::Bool(true)),
        // Trust the local certificate from the system store.
        ("http.experimental.systemCertificatesV2".to_owned(), Value::Bool(true)),
    ])
}

/// What AgentHarbor changed in the file, to undo it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub path: PathBuf,
    /// Each owned key's value before AgentHarbor wrote it; `None` when absent.
    pub originals: BTreeMap<String, Option<Value>>,
    pub written: BTreeMap<String, Value>,
}

impl Lease {
    /// The proxy Cursor used before, if it had one of its own.
    pub fn original_proxy(&self) -> Option<String> {
        self.originals
            .get("http.proxy")
            .cloned()
            .flatten()
            .and_then(|value| value.as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned))
    }
}

/// Cursor's user directory (`.../Cursor/User`), whether or not it exists.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("AGENT_HARBOR_CURSOR_USER_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    // An isolated client home keeps Cursor's settings inside it too.
    if let Some(home) = std::env::var_os("AGENT_HARBOR_CLIENT_HOME").filter(|home| !home.is_empty()) {
        return Some(PathBuf::from(home).join(".config").join("Cursor").join("User"));
    }
    let base = if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library").join("Application Support")
    } else {
        dirs::config_dir()?
    };
    Some(base.join("Cursor").join("User"))
}

pub fn settings_path() -> Option<PathBuf> {
    Some(user_dir()?.join("settings.json"))
}

/// Cursor IDE is installed here: its user directory exists.
pub fn installed() -> bool {
    user_dir().is_some_and(|dir| dir.is_dir())
}

/// Cursor IDE has a process running now.
pub fn running() -> bool {
    crate::platform::process::is_process_running(if cfg!(any(windows, target_os = "macos")) { "Cursor" } else { "cursor" })
}

fn parse(text: &str) -> Result<CstRootNode, String> {
    let text = if text.trim().is_empty() { "{}" } else { text };
    CstRootNode::parse(text, &ParseOptions::default()).map_err(|_| "cursor.errors.settingsUnreadable".to_owned())
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::Object(map) => CstInputValue::Object(map.iter().map(|(key, value)| (key.clone(), input(value))).collect()),
    }
}

fn current(object: &CstObject, key: &str) -> Option<Value> {
    object.get(key)?.value()?.to_serde_value()
}

fn set(object: &CstObject, key: &str, value: Option<&Value>) {
    match (object.get(key), value) {
        (Some(prop), Some(value)) => prop.set_value(input(value)),
        (Some(prop), None) => prop.remove(),
        (None, Some(value)) => {
            object.append(key, input(value));
        }
        (None, None) => {}
    }
}

fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err("cursor.errors.settingsUnreadable".into()),
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "cursor.errors.settingsWrite")?;
    }
    let temporary = path.with_extension("json.agentharbor-tmp");
    std::fs::write(&temporary, text).map_err(|_| "cursor.errors.settingsWrite")?;
    std::fs::rename(&temporary, path).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        "cursor.errors.settingsWrite".to_owned()
    })
}

/// Points Cursor at `proxy_url`. A lease already held keeps the values from
/// before AgentHarbor's first write. `previous_port` is the engine port of the
/// last run, which tells AgentHarbor's leftovers from the user's own settings.
pub fn apply(path: &Path, proxy_url: &str, held: Option<&Lease>, previous_port: u16) -> Result<Lease, String> {
    let text = read(path)?;
    let root = parse(&text)?;
    let object = root.object_value_or_set();
    let wanted = wanted(proxy_url);
    let held = held.filter(|lease| lease.path == path);
    // Settings AgentHarbor wrote before a crash, with the lease lost: what
    // was there before them is unknown, so they go when the lease ends.
    let stale = held.is_none() && leftover(&object, previous_port);
    let mut originals = held.map(|lease| lease.originals.clone()).unwrap_or_default();
    if stale {
        originals = wanted.keys().map(|key| (key.clone(), None)).collect();
    }
    for (key, value) in &wanted {
        let now = current(&object, key);
        if !originals.contains_key(key) {
            originals.insert(key.clone(), now.clone());
        }
        if now.as_ref() != Some(value) {
            set(&object, key, Some(value));
        }
    }
    let updated = root.to_string();
    if updated != text {
        write(path, &updated)?;
    }
    Ok(Lease { path: path.to_owned(), originals, written: wanted })
}

/// Whether the settings look like AgentHarbor's own, left behind: every
/// managed key set as it writes them, and the proxy the engine's address on
/// its previous port. A loopback proxy on any other port is the user's.
fn leftover(object: &CstObject, previous_port: u16) -> bool {
    let ours = current(object, "http.proxy")
        .and_then(|value| value.as_str().map(str::to_owned))
        .is_some_and(|url| super::left_by_engine(&url, previous_port));
    ours
        && wanted("").iter().filter(|(key, _)| *key != "http.proxy").all(|(key, value)| current(object, key).as_ref() == Some(value))
}

/// Puts back what AgentHarbor changed. A key the user has since changed is
/// theirs and stays as it is. A local proxy from before that no longer
/// listens (a tool that has since quit) is not put back: Cursor could not
/// reach anything through it, its sign-in included.
pub fn release(lease: &Lease) -> Result<(), String> {
    let text = read(&lease.path)?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let root = parse(&text)?;
    let object = root.object_value_or_set();
    for (key, written) in &lease.written {
        if current(&object, key).as_ref() == Some(written) {
            let original = lease.originals.get(key).and_then(Option::as_ref);
            let dead_proxy = key == "http.proxy" && original.and_then(Value::as_str).is_some_and(|proxy| !super::proxy_answers(proxy));
            set(&object, key, original.filter(|_| !dead_proxy));
        }
    }
    let updated = root.to_string();
    if updated != text {
        write(&lease.path, &updated)?;
    }
    Ok(())
}

/// Whether Cursor's settings point at `proxy_url` right now.
pub fn applied(path: &Path, proxy_url: &str) -> bool {
    let Ok(text) = read(path) else { return false };
    let Ok(root) = parse(&text) else { return false };
    let Some(object) = root.object_value() else { return false };
    wanted(proxy_url).iter().all(|(key, value)| current(&object, key).as_ref() == Some(value))
}

/// Quits Cursor IDE (asking it to close first, so it saves its state) and
/// starts it again so it picks up the proxy.
pub fn restart() -> Result<(), String> {
    #[cfg(windows)]
    {
        let executable = windows_executable().ok_or("cursor.errors.ideMissing")?;
        crate::platform::process::restart_windows_app("Cursor.exe", &executable).map_err(|_| "cursor.errors.restart".into())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript").args(["-e", "quit app \"Cursor\""]).output();
        std::thread::sleep(std::time::Duration::from_millis(2000));
        std::process::Command::new("open").args(["-a", "Cursor"]).spawn().map_err(|_| "cursor.errors.restart")?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = std::process::Command::new("pkill").args(["-x", "cursor"]).output();
        std::thread::sleep(std::time::Duration::from_millis(2000));
        std::process::Command::new("cursor").spawn().map_err(|_| "cursor.errors.restart")?;
        Ok(())
    }
}

/// Where Cursor IDE is installed on Windows: per user or for everyone.
#[cfg(windows)]
fn windows_executable() -> Option<PathBuf> {
    let per_user = dirs::data_local_dir().map(|dir| dir.join("Programs").join("cursor").join("Cursor.exe"));
    let everyone = std::env::var_os("ProgramFiles").map(|dir| PathBuf::from(dir).join("Cursor").join("Cursor.exe"));
    [per_user, everyone].into_iter().flatten().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_keeps_comments_and_releasing_restores_what_was_there() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = "{\n    // the user's own note\n    \"editor.fontSize\": 14,\n    \"http.proxy\": \"http://corp:8080\",\n}\n";
        std::fs::write(&path, original).unwrap();

        let lease = apply(&path, "http://127.0.0.1:4567", None, 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("// the user's own note"));
        assert!(text.contains("\"http.proxy\": \"http://127.0.0.1:4567\""));
        assert!(text.contains("\"cursor.general.disableHttp2\": true"));
        assert!(applied(&path, "http://127.0.0.1:4567"));
        assert_eq!(lease.original_proxy().as_deref(), Some("http://corp:8080"));

        // A second apply (a new port) keeps the values from before the first.
        let lease = apply(&path, "http://127.0.0.1:5678", Some(&lease), 4567).unwrap();
        assert_eq!(lease.original_proxy().as_deref(), Some("http://corp:8080"));

        release(&lease).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("// the user's own note"));
        assert!(text.contains("\"http.proxy\": \"http://corp:8080\""));
        assert!(!text.contains("disableHttp2"));
        assert!(!text.contains("systemCertificatesV2"));
        assert!(!applied(&path, "http://127.0.0.1:5678"));
    }

    #[test]
    fn settings_left_from_a_crash_are_not_taken_for_the_users() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let first = apply(&path, "http://127.0.0.1:4567", None, 0).unwrap();
        assert_eq!(first.original_proxy(), None);
        // The lease is lost; the next start finds the old values, on the port it ran on.
        let again = apply(&path, "http://127.0.0.1:5678", None, 4567).unwrap();
        assert_eq!(again.original_proxy(), None, "the old local port is not the user's proxy");
        release(&again).unwrap();
        let value: Value = json5::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn a_loopback_proxy_on_another_port_is_the_users_when_the_lease_is_lost() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        // The same managed keys as AgentHarbor writes, but the proxy is the user's own tool, still running.
        let users_proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://127.0.0.1:{}", users_proxy.local_addr().unwrap().port());
        std::fs::write(
            &path,
            format!("{{\n  \"http.proxy\": \"{proxy}\",\n  \"http.proxySupport\": \"on\",\n  \"cursor.general.disableHttp2\": true,\n  \"http.experimental.systemCertificatesV2\": true\n}}\n"),
        )
        .unwrap();
        let lease = apply(&path, "http://127.0.0.1:4567", None, 4567).unwrap();
        assert_eq!(lease.original_proxy().as_deref(), Some(proxy.as_str()));
        release(&lease).unwrap();
        let value: Value = json5::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["http.proxy"], proxy);
        assert_eq!(value["cursor.general.disableHttp2"], true);
    }

    #[test]
    fn a_local_proxy_that_quit_is_not_put_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        // Another tool's local proxy, left in the settings after it quit.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        std::fs::write(&path, format!("{{\n  \"editor.fontSize\": 14,\n  \"http.proxy\": \"http://127.0.0.1:{port}\"\n}}\n")).unwrap();
        let lease = apply(&path, "http://127.0.0.1:4567", None, 0).unwrap();
        release(&lease).unwrap();
        let value: Value = json5::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value, serde_json::json!({ "editor.fontSize": 14 }));
    }

    #[test]
    fn a_value_the_user_changed_since_is_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let lease = apply(&path, "http://127.0.0.1:4567", None, 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap().replace("\"http.proxySupport\": \"on\"", "\"http.proxySupport\": \"off\"");
        std::fs::write(&path, text).unwrap();
        release(&lease).unwrap();
        let value: Value = json5::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["http.proxySupport"], "off");
        assert!(value.get("http.proxy").is_none());
    }
}
