//! One SSH session to a remote host: the host key is checked, the user signs
//! in with a key or a password, and files are read and written over SFTP.
//! Writes go to a temporary file first and are moved into place, so a
//! client never reads half a configuration.

use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{client, ChannelMsg, Disconnect};
use russh_sftp::client::{error::Error as SftpError, SftpSession};
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Configuration files are small; anything larger is not one.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// SSH runs on its own runtime so the synchronous configuration code can
/// wait on it from any thread.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("agentharbor-ssh")
            .enable_all()
            .build()
            .expect("SSH runtime")
    })
}

/// Runs `future` in the background on the SSH runtime.
pub(super) fn spawn(future: impl std::future::Future<Output = ()> + Send + 'static) {
    runtime().spawn(future);
}

pub(super) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    runtime().block_on(future)
}

/// Where to connect and how to sign in, already resolved through
/// `~/.ssh/config`.
pub struct Target {
    /// The address connected to, which is also the name the host key is
    /// recorded under.
    pub host_name: String,
    /// The name the user typed, when it is an alias: a known_hosts entry
    /// under it is accepted too.
    pub alias: String,
    pub port: u16,
    pub user: String,
    pub method: Method,
}

pub enum Method {
    Password(String),
    Keys {
        files: Vec<PathBuf>,
        passphrase: Option<String>,
    },
}

/// Files that hold host keys: the user's own known_hosts (read only) and
/// the file of keys trusted from this app (read and appended).
pub struct KnownHosts {
    pub user: Option<PathBuf>,
    pub app: PathBuf,
}

/// Why a connection did not open. `UnknownHost` carries what the user must
/// confirm before the key is trusted.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectError {
    UnknownHost {
        fingerprint: String,
        algorithm: String,
    },
    HostKeyChanged {
        fingerprint: String,
    },
    /// An i18n key.
    Failed(String),
}

impl ConnectError {
    pub fn key(&self) -> String {
        match self {
            Self::UnknownHost { .. } => "remote.errors.unknownHost".into(),
            Self::HostKeyChanged { .. } => "remote.errors.hostKeyChanged".into(),
            Self::Failed(key) => key.clone(),
        }
    }
}

fn failed(key: &str) -> ConnectError {
    ConnectError::Failed(key.into())
}

#[derive(Clone)]
enum HostKeyIssue {
    Unknown {
        fingerprint: String,
        algorithm: String,
    },
    Changed {
        fingerprint: String,
    },
    Certificate,
}

pub struct Checker {
    names: Vec<String>,
    port: u16,
    known: Arc<KnownHosts>,
    /// The fingerprint the user confirmed for this host, if any.
    trust: Option<String>,
    issue: Arc<Mutex<Option<HostKeyIssue>>>,
    /// For a tunnel: the local port each connection the server forwards
    /// is joined to. Other sessions refuse forwarded connections.
    forward_to: Option<u16>,
}

impl Checker {
    fn report(&self, issue: HostKeyIssue) -> Result<bool, russh::Error> {
        if let Ok(mut slot) = self.issue.lock() {
            *slot = Some(issue);
        }
        Ok(false)
    }
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            return self.report(HostKeyIssue::Certificate);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let files = self
            .known
            .user
            .iter()
            .chain(std::iter::once(&self.known.app));
        for file in files {
            for name in &self.names {
                match keys::check_known_hosts_path(name, self.port, key, file) {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    // A recorded key of the same type that differs: refuse,
                    // whatever the user confirmed before.
                    Err(keys::Error::KeyChanged { .. }) => {
                        return self.report(HostKeyIssue::Changed { fingerprint });
                    }
                    // A line this parser cannot read hides nothing it could
                    // check; the other file and names still count.
                    Err(_) => {}
                }
            }
        }
        if self.trust.as_deref() == Some(fingerprint.as_str()) {
            let name = &self.names[0];
            if keys::known_hosts::learn_known_hosts_path(name, self.port, key, &self.known.app)
                .is_err()
            {
                log::warn!("could not record the trusted host key of {name}");
            }
            return Ok(true);
        }
        self.report(HostKeyIssue::Unknown {
            fingerprint,
            algorithm: key.algorithm().to_string(),
        })
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let Some(port) = self.forward_to else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        // Only this machine's loopback router is ever reached.
        let local = match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            Ok(local) => local,
            Err(_) => {
                reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
                return Ok(());
            }
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut local = local;
            let mut remote = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut remote, &mut local).await;
        });
        Ok(())
    }
}

