//! Remote hosts: machines reached over SSH whose Codex and Claude Code
//! configuration this app edits in place. Hosts and their secrets (a
//! password, a key passphrase) are kept like relay keys: encrypted for the
//! current user and never shown again. Remote clients are configured direct
//! only: they reach their relay themselves, so nothing waits on this app.

pub mod codex;
mod connection;
pub mod ssh_config;

use super::{client_config::ClientFiles, secret_file, storage};
pub use connection::ConnectError;
use connection::{block_on, Connection, KnownHosts, Method, Target};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const HOSTS_FILE: &str = "remote-hosts.dat";
const KNOWN_HOSTS_FILE: &str = "remote-known-hosts";
const JOURNAL_DIR: &str = "remote";
const DEFAULT_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteAuth {
    /// A private key: the one named, else `~/.ssh/config`'s, else the
    /// usual `~/.ssh/id_*` files.
    #[default]
    Key,
    Password,
}

// Deliberately no Debug: these records carry the host's secrets.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredHost {
    id: String,
    name: String,
    host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(default)]
    auth: RemoteAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<String>,
}

/// What `~/.ssh/config` and the defaults make of a host.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedHost {
    pub host_name: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_files: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHostView {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub auth: RemoteAuth,
    pub identity_file: Option<String>,
    pub has_password: bool,
    pub has_passphrase: bool,
    pub resolved: ResolvedHost,
    /// Whether any client on this host is configured by this app.
    pub in_use: bool,
}

/// A host as edited. Secrets left empty keep what is stored.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHostInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub auth: RemoteAuth,
    #[serde(default)]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub clear_passphrase: bool,
}

/// The outcome of a connection test, with what the user must confirm when
/// the host is new.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ConnectionTest {
    #[serde(rename_all = "camelCase")]
    Connected { home: String },
    #[serde(rename_all = "camelCase")]
    UnknownHost {
        fingerprint: String,
        algorithm: String,
    },
    #[serde(rename_all = "camelCase")]
    HostKeyChanged { fingerprint: String },
    #[serde(rename_all = "camelCase")]
    Failed { error: String },
}

pub struct RemoteStore {
    data_dir: PathBuf,
}

