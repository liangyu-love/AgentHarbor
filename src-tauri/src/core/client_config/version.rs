//! Best-effort detection of the installed Claude Code CLI version, used to
//! qualify the modelPicker notice. Detection never blocks anything: failures
//! simply leave the version unknown.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static DETECTED: OnceLock<Option<String>> = OnceLock::new();

/// The detected `claude` version ("2.1.285"), cached per process. `None` when
/// no usable CLI answered — the UI then only shows a generic notice.
pub(super) fn detected() -> Option<String> {
    DETECTED.get_or_init(detect).clone()
}

fn detect() -> Option<String> {
    candidates().into_iter().find_map(|mut command| {
        let output = run_with_timeout(&mut command)?;
        parse_version(&output)
    })
}

#[cfg(windows)]
fn candidates() -> Vec<Command> {
    // The npm shim is a .cmd batch file, which CreateProcess will not run
    // directly; going through cmd resolves shims and bare exes alike.
    let mut through_cmd = Command::new("cmd");
    through_cmd.arg("/c").arg("claude");
    vec![Command::new("claude"), through_cmd]
}

#[cfg(not(windows))]
fn candidates() -> Vec<Command> {
    vec![Command::new("claude")]
}

fn run_with_timeout(command: &mut Command) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::platform::windows::CREATE_NO_WINDOW);
    }
    let mut child = command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            // Timed out, or the spawn/wait machinery failed.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    stdout.read_to_string(&mut output).ok()?;
    Some(output)
}

/// The first whitespace token of the first version-like line:
/// "2.1.285 (Claude Code)" becomes "2.1.285".
fn parse_version(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let token = line.split_whitespace().next()?;
        let looks_like_version = !token.is_empty()
            && token
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
        looks_like_version.then(|| token.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_first_version_like_token() {
        assert_eq!(
            parse_version("2.1.285 (Claude Code)\n"),
            Some("2.1.285".into())
        );
        assert_eq!(parse_version("2.0.14\nmore text\n"), Some("2.0.14".into()));
        assert_eq!(parse_version("bash: claude: command not found\n"), None);
        assert_eq!(parse_version("2.1.x\n"), None);
        assert_eq!(parse_version(""), None);
    }
}
