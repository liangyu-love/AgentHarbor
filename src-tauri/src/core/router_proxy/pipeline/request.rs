//! One attempt's request pipeline: the stages in `STAGES` refine a draft in a
//! fixed order, then `meter` turns it into what is sent. Every attempt starts
//! from the immutable caller body; private member history never feeds a retry.
use super::super::*;

#[derive(Clone)]
pub(in crate::core::router_proxy) struct Input<'a> {
    pub context: &'a ArcContext,
    pub store: &'a router::Store,
    pub catalog: &'a crate::core::models_dev::Catalog,
    pub destination: &'a Destination,
    pub suffix: &'a str,
    pub query: Option<&'a str>,
    pub headers: &'a HeaderMap,
    pub body: Option<Bytes>,
    pub ingress: router::ModelProtocol,
    pub resource: bool,
    pub create: bool,
    pub one_m: bool,
    pub affinity_key: Option<&'a str>,
    pub client_scope: Option<protocol::message_history::ClientScope<'a>>,
    pub identity: request_diagnostics::RequestIdentity,
    pub trace: route_trace::Trace,
    pub selection_reason: String,
    pub retry_reason: Option<String>,
    pub attempt_id: String,
    /// The signed-in account an account source's attempt goes out as.
    pub account: Option<&'a account_sources::RequestAuth>,
}

pub(in crate::core::router_proxy) struct Prepared {
    pub client: Client,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub translation: Option<protocol::Translation>,
    pub protocol: router::ModelProtocol,
    pub model: String,
    pub provider_id: String,
    pub provider_name: String,
    pub input: Vec<Value>,
    pub history: Option<protocol::message_history::Context>,
    /// Reasoning a Chat upstream takes back, kept for a Responses caller.
    pub reasoning: Option<protocol::chat_reasoning::Context>,
    pub tool_adapter: tool_schema::Adapter,
    pub namespaces: protocol::tool_namespaces::ToolNamespaces,
    pub response_options: protocol::ResponseOptions,
    pub usage: Option<relay_usage::RelayUsageRequest>,
    pub metadata: route_trace::AttemptMetadata,
    pub hosted: Option<hosted_search::Plan>,
    pub search_backend: Option<hosted_search::SearchBackend>,
    pub credential: request_diagnostics::CredentialSnapshot,
    /// Set for an account source, whose refusals are explained to the client.
    pub account_source: Option<account_sources::SourceKind>,
    /// Turns a vendor wire's reply into Chat before anything reads it.
    pub wire: Option<account_sources::wire::Decoder>,
    /// Turns a source's reply into what the client takes (Grok's calls).
    pub reply_fix: Option<account_sources::ReplyFix>,
    /// A source that answers the request itself (Cursor's Runs).
    pub exchange: Option<account_sources::Exchange>,
    search_retry_allowed: bool,
    account: Option<account_sources::RequestAuth>,
    native_fallback: Option<Box<Prepared>>,
    native_recovery: Option<Box<Prepared>>,
    deferred_search: Option<hosted_search::Selection>,
    /// Physical target for learned search behavior (`hosted_search::Learned`).
    search_key: Option<String>,
    /// Diagnostics label overriding the one derived from the plan/options.
    search_label: Option<&'static str>,
}

/// One attempt's fixed facts, read by every stage.
struct Attempt<'a> {
    input: &'a Input<'a>,
    provider: &'a router::Provider,
    upstream_protocol: router::ModelProtocol,
    translation: Option<protocol::Translation>,
    /// The upstream model id this attempt asks for.
    model: String,
}

/// What the stages build for one attempt, refined in order.
#[derive(Default)]
struct Draft {
    url: String,
    headers: HeaderMap,
    history: Option<protocol::message_history::Context>,
    reasoning: Option<protocol::chat_reasoning::Context>,
    hosted: Option<hosted_search::Plan>,
    search_backend: Option<hosted_search::SearchBackend>,
    body: Option<Bytes>,
    input: Vec<Value>,
    namespaces: protocol::tool_namespaces::ToolNamespaces,
    response_options: protocol::ResponseOptions,
    tool_adapter: tool_schema::Adapter,
    /// Reads the reply of a source spoken through Chat (`account_sources::wire`).
    wire: Option<account_sources::wire::Decoder>,
    reply_fix: Option<account_sources::ReplyFix>,
    exchange: Option<account_sources::Exchange>,
    deferred_search: Option<hosted_search::Selection>,
}

