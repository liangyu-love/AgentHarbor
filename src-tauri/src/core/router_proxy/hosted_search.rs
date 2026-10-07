//! Execute Messages hosted search for any Chat model using an independent
//! search service. Model protocol, model credentials and network mode stay intact.
use super::*;
use std::collections::HashSet;

const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_ROUNDS: usize = 32;
const COMMAND_SEARCH_ENDPOINT: &str = "https://api.commandcode.ai/alpha/web-search";
#[cfg(test)]
pub(super) static TEST_SEARCH_ENDPOINT: Mutex<Option<String>> = Mutex::new(None);
const INVALID: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "invalid WebSearch request");
const INVALID_RESPONSE: (StatusCode, &str) = (
    StatusCode::BAD_GATEWAY,
    "invalid Chat model response during WebSearch",
);

pub(super) struct Plan {
    pub normalized: Bytes,
    name: String,
    allowed_domains: Vec<String>,
    options: protocol::ResponseOptions,
    streaming: bool,
    max_tokens: u64,
}

impl Plan {
    /// Command Code exposes search through /alpha/web-search, not as a hosted
    /// tool on /responses. Use the existing local executor for search turns
    /// when the selected model also has a Chat endpoint. Ordinary turns keep
    /// the configured Responses protocol.
    pub fn command_responses_search_to_chat(
        destination: &mut Destination,
        body: Option<&[u8]>,
    ) -> bool {
        #[derive(serde::Deserialize)]
        struct DeclaredTools {
            #[serde(default)]
            tools: Vec<Value>,
        }
        let Destination::Provider { provider, model_override, upstream_protocol, translation } = destination else {
            return false;
        };
        if *upstream_protocol != router::ModelProtocol::Responses
            || !translation.as_ref().is_some_and(|pair| pair.from == router::ModelProtocol::AnthropicMessages
                && pair.to == router::ModelProtocol::Responses)
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
            router::ModelProtocol::AnthropicMessages,
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
            translation: Some(pair),
            ..
        } = destination
        else {
            return Ok(None);
        };
        if pair.from != router::ModelProtocol::AnthropicMessages
            || pair.to != router::ModelProtocol::ChatCompletions
        {
            return Ok(None);
        }
        let Some(body) = body else { return Ok(None) };
        let value: Value = serde_json::from_slice(body).map_err(|_| INVALID)?;
        if !value["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(protocol::web_search::is_search))
        {
            return Ok(None);
        }
        Self::prepare(value).map(Some)
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
        let allowed_domains = tool["allowed_domains"]
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
            .collect::<ForwardResult<Vec<_>>>()?;
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
            name,
            allowed_domains,
            options,
            streaming,
            max_tokens,
        })
    }
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

/// Backend selection is separate from the model's protocol. Only the adapter
/// identifies a brand; the Chat executor itself accepts every Chat provider.
pub(super) struct SearchBackend {
    pub client: Client,
    pub endpoint: String,
    pub headers: HeaderMap,
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
            endpoint: {
                #[cfg(test)]
                {
                    TEST_SEARCH_ENDPOINT
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| COMMAND_SEARCH_ENDPOINT.into())
                }
                #[cfg(not(test))]
                {
                    COMMAND_SEARCH_ENDPOINT.into()
                }
            },
            headers,
        })
    }

    fn provider<'a>(
        store: &'a router::Store,
        destination: &'a Destination,
    ) -> ForwardResult<&'a router::Provider> {
        let selected = if let Some(id) = &store.router.web_search_provider_id {
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
            } else {
                Self::unique(store)?
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
    pub adapter: tool_schema::Adapter,
    pub credential: request_diagnostics::CredentialSnapshot,
}

