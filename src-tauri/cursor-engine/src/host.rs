//! What the embedding application supplies: the models Cursor may pick, the
//! settings for Tab and commit messages, web search, and how to reach the
//! internet. The engine reads these on every request, so a change applies at
//! once.
use std::{future::Future, pin::Pin, sync::Arc};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::{model::ModelConfig, search::SearchHit};

pub type SearchFuture = Pin<Box<dyn Future<Output = Result<Vec<SearchHit>, String>> + Send>>;

/// Lists the models to offer, read again whenever the last list is older
/// than [`MODELS_FRESH_FOR`].
pub type ModelSource = Arc<dyn Fn() -> Vec<ModelConfig> + Send + Sync>;

const MODELS_FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// Runs a web search for the WebSearch tool.
pub trait Searcher: Send + Sync {
    fn search(&self, query: String) -> SearchFuture;
}

/// Where Cursor Tab completions come from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum TabMode {
    /// Cursor's own Tab service, untouched.
    #[default]
    Official,
    /// A model the user picked, by its Cursor-facing id.
    Model { model_id: String },
}

/// The language commit messages are written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommitLocale {
    #[default]
    English,
    Chinese,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommitSettings {
    /// The model that writes commit messages; none leaves them to Cursor.
    pub model_id: Option<String>,
    pub locale: CommitLocale,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineSettings {
    pub tab: TabMode,
    pub commit: CommitSettings,
}

/// How the engine itself reaches Cursor and the web.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Egress {
    /// The system's proxy settings.
    #[default]
    System,
    /// No proxy.
    Direct,
    /// Through this proxy URL.
    Proxy(String),
}

#[derive(Clone, Default)]
pub struct Host {
    inner: Arc<HostInner>,
}

#[derive(Default)]
struct HostInner {
    models: RwLock<Vec<ModelConfig>>,
    source: RwLock<Option<ModelSource>>,
    read_at: RwLock<Option<std::time::Instant>>,
    settings: RwLock<EngineSettings>,
    searcher: RwLock<Option<Arc<dyn Searcher>>>,
    egress: RwLock<Egress>,
    revision: std::sync::atomic::AtomicU64,
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    /// The models offered to Cursor, in picker order.
    pub fn models(&self) -> Vec<ModelConfig> {
        self.refresh();
        self.inner.models.read().clone()
    }

    pub fn model(&self, model_hash: &str) -> Option<ModelConfig> {
        self.refresh();
        self.inner.models.read().iter().find(|model| model.model_hash == model_hash).cloned()
    }

    /// A fixed list, replacing any source.
    pub fn set_models(&self, models: Vec<ModelConfig>) {
        *self.inner.source.write() = None;
        *self.inner.models.write() = models;
    }

    /// Where the list comes from from now on.
    pub fn set_model_source(&self, source: ModelSource) {
        *self.inner.source.write() = Some(source);
        *self.inner.read_at.write() = None;
    }

    fn refresh(&self) {
        let Some(source) = self.inner.source.read().clone() else { return };
        let fresh = self.inner.read_at.read().is_some_and(|at| at.elapsed() < MODELS_FRESH_FOR);
        if fresh {
            return;
        }
        let models = source();
        *self.inner.models.write() = models;
        *self.inner.read_at.write() = Some(std::time::Instant::now());
    }

    pub fn settings(&self) -> EngineSettings {
        self.inner.settings.read().clone()
    }

    pub fn set_settings(&self, settings: EngineSettings) {
        *self.inner.settings.write() = settings;
    }

    pub fn searcher(&self) -> Option<Arc<dyn Searcher>> {
        self.inner.searcher.read().clone()
    }

    pub fn set_searcher(&self, searcher: Option<Arc<dyn Searcher>>) {
        *self.inner.searcher.write() = searcher;
    }

    pub fn egress(&self) -> Egress {
        self.inner.egress.read().clone()
    }

    /// Replaces how the engine reaches the network; clients are rebuilt.
    pub fn set_egress(&self, egress: Egress) {
        let mut current = self.inner.egress.write();
        if *current != egress {
            *current = egress;
            self.inner.revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn egress_revision(&self) -> u64 {
        self.inner.revision.load(std::sync::atomic::Ordering::Relaxed)
    }
}
