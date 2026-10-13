//! Search wire contracts are independent of the inference protocol. Unknown
//! Chat endpoints stay strict; the router can use its optional-tool fallback.
use super::{web_search, ModelProtocol};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const UNSUPPORTED: &str = "protocol.errors.unsupportedSearchOptions";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ChatSearch {
    #[default]
    Auto,
    OpenAi,
    Mimo,
    Command,
    Selected,
    Disabled,
}

impl ChatSearch {
    pub(crate) fn resolve(self, base: &str, model: &str) -> Option<Self> {
        match self {
            Self::Auto => Self::for_endpoint(base, model),
            Self::OpenAi | Self::Mimo => Some(self),
            Self::Command | Self::Selected | Self::Disabled => None,
        }
    }
    /// A known official endpoint establishes the wire contract, not whether a
    /// particular account/model has enabled the service. Relays may declare it.
    pub(crate) fn for_endpoint(base: &str, model: &str) -> Option<Self> {
        let url = reqwest::Url::parse(base).ok()?;
        match url.host_str()? {
            "api.openai.com" if model == "gpt-5-search-api" => Some(Self::OpenAi),
            "api.xiaomimimo.com" => Some(Self::Mimo),
            _ => None,
        }
    }
}

pub(super) struct Prepared {
    pub value: Value,
    tool: Option<Value>,
    chat: Option<ChatSearch>,
}

impl Prepared {
    pub fn apply(self, body: &mut Value) {
        if let Some(tool) = self.tool {
            if self.chat == Some(ChatSearch::OpenAi) {
                body["web_search_options"] = tool;
                // Chat refuses tool controls without tools; a search-only
                // request (Claude's forced WebSearch call) has none left.
                if body.get("tools").is_none() {
                    let object = body.as_object_mut().unwrap();
                    object.remove("tool_choice");
                    object.remove("parallel_tool_calls");
                }
            } else {
                body.as_object_mut().unwrap().entry("tools").or_insert_with(|| json!([]))
                    .as_array_mut().unwrap().push(tool);
            }
        }
    }
}

