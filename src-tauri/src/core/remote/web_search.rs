//! Web search on a remote host as an MCP server: a small Python script
//! (standard library only) in `~/.agentharbor/web-search`, its backend and
//! key in a `config.json` beside it readable by its owner only, registered
//! with Codex and Claude Code through their own `mcp add`. The agents then
//! search through the relay's own service, with no router in between:
//! DeepSeek's search API, or MiMo's search on its Chat API.

use super::{connection::quote, probe::FIND, RemoteSession};
use crate::core::{
    newapi,
    protocol::ChatSearch,
    router::{self, ModelProtocol, Provider, SearchApi, Store},
    secret_file,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const JOURNAL_FILE: &str = "web-search.dat";
const DIR: &str = ".agentharbor/web-search";
const SCRIPT: &str = include_str!("web_search_mcp.py");
const DEEPSEEK_SEARCH: &str = "https://api.deepseek.com/v1/tools/search";
/// The names the server is registered under (Codex takes no hyphen in TOML keys bare).
const CLAUDE_NAME: &str = "agentharbor-search";
const CODEX_NAME: &str = "agentharbor_search";
const LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    /// DeepSeek's search API: ranked web pages.
    DeepSeek,
    /// MiMo's own search on its Chat API: a sourced answer.
    Mimo,
}