type Stage = fn(&Attempt<'_>, &mut Draft) -> ForwardResult<()>;

/// The request pipeline, in order (see `pipeline` for what each stage owns).
const STAGES: [(&str, Stage); 10] = [
    ("target", target),
    ("beta", beta),
    ("history", history),
    ("search", search),
    ("translate", translate),
    ("restore", restore),
    ("thinking", thinking),
    ("compat", compat),
    ("tools", tools),
    ("vendor", vendor),
];

/// Upstream URL and credential headers: the account's for an account source,
/// the relay's own key otherwise.
fn target(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    let input = attempt.input;
    let (url, headers) = match input.account {
        Some(auth) => account_target(auth, attempt.upstream_protocol, attempt.translation, input.suffix, input.query, input.headers.clone())?,
        None if attempt.provider.router_only() => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "account source credentials were not resolved"));
        }
        None => provider_target(attempt.provider, attempt.upstream_protocol, attempt.translation, input.suffix, input.query, input.headers.clone())?,
    };
    draft.url = url;
    draft.headers = headers;
    Ok(())
}

/// Claude's 1M-context beta, when the client asked for the larger window.
fn beta(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    if attempt.input.one_m && attempt.upstream_protocol == router::ModelProtocol::AnthropicMessages {
        append_anthropic_beta(&mut draft.headers);
    }
    Ok(())
}

/// Messages ingress across protocols: remember the private history (thinking
/// signatures, tool ids) the target protocol cannot carry, per conversation.
fn history(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    let input = attempt.input;
    if input.ingress != router::ModelProtocol::AnthropicMessages || (attempt.translation.is_none() && !hosted_search::Plan::selected(input.store)) {
        return Ok(());
    }
    draft.history = input.body.as_deref().and_then(|bytes| {
        protocol::message_history::Context::new_with_client_scope(
            &[
                &input.context.store_path.to_string_lossy(),
                &attempt.provider.id,
                &draft.url,
                draft.headers
                    .get(AUTHORIZATION)
                    .or_else(|| draft.headers.get("x-api-key"))
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default(),
                &attempt.model,
                input.affinity_key.unwrap_or_default(),
            ],
            attempt.upstream_protocol,
            bytes,
            input.affinity_key,
            input.client_scope.as_ref(),
        )
    });
    Ok(())
}

/// A hosted WebSearch call the upstream cannot run is planned here, with the
/// backend that will run it.
fn search(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    let input = attempt.input;
    if !input.create || input.resource || !matches!(input.suffix,"responses" | "messages" | "chat/completions") {
        return Ok(());
    }
    if let Some(plan) = hosted_search::Plan::delegated(input.destination,input.store,input.body.as_deref())? {
        let selection = hosted_search::Selection::new(input.context,input.store,input.catalog,&input.trace,input.identity.clone())
            .and_then(|selection| {selection.validate(&plan)?;Ok(selection)});
        match selection {
            Ok(selection) => draft.deferred_search=Some(selection),
            Err(_) if !plan.required() && attempt.provider.chat_search != protocol::ChatSearch::Selected => return Ok(()),
            Err(error) => return Err(error),
        }
        draft.hosted = Some(plan);
        return Ok(());
    }
    if attempt.provider.chat_search == protocol::ChatSearch::Selected {
        let caller = attempt.translation.map_or(attempt.upstream_protocol,|pair| pair.from);
        if hosted_search::Plan::native(caller,input.body.as_deref())?.is_some() {
            return Err((StatusCode::BAD_REQUEST,"select a Responses or Messages WebSearch service and model"));
        }
    }
    let command_selected = hosted_search::Plan::command_enabled(attempt.provider,input.store);
    draft.hosted = if command_selected {
        hosted_search::Plan::for_destination(input.destination, input.body.as_deref())?
    } else { None };
    if attempt.provider.chat_search == protocol::ChatSearch::Command && draft.hosted.is_none()
        && input.body.as_deref().and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok()).is_some_and(|value|
            value["tool_choice"] != "none" && value["tool_choice"]["type"] != "none"
                && value["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search))) {
        return Err((StatusCode::BAD_REQUEST,"selected Command search requires a verified Chat endpoint for this model"));
    }
    draft.search_backend = draft.hosted.as_ref()
        .map(|_| hosted_search::SearchBackend::select(input.context, input.store, input.destination))
        .transpose()?;
    Ok(())
}

