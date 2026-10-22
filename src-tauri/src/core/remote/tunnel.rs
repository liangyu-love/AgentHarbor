//! Reverse tunnels. While a host has clients configured through this
//! machine's router, the host's `127.0.0.1:<tunnel port>` is forwarded here,
//! so those clients reach the router as local ones do. Each tunnel is one
//! background task that reconnects with a growing pause until it is stopped.

use super::connection::{self, KnownHosts, Target};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum TunnelState {
    Connecting,
    #[serde(rename_all = "camelCase")]
    Connected {
        since: i64,
    },
    /// The last attempt failed (an i18n key); another follows.
    #[serde(rename_all = "camelCase")]
    Retrying {
        error: String,
    },
}

struct Running {
    /// Where it forwards: the server's port and this machine's router port.
    ports: (u16, u16),
    stop: tokio::sync::watch::Sender<bool>,
    state: Arc<Mutex<TunnelState>>,
}

fn tunnels() -> &'static Mutex<HashMap<String, Running>> {
    static TUNNELS: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    TUNNELS.get_or_init(Default::default)
}

fn set(state: &Mutex<TunnelState>, next: TunnelState) {
    if let Ok(mut slot) = state.lock() {
        *slot = next;
    }
}

/// Keeps a tunnel for `host` from the server's `remote_port` to this
/// machine's `local_port`, replacing one with other ports.
pub(super) fn ensure(
    host: &str,
    target: Target,
    known: Arc<KnownHosts>,
    remote_port: u16,
    local_port: u16,
) {
    let mut running = tunnels().lock().unwrap_or_else(|error| error.into_inner());
    if running
        .get(host)
        .is_some_and(|tunnel| tunnel.ports == (remote_port, local_port))
    {
        return;
    }
    if let Some(previous) = running.remove(host) {
        let _ = previous.stop.send(true);
    }
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let state = Arc::new(Mutex::new(TunnelState::Connecting));
    let shared = state.clone();
    let name = host.to_owned();
    connection::spawn(async move {
        let mut pause = Duration::from_secs(2);
        loop {
            set(&shared, TunnelState::Connecting);
            match connection::tunnel(&target, known.clone(), remote_port, local_port).await {
                Ok(handle) => {
                    set(
                        &shared,
                        TunnelState::Connected {
                            since: chrono::Utc::now().timestamp(),
                        },
                    );
                    pause = Duration::from_secs(2);
                    loop {
                        tokio::select! {
                            _ = stopped.changed() => {
                                let _ = handle.disconnect(russh::Disconnect::ByApplication, "", "en").await;
                                return;
                            }
                            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                                if handle.is_closed() {
                                    break;
                                }
                            }
                        }
                    }
                    set(
                        &shared,
                        TunnelState::Retrying {
                            error: "remote.errors.tunnelLost".into(),
                        },
                    );
                }
                Err(error) => {
                    log::warn!("tunnel to remote host {name} failed: {}", error.key());
                    set(&shared, TunnelState::Retrying { error: error.key() });
                }
            }
            tokio::select! {
                _ = stopped.changed() => return,
                _ = tokio::time::sleep(pause) => {}
            }
            pause = (pause * 2).min(Duration::from_secs(60));
        }
    });
    running.insert(
        host.to_owned(),
        Running {
            ports: (remote_port, local_port),
            stop,
            state,
        },
    );
}

pub(super) fn stop(host: &str) {
    if let Some(tunnel) = tunnels()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(host)
    {
        let _ = tunnel.stop.send(true);
    }
}

/// Stops every tunnel except those of `keep`.
pub(super) fn retain(keep: &[String]) {
    let mut running = tunnels().lock().unwrap_or_else(|error| error.into_inner());
    running.retain(|host, tunnel| {
        let kept = keep.contains(host);
        if !kept {
            let _ = tunnel.stop.send(true);
        }
        kept
    });
}

pub fn state(host: &str) -> Option<TunnelState> {
    tunnels()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(host)
        .and_then(|tunnel| tunnel.state.lock().ok().map(|state| state.clone()))
}

pub fn stop_all() {
    retain(&[]);
}
