//! Native search uses the explicitly selected account/model. Only the query
//! and tool constraints cross this boundary; inference history stays at its target.
use super::*;

pub(in crate::core::router_proxy) struct Backend {
    pub protocol: router::ModelProtocol,
    pub model: String,
    pub provider_id: String,
    pub provider_name: String,
    pub account: Option<account_sources::RequestAuth>,
    pub credential: request_diagnostics::CredentialSnapshot,
    pub usage: relay_usage::RelayUsageRequest,
    max_tokens: u64,
}

pub(super) struct ResultSet {
    pub sources: Vec<Value>,
    pub summary: String,
    pub uses: u64,
}

/// Resolve credentials only once this fallback is actually selected.
pub(in crate::core::router_proxy) struct Selection {
    context: ArcContext,
    provider: router::Provider,
    protocol: router::ModelProtocol,
    model: String,
    usage: relay_usage::RelayUsageRequest,
    max_tokens: u64,
    account: Option<account_sources::RequestAuth>,
}

impl Selection {
    pub fn validate(&self, plan: &Plan) -> ForwardResult<()> {
        let mut request = plan.native_request.clone().ok_or(INVALID)?;
        if let Some(pair) = protocol::Translation::between(plan.caller,self.protocol) {
            request = protocol::request_body(pair,&request).map_err(|_| (StatusCode::BAD_REQUEST,"selected search cannot preserve these WebSearch options"))?.body;
        }
        request["model"] = json!(self.model);
        if plan.options.search.as_ref().and_then(|search| search.max_uses).is_some() {
            if let Some(shaped) = self.provider.account_source.and_then(|kind| account_sources::shape_body_for_source(kind,self.protocol,request.to_string().as_bytes())) {
                let shaped: Value = serde_json::from_slice(&shaped).map_err(|_| INVALID)?;
                if (self.protocol == router::ModelProtocol::Responses && shaped["max_tool_calls"] != request["max_tool_calls"])
                    || (self.protocol == router::ModelProtocol::AnthropicMessages && shaped["tools"][0]["max_uses"] != request["tools"][0]["max_uses"]) {
                    return Err((StatusCode::BAD_REQUEST,"selected search account cannot enforce the requested search budget"));
                }
            }
        }
        Ok(())
    }
    pub fn new(
        context: &ArcContext, store: &router::Store, catalog: &crate::core::models_dev::Catalog,
        trace: &route_trace::Trace, identity: request_diagnostics::RequestIdentity,
    ) -> ForwardResult<Self> {
        let id = store.router.web_search_provider_id.as_deref().ok_or(INVALID)?;
        let provider = store.providers.iter().find(|provider| provider.id == id)
            .ok_or((StatusCode::BAD_REQUEST,"configured WebSearch service is unavailable"))?;
        let model = store.router.web_search_model_id.as_deref()
            .ok_or((StatusCode::BAD_REQUEST,"select a native WebSearch service model"))?;
        if !router::native_search_models(provider).iter().any(|candidate| candidate == model) {
            return Err((StatusCode::BAD_REQUEST,"configured WebSearch model is unavailable"));
        }
        let protocol = provider.protocol_for_model(model);
        Ok(Self::for_provider(context,provider,model,protocol,catalog,trace,identity,None))
    }

    /// A relay that misclassifies its hosted tool still owns the search. Keep
    /// the exact inference target and resolved account; no global service choice.
    pub fn for_destination(context: &ArcContext, destination: &Destination, catalog: &crate::core::models_dev::Catalog,
        trace: &route_trace::Trace, identity: request_diagnostics::RequestIdentity, account: Option<&account_sources::RequestAuth>) -> ForwardResult<Self> {
        let Destination::Provider {provider,model_override,upstream_protocol,..} = destination else {return Err(INVALID)};
        if !matches!(upstream_protocol,router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages) {return Err(INVALID)}
        let model = model_override.as_deref().unwrap_or(&provider.model);
        Ok(Self::for_provider(context,provider,model,*upstream_protocol,catalog,trace,identity,account.cloned()))
    }