/// The body in the upstream's protocol; resource reads and token counts keep
/// their native body.
fn translate(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    let input = attempt.input;
    let prepared = if input.resource {
        let native = if input.suffix == "messages/count_tokens" {
            input.body.as_ref()
                .map(|bytes| prepare_relay_body(bytes, Some(&attempt.model), attempt.upstream_protocol))
                .transpose()?
        } else {
            input.body.clone()
        };
        PreparedForward::native(native)
    } else {
        let source = match &draft.hosted {
            Some(plan) => Some(plan.normalized.clone()),
            None => hosted_search::Plan::history_only(input.destination, input.body.clone())?,
        };
        upstream_body(input.destination, source)?
    };
    draft.body = prepared.body;
    draft.input = prepared.input;
    draft.namespaces = prepared.namespaces;
    draft.response_options = prepared.response_options;
    Ok(())
}

/// Remembered history goes back into the translated body: a Messages
/// caller's private turns, or the reasoning a Chat upstream wants from a
/// Responses caller that cannot replay it.
fn restore(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    if let (Some(history), Some(bytes)) = (&draft.history, &draft.body) {
        let mut value: Value = serde_json::from_slice(bytes)
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid translated request"))?;
        history.restore(&mut value);
        draft.body = Some(Bytes::from(value.to_string()));
    }
    let input = attempt.input;
    if input.ingress == router::ModelProtocol::Responses && !input.resource && attempt.translation.is_some()
        && attempt.upstream_protocol == router::ModelProtocol::ChatCompletions {
        if let Some(bytes) = &draft.body {
            let mut value: Value = serde_json::from_slice(bytes)
                .map_err(|_| (StatusCode::BAD_REQUEST, "invalid translated request"))?;
            let store = input.context.store_path.to_string_lossy();
            let scope = [store.as_ref(), attempt.provider.id.as_str(), draft.url.as_str(), attempt.model.as_str()];
            if let Some(reasoning) = protocol::chat_reasoning::Context::new(&scope, &value) {
                if reasoning.restore(&mut value) {
                    draft.body = Some(Bytes::from(value.to_string()));
                }
                draft.reasoning = Some(reasoning);
            }
        }
    }
    Ok(())
}

/// A Chat model whose thinking is a switch: the effort the request carries
/// becomes its upstream's on/off field.
fn thinking(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    if attempt.upstream_protocol != router::ModelProtocol::ChatCompletions || attempt.input.resource {
        return Ok(());
    }
    let input = attempt.input;
    draft.body = session_compat::prepare_thinking_switch(draft.body.take(), || {
        let official_snapshot = router::official_endpoint_applies(attempt.provider).then(|| {
            crate::core::model_catalog::official(&input.context.codex_home, &input.context.app_data_dir)
        });
        let official = official_snapshot
            .as_ref()
            .and_then(|snapshot| router::matched_official_model(&snapshot.models, &attempt.model));
        router::resolve_model_metadata(attempt.provider, &attempt.model, official, input.catalog)
    }, &attempt.provider.base_url)?;
    Ok(())
}

/// Messages ingress across protocols: reasoning and output limits the target
/// model takes.
fn compat(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    let input = attempt.input;
    if input.ingress != router::ModelProtocol::AnthropicMessages || attempt.translation.is_none() {
        return Ok(());
    }
    let official_snapshot = router::official_endpoint_applies(attempt.provider).then(|| {
        crate::core::model_catalog::official(&input.context.codex_home, &input.context.app_data_dir)
    });
    let official = official_snapshot
        .as_ref()
        .and_then(|snapshot| router::matched_official_model(&snapshot.models, &attempt.model));
    let metadata = router::resolve_model_metadata(attempt.provider, &attempt.model, official, input.catalog);
    draft.body = session_compat::prepare_message_translation(draft.body.take(), attempt.upstream_protocol, &metadata)?;
    Ok(())
}

