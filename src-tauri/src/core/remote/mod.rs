//! Remote hosts: machines reached over SSH whose Codex and Claude Code
//! configuration this app edits in place. Hosts and their secrets (a
//! password, a key passphrase) are kept like relay keys: encrypted for the
//! current user and never shown again. A remote client is configured
//! direct, reaching its relay itself, or through this machine's router over
//! a reverse tunnel ([`tunnel`]) that this app keeps up while it runs.
//! Where each client keeps its files follows the host's environment
//! (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, ...), read from its shells or set here.

pub mod codex;
mod connection;
pub mod probe;
pub mod ssh_config;
pub mod tunnel;
pub mod web_search;

use super::{
    client_config::{ClientFiles, ClientPaths},
    router::DEFAULT_PORT,
    secret_file, storage,
    targets::TargetId,
};
pub use connection::ConnectError;
use connection::{block_on, Connection, KnownHosts, Method, Target};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const HOSTS_FILE: &str = "remote-hosts.dat";
const KNOWN_HOSTS_FILE: &str = "remote-known-hosts";
const JOURNAL_DIR: &str = "remote";
const DEFAULT_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// The variables that move a client's files on a host: Codex's home, and
/// what [`ClientPaths::posix`] follows for the others.
pub const CLIENT_ENV: [&str; 5] = [
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "OPENCODE_CONFIG",
    "PI_CODING_AGENT_DIR",
];

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
    /// The server port the tunnel listens on; routed configurations are
    /// written against it. `None` is the router's default port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tunnel_port: Option<u16>,
    /// Client variables set here, over what the host's shells set.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    /// Client variables the host's shells set, as last read.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    detected_env: BTreeMap<String, String>,
}

