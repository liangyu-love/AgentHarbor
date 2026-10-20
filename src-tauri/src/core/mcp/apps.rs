//! How each agent holds MCP servers: where its configuration lives, how a
//! server reads and is written there, and what it cannot take. Every write
//! keeps what the page does not manage — other servers, the rest of the file,
//! and the fields of a server entry the page does not write (timeouts, OAuth,
//! tool lists) — backs the file up first, and is refused when the file changed
//! while it was being edited.
use super::{read_codex_servers, remove_mcp_server, set_codex_server};
use crate::core::client_config::{self, harness_mcp_rows, ClientPaths};
use crate::core::models::CoreError;
use crate::core::targets::TargetId;
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The agents whose MCP servers the page manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum McpApp {
    Codex,
    ClaudeCode,
    OpenCode,
    Pi,
    Cursor,
    Harness,
    ClaudeDesktop,
}

impl McpApp {
    pub const ALL: [McpApp; 7] = [
        McpApp::Codex,
        McpApp::ClaudeCode,
        McpApp::OpenCode,
        McpApp::Pi,
        McpApp::Cursor,
        McpApp::Harness,
        McpApp::ClaudeDesktop,
    ];

    fn key(self) -> &'static str {
        match self {
            McpApp::Codex => "codex",
            McpApp::ClaudeCode => "claude-code",
            McpApp::OpenCode => "opencode",
            McpApp::Pi => "pi",
            McpApp::Cursor => "cursor",
            McpApp::Harness => "harness",
            McpApp::ClaudeDesktop => "claude-desktop",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpKind {
    #[default]
    Stdio,
    Http,
    Sse,
}

/// One server as the page keeps it, whatever agent it is written to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpSpec {
    pub transport: McpKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl McpSpec {
    /// The spec trimmed to what its transport uses; refused when incomplete.
    pub fn clean(mut self) -> Result<Self, String> {
        let text = |value: Option<String>| {
            value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let map = |values: BTreeMap<String, String>| {
            values
                .into_iter()
                .map(|(key, value)| (key.trim().to_owned(), value))
                .filter(|(key, _)| !key.is_empty())
                .collect()
        };
        self.command = text(self.command);
        self.cwd = text(self.cwd);
        self.url = text(self.url);
        self.args.retain(|arg| !arg.is_empty());
        self.env = map(self.env);
        self.headers = map(self.headers);
        match self.transport {
            McpKind::Stdio => {
                if self.command.is_none() {
                    return Err("mcp.errors.command".into());
                }
                self.url = None;
                self.headers.clear();
            }
            McpKind::Http | McpKind::Sse => {
                let remote = self
                    .url
                    .as_deref()
                    .and_then(|url| reqwest::Url::parse(url).ok())
                    .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
                if !remote {
                    return Err("mcp.errors.url".into());
                }
                self.command = None;
                self.args.clear();
                self.env.clear();
                self.cwd = None;
            }
        }
        Ok(self)
    }
}

/// A server as an agent's configuration holds it now. `spec` is `None` for an
/// entry the page cannot read (an unknown shape, or Harness `!!js` values).
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub spec: Option<McpSpec>,
    /// The agent's own switch (Codex `enabled`, Pi and OpenCode `enabled`).
    pub on: bool,
}

/// One agent's MCP configuration file.
pub struct Target {
    pub app: McpApp,
    pub path: PathBuf,
    pub installed: bool,
    codex: Option<CodexPaths>,
}

/// Every agent's target. `desktop` is Claude Desktop's configuration, when found.
pub fn targets(codex: &CodexPaths, clients: &ClientPaths, desktop: Option<PathBuf>) -> Vec<Target> {
    let dir_exists = |path: &Path| path.parent().is_some_and(Path::is_dir);
    let pi = clients.pi.with_file_name("mcp.json");
    let cursor = clients.cursor_cli.join("mcp.json");
    let harness = clients.harness.join("cordis.patch.yml");
    let claude = clients.claude_user_config();
    vec![
        Target {
            app: McpApp::Codex,
            installed: codex.codex_home.is_dir(),
            path: codex.config_path.clone(),
            codex: Some(codex.clone()),
        },
        Target {
            app: McpApp::ClaudeCode,
            installed: clients.claude_root().is_dir() || claude.is_file(),
            path: claude,
            codex: None,
        },
        Target {
            app: McpApp::OpenCode,
            installed: dir_exists(&clients.opencode),
            path: clients.opencode.clone(),
            codex: None,
        },
        Target {
            app: McpApp::Pi,
            installed: dir_exists(&pi),
            path: pi,
            codex: None,
        },
        Target {
            app: McpApp::Cursor,
            installed: dir_exists(&cursor),
            path: cursor,
            codex: None,
        },
        Target {
            app: McpApp::Harness,
            installed: dir_exists(&harness),
            path: harness,
            codex: None,
        },
        Target {
            app: McpApp::ClaudeDesktop,
            installed: desktop.as_deref().is_some_and(dir_exists),
            path: desktop.unwrap_or_default(),
            codex: None,
        },
    ]
}

/// Harness names a server's tools after it: `[A-Za-z0-9_-]{1,32}`.
fn harness_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Windows runs these through `cmd /c`; Claude Code asks for the wrapper.
fn wants_cmd(command: &str) -> bool {
    cfg!(windows)
        && ["npx", "pnpx", "pnpm", "npm", "yarn", "bunx"]
            .iter()
            .any(|name| command.eq_ignore_ascii_case(name))
}

const SHAPE: &str = "mcp.errors.configShape";

fn strings(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// A `!!js` value or another tag the page cannot carry over.
fn tagged(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("!tag") || map.values().any(tagged),
        Value::Array(items) => items.iter().any(tagged),
        _ => false,
    }
}

impl Target {
    /// Why this agent cannot hold the server, as an i18n key.
    pub fn unsupported(&self, name: &str, spec: &McpSpec) -> Option<&'static str> {
        match (self.app, spec.transport) {
            (McpApp::Codex | McpApp::Pi | McpApp::Harness, McpKind::Sse) => {
                Some("mcp.unsupported.sse")
            }
            (McpApp::ClaudeDesktop, McpKind::Http | McpKind::Sse) => Some("mcp.unsupported.remote"),
            (McpApp::Harness, _) if !harness_name(name) => Some("mcp.unsupported.harnessName"),
            _ => None,
        }
    }

    /// The spec as this agent holds it, for comparing: fields it does not
    /// take are dropped, and agents that find the remote transport themselves
    /// do not tell SSE from HTTP.
    fn held(&self, spec: &McpSpec) -> McpSpec {
        let mut spec = spec.clone();
        if !matches!(self.app, McpApp::Codex | McpApp::Pi | McpApp::Harness) {
            spec.cwd = None;
        }
        if matches!(self.app, McpApp::OpenCode | McpApp::Cursor) && spec.transport == McpKind::Sse {
            spec.transport = McpKind::Http;
        }
        spec
    }

    /// Whether the agent holds `live` as the page would write `spec`.
    pub fn same(&self, live: &McpSpec, spec: &McpSpec) -> bool {
        self.held(live) == self.held(spec)
    }

    pub fn read(&self) -> Result<BTreeMap<String, Live>, String> {
        match self.app {
            McpApp::Codex => self.read_codex(),
            McpApp::Harness => self.read_harness(),
            _ => {
                let (_, document) = self.json_document(read_text(&self.path)?.as_deref())?;
                Ok(document
                    .get(self.entries_key())
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                    .map(|(name, entry)| (name.clone(), self.parse_entry(entry)))
                    .collect())
            }
        }
    }

    pub fn write(&self, name: &str, spec: &McpSpec, backups: &Path) -> Result<(), String> {
        match self.app {
            McpApp::Codex => set_codex_server(self.codex_paths(), name, &codex_table(spec))
                .map_err(|error| error.to_string()),
            McpApp::Harness => self.edit_text(backups, |text| {
                let rows = harness_mcp_rows::read(text)?;
                let existing = rows.iter().find(|(_, config)| {
                    config.get("serverName").and_then(Value::as_str) == Some(name)
                });
                let id = existing.map_or_else(|| format!("mcp-{name}"), |(id, _)| id.clone());
                let mut config = existing
                    .and_then(|(_, config)| config.as_object().cloned())
                    .unwrap_or_default();
                for key in HARNESS_KEYS {
                    config.remove(key);
                }
                config.extend(harness_config(name, spec));
                harness_mcp_rows::upsert(text, &id, &Value::Object(config))
            }),
            _ => self.edit_text(backups, |text| {
                let (id, mut document) = self.json_document(text)?;
                let root = document.as_object_mut().ok_or(SHAPE)?;
                let entries = root
                    .entry(self.entries_key())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or(SHAPE)?;
                let mut entry = entries
                    .get(name)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for key in self.managed_keys() {
                    entry.remove(*key);
                }
                entry.extend(self.native(spec));
                entries.insert(name.to_owned(), Value::Object(entry));
                forget_disabled(root, name);
                self.render(id, text, &document).map(Some)
            }),
        }
    }

    /// Takes the server out of the agent's configuration; absent is done.
    pub fn remove(&self, name: &str, backups: &Path) -> Result<(), String> {
        match self.app {
            McpApp::Codex => match remove_mcp_server(self.codex_paths(), name) {
                Ok(()) | Err(CoreError::NotFound(_)) => Ok(()),
                Err(error) => Err(error.to_string()),
            },
            McpApp::Harness => self.edit_text(backups, |text| {
                let id = harness_mcp_rows::read(text)?
                    .into_iter()
                    .find(|(_, config)| {
                        config.get("serverName").and_then(Value::as_str) == Some(name)
                    })
                    .map(|(id, _)| id);
                match id {
                    Some(id) => harness_mcp_rows::remove(text, &id),
                    None => Ok(text.map(str::to_owned)),
                }
            }),
            _ => self.edit_text(backups, |text| {
                let (id, mut document) = self.json_document(text)?;
                let root = document.as_object_mut().ok_or(SHAPE)?;
                let removed = root
                    .get_mut(self.entries_key())
                    .and_then(Value::as_object_mut)
                    .and_then(|entries| entries.remove(name))
                    .is_some();
                let forgotten = forget_disabled(root, name);
                if !removed && !forgotten {
                    return Ok(text.map(str::to_owned));
                }
                self.render(id, text, &document).map(Some)
            }),
        }
    }

    fn codex_paths(&self) -> &CodexPaths {
        self.codex
            .as_ref()
            .expect("the Codex target carries its paths")
    }

    fn read_codex(&self) -> Result<BTreeMap<String, Live>, String> {
        let servers = read_codex_servers(self.codex_paths()).map_err(|error| error.to_string())?;
        Ok(servers
            .into_iter()
            .map(|(name, table)| {
                let entry = serde_json::to_value(&table).unwrap_or(Value::Null);
                let on = entry.get("enabled").and_then(Value::as_bool) != Some(false);
                let spec = if let Some(url) = entry.get("url").and_then(Value::as_str) {
                    let sse = entry.get("transport").and_then(Value::as_str) == Some("sse");
                    Some(McpSpec {
                        transport: if sse { McpKind::Sse } else { McpKind::Http },
                        url: Some(url.to_owned()),
                        headers: strings(
                            entry.get("http_headers").or_else(|| entry.get("headers")),
                        ),
                        ..Default::default()
                    })
                } else {
                    entry
                        .get("command")
                        .and_then(Value::as_str)
                        .map(|command| McpSpec {
                            transport: McpKind::Stdio,
                            command: Some(command.to_owned()),
                            args: string_list(entry.get("args")),
                            env: strings(entry.get("env")),
                            cwd: entry.get("cwd").and_then(Value::as_str).map(str::to_owned),
                            ..Default::default()
                        })
                };
                (name, Live { spec, on })
            })
            .collect())
    }

    fn read_harness(&self) -> Result<BTreeMap<String, Live>, String> {
        let text = read_text(&self.path)?;
        Ok(harness_mcp_rows::read(text.as_deref())?
            .into_iter()
            .filter_map(|(_, config)| {
                let name = config.get("serverName")?.as_str()?.to_owned();
                let spec = (!tagged(&config))
                    .then(|| match config.get("transport").and_then(Value::as_str) {
                        Some("stdio") => Some(McpSpec {
                            transport: McpKind::Stdio,
                            command: Some(config.get("command")?.as_str()?.to_owned()),
                            args: string_list(config.get("args")),
                            env: strings(config.get("env")),
                            cwd: config.get("cwd").and_then(Value::as_str).map(str::to_owned),
                            ..Default::default()
                        }),
                        Some("streamable-http") => Some(McpSpec {
                            transport: McpKind::Http,
                            url: Some(config.get("url")?.as_str()?.to_owned()),
                            headers: strings(config.get("headers")),
                            ..Default::default()
                        }),
                        _ => None,
                    })
                    .flatten();
                Some((name, Live { spec, on: true }))
            })
            .collect())
    }

    fn entries_key(&self) -> &'static str {
        if self.app == McpApp::OpenCode {
            "mcp"
        } else {
            "mcpServers"
        }
    }