pub(super) fn prepare(target: ModelProtocol, value: &Value, chat: Option<ChatSearch>) -> Result<Prepared, String> {
    let mut value = value.clone();
    let tools = value["tools"].as_array().cloned().unwrap_or_default();
    let searches: Vec<_> = tools.iter().filter(|tool| web_search::is_search(tool)).collect();
    let Some(search) = searches.first() else {
        return Ok(Prepared { value, tool: None, chat });
    };
    if searches.len() != 1 { return Err(UNSUPPORTED.into()); }
    if target == ModelProtocol::ChatCompletions && chat.is_none() {
        return Err("protocol.errors.unsupportedHostedTool".into());
    }
    if searches[0].as_object().is_none() { return Err(UNSUPPORTED.into()); }
    if !matches!(search["type"].as_str(), Some("web_search" | "web_search_preview" | "web_search_preview_2025_03_11")) {
        return Err(UNSUPPORTED.into());
    }
    let object = search.as_object().unwrap();
    for key in object.keys() {
        if !matches!(key.as_str(), "type" | "name" | "filters" | "user_location" | "external_web_access"
            | "indexed_web_access" | "search_context_size" | "search_content_types") {
            return Err(UNSUPPORTED.into());
        }
    }
    // Codex defaults to cached/indexed access. Other providers keep no OpenAI
    // index, so by the user's choice these run as the target's live search
    // (text results only); the router labels the request as such.
    let disabled = value["tool_choice"] == "none";
    let forced = web_search::required(&value);
    if target == ModelProtocol::AnthropicMessages && tools.iter()
        .any(|tool| !web_search::is_search(tool) && tool["name"] == "web_search") {
        return Err(UNSUPPORTED.into());
    }
    let allowed = search.get("filters").filter(|filter| !filter.is_null());
    if let Some(filter) = allowed {
        if filter.as_object().is_none_or(|map| map.keys().any(|key| key != "allowed_domains"))
            || filter["allowed_domains"].as_array().is_none_or(|domains| domains.iter().any(|domain| !domain.is_string())) {
            return Err(UNSUPPORTED.into());
        }
    }
    let location = search.get("user_location").filter(|location| !location.is_null());
    if let Some(location) = location {
        if location["type"] != "approximate" || location.as_object().is_none_or(|map| map.iter()
            .any(|(key, value)| !matches!(key.as_str(), "type" | "country" | "city" | "region" | "timezone") || !value.is_string())) {
            return Err(UNSUPPORTED.into());
        }
    }
    let mut tool = match target {
        ModelProtocol::AnthropicMessages => {
            if search.get("search_context_size").is_some() { return Err(UNSUPPORTED.into()); }
            let mut tool = json!({"type":"web_search_20250305","name":"web_search"});
            if let Some(filter) = allowed { tool["allowed_domains"] = filter["allowed_domains"].clone(); }
            if let Some(location) = location { tool["user_location"] = location.clone(); }
            if let Some(max) = value.get("max_tool_calls") {
                // Responses limits built-in calls; client functions are outside
                // this budget. Search is the only hosted tool this codec emits.
                if max.as_u64().is_none_or(|n| n == 0) { return Err(UNSUPPORTED.into()); }
                tool["max_uses"] = max.clone();
            }
            tool
        }
        ModelProtocol::ChatCompletions => {
            if allowed.is_some() { return Err(UNSUPPORTED.into()); }
            // Chat search has no exact call budget. By the user's choice the
            // closest bound is used and the router labels it approximate.
            let budget = match value.get("max_tool_calls") {
                Some(max) => Some(max.as_u64().filter(|max| *max > 0).ok_or(UNSUPPORTED)?),
                None => None,
            };
            match chat.unwrap() {
                ChatSearch::OpenAi => {
                    if disabled { return Err(UNSUPPORTED.into()); }
                    // Search models retrieve once per completion: any budget >= 1 holds.
                    let mut tool = json!({});
                    if let Some(location) = location {
                        let mut approximate = location.clone();
                        approximate.as_object_mut().unwrap().remove("type");
                        tool["user_location"] = json!({"type":"approximate","approximate":approximate});
                    }
                    if let Some(size) = search.get("search_context_size") {
                        if !matches!(size.as_str(), Some("low" | "medium" | "high")) { return Err(UNSUPPORTED.into()); }
                        tool["search_context_size"] = size.clone();
                    }
                    tool
                }
                ChatSearch::Mimo => {
                    if search.get("search_context_size").is_some() { return Err(UNSUPPORTED.into()); }
                    let mut tool = json!({"type":"web_search","force_search":forced});
                    // MiMo caps the keywords (queries) of its search round.
                    if let Some(max) = budget { tool["max_keyword"] = json!(max); }
                    if let Some(location) = location {
                        if location.get("timezone").is_some() { return Err(UNSUPPORTED.into()); }
                        tool["user_location"] = location.clone();
                    }
                    tool
                }
                _ => return Err("protocol.errors.unsupportedHostedTool".into()),
            }
        }
        _ => return Err("protocol.errors.unsupportedTranslation".into()),
    };
    value["tools"] = json!(tools.iter().filter(|tool| !web_search::is_search(tool)).cloned().collect::<Vec<_>>());
    if forced {
        value["tool_choice"] = if target == ModelProtocol::AnthropicMessages {
            json!({"type":"function","name":"web_search"})
        } else { json!("auto") };
    }
    if disabled { tool = Value::Null; }
    Ok(Prepared { value, tool: (!tool.is_null()).then_some(tool), chat })
}