impl StoredHost {
    /// The client variables in effect.
    fn client_env(&self) -> BTreeMap<String, String> {
        let mut env = self.detected_env.clone();
        env.extend(self.env.clone());
        env
    }
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
    /// The server port the tunnel listens on.
    pub tunnel_port: u16,
    /// Whether a client on this host goes through this machine's router.
    pub routed: bool,
    /// The tunnel, while one runs.
    pub tunnel: Option<tunnel::TunnelState>,
    /// Client variables set here.
    pub env: BTreeMap<String, String>,
    /// Client variables the host's shells set, as last read.
    pub detected_env: BTreeMap<String, String>,
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
    #[serde(default)]
    pub tunnel_port: Option<u16>,
    /// Client variables to set; an empty value follows the host.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
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
            ["claude-code.dat", codex::JOURNAL_FILE, web_search::JOURNAL_FILE]
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
        let resolved = resolve(host, host_config(host, config));
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
            tunnel_port: host.tunnel_port.unwrap_or(DEFAULT_PORT),
            routed: self.routed(&host.id),
            tunnel: tunnel::state(&host.id),
            env: host.env.clone(),
            detected_env: host.detected_env.clone(),
        }
    }

    pub fn save_host(&self, input: RemoteHostInput) -> Result<RemoteHostView, String> {
        let _guard = lock()?;
        let name = input.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err("remote.errors.name".into());
        }
        let host = input.host.trim().to_owned();
        // A leading `-` would read as an option to `ssh -G`.
        if !valid_word(&host, 255) || host.starts_with('-') {
            return Err("remote.errors.host".into());
        }
        let user = clean(input.user);
        if user
            .as_deref()
            .is_some_and(|user| !valid_word(user, 64) || user.starts_with('-'))
        {
            return Err("remote.errors.user".into());
        }
        if input.port == Some(0) || input.tunnel_port == Some(0) {
            return Err("remote.errors.port".into());
        }
        let identity_file = clean(input.identity_file);
        if identity_file
            .as_deref()
            .is_some_and(|path| path.chars().any(char::is_control))
        {
            return Err("remote.errors.identityFile".into());
        }
        let mut env = BTreeMap::new();
        for (name, value) in input.env {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if !CLIENT_ENV.contains(&name.as_str())
                || !(value.starts_with('/') || value == "~" || value.starts_with("~/"))
                || value.len() > 1024
                || value.chars().any(char::is_control)
            {
                return Err("remote.errors.clientDir".into());
            }
            env.insert(name, value.to_owned());
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
        // Routed configurations name the tunnel's port: it stays while any is in place.
        if let Some(existing) = &existing {
            if self.routed(&existing.id)
                && existing.tunnel_port.unwrap_or(DEFAULT_PORT)
                    != input.tunnel_port.unwrap_or(DEFAULT_PORT)
            {
                return Err("remote.errors.tunnelPortInUse".into());
            }
            // Configured clients stay where their files were written.
            let mut moved = existing.detected_env.clone();
            moved.extend(env.clone());
            if self.in_use(&existing.id) && moved != existing.client_env() {
                return Err("remote.errors.clientDirInUse".into());
            }
        }
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
            tunnel_port: input.tunnel_port.filter(|port| *port != DEFAULT_PORT),
            env,
            detected_env: existing
                .as_ref()
                .map(|host| host.detected_env.clone())
                .unwrap_or_default(),
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
        tunnel::stop(id);
        if let Ok(dir) = self.journal_dir(id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    /// Whether a client on host `id` is configured through this machine's
    /// router, and so needs its tunnel.
    pub fn routed(&self, id: &str) -> bool {
        let Ok(dir) = self.journal_dir(id) else {
            return false;
        };
        codex::routed(&dir)
            || super::client_config::ConfigManager::remote(
                &self.data_dir,
                &dir,
                "/",
                Arc::new(RemoteSession::offline()),
            )
            .stored_request(TargetId::ClaudeCode)
            .ok()
            .flatten()
            .is_some_and(|request| request.needs_router())
    }

    /// Whether any host has a client that needs this machine's router.
    pub fn has_router_configs(&self) -> bool {
        self.load()
            .map(|hosts| hosts.iter().any(|host| self.routed(&host.id)))
            .unwrap_or(false)
    }

    /// The server port host `id`'s tunnel listens on.
    pub fn tunnel_port(&self, id: &str) -> Result<u16, String> {
        let host = self.stored(id).map_err(|error| error.key())?;
        Ok(host.tunnel_port.unwrap_or(DEFAULT_PORT))
    }

    /// Keeps a tunnel to `local_port` for every host with routed clients and
    /// stops the others; with `router_up` false, stops them all.
    pub fn sync_tunnels(&self, local_port: u16, router_up: bool) {
        let mut keep = Vec::new();
        if router_up {
            for host in self.load().unwrap_or_default() {
                if !self.routed(&host.id) {
                    continue;
                }
                match self.target(&host) {
                    Ok(target) => {
                        tunnel::ensure(
                            &host.id,
                            target,
                            self.known_hosts(),
                            host.tunnel_port.unwrap_or(DEFAULT_PORT),
                            local_port,
                        );
                        keep.push(host.id.clone());
                    }
                    Err(error) => {
                        log::warn!("no tunnel for remote host {}: {}", host.name, error.key())
                    }
                }
            }
        }
        tunnel::retain(&keep);
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
        let host = self.stored(id)?;
        let target = self.target(&host)?;
        let known = self.known_hosts();
        let connection = block_on(Connection::open(&target, known, trust))?;
        let mut session = RemoteSession {
            connection: Some(connection),
            env: BTreeMap::new(),
        };
        session.set_env(&host.client_env());
        Ok(session)
    }

    /// Reads the client variables the host's shells set and keeps them for
    /// host `id`, `session` following. While a client there is configured
    /// they are left as they were, its files staying where they were
    /// written: the variables that have moved since are returned.
    pub fn detect_env(&self, id: &str, session: &mut RemoteSession) -> Vec<String> {
        let Ok(found) = session.read_env() else {
            return Vec::new();
        };
        let Ok(_guard) = lock() else {
            return Vec::new();
        };
        let Ok(mut hosts) = self.load() else {
            return Vec::new();
        };
        let Some(host) = hosts.iter_mut().find(|host| host.id == id) else {
            return Vec::new();
        };
        if host.detected_env == found {
            return Vec::new();
        }
        if self.in_use(id) {
            return CLIENT_ENV
                .iter()
                .filter(|name| {
                    !host.env.contains_key(**name)
                        && found.get(**name) != host.detected_env.get(**name)
                })
                .map(|name| (*name).to_owned())
                .collect();
        }
        host.detected_env = found;
        let env = host.client_env();
        if self.save(&hosts).is_ok() {
            session.set_env(&env);
        }
        Vec::new()
    }

    fn stored(&self, id: &str) -> Result<StoredHost, ConnectError> {
        self.load()
            .map_err(ConnectError::Failed)?
            .into_iter()
            .find(|host| host.id == id)
            .ok_or_else(|| ConnectError::Failed("remote.errors.hostMissing".into()))
    }

    /// Where and how to sign in to `host`, through `~/.ssh/config`.
    fn target(&self, host: &StoredHost) -> Result<Target, ConnectError> {
        let resolved = resolve(host, host_config(host, &ssh_config::SshConfig::load()));
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
        Ok(Target {
            host_name: resolved.host_name,
            alias: host.host.clone(),
            port: resolved.port,
            user,
            method,
        })
    }

    pub fn test(&self, id: &str, trust: Option<String>) -> ConnectionTest {
        match self.connect(id, trust) {
            Ok(mut session) => {
                self.detect_env(id, &mut session);
                ConnectionTest::Connected {
                    home: session.home().to_owned(),
                }
            }
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

/// What `~/.ssh/config` says of `host`: OpenSSH's own answer when its
/// client is installed, else `config`'s.
fn host_config(host: &StoredHost, config: &ssh_config::SshConfig) -> ssh_config::HostConfig {
    ssh_config::openssh(&host.host, host.user.as_deref(), host.port)
        .unwrap_or_else(|| config.resolve(&host.host))
}

fn resolve(host: &StoredHost, from_config: ssh_config::HostConfig) -> Resolved {
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
        None if !from_config.identity_files.is_empty() => {
            let files: Vec<PathBuf> = from_config
                .identity_files
                .iter()
                .map(|file| ssh_config::expand_identity(file, &home, &host_name, &remote_user))
                .collect();
            // OpenSSH lists its default names for a host that names none:
            // the ones present come first.
            let (present, absent): (Vec<PathBuf>, Vec<PathBuf>) =
                files.into_iter().partition(|path| path.is_file());
            present.into_iter().chain(absent).collect()
        }
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
    /// The client variables in effect, as absolute paths: commands run
    /// with them, and files are found by them.
    env: BTreeMap<String, String>,
}

/// Prints `AHENV<tab>name<tab>value` for each client variable a shell has:
/// a plain one, the login shell, then an interactive one (whose rc files
/// often hold such exports), later lines winning.
const READ_ENV: &str = r#"v='for n in CODEX_HOME CLAUDE_CONFIG_DIR XDG_CONFIG_HOME OPENCODE_CONFIG PI_CODING_AGENT_DIR; do eval "x=\${$n:-}"; [ -n "$x" ] && printf "AHENV\t%s\t%s\n" "$n" "$x"; done; :'; sh -c "$v"; s=${SHELL:-/bin/sh}; t=; command -v timeout >/dev/null 2>&1 && t="timeout 8"; $t "$s" -lc "$v" </dev/null 2>/dev/null; $t "$s" -ic "$v" </dev/null 2>/dev/null; exit 0"#;

fn parse_env(output: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, '\t');
        if let (Some("AHENV"), Some(name), Some(value)) = (parts.next(), parts.next(), parts.next())
        {
            let value = value.trim_end_matches('/');
            if CLIENT_ENV.contains(&name) && value.starts_with('/') {
                env.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    env
}

/// A remote POSIX path for `path`, which was built from one.
fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl RemoteSession {
    /// A session that reaches nothing, for work on the journals alone.
    pub fn offline() -> Self {
        Self {
            connection: None,
            env: BTreeMap::new(),
        }
    }

    /// Takes `env` as the client variables, `~` read as the home folder.
    fn set_env(&mut self, env: &BTreeMap<String, String>) {
        let home = self.home().trim_end_matches('/').to_owned();
        self.env = env
            .iter()
            .filter_map(|(name, value)| {
                let value = match value.strip_prefix('~') {
                    Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                        format!("{home}{rest}")
                    }
                    _ => value.clone(),
                };
                let value = value.trim_end_matches('/').to_owned();
                value.starts_with('/').then(|| (name.clone(), value))
            })
            .collect();
    }

    /// The client variables in effect.
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Where Codex keeps its files on the host.
    pub fn codex_home(&self) -> String {
        self.env
            .get("CODEX_HOME")
            .cloned()
            .unwrap_or_else(|| format!("{}/.codex", self.home().trim_end_matches('/')))
    }

    /// Where the other clients keep theirs.
    pub fn client_paths(&self) -> ClientPaths {
        ClientPaths::posix(self.home(), |name| self.env.get(name).cloned())
    }

    /// The client variables the host's shells set.
    fn read_env(&self) -> Result<BTreeMap<String, String>, String> {
        let (_, output) = block_on(
            self.connection()?
                .run(READ_ENV, std::time::Duration::from_secs(30)),
        )?;
        Ok(parse_env(&output))
    }

    pub fn home(&self) -> &str {
        self.connection.as_ref().map(Connection::home).unwrap_or("")
    }

    fn connection(&self) -> Result<&Connection, String> {
        self.connection
            .as_ref()
            .ok_or_else(|| "remote.errors.closed".into())
    }

    /// Runs `command` on the host through the user's shell, given `limit`
    /// to finish: its exit status and output.
    /// The client variables are exported to it.
    pub fn run(&self, command: &str, limit: std::time::Duration) -> Result<(u32, String), String> {
        let exports: String = self
            .env
            .iter()
            .map(|(name, value)| format!("export {name}={}; ", connection::quote(value)))
            .collect();
        block_on(self.connection()?.run(&format!("{exports}{command}"), limit))
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
            tunnel_port: None,
            env: BTreeMap::new(),
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
    fn client_dirs_follow_the_host_and_stay_while_in_use() {
        let dir = std::env::temp_dir().join(format!("agentharbor-remote-{}", uuid::Uuid::new_v4()));
        let store = RemoteStore::new(&dir);
        let mut host = input("Dev");
        host.env.insert("CODEX_HOME".into(), "relative/path".into());
        assert_eq!(
            store.save_host(host).err().as_deref(),
            Some("remote.errors.clientDir")
        );
        let mut host = input("Dev");
        host.env.insert("CODEX_HOME".into(), "~/work/codex".into());
        host.env.insert("CLAUDE_CONFIG_DIR".into(), " ".into());
        let saved = store.save_host(host).unwrap();
        assert_eq!(saved.env.len(), 1);

        let mut session = RemoteSession::offline();
        session.set_env(&store.stored(&saved.id).ok().unwrap().client_env());
        // Offline, the home is empty: `~` reads as the root.
        assert_eq!(session.codex_home(), "/work/codex");
        assert!(session.client_paths().claude.ends_with(".claude/settings.json"));

        // A configured client keeps its directories.
        let journals = store.journal_dir(&saved.id).unwrap();
        std::fs::create_dir_all(&journals).unwrap();
        std::fs::write(journals.join(codex::JOURNAL_FILE), b"x").unwrap();
        let mut edit = input("Dev");
        edit.id = Some(saved.id.clone());
        assert_eq!(
            store.save_host(edit).err().as_deref(),
            Some("remote.errors.clientDirInUse")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shells_report_client_dirs() {
        let env = parse_env(
            "motd\nAHENV\tCODEX_HOME\t/srv/codex/\nAHENV\tPATH\t/bin\nAHENV\tCLAUDE_CONFIG_DIR\trelative\nAHENV\tCODEX_HOME\t/data/codex\n",
        );
        assert_eq!(env.len(), 1);
        assert_eq!(env["CODEX_HOME"], "/data/codex");
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
            tunnel_port: None,
            env: BTreeMap::new(),
            detected_env: BTreeMap::new(),
        };
        let resolved = resolve(&stored, config.resolve("dev"));
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
                tunnel_port: None,
                env: BTreeMap::new(),
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

        // Codex: config, key and catalog are written, then all taken back.
        let codex_home = session.codex_home();
        let at = |rest: &str| PathBuf::from(format!("{codex_home}/{rest}"));
        let codex_config = at("config.toml");
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
        let env_before = session.read(&at(".env")).unwrap();
        let codex = codex::CodexRemote::new(
            session.as_ref(),
            &session.codex_home(),
            &journals,
            data.path(),
            Vec::new(),
        );
        assert!(
            !codex.status(Some(&relays)).configured,
            "Codex must start unconfigured"
        );
        let request = codex::CodexRemoteRequest {
            provider_id: "responses".into(),
            model: "gpt-test".into(),
            web_search: false,
            profile: None,
            router: false,
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
        let env = session.read(&at(".env")).unwrap().unwrap();
        assert!(env.contains(KEY));
        assert!(session
            .read(&at("agentharbor-models.json"))
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
        assert_eq!(session.read(&at(".env")).unwrap(), env_before);
        assert!(session
            .read(&at("agentharbor-models.json"))
            .unwrap()
            .is_none());

        // Claude Code through the shared configuration engine, direct.
        let claude_settings = session.client_paths().claude;
        let before = session.read(&claude_settings).unwrap();
        assert!(
            !before
                .as_deref()
                .unwrap_or_default()
                .contains("ANTHROPIC_BASE_URL"),
            "Claude Code on this host already has a relay; use an unconfigured host"
        );
        let manager = ConfigManager::remote_at(
            data.path(),
            &journals,
            session.client_paths(),
            session.clone(),
        );
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
