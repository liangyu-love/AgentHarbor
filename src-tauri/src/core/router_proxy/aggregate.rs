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
    let session = (!count_tokens).then(|| session(headers)).flatten();
    let session_key = session.map(|session| {
        affinity::scope(&[
            &resource_scope,
            &aggregate.id,
            &aggregate.public_model_id,
            role,
            session,
        ])
    });
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
            || requirements.private_history
            || (requirements.continuation && session_key.is_some()))
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
    // Protocol preflight is shared by members with the same effective egress.
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
        let compatibility = ingress == router::ModelProtocol::AnthropicMessages
            && requirements.search
            && resolved.protocol == router::ModelProtocol::Responses
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
        if let Err(reason) = requirements.admit(&resolved, effective) {
            last_rejection = Some(reason);
            trace.skip_member(&member.id, reason);
            continue;
        }
        if create || compact {
            let valid = *preflight.entry(effective).or_insert_with(|| {
                let Some(pair) = protocol::Translation::between(ingress, effective) else {
                    return true;
                };
                let destination = Destination::Provider {
                    provider: Box::new(resolved.provider.clone()),
                    model_override: Some(member.actual_model_id.clone()),
                    upstream_protocol: effective,
                    translation: Some(pair),
                };
                let normalized = if ingress == router::ModelProtocol::AnthropicMessages
                    && effective == router::ModelProtocol::ChatCompletions
                    && requirements.search
                {
                    hosted_search::Plan::prepare(value.clone()).map(|plan| Some(plan.normalized))
                } else {
                    hosted_search::Plan::history_only(&destination, original.clone())
                };
                normalized
                    .ok()
                    .flatten()
                    .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
                    .is_some_and(|body| protocol::request_body(pair, &body).is_ok())
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
                    resource_owner.is_some() || requirements.private_history,
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
    let mut attempts = Attempts::new(
        aggregate.retry_policy.max_attempts,
        &requirements,
        bound.is_some(),
    );
    let candidates: Vec<_> = plans.iter().map(PlannedMember::candidate).collect();
    let mut excluded = HashSet::new();
    let mut retry_reason = None;
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
        } else if let Some(plan) = pinned {
            agg::runtime::shared()
                .reserve(&plan.candidate())
                .map(|lease| agg::runtime::Selection {
                    candidate: plan.candidate(),
                    lease,
                })
        } else {
            agg::runtime::shared().select(&aggregate.id, aggregate.strategy, &candidates, &excluded)
        };
        let selected = match selection {
            Ok(selected) => selected,
            Err(agg::runtime::SelectionError::Cooling { retry_after_ms }) => {
                trace.reject(429, "aggregate_members_cooling");
                return Ok(Response::builder().status(StatusCode::TOO_MANY_REQUESTS)
                    .header("retry-after", retry_after_ms.saturating_add(999).checked_div(1000).unwrap_or(1).max(1).to_string())
                    .header(CONTENT_TYPE, "application/json").body(full_body(json!({"error":{"type":"rate_limit_error","message":"aggregate members are cooling down"}}).to_string())).expect("static response"));
            }
            Err(error) => return Err(selection_error(error)),
        };
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
        let prepared = forward::prepare(forward::Input {
            context,
            store,
            catalog,
            destination: &destination,
            suffix: &actual_suffix,
            query,
            headers,
            body: portable_body.clone(),
            ingress,
            resource: resource_id.is_some(),
            one_m,
            affinity_key: session_key.as_deref(),
            client_scope,
            identity: attempt_identity,
            trace: trace.clone(),
            selection_reason: if bound.is_some() {
                "aggregate_session_binding".into()
            } else if retry_reason.is_some() {
                "aggregate_retry".into()
            } else {
                "aggregate_rotation".into()
            },
            retry_reason: retry_reason.clone(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
        })?;
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
            forward::protocol_name(plan.resolved.protocol),
            forward::protocol_name(plan.effective_protocol),
            plan.compatibility_plan,
        );
        if prepared.hosted.is_some() {
            attempts.commit();
            if let Some(reservation) = binding_reservation.take() {
                reservation.commit(owner.clone());
            }
            drop(decision);
            let response = prepared.search(trace.clone()).await?;
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
                },
                *method == hyper::Method::HEAD,
            )
            .await;
        }
        let started = std::time::Instant::now();
        let mut outbound = prepared
            .client
            .request(method.clone(), &prepared.url)
            .headers(prepared.headers.clone())
            .timeout(attempts.remaining());
        if let Some(body) = &prepared.body {
            outbound = outbound.body(body.clone());
        }
        let upstream = match outbound.send().await {
            Ok(response) => response,
            Err(error) => {
                let connect = error.is_connect() && !error.is_timeout();
                if let Some(usage) = prepared.usage {
                    usage.failed(502, "response_not_received");
                }
                trace.failed_attempt(
                    "connect",
                    if error.is_timeout() {
                        "timeout"
                    } else {
                        "transport_error"
                    },
                    started.elapsed().as_millis() as u64,
                );
                if connect {
                    let _ = agg::runtime::shared().record_failure(&plan.effective_target_key, None);
                }
                if connect && attempts.may_retry(Failure::ConnectBeforeSend) {
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
                trace.fail("connect", "transport_error");
                return Err((StatusCode::BAD_GATEWAY, "cannot reach aggregate provider"));
            }
        };
        trace.status(upstream.status().as_u16());
        if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
            let _ = agg::runtime::shared()
                .record_failure(&plan.effective_target_key, retry_after(upstream.headers()));
            if attempts.may_retry(Failure::RateLimited) {
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
        attempts.commit();
        if upstream.status().is_success() {
            if let Some(reservation) = binding_reservation.take() {
                reservation.commit(owner.clone());
            }
        }
        drop(decision);
        let response = prepared.response(upstream, trace.clone()).await?;
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
fn session(headers: &HeaderMap) -> Option<&str> {
    [
        "session_id",
        "x-session-id",
        "x-codex-session-id",
        "conversation_id",
        "x-opencode-session",
    ]
    .iter()
    .find_map(|header| {
        headers
            .get(*header)
            .and_then(|value| value.to_str().ok())
            .filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            })
    })
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
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
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
