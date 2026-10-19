//! Execute Messages hosted search for any Chat model using an independent
//! search service. Model protocol, model credentials and network mode stay intact.
use super::*;
use std::collections::HashSet;
mod native;
pub(super) use native::Selection;

const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_ROUNDS: usize = 32;
const COMMAND_SEARCH_ENDPOINT: &str = "https://api.commandcode.ai/alpha/web-search";
const DEEPSEEK_SEARCH_ENDPOINT: &str = "https://api.deepseek.com/v1/tools/search";
/// DeepSeek refuses longer queries.
const DEEPSEEK_QUERY_CHARS: usize = 128;
#[cfg(test)]
pub(super) static TEST_SEARCH_ENDPOINT: Mutex<Option<String>> = Mutex::new(None);
const INVALID: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "invalid WebSearch request");
const INVALID_RESPONSE: (StatusCode, &str) = (
    StatusCode::BAD_GATEWAY,
    "invalid model response during WebSearch",
);

pub(super) struct Plan {
    pub normalized: Bytes,
    caller: router::ModelProtocol,
    name: String,
    allowed_domains: Vec<String>,
    blocked_domains: Vec<String>,
    /// The tool's domain filters fit a REST service's host-only filtering.
    host_filters: bool,
    options: protocol::ResponseOptions,
    streaming: bool,
    max_tokens: u64,
    /// Search-only request in the caller's native contract; never the conversation.
    native_request: Option<Value>,
}

impl Plan {
    pub fn is_native(&self) -> bool {self.native_request.is_some()}
    pub fn host_filters(&self) -> bool {self.host_filters}
    pub fn required(&self) -> bool {self.options.search_required}
    pub fn native_reply_misclassified(&self, target: router::ModelProtocol, bytes: &[u8], sse: bool) -> bool {
        let Ok(reply) = native::reply(target,bytes,sse) else {return false};
        let Ok(reply) = native::canonical(target,&reply) else {return false};
        if reply["status"] != "completed" {return false}
        let Some(output) = reply["output"].as_array() else {return false};
        // Never repeat a search already performed by the provider, even if it
        // also emitted a malformed ordinary call in the same response.
        !output.iter().any(|item| item["type"] == "web_search_call")
            && output.iter().any(|item| item["type"] == "function_call"
                && self.options.search.as_ref().is_some_and(|search| item["name"] == search.name))
    }
    pub fn command_enabled(provider: &router::Provider, store: &router::Store) -> bool {
        provider.chat_search == protocol::ChatSearch::Command || (router::supports_command_search(provider)
            && store.router.web_search_provider_id.as_deref().is_some_and(|id| store.providers.iter().any(|selected| selected.id == id && router::supports_command_search(selected))))
    }
    /// Command Code exposes search through /alpha/web-search, not as a hosted
    /// tool on the configured native endpoint. Explicit search selections can
    /// use the existing executor when this model also has a verified Chat
    /// endpoint. Fresh ordinary turns keep the configured protocol.
    pub fn command_responses_search_to_chat(
        destination: &mut Destination,
        body: Option<&[u8]>,
        enabled: bool,
    ) -> bool {
        if !enabled { return false; }
        #[derive(serde::Deserialize)]
        struct DeclaredTools {
            #[serde(default)]
            tools: Vec<Value>,
        }
        let Destination::Provider { provider, model_override, upstream_protocol, translation } = destination else {
            return false;
        };
        let caller = translation.as_ref().map_or(*upstream_protocol, |pair| pair.from);
        if !matches!(*upstream_protocol,router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages)
            || !matches!(caller, router::ModelProtocol::AnthropicMessages | router::ModelProtocol::Responses)
            || !router::supports_command_search(provider)
            || !provider.supports_model_protocol(
                model_override.as_deref().unwrap_or(&provider.model),
                router::ModelProtocol::ChatCompletions,
            )
        {
            return false;
        }
        // Ignore the potentially large message history instead of materializing
        // a second full JSON tree just to inspect tool declarations.
        let has_search = body.and_then(|bytes| serde_json::from_slice::<DeclaredTools>(bytes).ok())
            .is_some_and(|value| value.tools.iter().any(protocol::web_search::is_search));
        if !has_search { return false; }
        *upstream_protocol = router::ModelProtocol::ChatCompletions;
        *translation = protocol::Translation::between(
            caller,
            router::ModelProtocol::ChatCompletions,
        );
        true
    }

