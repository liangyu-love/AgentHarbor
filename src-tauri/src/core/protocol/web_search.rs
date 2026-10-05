//! Hosted search is a protocol capability, not a function tool. Only the
//! Messages -> Responses bridge has an equivalent upstream tool today.
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

const INVALID: &str = "protocol.errors.invalidRequest";
const UNSUPPORTED: &str = "protocol.errors.unsupportedSearchOptions";

#[derive(Clone, Debug)]
pub struct Search {
    pub name: String,
    pub max_uses: Option<u64>,
}

pub(super) fn is_search(tool: &Value) -> bool {
    tool["type"]
        .as_str()
        .is_some_and(|t| t == "web_search" || t.starts_with("web_search_"))
}

pub(super) fn prepare(target: ModelProtocol, tool: &Value) -> Result<(Search, Value), String> {
    if target != ModelProtocol::Responses {
        return Err("protocol.errors.unsupportedHostedTool".into());
    }
    let direct_default = match tool["type"].as_str() {
        Some("web_search" | "web_search_20250305") => true,
        Some("web_search_20260209" | "web_search_20260318") => false,
        _ => return Err(UNSUPPORTED.into()),
    };
    match tool.get("allowed_callers") {
        None if direct_default => {}
        Some(callers) if *callers == json!(["direct"]) => {}
        _ => return Err(UNSUPPORTED.into()),
    }
    // Responses cannot express domain exclusion or programmatic tool execution.
    if tool
        .get("blocked_domains")
        .is_some_and(|v| !v.is_null() && *v != json!([]))
        || tool.get("response_inclusion").is_some()
        || tool["defer_loading"] == true
    {
        return Err(UNSUPPORTED.into());
    }
    let name = tool["name"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        })
        .ok_or(INVALID)?;
    let max_uses = match tool.get("max_uses").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().filter(|n| *n > 0).ok_or(INVALID)?),
        None => None,
    };
    let mut upstream = json!({"type":"web_search"});
    if let Some(domains) = tool.get("allowed_domains").filter(|v| !v.is_null()) {
        let domains = domains.as_array().ok_or(INVALID)?;
        if domains.iter().any(|v| {
            v.as_str()
                .is_none_or(|s| s.is_empty() || s.chars().any(char::is_control))
        }) {
            return Err(INVALID.into());
        }
        if !domains.is_empty() {
            upstream["filters"] = json!({"allowed_domains":domains});
        }
    }
    if let Some(location) = tool.get("user_location").filter(|v| !v.is_null()) {
        if location["type"] != "approximate" {
            return Err(UNSUPPORTED.into());
        }
        let mut mapped = json!({"type":"approximate"});
        for key in ["city", "region", "country", "timezone"] {
            if let Some(v) = location.get(key).filter(|v| !v.is_null()) {
                if !v.is_string() {
                    return Err(INVALID.into());
                }
                mapped[key] = v.clone();
            }
        }
        upstream["user_location"] = mapped;
    }
    Ok((
        Search {
            name: name.into(),
            max_uses,
        },
        upstream,
    ))
}

fn url(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let parsed = reqwest::Url::parse(text).ok()?;
    (matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && !text.chars().any(char::is_control))
    .then_some(text)
}

pub(super) fn results(item: &Value) -> Vec<Value> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for source in item["action"]["sources"].as_array().into_iter().flatten() {
        let Some(url) = url(&source["url"]) else {
            continue;
        };
        if !seen.insert(url) {
            continue;
        }
        results.push(json!({"type":"web_search_result","url":url,
            "title":source["title"].as_str().unwrap_or(url),
            // Responses does not return Anthropic's opaque search payload.
            "encrypted_content":""}));
    }
    results
}

/// Some gateways omit action.sources but still return citations in the final
/// answer. Match known URLs first; attach unattributed citations to the last
/// successful search, as CC Switch does, without inventing call attribution.
pub(super) fn enrich_output(output: &[Value], max_uses: Option<u64>) -> Vec<Value> {
    let mut output = output.to_vec();
    let mut citations = Vec::new();
    let mut by_url = std::collections::HashMap::new();
    for item in &output {
        for part in item["content"].as_array().into_iter().flatten() {
            for annotation in part["annotations"].as_array().into_iter().flatten() {
                if annotation["type"] != "url_citation" {
                    continue;
                }
                if let Some(url) = url(&annotation["url"]) {
                    if !by_url.contains_key(url) {
                        by_url.insert(url.to_owned(), citations.len());
                        citations.push(json!({"type":"url","url":url,"title":annotation["title"].as_str().unwrap_or(url)}));
                    }
                }
            }
        }
    }
    let mut attributed = std::collections::HashSet::new();
    let mut last = None;
    let mut ordinal = 0;
    for (index, item) in output.iter_mut().enumerate() {
        if item["type"] != "web_search_call" {
            continue;
        }
        let allowed = max_uses.is_none_or(|max| ordinal < max);
        ordinal += 1;
        if !allowed || item["status"] != "completed" {
            continue;
        }
        last = Some(index);
        for source in item["action"]["sources"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if let Some(url) = url(&source["url"]).map(str::to_owned) {
                attributed.insert(url.clone());
                if source["title"].as_str().is_none_or(str::is_empty) {
                    if let Some(index) = by_url.get(&url) {
                        source["title"] = citations[*index]["title"].clone();
                    }
                }
            }
        }
    }
    if let Some(last) = last {
        let sources = &mut output[last]["action"]["sources"];
        if !sources.is_array() {
            *sources = json!([]);
        }
        sources.as_array_mut().unwrap().extend(
            citations
                .into_iter()
                .filter(|c| !attributed.contains(c["url"].as_str().unwrap())),
        );
    }
    output
}