    /// The reader the agent's own file goes through: OpenCode and Pi take comments.
    fn client_id(&self) -> Option<TargetId> {
        match self.app {
            McpApp::OpenCode => Some(TargetId::OpenCode),
            McpApp::Pi => Some(TargetId::Pi),
            _ => None,
        }
    }

    fn json_document(&self, text: Option<&str>) -> Result<(Option<TargetId>, Value), String> {
        let id = self.client_id();
        let document = match (id, text) {
            (Some(id), text) => client_config::read_client_json(id, text)?,
            (None, Some(text)) if !text.trim().is_empty() => {
                serde_json::from_str(text.trim_start_matches('\u{feff}'))
                    .map_err(|_| "mcp.errors.configUnreadable")?
            }
            (None, _) => json!({}),
        };
        if !document.is_object()
            || document
                .get(self.entries_key())
                .is_some_and(|entries| !entries.is_object())
        {
            return Err(SHAPE.into());
        }
        Ok((id, document))
    }

    fn render(
        &self,
        id: Option<TargetId>,
        base: Option<&str>,
        document: &Value,
    ) -> Result<String, String> {
        match id {
            Some(id) => client_config::render_client_json(id, base, document),
            None => serde_json::to_string_pretty(document)
                .map(|text| format!("{text}\n"))
                .map_err(|_| "mcp.errors.write".into()),
        }
    }

