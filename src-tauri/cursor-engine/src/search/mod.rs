//! Web search and fetch for the WebSearch and WebFetch tools. Search is
//! Cursor's own when Cursor is signed in, else the host's (AgentHarbor routes
//! it like any other WebSearch); fetching a page happens here.
mod cache;
mod cursor;
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
    /// Where Cursor's own search is called; Cursor's servers outside tests.
    cursor_upstream: Option<String>,
}

impl WebSearch {
    /// No search available.
    pub fn built_in() -> Self {
        Self::default()
    }

    /// Cursor's own search on the signed-in account, then whatever search the
    /// host offers when a search runs.
    pub fn managed(host: Host) -> Self {
        Self { host: Some(host), ..Self::default() }
    }

    pub fn with_searcher(searcher: Arc<dyn Searcher>) -> Self {
        Self { fixed: Some(searcher), ..Self::default() }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(SearchError("query is empty".into()));
        }
        let cursor = match &self.host {
            Some(host) => {
                let upstream = self.cursor_upstream.as_deref().unwrap_or(crate::api::cursor::proxy::CURSOR_UPSTREAM);
                cursor::search(host, upstream, query).await
            }
            None => None,
        };
        let cursor_error = match cursor {
            Some(Ok(hits)) => return Ok(hits),
            Some(Err(error)) => {
                tracing::warn!(%error, "Cursor web search failed; trying AgentHarbor's");
                Some(error)
            }
            None => None,
        };
        let Some(searcher) = self.fixed.clone().or_else(|| self.host.as_ref()?.searcher()) else {
            return Err(SearchError(match cursor_error {
                Some(error) => format!("Cursor's web search failed ({error}) and no web search service is set up in AgentHarbor to fall back on"),
                None => "Cursor is not signed in, so its web search is unavailable, and no web search service is set up in AgentHarbor".into(),
            }));
        };
        searcher.search(query.to_owned()).await.map_err(|error| SearchError(match cursor_error {
            Some(cursor_error) => format!("Cursor's web search failed ({cursor_error}); AgentHarbor's failed too ({error})"),
            None => error,
        }))
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Bytes, http::{HeaderMap, StatusCode}, routing::post, Router};
    use prost::Message;

    use super::*;
    use crate::{cursor::protocol::proto::aiserver::v1 as ai, host::SearchFuture};

    struct Fixed(Result<Vec<SearchHit>, String>);

    impl Searcher for Fixed {
        fn search(&self, _query: String) -> SearchFuture {
            let result = self.0.clone();
            Box::pin(async move { result })
        }
    }

    fn hit(url: &str) -> SearchHit {
        SearchHit { title: url.into(), url: url.into(), chunk: String::new() }
    }

    /// A Cursor upstream answering RunWebSearch: documents for the right
    /// token, an error otherwise.
    async fn upstream() -> String {
        let app = Router::new().route("/aiserver.v1.AiService/RunWebSearch", post(|headers: HeaderMap, body: Bytes| async move {
            let request = ai::RunWebSearchRequest::decode(body).unwrap();
            if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer signed-in") {
                return (StatusCode::UNAUTHORIZED, br#"{"code":"unauthenticated","message":"Not logged in"}"#.to_vec());
            }
            assert_eq!(headers.get("x-cursor-client-version").unwrap(), "3.0.0");
            let document = ai::WebSearchDocument { url: "https://cursor.example".into(), title: request.search_term, text: "found".into() };
            (StatusCode::OK, ai::RunWebSearchResponse { answer: None, documents: vec![document] }.encode_to_vec())
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    fn signed_in(host: &Host, token: &str) {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        headers.insert("x-cursor-client-version", "3.0.0".parse().unwrap());
        host.remember_cursor_session(headers);
    }

    fn search(host: &Host, upstream: String) -> WebSearch {
        host.set_egress(crate::Egress::Direct);
        WebSearch { cursor_upstream: Some(upstream), ..WebSearch::managed(host.clone()) }
    }

    #[tokio::test]
    async fn cursor_search_comes_first_when_signed_in() {
        let host = Host::new();
        host.set_searcher(Some(Arc::new(Fixed(Ok(vec![hit("https://agentharbor.example")])))));
        signed_in(&host, "signed-in");
        let hits = search(&host, upstream().await).search("rust").await.unwrap();
        assert_eq!(hits, vec![SearchHit { title: "rust".into(), url: "https://cursor.example".into(), chunk: "found".into() }]);
    }

    #[tokio::test]
    async fn agentharbor_search_backs_cursor_up() {
        let host = Host::new();
        host.set_searcher(Some(Arc::new(Fixed(Ok(vec![hit("https://agentharbor.example")])))));
        // Not signed in: straight to AgentHarbor's search.
        let web = search(&host, upstream().await);
        assert_eq!(web.search("rust").await.unwrap()[0].url, "https://agentharbor.example");
        // Cursor refuses the account: AgentHarbor's search still answers.
        signed_in(&host, "expired");
        assert_eq!(web.search("rust").await.unwrap()[0].url, "https://agentharbor.example");
    }

    #[tokio::test]
    async fn a_failure_names_both_searches() {
        let host = Host::new();
        let web = search(&host, upstream().await);
        let error = web.search("rust").await.unwrap_err().to_string();
        assert!(error.contains("Cursor is not signed in"), "{error}");
        signed_in(&host, "expired");
        let error = web.search("rust").await.unwrap_err().to_string();
        assert!(error.contains("HTTP 401 Unauthorized Not logged in") && error.contains("no web search service"), "{error}");
        host.set_searcher(Some(Arc::new(Fixed(Err("relay down".into())))));
        let error = web.search("rust").await.unwrap_err().to_string();
        assert!(error.contains("Not logged in") && error.contains("relay down"), "{error}");
    }
}
