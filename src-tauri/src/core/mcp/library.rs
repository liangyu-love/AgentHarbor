//! The MCP library: one list of servers, and the agents each one is on. A
//! server turned on for an agent is written to that agent's configuration;
//! turned off, it is taken out. Only names in the library are ever touched,
//! so servers the user keeps elsewhere stay as they are until imported.
use super::apps::{self, Live, McpApp, McpSpec, Target};
use crate::core::client_config::ClientPaths;
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE: &str = "mcp-library.json";
const VERSION: u32 = 1;
const MAX_NAME_CHARS: usize = 64;
const MAX_NOTE_CHARS: usize = 500;

/// Edits of the library and the agents' files happen one at a time.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerEntry {
    pub name: String,
    pub spec: McpSpec,
    #[serde(default)]
    pub note: String,
    /// The agents it is on.
    #[serde(default)]
    pub apps: Vec<McpApp>,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    version: u32,
    servers: Vec<McpServerEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpCellState {
    /// On, and the agent holds it as the library has it.
    Synced,
    /// On, but the agent's configuration does not have it.
    Missing,
    /// On, but the agent holds something else under this name.
    Differs,
    /// Off, and the agent does not have it.
    Off,
    /// Off, yet the agent's configuration has a server by this name.
    Outside,
    Unsupported,
    NotInstalled,
    /// The agent's configuration cannot be read.
    Unreadable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCell {
    pub app: McpApp,
    pub on: bool,
    pub state: McpCellState,
    /// An i18n key saying why the agent cannot hold the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub name: String,
    pub spec: McpSpec,
    pub note: String,
    pub cells: Vec<McpCell>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppView {
    pub app: McpApp,
    pub path: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Servers in the agent's configuration the library does not have.
    pub unmanaged: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpFailure {
    pub name: String,
    pub app: McpApp,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLibraryView {
    pub servers: Vec<McpServerView>,
    pub apps: Vec<McpAppView>,
    /// Writes or imports that did not take; the rest of the change stands.
    pub failures: Vec<McpFailure>,
    /// Servers an import added to the library.
    pub added: Vec<String>,
}

/// Where the library and the agents' files are.
pub struct McpContext {
    app_data_dir: PathBuf,
    targets: Vec<Target>,
}

impl McpContext {
    pub fn new(codex: &CodexPaths, clients: &ClientPaths, desktop: Option<PathBuf>) -> Self {
        Self {
            app_data_dir: codex.app_data_dir.clone(),
            targets: apps::targets(codex, clients, desktop),
        }
    }

    /// The real agents' locations, Claude Desktop's as its integration resolves it.
    pub fn discover(codex: &CodexPaths) -> Self {
        let desktop = crate::core::claude_desktop::Manager::new(&codex.app_data_dir)
            .current_paths()
            .map(|paths| paths.normal_config);
        Self::new(codex, &ClientPaths::discover(), desktop)
    }

    fn file(&self) -> PathBuf {
        self.app_data_dir.join(FILE)
    }

    fn backups(&self) -> PathBuf {
        self.app_data_dir.join("mcp-backups")
    }

    fn target(&self, app: McpApp) -> &Target {
        self.targets
            .iter()
            .find(|target| target.app == app)
            .expect("every app has a target")
    }
}

fn valid_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAX_NAME_CHARS).contains(&count)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
}

fn read_store(path: &Path) -> Result<Option<Stored>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "mcp.errors.library".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("mcp.errors.library".into()),
    }
}

fn write_store(path: &Path, servers: &[McpServerEntry]) -> Result<(), String> {
    let mut servers = servers.to_vec();
    servers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    for server in &mut servers {
        server.apps.sort();
        server.apps.dedup();
    }
    let stored = Stored {
        version: VERSION,
        servers,
    };
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|_| "mcp.errors.library")?;
    crate::core::storage::write_atomic(path, &bytes).map_err(|_| "mcp.errors.library".into())
}

/// Each installed agent's servers now, or why they cannot be read.
fn read_all(context: &McpContext) -> BTreeMap<McpApp, Result<BTreeMap<String, Live>, String>> {
    context
        .targets
        .iter()
        .filter(|target| target.installed)
        .map(|target| (target.app, target.read()))
        .collect()
}

