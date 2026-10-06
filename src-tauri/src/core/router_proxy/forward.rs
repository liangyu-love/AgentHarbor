//! One attempt's protocol/history/credential preparation. Every attempt starts
//! with the immutable caller body; private member history never feeds a retry.
use super::*;

pub(super) struct Input<'a> {
    pub context: &'a ArcContext,
    pub store: &'a router::Store,
    pub catalog: &'a super::super::models_dev::Catalog,
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
}

pub(super) struct Prepared {
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
}

pub(super) fn prepare(input: Input<'_>) -> ForwardResult<Prepared> {
    let Input {
        context,
        store,
        catalog,
        destination,
        suffix,
        query,
        headers,
        body,
        ingress,
        resource,
        one_m,
        affinity_key,
        client_scope,
        identity,
        trace,
        selection_reason,
        retry_reason,
        attempt_id,
    } = input;
    let Destination::Provider {
        provider,
        model_override,
        upstream_protocol,
        translation,
    } = destination
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            "aggregate member requires a provider",
        ));
    };
    let model = model_override
        .clone()
        .unwrap_or_else(|| provider.model.clone());
    let (url, mut attempt_headers) = provider_target(
        provider,
        *upstream_protocol,
        *translation,
        suffix,
        query,
        headers.clone(),
    )?;
    if one_m && *upstream_protocol == router::ModelProtocol::AnthropicMessages {
        append_anthropic_beta(&mut attempt_headers);
    }
    let history = if ingress == router::ModelProtocol::AnthropicMessages && translation.is_some() {
        body.as_deref().and_then(|bytes| {
            protocol::message_history::Context::new_with_client_scope(
                &[
                    &context.store_path.to_string_lossy(),
                    &provider.id,
                    &url,
                    attempt_headers
                        .get(AUTHORIZATION)
                        .or_else(|| attempt_headers.get("x-api-key"))
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default(),
                    &model,
                    affinity_key.unwrap_or_default(),
                ],
                *upstream_protocol,
                bytes,
                affinity_key,
                client_scope.as_ref(),
            )
        })
    } else {
        None
    };
    let hosted = hosted_search::Plan::for_destination(destination, body.as_deref())?;
    let search_backend = hosted
        .as_ref()
        .map(|_| hosted_search::SearchBackend::select(context, store, destination))
        .transpose()?;
    let translation_body = if let Some(plan) = &hosted {
        Some(plan.normalized.clone())
    } else {
        hosted_search::Plan::history_only(destination, body.clone())?
    };
    let PreparedForward {
        body: mut sent,
        input,
        namespaces,
        response_options,
    } = if resource {
        PreparedForward::native(body)
    } else {
        upstream_body(destination, translation_body)?
    };
    if let (Some(history), Some(bytes)) = (&history, &sent) {
        let mut value: Value = serde_json::from_slice(bytes)
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid translated request"))?;
        history.restore(&mut value);
        sent = Some(Bytes::from(value.to_string()));
    }
    if ingress == router::ModelProtocol::AnthropicMessages && translation.is_some() {
        let official_snapshot = router::official_endpoint_applies(provider).then(|| {
            super::super::model_catalog::official(&context.codex_home, &context.app_data_dir)
        });
        let official = official_snapshot
            .as_ref()
            .and_then(|snapshot| router::matched_official_model(&snapshot.models, &model));
        let metadata = router::resolve_model_metadata(provider, &model, official, catalog);
        sent = session_compat::prepare_message_translation(sent, *upstream_protocol, &metadata)?;
    }
    let (body, tool_adapter) = if resource {
        (sent, Default::default())
    } else {
        tool_schema::prepare(sent, &model, *upstream_protocol)?
    };
    let client = client_for(context, destination)?;
    let credential = request_diagnostics::CredentialSnapshot::from_header(
        &context.app_data_dir,
        "providerApiKey",
        &format!("{}|{}", provider.id, url.split('?').next().unwrap_or(&url)),
        attempt_headers
            .get(AUTHORIZATION)
            .or_else(|| attempt_headers.get("x-api-key"))
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
        retry_reason: retry_reason,
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
        url,
        headers: attempt_headers,
        body,
        translation: *translation,
        protocol: *upstream_protocol,
        model,
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        input,
        history,
        tool_adapter,
        namespaces,
        response_options,
        usage,
        metadata,
        hosted,
        search_backend,
        credential,
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
    ) -> ForwardResult {
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
        )
        .await
    }

    pub async fn search(self, trace: route_trace::Trace) -> ForwardResult {
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
            provider_id: self.provider_id,
            provider_name: self.provider_name,
            model: self.model,
            history: self.history,
            adapter: self.tool_adapter,
            credential: self.credential,
        })
        .await
    }
}

pub(super) fn protocol_name(protocol: router::ModelProtocol) -> &'static str {
    match protocol {
        router::ModelProtocol::Responses => "responses",
        router::ModelProtocol::ChatCompletions => "chatCompletions",
        router::ModelProtocol::AnthropicMessages => "anthropicMessages",
    }
}