/// Tool schemas adapted to what the model accepts.
fn tools(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    if attempt.input.resource {
        return Ok(());
    }
    let (body, adapter) = tool_schema::prepare(draft.body.take(), &attempt.model, attempt.upstream_protocol)?;
    draft.body = body;
    draft.tool_adapter = adapter;
    Ok(())
}

/// An account source's own request shape: headers that depend on the body
/// (who started a Copilot turn), WorkBuddy's system message, the fields a
/// ChatGPT sign-in refuses.
fn vendor(attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    // Hosted loops apply the source envelope/decoder freshly on every round.
    if draft.hosted.is_some() {return Ok(())}
    let input = attempt.input;
    if let (Some(auth), Some(bytes), false) = (input.account, &draft.body, input.resource) {
        if let Some(exchange) = auth.exchange(attempt.upstream_protocol, bytes, &attempt.model).transpose()? {
            draft.url = exchange.url(&auth.base);
            draft.exchange = Some(exchange);
            return Ok(());
        }
        if let Some(encoded) = auth.encode(attempt.upstream_protocol, bytes, &attempt.model).transpose()? {
            draft.url = format!("{}/{}", auth.base.trim_end_matches('/'), encoded.path);
            for (name, value) in encoded.headers {
                let value = HeaderValue::from_str(&value)
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid account source header"))?;
                draft.headers.insert(hyper::header::HeaderName::from_static(name), value);
            }
            draft.headers.remove(hyper::header::CONTENT_LENGTH);
            draft.body = Some(Bytes::from(encoded.body));
            draft.wire = Some(encoded.decoder);
            return Ok(());
        }
        if let Some(rerouted) = auth.reroute(attempt.upstream_protocol, bytes).transpose()? {
            if let Some(rest) = draft.url.strip_prefix(auth.base.trim_end_matches('/')) {
                draft.url = format!("{}{rest}", rerouted.base.trim_end_matches('/'));
            }
            for (name, _) in &auth.headers {
                draft.headers.remove(*name);
            }
            for (name, value) in rerouted.headers {
                let value = HeaderValue::from_str(&value)
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid account source header"))?;
                draft.headers.insert(hyper::header::HeaderName::from_static(name), value);
            }
            draft.headers.remove(hyper::header::CONTENT_LENGTH);
            draft.body = Some(Bytes::from(rerouted.body));
            draft.reply_fix = rerouted.fix;
            return Ok(());
        }
        draft.reply_fix = auth.reply_fix(attempt.upstream_protocol, bytes);
        for (name, value) in auth.body_headers(attempt.upstream_protocol, bytes) {
            let value = HeaderValue::from_str(&value)
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid account source header"))?;
            draft.headers.insert(hyper::header::HeaderName::from_static(name), value);
        }
        if let Some(shaped) = auth.shape_body(attempt.upstream_protocol, bytes) {
            draft.body = Some(Bytes::from(shaped));
        }
    }
    Ok(())
}