/// Flatten OpenAI, MiMo and Anthropic citations for one message. Ranged
/// citations annotate the returned text. Unranged ones (Claude's search
/// locations) become a single deduplicated source list at the end, because
/// Codex renders text but not annotations.
pub(super) fn citations(text: &mut String, annotations: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let mut ranged = std::collections::HashSet::new();
    let mut listed: Vec<(&str, String)> = Vec::new();
    let original_length = text.chars().count() as u64;
    for annotation in annotations.as_array().into_iter().flatten() {
        if !matches!(annotation["type"].as_str(), Some("url_citation" | "web_search_result_location")) { continue; }
        let source = if annotation["url_citation"].is_object() { &annotation["url_citation"] } else { annotation };
        let Some(url) = web_search::url(&source["url"]) else { continue; };
        let title = source["title"].as_str().map(str::trim).filter(|title| !title.is_empty()).unwrap_or(url);
        match (source["start_index"].as_u64(), source["end_index"].as_u64()) {
            (Some(start),Some(end)) if start < end && end <= original_length => {
                if ranged.insert((url,start,end)) {
                    out.push(json!({"type":"url_citation","url":url,"title":title,"start_index":start,"end_index":end}));
                }
            }
            _ if !listed.iter().any(|(listed, _)| *listed == url) => listed.push((url, title.to_owned())),
            _ => {}
        }
    }
    listed.retain(|(url, _)| !out.iter().any(|annotation| annotation["url"] == *url));
    if !listed.is_empty() {
        text.push_str(if text.is_empty() { "Sources:" } else { "\n\nSources:" });
        for (url, title) in listed {
            let title: String = title.chars().map(|c| if matches!(c, '[' | ']' | '\n' | '\r') { ' ' } else { c }).collect();
            text.push_str("\n- ");
            let start = text.chars().count() as u64;
            text.push_str(&format!("[{}]({url})", title.trim()));
            out.push(json!({"type":"url_citation","url":url,"title":title.trim(),
                "start_index":start,"end_index":text.chars().count() as u64}));
        }
    }
    out
}

/// Clients replay whatever they stored: Codex drops unprefixed IDs, keeps a
/// missing action as absent and unknown actions as `other`. History is only
/// context, so any search item stays readable instead of failing the turn.
pub(super) fn readable_history(item: &Value) -> Result<Value, String> {
    let mut text = String::from("[Previous web search]");
    if let Some(query) = item["action"]["query"].as_str().filter(|query| !query.is_empty()) {
        text.push_str(&format!("\n{query}"));
    }
    for query in item["action"]["queries"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        text.push_str(&format!("\n{query}"));
    }
    if let Some(url) = web_search::url(&item["action"]["url"]) { text.push_str(&format!("\n{url}")); }
    if let Some(pattern) = item["action"]["pattern"].as_str() { text.push_str(&format!("\n{pattern}")); }
    for source in item["action"]["sources"].as_array().into_iter().flatten() {
        if let Some(url) = web_search::url(&source["url"]) {
            text.push_str(&format!("\n{}: {url}", source["title"].as_str().unwrap_or(url)));
        }
    }
    if item["status"] != "completed" { text.push_str("\nSearch did not complete."); }
    Ok(json!({"type":"message","role":"assistant","content":[{"type":"input_text","text":text}]}))
}

/// Chat search endpoints often report citations/usage but no per-query server
/// call IDs. Represent that evidence as one aggregate result; never invent a
/// query. Without a declared native search contract, citations stay citations.
pub(super) fn chat_result(output: &[Value], usage: &Value, id: &str) -> Option<Value> {
    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in output {
        for part in item["content"].as_array().into_iter().flatten() {
            for annotation in part["annotations"].as_array().into_iter().flatten() {
                let Some(url) = web_search::url(&annotation["url"]) else { continue; };
                if seen.insert(url) {
                    sources.push(json!({"type":"url","url":url,"title":annotation["title"]}));
                }
            }
        }
    }
    if sources.is_empty() && usage["web_search_usage"]["tool_usage"].as_u64().unwrap_or(0) == 0 { return None; }
    Some(json!({"type":"web_search_call","id":format!("ws_{id}"),"status":"completed",
        "action":{"type":"search","sources":sources}}))
}