/// Links retain real citations without inventing an Anthropic encrypted index.
/// Append-only rendering can be shared by nonstream and incremental replies.
pub(super) fn citation_suffix(part: &Value) -> String {
    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for annotation in part["annotations"].as_array().into_iter().flatten() {
        if annotation["type"] != "url_citation" {
            continue;
        }
        if let Some(url) = url(&annotation["url"]) {
            if seen.insert(url) {
                links.push(url);
            }
        }
    }
    if links.is_empty() {
        return String::new();
    }
    format!(
        "\n\n{}",
        links
            .iter()
            .enumerate()
            .map(|(i, url)| {
                let escaped = url
                    .replace('(', "%28")
                    .replace(')', "%29")
                    .replace('<', "%3C")
                    .replace('>', "%3E")
                    .replace(' ', "%20");
                format!("[{}]({escaped})", i + 1)
            })
            .collect::<Vec<_>>()
            .join(" ")
    )
}

pub(super) fn blocks(
    item: &Value,
    search: Option<&Search>,
    ordinal: u64,
) -> Result<Vec<Value>, String> {
    let id = item["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("protocol.errors.invalidResponse")?;
    let action = &item["action"];
    let input = match action["type"].as_str() {
        Some("search") => {
            let queries = action["queries"].as_array().filter(|q| !q.is_empty());
            let query = action["query"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    queries.map(|q| {
                        q.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                })
                .unwrap_or_default();
            json!({"query":query})
        }
        Some("open_page") => json!({"url":action["url"]}),
        Some("find_in_page") => json!({"url":action["url"],"pattern":action["pattern"]}),
        None if matches!(item["status"].as_str(), Some("failed" | "incomplete")) => json!({}),
        _ => return Err("protocol.errors.invalidResponse".into()),
    };
    let error = if search
        .and_then(|s| s.max_uses)
        .is_some_and(|max| ordinal >= max)
    {
        Some("max_uses_exceeded")
    } else if item["status"] != "completed" {
        Some("unavailable")
    } else {
        None
    };
    let content = error
        .map(|code| json!({"type":"web_search_tool_result_error","error_code":code}))
        .unwrap_or_else(|| json!(results(item)));
    Ok(vec![
        json!({"type":"server_tool_use","id":id,"name":search.map_or("web_search", |s| s.name.as_str()),"input":input}),
        json!({"type":"web_search_tool_result","tool_use_id":id,"content":content}),
    ])
}

/// Replay hosted context without treating a server tool as a client function.
/// The bounded conversation cache replaces these reconstructed items with the
/// exact original upstream output whenever it is still available.
pub(super) fn history_call(call: &Value, result: &Value) -> Result<Value, String> {
    let id = call["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(INVALID)?;
    if result["tool_use_id"] != id {
        return Err(INVALID.into());
    }
    let input = &call["input"];
    let mut action = if let Some(query) = input["query"].as_str() {
        json!({"type":"search","query":query})
    } else if let Some(url) = url(&input["url"]) {
        if let Some(pattern) = input["pattern"].as_str() {
            json!({"type":"find_in_page","url":url,"pattern":pattern})
        } else {
            json!({"type":"open_page","url":url})
        }
    } else if result["content"]["type"] == "web_search_tool_result_error" {
        // A failed hosted call may never receive action metadata.
        json!({"type":"search","query":""})
    } else {
        return Err(UNSUPPORTED.into());
    };
    let status = if let Some(items) = result["content"].as_array() {
        let sources: Vec<_> = items
            .iter()
            .filter_map(|r| url(&r["url"]).map(|u| json!({"type":"url","url":u})))
            .collect();
        action["sources"] = json!(sources);
        "completed"
    } else if result["content"]["type"] == "web_search_tool_result_error" {
        "failed"
    } else {
        return Err(INVALID.into());
    };
    Ok(json!({"type":"web_search_call","id":id,"status":status,"action":action}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_and_citations_exclude_unsafe_urls_and_deduplicate() {
        let sources = json!({"action":{"sources":[
            {"url":"javascript:alert(1)"}, {"url":"file:///secret"},
            {"url":"https://example.org/a(b)","title":"source"},
            {"url":"https://example.org/a(b)","title":"duplicate"}
        ]}});
        assert_eq!(results(&sources).len(), 1);
        assert_eq!(results(&sources)[0]["title"], "source");
        assert_eq!(
            citation_suffix(&json!({"annotations":[
                {"type":"url_citation","url":"javascript:alert(1)"},
                {"type":"url_citation","url":"https://example.org/a(b)"},
                {"type":"url_citation","url":"https://example.org/a(b)"}
            ]})),
            "\n\n[1](https://example.org/a%28b%29)"
        );
    }

    #[test]
    fn search_limit_and_failed_history_remain_explicit() {
        let search = Search {
            name: "lookup".into(),
            max_uses: Some(1),
        };
        let item = json!({"id":"ws1","status":"completed","action":{"type":"search","query":"q","sources":[]}});
        assert_eq!(
            blocks(&item, Some(&search), 0).unwrap()[1]["content"],
            json!([])
        );
        let limited = blocks(&item, Some(&search), 1).unwrap();
        assert_eq!(limited[1]["content"]["error_code"], "max_uses_exceeded");
        let failed = blocks(&json!({"id":"ws2","status":"failed"}), Some(&search), 0).unwrap();
        let replay = history_call(&failed[0], &failed[1]).unwrap();
        assert_eq!(replay["status"], "failed");
        assert_eq!(replay["action"]["query"], "");
    }
}