fn view(
    context: &McpContext,
    servers: &[McpServerEntry],
    failures: Vec<McpFailure>,
    added: Vec<String>,
) -> McpLibraryView {
    let live = read_all(context);
    let mut servers = servers.to_vec();
    servers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let servers = servers
        .into_iter()
        .map(|server| {
            let cells = context
                .targets
                .iter()
                .map(|target| {
                    let on = server.apps.contains(&target.app);
                    let reason = target.unsupported(&server.name, &server.spec);
                    let state = if !target.installed {
                        McpCellState::NotInstalled
                    } else if reason.is_some() {
                        McpCellState::Unsupported
                    } else {
                        match live.get(&target.app) {
                            Some(Ok(entries)) => match (on, entries.get(&server.name)) {
                                (
                                    true,
                                    Some(Live {
                                        spec: Some(spec),
                                        on: true,
                                    }),
                                ) if target.same(spec, &server.spec) => McpCellState::Synced,
                                (true, Some(_)) => McpCellState::Differs,
                                (true, None) => McpCellState::Missing,
                                (false, Some(_)) => McpCellState::Outside,
                                (false, None) => McpCellState::Off,
                            },
                            _ => McpCellState::Unreadable,
                        }
                    };
                    McpCell {
                        app: target.app,
                        on,
                        state,
                        reason: reason.map(str::to_owned),
                    }
                })
                .collect();
            McpServerView {
                name: server.name,
                spec: server.spec,
                note: server.note,
                cells,
            }
        })
        .collect::<Vec<_>>();
    let apps = context
        .targets
        .iter()
        .map(|target| {
            let (error, unmanaged) = match live.get(&target.app) {
                Some(Ok(entries)) => (
                    None,
                    entries
                        .keys()
                        .filter(|name| !servers.iter().any(|server| &server.name == *name))
                        .cloned()
                        .collect(),
                ),
                Some(Err(error)) => (Some(error.clone()), Vec::new()),
                None => (None, Vec::new()),
            };
            McpAppView {
                app: target.app,
                path: target.path.display().to_string(),
                installed: target.installed,
                error,
                unmanaged,
            }
        })
        .collect();
    McpLibraryView {
        servers,
        apps,
        failures,
        added,
    }
}

/// Bring every installed agent's servers into the library: a new name joins
/// with the agents that have it; a known name gains the agents that hold it
/// the same way; a known name held differently is reported, not changed.
fn import_into(
    context: &McpContext,
    servers: &mut Vec<McpServerEntry>,
) -> (Vec<String>, Vec<McpFailure>) {
    let mut added = Vec::new();
    let mut failures = Vec::new();
    for target in context.targets.iter().filter(|target| target.installed) {
        let entries = match target.read() {
            Ok(entries) => entries,
            Err(error) => {
                failures.push(McpFailure {
                    name: String::new(),
                    app: target.app,
                    error,
                });
                continue;
            }
        };
        for (name, live) in entries {
            let Some(spec) = live.spec.and_then(|spec| spec.clean().ok()) else {
                failures.push(McpFailure {
                    name,
                    app: target.app,
                    error: "mcp.errors.importUnreadable".into(),
                });
                continue;
            };
            if !valid_name(&name) {
                failures.push(McpFailure {
                    name,
                    app: target.app,
                    error: "mcp.errors.name".into(),
                });
                continue;
            }
            match servers.iter_mut().find(|server| server.name == name) {
                Some(server) if target.same(&spec, &server.spec) => {
                    if live.on && !server.apps.contains(&target.app) {
                        server.apps.push(target.app);
                    }
                }
                Some(_) => failures.push(McpFailure {
                    name,
                    app: target.app,
                    error: "mcp.errors.importConflict".into(),
                }),
                None => {
                    servers.push(McpServerEntry {
                        name: name.clone(),
                        spec,
                        note: String::new(),
                        apps: if live.on {
                            vec![target.app]
                        } else {
                            Vec::new()
                        },
                    });
                    added.push(name);
                }
            }
        }
    }
    (added, failures)
}