/// A relay card whose own search service can back the server.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProvider {
    pub id: String,
    pub name: String,
    pub backend: Backend,
    /// For MiMo, the models that can run the search.
    pub models: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchRequest {
    pub provider_id: String,
    /// The MiMo model that searches; unused for DeepSeek.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    request: WebSearchRequest,
    backend: Backend,
    /// The clients it was registered with.
    codex: bool,
    claude_code: bool,
    /// The backend's address, key and model as written.
    connection: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchStatus {
    pub deployed: bool,
    pub path: String,
    pub request: Option<WebSearchRequest>,
    pub backend: Option<Backend>,
    /// Registered with each client, as their configuration says now.
    pub codex: bool,
    pub claude_code: bool,
    /// The card changed (or is gone) since the key was written.
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTest {
    pub ok: bool,
    pub output: String,
}

fn backend_of(provider: &Provider) -> Option<Backend> {
    if provider.router_only() || !provider.has_route_credentials() {
        return None;
    }
    if router::search_api(provider) == Some(SearchApi::DeepSeek) {
        return Some(Backend::DeepSeek);
    }
    (ChatSearch::for_endpoint(&provider.base_url, &provider.model) == Some(ChatSearch::Mimo))
        .then_some(Backend::Mimo)
}

fn mimo_models(provider: &Provider) -> Vec<String> {
    let mut models: Vec<String> = std::iter::once(provider.model.clone())
        .chain(
            provider
                .models
                .iter()
                .filter(|model| model.included)
                .map(|model| model.id.clone()),
        )
        .filter(|model| !model.is_empty() && provider.supports_model(model))
        .collect();
    models.sort();
    models.dedup();
    models
}

/// The cards that can back the server.
pub fn providers(store: &Store) -> Vec<SearchProvider> {
    store
        .providers
        .iter()
        .filter_map(|provider| {
            let backend = backend_of(provider)?;
            Some(SearchProvider {
                id: provider.id.clone(),
                name: provider.name.clone(),
                backend,
                models: if backend == Backend::Mimo {
                    mimo_models(provider)
                } else {
                    Vec::new()
                },
            })
        })
        .collect()
}

/// The server's `config.json` for `request`, and its fingerprint.
fn config(store: &Store, request: &WebSearchRequest) -> Result<(Backend, Value, String), String> {
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == request.provider_id)
        .ok_or("clients.errors.provider")?;
    let backend = backend_of(provider).ok_or("remote.errors.searchProvider")?;
    let key = newapi::api_key_header(&provider.api_key)?
        .to_str()
        .map_err(|_| "newapi.errors.token")?
        .to_owned();
    let value = match backend {
        Backend::DeepSeek => {
            json!({ "backend": "deepseek", "endpoint": DEEPSEEK_SEARCH, "key": key })
        }
        Backend::Mimo => {
            let model = request.model.clone().ok_or("clients.errors.model")?;
            if !mimo_models(provider).contains(&model) {
                return Err("clients.errors.model".into());
            }
            let base =
                newapi::protocol_base_url(&provider.base_url, ModelProtocol::ChatCompletions)?;
            let endpoint = newapi::provider_endpoint(base.as_str(), "v1/chat/completions")?;
            json!({ "backend": "mimo", "endpoint": endpoint.as_str(), "key": key, "model": model })
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(value.to_string().as_bytes());
    Ok((backend, value, format!("{:x}", hasher.finalize())))
}

pub struct WebSearch<'a> {
    session: &'a RemoteSession,
    home: String,
    journal: PathBuf,
}

impl<'a> WebSearch<'a> {
    pub fn new(session: &'a RemoteSession, journal_dir: &Path) -> Self {
        Self {
            session,
            home: session.home().trim_end_matches('/').to_owned(),
            journal: journal_dir.join(JOURNAL_FILE),
        }
    }

    fn path(&self, name: &str) -> String {
        format!("{}/{DIR}/{name}", self.home)
    }

    fn load_journal(&self) -> Result<Option<Journal>, String> {
        secret_file::read(&self.journal)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|_| "remote.errors.storage".to_owned())
            })
            .transpose()
    }

    /// Runs `script` on the host after the `find` helper, returning its
    /// exit status and output.
    fn run(&self, script: &str) -> Result<(u32, String), String> {
        self.session.run(&format!("{FIND}; {script}"), LIMIT)
    }

    fn python(&self) -> Result<String, String> {
        let (_, output) =
            self.run("p=/usr/bin/python3; [ -x \"$p\" ] || p=$(find python3); printf %s \"$p\"")?;
        let python = output.trim().to_owned();
        if python.is_empty() {
            return Err("remote.errors.pythonMissing".into());
        }
        Ok(python)
    }

    /// Registers the server with `cli` (`codex` or `claude`); `Ok(false)`
    /// when that CLI is not installed.
    fn register(&self, cli: &str, python: &str) -> Result<bool, String> {
        let (name, scope) = if cli == "claude" {
            (CLAUDE_NAME, "--scope user ")
        } else {
            (CODEX_NAME, "")
        };
        let script = format!(
            "b=$(find {cli}); [ -n \"$b\" ] || exit 3; \"$b\" mcp remove {scope}{name} >/dev/null 2>&1; \"$b\" mcp add {scope}{name} -- {} {} 2>&1",
            quote(python),
            quote(&self.path("server.py")),
        );
        match self.run(&script)? {
            (0, _) if cli == "codex" => self.approve_in_codex().map(|_| true),
            (0, _) => Ok(true),
            (3, _) => Ok(false),
            (_, output) => {
                log::warn!("{cli} mcp add failed on the remote host: {}", output.trim());
                Err("remote.errors.searchRegister".into())
            }
        }
    }

    /// Codex asks before every MCP tool call, which `codex exec` cannot
    /// answer. Searching changes nothing, so the tool is approved in the
    /// server's own entry, which `codex mcp remove` takes away whole.
    fn approve_in_codex(&self) -> Result<(), String> {
        let files = self.session as &dyn crate::core::client_config::ClientFiles;
        let path = format!("{}/config.toml", self.session.codex_home());
        let text = files.read(Path::new(&path))?.unwrap_or_default();
        let mut document: toml_edit::DocumentMut =
            text.parse().map_err(|_| "remote.errors.codexConfig")?;
        if document
            .get("mcp_servers")
            .and_then(|servers| servers.get(CODEX_NAME))
            .is_none()
        {
            return Err("remote.errors.searchRegister".into());
        }
        document["mcp_servers"][CODEX_NAME]["tools"]["web_search"]["approval_mode"] =
            toml_edit::value("approve");
        files.write(Path::new(&path), document.to_string().as_bytes())
    }

    fn unregister(&self, cli: &str) -> Result<(), String> {
        let (name, scope) = if cli == "claude" {
            (CLAUDE_NAME, "--scope user ")
        } else {
            (CODEX_NAME, "")
        };
        self.run(&format!("b=$(find {cli}); [ -n \"$b\" ] || exit 0; \"$b\" mcp remove {scope}{name} >/dev/null 2>&1; exit 0"))?;
        Ok(())
    }

    /// Writes the server and its key, and registers it with every
    /// installed client. Deploying again replaces the backend in place.
    pub fn deploy(&self, store: &Store, request: &WebSearchRequest) -> Result<(), String> {
        let (backend, config, connection) = config(store, request)?;
        let python = self.python()?;
        let files = self.session as &dyn crate::core::client_config::ClientFiles;
        files.write(Path::new(&self.path("server.py")), SCRIPT.as_bytes())?;
        files.write(
            Path::new(&self.path("config.json")),
            serde_json::to_string_pretty(&config)
                .map_err(|_| "remote.errors.storage")?
                .as_bytes(),
        )?;
        let codex = self.register("codex", &python)?;
        let claude_code = self.register("claude", &python)?;
        if !codex && !claude_code {
            return Err("remote.errors.searchNoClient".into());
        }
        let journal = Journal {
            request: request.clone(),
            backend,
            codex,
            claude_code,
            connection,
        };
        secret_file::write(
            &self.journal,
            &serde_json::to_vec(&journal).map_err(|_| "remote.errors.storage")?,
        )
    }

    /// Unregisters the server and removes its files and key.
    pub fn remove(&self) -> Result<(), String> {
        let journal = self.load_journal()?.ok_or("clients.errors.notConfigured")?;
        if journal.codex {
            self.unregister("codex")?;
        }
        if journal.claude_code {
            self.unregister("claude")?;
        }
        let files = self.session as &dyn crate::core::client_config::ClientFiles;
        files.remove(Path::new(&self.path("config.json")))?;
        files.remove(Path::new(&self.path("server.py")))?;
        super::remove_journal(&self.journal)
    }

    pub fn status(&self, store: Option<&Store>) -> WebSearchStatus {
        let mut status = WebSearchStatus {
            deployed: false,
            path: self.path("server.py"),
            request: None,
            backend: None,
            codex: false,
            claude_code: false,
            stale: false,
            error: None,
        };
        let result = (|| -> Result<(), String> {
            let Some(journal) = self.load_journal()? else {
                return Ok(());
            };
            status.deployed = true;
            let files = self.session as &dyn crate::core::client_config::ClientFiles;
            // What each client's own configuration says now.
            status.codex = files
                .read(Path::new(&format!(
                    "{}/config.toml",
                    self.session.codex_home()
                )))?
                .and_then(|text| text.parse::<toml::Table>().ok())
                .is_some_and(|config| {
                    config
                        .get("mcp_servers")
                        .and_then(|servers| servers.get(CODEX_NAME))
                        .is_some()
                });
            status.claude_code = files
                .read(&self.session.client_paths().claude_user)?
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .is_some_and(|config| config["mcpServers"].get(CLAUDE_NAME).is_some());
            if let Some(store) = store {
                status.stale = config(store, &journal.request)
                    .map(|(_, _, connection)| connection != journal.connection)
                    .unwrap_or(true);
            }
            status.backend = Some(journal.backend);
            status.request = Some(journal.request);
            Ok(())
        })();
        status.error = result.err();
        status
    }

    /// Asks the deployed server, as an agent would, to search for `query`.
    pub fn test(&self, query: &str) -> Result<SearchTest, String> {
        let call = json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "web_search", "arguments": { "query": query, "count": 3 } },
        });
        let lines = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}).to_string(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            call.to_string(),
        ];
        let python = self.python()?;
        let input = lines
            .iter()
            .map(|line| quote(line))
            .collect::<Vec<_>>()
            .join(" ");
        let (_, output) = self.run(&format!(
            "printf '%s\\n' {input} | {} {} 2>&1",
            quote(&python),
            quote(&self.path("server.py"))
        ))?;
        let reply = output
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|reply| reply["id"] == 2)
            .ok_or("remote.errors.searchTest")?;
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(SearchTest {
            ok: reply["result"]["isError"] == false && !text.is_empty(),
            output: text.chars().take(4000).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::newapi;

    fn card(id: &str, base: &str) -> Provider {
        Provider {
            id: id.into(),
            name: id.into(),
            base_url: base.into(),
            api_key: "Bearer sk-test".into(),
            enabled: true,
            model: "model-a".into(),
            models: vec![newapi::Model {
                id: "model-a".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn deepseek_and_mimo_cards_back_the_server() {
        let store = Store {
            providers: vec![
                card("deepseek", "https://api.deepseek.com/"),
                card("mimo", "https://api.xiaomimimo.com/"),
                card("other", "https://relay.example.com/"),
            ],
            ..Default::default()
        };
        let offered = providers(&store);
        assert_eq!(
            offered
                .iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>(),
            ["deepseek", "mimo"]
        );
        let (backend, value, _) = config(
            &store,
            &WebSearchRequest {
                provider_id: "deepseek".into(),
                model: None,
            },
        )
        .unwrap();
        assert!(backend == Backend::DeepSeek);
        assert_eq!(value["endpoint"], DEEPSEEK_SEARCH);
        assert_eq!(value["key"], "sk-test", "a pasted Bearer prefix is dropped");
        let mimo = WebSearchRequest {
            provider_id: "mimo".into(),
            model: Some("model-a".into()),
        };
        let (backend, value, _) = config(&store, &mimo).unwrap();
        assert!(backend == Backend::Mimo);
        assert_eq!(
            value["endpoint"],
            "https://api.xiaomimimo.com/v1/chat/completions"
        );
        assert_eq!(value["model"], "model-a");
        assert!(config(
            &store,
            &WebSearchRequest {
                model: None,
                ..mimo
            }
        )
        .is_err());
        assert!(config(
            &store,
            &WebSearchRequest {
                provider_id: "other".into(),
                model: None
            }
        )
        .is_err());
    }
}