    /// A later turn can disable search while retaining its prior result blocks.
    /// Replaying those blocks must not require a search account or enable a tool.
    pub fn history_only(
        destination: &Destination,
        body: Option<Bytes>,
    ) -> ForwardResult<Option<Bytes>> {
        let Destination::Provider {
            translation: Some(pair),
            ..
        } = destination
        else {
            return Ok(body);
        };
        if pair.from != router::ModelProtocol::AnthropicMessages
            || pair.to != router::ModelProtocol::ChatCompletions
        {
            return Ok(body);
        }
        let Some(bytes) = &body else { return Ok(body) };
        let mut value: Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        let visible_name = value["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["role"] == "assistant")
            .flat_map(|m| m["content"].as_array().into_iter().flatten())
            .find(|b| b["type"] == "server_tool_use")
            .and_then(|b| b["name"].as_str())
            .map(str::to_owned);
        let Some(visible_name) = visible_name else {
            return Ok(body);
        };
        let name = internal_name(value["tools"].as_array().map(Vec::as_slice).unwrap_or(&[]));
        value["messages"] = json!(protocol::web_search::local_search_history(
            value["messages"].as_array().ok_or(INVALID)?,
            &name,
            &visible_name
        )
        .map_err(|_| INVALID)?);
        Ok(Some(Bytes::from(value.to_string())))
    }

    pub fn for_destination(
        destination: &Destination,
        body: Option<&[u8]>,
    ) -> ForwardResult<Option<Self>> {
        let Destination::Provider {
            provider,
            translation: Some(pair),
            ..
        } = destination
        else {
            return Ok(None);
        };
        if provider.chat_search != protocol::ChatSearch::Command && !router::supports_command_search(provider) {
            return Ok(None);
        }
        if !matches!(pair.from, router::ModelProtocol::AnthropicMessages | router::ModelProtocol::Responses)
            || pair.to != router::ModelProtocol::ChatCompletions
        {
            return Ok(None);
        }
        let Some(body) = body else { return Ok(None) };
        let value: Value = serde_json::from_slice(body).map_err(|_| INVALID)?;
        if value["tool_choice"] == "none" || value["tool_choice"]["type"] == "none" { return Ok(None); }
        if !value["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(protocol::web_search::is_search))
        {
            return Ok(None);
        }
        if pair.from == router::ModelProtocol::Responses {
            Self::prepare_responses(value).map(Some)
        } else {
            Self::prepare(value).map(Some)
        }
    }

    fn prepare_responses(mut value: Value) -> ForwardResult<Self> {
        let tools = value["tools"].as_array().ok_or(INVALID)?;
        let search = tools.iter().find(|tool| protocol::web_search::is_search(tool)).ok_or(INVALID)?.clone();
        let mut probe = value.clone();
        probe["input"] = json!("search");
        probe.as_object_mut().ok_or(INVALID)?.remove("previous_response_id");
        probe["tools"] = json!([search]);
        probe["tool_choice"] = json!("auto");
        let native = protocol::request_body(protocol::Translation {
            from:router::ModelProtocol::Responses, to:router::ModelProtocol::AnthropicMessages,
        }, &probe).map_err(|_| (StatusCode::BAD_REQUEST, "Command search cannot preserve these WebSearch options"))?.body;
        let mut plan = Self::prepare(native)?;
        plan.name = internal_name(tools);
        for tool in value["tools"].as_array_mut().ok_or(INVALID)? {
            if protocol::web_search::is_search(tool) {
                *tool = json!({"type":"function","name":plan.name,
                    "description":"Search the web for current information and cite the returned source URLs.",
                    "parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}});
            }
        }
        if value["tool_choice"]["type"].as_str().is_some_and(|kind| kind.starts_with("web_search")) {
            value["tool_choice"] = json!({"type":"function","name":plan.name});
        }
        value["stream"] = json!(false);
        plan.normalized = Bytes::from(value.to_string());
        plan.caller = router::ModelProtocol::Responses;
        Ok(plan)
    }

    pub(super) fn prepare(mut value: Value) -> ForwardResult<Self> {
        let options =
            protocol::web_search::options_from_tools(router::ModelProtocol::Responses, &value)
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Command search cannot preserve these WebSearch options",
                    )
                })?;
        let search = options.search.as_ref().ok_or(INVALID)?;
        // Location is meaningful in the native search API but absent from Command's REST contract.
        let tools = value["tools"].as_array().ok_or(INVALID)?;
        let tool = tools
            .iter()
            .find(|tool| protocol::web_search::is_search(tool))
            .ok_or(INVALID)?;
        if tool.get("user_location").is_some_and(|v| !v.is_null()) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Command search does not support user_location",
            ));
        }
        let allowed_domains = domains(&tool["allowed_domains"])?;
        let name = internal_name(tools);
        let streaming = value["stream"] == true;
        let max_tokens = value["max_tokens"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or(INVALID)?;
        let messages = value["messages"].as_array().ok_or(INVALID)?;
        value["messages"] =
            json!(
                protocol::web_search::local_search_history(messages, &name, &search.name)
                    .map_err(|_| INVALID)?
            );
        for tool in value["tools"].as_array_mut().ok_or(INVALID)? {
            if protocol::web_search::is_search(tool) {
                *tool = json!({"name":name,"description":"Search the web for current information. Results contain real source URLs, titles and snippets; cite the source URLs in your answer.",
                    "input_schema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}});
            }
        }
        if value["tool_choice"]["type"] == "tool" && value["tool_choice"]["name"] == search.name {
            value["tool_choice"]["name"] = json!(name);
        }
        value["stream"] = json!(false);
        Ok(Self {
            normalized: Bytes::from(value.to_string()),
            caller: router::ModelProtocol::AnthropicMessages,
            name,
            allowed_domains,
            blocked_domains: Vec::new(),
            host_filters: true,
            options,
            streaming,
            max_tokens,
            native_request: None,
        })
    }

    pub fn selected(store: &router::Store) -> bool {
        store.router.web_search_provider_id.as_deref().is_some_and(|id| store.router.web_search_model_id.is_some() || store.providers.iter()
            .any(|provider| provider.id == id && !router::supports_command_search(provider)))
    }

    /// Whether a selected native service is needed before sending this request.
    /// A representable native contract gets the first attempt in automatic mode.
    pub fn delegated(destination: &Destination, store: &router::Store, body: Option<&[u8]>) -> ForwardResult<Option<Self>> {
        let Destination::Provider {provider,upstream_protocol,translation,..} = destination else {return Ok(None)};
        if matches!(provider.chat_search,protocol::ChatSearch::Disabled | protocol::ChatSearch::Command) || !Self::selected(store) {return Ok(None)}
        let caller = translation.map_or(*upstream_protocol, |pair| pair.from);
        let Some(bytes) = body else {return Ok(None)};
        let original: Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        if original["tool_choice"] == "none" || original["tool_choice"]["type"] == "none"
            || !original["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {return Ok(None)}
        let representable = if translation.is_some() {
            upstream_body(destination,Some(Bytes::copy_from_slice(bytes))).is_ok_and(|prepared| !prepared.response_options.search_downgraded)
        } else {true};
        if provider.chat_search == protocol::ChatSearch::Selected || !representable {Self::native(caller,body)} else {Ok(None)}
    }

    pub fn native(caller: router::ModelProtocol, body: Option<&[u8]>) -> ForwardResult<Option<Self>> {
        if !matches!(caller, router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages) {return Ok(None)}
        let Some(bytes) = body else {return Ok(None)};
        let mut value: Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        if value["tool_choice"] == "none" || value["tool_choice"]["type"] == "none" {return Ok(None)}
        let tools = value["tools"].as_array().cloned().unwrap_or_default();
        let searches: Vec<_> = tools.iter().filter(|tool| protocol::web_search::is_search(tool)).cloned().collect();
        if searches.is_empty() {return Ok(None)}
        if searches.len() != 1 {return Err(INVALID)}
        // A single budget cannot be split safely across other hosted tools.
        if caller == router::ModelProtocol::Responses && value.get("max_tool_calls").is_some()
            && tools.iter().any(|tool| !protocol::web_search::is_search(tool) && !matches!(tool["type"].as_str(),Some("function"|"custom"|"namespace"))) {
            return Err((StatusCode::BAD_REQUEST,"selected search cannot split a shared hosted-tool budget"));
        }
        let streaming = value["stream"] == true;
        let name = internal_name(&tools);
        // A model service gets these in its own request; a REST service filters with them.
        let filters = domains(if caller == router::ModelProtocol::Responses {&searches[0]["filters"]["allowed_domains"]} else {&searches[0]["allowed_domains"]})
            .and_then(|allowed| Ok((allowed,domains(&searches[0]["blocked_domains"])?)));
        let host_filters = filters.is_ok();
        let (allowed_domains,blocked_domains) = filters.unwrap_or_default();
        let mut probe = if caller == router::ModelProtocol::Responses {
            json!({"model":"search","input":"search","tools":searches,"tool_choice":"auto"})
        } else {json!({"model":"search","max_tokens":4096,"messages":[{"role":"user","content":"search"}],"tools":searches,"tool_choice":{"type":"auto"}})};
        let options = if caller == router::ModelProtocol::Responses {
            if let Some(budget) = value.get("max_tool_calls") {probe["max_tool_calls"] = budget.clone();}
            protocol::web_search::Search {name:"web_search".into(),max_uses:search_limit(value.get("max_tool_calls"))?}
        } else {
            let tool = &probe["tools"][0];
            if tool.get("allowed_callers").is_some_and(|callers| *callers != json!(["direct"]))
                || (tool["type"] != "web_search_20250305" && tool.get("allowed_callers").is_none()
                    && protocol::web_search::programmatic_callers(&tools)) {
                return Err((StatusCode::BAD_REQUEST,"selected search requires direct hosted calls"));
            }
            let visible = tool["name"].as_str().filter(|name| !name.is_empty() && name.len() <= 64 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte,b'_'|b'-'))).ok_or(INVALID)?;
            if tools.iter().any(|tool| !protocol::web_search::is_search(tool) && tool["name"] == visible) {return Err(INVALID)}
            protocol::web_search::Search {name:visible.into(),max_uses:search_limit(tool.get("max_uses"))?}
        };
        let options = protocol::ResponseOptions {search:Some(options),search_required:protocol::web_search::required(&value),..Default::default()};
        if caller == router::ModelProtocol::AnthropicMessages {
            value["messages"] = json!(protocol::web_search::local_search_history(value["messages"].as_array().ok_or(INVALID)?,&name,
                &options.search.as_ref().ok_or(INVALID)?.name).map_err(|_| INVALID)?);
        }
        let max_tokens = value[if caller == router::ModelProtocol::Responses {"max_output_tokens"} else {"max_tokens"}]
            .as_u64().filter(|value| *value > 0).unwrap_or(8192);
        for tool in value["tools"].as_array_mut().ok_or(INVALID)? {
            if protocol::web_search::is_search(tool) {
                *tool = if caller == router::ModelProtocol::Responses {
                    json!({"type":"function","name":name,"description":"Search the web and cite real source URLs returned by this tool.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}})
                } else {json!({"name":name,"description":"Search the web and cite real source URLs returned by this tool.","input_schema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}})};
            }
        }
        if let Some(choice) = value.get_mut("tool_choice") {
            if choice["type"].as_str().is_some_and(|kind| kind.starts_with("web_search")) {
                *choice = json!({"type":"function","name":name});
            } else if choice["type"] == "tool" && choice["name"] == options.search.as_ref().ok_or(INVALID)?.name {
                choice["name"] = json!(name);
            } else if choice["type"] == "allowed_tools" {
                for tool in choice["tools"].as_array_mut().ok_or(INVALID)? {
                    if protocol::web_search::is_search(tool) {*tool=json!({"type":"function","name":name});}
                }
            }
        }
        if caller == router::ModelProtocol::Responses {value.as_object_mut().ok_or(INVALID)?.remove("max_tool_calls");}
        value["stream"] = json!(false);
        Ok(Some(Self {normalized:Bytes::from(value.to_string()),caller,name,allowed_domains,blocked_domains,host_filters,options,streaming,max_tokens,native_request:Some(probe)}))
    }
}

fn endpoint(default: &str) -> String {
    #[cfg(test)]
    if let Some(endpoint) = TEST_SEARCH_ENDPOINT.lock().unwrap().clone() {
        return endpoint;
    }
    default.into()
}

fn internal_name(tools: &[Value]) -> String {
    let mut name = "__agentharbor_web_search".to_owned();
    let mut suffix = 0_u64;
    while tools.iter().any(|tool| tool["name"] == name) {
        suffix += 1;
        name = format!("__agentharbor_web_search_{suffix}");
    }
    name
}

/// Bare host names; a scheme, path, port or credentials make the filter invalid.
fn domains(values: &Value) -> ForwardResult<Vec<String>> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .map(|domain| {
            let domain = domain.as_str().ok_or(INVALID)?.trim_start_matches('.');
            let parsed =
                reqwest::Url::parse(&format!("https://{domain}")).map_err(|_| INVALID)?;
            if parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.port().is_some()
                || domain.contains(['/', ':', '?', '#', '@'])
            {
                return Err(INVALID);
            }
            parsed
                .host_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_end_matches('.').to_owned())
                .ok_or(INVALID)
        })
        .collect()
}

fn on_domain(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'))
}

/// The model's query with site operators for the domain filters. Operators
/// that do not fit are left out; results are filtered locally either way.
fn deepseek_query(query: &str, allowed: &[String], blocked: &[String]) -> String {
    let mut operators = String::new();
    let single = match allowed {
        [domain] => Some(format!("site:{domain}")),
        _ => None,
    };
    for operator in single.into_iter().chain(blocked.iter().map(|domain| format!("-site:{domain}"))) {
        if operators.chars().count() + operator.chars().count() < DEEPSEEK_QUERY_CHARS / 2 {
            operators.push(' ');
            operators.push_str(&operator);
        }
    }
    let room = DEEPSEEK_QUERY_CHARS - operators.chars().count();
    let query: String = query.trim().chars().take(room).collect();
    format!("{}{operators}", query.trim_end())
}

fn search_limit(value: Option<&Value>) -> ForwardResult<Option<u64>> {
    value.filter(|value| !value.is_null()).map(|value| value.as_u64().filter(|limit| *limit > 0).ok_or(INVALID)).transpose()
}

/// Backend selection is separate from the model's protocol. Only the adapter
/// identifies a brand; the Chat executor itself accepts every Chat provider.
pub(super) struct SearchBackend {
    pub client: Client,
    pub endpoint: String,
    pub headers: HeaderMap,
    pub(super) native: Option<native::Backend>,
    /// The REST contract when there is no native backend; Command's by default.
    pub api: Option<router::SearchApi>,
}

impl SearchBackend {
    pub fn select(
        context: &ArcContext,
        store: &router::Store,
        destination: &Destination,
    ) -> ForwardResult<Self> {
        let selected = Self::provider(store, destination)?;
        let search_destination = Destination::Provider {
            provider: Box::new(selected.clone()),
            model_override: None,
            upstream_protocol: router::ModelProtocol::ChatCompletions,
            translation: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", selected.model_credential().api_key))
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "invalid search service key",
                    )
                })?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers.insert("x-command-code-version", HeaderValue::from_static("1.73.0"));
        headers.insert("x-cli-environment", HeaderValue::from_static("production"));
        Ok(Self {
            client: client_for(context, &search_destination)?,
            endpoint: endpoint(COMMAND_SEARCH_ENDPOINT),
            headers,
            native: None,
            api: Some(router::SearchApi::Command),
        })
    }

    /// DeepSeek's Search API on the selected card's key and network mode.
    pub fn deepseek(context: &ArcContext, provider: &router::Provider) -> ForwardResult<Self> {
        let destination = Destination::Provider {
            provider: Box::new(provider.clone()),
            model_override: None,
            upstream_protocol: router::ModelProtocol::ChatCompletions,
            translation: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", provider.model_credential().api_key.trim()))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid search service key"))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        Ok(Self {
            client: client_for(context, &destination)?,
            endpoint: endpoint(DEEPSEEK_SEARCH_ENDPOINT),
            headers,
            native: None,
            api: Some(router::SearchApi::DeepSeek),
        })
    }

    fn provider<'a>(
        store: &'a router::Store,
        destination: &'a Destination,
    ) -> ForwardResult<&'a router::Provider> {
        let selected_command = store.router.web_search_provider_id.as_ref().filter(|_| !Plan::selected(store));
        let selected = if let Some(id) = selected_command {
            store
                .providers
                .iter()
                .find(|p| &p.id == id && router::supports_command_search(p))
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    "configured WebSearch service is unavailable",
                ))?
        } else if let Destination::Provider { provider, .. } = destination {
            if router::supports_command_search(provider) {
                provider.as_ref()
            } else if provider.chat_search == protocol::ChatSearch::Command {
                Self::unique(store)?
            } else {
                return Err((StatusCode::BAD_REQUEST, "Command search was not selected for this model"));
            }
        } else {
            Self::unique(store)?
        };
        Ok(selected)
    }

    fn unique(store: &router::Store) -> ForwardResult<&router::Provider> {
        let mut candidates = store
            .providers
            .iter()
            .filter(|p| router::supports_command_search(p));
        let first = candidates.next().ok_or((
            StatusCode::BAD_REQUEST,
            "Chat WebSearch requires a configured search service",
        ))?;
        if candidates.next().is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                "select a WebSearch search service in routing settings",
            ));
        }
        Ok(first)
    }
}

