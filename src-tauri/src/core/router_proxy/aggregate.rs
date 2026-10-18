//! Aggregate HTTP execution reuses the existing transport, protocol adapters,
//! history and accounting. Selection is independent of client injection.
use super::super::aggregation::{
    self as agg, affinity,
    execution::{Attempts, Failure, PlannedMember, Requirements},
};
use super::*;
use std::collections::HashSet;

pub(super) struct DesktopTarget {
    pub aggregate_id: String,
    pub alias: String,
    pub config_identity: String,
    pub route_revision: u64,
    pub one_m: bool,
}

pub(super) fn catalog(
    store: &router::Store,
    catalog: &super::super::models_dev::Catalog,
    anthropic: bool,
) -> Value {
    let entries: Vec<_> = store.aggregates.iter().filter_map(|aggregate| {
        let view = agg::project(store, aggregate, catalog);
        if !view.available { return None; }
        let projection = view.projection?;
        Some(if anthropic {
            json!({"id":aggregate.public_model_id,"type":"model","display_name":aggregate.display_name,
                "created_at":"2026-10-06T00:00:00Z"})
        } else {
            let mut entry = router::aggregate_catalog_entry(aggregate, &projection);
            entry["id"] = json!(aggregate.public_model_id);
            entry["object"] = json!("model");
            entry
        })
    }).collect();
    if anthropic {
        json!({"data":entries,"has_more":false,"first_id":entries.first().map(|entry| &entry["id"]),"last_id":entries.last().map(|entry| &entry["id"])})
    } else {
        json!({"object":"list","data":entries})
    }
}