    fn for_provider(context: &ArcContext, provider: &router::Provider, model: &str, protocol: router::ModelProtocol,
        catalog: &crate::core::models_dev::Catalog, trace: &route_trace::Trace, identity: request_diagnostics::RequestIdentity,
        account: Option<account_sources::RequestAuth>) -> Self {
        let usage = relay_usage::RelayUsageRequest::new(context.app_data_dir.clone(),provider.id.clone(),provider.name.clone(),model.into(),
            pipeline::request::protocol_name(protocol).into(),router::pricing_for(provider,model,catalog))
            .with_request_id(trace.request_id()).with_identity(identity);
        let max_tokens = router::resolve_model_metadata(provider,model,None,catalog).output_limit.value.unwrap_or(4096).min(4096).max(1);
        Self {context:context.clone(),provider:provider.clone(),protocol,model:model.into(),usage,max_tokens,account}
    }

    pub async fn activate(self) -> ForwardResult<SearchBackend> {
        let Self {context,provider,protocol,model,usage,max_tokens,account} = self;
        let destination = Destination::Provider {provider:Box::new(provider.clone()),model_override:Some(model.clone()),upstream_protocol:protocol,translation:None};
        let client = client_for(&context,&destination)?;
        // Account refresh futures include every source's auth flow. Keep them
        // off the Windows worker stack inside the multi-round executor.
        let account = match account {Some(account)=>Some(account),None=>Box::pin(account_auth(&context,&destination)).await?};
        let suffix = if protocol == router::ModelProtocol::Responses {"responses"} else {"messages"};
        let mut initial = HeaderMap::new();
        initial.insert(CONTENT_TYPE,HeaderValue::from_static("application/json"));
        initial.insert(ACCEPT_ENCODING,HeaderValue::from_static("identity"));
        let (endpoint,headers) = match &account {
            Some(auth) => account_target(auth,protocol,None,suffix,None,initial)?,
            None => provider_target(&provider,protocol,None,suffix,None,initial)?,
        };
        let credential = request_diagnostics::CredentialSnapshot::from_header(&context.app_data_dir,
            if account.is_some() {"accountSource"} else {"providerApiKey"},
            &format!("{}|{}",provider.id,endpoint),headers.get(AUTHORIZATION).or_else(|| headers.get("x-api-key")).and_then(|header| header.to_str().ok()),None);
        let usage = usage.with_credential(credential.clone());
        Ok(SearchBackend {client,endpoint,headers,native:Some(Backend {protocol,model,provider_id:provider.id.clone(),provider_name:provider.name.clone(),account,credential,usage,max_tokens})})
    }
}