/// The library as it is; the first time, seeded from the agents' own servers.
pub fn load_library(context: &McpContext) -> Result<McpLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    match read_store(&context.file())? {
        Some(stored) => Ok(view(context, &stored.servers, Vec::new(), Vec::new())),
        None => {
            let mut servers = Vec::new();
            let (added, failures) = import_into(context, &mut servers);
            write_store(&context.file(), &servers)?;
            Ok(view(context, &servers, failures, added))
        }
    }
}

pub fn import_servers(context: &McpContext) -> Result<McpLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    let mut servers = read_store(&context.file())?.unwrap_or_default().servers;
    let (added, failures) = import_into(context, &mut servers);
    write_store(&context.file(), &servers)?;
    Ok(view(context, &servers, failures, added))
}

/// Turn `server` on for `app` (written) or off (taken out). A failure leaves
/// the switch as it was.
fn switch(
    context: &McpContext,
    server: &mut McpServerEntry,
    app: McpApp,
    on: bool,
) -> Option<McpFailure> {
    let target = context.target(app);
    let failure = |error: &str| McpFailure {
        name: server.name.clone(),
        app,
        error: error.to_owned(),
    };
    if on {
        if !target.installed {
            return Some(failure("mcp.errors.notInstalled"));
        }
        if let Some(reason) = target.unsupported(&server.name, &server.spec) {
            return Some(failure(reason));
        }
        if let Err(error) = target.write(&server.name, &server.spec, &context.backups()) {
            return Some(failure(&error));
        }
        if !server.apps.contains(&app) {
            server.apps.push(app);
        }
    } else {
        if target.installed {
            if let Err(error) = target.remove(&server.name, &context.backups()) {
                return Some(failure(&error));
            }
        }
        server.apps.retain(|held| *held != app);
    }
    None
}

/// Add a server, or change one (`original` is its name; renaming is not
/// supported). The agents it is on are rewritten, those it left are cleared.
pub fn save_server(
    context: &McpContext,
    original: Option<&str>,
    entry: McpServerEntry,
) -> Result<McpLibraryView, String> {
    let name = entry.name.trim().to_owned();
    if !valid_name(&name) {
        return Err("mcp.errors.name".into());
    }
    if original.is_some_and(|original| original != name) {
        return Err("mcp.errors.rename".into());
    }
    if entry.note.chars().count() > MAX_NOTE_CHARS {
        return Err("mcp.errors.note".into());
    }
    let spec = entry.spec.clean()?;
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    let mut servers = read_store(&context.file())?.unwrap_or_default().servers;
    let index = match (
        original,
        servers.iter().position(|server| server.name == name),
    ) {
        (None, Some(_)) => return Err("mcp.errors.duplicate".into()),
        (Some(_), None) => return Err("mcp.errors.notFound".into()),
        (None, None) => {
            servers.push(McpServerEntry {
                name: name.clone(),
                spec: spec.clone(),
                note: String::new(),
                apps: Vec::new(),
            });
            servers.len() - 1
        }
        (Some(_), Some(index)) => index,
    };
    let server = &mut servers[index];
    server.spec = spec;
    server.note = entry.note.trim().to_owned();
    let mut failures = Vec::new();
    for app in McpApp::ALL {
        let wanted = entry.apps.contains(&app);
        if wanted || server.apps.contains(&app) {
            failures.extend(switch(context, server, app, wanted));
        }
    }
    write_store(&context.file(), &servers)?;
    Ok(view(context, &servers, failures, Vec::new()))
}

/// Turn the named servers on or off for `app`, or for every agent when `app`
/// is `None`. Agents that cannot hold a server are skipped when turning on all.
pub fn set_servers(
    context: &McpContext,
    names: &[String],
    app: Option<McpApp>,
    on: bool,
) -> Result<McpLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    let mut servers = read_store(&context.file())?.unwrap_or_default().servers;
    let mut failures = Vec::new();
    for server in servers
        .iter_mut()
        .filter(|server| names.contains(&server.name))
    {
        match app {
            Some(app) => failures.extend(switch(context, server, app, on)),
            None => {
                for app in McpApp::ALL {
                    let target = context.target(app);
                    let able = target.installed
                        && target.unsupported(&server.name, &server.spec).is_none();
                    if (on && able) || (!on && server.apps.contains(&app)) {
                        failures.extend(switch(context, server, app, on));
                    }
                }
            }
        }
    }
    write_store(&context.file(), &servers)?;
    Ok(view(context, &servers, failures, Vec::new()))
}

