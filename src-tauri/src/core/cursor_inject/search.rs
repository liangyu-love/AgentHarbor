//! Web search for Cursor's WebSearch tool: one Messages request with the
//! hosted `web_search` tool to the model chosen for WebSearch, through the
//! router, which runs the search the way it does for Claude Code and Codex.
use cursor_engine::{host::SearchFuture, search::SearchHit, Searcher};
use serde_json::{json, Value};

pub struct RouterSearch {
    pub url: String,
    pub token: String,
    /// The search model's gateway id.
    pub model: String,
}

impl Searcher for RouterSearch {
    fn search(&self, query: String) -> SearchFuture {
        let url = self.url.clone();
        let token = self.token.clone();
        let model = self.model.clone();
        Box::pin(async move {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .map_err(|error| error.to_string())?;
            let response = client
                .post(url)
                .header("x-api-key", token)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "max_tokens": 2048,
                    "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": 2}],
                    "messages": [{
                        "role": "user",
                        "content": format!("Search the web for: {query}\n\nThen list what the results say, citing them."),
                    }],
                }))
                .send()
                .await
                .map_err(|error| format!("search request failed: {error}"))?;
            let status = response.status();
            let value: Value = response.json().await.map_err(|error| format!("search reply unreadable: {error}"))?;
            if !status.is_success() {
                let message = value["error"]["message"].as_str().unwrap_or("search failed");
                return Err(format!("{status}: {message}"));
            }
            let hits = hits(&value);
            if hits.is_empty() {
                return Err("the search returned no results".into());
            }
            Ok(hits)
        })
    }
}

/// The results in a Messages reply: each result once, with the passages the
/// answer cited from it.
pub fn hits(reply: &Value) -> Vec<SearchHit> {
    let blocks = reply["content"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut found: Vec<SearchHit> = Vec::new();
    for block in blocks.iter().filter(|block| block["type"] == "web_search_tool_result") {
        for result in block["content"].as_array().into_iter().flatten() {
            let Some(url) = result["url"].as_str() else { continue };
            if found.iter().any(|hit| hit.url == url) {
                continue;
            }
            found.push(SearchHit {
                title: result["title"].as_str().unwrap_or(url).to_owned(),
                url: url.to_owned(),
                chunk: String::new(),
            });
        }
    }
    for block in blocks.iter().filter(|block| block["type"] == "text") {
        for citation in block["citations"].as_array().into_iter().flatten() {
            let (Some(url), Some(cited)) = (citation["url"].as_str(), citation["cited_text"].as_str()) else { continue };
            match found.iter_mut().find(|hit| hit.url == url) {
                Some(hit) if !hit.chunk.contains(cited) => {
                    if !hit.chunk.is_empty() {
                        hit.chunk.push_str(" … ");
                    }
                    hit.chunk.push_str(cited);
                }
                Some(_) => {}
                None => found.push(SearchHit {
                    title: citation["title"].as_str().unwrap_or(url).to_owned(),
                    url: url.to_owned(),
                    chunk: cited.to_owned(),
                }),
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_carry_the_passages_cited_from_them() {
        let reply = json!({"content": [
            {"type": "server_tool_use", "name": "web_search"},
            {"type": "web_search_tool_result", "content": [
                {"type": "web_search_result", "url": "https://a.example", "title": "A"},
                {"type": "web_search_result", "url": "https://b.example", "title": "B"},
                {"type": "web_search_result", "url": "https://a.example", "title": "A again"}
            ]},
            {"type": "text", "text": "A says x.", "citations": [
                {"type": "web_search_result_location", "url": "https://a.example", "title": "A", "cited_text": "x is true"},
                {"type": "web_search_result_location", "url": "https://c.example", "title": "C", "cited_text": "c"}
            ]}
        ]});
        let hits = hits(&reply);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0], SearchHit { title: "A".into(), url: "https://a.example".into(), chunk: "x is true".into() });
        assert_eq!(hits[1].chunk, "");
        assert_eq!(hits[2].url, "https://c.example");
    }
}
