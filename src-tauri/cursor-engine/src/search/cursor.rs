//! Cursor's own web search (`AiService/RunWebSearch`), run on the signed-in
//! user's account the way Cursor's Agent runs it.
use std::time::Duration;

use prost::Message;
use reqwest::header::CONTENT_TYPE;

use super::SearchHit;
use crate::{
    cursor::protocol::{connect, proto::aiserver::v1 as ai},
    host::Host,
};

const TIMEOUT: Duration = Duration::from_secs(45);

/// None when Cursor has not sent a signed-in request yet: there is no
/// account to search on.
pub(super) async fn search(
    host: &Host,
    upstream: &str,
    query: &str,
) -> Option<Result<Vec<SearchHit>, String>> {
    let session = host.cursor_session()?;
    Some(run(host, upstream, session, query).await)
}

async fn run(
    host: &Host,
    upstream: &str,
    session: reqwest::header::HeaderMap,
    query: &str,
) -> Result<Vec<SearchHit>, String> {
    let client = crate::network::client_builder(&host.egress())
        .map_err(|error| error.to_string())?
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let request = ai::RunWebSearchRequest {
        search_term: query.to_owned(),
        explanation: None,
        model_id: String::new(),
    };
    let response = client
        .post(format!("{upstream}/aiserver.v1.AiService/RunWebSearch"))
        .headers(session)
        .header(CONTENT_TYPE, "application/proto")
        .header("connect-protocol-version", "1")
        .body(request.encode_to_vec())
        .send()
        .await
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("reply unreadable: {error}"))?;
    if !status.is_success() {
        // Connect reports errors as JSON: {"code": "...", "message": "..."}.
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        let detail = error["message"]
            .as_str()
            .or(error["code"].as_str())
            .unwrap_or_default();
        return Err(format!("HTTP {status} {detail}").trim_end().to_owned());
    }
    let reply: ai::RunWebSearchResponse =
        connect::decode_unary(&body).map_err(|error| format!("reply unreadable: {error}"))?;
    let hits = hits(reply);
    if hits.is_empty() {
        return Err("the search returned no results".into());
    }
    Ok(hits)
}

/// Each document with a URL; the answer alone when there are none.
fn hits(reply: ai::RunWebSearchResponse) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = reply
        .documents
        .into_iter()
        .filter(|document| !document.url.trim().is_empty())
        .map(|document| SearchHit {
            title: if document.title.trim().is_empty() {
                document.url.clone()
            } else {
                document.title
            },
            url: document.url,
            chunk: document.text,
        })
        .collect();
    if hits.is_empty() {
        if let Some(answer) = reply.answer.filter(|answer| !answer.trim().is_empty()) {
            hits.push(SearchHit {
                title: "Cursor web search".into(),
                url: String::new(),
                chunk: answer,
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_become_hits_and_the_answer_stands_in_for_none() {
        let reply = ai::RunWebSearchResponse {
            answer: Some("summary".into()),
            documents: vec![
                ai::WebSearchDocument {
                    url: "https://a.example".into(),
                    title: "A".into(),
                    text: "a text".into(),
                },
                ai::WebSearchDocument {
                    url: "https://b.example".into(),
                    title: " ".into(),
                    text: String::new(),
                },
                ai::WebSearchDocument {
                    url: String::new(),
                    title: "no url".into(),
                    text: "dropped".into(),
                },
            ],
        };
        let hits = hits(reply);
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0],
            SearchHit {
                title: "A".into(),
                url: "https://a.example".into(),
                chunk: "a text".into()
            }
        );
        assert_eq!(hits[1].title, "https://b.example");

        let answer_only = hits_of(Some("only an answer"));
        assert_eq!(answer_only.len(), 1);
        assert_eq!(answer_only[0].chunk, "only an answer");
        assert!(hits_of(Some("  ")).is_empty());
    }

    fn hits_of(answer: Option<&str>) -> Vec<SearchHit> {
        hits(ai::RunWebSearchResponse {
            answer: answer.map(str::to_owned),
            documents: Vec::new(),
        })
    }
}