pub(super) struct Execution {
    pub plan: Plan,
    pub backend: SearchBackend,
    pub client: Client,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub trace: route_trace::Trace,
    pub usage: Option<relay_usage::RelayUsageRequest>,
    pub attempt: route_trace::AttemptMetadata,
    pub model: String,
    pub history: Option<protocol::message_history::Context>,
    pub input: Vec<Value>,
    pub namespaces: protocol::tool_namespaces::ToolNamespaces,
    pub adapter: tool_schema::Adapter,
    pub credential: request_diagnostics::CredentialSnapshot,
    pub health: Option<super::super::aggregation::runtime::Health>,
    pub protocol: router::ModelProtocol,
    pub account: Option<account_sources::RequestAuth>,
    pub selection: Option<Selection>,
}

pub(super) async fn relay(execution: Execution) -> ForwardResult {
    let streaming = execution.plan.streaming;
    let caller = execution.plan.caller;
    let options = execution.plan.options.clone();
    let trace = execution.trace.clone();
    let future = async move {
        match tokio::time::timeout(Duration::from_secs(900), Box::pin(execution.run())).await {
            Ok(result) => result,
            Err(_) => Err((StatusCode::GATEWAY_TIMEOUT, "WebSearch request timed out")),
        }
    };
    if !streaming {
        let canonical = future.await?;
        let visible = visible(&canonical, &options, caller)?;
        trace.finish();
        return Ok(Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(full_body(visible.to_string()))
            .unwrap());
    }
    let stream = futures_util::stream::once(async move {
        Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(
            if caller == router::ModelProtocol::Responses { b": ping\n\n" } else { b"event: ping\ndata: {}\n\n" },
        )))
    })
    .chain(futures_util::stream::once(async move {
        let output = match future.await {
            Ok(canonical) => {
                if caller == router::ModelProtocol::Responses {
                    trace.finish();
                    return Ok(Frame::data(Bytes::from(responses_stream(&canonical))));
                }
                let mut translator = protocol::stream_translator_with_options(
                    protocol::Translation {
                        from: router::ModelProtocol::AnthropicMessages,
                        to: router::ModelProtocol::Responses,
                    },
                    options,
                );
                let kind = if canonical["status"] == "incomplete" {
                    "response.incomplete"
                } else {
                    "response.completed"
                };
                let frame = format!(
                    "event: {kind}\ndata: {}\n\n",
                    json!({"type":kind,"response":canonical})
                );
                let output = translator.push(frame.as_bytes()).concat();
                if translator.failure().is_some() {
                    trace.fail("translation", "invalid_upstream_response");
                } else {
                    trace.finish();
                }
                output
            }
            Err((status, message)) => {
                trace.reject(status.as_u16(), message);
                if caller == router::ModelProtocol::Responses {
                    return Ok(Frame::data(Bytes::from(responses_stream(&json!({"id":format!("resp_{}",uuid::Uuid::new_v4().simple()),
                        "object":"response","status":"failed","output":[],"error":{"code":if status.is_client_error() {"invalid_prompt"} else {"server_error"},"message":message}})))));
                }
                format!(
                    "event: error\ndata: {}\n\n",
                    json!({"type":"error","error":{"type":match status.as_u16() {
                        400 | 404 | 413 | 422 => "invalid_request_error",
                        401 => "authentication_error", 403 => "permission_error",
                        429 => "rate_limit_error", _ => "api_error",
                    },"message":message}})
                )
            }
        };
        Ok(Frame::data(Bytes::from(output)))
    }));
    let stream = sse_keepalive(stream, Duration::from_secs(15))
        .map(|item| item.map_err(|never| match never {}));
    Ok(Response::builder()
        .header(CONTENT_TYPE, "text/event-stream")
        .body(BodyExt::boxed(StreamBody::new(stream)))
        .unwrap())
}