fn lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().map_err(|_| "remote.errors.storage".into())
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn valid_word(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl RemoteStore {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            data_dir: app_data_dir.to_path_buf(),
        }
    }

    fn load(&self) -> Result<Vec<StoredHost>, String> {
        match secret_file::read(&self.data_dir.join(HOSTS_FILE))? {
            Some(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "remote.errors.storage".into())
            }
            None => Ok(Vec::new()),
        }
    }

    fn save(&self, hosts: &[StoredHost]) -> Result<(), String> {
        let bytes = serde_json::to_vec(hosts).map_err(|_| "remote.errors.storage")?;
        secret_file::write(&self.data_dir.join(HOSTS_FILE), &bytes)
    }

    /// The journals of the clients configured on host `id`.
    pub fn journal_dir(&self, id: &str) -> Result<PathBuf, String> {
        let id = uuid::Uuid::parse_str(id).map_err(|_| "remote.errors.hostMissing")?;
        Ok(self.data_dir.join(JOURNAL_DIR).join(id.to_string()))
    }

    fn in_use(&self, id: &str) -> bool {
        self.journal_dir(id).is_ok_and(|dir| {
            ["claude-code.dat", codex::JOURNAL_FILE]
                .iter()
                .any(|name| dir.join(name).exists())
        })
    }

    pub fn list(&self) -> Result<Vec<RemoteHostView>, String> {
        let config = ssh_config::SshConfig::load();
        Ok(self
            .load()?
            .iter()
            .map(|host| self.view(host, &config))
            .collect())
    }

    fn view(&self, host: &StoredHost, config: &ssh_config::SshConfig) -> RemoteHostView {
        let resolved = resolve(host, config);
        RemoteHostView {
            id: host.id.clone(),
            name: host.name.clone(),
            host: host.host.clone(),
            port: host.port,
            user: host.user.clone(),
            auth: host.auth,
            identity_file: host.identity_file.clone(),
            has_password: host.password.is_some(),
            has_passphrase: host.passphrase.is_some(),
            resolved: ResolvedHost {
                host_name: resolved.host_name,
                port: resolved.port,
                user: resolved.user,
                identity_files: resolved
                    .identity_files
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            },
            in_use: self.in_use(&host.id),
        }
    }

    pub fn save_host(&self, input: RemoteHostInput) -> Result<RemoteHostView, String> {
        let _guard = lock()?;
        let name = input.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err("remote.errors.name".into());
        }
        let host = input.host.trim().to_owned();
        if !valid_word(&host, 255) {
            return Err("remote.errors.host".into());
        }
        let user = clean(input.user);
        if user.as_deref().is_some_and(|user| !valid_word(user, 64)) {
            return Err("remote.errors.user".into());
        }
        if input.port == Some(0) {
            return Err("remote.errors.port".into());
        }
        let identity_file = clean(input.identity_file);
        if identity_file
            .as_deref()
            .is_some_and(|path| path.chars().any(char::is_control))
        {
            return Err("remote.errors.identityFile".into());
        }
        let mut hosts = self.load()?;
        let previous = match &input.id {
            Some(id) => Some(
                hosts
                    .iter()
                    .position(|host| &host.id == id)
                    .ok_or("remote.errors.hostMissing")?,
            ),
            None => None,
        };
        let existing = previous.map(|index| hosts[index].clone());
        let password = match input.auth {
            RemoteAuth::Password => clean(input.password)
                .or_else(|| existing.as_ref().and_then(|host| host.password.clone())),
            // A host that signs in with a key keeps no password.
            RemoteAuth::Key => None,
        };
        if input.auth == RemoteAuth::Password && password.is_none() {
            return Err("remote.errors.passwordMissing".into());
        }
        let passphrase = if input.clear_passphrase || input.auth == RemoteAuth::Password {
            None
        } else {
            clean(input.passphrase)
                .or_else(|| existing.as_ref().and_then(|host| host.passphrase.clone()))
        };
        let stored = StoredHost {
            id: existing
                .as_ref()
                .map(|host| host.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            host,
            port: input.port,
            user,
            auth: input.auth,
            identity_file,
            password,
            passphrase,
        };
        match previous {
            Some(index) => hosts[index] = stored.clone(),
            None => hosts.push(stored.clone()),
        }
        self.save(&hosts)?;
        Ok(self.view(&stored, &ssh_config::SshConfig::load()))
    }

    /// Forgets a host. One with configured clients is refused: without its
    /// journals they could no longer be restored.
    pub fn delete_host(&self, id: &str) -> Result<(), String> {
        let _guard = lock()?;
        if self.in_use(id) {
            return Err("remote.errors.hostInUse".into());
        }
        let mut hosts = self.load()?;
        let before = hosts.len();
        hosts.retain(|host| host.id != id);
        if hosts.len() == before {
            return Err("remote.errors.hostMissing".into());
        }
        self.save(&hosts)?;
        if let Ok(dir) = self.journal_dir(id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    fn known_hosts(&self) -> Arc<KnownHosts> {
        Arc::new(KnownHosts {
            user: dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts")),
            app: self.data_dir.join(KNOWN_HOSTS_FILE),
        })
    }

    /// Opens a session to host `id`. `trust` is a host key fingerprint the
    /// user confirmed; a new host is refused without it.
    pub fn connect(&self, id: &str, trust: Option<String>) -> Result<RemoteSession, ConnectError> {
        let host = self
            .load()
            .map_err(ConnectError::Failed)?
            .into_iter()
            .find(|host| host.id == id)
            .ok_or_else(|| ConnectError::Failed("remote.errors.hostMissing".into()))?;
        let resolved = resolve(&host, &ssh_config::SshConfig::load());
        let user = resolved
            .user
            .ok_or_else(|| ConnectError::Failed("remote.errors.userMissing".into()))?;
        let method = match host.auth {
            RemoteAuth::Password => Method::Password(
                host.password
                    .clone()
                    .ok_or_else(|| ConnectError::Failed("remote.errors.passwordMissing".into()))?,
            ),
            RemoteAuth::Key => {
                let files: Vec<PathBuf> = resolved
                    .identity_files
                    .into_iter()
                    .filter(|path| path.is_file())
                    .collect();
                if files.is_empty() {
                    return Err(ConnectError::Failed("remote.errors.noKey".into()));
                }
                Method::Keys {
                    files,
                    passphrase: host.passphrase.clone(),
                }
            }
        };
        let target = Target {
            host_name: resolved.host_name,
            alias: host.host.clone(),
            port: resolved.port,
            user,
            method,
        };
        let known = self.known_hosts();
        let connection = block_on(Connection::open(&target, known, trust))?;
        Ok(RemoteSession {
            connection: Some(connection),
        })
    }

    pub fn test(&self, id: &str, trust: Option<String>) -> ConnectionTest {
        match self.connect(id, trust) {
            Ok(session) => ConnectionTest::Connected {
                home: session.home().to_owned(),
            },
            Err(ConnectError::UnknownHost {
                fingerprint,
                algorithm,
            }) => ConnectionTest::UnknownHost {
                fingerprint,
                algorithm,
            },
            Err(ConnectError::HostKeyChanged { fingerprint }) => {
                ConnectionTest::HostKeyChanged { fingerprint }
            }
            Err(ConnectError::Failed(error)) => ConnectionTest::Failed { error },
        }
    }
}

struct Resolved {
    host_name: String,
    port: u16,
    user: Option<String>,
    identity_files: Vec<PathBuf>,
}

fn resolve(host: &StoredHost, config: &ssh_config::SshConfig) -> Resolved {
    let from_config = config.resolve(&host.host);
    let host_name = from_config.host_name.unwrap_or_else(|| host.host.clone());
    let user = host.user.clone().or(from_config.user);
    let home = dirs::home_dir().unwrap_or_default();
    let remote_user = user.clone().unwrap_or_default();
    let identity_files = match &host.identity_file {
        Some(file) => vec![ssh_config::expand_identity(
            file,
            &home,
            &host_name,
            &remote_user,
        )],
        None if !from_config.identity_files.is_empty() => from_config
            .identity_files
            .iter()
            .map(|file| ssh_config::expand_identity(file, &home, &host_name, &remote_user))
            .collect(),
        None => DEFAULT_KEYS
            .iter()
            .map(|name| home.join(".ssh").join(name))
            .collect(),
    };
    Resolved {
        host_name,
        port: host.port.or(from_config.port).unwrap_or(22),
        user,
        identity_files,
    }
}

/// The concrete aliases of `~/.ssh/config`, offered when adding a host.
pub fn ssh_aliases() -> Vec<String> {
    ssh_config::SshConfig::load().aliases()
}

/// An open session. Closed when dropped.
pub struct RemoteSession {
    connection: Option<Connection>,
}

/// A remote POSIX path for `path`, which was built from one.
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl RemoteSession {
    /// A session that reaches nothing, for work on the journals alone.
    pub fn offline() -> Self {
        Self { connection: None }
    }

    pub fn home(&self) -> &str {
        self.connection.as_ref().map(Connection::home).unwrap_or("")
    }

    fn connection(&self) -> Result<&Connection, String> {
        self.connection
            .as_ref()
            .ok_or_else(|| "remote.errors.closed".into())
    }
}

impl ClientFiles for RemoteSession {
    fn read(&self, path: &Path) -> Result<Option<String>, String> {
        block_on(self.connection()?.read(&posix(path)))?
            .map(|bytes| String::from_utf8(bytes).map_err(|_| "clients.errors.read".to_owned()))
            .transpose()
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        block_on(self.connection()?.write(&posix(path), bytes))
    }

    fn remove(&self, path: &Path) -> Result<(), String> {
        block_on(self.connection()?.remove(&posix(path)))
    }
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            block_on(connection.close());
        }
    }
}

