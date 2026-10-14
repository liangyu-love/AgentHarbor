//! Reusable outbound HTTP clients, rebuilt when the host changes how the
//! engine reaches the network.
use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;

use crate::{
    host::{Egress, Host},
    Error, Result,
};

#[derive(Clone)]
pub struct NetworkClients {
    host: Host,
    cache: Arc<Mutex<ClientCache>>,
}

#[derive(Default)]
struct ClientCache {
    revision: u64,
    default: Option<reqwest::Client>,
    cursor: Option<reqwest::Client>,
    provider: Option<(Duration, reqwest::Client)>,
}

impl NetworkClients {
    pub fn new(host: Host) -> Self {
        Self { host, cache: Arc::new(Mutex::new(ClientCache::default())) }
    }

    fn cache(&self) -> parking_lot::MutexGuard<'_, ClientCache> {
        let mut cache = self.cache.lock();
        let revision = self.host.egress_revision();
        if cache.revision != revision {
            *cache = ClientCache { revision, ..ClientCache::default() };
        }
        cache
    }

    /// For web pages: follows the host's egress.
    pub async fn default_client(&self) -> Result<reqwest::Client> {
        let mut cache = self.cache();
        if let Some(client) = cache.default.clone() {
            return Ok(client);
        }
        let client = client_builder(&self.host.egress())?.build()?;
        cache.default = Some(client.clone());
        Ok(client)
    }

    /// For Cursor's own servers: no redirects followed, bodies passed as sent.
    pub async fn cursor_client(&self) -> Result<reqwest::Client> {
        let mut cache = self.cache();
        if let Some(client) = cache.cursor.clone() {
            return Ok(client);
        }
        let client = client_builder(&self.host.egress())?
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        cache.cursor = Some(client.clone());
        Ok(client)
    }

    /// For model calls. They go to AgentHarbor's own router on loopback, so
    /// no proxy ever applies.
    pub async fn provider_client(&self, timeout: Duration) -> Result<reqwest::Client> {
        let mut cache = self.cache();
        if let Some((_, client)) = cache.provider.as_ref().filter(|(cached, _)| *cached == timeout) {
            return Ok(client.clone());
        }
        let client = reqwest::Client::builder().no_proxy().timeout(timeout).build()?;
        cache.provider = Some((timeout, client.clone()));
        Ok(client)
    }
}

pub fn client_builder(egress: &Egress) -> Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    Ok(match egress {
        Egress::System => builder,
        Egress::Direct => builder.no_proxy(),
        Egress::Proxy(url) => builder.no_proxy().proxy(
            reqwest::Proxy::all(url).map_err(|error| Error::Config(format!("invalid proxy {url}: {error}")))?,
        ),
    })
}
