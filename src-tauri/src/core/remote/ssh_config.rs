//! The part of `~/.ssh/config` a remote host needs: which address, port,
//! user and identity files an alias stands for. OpenSSH answers that itself
//! when its client is installed ([`openssh`], `ssh -G`), `Match` blocks and
//! every token included. Without it, [`SshConfig`] reads the file: `Host`
//! blocks follow OpenSSH's rules (the first value obtained wins, identity
//! files add up, `!` negates a pattern), `Include` is followed, and `Match`
//! blocks are skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// How deep `Include` may nest, as in OpenSSH.
const MAX_INCLUDE_DEPTH: usize = 16;
/// How long an answer from `ssh -G` is reused while the config is unchanged.
const OPENSSH_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostConfig {
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<String>,
    /// `IdentityAgent`: a socket, `SSH_AUTH_SOCK`, or `none`.
    pub identity_agent: Option<String>,
    /// `IdentitiesOnly`: only the agent keys of `identity_files`.
    pub identities_only: bool,
}

struct Block {
    /// `None` for a `Match` block, which is never applied.
    patterns: Option<Vec<String>>,
    options: Vec<(String, String)>,
}

pub struct SshConfig {
    blocks: Vec<Block>,
}

pub fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

impl SshConfig {
    fn empty() -> Self {
        // Options before the first `Host` apply to every host.
        Self {
            blocks: vec![Block {
                patterns: Some(vec!["*".into()]),
                options: Vec::new(),
            }],
        }
    }

    /// The user's config with what it includes; a missing or unreadable
    /// file is an empty one.
    pub fn load() -> Self {
        let mut config = Self::empty();
        if let (Some(path), Some(home)) = (default_path(), dirs::home_dir()) {
            config.read_file(&path, &home, 0);
        }
        config
    }

    /// `text` alone, its `Include` lines not followed.
    pub fn parse(text: &str) -> Self {
        let mut config = Self::empty();
        config.parse_into(text, None);
        config
    }

    fn read_file(&mut self, path: &Path, home: &Path, depth: usize) {
        if let Ok(text) = std::fs::read_to_string(path) {
            self.parse_into(&text, Some((home, depth)));
        }
    }

    /// Adds the lines of `text`; `include` is the home folder `Include`
    /// paths start from and how deep this file is, or `None` to skip them.
    fn parse_into(&mut self, text: &str, include: Option<(&Path, usize)>) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (keyword, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
                Some(at) => (
                    &line[..at],
                    line[at..].trim_start_matches(|c: char| c.is_whitespace() || c == '='),
                ),
                None => (line, ""),
            };
            let keyword = keyword.to_ascii_lowercase();
            let args = split_args(rest);
            match keyword.as_str() {
                "host" => self.blocks.push(Block {
                    patterns: Some(args),
                    options: Vec::new(),
                }),
                "match" => self.blocks.push(Block {
                    patterns: None,
                    options: Vec::new(),
                }),
                "include" => {
                    let Some((home, depth)) = include else {
                        continue;
                    };
                    if depth >= MAX_INCLUDE_DEPTH {
                        continue;
                    }
                    // An included file applies within the block it is named
                    // in, which goes on after it.
                    let outer = self.blocks.last().and_then(|block| block.patterns.clone());
                    let before = self.blocks.len();
                    for pattern in args {
                        for path in include_paths(&pattern, home) {
                            self.read_file(&path, home, depth + 1);
                        }
                    }
                    if self.blocks.len() != before {
                        self.blocks.push(Block {
                            patterns: outer,
                            options: Vec::new(),
                        });
                    }
                }
                _ => {
                    if let (Some(block), Some(value)) =
                        (self.blocks.last_mut(), args.into_iter().next())
                    {
                        block.options.push((keyword, value));
                    }
                }
            }
        }
    }

    /// What `alias` resolves to.
    pub fn resolve(&self, alias: &str) -> HostConfig {
        let mut config = HostConfig::default();
        let mut seen_identities_only = false;
        for block in &self.blocks {
            let Some(patterns) = &block.patterns else {
                continue;
            };
            if !host_matches(patterns, alias) {
                continue;
            }
            for (keyword, value) in &block.options {
                match keyword.as_str() {
                    "hostname" if config.host_name.is_none() => {
                        config.host_name = Some(value.clone())
                    }
                    "user" if config.user.is_none() => config.user = Some(value.clone()),
                    "port" if config.port.is_none() => config.port = value.parse().ok(),
                    "identityagent" if config.identity_agent.is_none() => {
                        config.identity_agent = Some(value.clone())
                    }
                    "identitiesonly" if !seen_identities_only => {
                        seen_identities_only = true;
                        config.identities_only = value.eq_ignore_ascii_case("yes");
                    }
                    "identityfile" if !config.identity_files.contains(value) => {
                        config.identity_files.push(value.clone());
                    }
                    _ => {}
                }
            }
        }
        config
    }

    /// The concrete aliases (no wildcards, no negation), in file order.
    pub fn aliases(&self) -> Vec<String> {
        let mut aliases: Vec<String> = Vec::new();
        for pattern in self
            .blocks
            .iter()
            .filter_map(|block| block.patterns.as_ref())
            .flatten()
        {
            if !pattern.contains(['*', '?', '!']) && !aliases.contains(pattern) {
                aliases.push(pattern.clone());
            }
        }
        aliases
    }
}

