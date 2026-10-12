//! Account sources signed in through a vendor's own CLI (Grok, Cursor). The
//! CLI is found where its installer puts it, run for its sign-in with the
//! link it prints handed to the window, and run again to renew its token:
//! AgentHarbor reads that token where the CLI keeps it and never keeps it.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

/// ANSI escapes a CLI colours its output with.
fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// The program names to look for on this system: a Windows CLI may be an
/// .exe or an npm-style .cmd shim.
fn file_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_owned()]
    }
}

/// The first of `names` in `dirs`, then on PATH, that `accept` takes.
pub fn find(names: &[&str], dirs: &[PathBuf], accept: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    for (dir, own) in dirs.iter().map(|dir| (dir, true)).chain(path_dirs.iter().map(|dir| (dir, false))) {
        if !dir.is_absolute() {
            continue;
        }
        for name in names {
            for file in file_names(name) {
                let candidate = dir.join(file);
                if candidate.is_file() && (own || accept(&candidate)) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The app proxy as the environment a CLI reads, when it is an HTTP one.
pub fn proxy_env(app_data_dir: &Path) -> Vec<(String, String)> {
    let proxy = crate::core::newapi::proxy_from_settings(&app_data_dir.join("settings.json"));
    match proxy.url.filter(|_| proxy.mode == crate::core::models::ApiProxyMode::Manual) {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
            ["HTTPS_PROXY", "HTTP_PROXY"].into_iter().map(|name| (name.to_owned(), url.clone())).collect()
        }
        _ => Vec::new(),
    }
}

/// The CLI run with `args` and `env` added, without a console window.
pub fn command(exe: &Path, args: &[&str], env: &[(String, String)]) -> Command {
    let shim = exe.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    let mut command = if shim {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(exe);
        command
    } else {
        Command::new(exe)
    };
    command.args(args);
    for (name, value) in env {
        command.env(name, value);
    }
    if let Some(home) = dirs::home_dir() {
        command.current_dir(home);
    }
    command.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::platform::windows::CREATE_NO_WINDOW);
    command
}

/// Run to its end within `timeout`: its standard output, or why it failed.
pub async fn run(mut command: Command, timeout: Duration) -> Result<String, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().map_err(|error| error.to_string())?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => Ok(plain(&String::from_utf8_lossy(&output.stdout))),
        Ok(Ok(output)) => {
            let said = plain(&String::from_utf8_lossy(&output.stderr));
            let said = said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default().trim().to_owned();
            Err(if said.is_empty() { format!("exited with {}", output.status) } else { said })
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("timed out".into()),
    }
}

/// A sign-in the CLI is running, its link already printed.
pub struct Login {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
    last: String,
}

/// The first link a line holds.
fn link_in(line: &str) -> Option<&str> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches(['.', ',', ')', '"', '\'']))
}

/// Whether a line carries on a link the CLI broke across lines.
fn continues_link(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && !line.contains(char::is_whitespace)
        && line.chars().all(|ch| ch.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(ch))
}

/// Start the CLI's sign-in and wait for the link it prints; `whole` tells a
/// link that is complete from one cut where the CLI wrapped it.
pub async fn start_login(mut command: Command, whole: impl Fn(&str) -> bool, timeout: Duration) -> Result<(Login, String), String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let (sender, lines) = mpsc::unbounded_channel();
    for reader in [child.stdout.take().map(|out| Box::new(out) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|err| Box::new(err) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)].into_iter().flatten()
    {
        let sender = sender.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if sender.send(plain(&line)).is_err() {
                    break;
                }
            }
        });
    }
    let mut login = Login { child, lines, last: String::new() };
    let found = tokio::time::timeout(timeout, async {
        let mut link = String::new();
        loop {
            // A complete link still takes what the CLI printed with it: the
            // rest of a query it wrapped after a parameter.
            let line = if !link.is_empty() && whole(&link) {
                match tokio::time::timeout(Duration::from_millis(200), login.lines.recv()).await {
                    Ok(Some(line)) if line.trim().starts_with('&') && continues_link(&line) => {
                        link.push_str(line.trim());
                        continue;
                    }
                    _ => return Some(link),
                }
            } else {
                login.lines.recv().await
            };
            let Some(line) = line else {
                return (!link.is_empty()).then_some(link);
            };
            if !line.trim().is_empty() {
                login.last = line.trim().to_owned();
            }
            if link.is_empty() {
                if let Some(found) = link_in(&line) {
                    link = found.to_owned();
                }
            } else if continues_link(&line) {
                link.push_str(line.trim());
            } else {
                return Some(link);
            }
        }
    })
    .await;
    match found {
        Ok(Some(link)) => Ok((login, link)),
        Ok(None) => Err(format!("the sign-in printed no link{}", if login.last.is_empty() { String::new() } else { format!(": {}", login.last) })),
        Err(_) => Err("the sign-in printed no link in time".into()),
    }
}

impl Login {
    /// Wait for the CLI to finish signing in, or stop it when cancelled.
    pub async fn finish(mut self, mut cancel: watch::Receiver<bool>, timeout: Duration) -> Result<(), String> {
        let wait = async {
            loop {
                tokio::select! {
                    status = self.child.wait() => return status.map_err(|error| error.to_string()),
                    line = self.lines.recv() => if let Some(line) = line.filter(|line| !line.trim().is_empty()) {
                        self.last = line.trim().to_owned();
                    },
                }
            }
        };
        let outcome = tokio::select! {
            outcome = tokio::time::timeout(timeout, wait) => outcome,
            _ = super::cancelled(&mut cancel) => {
                let _ = self.child.kill().await;
                return Err("accounts.loginCancelled".into());
            }
        };
        match outcome {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(_)) => Err(if self.last.is_empty() { "the sign-in did not finish".into() } else { self.last }),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = self.child.kill().await;
                Err("accounts.loginTimeout".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_read_out_of_coloured_output_and_wrapped_lines() {
        assert_eq!(plain("\u{1b}[1;32mOpen\u{1b}[0m https://x.ai/device?code=AB"), "Open https://x.ai/device?code=AB");
        assert_eq!(link_in("Visit https://accounts.x.ai/device?user_code=ABCD to sign in."), Some("https://accounts.x.ai/device?user_code=ABCD"));
        assert_eq!(link_in("no link here"), None);
        assert!(continues_link("&mode=login&redirectTarget=cli"));
        assert!(!continues_link("Waiting for sign-in..."));
    }

    #[tokio::test]
    async fn a_sign_in_hands_on_its_link_then_waits_for_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let exe = {
            let script = dir.path().join("fake.cmd");
            std::fs::write(&script, "@echo off
echo Open this link:
echo https://example.test/login?challenge=c^&uuid=u
echo ^&mode=login
echo Waiting...
exit /b 0
").unwrap();
            script
        };
        #[cfg(unix)]
        let exe = {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.path().join("fake.sh");
            std::fs::write(&script, "#!/bin/sh
echo 'Open this link:'
echo 'https://example.test/login?challenge=c&uuid=u'
echo '&mode=login'
echo 'Waiting...'
exit 0
").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        };
        let args: Vec<&str> = Vec::new();
        let whole = |link: &str| link.contains("mode=login");
        let (login, link) = start_login(command(&exe, &args, &[]), whole, Duration::from_secs(10)).await.unwrap();
        assert_eq!(link, "https://example.test/login?challenge=c&uuid=u&mode=login");
        let (_keep, cancel) = watch::channel(false);
        login.finish(cancel, Duration::from_secs(10)).await.unwrap();
    }
}