impl Backend {
    pub(super) async fn search(&self, execution: &Execution, query: &str, remaining: Option<u64>) -> Result<ResultSet,&'static str> {
        let mut request = execution.plan.native_request.clone().ok_or("invalid_search_contract")?;
        let caller = execution.plan.caller;
        if caller == router::ModelProtocol::Responses {
            request["input"] = json!(query);
            request["tool_choice"] = json!({"type":request["tools"][0]["type"]});
            if let Some(remaining) = remaining {request["max_tool_calls"] = json!(remaining);}
        } else {
            request["messages"] = json!([{"role":"user","content":query}]);
            request["tool_choice"] = json!({"type":"tool","name":request["tools"][0]["name"]});
            if let Some(remaining) = remaining {request["tools"][0]["max_uses"] = json!(remaining);}
        }
        if let Some(pair) = protocol::Translation::between(caller,self.protocol) {
            request = protocol::request_body(pair,&request).map_err(|_| "unsupported_search_options")?.body;
        }
        request["model"] = json!(self.model);
        request["stream"] = json!(false);
        if self.protocol == router::ModelProtocol::Responses {
            request["max_output_tokens"] = json!(self.max_tokens);
            request["include"] = json!(["web_search_call.action.sources"]);
        } else {request["max_tokens"] = json!(self.max_tokens);}
        // Account shapers may remove unsupported hard limits. Never send a
        // budgeted search if that budget would disappear before the request.
        if remaining.is_some() {
            if let Some(shaped) = self.account.as_ref().and_then(|auth| auth.shape_body(self.protocol,request.to_string().as_bytes())) {
                let shaped: Value = serde_json::from_slice(&shaped).map_err(|_| "unsupported_search_options")?;
                if (self.protocol == router::ModelProtocol::Responses && shaped["max_tool_calls"] != request["max_tool_calls"])
                    || (self.protocol == router::ModelProtocol::AnthropicMessages && shaped["tools"][0]["max_uses"] != request["tools"][0]["max_uses"]) {
                    return Err("search_budget_unsupported");
                }
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut diagnostic = SearchAttempt::new(execution.trace.clone(),&id,self);
        let mut usage = UsageRound {pending:Some(self.usage.followup(&id)),observer:None,complete:false};
        let response = tokio::time::timeout(Duration::from_secs(90), Box::pin(send(&execution.backend.client,&execution.backend.endpoint,
            &execution.backend.headers,&request,self.protocol,&self.model,self.account.as_ref()))).await
            .map_err(|_| "search_timeout")?.map_err(|_| "search_unavailable")?;
        let status = response.status();
        let sse = event_stream(&response);
        diagnostic.status = Some(status.as_u16());
        usage.observer = usage.pending.take().map(|request| request.observe(status.as_u16(),sse));
        let mut served = request_diagnostics::ServedModelObserver::new(self.protocol,&self.model,status.as_u16(),sse);
        let bytes = tokio::time::timeout(Duration::from_secs(90),limited(response)).await.map_err(|_| "search_timeout")?.map_err(|_| "search_unavailable")?;
        served.push(&bytes);
        let observation = served.finish(true);
        diagnostic.observation = Some(observation.clone());
        if let Some(observer) = usage.observer.as_mut() {observer.observe(&bytes);}
        if let Some(observer) = usage.observer.as_mut() {observer.set_model_observation(observation);}
        if !status.is_success() {return Err("search_upstream_error")}
        let value = reply(self.protocol,&bytes,sse).map_err(|_| "invalid_search_response")?;
        let canonical = canonical(self.protocol,&value).map_err(|_| "invalid_search_response")?;
        if canonical["status"] != "completed" {return Err("search_incomplete")}
        let output = canonical["output"].as_array().ok_or("invalid_search_response")?;
        let calls: Vec<_> = output.iter().filter(|item| item["type"] == "web_search_call").collect();
        if calls.is_empty() {return Err("search_not_executed")}
        let mut ids = HashSet::new();
        if calls.iter().any(|item| item["id"].as_str().filter(|id| !id.is_empty()).is_none_or(|id| !ids.insert(id))) {return Err("invalid_search_response")}
        if calls.iter().any(|item| item["status"] != "completed") {return Err("search_tool_failed")}
        let uses = calls.len() as u64;
        if uses > MAX_ROUNDS as u64 || remaining.is_some_and(|remaining| uses > remaining) {return Err("search_budget_exceeded")}
        let mut seen = HashSet::new();
        let mut sources = Vec::new();
        let mut summary = String::new();
        for item in protocol::web_search::enrich_output(output,remaining) {
            for source in item["action"]["sources"].as_array().into_iter().flatten() {
                if let Some(url) = protocol::web_search::url(&source["url"]) {
                    if seen.insert(url.to_owned()) {sources.push(json!({"url":url,"title":source["title"].as_str().unwrap_or(url),"snippet":source["snippet"].as_str().unwrap_or_default()}));}
                }
            }
            for part in item["content"].as_array().into_iter().flatten() {
                if part["type"] == "output_text" {
                    if let Some(text) = part["text"].as_str() {summary.push_str(text);summary.push('\n');}
                }
            }
        }
        // An actual completed search may legitimately have no hits.
        diagnostic.error = None;
        usage.complete = true;
        Ok(ResultSet {sources,summary,uses})
    }
}

/// Shared per-round account transport: keep standard protocol history between
/// rounds and apply source-specific envelopes/decoders to each outbound request.
pub(super) async fn send(client: &Client, url: &str, headers: &HeaderMap, body: &Value,
    protocol: router::ModelProtocol, model: &str, account: Option<&account_sources::RequestAuth>) -> ForwardResult<reqwest::Response> {
    let mut bytes = body.to_string().into_bytes();
    let mut url = url.to_owned();
    let mut headers = headers.clone();
    headers.remove(CONTENT_LENGTH);
    let mut decoder = None;
    let mut fix = None;
    if let Some(auth) = account {
        if let Some(exchange) = auth.exchange(protocol,&bytes,model).transpose()? {return Ok(exchange.run().await)}
        if let Some(encoded) = auth.encode(protocol,&bytes,model).transpose()? {
            url = format!("{}/{}",auth.base.trim_end_matches('/'),encoded.path);
            bytes = encoded.body;
            decoder = Some(encoded.decoder);
            for (name,value) in encoded.headers {headers.insert(hyper::header::HeaderName::from_static(name),HeaderValue::from_str(&value).map_err(|_| INVALID)?);}
        } else if let Some(rerouted) = auth.reroute(protocol,&bytes).transpose()? {
            if let Some(rest) = url.strip_prefix(auth.base.trim_end_matches('/')) {url = format!("{}{rest}",rerouted.base.trim_end_matches('/'));}
            for (name,_) in &auth.headers {headers.remove(*name);}
            for (name,value) in rerouted.headers {headers.insert(hyper::header::HeaderName::from_static(name),HeaderValue::from_str(&value).map_err(|_| INVALID)?);}
            bytes = rerouted.body;
            fix = rerouted.fix;
        } else {
            fix = auth.reply_fix(protocol,&bytes);
            for (name,value) in auth.body_headers(protocol,&bytes) {headers.insert(hyper::header::HeaderName::from_static(name),HeaderValue::from_str(&value).map_err(|_| INVALID)?);}
            if let Some(shaped) = auth.shape_body(protocol,&bytes) {bytes = shaped;}
        }
    }
    let mut response = client.post(url).headers(headers).body(bytes).send().await
        .map_err(|_| (StatusCode::BAD_GATEWAY,"cannot reach model provider during WebSearch"))?;
    if let Some(fix) = fix {response = fix.apply(response).await;}
    if let Some(decoder) = decoder {response = decoder.decode(response).await;}
    Ok(response)
}

pub(super) fn event_stream(response: &reqwest::Response) -> bool {
    response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).is_some_and(|value| value.starts_with("text/event-stream"))
}