/// Removes a journal file the way the client configuration does.
pub(crate) fn remove_journal(path: &Path) -> Result<(), String> {
    if path.exists() {
        storage::remove_file_durable(path).map_err(|_| "remote.errors.storage".to_owned())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> RemoteHostInput {
        RemoteHostInput {
            id: None,
            name: name.into(),
            host: "dev".into(),
            port: None,
            user: Some("ubuntu".into()),
            auth: RemoteAuth::Password,
            identity_file: None,
            password: Some("secret".into()),
            passphrase: None,
            clear_passphrase: false,
        }
    }

    #[test]
    fn secrets_are_kept_but_never_shown() {
        let dir = std::env::temp_dir().join(format!("agentharbor-remote-{}", uuid::Uuid::new_v4()));
        let store = RemoteStore::new(&dir);
        let saved = store.save_host(input("Dev")).unwrap();
        assert!(saved.has_password);
        let json = serde_json::to_string(&store.list().unwrap()).unwrap();
        assert!(!json.contains("secret"));
        // An empty password keeps the stored one.
        let mut edit = input("Dev box");
        edit.id = Some(saved.id.clone());
        edit.password = Some(String::new());
        assert!(store.save_host(edit).unwrap().has_password);
        assert_eq!(store.load().unwrap()[0].password.as_deref(), Some("secret"));
        // Switching to a key drops the password.
        let mut edit = input("Dev box");
        edit.id = Some(saved.id.clone());
        edit.auth = RemoteAuth::Key;
        edit.passphrase = Some("words".into());
        let view = store.save_host(edit).unwrap();
        assert!(!view.has_password && view.has_passphrase);
        store.delete_host(&saved.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_password_host_needs_a_password() {
        let dir = std::env::temp_dir().join(format!("agentharbor-remote-{}", uuid::Uuid::new_v4()));
        let mut host = input("Dev");
        host.password = None;
        assert_eq!(
            RemoteStore::new(&dir).save_host(host).err().as_deref(),
            Some("remote.errors.passwordMissing")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hosts_resolve_through_ssh_config() {
        let config = ssh_config::SshConfig::parse(
            "Host dev\n HostName 10.1.2.3\n User admin\n Port 2200\n IdentityFile ~/.ssh/dev_key\n",
        );
        let stored = StoredHost {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Dev".into(),
            host: "dev".into(),
            port: None,
            user: None,
            auth: RemoteAuth::Key,
            identity_file: None,
            password: None,
            passphrase: None,
        };
        let resolved = resolve(&stored, &config);
        assert_eq!(resolved.host_name, "10.1.2.3");
        assert_eq!(resolved.port, 2200);
        assert_eq!(resolved.user.as_deref(), Some("admin"));
        assert!(
            resolved.identity_files[0].ends_with(".ssh/dev_key")
                || resolved.identity_files[0].ends_with(".ssh\\dev_key")
        );
    }
}

/// Against a real host: `AGENTHARBOR_TEST_REMOTE_HOST` names a host (a
/// `~/.ssh/config` alias works) signed in with a key; set
/// `AGENTHARBOR_TEST_REMOTE_PASSPHRASE` for an encrypted key and
/// `AGENTHARBOR_TEST_REMOTE_TRUST` to the fingerprint of a host not in
/// `~/.ssh/known_hosts`. Codex and Claude Code there are configured with a
/// synthetic relay and restored; both must be unconfigured to begin with.
#[cfg(test)]
mod real_remote {
    use super::*;
    use crate::core::{
        client_config::{ClientAccessMode, ClientOptions, ConfigManager, ConfigRequest},
        newapi,
        router::{ModelProtocol, Provider, Store},
        targets::TargetId,
    };

    const KEY: &str = "sk-agentharbor-remote-test-not-a-real-key";

    fn synthetic_relays() -> Store {
        let model = |id: &str, protocol| newapi::Model {
            id: id.into(),
            protocol: Some(protocol),
            ..Default::default()
        };
        let relay = |id: &str, protocol, model_id: &str| Provider {
            id: id.into(),
            name: format!("Remote test {id}"),
            base_url: "https://relay.agentharbor-test.invalid".into(),
            api_key: KEY.into(),
            enabled: true,
            protocol,
            model: model_id.into(),
            models: vec![model(model_id, protocol)],
            ..Default::default()
        };
        Store {
            providers: vec![
                relay("responses", ModelProtocol::Responses, "gpt-test"),
                relay("messages", ModelProtocol::AnthropicMessages, "claude-test"),
            ],
            ..Default::default()
        }
    }

    #[test]
    #[ignore]
    fn real_remote_codex_and_claude_code_apply_and_restore() {
        let host =
            std::env::var("AGENTHARBOR_TEST_REMOTE_HOST").expect("AGENTHARBOR_TEST_REMOTE_HOST");
        let data = tempfile::tempdir().unwrap();
        let store = RemoteStore::new(data.path());
        let saved = store
            .save_host(RemoteHostInput {
                id: None,
                name: host.clone(),
                host,
                port: None,
                user: None,
                auth: RemoteAuth::Key,
                identity_file: None,
                password: None,
                passphrase: std::env::var("AGENTHARBOR_TEST_REMOTE_PASSPHRASE").ok(),
                clear_passphrase: false,
            })
            .unwrap();
        let trust = std::env::var("AGENTHARBOR_TEST_REMOTE_TRUST").ok();
        let home = match store.test(&saved.id, trust.clone()) {
            ConnectionTest::Connected { home } => home,
            ConnectionTest::UnknownHost {
                fingerprint,
                algorithm,
            } => {
                panic!("unknown host key {algorithm} {fingerprint}: set AGENTHARBOR_TEST_REMOTE_TRUST after checking it")
            }
            ConnectionTest::HostKeyChanged { fingerprint } => {
                panic!("host key changed: {fingerprint}")
            }
            ConnectionTest::Failed { error } => panic!("connection failed: {error}"),
        };
        let session = Arc::new(store.connect(&saved.id, trust).unwrap());
        assert_eq!(session.home(), home);
        let journals = store.journal_dir(&saved.id).unwrap();
        let relays = synthetic_relays();
        let at = |rest: &str| PathBuf::from(format!("{home}/{rest}"));

        // Codex: config, key and catalog are written, then all taken back.
        let codex_config = at(".codex/config.toml");
        let before = session.read(&codex_config).unwrap();
        // This test's journals are its own: a host configured for real would
        // lose the key block it shares, so such a host is refused.
        assert!(
            !before
                .as_deref()
                .unwrap_or_default()
                .contains("agentharbor_remote"),
            "Codex on this host is configured by AgentHarbor; use an unconfigured host"
        );
        let env_before = session.read(&at(".codex/.env")).unwrap();
        let codex =
            codex::CodexRemote::new(session.as_ref(), &home, &journals, data.path(), Vec::new());
        assert!(
            !codex.status(Some(&relays)).configured,
            "Codex must start unconfigured"
        );
        let request = codex::CodexRemoteRequest {
            provider_id: "responses".into(),
            model: "gpt-test".into(),
            web_search: false,
        };
        let preview = codex.preview(&relays, &request, false).unwrap();
        assert!(!preview.content.contains(KEY));
        codex
            .apply(&relays, &request, &preview.revision, false)
            .unwrap();
        let written = session.read(&codex_config).unwrap().unwrap();
        let document: toml::Value = toml::from_str(&written).unwrap();
        assert_eq!(
            document["model_provider"].as_str(),
            Some("agentharbor_remote")
        );
        assert_eq!(document["web_search"].as_str(), Some("disabled"));
        assert_eq!(
            document["model_providers"]["agentharbor_remote"]["base_url"].as_str(),
            Some("https://relay.agentharbor-test.invalid/v1")
        );
        let env = session.read(&at(".codex/.env")).unwrap().unwrap();
        assert!(env.contains(KEY));
        assert!(session
            .read(&at(".codex/agentharbor-models.json"))
            .unwrap()
            .is_some());
        let status = codex.status(Some(&relays));
        assert!(
            status.configured && !status.conflict && !status.stale,
            "{:?}",
            status.conflict_fields
        );
        codex.restore(false).unwrap();
        assert_eq!(session.read(&codex_config).unwrap(), before);
        assert_eq!(session.read(&at(".codex/.env")).unwrap(), env_before);
        assert!(session
            .read(&at(".codex/agentharbor-models.json"))
            .unwrap()
            .is_none());

        // Claude Code through the shared configuration engine, direct.
        let claude_settings = at(".claude/settings.json");
        let before = session.read(&claude_settings).unwrap();
        assert!(
            !before
                .as_deref()
                .unwrap_or_default()
                .contains("ANTHROPIC_BASE_URL"),
            "Claude Code on this host already has a relay; use an unconfigured host"
        );
        let manager = ConfigManager::remote(data.path(), &journals, &home, session.clone());
        assert!(
            !manager
                .status(TargetId::ClaudeCode, Some(&relays))
                .configured,
            "Claude Code must start unconfigured"
        );
        let request = ConfigRequest {
            target_id: TargetId::ClaudeCode,
            provider_id: "messages".into(),
            model: "claude-test".into(),
            context_window: None,
            max_output_tokens: None,
            options: ClientOptions {
                access_mode: ClientAccessMode::Native,
                ..Default::default()
            },
        };
        let preview = manager.preview(&relays, &request).unwrap();
        assert!(!preview.content.contains(KEY));
        manager.apply(&relays, &request, &preview.revision).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&session.read(&claude_settings).unwrap().unwrap()).unwrap();
        assert_eq!(
            written["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.agentharbor-test.invalid"
        );
        assert_eq!(written["env"]["ANTHROPIC_AUTH_TOKEN"], KEY);
        let status = manager.status(TargetId::ClaudeCode, Some(&relays));
        assert!(
            status.configured && !status.conflict && !status.stale,
            "{:?}",
            status.conflict_fields
        );
        manager.restore(TargetId::ClaudeCode).unwrap();
        assert_eq!(session.read(&claude_settings).unwrap(), before);
    }
}
