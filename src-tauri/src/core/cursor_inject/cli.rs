//! `cursor-agent`: `CURSOR_API_ENDPOINT` points it at the local front. On
//! Windows the variable is set for the user, so every new terminal picks it
//! up; AgentHarbor puts the old value back when it lets go. Elsewhere there is
//! no user-wide place for it, so the command to use is shown instead.
use serde::{Deserialize, Serialize};

use crate::platform::terminal_environment::EnvironmentValue;

pub const VARIABLE: &str = "CURSOR_API_ENDPOINT";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub(crate) original: Option<EnvironmentValue>,
    pub written: String,
}

/// `cursor-agent` is installed where AgentHarbor looks for it.
pub fn installed() -> bool {
    crate::core::account_sources::cursor::executable().is_some()
}

/// Whether the user-wide variable can be managed here: on Windows, and not
/// while clients are isolated in a home of their own.
pub fn supported() -> bool {
    cfg!(windows) && std::env::var_os("AGENT_HARBOR_CLIENT_HOME").is_none_or(|home| home.is_empty())
}

/// How to run the CLI against the front by hand.
pub fn command(url: &str) -> String {
    if cfg!(windows) {
        format!("$env:{VARIABLE}=\"{url}\"; cursor-agent")
    } else {
        format!("{VARIABLE}={url} cursor-agent")
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::platform::terminal_environment::{windows::UserEnvironment, Environment};

    pub fn apply(url: &str, held: Option<&Lease>) -> Result<Lease, String> {
        let mut environment = UserEnvironment::open(true)?;
        let original = match held {
            Some(lease) => lease.original.clone(),
            // A loopback value with no lease is AgentHarbor's own, left by a
            // crash: there was nothing before it to put back.
            None => environment.read(VARIABLE)?.filter(|value| {
                !value.text().and_then(|text| reqwest::Url::parse(&text).ok()).is_some_and(|url| {
                    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                })
            }),
        };
        let value = EnvironmentValue::string(url);
        if environment.read(VARIABLE)?.as_ref() != Some(&value) {
            environment.write(VARIABLE, Some(&value))?;
            environment.notify();
        }
        Ok(Lease { original, written: url.to_owned() })
    }

    pub fn release(lease: &Lease) -> Result<(), String> {
        let mut environment = UserEnvironment::open(true)?;
        let current = environment.read(VARIABLE)?;
        // A value the user set since is theirs.
        if current.as_ref().and_then(EnvironmentValue::text).as_deref() == Some(lease.written.as_str()) {
            environment.write(VARIABLE, lease.original.as_ref())?;
            environment.notify();
        }
        Ok(())
    }

    pub fn current() -> Option<String> {
        UserEnvironment::open(false).ok()?.read(VARIABLE).ok()??.text()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn apply(_url: &str, _held: Option<&Lease>) -> Result<Lease, String> {
        Err("cursor.errors.cliUnsupported".into())
    }

    pub fn release(_lease: &Lease) -> Result<(), String> {
        Ok(())
    }

    pub fn current() -> Option<String> {
        std::env::var(VARIABLE).ok()
    }
}

pub use platform::{apply, current, release};

/// Opens a terminal whose `cursor-agent` uses the front.
pub fn open_terminal(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::platform::terminal_environment::windows::open_terminal(&[(VARIABLE.to_owned(), url.to_owned())])
    }
    #[cfg(not(windows))]
    {
        crate::platform::process::open_terminal_with_command(&command(url)).map_err(|error| error.to_string())
    }
}
