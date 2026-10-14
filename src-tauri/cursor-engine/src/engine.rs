//! Assembles the engine: conversation store, prompts, model calls through the
//! host's models, the Cursor API router and the local front.
use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::{
    access::{Authority, Front},
    api::cursor::proxy::CursorProxy,
    cursor::{
        prompting::{PromptAssets, PromptCompiler},
        TransportRegistry,
    },
    host::Host,
    network::NetworkClients,
    provider::ProviderRouter,
    search::WebCache,
    store::Store,
    Result,
};

/// A model call may run this long in all.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// A model stream may stay silent this long.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct EngineConfig {
    /// Where conversations, fetched pages and the certificate authority live.
    pub data_dir: PathBuf,
    /// The port to listen on; another is used when it is taken, any when 0.
    pub port: u16,
}

pub struct Engine {
    host: Host,
    registry: TransportRegistry,
    authority: Arc<Authority>,
    front: Front,
}

impl Engine {
    pub async fn start(config: EngineConfig, host: Host) -> Result<Self> {
        let store = Store::open(&config.data_dir.join("conversations.db")).await?;
        let compiler = PromptCompiler::new(PromptAssets::embedded()?);
        let clients = NetworkClients::new(host.clone());
        let provider = Arc::new(ProviderRouter::new(
            store.clone(),
            host.clone(),
            clients.clone(),
            REQUEST_TIMEOUT,
            STREAM_IDLE_TIMEOUT,
        ));
        let web_cache = WebCache::at(config.data_dir.join("web-cache"))?;
        let registry = TransportRegistry::new(store, host.clone(), provider, compiler, web_cache.clone());
        let router = crate::api::cursor::router(registry.clone(), CursorProxy::cursor(clients));
        let authority = Arc::new(Authority::load_or_create(&config.data_dir)?);
        let front = Front::bind(config.port, router, Some(authority.clone()), host.clone()).await?;
        web_cache.set_service_addr(([127, 0, 0, 1], front.port()).into());
        log::info!("Cursor engine listening on {}", front.url());
        Ok(Self { host, registry, authority, front })
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn authority(&self) -> &Arc<Authority> {
        &self.authority
    }

    pub fn port(&self) -> u16 {
        self.front.port()
    }

    /// The proxy for the IDE and the API endpoint for `cursor-agent`.
    pub fn url(&self) -> String {
        self.front.url()
    }

    /// Cancels running conversations and closes the port.
    pub async fn stop(self) {
        self.registry.shutdown().await;
        self.front.stop().await;
    }
}