    fn managed_keys(&self) -> &'static [&'static str] {
        match self.app {
            McpApp::OpenCode => &[
                "type",
                "command",
                "environment",
                "url",
                "headers",
                "enabled",
            ],
            McpApp::Pi => &[
                "type", "command", "args", "env", "cwd", "url", "headers", "enabled",
            ],
            _ => &["type", "command", "args", "env", "url", "headers"],
        }
    }

    /// The fields the page writes for `spec`, in the agent's own shape.
    fn native(&self, spec: &McpSpec) -> Map<String, Value> {
        let mut entry = Map::new();
        let command = spec.command.clone().unwrap_or_default();
        match (self.app, spec.transport) {
            (McpApp::OpenCode, McpKind::Stdio) => {
                entry.insert("type".into(), json!("local"));
                let mut line = vec![command];
                line.extend(spec.args.iter().cloned());
                entry.insert("command".into(), json!(line));
                if !spec.env.is_empty() {
                    entry.insert("environment".into(), json!(spec.env));
                }
                entry.insert("enabled".into(), json!(true));
            }
            (McpApp::OpenCode, _) => {
                entry.insert("type".into(), json!("remote"));
                entry.insert("url".into(), json!(spec.url));
                if !spec.headers.is_empty() {
                    entry.insert("headers".into(), json!(spec.headers));
                }
                entry.insert("enabled".into(), json!(true));
            }
            (_, McpKind::Stdio) => {
                if self.app == McpApp::ClaudeCode {
                    entry.insert("type".into(), json!("stdio"));
                }
                if self.app == McpApp::ClaudeCode && wants_cmd(&command) {
                    let mut args = vec!["/c".to_owned(), command];
                    args.extend(spec.args.iter().cloned());
                    entry.insert("command".into(), json!("cmd"));
                    entry.insert("args".into(), json!(args));
                } else {
                    entry.insert("command".into(), json!(command));
                    if !spec.args.is_empty() || self.app != McpApp::Pi {
                        entry.insert("args".into(), json!(spec.args));
                    }
                }
                if !spec.env.is_empty() {
                    entry.insert("env".into(), json!(spec.env));
                }
                if self.app == McpApp::Pi {
                    if let Some(cwd) = &spec.cwd {
                        entry.insert("cwd".into(), json!(cwd));
                    }
                }
            }
            (_, kind) => {
                if self.app == McpApp::ClaudeCode {
                    let kind = if kind == McpKind::Sse { "sse" } else { "http" };
                    entry.insert("type".into(), json!(kind));
                }
                entry.insert("url".into(), json!(spec.url));
                if !spec.headers.is_empty() {
                    entry.insert("headers".into(), json!(spec.headers));
                }
            }
        }
        if self.app == McpApp::Pi {
            entry.insert("enabled".into(), json!(true));
        }
        entry
    }

    fn parse_entry(&self, entry: &Value) -> Live {
        let on = !matches!(self.app, McpApp::OpenCode | McpApp::Pi)
            || entry.get("enabled").and_then(Value::as_bool) != Some(false);
        let spec = if self.app == McpApp::OpenCode {
            match entry.get("type").and_then(Value::as_str) {
                Some("remote") => entry.get("url").and_then(Value::as_str).map(|url| McpSpec {
                    transport: McpKind::Http,
                    url: Some(url.to_owned()),
                    headers: strings(entry.get("headers")),
                    ..Default::default()
                }),
                _ => {
                    let mut line = string_list(entry.get("command")).into_iter();
                    line.next().map(|command| McpSpec {
                        transport: McpKind::Stdio,
                        command: Some(command),
                        args: line.collect(),
                        env: strings(entry.get("environment")),
                        ..Default::default()
                    })
                }
            }
        } else if let Some(url) = entry.get("url").and_then(Value::as_str) {
            let sse = entry.get("type").and_then(Value::as_str) == Some("sse");
            Some(McpSpec {
                transport: if sse { McpKind::Sse } else { McpKind::Http },
                url: Some(url.to_owned()),
                headers: strings(entry.get("headers")),
                ..Default::default()
            })
        } else {
            entry.get("command").and_then(Value::as_str).map(|command| {
                let mut command = command.to_owned();
                let mut args = string_list(entry.get("args"));
                // Claude Code's `cmd /c npx …` on Windows is still `npx …`.
                if self.app == McpApp::ClaudeCode
                    && command.eq_ignore_ascii_case("cmd")
                    && args.len() >= 2
                    && args[0].eq_ignore_ascii_case("/c")
                {
                    command = args[1].clone();
                    args.drain(..2);
                }
                McpSpec {
                    transport: McpKind::Stdio,
                    command: Some(command),
                    args,
                    env: strings(entry.get("env")),
                    cwd: entry.get("cwd").and_then(Value::as_str).map(str::to_owned),
                    ..Default::default()
                }
            })
        };
        Live { spec, on }
    }

    /// Read the file, edit its text, and write the result when it changed:
    /// backed up first, and refused when the file changed meanwhile.
    fn edit_text(
        &self,
        backups: &Path,
        edit: impl FnOnce(Option<&str>) -> Result<Option<String>, String>,
    ) -> Result<(), String> {
        let before = read_text(&self.path)?;
        let Some(after) = edit(before.as_deref())? else {
            return Ok(());
        };
        if before.as_deref() == Some(after.as_str()) {
            return Ok(());
        }
        if read_text(&self.path)? != before {
            return Err("mcp.errors.changedOutside".into());
        }
        if let Some(before) = &before {
            std::fs::create_dir_all(backups).map_err(|_| "mcp.errors.write")?;
            crate::core::secret_file::write(
                &backups.join(format!("{}.dat", self.app.key())),
                before.as_bytes(),
            )?;
        }
        crate::core::storage::write_atomic(&self.path, after.as_bytes())
            .map_err(|_| "mcp.errors.write".into())
    }
}