fn visible(canonical: &Value, options: &protocol::ResponseOptions, caller: router::ModelProtocol) -> ForwardResult<Value> {
    if caller == router::ModelProtocol::Responses { return Ok(canonical.clone()); }
    protocol::response_body_with_options(
        protocol::Translation {
            from: router::ModelProtocol::AnthropicMessages,
            to: router::ModelProtocol::Responses,
        },
        canonical,
        options,
    )
    .map_err(|_| INVALID_RESPONSE)
}

fn responses_stream(response: &Value) -> String {
    let mut frames = String::new();
    let mut frame = |kind: &str, mut value: Value| {
        value["type"] = json!(kind);
        frames.push_str(&format!("event: {kind}\ndata: {value}\n\n"));
    };
    frame("response.created", json!({"response":{"id":response["id"],"status":"in_progress","model":response["model"]}}));
    for (index, item) in response["output"].as_array().into_iter().flatten().enumerate() {
        let mut opened = item.clone();
        opened["status"] = json!("in_progress");
        if opened["type"] == "function_call" { opened["arguments"] = json!(""); }
        if opened["type"] == "message" { opened["content"] = json!([]); }
        frame("response.output_item.added", json!({"output_index":index,"item":opened}));
        if item["type"] == "message" {
            for (part_index, part) in item["content"].as_array().into_iter().flatten().enumerate() {
                if part["type"] != "output_text" { continue; }
                frame("response.content_part.added", json!({"item_id":item["id"],"output_index":index,"content_index":part_index,"part":{"type":"output_text","text":"","annotations":[]}}));
                frame("response.output_text.delta", json!({"item_id":item["id"],"output_index":index,"content_index":part_index,"delta":part["text"]}));
                frame("response.output_text.done", json!({"item_id":item["id"],"output_index":index,"content_index":part_index,"text":part["text"]}));
                frame("response.content_part.done", json!({"item_id":item["id"],"output_index":index,"content_index":part_index,"part":part}));
            }
        } else if item["type"] == "function_call" {
            frame("response.function_call_arguments.delta", json!({"item_id":item["id"],"output_index":index,"delta":item["arguments"]}));
            frame("response.function_call_arguments.done", json!({"item_id":item["id"],"output_index":index,"arguments":item["arguments"]}));
        }
        frame("response.output_item.done", json!({"output_index":index,"item":item}));
    }
    frame(match response["status"].as_str() { Some("incomplete") => "response.incomplete", Some("failed") => "response.failed", _ => "response.completed" }, json!({"response":response}));
    frames
}

