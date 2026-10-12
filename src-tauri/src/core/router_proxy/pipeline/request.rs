//! One attempt's request pipeline: the stages in `STAGES` refine a draft in a
//! fixed order, then `meter` turns it into what is sent. Every attempt starts
//! from the immutable caller body; private member history never feeds a retry.
use super::super::*;

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
    pub exchange: Option<account_sources::cursor::Exchange>,
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
    exchange: Option<account_sources::cursor::Exchange>,
}

type Stage = fn(&Attempt<'_>, &mut Draft) -> ForwardResult<()>;

/// The request pipeline, in order (see `pipeline` for what each stage owns).
const STAGES: [(&str, Stage); 9] = [
    ("target", target),
    ("beta", beta),
    ("history", history),
    ("search", search),
    ("translate", translate),
    ("restore", restore),
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
    if input.ingress != router::ModelProtocol::AnthropicMessages || attempt.translation.is_none() {
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
    if input.suffix == "messages/count_tokens" {
        return Ok(());
    }
    draft.hosted = hosted_search::Plan::for_destination(input.destination, input.body.as_deref())?;
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

/// Remembered history goes back into the translated body.
fn restore(_attempt: &Attempt<'_>, draft: &mut Draft) -> ForwardResult<()> {
    if let (Some(history), Some(bytes)) = (&draft.history, &draft.body) {
        let mut value: Value = serde_json::from_slice(bytes)
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid translated request"))?;
        history.restore(&mut value);
        draft.body = Some(Bytes::from(value.to_string()));
    }
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
    let input = attempt.input;
    if let (Some(auth), Some(bytes), false) = (input.account, &draft.body, input.resource) {
        if let Some(exchange) = auth.exchange(attempt.upstream_protocol, bytes, &attempt.model).transpose()? {
            draft.url = format!("{}/agent.v1.AgentService/Run", auth.base.trim_end_matches('/'));
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
pub(in crate::core::router_proxy) fn prepare(input: Input<'_>) -> ForwardResult<Prepared> {
    let Destination::Provider { provider, model_override, upstream_protocol, translation } = input.destination else {
        return Err((StatusCode::BAD_REQUEST, "aggregate member requires a provider"));
    };
    let model = model_override.clone().unwrap_or_else(|| provider.model.clone());
    let mut draft = Draft::default();
    {
        let attempt = Attempt {
            input: &input,
            provider,
            upstream_protocol: *upstream_protocol,
            translation: *translation,
            model: model.clone(),
        };
        for (_, stage) in STAGES {
            stage(&attempt, &mut draft)?;
        }
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
    }

    pub async fn response(
        self,
        response: reqwest::Response,
        trace: route_trace::Trace,
        health: Option<crate::core::aggregation::runtime::Health>,
    ) -> ForwardResult {
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
        hosted_search::relay(hosted_search::Execution {
            plan: self.hosted.expect("hosted plan"),
            backend: self.search_backend.expect("validated search backend"),
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
            adapter: self.tool_adapter,
            credential: self.credential,
            health,
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
        // translation reads the search plan, restore and compat work on the
        // translated body, and the vendor shape is applied last.
        assert_eq!(STAGES.map(|(name, _)| name),
            ["target", "beta", "history", "search", "translate", "restore", "compat", "tools", "vendor"]);
    }
}