pub(super) fn canonical(protocol: router::ModelProtocol, value: &Value) -> ForwardResult<Value> {
    protocol::canonical_response(protocol::Translation {from:router::ModelProtocol::Responses,to:protocol},value).map_err(|_| INVALID_RESPONSE)
}

pub(super) fn reply(target: router::ModelProtocol, bytes: &[u8], sse: bool) -> ForwardResult<Value> {
    if !sse {return serde_json::from_slice(bytes).map_err(|_| INVALID_RESPONSE)}
    let encoded = if target == router::ModelProtocol::Responses {
        std::str::from_utf8(bytes).map_err(|_| INVALID_RESPONSE)?.to_owned()
    } else {
        let mut translator = protocol::stream_translator(protocol::Translation {from:router::ModelProtocol::Responses,to:target});
        let mut encoded = translator.push(bytes).concat();
        encoded.push_str(&translator.finish().concat());
        if translator.failure().is_some() {return Err(INVALID_RESPONSE)}
        encoded
    };
    let mut terminal = None;
    let mut data = String::new();
    for line in encoded.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !data.is_empty() && data.trim() != "[DONE]" {
                let frame: Value = serde_json::from_str(data.trim_end()).map_err(|_| INVALID_RESPONSE)?;
                if matches!(frame["type"].as_str(),Some("response.completed"|"response.incomplete"|"response.failed")) {
                    if terminal.is_some() {return Err(INVALID_RESPONSE)}
                    terminal = Some(frame["response"].clone());
                } else if frame["type"] == "error" {return Err(INVALID_RESPONSE)}
            }
            data.clear();
        } else if let Some(line) = line.strip_prefix("data:") {data.push_str(line.trim_start());data.push('\n');}
    }
    let canonical = terminal.ok_or(INVALID_RESPONSE)?;
    if target == router::ModelProtocol::Responses {Ok(canonical)} else {
        protocol::response_body(protocol::Translation {from:target,to:router::ModelProtocol::Responses},&canonical).map_err(|_| INVALID_RESPONSE)
    }
}

struct SearchAttempt {trace:route_trace::Trace,id:String,provider:String,name:String,model:String,protocol:router::ModelProtocol,
    credential:request_diagnostics::CredentialSnapshot,status:Option<u16>,error:Option<&'static str>,started:std::time::Instant,observation:Option<request_diagnostics::ServedModelObservation>}
impl SearchAttempt {
    fn new(trace:route_trace::Trace,id:&str,backend:&Backend)->Self {Self {trace,id:id.into(),provider:backend.provider_id.clone(),name:backend.provider_name.clone(),model:backend.model.clone(),protocol:backend.protocol,
        credential:backend.credential.clone(),status:None,error:Some("search_not_completed"),started:std::time::Instant::now(),observation:None}}
}
impl Drop for SearchAttempt {
    fn drop(&mut self) {self.trace.search_execution(&self.id,&self.provider,&self.name,&self.model,pipeline::request::protocol_name(self.protocol),self.status,
        self.error,self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,self.credential.clone(),self.observation.take());}
}
