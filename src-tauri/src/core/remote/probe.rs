//! What runs on a host: which clients are installed, and one small request
//! through any of them as it is configured, so the user sees the
//! configuration work end to end.

use super::{connection::quote, RemoteSession};
use crate::core::{client_config::ConfigRequest, targets::TargetId};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};

/// A shell function printing where command `$1` is: on the non-interactive
/// PATH, the login shell's, or `~/.local/bin` (where Codex and Claude Code
/// install).
pub(super) const FIND: &str = r#"find() { p=$(command -v "$1" 2>/dev/null) || p=$(bash -lc "command -v $1" 2>/dev/null) || p=; if [ -z "$p" ] && [ -x "$HOME/.local/bin/$1" ]; then p="$HOME/.local/bin/$1"; fi; printf %s "$p"; }"#;
const PROMPT: &str = "Reply with exactly the word: pong";
const PROBE_LIMIT: Duration = Duration::from_secs(150);

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub codex: Option<String>,
    pub claude_code: Option<String>,
    pub opencode: Option<String>,
    pub pi: Option<String>,
}

/// The versions of the CLIs on the host; a missing one is `None`.
pub fn installed(session: &RemoteSession) -> Installed {
    let script = format!(
        "{FIND}; for c in codex claude opencode pi; do b=$(find $c); if [ -n \"$b\" ]; then printf '%s\\t' $c; \"$b\" --version 2>/dev/null | head -n 1; fi; done"
    );
    let Ok((_, output)) = session.run(&script, Duration::from_secs(20)) else {
        return Installed::default();
    };
    let mut installed = Installed::default();
    for line in output.lines() {
        let Some((name, version)) = line.split_once('\t') else {
            continue;
        };
        let version = Some(version.trim().to_owned()).filter(|version| !version.is_empty());
        match name {
            "codex" => installed.codex = version,
            "claude" => installed.claude_code = version,
            "opencode" => installed.opencode = version,
            "pi" => installed.pi = version,
            _ => {}
        }
    }
    installed
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// The client answered `pong`.
    pub ok: bool,
    /// The end of what the client printed.
    pub output: String,
    pub elapsed_ms: u64,
}

/// Asks `client` on the host for a one-word reply as it is configured;
/// `codex_profile` runs Codex with that profile, and `pi` is the request Pi
/// was configured with, whose model it is asked for by name.
pub fn probe(
    session: &RemoteSession,
    client: TargetId,
    codex_profile: Option<&str>,
    pi: Option<&ConfigRequest>,
) -> Result<Probe, String> {
    let run = match client {
        TargetId::Codex => {
            let profile = match codex_profile {
                // Profile names are letters, digits, - and _ only.
                Some(name)
                    if name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
                {
                    format!("-p {name} ")
                }
                Some(_) => return Err("remote.errors.codexProfile".into()),
                None => String::new(),
            };
            format!("b=$(find codex); [ -n \"$b\" ] || {{ echo 'codex: not found'; exit 127; }}; \"$b\" {profile}exec --skip-git-repo-check '{PROMPT}'")
        }
        TargetId::ClaudeCode => {
            format!("b=$(find claude); [ -n \"$b\" ] || {{ echo 'claude: not found'; exit 127; }}; \"$b\" -p '{PROMPT}'")
        }
        // OpenCode runs the model its configuration names.
        TargetId::OpenCode => {
            format!("b=$(find opencode); [ -n \"$b\" ] || {{ echo 'opencode: not found'; exit 127; }}; \"$b\" run '{PROMPT}'")
        }
        TargetId::Pi => {
            let request = pi.ok_or("clients.errors.notConfigured")?;
            let models = session.client_paths().pi;
            let text = crate::core::client_config::ClientFiles::read(session, &models)?;
            let model = match text.as_deref().and_then(|text| pi_provider(text, request)) {
                Some(provider) => format!(
                    "--provider {} --model {}",
                    quote(&provider),
                    quote(&request.model)
                ),
                None => format!("--model {}", quote(&request.model)),
            };
            format!("b=$(find pi); [ -n \"$b\" ] || {{ echo 'pi: not found'; exit 127; }}; \"$b\" {model} --print '{PROMPT}'")
        }
        _ => return Err("clients.errors.unsupported".into()),
    };
    let script = format!("{FIND}; cd /tmp && {{ {run}; }} </dev/null 2>&1 | tail -n 24");
    let started = Instant::now();
    let (_, output) = session.run(&script, PROBE_LIMIT)?;
    Ok(Probe {
        ok: answered(&output),
        output: output.trim().to_owned(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// The provider of Pi's `models.json` this app wrote with `request`'s model,
/// its relay's first.
fn pi_provider(models: &str, request: &ConfigRequest) -> Option<String> {
    let models: Value = serde_json::from_str(models).ok()?;
    let relay = format!("agentharbor-relay-{}-", request.provider_id);
    let mut found: Vec<&String> = models["providers"]
        .as_object()?
        .iter()
        .filter(|(key, provider)| {
            key.starts_with("agentharbor-")
                && provider["models"].as_array().is_some_and(|models| {
                    models
                        .iter()
                        .any(|model| model["id"] == request.model.as_str())
                })
        })
        .map(|(key, _)| key)
        .collect();
    found.sort_by_key(|key| !key.starts_with(&relay));
    found.first().map(|key| (*key).clone())
}

fn answered(output: &str) -> bool {
    output.lines().any(|line| {
        line.trim()
            .trim_end_matches('.')
            .eq_ignore_ascii_case("pong")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_counts_only_on_its_own_line() {
        assert!(answered("codex\npong\ntokens used\n57\npong\n"));
        assert!(answered("Pong.\n"));
        assert!(!answered(
            "Reply with exactly the word: pong\nERROR: unauthorized\n"
        ));
    }

    #[test]
    fn pi_is_asked_for_its_relay_by_name() {
        let request: ConfigRequest = serde_json::from_value(serde_json::json!({
            "targetId": "pi", "providerId": "mimo", "model": "mimo-v2.6-flash"
        }))
        .unwrap();
        let models = r#"{"providers": {
            "openai": {"models": [{"id": "mimo-v2.6-flash"}]},
            "agentharbor-relay-other-openai-completions": {"models": [{"id": "mimo-v2.6-flash"}]},
            "agentharbor-relay-mimo-anthropic-messages": {"models": [{"id": "mimo-v2.6-flash"}]}
        }}"#;
        assert_eq!(
            pi_provider(models, &request).as_deref(),
            Some("agentharbor-relay-mimo-anthropic-messages")
        );
        assert_eq!(pi_provider("{}", &request), None);
    }
}