fn split_args(rest: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// The files an `Include` argument names: `~` is the home folder, a
/// relative path is under `~/.ssh`, and the file name may hold `*` and `?`.
fn include_paths(pattern: &str, home: &Path) -> Vec<PathBuf> {
    let path = match pattern
        .strip_prefix("~/")
        .or_else(|| pattern.strip_prefix("~\\"))
    {
        Some(rest) => home.join(rest),
        None if Path::new(pattern).is_absolute() => PathBuf::from(pattern),
        None => home.join(".ssh").join(pattern),
    };
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| wildcard(&name, &entry.file_name().to_string_lossy()))
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

/// What OpenSSH makes of `alias` (`ssh -G`), with the user and port given
/// to it as on its command line. `None` when no OpenSSH client answers.
pub fn openssh(alias: &str, user: Option<&str>, port: Option<u16>) -> Option<HostConfig> {
    // An argument ssh would take for an option is never passed to it.
    if alias.is_empty() || alias.starts_with('-') || user.is_some_and(|user| user.starts_with('-'))
    {
        return None;
    }
    type Key = (String, Option<String>, Option<u16>);
    type Entry = (Instant, Option<SystemTime>, Option<HostConfig>);
    static CACHE: Mutex<Option<HashMap<Key, Entry>>> = Mutex::new(None);
    let modified = default_path()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let key = (alias.to_owned(), user.map(str::to_owned), port);
    if let Ok(cache) = CACHE.lock() {
        if let Some((at, when, config)) = cache.as_ref().and_then(|cache| cache.get(&key)) {
            if at.elapsed() < OPENSSH_TTL && *when == modified {
                return config.clone();
            }
        }
    }
    let config = run_openssh(alias, user, port);
    if let Ok(mut cache) = CACHE.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(key, (Instant::now(), modified, config.clone()));
    }
    config
}

fn openssh_command() -> Command {
    // Windows' own OpenSSH reads the same `~/.ssh` as this app; another
    // `ssh` on the PATH (Git's) may take another home.
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join(r"System32\OpenSSH\ssh.exe");
        if system.is_file() {
            return crate::platform::windows::background_command_path(&system);
        }
        crate::platform::windows::background_command("ssh")
    }
    #[cfg(not(windows))]
    Command::new("ssh")
}

fn run_openssh(alias: &str, user: Option<&str>, port: Option<u16>) -> Option<HostConfig> {
    let mut command = openssh_command();
    command.arg("-G");
    if let Some(user) = user {
        command.arg("-l").arg(user);
    }
    if let Some(port) = port {
        command.arg("-p").arg(port.to_string());
    }
    let mut child = command
        .arg(alias)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read while it runs: its output can outgrow a pipe's buffer.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut text)
            .ok()
            .map(|_| text)
    });
    // `Match exec` runs the user's commands: they get a few seconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().ok().flatten()?;
    if !status.success() {
        return None;
    }
    parse_openssh(&text)
}

/// `ssh -G` output: one lowercase keyword and its value per line.
fn parse_openssh(text: &str) -> Option<HostConfig> {
    let mut config = HostConfig::default();
    for line in text.lines() {
        let Some((keyword, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match keyword {
            "hostname" => config.host_name = Some(value.to_owned()),
            "user" => config.user = Some(value.to_owned()),
            "port" => config.port = value.parse().ok(),
            "identityagent" => config.identity_agent = Some(value.to_owned()),
            "identitiesonly" => config.identities_only = value == "yes",
            "identityfile" if !config.identity_files.iter().any(|file| file == value) => {
                config.identity_files.push(value.to_owned());
            }
            _ => {}
        }
    }
    config.host_name.is_some().then_some(config)
}

fn host_matches(patterns: &[String], host: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if wildcard(negated, host) {
                return false;
            }
        } else if wildcard(pattern, host) {
            matched = true;
        }
    }
    matched
}

