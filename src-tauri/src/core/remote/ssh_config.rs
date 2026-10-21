//! The part of `~/.ssh/config` a remote host needs: which address, port,
//! user and identity files an alias stands for. `Host` blocks follow
//! OpenSSH's rules (the first value obtained wins, identity files add up,
//! `!` negates a pattern); `Match` blocks and `Include` are not followed.

use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq)]
pub struct HostConfig {
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<String>,
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
    /// The user's config; a missing or unreadable file is an empty one.
    pub fn load() -> Self {
        default_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or(Self { blocks: Vec::new() })
    }

    pub fn parse(text: &str) -> Self {
        // Options before the first `Host` apply to every host.
        let mut blocks = vec![Block {
            patterns: Some(vec!["*".into()]),
            options: Vec::new(),
        }];
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
                "host" => blocks.push(Block {
                    patterns: Some(args),
                    options: Vec::new(),
                }),
                "match" => blocks.push(Block {
                    patterns: None,
                    options: Vec::new(),
                }),
                _ => {
                    if let (Some(block), Some(value)) = (blocks.last_mut(), args.into_iter().next())
                    {
                        block.options.push((keyword, value));
                    }
                }
            }
        }
        Self { blocks }
    }

    /// What `alias` resolves to.
    pub fn resolve(&self, alias: &str) -> HostConfig {
        let mut config = HostConfig::default();
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
                    "identityfile" => {
                        if !config.identity_files.contains(value) {
                            config.identity_files.push(value.clone());
                        }
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