/// Take the server out of every agent, then out of the library. When an agent
/// cannot be cleared, the server stays with that agent so it can be retried.
pub fn remove_server(context: &McpContext, name: &str) -> Result<McpLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    let mut servers = read_store(&context.file())?.unwrap_or_default().servers;
    let index = servers
        .iter()
        .position(|server| server.name == name)
        .ok_or("mcp.errors.notFound")?;
    let mut failures = Vec::new();
    let server = &mut servers[index];
    for app in server.apps.clone() {
        failures.extend(switch(context, server, app, false));
    }
    if server.apps.is_empty() {
        servers.remove(index);
    }
    write_store(&context.file(), &servers)?;
    Ok(view(context, &servers, failures, Vec::new()))
}

/// Write every server again to the agents it is on, for configurations
/// changed or reset outside the page.
pub fn resync_servers(context: &McpContext) -> Result<McpLibraryView, String> {
    let _guard = LOCK.lock().map_err(|_| "mcp.errors.library")?;
    let mut servers = read_store(&context.file())?.unwrap_or_default().servers;
    let mut failures = Vec::new();
    for server in &mut servers {
        for app in server.apps.clone() {
            failures.extend(switch(context, server, app, true));
        }
    }
    write_store(&context.file(), &servers)?;
    Ok(view(context, &servers, failures, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mcp::McpKind;
    use std::collections::BTreeMap;

    fn context() -> (tempfile::TempDir, McpContext) {
        let temp = tempfile::tempdir().unwrap();
        let codex = CodexPaths::from_home(temp.path().join(".codex"));
        for dir in [
            codex.codex_home.clone(),
            codex.app_data_dir.clone(),
            temp.path().join(".claude"),
            temp.path().join(".cursor"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let clients = ClientPaths::from_home(temp.path());
        let context = McpContext::new(&codex, &clients, None);
        (temp, context)
    }

    fn fetch(apps: Vec<McpApp>) -> McpServerEntry {
        McpServerEntry {
            name: "fetch".into(),
            spec: McpSpec {
                transport: McpKind::Stdio,
                command: Some("uvx".into()),
                args: vec!["mcp-server-fetch".into()],
                ..Default::default()
            },
            note: String::new(),
            apps,
        }
    }

    fn cell(view: &McpLibraryView, name: &str, app: McpApp) -> McpCell {
        view.servers
            .iter()
            .find(|server| server.name == name)
            .unwrap()
            .cells
            .iter()
            .find(|cell| cell.app == app)
            .unwrap()
            .clone()
    }

    #[test]
    fn the_first_load_imports_what_the_agents_have() {
        let (temp, context) = context();
        std::fs::write(
            temp.path().join(".claude.json"),
            r#"{"mcpServers":{"fetch":{"type":"stdio","command":"uvx","args":["mcp-server-fetch"]},"docs":{"type":"http","url":"https://d.invalid/mcp"}}}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".cursor/mcp.json"),
            r#"{"mcpServers":{"fetch":{"command":"uvx","args":["mcp-server-fetch"]},"docs":{"url":"https://other.invalid/mcp"}}}"#,
        )
        .unwrap();
        let view = load_library(&context).unwrap();
        assert_eq!(view.added, ["docs", "fetch"]);
        assert_eq!(
            cell(&view, "fetch", McpApp::ClaudeCode).state,
            McpCellState::Synced
        );
        assert_eq!(
            cell(&view, "fetch", McpApp::Cursor).state,
            McpCellState::Synced
        );
        // The same name held differently is reported and left off.
        assert_eq!(
            cell(&view, "docs", McpApp::Cursor).state,
            McpCellState::Outside
        );
        assert!(view
            .failures
            .iter()
            .any(|failure| failure.name == "docs" && failure.error == "mcp.errors.importConflict"));
        assert_eq!(
            cell(&view, "fetch", McpApp::OpenCode).state,
            McpCellState::NotInstalled
        );
        // Later loads read the library, not the agents.
        std::fs::write(temp.path().join(".cursor/mcp.json"), "{}").unwrap();
        let view = load_library(&context).unwrap();
        assert!(view.added.is_empty());
        assert_eq!(
            cell(&view, "fetch", McpApp::Cursor).state,
            McpCellState::Missing
        );
    }

    #[test]
    fn switches_write_and_take_out_and_saves_follow_the_switches() {
        let (temp, context) = context();
        load_library(&context).unwrap();
        let view = save_server(
            &context,
            None,
            fetch(vec![McpApp::Codex, McpApp::ClaudeCode]),
        )
        .unwrap();
        assert!(view.failures.is_empty(), "{:?}", view.failures);
        assert_eq!(
            cell(&view, "fetch", McpApp::Codex).state,
            McpCellState::Synced
        );
        let claude = std::fs::read_to_string(temp.path().join(".claude.json")).unwrap();
        assert!(claude.contains("mcp-server-fetch"));
        assert_eq!(
            save_server(&context, None, fetch(vec![])).unwrap_err(),
            "mcp.errors.duplicate"
        );

        let view =
            set_servers(&context, &["fetch".into()], Some(McpApp::ClaudeCode), false).unwrap();
        assert_eq!(
            cell(&view, "fetch", McpApp::ClaudeCode).state,
            McpCellState::Off
        );
        let claude = std::fs::read_to_string(temp.path().join(".claude.json")).unwrap();
        assert!(!claude.contains("mcp-server-fetch"));

        // A change reaches the agents it is on, and leaving one clears it.
        let mut changed = fetch(vec![McpApp::Cursor]);
        changed.spec.args.push("--verbose".into());
        let view = save_server(&context, Some("fetch"), changed).unwrap();
        assert_eq!(
            cell(&view, "fetch", McpApp::Cursor).state,
            McpCellState::Synced
        );
        assert_eq!(cell(&view, "fetch", McpApp::Codex).state, McpCellState::Off);
        assert!(crate::core::mcp::read_codex_servers(&CodexPaths::from_home(
            temp.path().join(".codex")
        ))
        .unwrap()
        .is_empty());

        // Turning on every agent skips the ones not installed.
        let view = set_servers(&context, &["fetch".into()], None, true).unwrap();
        assert!(view.failures.is_empty(), "{:?}", view.failures);
        assert!(cell(&view, "fetch", McpApp::Codex).on);
        assert!(!cell(&view, "fetch", McpApp::Pi).on);

        let view = remove_server(&context, "fetch").unwrap();
        assert!(view.servers.is_empty());
        assert!(
            !std::fs::read_to_string(temp.path().join(".cursor/mcp.json"))
                .unwrap()
                .contains("fetch")
        );
    }

    #[test]
    fn a_server_an_agent_cannot_hold_stays_off_there() {
        let (_temp, context) = context();
        load_library(&context).unwrap();
        let mut sse = fetch(vec![McpApp::Codex, McpApp::ClaudeCode]);
        sse.spec = McpSpec {
            transport: McpKind::Sse,
            url: Some("https://s.invalid/sse".into()),
            headers: BTreeMap::new(),
            ..Default::default()
        };
        let view = save_server(&context, None, sse).unwrap();
        assert_eq!(view.failures.len(), 1);
        assert_eq!(view.failures[0].error, "mcp.unsupported.sse");
        let codex = cell(&view, "fetch", McpApp::Codex);
        assert!(!codex.on);
        assert_eq!(codex.state, McpCellState::Unsupported);
        assert_eq!(
            cell(&view, "fetch", McpApp::ClaudeCode).state,
            McpCellState::Synced
        );
    }

    #[test]
    fn names_and_specs_are_checked_before_anything_is_written() {
        let (_temp, context) = context();
        let mut bad = fetch(vec![McpApp::Codex]);
        bad.name = "has space".into();
        assert_eq!(
            save_server(&context, None, bad).unwrap_err(),
            "mcp.errors.name"
        );
        let mut bad = fetch(vec![McpApp::Codex]);
        bad.spec.command = None;
        assert_eq!(
            save_server(&context, None, bad).unwrap_err(),
            "mcp.errors.command"
        );
        assert_eq!(
            save_server(&context, Some("other"), fetch(vec![])).unwrap_err(),
            "mcp.errors.rename"
        );
    }
}