/// `*` and `?` glob matching, case-insensitive like OpenSSH host patterns.
fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// An `IdentityFile` value as a local path: `~` and the `%d %u %h %r %%`
/// tokens expanded.
pub fn expand_identity(value: &str, home: &Path, host: &str, user: &str) -> PathBuf {
    let local_user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let mut expanded = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            expanded.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => expanded.push_str(&home.to_string_lossy()),
            Some('u') => expanded.push_str(&local_user),
            Some('h') => expanded.push_str(host),
            Some('r') => expanded.push_str(user),
            Some('%') => expanded.push('%'),
            Some(other) => {
                expanded.push('%');
                expanded.push(other);
            }
            None => expanded.push('%'),
        }
    }
    match expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
    {
        Some(rest) => home.join(rest),
        None if expanded == "~" => home.to_path_buf(),
        None => PathBuf::from(expanded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
# global
IdentityFile ~/.ssh/global_key

Host dev box-*
    HostName 10.0.0.5
    User ubuntu
    Port 2222
    IdentityFile "~/.ssh/my key"

Host box-* !box-skip
    User other
    HostName ignored.example

Match host dev
    User never

Host *
    Port 22
    User fallback
"#;

    #[test]
    fn first_value_wins_and_identity_files_add_up() {
        let config = SshConfig::parse(CONFIG);
        assert_eq!(
            config.resolve("dev"),
            HostConfig {
                host_name: Some("10.0.0.5".into()),
                user: Some("ubuntu".into()),
                port: Some(2222),
                identity_files: vec!["~/.ssh/global_key".into(), "~/.ssh/my key".into()],
                identity_agent: None,
                identities_only: false,
            }
        );
    }

    #[test]
    fn negation_and_wildcards() {
        let config = SshConfig::parse(CONFIG);
        assert_eq!(config.resolve("box-1").user.as_deref(), Some("ubuntu"));
        let skipped = config.resolve("box-skip");
        assert_eq!(skipped.host_name.as_deref(), Some("10.0.0.5"));
        let unknown = config.resolve("elsewhere");
        assert_eq!(unknown.host_name, None);
        assert_eq!(unknown.user.as_deref(), Some("fallback"));
        assert_eq!(unknown.port, Some(22));
    }

    #[test]
    fn aliases_skip_patterns() {
        assert_eq!(SshConfig::parse(CONFIG).aliases(), vec!["dev".to_string()]);
    }

    #[test]
    fn equals_sign_and_case() {
        let config = SshConfig::parse("HOST a\n  hostname=1.2.3.4\n  PORT = 2200\n");
        let resolved = config.resolve("A");
        assert_eq!(resolved.host_name.as_deref(), Some("1.2.3.4"));
        assert_eq!(resolved.port, Some(2200));
    }

    #[test]
    fn includes_are_followed_within_their_block() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(ssh.join("conf.d")).unwrap();
        std::fs::write(
            ssh.join("conf.d").join("a.conf"),
            "Host lab\n HostName 10.9.9.9\n",
        )
        .unwrap();
        std::fs::write(ssh.join("conf.d").join("b.conf"), "Host gpu\n User root\n").unwrap();
        std::fs::write(ssh.join("common"), "IdentityFile ~/.ssh/shared\n").unwrap();
        let mut config = SshConfig::empty();
        config.parse_into(
            "Include conf.d/*.conf\nHost web\n Include common\n Port 2201\n",
            Some((home.path(), 0)),
        );
        assert_eq!(config.aliases(), vec!["lab", "gpu", "web"]);
        assert_eq!(config.resolve("lab").host_name.as_deref(), Some("10.9.9.9"));
        let web = config.resolve("web");
        assert_eq!(web.identity_files, vec!["~/.ssh/shared".to_string()]);
        assert_eq!(web.port, Some(2201));
        assert_eq!(config.resolve("gpu").port, None);
    }

    #[test]
    fn includes_stop_at_a_loop() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("self"), "Include self\nHost x\n").unwrap();
        let mut config = SshConfig::empty();
        config.read_file(&ssh.join("self"), home.path(), 0);
        assert_eq!(config.aliases(), vec!["x"]);
    }

    #[test]
    fn openssh_output() {
        let config = parse_openssh(
            "host dev\nuser lab\nhostname 10.0.0.5\nport 2222\nidentitiesonly yes\nidentityagent ~/.1password/agent.sock\nidentityfile ~/.ssh/a\nidentityfile ~/.ssh/b\nidentityfile ~/.ssh/a\n",
        )
        .unwrap();
        assert_eq!(
            config,
            HostConfig {
                host_name: Some("10.0.0.5".into()),
                user: Some("lab".into()),
                port: Some(2222),
                identity_files: vec!["~/.ssh/a".into(), "~/.ssh/b".into()],
                identity_agent: Some("~/.1password/agent.sock".into()),
                identities_only: true,
            }
        );
        assert_eq!(parse_openssh("garbage"), None);
        assert_eq!(openssh("-oProxyCommand=x", None, None), None);
    }

    #[test]
    fn identity_expansion() {
        let home = Path::new("/home/me");
        assert_eq!(
            expand_identity("~/.ssh/id_%h_%r", home, "srv", "root"),
            home.join(".ssh/id_srv_root")
        );
        assert_eq!(
            expand_identity("/keys/%%x", home, "srv", "root"),
            PathBuf::from("/keys/%x")
        );
    }
}