/// A signed-in SSH session to `target`; `forward_to` makes it a tunnel's.
async fn session(
    target: &Target,
    known: Arc<KnownHosts>,
    trust: Option<String>,
    forward_to: Option<u16>,
) -> Result<client::Handle<Checker>, ConnectError> {
    let issue = Arc::new(Mutex::new(None));
    let mut names = vec![target.host_name.clone()];
    if !target.alias.is_empty() && !target.alias.eq_ignore_ascii_case(&target.host_name) {
        names.push(target.alias.clone());
    }
    let checker = Checker {
        names,
        port: target.port,
        known,
        trust,
        issue: issue.clone(),
        forward_to,
    };
    let config = Arc::new(match forward_to {
        // A tunnel stays up while idle and notices a dead link by keepalives.
        Some(_) => client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: 3,
            ..Default::default()
        },
        None => client::Config {
            inactivity_timeout: Some(Duration::from_secs(120)),
            ..Default::default()
        },
    });
    let stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((target.host_name.as_str(), target.port)),
    )
    .await
    .map_err(|_| failed("remote.errors.timeout"))?
    .map_err(|_| failed("remote.errors.unreachable"))?;
    let handshake = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect_stream(config, stream, checker),
    )
    .await;
    let mut handle = match handshake {
        Ok(Ok(handle)) => handle,
        Ok(Err(_)) | Err(_) => {
            let issue = issue.lock().ok().and_then(|slot| slot.clone());
            return Err(match issue {
                Some(HostKeyIssue::Unknown {
                    fingerprint,
                    algorithm,
                }) => ConnectError::UnknownHost {
                    fingerprint,
                    algorithm,
                },
                Some(HostKeyIssue::Changed { fingerprint }) => {
                    ConnectError::HostKeyChanged { fingerprint }
                }
                Some(HostKeyIssue::Certificate) => failed("remote.errors.hostCertificate"),
                None if handshake.is_err() => failed("remote.errors.timeout"),
                None => failed("remote.errors.handshake"),
            });
        }
    };
    tokio::time::timeout(CONNECT_TIMEOUT, authenticate(&mut handle, target))
        .await
        .unwrap_or_else(|_| Err(failed("remote.errors.timeout")))?;
    Ok(handle)
}

/// A reverse tunnel: the server's `127.0.0.1:remote_port` reaches this
/// machine's `127.0.0.1:local_port` for as long as the returned session lives.
pub async fn tunnel(
    target: &Target,
    known: Arc<KnownHosts>,
    remote_port: u16,
    local_port: u16,
) -> Result<client::Handle<Checker>, ConnectError> {
    let handle = session(target, known, None, Some(local_port)).await?;
    // Bound to the server's loopback only: nothing else there can reach it,
    // and the router still asks every request for its token.
    tokio::time::timeout(
        CONNECT_TIMEOUT,
        handle.tcpip_forward("127.0.0.1", remote_port as u32),
    )
    .await
    .map_err(|_| failed("remote.errors.timeout"))?
    .map_err(|_| failed("remote.errors.tunnelPort"))?;
    Ok(handle)
}

pub struct Connection {
    handle: client::Handle<Checker>,
    sftp: SftpSession,
    home: String,
}

fn not_found(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile)
}

async fn timed<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(OPERATION_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| Err("remote.errors.timeout".into()))
}