/// Run every request stage for one attempt, then meter it: the outbound
/// client, the credential snapshot, the price and the usage record.
pub(in crate::core::router_proxy) async fn prepare(input: Input<'_>) -> ForwardResult<Prepared> {
    let mut prepared = build(input.clone(),None,None).await?;
    if prepared.hosted.is_none() && input.create && !input.resource && hosted_search::Plan::selected(input.store)
        && matches!(input.suffix,"responses" | "messages" | "chat/completions") {
        let Destination::Provider {provider,upstream_protocol,translation,..} = input.destination else {return Ok(prepared)};
        if provider.chat_search != protocol::ChatSearch::Disabled {
            let caller = translation.map_or(*upstream_protocol,|pair| pair.from);
            if let Ok(Some(plan)) = hosted_search::Plan::native(caller,input.body.as_deref()) {
                let mut fallback_input = input.clone();
                fallback_input.attempt_id = uuid::Uuid::new_v4().to_string();
                // An unavailable optional fallback must not prevent a supported
                // native request. It is never sent unless the tool is rejected.
                if let Ok(fallback) = build(fallback_input,Some(plan),None).await {prepared.native_fallback = Some(Box::new(fallback));}
            }
        }
    }
    if prepared.hosted.is_none() && input.create && !input.resource && prepared.wire.is_none()
        && !prepared.response_options.search_downgraded && matches!(input.suffix,"responses" | "messages") {
        if let Destination::Provider {provider,upstream_protocol,translation,..} = input.destination {
            if !matches!(provider.chat_search,protocol::ChatSearch::Disabled | protocol::ChatSearch::Command)
                && matches!(upstream_protocol,router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages)
                && input.body.as_deref().and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok()).is_some_and(|body|
                    body["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| !protocol::web_search::is_search(tool)))) {
                let caller = translation.map_or(*upstream_protocol,|pair| pair.from);
                if let Ok(Some(plan)) = hosted_search::Plan::native(caller,input.body.as_deref()) {
                    if let Ok(selection) = hosted_search::Selection::for_destination(input.context,input.destination,input.catalog,
                        &input.trace,input.identity.clone(),input.account).and_then(|selection| {selection.validate(&plan)?;Ok(selection)}) {
                        let mut retry = input.clone();
                        retry.attempt_id = uuid::Uuid::new_v4().to_string();
                        retry.retry_reason = Some("native_search_tool_misclassified".into());
                        if let Ok(recovery) = Box::pin(build(retry,Some(plan),Some(selection))).await {prepared.native_recovery=Some(Box::new(recovery));}
                    }
                }
            }
        }
    }
    prepared.search_key = hosted_search::target_key(input.destination);
    Ok(apply_learned(prepared))
}

/// Skip a first attempt this target already failed the same way: run the
/// selected search service, omit the optional declaration, or search
/// separately from client functions on the same account and model.
fn apply_learned(mut prepared: Prepared) -> Prepared {
    let Some(key) = prepared.search_key.clone() else { return prepared };
    if prepared.hosted.is_some() || !prepared.search_retry_allowed { return prepared; }
    if hosted_search::learned(&key, hosted_search::Learned::RejectsSearch) {
        if let Some(mut fallback) = prepared.native_fallback.take() {
            fallback.search_key = Some(key);
            return *fallback;
        }
        if !prepared.response_options.search_required && prepared.wire.is_none() && prepared.exchange.is_none() {
            if let Some(stripped) = prepared.body.as_deref().and_then(super::translate::optional_native_search) {
                prepared.body = Some(stripped);
                prepared.response_options.search = None;
                prepared.response_options.search_downgraded = true;
                prepared.native_recovery = None;
            }
        }
        return prepared;
    }
    if hosted_search::learned(&key, hosted_search::Learned::SeparateSearch) {
        if let Some(mut recovery) = prepared.native_recovery.take() {
            recovery.metadata.retry_reason = None;
            recovery.search_key = Some(key);
            recovery.search_label = Some("native_search_separate");
            return *recovery;
        }
    }
    prepared
}

async fn build(input: Input<'_>, mut forced: Option<hosted_search::Plan>, selection: Option<hosted_search::Selection>) -> ForwardResult<Prepared> {
    let Destination::Provider { provider, model_override, upstream_protocol, translation } = input.destination else {
        return Err((StatusCode::BAD_REQUEST, "aggregate member requires a provider"));
    };
    let model = model_override.clone().unwrap_or_else(|| provider.model.clone());
    let mut draft = Draft::default();
    draft.deferred_search = selection;
    {
        let attempt = Attempt {
            input: &input,
            provider,
            upstream_protocol: *upstream_protocol,
            translation: *translation,
            model: model.clone(),
        };
        for (name, stage) in STAGES {
            if name == "search" && forced.is_some() {draft.hosted = forced.take();}
            else {stage(&attempt, &mut draft)?;}
        }
    }
    if draft.hosted.as_ref().is_some_and(|plan| plan.is_native()) && draft.deferred_search.is_none() {
        let selection = hosted_search::Selection::new(input.context,input.store,input.catalog,&input.trace,input.identity.clone())?;
        selection.validate(draft.hosted.as_ref().expect("native plan"))?;
        draft.deferred_search = Some(selection);
    }
    meter(input, provider, *upstream_protocol, *translation, model, draft)
}