pub(in crate::core::router_proxy) async fn limited(response: reqwest::Response) -> ForwardResult<Bytes> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| (StatusCode::BAD_GATEWAY, "cannot read WebSearch response"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err((
                StatusCode::BAD_GATEWAY,
                "response exceeded the WebSearch buffer limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(bytes))
}

/// Search behavior a target has shown in an explicit reply. It only skips a
/// request already known to fail the same way; it never claims search works.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Learned {
    /// The provider refused the optional native search declaration.
    RejectsSearch,
    /// Hosted search works alone, not mixed with client functions.
    SeparateSearch,
}

/// Long enough to stop repeating a failing first attempt every turn, short
/// enough that enabling search upstream takes effect without a restart.
const LEARNED_TTL: Duration = Duration::from_secs(30 * 60);

fn learned_store() -> &'static Mutex<std::collections::HashMap<(String, Learned), std::time::Instant>> {
    static STORE: std::sync::OnceLock<Mutex<std::collections::HashMap<(String, Learned), std::time::Instant>>> =
        std::sync::OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// The physical target: provider card, endpoint, model and upstream protocol.
pub(super) fn target_key(destination: &Destination) -> Option<String> {
    let Destination::Provider { provider, model_override, upstream_protocol, .. } = destination else {
        return None;
    };
    Some(format!("{}|{}|{}|{}", provider.id, provider.base_url,
        model_override.as_deref().unwrap_or(&provider.model), pipeline::request::protocol_name(*upstream_protocol)))
}

pub(super) fn learn(key: &str, what: Learned) {
    let mut store = learned_store().lock().unwrap_or_else(|error| error.into_inner());
    store.retain(|_, at| at.elapsed() < LEARNED_TTL);
    store.insert((key.to_owned(), what), std::time::Instant::now());
}

pub(super) fn learned(key: &str, what: Learned) -> bool {
    learned_store().lock().unwrap_or_else(|error| error.into_inner())
        .get(&(key.to_owned(), what)).is_some_and(|at| at.elapsed() < LEARNED_TTL)
}

/// Forward a reply unchanged while keeping a bounded copy. A reply that turns
/// the hosted tool into an ordinary call teaches this target to run search
/// separately from the next request on; this one is never delayed or replayed.
pub(super) fn watch_misclassified(response: reqwest::Response, plan: Plan, protocol: router::ModelProtocol,
    key: String, trace: route_trace::Trace) -> reqwest::Response {
    struct Watch { plan: Plan, protocol: router::ModelProtocol, sse: bool, key: String, trace: route_trace::Trace, seen: Option<Vec<u8>> }
    impl Watch {
        fn check(&mut self) {
            if self.seen.as_ref().is_some_and(|seen| self.plan.native_reply_misclassified(self.protocol, seen, self.sse)) {
                self.seen = None;
                learn(&self.key, Learned::SeparateSearch);
                self.trace.search_compatibility("native_search_tool_misclassified");
            }
        }
    }
    let status = response.status();
    let headers = response.headers().clone();
    let watch = Watch { plan, protocol, sse: native::event_stream(&response), key, trace, seen: Some(Vec::new()) };
    let stream = futures_util::stream::unfold(Some((response.bytes_stream(), watch)), |state| async move {
        let (mut source, mut watch) = state?;
        match source.next().await {
            Some(Ok(bytes)) => {
                if watch.seen.as_ref().is_some_and(|seen| seen.len() + bytes.len() > MAX_BODY) {
                    watch.seen = None;
                }
                if let Some(seen) = watch.seen.as_mut() {
                    seen.extend_from_slice(&bytes);
                    // Stream relays stop reading at the terminal frame, so the
                    // body's end may never be polled.
                    let tail = &seen[seen.len().saturating_sub(bytes.len() + 24)..];
                    if watch.sse && [&b"message_stop"[..], b"response.completed", b"response.incomplete"].iter()
                        .any(|marker| tail.windows(marker.len()).any(|window| window == *marker)) {
                        watch.check();
                    }
                }
                Some((Ok(bytes), Some((source, watch))))
            }
            Some(Err(error)) => Some((Err(error), None)),
            None => {
                watch.check();
                None
            }
        }
    });
    let mut forwarded = hyper::Response::builder().status(status);
    for (name, value) in &headers {
        if name != CONTENT_LENGTH && name != TRANSFER_ENCODING { forwarded = forwarded.header(name, value); }
    }
    reqwest::Response::from(forwarded.body(reqwest::Body::wrap_stream(stream)).expect("valid response parts"))
}

/// Cancellation before headers or during the body still records the actual
/// attempt, without reporting a successfully delivered model response.
struct UsageRound {
    pending: Option<relay_usage::RelayUsageRequest>,
    observer: Option<relay_usage::RelayUsageObserver>,
    complete: bool,
}

impl Drop for UsageRound {
    fn drop(&mut self) {
        if let Some(request) = self.pending.take() {
            request.failed(502, "response_not_received");
        }
        if !self.complete {
            if let Some(observer) = self.observer.as_mut() {
                observer.fail_delivery();
            }
        }
    }
}

fn append_output(output: &mut Vec<Value>, bytes: &mut usize, item: Value) -> ForwardResult<()> {
    *bytes = bytes.saturating_add(item.to_string().len());
    if *bytes > MAX_BODY {
        return Err((
            StatusCode::BAD_GATEWAY,
            "WebSearch output exceeded the buffer limit",
        ));
    }
    output.push(item);
    Ok(())
}

impl Execution {
    async fn run(mut self) -> ForwardResult<Value> {
        let mut body: Value = serde_json::from_slice(&self.body).map_err(|_| INVALID)?;
        body.as_object().ok_or(INVALID)?;
        body["stream"] = json!(false);
        body.as_object_mut()
            .ok_or(INVALID)?
            .remove("stream_options");
        let mut replay = Vec::new();
        let mut remaining = self.plan.max_tokens;
        let mut output = Vec::new();
        let mut output_bytes = 0;
        let mut uses = 0;
        let mut used_ids = HashSet::new();
        let mut totals = json!({"input_tokens":0,"output_tokens":0,"total_tokens":0,"input_tokens_details":{"cached_tokens":0}});
        for round in 0..MAX_ROUNDS {
            let health = self.health.as_ref().map(|health| health.next_attempt());
            let mut attempt = self.attempt.clone();
            if round > 0 {
                attempt.attempt_id = uuid::Uuid::new_v4().to_string();
                self.trace.tool_followup(&attempt.attempt_id);
            }
            let key = if self.protocol == router::ModelProtocol::Responses {"max_output_tokens"} else if body.get("max_completion_tokens").is_some() {
                "max_completion_tokens"
            } else {
                "max_tokens"
            };
            if let Some(limit) = body[key].as_u64() {
                body[key] = json!(limit.min(remaining));
            }
            let encoded = body.to_string();
            if encoded.len() > MAX_REQUEST_BODY {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "WebSearch conversation exceeded the request limit",
                ));
            }
            self.trace
                .sent(Some(encoded.as_bytes()), self.credential.clone());
            let mut usage = UsageRound {
                pending: self
                    .usage
                    .as_ref()
                    .map(|usage| usage.followup(&attempt.attempt_id)),
                observer: None,
                complete: false,
            };
            let mut headers = self.headers.clone();
            if round > 0 && headers.contains_key("idempotency-key") {
                headers.insert("idempotency-key", HeaderValue::from_str(&attempt.attempt_id).expect("UUID header"));
            }
            let response = match Box::pin(native::send(&self.client,&self.url,&headers,&body,self.protocol,&self.model,self.account.as_ref())).await
            {
                Ok(response) => response,
                Err(_) => {
                    if let Some(health) = &health { health.fail(None); }
                    self.trace.fail("connect", "transport_error");
                    return Err((StatusCode::BAD_GATEWAY, "cannot reach model provider"));
                }
            };
            let status = response.status();
            if let Some(health) = &health {
                if super::super::aggregation::health::http_failure(status.as_u16()) {
                    health.fail(super::aggregate::retry_after(response.headers()));
                } else if !status.is_success() { health.neutral(); }
            }
            self.trace.status(status.as_u16());
            let sse = native::event_stream(&response);
            usage.observer = usage
                .pending
                .take()
                .map(|request| request.observe(status.as_u16(), sse));
            let mut served = request_diagnostics::ServedModelObserver::new(
                self.protocol,
                &self.model,
                status.as_u16(),
                sse,
            );
            let bytes = match limited(response).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    if error.1 == "cannot read WebSearch response" {
                        if let Some(health) = &health { health.fail(None); }
                    }
                    self.trace
                        .observed(&attempt.attempt_id, served.finish(false));
                    self.trace.fail("body", "stream_transport_error");
                    return Err(error);
                }
            };
            if let Some(observer) = usage.observer.as_mut() {
                observer.observe(&bytes);
            }
            served.push(&bytes);
            let observation = served.finish(true);
            self.trace
                .observed(&attempt.attempt_id, observation.clone());
            if let Some(observer) = usage.observer.as_mut() {
                observer.set_model_observation(observation);
            }
            self.trace.attempt_elapsed();
            if !status.is_success() {
                self.trace.fail("headers", "upstream_http_error");
                return Err((status, "model request failed during WebSearch"));
            }
            let mut response = native::reply(self.protocol,&bytes,sse)?;
            if super::super::aggregation::health::channel_error(&response) {
                if let Some(health) = &health { health.fail(None); }
            } else if !response["error"].is_null() {
                if let Some(health) = &health { health.neutral(); }
            }
            let private_response = response.clone();
            self.adapter
                .response(&mut response, self.protocol)
                .map_err(|_| INVALID_RESPONSE)?;
            if self.protocol == router::ModelProtocol::ChatCompletions {
                replay.push(response.pointer("/choices/0/message").cloned().ok_or(INVALID_RESPONSE)?);
            } else if self.protocol == router::ModelProtocol::AnthropicMessages {
                replay.push(json!({"role":"assistant","content":response["content"]}));
            } else {
                replay.extend(response["output"].as_array().ok_or(INVALID_RESPONSE)?.iter().cloned());
            }
            let mut canonical = native::canonical(self.protocol,&response)?;
            if canonical["status"] == "failed" {
                return Err(INVALID_RESPONSE);
            }
            usage.complete = true;
            if let Some(health) = &health { health.succeed(); }
            drop(usage);
            append_assistant(&mut body,&private_response,self.protocol)?;
            for key in ["input_tokens", "output_tokens", "total_tokens"] {
                totals[key] = json!(totals[key]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(canonical["usage"][key].as_u64().unwrap_or(0)));
            }
            totals["input_tokens_details"]["cached_tokens"] = json!(totals["input_tokens_details"]
                ["cached_tokens"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(
                    canonical["usage"]["input_tokens_details"]["cached_tokens"]
                        .as_u64()
                        .unwrap_or(0)
                ));
            remaining =
                remaining.saturating_sub(canonical["usage"]["output_tokens"].as_u64().unwrap_or(0));
            let mut local = 0;
            let mut client_calls = false;
            let mut search_failed = false;
            for item in canonical["output"].as_array().ok_or(INVALID_RESPONSE)? {
                // Items are replayed and streamed by key: a relay's other shapes are refused.
                if !item.is_object() {
                    return Err(INVALID_RESPONSE);
                }
                if item["type"] != "function_call" {
                    append_output(&mut output, &mut output_bytes, item.clone())?;
                    continue;
                }
                let id = item["call_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(INVALID_RESPONSE)?;
                if !used_ids.insert(id.to_owned()) {
                    return Err(INVALID_RESPONSE);
                }
                if item["name"] != self.plan.name {
                    client_calls = true;
                    append_output(&mut output, &mut output_bytes, item.clone())?;
                    continue;
                }
                if canonical["status"] != "completed" {
                    return Err(INVALID_RESPONSE);
                }
                let input: Value =
                    serde_json::from_str(item["arguments"].as_str().ok_or(INVALID_RESPONSE)?)
                        .map_err(|_| INVALID_RESPONSE)?;
                let query = input["query"]
                    .as_str()
                    .filter(|q| {
                        !q.trim().is_empty() && q.len() <= 8192 && !q.chars().any(char::is_control)
                    })
                    .ok_or(INVALID_RESPONSE)?;
                let over_limit = uses >= MAX_ROUNDS as u64 || self
                    .plan
                    .options
                    .search
                    .as_ref()
                    .and_then(|s| s.max_uses)
                    .is_some_and(|max| uses >= max);
                let sources = if search_failed {
                    Err("search_stopped_after_failure")
                } else if over_limit {
                    Err(if uses >= MAX_ROUNDS as u64 {"local_search_limit_exceeded"} else {"max_uses_exceeded"})
                } else {
                    self.search(query,self.plan.options.search.as_ref().and_then(|search| search.max_uses).map(|max| max.saturating_sub(uses))).await
                };
                let failed = sources.is_err();
                search_failed |= failed;
                let tool_result = match &sources {
                    Ok(sources) => json!({"results":sources.sources,"summary":sources.summary}),
                    Err(error) => json!({"error":error}),
                };
                let (sources, actual_uses) = match sources {Ok(result)=>(result.sources,result.uses),Err(error)=>{
                    if self.plan.options.search_required {return Err((StatusCode::BAD_GATEWAY,error));}
                    (Vec::new(),1)
                }};
                let item = json!({"type":"web_search_call","id":id,"status":if failed {"failed"} else {"completed"},
                    "action":{"type":"search","query":query,"sources":sources}});
                append_output(&mut output, &mut output_bytes, item)?;
                let result =
                    json!({"role":"tool","tool_call_id":id,"content":tool_result.to_string()});
                append_result(&mut body,self.protocol,id,&tool_result)?;
                match self.protocol {
                    router::ModelProtocol::ChatCompletions => replay.push(result),
                    router::ModelProtocol::Responses => replay.push(json!({"type":"function_call_output","call_id":id,"output":tool_result.to_string()})),
                    router::ModelProtocol::AnthropicMessages => {
                        let result = json!({"type":"tool_result","tool_use_id":id,"content":tool_result.to_string()});
                        if let Some(content) = replay.last_mut().filter(|message| message["role"] == "user").and_then(|message| message["content"].as_array_mut()) {content.push(result)}
                        else {replay.push(json!({"role":"user","content":[result]}));}
                    }
                }
                uses += actual_uses;
                local += 1;
            }
            if local == 0 || client_calls || remaining == 0 {
                if self.plan.options.search_required && uses == 0 && !client_calls {
                    return Err((StatusCode::BAD_GATEWAY,"model did not execute required WebSearch"));
                }
                canonical["output"] = json!(output);
                canonical["usage"] = totals;
                if remaining == 0 && local > 0 && !client_calls {
                    canonical["status"] = json!("incomplete");
                    canonical["incomplete_details"] = json!({"reason":"max_output_tokens"});
                }
                if self.plan.caller == router::ModelProtocol::Responses {
                    self.namespaces.restore_response(&mut canonical);
                    protocol::history::remember(&canonical, self.input.clone());
                }
                let visible = visible(&canonical, &self.plan.options, self.plan.caller)?;
                if let Some(history) = &self.history {
                    history.remember_local_search(
                        &visible,
                        &self.plan.name,
                        &self.plan.options.search.as_ref().unwrap().name,
                        &replay,
                    );
                }
                return Ok(canonical);
            }
            // The first forced search has completed. Do not force it forever.
            body["tool_choice"] = if self.protocol == router::ModelProtocol::AnthropicMessages {json!({"type":"auto"})} else {json!("auto")};
            if search_failed
                || self
                    .plan
                    .options
                    .search
                    .as_ref()
                    .and_then(|s| s.max_uses)
                    .is_some_and(|max| uses >= max)
            {
                if let Some(tools) = body["tools"].as_array_mut() {
                    tools.retain(|tool| tool["function"]["name"] != self.plan.name && tool["name"] != self.plan.name);
                    if tools.is_empty() {
                        body.as_object_mut().unwrap().remove("tools");
                        body.as_object_mut().unwrap().remove("tool_choice");
                    }
                }
            }
        }
        Err((
            StatusCode::BAD_GATEWAY,
            "WebSearch exceeded the model round limit",
        ))
    }

    async fn search(&mut self, query: &str, remaining: Option<u64>) -> Result<native::ResultSet, &'static str> {
        self.trace.stage("search");
        if let Some(selection) = self.selection.take() {
            self.backend = Box::pin(selection.activate()).await.map_err(|(status,_)| if status == StatusCode::UNAUTHORIZED {"search_authentication_failed"} else {"search_service_unavailable"})?;
        }
        if let Some(backend) = &self.backend.native {return Box::pin(backend.search(self,query,remaining)).await}
        let deepseek = self.backend.api == Some(router::SearchApi::DeepSeek);
        let request = if deepseek {
            let filtered = !self.plan.allowed_domains.is_empty() || !self.plan.blocked_domains.is_empty();
            // Extra hits leave enough after the local domain filter.
            json!({"query":deepseek_query(query,&self.plan.allowed_domains,&self.plan.blocked_domains),"count":if filtered {30} else {10}})
        } else {
            let mut request = json!({"query":query,"numResults":5});
            if !self.plan.allowed_domains.is_empty() {
                request["allowedDomains"] = json!(self.plan.allowed_domains);
            }
            request
        };
        let response = self
            .backend
            .client
            .post(&self.backend.endpoint)
            .headers(self.backend.headers.clone())
            .json(&request)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| "unavailable")?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 | 403 if deepseek => "search_authentication_failed",
                402 if deepseek => "search_balance_insufficient",
                429 if deepseek => "too_many_requests",
                _ => "unavailable",
            });
        }
        let bytes = limited(response).await.map_err(|_| "unavailable")?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "unavailable")?;
        let results = value["results"].as_array().ok_or("unavailable")?;
        let mut seen = HashSet::new();
        let mut sources = Vec::new();
        for result in results {
            let Some(url) = result["url"]
                .as_str()
                .filter(|url| !url.chars().any(char::is_control))
            else {
                continue;
            };
            let Ok(parsed) = reqwest::Url::parse(url) else {
                continue;
            };
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !seen.insert(url)
            {
                continue;
            }
            let host = parsed.host_str().unwrap().trim_end_matches('.');
            if (!self.plan.allowed_domains.is_empty()
                && !self.plan.allowed_domains.iter().any(|domain| on_domain(host, domain)))
                || self.plan.blocked_domains.iter().any(|domain| on_domain(host, domain))
            {
                continue;
            }
            // DeepSeek's longer summary is written for model input.
            let snippet = result["summary"].as_str().filter(|text| !text.trim().is_empty())
                .or_else(|| result["snippet"].as_str()).unwrap_or_default();
            sources.push(json!({"url":url,"title":result["title"].as_str().filter(|title| !title.is_empty()).unwrap_or(url),"snippet":snippet}));
            if sources.len() == 10 {
                break;
            }
        }
        Ok(native::ResultSet {sources,summary:String::new(),uses:1})
    }
}