impl Connection {
    pub async fn open(
        target: &Target,
        known: Arc<KnownHosts>,
        trust: Option<String>,
    ) -> Result<Self, ConnectError> {
        let handle = session(target, known, trust, None).await?;
        let sftp = tokio::time::timeout(CONNECT_TIMEOUT, async {
            let channel = handle
                .channel_open_session()
                .await
                .map_err(|_| failed("remote.errors.sftp"))?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|_| failed("remote.errors.sftp"))?;
            SftpSession::new(channel.into_stream())
                .await
                .map_err(|_| failed("remote.errors.sftp"))
        })
        .await
        .unwrap_or_else(|_| Err(failed("remote.errors.timeout")))?;
        let home = sftp
            .canonicalize(".")
            .await
            .map_err(|_| failed("remote.errors.sftp"))?;
        Ok(Self { handle, sftp, home })
    }

    /// The signed-in user's home directory, as an absolute POSIX path.
    pub fn home(&self) -> &str {
        &self.home
    }

    pub async fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        timed(async {
            match self.sftp.metadata(path).await {
                Ok(metadata) => {
                    if metadata.is_dir() {
                        return Err("remote.errors.read".into());
                    }
                    if metadata.size.is_some_and(|size| size > MAX_FILE_BYTES) {
                        return Err("remote.errors.tooLarge".into());
                    }
                }
                Err(error) if not_found(&error) => return Ok(None),
                Err(_) => return Err("remote.errors.read".into()),
            }
            match self.sftp.read(path).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if not_found(&error) => Ok(None),
                Err(_) => Err("remote.errors.read".into()),
            }
        })
        .await
    }

    /// Replaces `path` with `bytes`: written beside it, then moved over it.
    /// A symlink is followed so its target is replaced, not the link. A new
    /// file is readable by its owner only; an existing one keeps its mode.
    pub async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        timed(async {
            let mut target = path.to_owned();
            let mode = match self.sftp.symlink_metadata(path).await {
                Ok(metadata) => {
                    if metadata.is_symlink() {
                        target = self
                            .sftp
                            .canonicalize(path)
                            .await
                            .map_err(|_| "remote.errors.write".to_owned())?;
                        self.sftp
                            .metadata(&target)
                            .await
                            .ok()
                            .and_then(|metadata| metadata.permissions)
                    } else {
                        metadata.permissions
                    }
                }
                Err(error) if not_found(&error) => None,
                Err(_) => return Err("remote.errors.write".into()),
            }
            .map(|mode| mode & 0o7777)
            .unwrap_or(0o600);
            if let Some((parent, _)) = target.rsplit_once('/') {
                self.create_dirs(parent).await?;
            }
            let temporary = format!("{target}.agentharbor-{}.tmp", uuid::Uuid::new_v4().simple());
            let mut attributes = FileAttributes::empty();
            attributes.permissions = Some(mode);
            let written = async {
                let mut file = self
                    .sftp
                    .open_with_flags_and_attributes(
                        &temporary,
                        OpenFlags::CREATE
                            | OpenFlags::EXCLUDE
                            | OpenFlags::WRITE
                            | OpenFlags::TRUNCATE,
                        attributes,
                    )
                    .await
                    .map_err(|_| "remote.errors.write".to_owned())?;
                file.write_all(bytes)
                    .await
                    .map_err(|_| "remote.errors.write".to_owned())?;
                file.shutdown()
                    .await
                    .map_err(|_| "remote.errors.write".to_owned())?;
                // The umask may have narrowed the mode it was created with.
                let mut attributes = FileAttributes::empty();
                attributes.permissions = Some(mode);
                let _ = self.sftp.set_metadata(&temporary, attributes).await;
                let status = self
                    .exec(&format!(
                        "mv -f -- {} {}",
                        quote(&temporary),
                        quote(&target)
                    ))
                    .await?;
                if status != 0 {
                    return Err("remote.errors.write".into());
                }
                Ok(())
            }
            .await;
            if written.is_err() {
                let _ = self.sftp.remove_file(&temporary).await;
            }
            written
        })
        .await
    }

    pub async fn remove(&self, path: &str) -> Result<(), String> {
        timed(async {
            match self.sftp.remove_file(path).await {
                Ok(()) => Ok(()),
                Err(error) if not_found(&error) => Ok(()),
                Err(_) => Err("remote.errors.write".into()),
            }
        })
        .await
    }

    async fn create_dirs(&self, dir: &str) -> Result<(), String> {
        let mut missing = Vec::new();
        let mut current = dir.trim_end_matches('/').to_owned();
        while !current.is_empty() {
            match self.sftp.metadata(&current).await {
                Ok(metadata) if metadata.is_dir() => break,
                Ok(_) => return Err("remote.errors.write".into()),
                Err(error) if not_found(&error) => {
                    missing.push(current.clone());
                    current = current
                        .rsplit_once('/')
                        .map(|(parent, _)| parent.to_owned())
                        .unwrap_or_default();
                }
                Err(_) => return Err("remote.errors.write".into()),
            }
        }
        for dir in missing.into_iter().rev() {
            let mut attributes = FileAttributes::empty();
            attributes.permissions = Some(0o700);
            if self.sftp.create_dir(&dir).await.is_err() {
                return Err("remote.errors.write".into());
            }
            let _ = self.sftp.set_metadata(&dir, attributes).await;
        }
        Ok(())
    }

    /// Runs `command` through the user's shell and returns its exit status.
    async fn exec(&self, command: &str) -> Result<u32, String> {
        self.output(command).await.map(|(status, _)| status)
    }

    /// Runs `command` through the user's shell and returns its exit status
    /// and its output (stdout and stderr together, at most 64 KiB kept).
    async fn output(&self, command: &str) -> Result<(u32, String), String> {
        const MAX_OUTPUT: usize = 64 * 1024;
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|_| "remote.errors.exec".to_owned())?;
        channel
            .exec(true, command)
            .await
            .map_err(|_| "remote.errors.exec".to_owned())?;
        let mut status = None;
        let mut output = Vec::new();
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    let room = MAX_OUTPUT.saturating_sub(output.len());
                    output.extend_from_slice(&data[..data.len().min(room)]);
                }
                _ => {}
            }
        }
        let status = status.ok_or_else(|| "remote.errors.exec".to_owned())?;
        Ok((status, String::from_utf8_lossy(&output).into_owned()))
    }

    /// [`Self::output`] given `limit` to finish.
    pub async fn run(&self, command: &str, limit: Duration) -> Result<(u32, String), String> {
        tokio::time::timeout(limit, self.output(command))
            .await
            .unwrap_or_else(|_| Err("remote.errors.timeout".into()))
    }

    pub async fn close(self) {
        let _ = self.sftp.close().await;
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

/// A POSIX shell word for `value`.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

async fn authenticate(
    handle: &mut client::Handle<Checker>,
    target: &Target,
) -> Result<(), ConnectError> {
    match &target.method {
        Method::Password(password) => {
            let result = handle
                .authenticate_password(target.user.clone(), password.clone())
                .await
                .map_err(|_| failed("remote.errors.handshake"))?;
            if result.success() {
                return Ok(());
            }
            // A server that checks passwords through PAM offers them as
            // keyboard-interactive instead: each hidden prompt gets it.
            let mut reply = handle
                .authenticate_keyboard_interactive_start(target.user.clone(), None)
                .await
                .map_err(|_| failed("remote.errors.handshake"))?;
            // A few rounds at most: an empty round, then the password one.
            for _ in 0..4 {
                match reply {
                    client::KeyboardInteractiveAuthResponse::Success => return Ok(()),
                    client::KeyboardInteractiveAuthResponse::Failure { .. } => break,
                    client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                        // An echoed prompt asks for something else (a user
                        // name, a code), never for the password.
                        if prompts.iter().any(|prompt| prompt.echo) {
                            return Err(failed("remote.errors.authPrompt"));
                        }
                        let responses = prompts.iter().map(|_| password.clone()).collect();
                        reply = handle
                            .authenticate_keyboard_interactive_respond(responses)
                            .await
                            .map_err(|_| failed("remote.errors.handshake"))?;
                    }
                }
            }
            Err(failed("remote.errors.authPassword"))
        }
        Method::Keys { files, passphrase } => {
            let mut error = "remote.errors.noKey";
            for file in files {
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                let key = match keys::decode_secret_key(&text, passphrase.as_deref()) {
                    Ok(key) => key,
                    Err(keys::Error::KeyIsEncrypted) => {
                        error = "remote.errors.passphraseNeeded";
                        continue;
                    }
                    Err(_) => {
                        // An encrypted key that the stored passphrase does not open.
                        let encrypted = matches!(
                            keys::decode_secret_key(&text, None),
                            Err(keys::Error::KeyIsEncrypted)
                        );
                        if passphrase.is_some() && encrypted {
                            error = "remote.errors.passphraseWrong";
                        } else if error == "remote.errors.noKey" {
                            error = "remote.errors.keyUnreadable";
                        }
                        continue;
                    }
                };
                let hash = if key.algorithm().is_rsa() {
                    handle
                        .best_supported_rsa_hash()
                        .await
                        .ok()
                        .flatten()
                        .flatten()
                } else {
                    None
                };
                let result = handle
                    .authenticate_publickey(
                        target.user.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
                    .map_err(|_| failed("remote.errors.handshake"))?;
                if result.success() {
                    return Ok(());
                }
                error = "remote.errors.authKey";
            }
            Err(failed(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::server::{self, Auth};
    use russh::{MethodKind, MethodSet};
    use std::borrow::Cow;

    #[test]
    fn quotes_shell_words() {
        assert_eq!(quote("/home/a b/.codex"), "'/home/a b/.codex'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    const PASSWORD: &str = "s3cret pass";

    /// A server taking the password as `password`, or only through
    /// keyboard-interactive prompts as PAM setups do.
    struct PasswordServer {
        keyboard_only: bool,
    }

    impl server::Handler for PasswordServer {
        type Error = russh::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
            Ok(
                if !self.keyboard_only && user == "lab" && password == PASSWORD {
                    Auth::Accept
                } else {
                    Auth::reject()
                },
            )
        }

        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            user: &str,
            _submethods: &str,
            response: Option<server::Response<'a>>,
        ) -> Result<Auth, Self::Error> {
            Ok(match response {
                None if user == "lab" => Auth::Partial {
                    name: Cow::Borrowed(""),
                    instructions: Cow::Borrowed(""),
                    prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
                },
                Some(mut answers) => {
                    if answers.next().as_deref() == Some(PASSWORD.as_bytes()) {
                        Auth::Accept
                    } else {
                        Auth::reject()
                    }
                }
                None => Auth::reject(),
            })
        }
    }

    /// Signs in to an in-process server on a loopback port.
    fn sign_in(keyboard_only: bool, password: &str) -> Result<(), ConnectError> {
        let host_key =
            keys::PrivateKey::from(keys::ssh_key::private::Ed25519Keypair::from_seed(&[7; 32]));
        let fingerprint = host_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        let methods: &[MethodKind] = if keyboard_only {
            &[MethodKind::KeyboardInteractive]
        } else {
            &[MethodKind::Password, MethodKind::KeyboardInteractive]
        };
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            methods: MethodSet::from(methods),
            ..Default::default()
        });
        let known = tempfile::tempdir().unwrap();
        block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                if let Ok(session) =
                    server::run_stream(config, stream, PasswordServer { keyboard_only }).await
                {
                    let _ = session.await;
                }
            });
            let target = Target {
                host_name: "127.0.0.1".into(),
                alias: String::new(),
                port,
                user: "lab".into(),
                method: Method::Password(password.into()),
            };
            let known = Arc::new(KnownHosts {
                user: None,
                app: known.path().join("known_hosts"),
            });
            session(&target, known, Some(fingerprint), None)
                .await
                .map(|_| ())
        })
    }

    #[test]
    fn a_password_signs_in_by_password_or_by_keyboard_interactive() {
        assert_eq!(sign_in(false, PASSWORD), Ok(()));
        assert_eq!(
            sign_in(true, PASSWORD),
            Ok(()),
            "PAM servers ask through keyboard-interactive"
        );
        assert_eq!(
            sign_in(true, "wrong")
                .err()
                .map(|error| error.key())
                .as_deref(),
            Some("remote.errors.authPassword")
        );
        assert_eq!(
            sign_in(false, "wrong")
                .err()
                .map(|error| error.key())
                .as_deref(),
            Some("remote.errors.authPassword")
        );
    }
}
