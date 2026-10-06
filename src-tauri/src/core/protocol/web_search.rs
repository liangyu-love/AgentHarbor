//! Hosted-search declarations, results and conversation replay. A local search
//! executor can represent its completed calls as ordinary Chat call/result pairs.
use crate::core::router::ModelProtocol;
use serde_json::{json, Value};

const INVALID: &str = "protocol.errors.invalidRequest";
const UNSUPPORTED: &str = "protocol.errors.unsupportedSearchOptions";

/// Split locally completed search rounds without moving the post-search answer
/// before its results. A final mixed client/server group stays together so the
/// caller's following tool results can still close that same Chat call group.
pub(crate) fn local_search_history(
    messages: &[Value],
    name: &str,
    visible_name: &str,
) -> Result<Vec<Value>, String> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for message in messages {
        let Some(blocks) = message["content"]
            .as_array()
            .filter(|_| message["role"] == "assistant")
        else {
            normalized.push(message.clone());
            continue;
        };
        let mut pending = std::collections::HashMap::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("server_tool_use" | "tool_use") => {
                    let id = block["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or(INVALID)?;
                    if !seen.insert(id) {
                        return Err(INVALID.into());
                    }
                    if block["type"] == "server_tool_use" {
                        if block["name"] != visible_name {
                            return Err(UNSUPPORTED.into());
                        }
                        if !block["input"].is_object() {
                            return Err(INVALID.into());
                        }
                        pending.insert(id, block);
                    }
                }
                Some("web_search_tool_result") => {
                    let id = block["tool_use_id"].as_str().ok_or(INVALID)?;
                    let call = pending.remove(id).ok_or(INVALID)?;
                    history_call(call, block)?;
                }
                _ => {}
            }
        }
        if !pending.is_empty() {
            return Err(INVALID.into());
        }
        if !blocks
            .iter()
            .any(|block| block["type"] == "server_tool_use")
        {
            normalized.push(message.clone());
            continue;
        }
        let mut content = Vec::new();
        let mut results = Vec::new();
        let mut unanswered = std::collections::HashSet::new();
        let mut client_pending = false;
        for block in blocks {
            if !client_pending
                && unanswered.is_empty()
                && block["type"] != "web_search_tool_result"
                && !results.is_empty()
            {
                push_local_round(&mut normalized, message, &mut content, &mut results);
            }
            match block["type"].as_str() {
                Some("server_tool_use") => {
                    unanswered.insert(block["id"].as_str().unwrap());
                    content.push(json!({"type":"tool_use",
                        "id":block["id"],"name":name,"input":block["input"]}));
                }
                Some("web_search_tool_result") => {
                    unanswered.remove(block["tool_use_id"].as_str().unwrap());
                    results.push(json!({"type":"tool_result",
                        "tool_use_id":block["tool_use_id"],"content":local_result(block).to_string()}));
                }
                Some("tool_use") => {
                    client_pending = true;
                    content.push(block.clone());
                }
                _ => content.push(block.clone()),
            }
        }
        push_local_round(&mut normalized, message, &mut content, &mut results);
    }
    Ok(normalized)
}

fn local_result(block: &Value) -> Value {
    if let Some(results) = block["content"].as_array() {
        let results: Vec<_> = results
            .iter()
            .filter_map(|result| {
                url(&result["url"]).map(|url| {
                    json!({"url":url,
                "title":result["title"].as_str().unwrap_or(url)})
                })
            })
            .collect();
        json!({"results":results})
    } else {
        json!({"error":block["content"]["error_code"]})
    }
}

fn push_local_round(
    normalized: &mut Vec<Value>,
    original: &Value,
    content: &mut Vec<Value>,
    results: &mut Vec<Value>,
) {
    if !content.is_empty() {
        let mut assistant = original.clone();
        assistant["content"] = json!(std::mem::take(content));
        normalized.push(assistant);
    }
    if !results.is_empty() {
        normalized.push(json!({"role":"user","content":std::mem::take(results)}));
    }
}

pub(crate) fn options_from_tools(
    target: ModelProtocol,
    value: &Value,
) -> Result<super::ResponseOptions, String> {
    let mut options = super::ResponseOptions::default();
    if let Some(tools) = value.get("tools") {
        for tool in tools.as_array().ok_or(INVALID)?.iter().filter(|tool| is_search(tool)) {
            if options.search.is_some() { return Err(UNSUPPORTED.into()); }
            options.search = Some(prepare(target, tool)?.0);
        }
    }
    Ok(options)
}

#[derive(Clone, Debug)]
pub struct Search {
    pub name: String,
    pub max_uses: Option<u64>,
}