fn response_input(body: &mut Value) -> ForwardResult<&mut Vec<Value>> {
    if let Some(text) = body["input"].as_str() {body["input"] = json!([{"role":"user","content":text}]);}
    body["input"].as_array_mut().ok_or(INVALID)
}

fn append_assistant(body: &mut Value, response: &Value, protocol: router::ModelProtocol) -> ForwardResult<()> {
    match protocol {
        router::ModelProtocol::ChatCompletions => {
            let mut message = response.pointer("/choices/0/message").filter(|message| message.is_object()).cloned().ok_or(INVALID_RESPONSE)?;
            message["role"] = json!("assistant");
            body["messages"].as_array_mut().ok_or(INVALID)?.push(message);
        }
        router::ModelProtocol::AnthropicMessages => {
            let content = response["content"].as_array().ok_or(INVALID_RESPONSE)?;
            body["messages"].as_array_mut().ok_or(INVALID)?.push(json!({"role":"assistant","content":content}));
        }
        router::ModelProtocol::Responses => {
            response_input(body)?.extend(response["output"].as_array().ok_or(INVALID_RESPONSE)?.iter().cloned());
        }
    }
    Ok(())
}

fn append_result(body: &mut Value, protocol: router::ModelProtocol, id: &str, result: &Value) -> ForwardResult<()> {
    let text = result.to_string();
    match protocol {
        router::ModelProtocol::ChatCompletions => body["messages"].as_array_mut().ok_or(INVALID)?.push(json!({"role":"tool","tool_call_id":id,"content":text})),
        router::ModelProtocol::Responses => response_input(body)?.push(json!({"type":"function_call_output","call_id":id,"output":text})),
        router::ModelProtocol::AnthropicMessages => {
            let messages = body["messages"].as_array_mut().ok_or(INVALID)?;
            let result = json!({"type":"tool_result","tool_use_id":id,"content":text});
            if let Some(content) = messages.last_mut().filter(|message| message["role"] == "user").and_then(|message| message["content"].as_array_mut()) {content.push(result)}
            else {messages.push(json!({"role":"user","content":[result]}))}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_responses_search_uses_local_chat_executor_only_when_supported() {
        let card = router::Provider {
            api_key: "fixture-key".into(),
            base_url: "https://api.commandcode.ai/provider/v1".into(),
            model: "deepseek/deepseek-v4.1-flash-fast".into(),
            protocol: router::ModelProtocol::Responses,
            models: vec![crate::core::newapi::Model {
                id: "deepseek/deepseek-v4.1-flash-fast".into(),
                endpoint_types: vec!["/chat/completions".into(), "/responses".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let destination = |provider| Destination::Provider {
            provider: Box::new(provider), model_override: None,
            upstream_protocol: router::ModelProtocol::Responses,
            translation: protocol::Translation::between(
                router::ModelProtocol::AnthropicMessages,
                router::ModelProtocol::Responses,
            ),
        };
        let search = br#"{"model":"claude-haiku-4-5","max_tokens":128,"messages":[{"role":"user","content":"search"}],"tools":[{"type":"web_search_20250305","name":"web_search_next"}]}"#;
        let mut selected = destination(card.clone());
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(search), false));
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(br#"{"messages":[]}"#), true));
        assert!(Plan::command_responses_search_to_chat(&mut selected, Some(search), true));
        assert!(matches!(selected, Destination::Provider { upstream_protocol: router::ModelProtocol::ChatCompletions, .. }));
        assert!(Plan::for_destination(&selected, Some(search)).unwrap().is_some());

        let mut native_messages = Destination::Provider {
            provider:Box::new(card.clone()),model_override:Some("claude-haiku-4-5".into()),
            upstream_protocol:router::ModelProtocol::AnthropicMessages,translation:None,
        };
        assert!(Plan::command_responses_search_to_chat(&mut native_messages,Some(search),true));
        assert!(Plan::for_destination(&native_messages,Some(search)).unwrap().is_some());
        let mut native_codex = Destination::Provider {
            provider:Box::new(card.clone()),model_override:Some("claude-haiku-4-5".into()),
            upstream_protocol:router::ModelProtocol::Responses,translation:None,
        };
        assert!(Plan::command_responses_search_to_chat(&mut native_codex,Some(br#"{"model":"claude-haiku-4-5","input":"search","tools":[{"type":"web_search"}]}"#),true));
        assert!(matches!(native_codex,Destination::Provider {translation:Some(protocol::Translation {from:router::ModelProtocol::Responses,..}),..}));

        let mut responses_only = card.clone();
        responses_only.models[0].endpoint_types = vec!["/responses".into()];
        let mut selected = destination(responses_only);
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(search), true));

        let mut unrelated = card;
        unrelated.base_url = "https://other.example/v1".into();
        let mut selected = destination(unrelated);
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(search), true));
    }

    #[test]
    fn deepseek_queries_fit_its_limit_and_carry_domain_operators() {
        let long = "长".repeat(200);
        assert_eq!(deepseek_query(&long, &[], &[]).chars().count(), DEEPSEEK_QUERY_CHARS);
        assert_eq!(deepseek_query(" Rust release ", &[], &[]), "Rust release");
        let domains = |list: &[&str]| list.iter().map(|domain| domain.to_string()).collect::<Vec<_>>();
        assert_eq!(deepseek_query("Rust", &domains(&["rust-lang.org"]), &domains(&["spam.example"])),
            "Rust site:rust-lang.org -site:spam.example");
        // Several allowed hosts cannot be one site operator; local filtering keeps them.
        assert_eq!(deepseek_query("Rust", &domains(&["a.example", "b.example"]), &[]), "Rust");
        let crowded = deepseek_query(&long, &domains(&["rust-lang.org"]), &domains(&["x".repeat(80).as_str()]));
        assert!(crowded.ends_with(" site:rust-lang.org") && crowded.chars().count() == DEEPSEEK_QUERY_CHARS);
    }

    #[test]
    fn search_selection_respects_explicit_account_and_ambiguous_automatic_mode() {
        let card = |id: &str| router::Provider { id:id.into(), api_key:format!("fixture-{id}"),
            base_url:"https://api.commandcode.ai/provider/v1".into(), ..Default::default() };
        let destination = |provider| Destination::Provider { provider:Box::new(provider), model_override:None,
            upstream_protocol:router::ModelProtocol::ChatCompletions, translation:None };
        let unrelated = destination(router::Provider { chat_search: protocol::ChatSearch::Command, ..Default::default() });
        let mut store = router::Store::default();
        assert!(SearchBackend::provider(&store, &unrelated).is_err());
        store.providers.push(card("first"));
        assert_eq!(SearchBackend::provider(&store, &unrelated).unwrap().id, "first");
        store.providers.push(card("second"));
        assert!(SearchBackend::provider(&store, &unrelated).is_err());
        let own_card = destination(card("second"));
        assert_eq!(SearchBackend::provider(&store, &own_card).unwrap().id, "second");
        store.router.web_search_provider_id = Some("first".into());
        assert_eq!(SearchBackend::provider(&store, &own_card).unwrap().id, "first");
        store.providers.remove(0);
        assert!(SearchBackend::provider(&store, &own_card).is_err(), "A lost explicit account must not use another account");
        store.providers.push(router::Provider {id:"native".into(),model:"search-model".into(),protocol:router::ModelProtocol::Responses,..Default::default()});
        store.router.web_search_provider_id = Some("native".into());
        store.router.web_search_model_id = Some("search-model".into());
        assert!(!Plan::command_enabled(&store.providers[0],&store),"a native fallback never selects Command's alternate Chat endpoint");
        assert_eq!(SearchBackend::provider(&store,&own_card).unwrap().id,"second","explicit Command uses its own account even with a native fallback configured");
        assert!(Plan::delegated(&unrelated,&store,Some(br#"{"model":"test","input":"search","tools":[{"type":"web_search"}]}"#)).unwrap().is_none());
    }
}