fn meter(
    input: Input<'_>,
    provider: &router::Provider,
    upstream_protocol: router::ModelProtocol,
    translation: Option<protocol::Translation>,
    model: String,
    draft: Draft,
) -> ForwardResult<Prepared> {
    let search_retry_allowed = !input.resource && matches!(input.suffix,"responses" | "messages" | "chat/completions");
    let Input { context, catalog, destination, suffix, ingress, identity, trace, selection_reason, retry_reason, attempt_id, account, .. } = input;
    let client = client_for(context, destination)?;
    let credential = request_diagnostics::CredentialSnapshot::from_header(
        &context.app_data_dir,
        if account.is_some() { "accountSource" } else { "providerApiKey" },
        &format!("{}|{}", provider.id, draft.url.split('?').next().unwrap_or(&draft.url)),
        draft.headers
            .get(AUTHORIZATION)
            .or_else(|| draft.headers.get("x-api-key"))
            .and_then(|value| value.to_str().ok()),
        None,
    );
    let pricing = router::pricing_for(provider, &model, catalog);
    let metadata = route_trace::AttemptMetadata {
        attempt_id: attempt_id.clone(),
        catalog_revision: pricing.revision.clone(),
        match_kind: pricing.match_kind.clone(),
        pricing_status: pricing.status().into(),
        pricing_source: pricing.source.clone(),
        selection_reason,
        retry_reason,
    };
    // Only inference creates produce a billable usage record. Token counting,
    // resource reads and mutations reuse an existing response/connection and
    // may repeat the original response usage in their body.
    let usage = is_billable_inference_suffix(suffix)
        .then(|| relay_usage_request(context, destination, Some(ingress), Some(&model), pricing))
        .flatten()
        .map(|request| {
            request
                .with_request_id(trace.request_id())
                .with_attempt_id(&attempt_id)
                .with_identity(identity)
                .with_credential(credential.clone())
        });
    Ok(Prepared {
        client,
        url: draft.url,
        headers: draft.headers,
        body: draft.body,
        translation,
        protocol: upstream_protocol,
        model,
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        input: draft.input,
        history: draft.history,
        reasoning: draft.reasoning,
        tool_adapter: draft.tool_adapter,
        namespaces: draft.namespaces,
        response_options: draft.response_options,
        usage,
        metadata,
        hosted: draft.hosted,
        search_backend: draft.search_backend,
        credential,
        account_source: account.map(|auth| auth.kind),
        wire: draft.wire,
        reply_fix: draft.reply_fix,
        exchange: draft.exchange,
        search_retry_allowed,
        account: account.cloned(),
        native_fallback: None,
        native_recovery: None,
        deferred_search: draft.deferred_search,
        search_key: None,
        search_label: None,
    })
}

fn is_billable_inference_suffix(suffix: &str) -> bool {
    matches!(
        suffix,
        "responses" | "responses/compact" | "messages" | "chat/completions"
    )
}

impl Prepared {
    pub fn sent(&self, trace: &route_trace::Trace) {
        trace.attempt(
            &self.provider_id,
            &self.provider_name,
            &self.model,
            protocol_name(self.protocol),
            self.metadata.clone(),
        );
        trace.sent(self.body.as_deref(), self.credential.clone());
        if let Some(label) = self.search_label {
            trace.search_compatibility(label);
        } else if self.response_options.search_downgraded {
            trace.search_compatibility("optional_search_omitted");
        } else if self.hosted.is_some() {
            trace.search_compatibility(if self.hosted.as_ref().is_some_and(|plan| plan.is_native()) {"selected_search"} else {"command_search"});
        } else if self.response_options.search.is_some() {
            trace.search_compatibility(if self.response_options.search_budget_approximate {"native_search_budget_approx"}
                else if self.response_options.search_cached_as_live {"native_search_live"} else {"native_search"});
        }
    }