/// Claude Code's former switch: AgentHarbor once listed turned-off servers in a
/// top-level `disabledMcpServers`, which Claude Code does not read. A server
/// the page writes or takes out is dropped from it.
fn forget_disabled(root: &mut Map<String, Value>, name: &str) -> bool {
    let Some(list) = root
        .get_mut("disabledMcpServers")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = list.len();
    list.retain(|value| value.as_str() != Some(name));
    let changed = list.len() != before;
    if list.is_empty() {
        root.remove("disabledMcpServers");
    }
    changed
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("mcp.errors.configUnreadable".into()),
    }
}

/// The keys of a Codex server table the page sets for `spec`.
fn codex_table(spec: &McpSpec) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("enabled".into(), true.into());
    let map = |values: &BTreeMap<String, String>| {
        toml::Value::Table(
            values
                .iter()
                .map(|(key, value)| (key.clone(), toml::Value::String(value.clone())))
                .collect(),
        )
    };
    match spec.transport {
        McpKind::Stdio => {
            table.insert(
                "command".into(),
                spec.command.clone().unwrap_or_default().into(),
            );
            if !spec.args.is_empty() {
                table.insert(
                    "args".into(),
                    toml::Value::Array(
                        spec.args.iter().cloned().map(toml::Value::String).collect(),
                    ),
                );
            }
            if !spec.env.is_empty() {
                table.insert("env".into(), map(&spec.env));
            }
            if let Some(cwd) = &spec.cwd {
                table.insert("cwd".into(), cwd.clone().into());
            }
        }
        McpKind::Http | McpKind::Sse => {
            table.insert("url".into(), spec.url.clone().unwrap_or_default().into());
            if !spec.headers.is_empty() {
                table.insert("http_headers".into(), map(&spec.headers));
            }
        }
    }
    table
}