pub(super) async fn execute(
    context: &ArcContext,
    store: &router::Store,
    catalog: &super::super::models_dev::Catalog,
    method: &hyper::Method,
    segments: &[&str],
    query: Option<&str>,
    headers: &HeaderMap,
    original: Option<Bytes>,
    ingress: router::ModelProtocol,
    requested: Option<&str>,
    one_m: bool,
    desktop: Option<DesktopTarget>,
    tagged_source: Option<&str>,
    trace: route_trace::Trace,
) -> ForwardResult {
    let resource_id = match segments {
        ["aggregate", "v1", "responses", id] if *id != "compact" => Some(*id),
        ["aggregate", "v1", "responses", id, "cancel" | "input_items"] => Some(*id),
        _ => None,
    };
    let suffix = match segments {
        ["aggregate", "v1", rest @ ..] | ["claude-desktop", "v1", rest @ ..] => rest.join("/"),
        _ => return Err((StatusCode::NOT_FOUND, "unknown aggregate endpoint")),
    };
    let compact = suffix == "responses/compact";
    let count_tokens = suffix == "messages/count_tokens";
    let create = matches!(
        suffix.as_str(),
        "responses" | "messages" | "chat/completions"
    );
    let header_source = headers
        .get("x-agentharbor-client")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
        });
    let client = if desktop.is_some() {
        "claudeDesktop"
    } else {
        tagged_source.or(header_source).unwrap_or("aggregate")
    };
    let config = desktop
        .as_ref()
        .map(|desktop| desktop.config_identity.as_str())
        .unwrap_or("shared");
    let resource_scope = affinity::scope(&[&context.store_path.to_string_lossy(), client, config]);
    // Native resource operations may carry no body (cancel/delete), unlike
    // inference creates. Preserve their empty bytes instead of requiring JSON.
    let request_json = original
        .as_deref()
        .filter(|bytes| resource_id.is_none() || !bytes.is_empty());
    let value: Value = request_json
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid aggregate request"))?
        .unwrap_or_else(|| json!({}));
    if value
        .get("conversation")
        .is_some_and(|value| !value.is_null())
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "aggregate conversation resources are not supported",
        ));
    }
    let previous = value["previous_response_id"].as_str();
    let resource_owner = resource_id
        .or(previous)
        .map(|id| affinity::shared().resource(&resource_scope, id))
        .transpose()
        .map_err(|_| {
            (
                StatusCode::CONFLICT,
                "response resource owner is unknown, expired or ambiguous",
            )
        })?;
    let aggregate = if let Some(desktop) = &desktop {
        store
            .aggregates
            .iter()
            .find(|aggregate| aggregate.id == desktop.aggregate_id)
    } else if let Some(owner) = &resource_owner {
        let aggregate = store
            .aggregates
            .iter()
            .find(|aggregate| aggregate.id == owner.aggregate_id);
        if requested.is_some_and(|model| {
            aggregate.is_none_or(|aggregate| aggregate.public_model_id != model)
        }) {
            return Err((
                StatusCode::BAD_REQUEST,
                "response resource belongs to a different aggregate",
            ));
        }
        aggregate
    } else {
        store
            .aggregates
            .iter()
            .find(|aggregate| Some(aggregate.public_model_id.as_str()) == requested)
    }
    .ok_or((StatusCode::NOT_FOUND, "unknown aggregate model"))?;
    if !aggregate.enabled {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "aggregate model is disabled",
        ));
    }
    let role = desktop
        .as_ref()
        .map(|desktop| desktop.alias.as_str())
        .unwrap_or_default();
    let one_m = desktop
        .as_ref()
        .map(|desktop| desktop.one_m)
        .unwrap_or(one_m);
    let requirements = Requirements::parse(ingress, &value, one_m)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid aggregate request"))?;
    // Token counting chooses a compatible endpoint without establishing a
    // conversation's inference member before the first actual create.
    let conversation_scope = affinity::scope(&[
        &resource_scope, &aggregate.id, &aggregate.public_model_id, role,
    ]);
    let session = (!count_tokens).then(|| aggregate_session::explicit(headers, &value)).flatten();
    let transcript_namespace = affinity::scope(&[&conversation_scope, "messages-transcript"]);
    let fingerprint = (create && ingress == router::ModelProtocol::AnthropicMessages && session.is_none())
        .then(|| aggregate_session::Fingerprint::request(&value)).flatten();
    let session_key = if let Some(session) = &session {
        Some(affinity::scope(&[&conversation_scope, "explicit", session]))
    } else if let Some((_, previous)) = &fingerprint {
        Some(if let Some(previous) = previous {
            affinity::shared().transcript(&affinity::scope(&[&transcript_namespace, previous]))
                .map_err(|_| (StatusCode::CONFLICT,
                    "conversation member is unknown, expired or ambiguous; start a new conversation"))?
        } else {
            affinity::scope(&[&conversation_scope, "new-transcript", &uuid::Uuid::new_v4().to_string()])
        })
    } else {
        None
    };
    let decision = if let Some(key) = &session_key {
        Some(affinity::decision_lock(key).await)
    } else {
        None
    };
    let binding = session_key
        .as_deref()
        .and_then(|key| affinity::shared().binding(key));
    let bound = resource_owner.as_ref().or(binding.as_ref().filter(|_| {
        aggregate.affinity == agg::Affinity::Session
            || requirements.continuation
            || requirements.private_history
    }));
    if bound.is_none()
        && (resource_id.is_some()
            || previous.is_some()
            || (!count_tokens && (requirements.private_history
                || (requirements.continuation
                    && (session_key.is_some() || ingress == router::ModelProtocol::AnthropicMessages)))))
    {
        return Err((
            StatusCode::CONFLICT,
            "conversation member is unknown or expired; start a new conversation",
        ));
    }
    if resource_id.is_some()
        && resource_owner
            .as_ref()
            .is_some_and(|owner| !owner.native_resource)
    {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "this response belongs to local continuation history",
        ));
    }
    let mut plans = Vec::new();
    // Search contracts belong to the actual member, including account sources.
    // It never restores any candidate's private cache or touches network state.
    let mut preflight = std::collections::HashMap::new();
    let mut last_rejection = None;
    for member in &aggregate.members {
        let resolved = match agg::resolve_member(store, aggregate, member, catalog) {
            Ok(resolved) => resolved,
            Err(reason) => {
                trace.skip_member(&member.id, &reason);
                continue;
            }
        };
        let bound_chat = bound.is_some_and(|owner| {
            owner.member_id == member.id
                && owner.connection_key == resolved.target_key
                && owner.effective_protocol == router::ModelProtocol::ChatCompletions
                && matches!(owner.configured_protocol,router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages)
        });
        // A Command search turn can choose Chat for a configured Responses
        // member. Keep that exact effective protocol for subsequent history,
        // even when the next request no longer declares the search tool.
        let command_selected = hosted_search::Plan::command_enabled(&resolved.provider,store);
        let compatibility = matches!(ingress,router::ModelProtocol::AnthropicMessages | router::ModelProtocol::Responses)
            && ((requirements.search && command_selected) || bound_chat)
            && matches!(resolved.protocol,router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages)
            && router::supports_command_search(&resolved.provider)
            && resolved.provider.supports_model_protocol(
                &member.actual_model_id,
                router::ModelProtocol::ChatCompletions,
            );
        let effective = if compatibility {
            router::ModelProtocol::ChatCompletions
        } else {
            resolved.protocol
        };
        if count_tokens && effective != router::ModelProtocol::AnthropicMessages {
            last_rejection = Some("count_tokens_requires_messages");
            trace.skip_member(&member.id, "count_tokens_requires_messages");
            continue;
        }
        let mut member_requirements = requirements.clone();
        let probe_destination = Destination::Provider {provider:Box::new(resolved.provider.clone()),model_override:Some(member.actual_model_id.clone()),
            upstream_protocol:effective,translation:protocol::Translation::between(ingress,effective)};
        let delegated = if requirements.search && !count_tokens {
            match hosted_search::Plan::delegated(&probe_destination,store,original.as_deref()) {
                Ok(plan) => plan.map(|plan| plan.normalized),
                Err(_) => {last_rejection=Some("request_not_representable");trace.skip_member(&member.id,"request_not_representable");continue;}
            }
        } else {None};
        // Function-calling metadata does not establish hosted search support.
        // A locally executed Command search does need function calling.
        if value["tools"].as_array().is_some_and(|tools| !tools.is_empty() && tools.iter().all(protocol::web_search::is_search)) {
            member_requirements.tools = (command_selected || delegated.is_some()) && value["tool_choice"] != "none" && value["tool_choice"]["type"] != "none";
        }
        if let Err(reason) = member_requirements.admit(&resolved, effective) {
            last_rejection = Some(reason);
            trace.skip_member(&member.id, reason);
            continue;
        }
        if create || compact {
            let valid = *preflight.entry((effective,resolved.provider.chat_search,resolved.target_key.clone())).or_insert_with(|| {
                if protocol::Translation::between(ingress, effective).is_none() && delegated.is_none() {
                    return true;
                }
                let destination = &probe_destination;
                let normalized = if let Some(normalized) = &delegated {
                    Ok(Some(normalized.clone()))
                } else if command_selected && requirements.search {
                    hosted_search::Plan::for_destination(&destination, original.as_deref()).and_then(|plan| {
                        match plan { Some(plan) => Ok(Some(plan.normalized)), None => hosted_search::Plan::history_only(&destination,original.clone()) }
                    })
                } else {
                    hosted_search::Plan::history_only(&destination, original.clone())
                };
                normalized
                    .ok()
                    .flatten()
                    .is_some_and(|body| upstream_body(&destination,Some(body)).is_ok())
            });
            if !valid {
                last_rejection = Some("request_not_representable");
                trace.skip_member(&member.id, "request_not_representable");
                continue;
            }
        }
        if compact && effective != router::ModelProtocol::Responses {
            last_rejection = Some("compact_requires_responses");
            trace.skip_member(&member.id, "compact_requires_responses");
            continue;
        }
        if resource_id.is_some() && effective != router::ModelProtocol::Responses {
            trace.skip_member(&member.id, "resource_requires_responses");
            continue;
        }
        let effective_target_key =
            agg::target_key(&resolved.provider, &member.actual_model_id, effective);
        plans.push(PlannedMember {
            resolved,
            effective_protocol: effective,
            compatibility_plan: compatibility.then_some("commandSearchViaChat"),
            effective_target_key,
        });
    }
    if plans.is_empty() {
        if count_tokens {
            return Err((
                StatusCode::NOT_IMPLEMENTED,
                "token counting requires a native Anthropic Messages member",
            ));
        }
        return Err((
            StatusCode::BAD_REQUEST,
            match last_rejection {
                Some("context_1m_unavailable") => {
                    "no aggregate member supports the requested 1M context"
                }
                Some("output_limit_exceeded") => "requested output exceeds aggregate member limits",
                Some("private_history_protocol") => {
                    "private history requires its original protocol"
                }
                Some("vision_unavailable") => "no aggregate member has verified image support",
                Some("tools_unavailable") => "aggregate members do not support the requested tools",
                Some("request_not_representable") => {
                    "no aggregate egress can represent these tools or conversation fields"
                }
                _ => "no aggregate member can serve this request",
            },
        ));
    }
    let pinned = bound
        .map(|owner| {
            plans.iter().find(|plan| {
                plan.agrees(
                    owner,
                    resource_owner.is_some() || requirements.continuation || requirements.private_history,
                )
            })
        })
        .transpose_option()
        .ok_or((
            StatusCode::CONFLICT,
            "bound aggregate member is unavailable or its connection changed",
        ))?;
    let binding_reservation = session_key
        .as_deref()
        .map(|key| affinity::shared().reserve_binding(key))
        .transpose()
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "conversation routing capacity reached",
            )
        })?;
    let mut binding_reservation = binding_reservation;
    let mut transcript = if let Some((prefix, previous)) = fingerprint {
        Some(aggregate_session::Tracker::new(
            transcript_namespace, prefix, session_key.clone().expect("transcript session"),
            affinity::shared().reserve_transcript().map_err(|_| (
                StatusCode::SERVICE_UNAVAILABLE, "conversation transcript capacity reached",
            ))?,
            previous.is_none(),
        ))
    } else {
        None
    };
    let resource_reservation = if create && ingress == router::ModelProtocol::Responses {
        Some(
            affinity::shared()
                .reserve_resource(&resource_scope)
                .map_err(|_| {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "response resource capacity reached",
                    )
                })?,
        )
    } else {
        None
    };
    // A session ID alone is not opaque state. Fresh, portable requests can
    // replace a failed binding; signed/tool/resource continuations keep theirs.
    let switching = !count_tokens && requirements.portable_fresh && pinned.is_some_and(|plan|
        agg::runtime::shared().should_switch(&plan.effective_target_key));
    let active_pin = pinned.filter(|_| !switching);
    let mut attempts = Attempts::new(
        aggregate.retry_policy.max_attempts,
        &requirements,
        bound.is_some() && !requirements.portable_fresh,
    );
    let candidates: Vec<_> = plans.iter().map(PlannedMember::candidate).collect();
    let mut excluded = HashSet::new();
    let mut retry_reason = switching.then(|| "aggregate_failure_threshold".into());
    let mut reserved = None;
    let identity = request_diagnostics::RequestIdentity {
        client_source: Some(client.into()),
        requested_model: desktop
            .as_ref()
            .map(|desktop| desktop.alias.clone())
            .or_else(|| Some(aggregate.public_model_id.clone())),
        resolved_model: None,
        route_revision: desktop.as_ref().map(|desktop| desktop.route_revision),
    };
    trace.identity(&identity);
    let portable_body = if ingress == router::ModelProtocol::Responses && create {
        session_compat::prepare_reasoning(original.clone(), || {
            let view = agg::project(store, aggregate, catalog);
            view.projection
                .map(|projection| router::aggregate_catalog_entry(aggregate, &projection))
        })?
    } else {
        original.clone()
    };
    loop {
        attempts.begin().map_err(|_| {
            (
                StatusCode::GATEWAY_TIMEOUT,
                "aggregate request deadline exceeded",
            )
        })?;
        let selection = if let Some(selected) = reserved.take() {
            Ok(selected)
        } else if let Some(plan) = active_pin {
            agg::runtime::shared()
                .reserve(&plan.candidate())
                .map(|lease| agg::runtime::Selection {
                    candidate: plan.candidate(),
                    lease,
                })
        } else if count_tokens {
            agg::runtime::shared().select_auxiliary(&candidates)
        } else {
            agg::runtime::shared().select(&aggregate.id, aggregate.strategy, &candidates, &excluded)
        };
        let mut selected = match selection {
            Ok(selected) => selected,
            Err(agg::runtime::SelectionError::Cooling { retry_after_ms }) => {
                trace.reject(429, "aggregate_members_cooling");
                return Ok(Response::builder().status(StatusCode::TOO_MANY_REQUESTS)
                    .header("retry-after", retry_after_ms.saturating_add(999).checked_div(1000).unwrap_or(1).max(1).to_string())
                    .header(CONTENT_TYPE, "application/json").body(full_body(json!({"error":{"type":"rate_limit_error","message":"aggregate members are cooling down"}}).to_string())).expect("static response"));
            }
            Err(error) => return Err(selection_error(error)),
        };
        selected.lease.inference(aggregate.retry_policy.failure_threshold, create || compact);
        let plan = plans
            .iter()
            .find(|plan| plan.resolved.member.id == selected.candidate.member_id)
            .expect("selected planned member");
        let destination = Destination::Provider {
            provider: Box::new(plan.resolved.provider.clone()),
            model_override: Some(plan.resolved.member.actual_model_id.clone()),
            upstream_protocol: plan.effective_protocol,
            translation: protocol::Translation::between(ingress, plan.effective_protocol),
        };
        let actual_suffix = if resource_id.is_some() || compact || count_tokens {
            suffix.clone()
        } else {
            translate_endpoint(&suffix, plan.effective_protocol)
        };
        let client_scope = desktop
            .as_ref()
            .map(|desktop| protocol::message_history::ClientScope {
                client_source: "claudeDesktop",
                config_identity: &desktop.config_identity,
                model_alias: &desktop.alias,
                route_revision: desktop.route_revision,
            });
        let mut attempt_identity = identity.clone();
        attempt_identity.resolved_model = Some(plan.resolved.member.actual_model_id.clone());
        trace.identity(&attempt_identity);
        // An account source with no usable sign-in is passed over like a
        // member that cannot be reached.
        let account = match account_auth(context, &destination).await {
            Ok(account) => account,
            Err(error) => {
                trace.failed_attempt("credentials", "account_unavailable", 0);
                if attempts.may_retry(Failure::ConnectBeforeSend) {
                    excluded.insert(plan.effective_target_key.clone());
                    retry_reason = Some("aggregate_account_unavailable".into());
                    if let Ok(next) = agg::runtime::shared().select(&aggregate.id, aggregate.strategy, &candidates, &excluded) {
                        reserved = Some(next);
                        drop(selected);
                        continue;
                    }
                }
                return Err(error);
            }
        };
        let mut prepared = pipeline::request::prepare(pipeline::request::Input {
            context,
            store,
            catalog,
            destination: &destination,
            suffix: &actual_suffix,
            query,
            headers,
            body: portable_body.clone(),
            ingress,
            resource: resource_id.is_some() || count_tokens,
            create: create && *method == hyper::Method::POST,
            one_m,
            affinity_key: session_key.as_deref(),
            client_scope,
            identity: attempt_identity,
            trace: trace.clone(),
            selection_reason: if retry_reason.is_some() {
                "aggregate_retry".into()
            } else if bound.is_some() {
                "aggregate_session_binding".into()
            } else {
                "aggregate_preferred".into()
            },
            retry_reason: retry_reason.clone(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            account: account.as_ref(),
        }).await?;
        // A frozen retry may be vetoed by revocation, never silently updated.
        let current = router::read_store(&context.store_path).map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "router storage unavailable",
            )
        })?;
        if !agg::execution::still_authorized(&current, &aggregate.id, plan) {
            return Err((
                StatusCode::CONFLICT,
                "aggregate member changed before dispatch",
            ));
        }
        let owner = plan.owner(
            &aggregate.id,
            ingress == router::ModelProtocol::Responses
                && plan.effective_protocol == router::ModelProtocol::Responses,
        );
        prepared.sent(&trace);
        trace.aggregate_attempt(
            &aggregate.id,
            &plan.resolved.member.id,
            &agg::projection::route_revision(store, aggregate),
            pipeline::request::protocol_name(plan.resolved.protocol),
            pipeline::request::protocol_name(plan.effective_protocol),
            plan.compatibility_plan,
        );
        if prepared.hosted.is_some() {
            attempts.commit();
            if let Some(reservation) = binding_reservation.take() {
                reservation.commit(owner.clone());
            }
            drop(decision);
            let health = selected.lease.health();
            // Each internal model round reports separately. The final visible
            // search output must not clear a fault from a later model round.
            health.neutral();
            let response = prepared.search(trace.clone(), Some(health)).await?;
            return aggregate_output::project(
                response,
                aggregate_output::Projection {
                    model: aggregate.public_model_id.clone(),
                    protocol: ingress,
                    owner,
                    resource: resource_reservation,
                    lease: selected.lease,
                    trace,
                    terminal: Default::default(),
                    at_start: true,
                    transcript: transcript.take(),
                },
                *method == hyper::Method::HEAD,
            )
            .await;
        }
        let started = std::time::Instant::now();
        let sent = match prepared.exchange.take() {
            // A source that answers the request itself (Cursor's Runs).
            Some(exchange) => Ok(exchange.run().await),
            None => {
                let mut outbound = prepared
                    .client
                    .request(method.clone(), &prepared.url)
                    .headers(prepared.headers.clone())
                    .timeout(attempts.remaining());
                if let Some(body) = &prepared.body {
                    outbound = outbound.body(body.clone());
                }
                outbound.send().await
            }
        };
        let mut upstream = match sent {
            Ok(response) => response,
            Err(error) => {
                let connect = error.is_connect() && !error.is_timeout();
                if let Some(usage) = prepared.usage {
                    usage.failed(502, "response_not_received");
                }
                let failure = crate::core::api_client::TransportFailure::of(&error);
                trace.failed_attempt("connect", failure.code(), started.elapsed().as_millis() as u64);
                let switch = selected.lease.fail(None);
                if switch && connect && attempts.may_retry(Failure::ConnectBeforeSend) {
                    excluded.insert(plan.effective_target_key.clone());
                    retry_reason = Some("aggregate_connect_error".into());
                    if let Ok(next) = agg::runtime::shared().select(
                        &aggregate.id,
                        aggregate.strategy,
                        &candidates,
                        &excluded,
                    ) {
                        reserved = Some(next);
                        drop(selected);
                        continue;
                    }
                }
                trace.fail("connect", failure.code());
                return Err((StatusCode::BAD_GATEWAY, "cannot reach aggregate provider"));
            }
        };
        trace.status(upstream.status().as_u16());
        let switch = if agg::health::http_failure(upstream.status().as_u16()) {
            selected.lease.fail(retry_after(upstream.headers()))
        } else {
            if !upstream.status().is_success() { selected.lease.health().neutral(); }
            false
        };
        // An account source's refusal (WorkBuddy turning a client's prompt
        // away, a ChatGPT plan out of its limit) is explained, and another
        // member is tried while nothing has reached the client.
        if let Some(kind) = prepared.account_source.filter(|_| upstream.status().is_client_error()) {
            let (explained, refused) = explain_account_refusal(kind, upstream).await;
            upstream = explained;
            prepared.account_source = None;
            if refused && attempts.may_retry(Failure::RateLimited) {
                excluded.insert(plan.effective_target_key.clone());
                if let Ok(next) = agg::runtime::shared().select(&aggregate.id, aggregate.strategy, &candidates, &excluded) {
                    if let Some(usage) = prepared.usage {
                        usage.failed(upstream.status().as_u16(), "upstream_http_error");
                    }
                    trace.failed_attempt("headers", "account_refused", started.elapsed().as_millis() as u64);
                    retry_reason = Some("aggregate_account_refused".into());
                    reserved = Some(next);
                    drop(selected);
                    continue;
                }
            }
        }
        if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
            if switch && attempts.may_retry(Failure::RateLimited) {
                excluded.insert(plan.effective_target_key.clone());
                if let Ok(next) = agg::runtime::shared().select(
                    &aggregate.id,
                    aggregate.strategy,
                    &candidates,
                    &excluded,
                ) {
                    if let Some(usage) = prepared.usage {
                        usage.failed(429, "upstream_http_error");
                    }
                    trace.failed_attempt(
                        "headers",
                        "rate_limited",
                        started.elapsed().as_millis() as u64,
                    );
                    retry_reason = Some("aggregate_rate_limited".into());
                    reserved = Some(next);
                    drop(selected);
                    continue;
                }
            }
        }
        // A streamed reply that shows nothing in time is given up for another
        // member while none of it has reached the client. The last member a
        // request may use is always waited for.
        let first_token = Duration::from_secs(aggregate.first_token_timeout_secs.into());
        if upstream.status().is_success()
            && create
            && requirements.stream
            && !first_token.is_zero()
            && attempts.may_abandon_slow()
            && agg::runtime::shared().has_alternative(&candidates, &{
                let mut others = excluded.clone();
                others.insert(plan.effective_target_key.clone());
                others
            })
        {
            match agg::first_token::await_content(upstream, first_token.min(attempts.remaining())).await {
                agg::first_token::Outcome::Ready(response) => upstream = response,
                agg::first_token::Outcome::Slow => {
                    if let Some(usage) = prepared.usage {
                        usage.failed(504, "first_token_timeout");
                    }
                    trace.failed_attempt(
                        "first_token",
                        "first_token_timeout",
                        started.elapsed().as_millis() as u64,
                    );
                    let _ = selected.lease.fail(None);
                    excluded.insert(plan.effective_target_key.clone());
                    retry_reason = Some("aggregate_first_token_timeout".into());
                    if let Ok(next) = agg::runtime::shared().select(
                        &aggregate.id,
                        aggregate.strategy,
                        &candidates,
                        &excluded,
                    ) {
                        reserved = Some(next);
                        drop(selected);
                        continue;
                    }
                    trace.fail("first_token", "first_token_timeout");
                    return Err((
                        StatusCode::GATEWAY_TIMEOUT,
                        "aggregate members did not start replying in time",
                    ));
                }
            }
        }
        attempts.commit();
        drop(decision);
        let response = prepared.response(upstream, method.clone(), trace.clone(), Some(selected.lease.health())).await.map_err(|error| {
            if error.1 == "cannot read provider response" { selected.lease.fail(None); }
            error
        })?;
        if response.status().is_success() {
            if let Some(reservation) = binding_reservation.take() {
                reservation.commit(owner.clone());
            }
        }
        return aggregate_output::project(
            response,
            aggregate_output::Projection {
                model: aggregate.public_model_id.clone(),
                protocol: ingress,
                owner,
                resource: resource_reservation,
                lease: selected.lease,
                trace,
                terminal: Default::default(),
                at_start: true,
                transcript: transcript.take(),
            },
            *method == hyper::Method::HEAD,
        )
        .await;
    }
}

trait OptionalResult<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> OptionalResult<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(Some(value)) => Some(Some(value)),
            Some(None) => None,
        }
    }
}
fn selection_error(error: agg::runtime::SelectionError) -> (StatusCode, &'static str) {
    match error {
        agg::runtime::SelectionError::Cooling { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "aggregate members are cooling down",
        ),
        agg::runtime::SelectionError::ConcurrencyLimit => (
            StatusCode::SERVICE_UNAVAILABLE,
            "aggregate concurrency limit reached",
        ),
        agg::runtime::SelectionError::StateCapacity => (
            StatusCode::SERVICE_UNAVAILABLE,
            "aggregate routing state capacity reached",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "no available aggregate member",
        ),
    }
}
pub(super) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get("retry-after")?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.min(3600)));
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| {
            Duration::from_secs(
                (date.timestamp() - chrono::Utc::now().timestamp()).clamp(0, 3600) as u64,
            )
        })
}