    pub async fn response(
        mut self,
        mut response: reqwest::Response,
        method: hyper::Method,
        trace: route_trace::Trace,
        health: Option<crate::core::aggregation::runtime::Health>,
    ) -> ForwardResult {
        if method == hyper::Method::POST && self.native_recovery.is_some()
            && matches!(response.status(),StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY) {
            let status = response.status();
            let headers = response.headers().clone();
            let mut metered = self.usage.take().map(|usage| usage.observe(status.as_u16(),false));
            let bytes = tokio::time::timeout(Duration::from_secs(90),hosted_search::limited(response)).await
                .map_err(|_| (StatusCode::GATEWAY_TIMEOUT,"native search error response timed out"))??;
            if super::translate::rejected_mixed_search(status,&bytes) {
                let mut recovery = self.native_recovery.take().expect("prepared same-provider recovery");
                if let Some(key) = &self.search_key { hosted_search::learn(key, hosted_search::Learned::SeparateSearch); }
                let reason = "native_search_mixed_tools";
                recovery.metadata.retry_reason = Some(reason.into());
                trace.status(status.as_u16());
                trace.failed_attempt("response",reason,0);
                trace.attempt_elapsed();
                if let Some(mut rejected) = metered.take() {rejected.observe(&bytes);}
                trace.native_search_recovery(&recovery.metadata.attempt_id,reason);
                if recovery.headers.contains_key("idempotency-key") {
                    recovery.headers.insert("idempotency-key",HeaderValue::from_str(&recovery.metadata.attempt_id).expect("UUID header"));
                }
                trace.sent(recovery.body.as_deref(),recovery.credential.clone());
                trace.search_compatibility("native_search_separate");
                return Box::pin(recovery.search(trace,health)).await;
            }
            self.usage = metered.and_then(relay_usage::RelayUsageObserver::resume_request);
            let mut restored = hyper::Response::builder().status(status);
            for (name,value) in &headers {
                if name != CONTENT_LENGTH && name != TRANSFER_ENCODING {restored=restored.header(name,value);}
            }
            response = reqwest::Response::from(restored.body(bytes).expect("valid response parts"));
        }
        // A successful reply streams through untouched. A misclassified hosted
        // call is learned for the next request instead of buffering this one.
        if response.status().is_success() && method == hyper::Method::POST {
            if let (Some(plan), Some(key)) = (self.native_recovery.take().and_then(|mut recovery| recovery.hosted.take()), self.search_key.clone()) {
                response = hosted_search::watch_misclassified(response, plan, self.protocol, key, trace.clone());
            }
        }
        if self.search_retry_allowed && method == hyper::Method::POST && self.wire.is_none() && (self.native_fallback.is_some() || !self.response_options.search_required)
            && matches!(response.status(), StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY) {
            let stripped = self.body.as_deref().and_then(super::translate::optional_native_search);
            if stripped.is_some() || self.native_fallback.is_some() {
                let status = response.status();
                let headers = response.headers().clone();
                let mut bytes = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| (StatusCode::BAD_GATEWAY, "cannot read provider response"))?;
                    if bytes.len() + chunk.len() > 256 * 1024 {
                        return Err((StatusCode::BAD_GATEWAY, "provider error exceeded the response limit"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if super::translate::rejected_native_search(status, &bytes) {
                    if let Some(key) = &self.search_key { hosted_search::learn(key, hosted_search::Learned::RejectsSearch); }
                    trace.status(status.as_u16());
                    trace.failed_attempt("response", "optional_search_unsupported", 0);
                    trace.attempt_elapsed();
                    let attempt_id = self.native_fallback.as_ref().map(|fallback| fallback.metadata.attempt_id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    if let Some(usage) = self.usage.take() {
                        let next = usage.followup(&attempt_id);
                        let mut rejected = usage.observe(status.as_u16(),false);
                        rejected.observe(&bytes);
                        self.usage = Some(next);
                    }
                    if let Some(mut fallback) = self.native_fallback.take() {
                        fallback.usage = self.usage.take();
                        trace.search_retry(&fallback.metadata.attempt_id);
                        if fallback.headers.contains_key("idempotency-key") {fallback.headers.insert("idempotency-key",HeaderValue::from_str(&fallback.metadata.attempt_id).expect("UUID header"));}
                        trace.sent(fallback.body.as_deref(),fallback.credential.clone());
                        trace.search_compatibility("selected_search");
                        return fallback.search(trace,health).await;
                    }
                    let stripped = stripped.expect("optional search or validated fallback");
                    trace.search_retry(&attempt_id);
                    self.metadata.attempt_id = attempt_id;
                    if self.headers.contains_key("idempotency-key") {
                        self.headers.insert("idempotency-key",HeaderValue::from_str(&self.metadata.attempt_id).expect("UUID header"));
                    }
                    self.body = Some(stripped.clone());
                    self.response_options.search = None;
                    self.response_options.search_downgraded = true;
                    trace.sent(Some(&stripped), self.credential.clone());
                    trace.search_compatibility("optional_search_omitted");
                    response = self.client.request(method.clone(),&self.url).headers(self.headers.clone()).body(stripped).send().await
                        .map_err(|_| (StatusCode::BAD_GATEWAY, "cannot reach provider after omitting unsupported search"))?;
                } else {
                    let mut restored = hyper::Response::builder().status(status);
                    for (name, value) in &headers {
                        if name != CONTENT_LENGTH && name != TRANSFER_ENCODING { restored = restored.header(name, value); }
                    }
                    response = reqwest::Response::from(restored.body(bytes).expect("valid response parts"));
                }
            }
        }
        let response = match self.account_source {
            Some(kind) if !response.status().is_success() => explain_account_refusal(kind, response).await.0,
            _ => response,
        };
        let response = match self.reply_fix {
            Some(fix) => fix.apply(response).await,
            None => response,
        };
        // A vendor wire's reply is Chat from here on.
        let response = match self.wire {
            Some(decoder) => decoder.decode(response).await,
            None => response,
        };
        relay_response(
            response,
            self.translation,
            self.usage,
            self.input,
            trace,
            self.protocol,
            self.model,
            self.metadata.attempt_id,
            self.history,
            self.reasoning,
            self.tool_adapter,
            self.namespaces,
            self.response_options,
            health,
        )
        .await
    }

    pub async fn search(
        self,
        trace: route_trace::Trace,
        health: Option<crate::core::aggregation::runtime::Health>,
    ) -> ForwardResult {
        let backend = self.search_backend.unwrap_or_else(|| hosted_search::SearchBackend {
            client:self.client.clone(),endpoint:String::new(),headers:HeaderMap::new(),native:None,api:None,
        });
        hosted_search::relay(hosted_search::Execution {
            plan: self.hosted.expect("hosted plan"),
            backend,
            client: self.client,
            url: self.url,
            headers: self.headers,
            body: self
                .body
                .ok_or((StatusCode::BAD_REQUEST, "WebSearch requires a body"))?,
            trace,
            usage: self.usage,
            attempt: self.metadata,
            model: self.model,
            history: self.history,
            input: self.input,
            namespaces: self.namespaces,
            adapter: self.tool_adapter,
            credential: self.credential,
            health,
            protocol: self.protocol,
            account: self.account,
            selection: self.deferred_search,
        })
        .await
    }

}

pub(in crate::core::router_proxy) fn protocol_name(protocol: router::ModelProtocol) -> &'static str {
    match protocol {
        router::ModelProtocol::Responses => "responses",
        router::ModelProtocol::ChatCompletions => "chatCompletions",
        router::ModelProtocol::AnthropicMessages => "anthropicMessages",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_run_in_the_documented_order() {
        // Order matters: history needs the target's URL and credential,
        // translation reads the search plan, restore, thinking and compat
        // work on the translated body, and the vendor shape is applied last.
        assert_eq!(STAGES.map(|(name, _)| name),
            ["target", "beta", "history", "search", "translate", "restore", "thinking", "compat", "tools", "vendor"]);
    }
}