const HARNESS_KEYS: [&str; 8] = [
    "serverName",
    "transport",
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "headers",
];

fn harness_config(name: &str, spec: &McpSpec) -> Map<String, Value> {
    let mut config = Map::new();
    config.insert("serverName".into(), json!(name));
    match spec.transport {
        McpKind::Stdio => {
            config.insert("transport".into(), json!("stdio"));
            config.insert("command".into(), json!(spec.command));
            config.insert("args".into(), json!(spec.args));
            if !spec.env.is_empty() {
                config.insert("env".into(), json!(spec.env));
            }
            if let Some(cwd) = &spec.cwd {
                config.insert("cwd".into(), json!(cwd));
            }
        }
        McpKind::Http | McpKind::Sse => {
            config.insert("transport".into(), json!("streamable-http"));
            config.insert("url".into(), json!(spec.url));
            if !spec.headers.is_empty() {
                config.insert("headers".into(), json!(spec.headers));
            }
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Vec<Target>, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let codex = CodexPaths::from_home(temp.path().join(".codex"));
        let clients = ClientPaths::from_home(temp.path());
        let desktop = temp.path().join("Claude/claude_desktop_config.json");
        for dir in [
            codex.codex_home.clone(),
            temp.path().join(".claude"),
            temp.path().join(".config/opencode"),
            temp.path().join(".pi/agent"),
            temp.path().join(".cursor"),
            temp.path().join(".dsh"),
            temp.path().join("Claude"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let backups = temp.path().join("backups");
        (temp, targets(&codex, &clients, Some(desktop)), backups)
    }

    fn stdio() -> McpSpec {
        McpSpec {
            transport: McpKind::Stdio,
            command: Some("uvx".into()),
            args: vec!["mcp-server-fetch".into()],
            env: BTreeMap::from([("TOKEN".into(), "${TOKEN}".into())]),
            ..Default::default()
        }
    }

    fn remote(kind: McpKind) -> McpSpec {
        McpSpec {
            transport: kind,
            url: Some("https://example.invalid/mcp".into()),
            headers: BTreeMap::from([("Authorization".into(), "Bearer test-only".into())]),
            ..Default::default()
        }
    }

    #[test]
    fn every_agent_reads_back_what_it_was_written() {
        let (_temp, targets, backups) = fixture();
        assert!(targets.iter().all(|target| target.installed));
        for target in &targets {
            for spec in [stdio(), remote(McpKind::Http), remote(McpKind::Sse)] {
                if target.unsupported("fetch", &spec).is_some() {
                    continue;
                }
                target.write("fetch", &spec, &backups).unwrap();
                let live = target.read().unwrap().remove("fetch").unwrap();
                assert!(live.on, "{:?}", target.app);
                assert!(
                    target.same(live.spec.as_ref().unwrap(), &spec),
                    "{:?}: {:?} vs {spec:?}",
                    target.app,
                    live.spec
                );
            }
            target.remove("fetch", &backups).unwrap();
            assert!(
                !target.read().unwrap().contains_key("fetch"),
                "{:?}",
                target.app
            );
            // Taking out what is not there is done.
            target.remove("fetch", &backups).unwrap();
        }
    }

    #[test]
    fn writes_keep_other_servers_unmanaged_fields_and_comments() {
        let (temp, targets, backups) = fixture();
        let claude = &targets[1];
        std::fs::write(
            &claude.path,
            json!({"oauthAccount": {"id": "keep"}, "projects": {"C:/w": {"mcpServers": {"p": {"command": "x"}}}},
                "mcpServers": {"fetch": {"type": "http", "url": "https://old.invalid", "oauth": {"clientId": "keep"}},
                "neighbor": {"command": "keep"}}, "disabledMcpServers": ["fetch", "other"]})
            .to_string(),
        )
        .unwrap();
        claude
            .write("fetch", &remote(McpKind::Http), &backups)
            .unwrap();
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(&claude.path).unwrap()).unwrap();
        assert_eq!(raw["oauthAccount"]["id"], "keep");
        assert_eq!(raw["projects"]["C:/w"]["mcpServers"]["p"]["command"], "x");
        assert_eq!(raw["mcpServers"]["fetch"]["oauth"]["clientId"], "keep");
        assert_eq!(raw["mcpServers"]["neighbor"]["command"], "keep");
        assert_eq!(
            raw["disabledMcpServers"],
            json!(["other"]),
            "the old switch is dropped"
        );
        assert!(backups.join("claude-code.dat").exists());

        let opencode = &targets[2];
        std::fs::write(
            &opencode.path,
            "{\n  // the user's note\n  \"theme\": \"dark\",\n  \"mcp\": {\"mine\": {\"type\": \"local\", \"command\": [\"node\"], \"timeout\": 5000}},\n}\n",
        )
        .unwrap();
        opencode.write("mine", &stdio(), &backups).unwrap();
        let text = std::fs::read_to_string(&opencode.path).unwrap();
        assert!(text.contains("// the user's note"), "{text}");
        let raw: Value = json5::from_str(&text).unwrap();
        assert_eq!(raw["mcp"]["mine"]["timeout"], 5000);
        assert_eq!(
            raw["mcp"]["mine"]["command"],
            json!(["uvx", "mcp-server-fetch"])
        );
        assert_eq!(raw["mcp"]["mine"]["environment"]["TOKEN"], "${TOKEN}");

        let harness = &targets[5];
        std::fs::write(
            &harness.path,
            "# my patch\n- id: mcp-github\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: github\n    transport: stdio\n    command: old\n    toolCallTimeoutMs: 9000\n",
        )
        .unwrap();
        harness.write("github", &stdio(), &backups).unwrap();
        harness
            .write("fetch", &remote(McpKind::Http), &backups)
            .unwrap();
        let text = std::fs::read_to_string(&harness.path).unwrap();
        assert!(text.contains("# my patch"), "{text}");
        let live = harness.read().unwrap();
        assert!(harness.same(live["github"].spec.as_ref().unwrap(), &stdio()));
        let rows = harness_mcp_rows::read(Some(&text)).unwrap();
        assert_eq!(rows[0].1["toolCallTimeoutMs"], 9000);
        assert_eq!(rows[1].0, "mcp-fetch");
        drop(temp);
    }

    #[test]
    fn what_an_agent_cannot_hold_is_named() {
        let (_temp, targets, _) = fixture();
        let sse = remote(McpKind::Sse);
        let reasons: Vec<_> = targets
            .iter()
            .map(|target| target.unsupported("fetch", &sse))
            .collect();
        assert_eq!(reasons[0], Some("mcp.unsupported.sse"), "Codex");
        assert_eq!(reasons[1], None, "Claude Code");
        assert_eq!(reasons[3], Some("mcp.unsupported.sse"), "Pi");
        assert_eq!(reasons[6], Some("mcp.unsupported.remote"), "Claude Desktop");
        assert_eq!(
            targets[5].unsupported("my.server", &stdio()),
            Some("mcp.unsupported.harnessName")
        );
        assert_eq!(targets[5].unsupported("my-server", &stdio()), None);
    }

    #[test]
    fn specs_are_cleaned_to_their_transport() {
        let mut spec = stdio();
        spec.url = Some("https://x.invalid".into());
        spec.command = Some("  uvx ".into());
        let clean = spec.clean().unwrap();
        assert_eq!(clean.command.as_deref(), Some("uvx"));
        assert_eq!(clean.url, None);
        assert_eq!(
            McpSpec::default().clean().unwrap_err(),
            "mcp.errors.command"
        );
        let mut bad = remote(McpKind::Http);
        bad.url = Some("file:///private".into());
        assert_eq!(bad.clean().unwrap_err(), "mcp.errors.url");
    }

    #[test]
    fn a_broken_file_is_reported_and_left_alone() {
        let (_temp, targets, backups) = fixture();
        let cursor = &targets[4];
        for broken in ["{", "[]", r#"{"mcpServers":[]}"#] {
            std::fs::write(&cursor.path, broken).unwrap();
            assert!(cursor.read().is_err());
            assert!(cursor.write("fetch", &stdio(), &backups).is_err());
            assert_eq!(std::fs::read_to_string(&cursor.path).unwrap(), broken);
        }
    }
}