pub(crate) fn is_search(tool: &Value) -> bool {
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
        Some("open_page") => {
            json!({"url":url(&action["url"]).ok_or("protocol.errors.invalidResponse")?})
        }
        Some("find_in_page") => {
            json!({"url":url(&action["url"]).ok_or("protocol.errors.invalidResponse")?,"pattern":action["pattern"].as_str().ok_or("protocol.errors.invalidResponse")?})
        }
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

    fn local_call(id: &str) -> Value {
        json!({"type":"server_tool_use","id":id,"name":"web_search","input":{"query":id}})
    }
    fn local_result_block(id: &str) -> Value {
        json!({"type":"web_search_tool_result","tool_use_id":id,"content":[
            {"type":"web_search_result","url":"https://example.org/source","title":"Real source","encrypted_content":"opaque"},
            {"type":"web_search_result","url":"javascript:alert(1)","title":"unsafe"}
        ]})
    }

    #[test]
    fn local_search_cache_miss_preserves_round_order_and_real_source_metadata() {
        let source = json!({"role":"assistant","content":[
            {"type":"text","text":"Searching"},local_call("s1"),local_result_block("s1"),
            {"type":"text","text":"Checking another source"},local_call("s2"),local_result_block("s2"),
            {"type":"text","text":"Final answer"}
        ]});
        let normalized = local_search_history(&[source], "internal_search", "web_search").unwrap();
        let chat = super::super::anthropic::request_body(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","max_tokens":100,"messages":normalized}),
        )
        .unwrap();
        super::super::chat::validate_tool_history(&chat).unwrap();
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0]["content"], "Searching");
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["name"],
            "internal_search"
        );
        let result: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            result,
            json!({"results":[{"url":"https://example.org/source","title":"Real source"}]})
        );
        assert_eq!(messages[2]["content"], "Checking another source");
        assert_eq!(messages[3]["tool_call_id"], "s2");
        assert_eq!(messages[4]["content"], "Final answer");
    }

    #[test]
    fn local_search_mixed_client_calls_keep_contiguous_results() {
        let assistant = json!({"role":"assistant","content":[
            {"type":"tool_use","id":"client","name":"read_file","input":{"path":"a"}},
            local_call("search"),local_result_block("search")
        ]});
        let user = json!({"role":"user","content":[
            {"type":"text","text":"Continue after reading"},
            {"type":"tool_result","tool_use_id":"client","content":"file contents"}
        ]});
        let normalized =
            local_search_history(&[assistant, user], "internal_search", "web_search").unwrap();
        let chat = super::super::anthropic::request_body(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","max_tokens":100,"messages":normalized}),
        )
        .unwrap();
        super::super::chat::validate_tool_history(&chat).unwrap();
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[1]["tool_call_id"], "search");
        assert_eq!(messages[2]["tool_call_id"], "client");
        assert_eq!(messages[3]["content"], "Continue after reading");
    }

    #[test]
    fn local_search_rejects_duplicate_ids_or_unpaired_results() {
        for blocks in [
            vec![
                local_call("s"),
                local_result_block("s"),
                local_call("s"),
                local_result_block("s"),
            ],
            vec![
                local_call("s"),
                local_result_block("s"),
                local_result_block("s"),
            ],
            vec![local_result_block("s"), local_call("s")],
            vec![local_call("s")],
            vec![local_call("s"), local_result_block("other")],
            vec![
                json!({"type":"tool_use","id":"s","name":"read","input":{}}),
                local_call("s"),
                local_result_block("s"),
            ],
        ] {
            assert!(local_search_history(
                &[json!({"role":"assistant","content":blocks})],
                "internal_search",
                "web_search"
            )
            .is_err());
        }
        let assistant =
            json!({"role":"assistant","content":[local_call("s"),local_result_block("s")]});
        assert!(local_search_history(
            &[assistant.clone(), assistant],
            "internal_search",
            "web_search"
        )
        .is_err());
    }

    #[test]
    fn parallel_local_results_are_not_split_before_the_group_completes() {
        let assistant = json!({"role":"assistant","content":[
            local_call("a"),local_call("b"),local_result_block("a"),
            {"type":"text","text":"Checking"},local_result_block("b"),
            {"type":"text","text":"Done"}
        ]});
        let normalized =
            local_search_history(&[assistant], "internal_search", "web_search").unwrap();
        let chat = super::super::anthropic::request_body(
            ModelProtocol::ChatCompletions,
            &json!({"model":"m","max_tokens":100,"messages":normalized}),
        )
        .unwrap();
        super::super::chat::validate_tool_history(&chat).unwrap();
        assert_eq!(
            chat["messages"][0]["tool_calls"].as_array().unwrap().len(),
            2
        );
        assert_eq!(chat["messages"][1]["tool_call_id"], "a");
        assert_eq!(chat["messages"][2]["tool_call_id"], "b");
        assert_eq!(chat["messages"][3]["content"], "Done");
    }

    #[test]
    fn search_navigation_actions_reject_unsafe_urls() {
        for action in ["open_page", "find_in_page"] {
            for url in ["javascript:alert(1)", "file:///secret", "https://example.org/a\n"] {
                assert!(blocks(&json!({"id":"ws","status":"completed","action":{"type":action,"url":url,"pattern":"x"}}), None, 0).is_err());
            }
            assert!(blocks(&json!({"id":"ws","status":"completed","action":{"type":action,"url":"https://example.org","pattern":"x"}}), None, 0).is_ok());
        }
    }

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
