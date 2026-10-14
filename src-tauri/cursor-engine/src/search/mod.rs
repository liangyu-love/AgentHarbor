//! Web search and fetch for the WebSearch and WebFetch tools. Search is the
//! host's (AgentHarbor routes it like any other WebSearch); fetching a page
//! happens here.
mod cache;
mod fetch;

use std::sync::Arc;

pub use cache::{WebCache, WebCacheEntry, CACHE_ROUTE as WEB_CACHE_ROUTE};
pub use fetch::{FetchError, FetchedPage, WebFetch};

use crate::host::{Host, Searcher};

#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub chunk: String,
}

#[derive(Debug, thiserror::Error)]
#[error("web search failed: {0}")]
pub struct SearchError(String);

#[derive(Clone, Default)]
pub struct WebSearch {
    host: Option<Host>,
    fixed: Option<Arc<dyn Searcher>>,
}

impl WebSearch {
    /// No search available.
    pub fn built_in() -> Self {
        Self::default()
    }

    /// Whatever search the host offers when a search runs.
    pub fn managed(host: Host) -> Self {
        Self { host: Some(host), fixed: None }
    }

    pub fn with_searcher(searcher: Arc<dyn Searcher>) -> Self {
        Self { host: None, fixed: Some(searcher) }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(SearchError("query is empty".into()));
        }
        let searcher = self.fixed.clone().or_else(|| self.host.as_ref()?.searcher())
            .ok_or_else(|| SearchError("no web search model is set up in AgentHarbor".into()))?;
        searcher.search(query.to_owned()).await.map_err(SearchError)
    }
}