pub(super) async fn relay(execution: Execution) -> ForwardResult {
    let streaming = execution.plan.streaming;
    let options = execution.plan.options.clone();
    let trace = execution.trace.clone();
    let future = async move {
        match tokio::time::timeout(Duration::from_secs(900), execution.run()).await {
            Ok(result) => result,
            Err(_) => Err((StatusCode::GATEWAY_TIMEOUT, "WebSearch request timed out")),
        }
    };
    if !streaming {
        let canonical = future.await?;
        let visible = visible(&canonical, &options)?;
        trace.finish();
        return Ok(Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(full_body(visible.to_string()))
            .unwrap());
    }
    let stream = futures_util::stream::once(async {
        Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(
            b"event: ping\ndata: {}\n\n",
        )))
    })
    .chain(futures_util::stream::once(async move {
        let output = match future.await {
            Ok(canonical) => {
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

fn visible(canonical: &Value, options: &protocol::ResponseOptions) -> ForwardResult<Value> {
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

async fn limited(response: reqwest::Response) -> ForwardResult<Bytes> {
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
    async fn run(self) -> ForwardResult<Value> {
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
            let mut attempt = self.attempt.clone();
            if round > 0 {
                attempt.attempt_id = uuid::Uuid::new_v4().to_string();
                self.trace.tool_followup(&attempt.attempt_id);
            }
            let key = if body.get("max_completion_tokens").is_some() {
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
            let response = match self
                .client
                .post(&self.url)
                .headers(self.headers.clone())
                .body(encoded)
                .send()
                .await
            {
                Ok(response) => response,
                Err(_) => {
                    self.trace.fail("connect", "transport_error");
                    return Err((StatusCode::BAD_GATEWAY, "cannot reach Chat model provider"));
                }
            };
            let status = response.status();
            self.trace.status(status.as_u16());
            usage.observer = usage
                .pending
                .take()
                .map(|request| request.observe(status.as_u16(), false));
            let mut served = request_diagnostics::ServedModelObserver::new(
                router::ModelProtocol::ChatCompletions,
                &self.model,
                status.as_u16(),
                false,
            );
            let bytes = match limited(response).await {
                Ok(bytes) => bytes,
                Err(error) => {
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
                return Err((status, "Chat model request failed during WebSearch"));
            }
            let mut response: Value =
                serde_json::from_slice(&bytes).map_err(|_| INVALID_RESPONSE)?;
            let mut message = response
                .pointer("/choices/0/message")
                .filter(|message| message.is_object())
                .cloned()
                .ok_or(INVALID_RESPONSE)?;
            message["role"] = json!("assistant");
            self.adapter
                .response(&mut response, router::ModelProtocol::ChatCompletions)
                .map_err(|_| INVALID_RESPONSE)?;
            replay.push(
                response
                    .pointer("/choices/0/message")
                    .cloned()
                    .ok_or(INVALID_RESPONSE)?,
            );
            let mut canonical = protocol::canonical_response(
                protocol::Translation {
                    from: router::ModelProtocol::AnthropicMessages,
                    to: router::ModelProtocol::ChatCompletions,
                },
                &response,
            )
            .map_err(|_| INVALID_RESPONSE)?;
            if canonical["status"] == "failed" {
                return Err(INVALID_RESPONSE);
            }
            usage.complete = true;
            drop(usage);
            body["messages"]
                .as_array_mut()
                .ok_or(INVALID)?
                .push(message);
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
                let over_limit = self
                    .plan
                    .options
                    .search
                    .as_ref()
                    .and_then(|s| s.max_uses)
                    .is_some_and(|max| uses >= max);
                let sources = if over_limit {
                    Err("max_uses_exceeded")
                } else {
                    self.search(query).await
                };
                search_failed |= sources.is_err();
                let tool_result = match &sources {
                    Ok(sources) => json!({"results":sources}),
                    Err(error) => json!({"error":error}),
                };
                let item = json!({"type":"web_search_call","id":id,"status":if sources.is_ok() {"completed"} else {"failed"},
                    "action":{"type":"search","query":query,"sources":sources.unwrap_or_default()}});
                append_output(&mut output, &mut output_bytes, item)?;
                let result =
                    json!({"role":"tool","tool_call_id":id,"content":tool_result.to_string()});
                body["messages"]
                    .as_array_mut()
                    .ok_or(INVALID)?
                    .push(result.clone());
                replay.push(result);
                uses += 1;
                local += 1;
            }
            if local == 0 || client_calls || remaining == 0 {
                canonical["output"] = json!(output);
                canonical["usage"] = totals;
                if remaining == 0 && local > 0 && !client_calls {
                    canonical["status"] = json!("incomplete");
                    canonical["incomplete_details"] = json!({"reason":"max_output_tokens"});
                }
                let visible = visible(&canonical, &self.plan.options)?;
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
            body["tool_choice"] = json!("auto");
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
                    tools.retain(|tool| tool["function"]["name"] != self.plan.name);
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

    async fn search(&self, query: &str) -> Result<Vec<Value>, &'static str> {
        self.trace.stage("search");
        let mut request = json!({"query":query,"numResults":5});
        if !self.plan.allowed_domains.is_empty() {
            request["allowedDomains"] = json!(self.plan.allowed_domains);
        }
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
            return Err("unavailable");
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
            if !self.plan.allowed_domains.is_empty()
                && !self.plan.allowed_domains.iter().any(|domain| {
                    host == domain
                        || host
                            .strip_suffix(domain)
                            .is_some_and(|prefix| prefix.ends_with('.'))
                })
            {
                continue;
            }
            sources.push(json!({"url":url,"title":result["title"].as_str().unwrap_or(url),"snippet":result["snippet"].as_str().unwrap_or_default()}));
            if sources.len() == 10 {
                break;
            }
        }
        Ok(sources)
    }
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
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(br#"{"messages":[]}"#)));
        assert!(Plan::command_responses_search_to_chat(&mut selected, Some(search)));
        assert!(matches!(selected, Destination::Provider { upstream_protocol: router::ModelProtocol::ChatCompletions, .. }));
        assert!(Plan::for_destination(&selected, Some(search)).unwrap().is_some());

        let mut responses_only = card.clone();
        responses_only.models[0].endpoint_types = vec!["/responses".into()];
        let mut selected = destination(responses_only);
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(search)));

        let mut unrelated = card;
        unrelated.base_url = "https://other.example/v1".into();
        let mut selected = destination(unrelated);
        assert!(!Plan::command_responses_search_to_chat(&mut selected, Some(search)));
    }

    #[test]
    fn search_selection_respects_explicit_account_and_ambiguous_automatic_mode() {
        let card = |id: &str| router::Provider { id:id.into(), api_key:format!("fixture-{id}"),
            base_url:"https://api.commandcode.ai/provider/v1".into(), ..Default::default() };
        let destination = |provider| Destination::Provider { provider:Box::new(provider), model_override:None,
            upstream_protocol:router::ModelProtocol::ChatCompletions, translation:None };
        let unrelated = destination(router::Provider::default());
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
    }
}
