//! Logical model projection happens after raw upstream diagnostics/usage. The
//! resource owner is indexed before an ID is sent to the downstream client.
use super::super::aggregation::{
    affinity::{Owner, ResourceReservation},
    runtime::Lease,
};
use super::*;

const MAX_EVENT: usize = 256 * 1024;
const MAX_JSON: usize = 16 * 1024 * 1024;
pub(super) struct Projection {
    pub model: String,
    pub protocol: router::ModelProtocol,
    pub owner: Owner,
    pub resource: Option<ResourceReservation>,
    pub lease: Lease,
    pub trace: route_trace::Trace,
}

impl Projection {
    fn value(&mut self, value: &mut Value) -> Result<bool, &'static str> {
        let terminal = matches!(
            value["type"].as_str(),
            Some("response.completed" | "response.incomplete" | "message_stop")
        ) || self.protocol == router::ModelProtocol::ChatCompletions
            && value["choices"].as_array().is_some_and(|choices| {
                choices
                    .iter()
                    .any(|choice| !choice["finish_reason"].is_null())
            });
        let object = match self.protocol {
            router::ModelProtocol::Responses => {
                if matches!(
                    value["type"].as_str(),
                    Some(
                        "response.created"
                            | "response.in_progress"
                            | "response.queued"
                            | "response.completed"
                            | "response.incomplete"
                            | "response.failed"
                            | "response.cancelled"
                    )
                ) {
                    value.get_mut("response")
                } else if value["object"] == "response"
                    || value["type"] == "response"
                    || (value.get("output").is_some() && value.get("id").is_some())
                {
                    Some(value)
                } else {
                    None
                }
            }
            router::ModelProtocol::AnthropicMessages => {
                if value["type"] == "message_start" {
                    value.get_mut("message")
                } else if value["type"] == "message" {
                    Some(value)
                } else {
                    None
                }
            }
            router::ModelProtocol::ChatCompletions => matches!(
                value["object"].as_str(),
                Some("chat.completion" | "chat.completion.chunk")
            )
            .then_some(value),
        };
        let Some(object) = object else {
            if terminal {
                self.lease.succeed();
            }
            return Ok(false);
        };
        if let (Some(reservation), Some(id)) = (&mut self.resource, object["id"].as_str()) {
            reservation
                .register(id, &self.owner)
                .map_err(|_| "response_ownership_conflict")?;
        }
        let mut patched = false;
        if let Some(model) = object.get_mut("model") {
            if model.as_str().is_some() && model.as_str() != Some(self.model.as_str()) {
                *model = Value::String(self.model.clone());
                patched = true;
            }
        }
        // A terminal frame acknowledges delivery only after resource ownership
        // validation succeeds. Conflicting IDs must not clear the lease.
        if terminal {
            self.lease.succeed();
        }
        Ok(patched)
    }
    fn event(&mut self, raw: &[u8]) -> Result<Bytes, &'static str> {
        let Ok(text) = std::str::from_utf8(raw) else {
            return Err("invalid_provider_event");
        };
        let data: Vec<_> = text
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|data| data.strip_prefix(' ').unwrap_or(data))
            })
            .collect();
        if data.is_empty() {
            return Ok(Bytes::copy_from_slice(raw));
        }
        if data.join("\n").trim() == "[DONE]" {
            return Ok(Bytes::copy_from_slice(raw));
        }
        let Ok(mut value) = serde_json::from_str::<Value>(&data.join("\n")) else {
            return Ok(Bytes::copy_from_slice(raw));
        };
        if !self.value(&mut value)? {
            return Ok(Bytes::copy_from_slice(raw));
        }
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut output = String::new();
        let mut replaced = false;
        for line in text.lines() {
            if line.starts_with("data:") {
                if !replaced {
                    output.push_str(&format!("data: {value}{newline}"));
                    replaced = true;
                }
            } else {
                output.push_str(line);
                output.push_str(newline);
            }
        }
        if !output.ends_with(&format!("{newline}{newline}")) {
            output.push_str(newline);
        }
        Ok(Bytes::from(output))
    }
}

pub(super) async fn project(
    response: Response<ResponseBody>,
    projection: Projection,
    head: bool,
) -> ForwardResult {
    let successful = response.status().is_success();
    let sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    let (mut parts, mut body) = response.into_parts();
    parts.headers.remove(CONTENT_LENGTH);
    // HEAD and explicitly bodyless responses have no protocol object to patch.
    // Their successful status must not turn into a JSON parse failure.
    if head
        || matches!(
            parts.status,
            StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT
        )
    {
        if successful {
            projection.lease.succeed();
        }
        drop(projection);
        return Ok(Response::from_parts(parts, body));
    }
    if !sse && successful {
        let mut projection = projection;
        let mut buffer = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame =
                frame.map_err(|_| (StatusCode::BAD_GATEWAY, "cannot read aggregate response"))?;
            if let Ok(data) = frame.into_data() {
                if buffer.len().saturating_add(data.len()) > MAX_JSON {
                    projection.trace.fail("projection", "response_limit");
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        "aggregate response exceeds projection limit",
                    ));
                }
                buffer.extend_from_slice(&data);
            }
        }
        let mut value: Value = serde_json::from_slice(&buffer)
            .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid aggregate response"))?;
        let patched = projection.value(&mut value).map_err(|_| {
            (
                StatusCode::CONFLICT,
                "response resource ownership is ambiguous",
            )
        })?;
        if value["error"].is_null()
            && !matches!(value["status"].as_str(), Some("failed" | "cancelled"))
        {
            projection.lease.succeed();
        }
        let output = if patched {
            Bytes::from(value.to_string())
        } else {
            Bytes::from(buffer)
        };
        // A collected body is fully delivered from memory; holding the physical
        // upstream slot beyond this point would not constrain actual traffic.
        drop(projection);
        return Ok(Response::from_parts(
            parts,
            Full::new(output).map_err(|never| match never {}).boxed(),
        ));
    }
    let stream = futures_util::stream::unfold(
        (body, projection, Vec::<u8>::new(), false),
        move |(mut body, mut projection, mut pending, done)| async move {
            if done {
                return None;
            }
            loop {
                if sse && successful {
                    if let Some(end) = event_end(&pending) {
                        if end > MAX_EVENT {
                            projection.trace.fail("projection", "event_limit");
                            return Some((
                                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(
                                    Bytes::from(stream_error_event(
                                        projection.protocol,
                                        "aggregate_event_limit",
                                    )),
                                )),
                                (body, projection, Vec::new(), true),
                            ));
                        }
                        let raw: Vec<_> = pending.drain(..end).collect();
                        let bytes = match projection.event(&raw) {
                            Ok(bytes) => bytes,
                            Err(code) => {
                                projection.trace.fail("projection", code);
                                return Some((
                                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(
                                        Bytes::from(stream_error_event(projection.protocol, code)),
                                    )),
                                    (body, projection, Vec::new(), true),
                                ));
                            }
                        };
                        return Some((Ok(Frame::data(bytes)), (body, projection, pending, false)));
                    }
                }
                match body.frame().await {
                    Some(Ok(frame)) if !sse || !successful => {
                        return Some((Ok(frame), (body, projection, pending, false)))
                    }
                    Some(Ok(frame)) => {
                        if let Ok(data) = frame.into_data() {
                            pending.extend_from_slice(&data);
                        }
                        if pending.len() > MAX_EVENT
                            && event_end(&pending).is_none_or(|end| end > MAX_EVENT)
                        {
                            projection.trace.fail("projection", "event_limit");
                            return Some((
                                Ok(Frame::data(Bytes::from(stream_error_event(
                                    projection.protocol,
                                    "aggregate_event_limit",
                                )))),
                                (body, projection, Vec::new(), true),
                            ));
                        }
                    }
                    Some(Err(error)) => {
                        return Some((Err(error), (body, projection, Vec::new(), true)))
                    }
                    None => {
                        if pending.is_empty() {
                            return None;
                        }
                        // An unterminated event must not expose a resource ID
                        // that could not first be validated/indexed.
                        projection.trace.fail("projection", "truncated_event");
                        return Some((
                            Ok(Frame::data(Bytes::from(stream_error_event(
                                projection.protocol,
                                "aggregate_truncated_event",
                            )))),
                            (body, projection, Vec::new(), true),
                        ));
                    }
                }
            }
        },
    );
    Ok(Response::from_parts(
        parts,
        BodyExt::boxed(StreamBody::new(stream)),
    ))
}

fn event_end(bytes: &[u8]) -> Option<usize> {
    let lf = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| index + 2);
    let crlf = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4);
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::aggregation::{self, selector::Candidate};
    use super::*;
    fn projection() -> Projection {
        let candidate = Candidate {
            member_id: "a".into(),
            provider_id: "a".into(),
            actual_model_id: "wire".into(),
            priority: 0,
            weight: 1,
            target_key: "physical".into(),
        };
        Projection {
            model: "public".into(),
            protocol: router::ModelProtocol::Responses,
            owner: aggregation::affinity::Owner {
                aggregate_id: "pool".into(),
                member_id: "a".into(),
                provider_id: "a".into(),
                actual_model_id: "wire".into(),
                connection_key: "physical".into(),
                configured_protocol: router::ModelProtocol::Responses,
                effective_protocol: router::ModelProtocol::Responses,
                native_resource: true,
            },
            resource: None,
            lease: aggregation::runtime::Runtime::default()
                .reserve(&candidate)
                .unwrap(),
            trace: route_trace::Trace::begin("public", None),
        }
    }
    #[test]
    fn patch_only_protocol_model_locations_and_preserve_unknown_events() {
        let mut projection = projection();
        let mut value = json!({"object":"response","id":"resp_x","model":"wire","output":[{"text":"wire"}],"unknown":{"model":"wire"}});
        assert!(projection.value(&mut value).unwrap());
        assert_eq!(value["model"], "public");
        assert_eq!(value["output"][0]["text"], "wire");
        assert_eq!(value["unknown"]["model"], "wire");
        let unknown = b"event: response.future\r\ndata: {\"type\":\"response.future\",\"response\":{\"id\":\"resp_x\",\"model\":\"wire\"}}\r\n\r\n";
        assert_eq!(projection.event(unknown).unwrap().as_ref(), unknown);
        let known = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_x\",\"model\":\"wire\"}}\n\n";
        assert!(String::from_utf8(projection.event(known).unwrap().to_vec())
            .unwrap()
            .contains("public"));
    }

    #[test]
    fn conflicting_terminal_resource_does_not_acknowledge_recovery() {
        let runtime = aggregation::runtime::Runtime::default();
        let candidate = Candidate {
            member_id: "a".into(),
            provider_id: "a".into(),
            actual_model_id: "wire".into(),
            priority: 0,
            weight: 1,
            target_key: "physical".into(),
        };
        runtime
            .cooldown(&candidate.target_key, Duration::ZERO)
            .unwrap();
        let registry = aggregation::affinity::Registry::default();
        let scope = aggregation::affinity::scope(&["projection-conflict"]);
        let mut projection = projection();
        projection.lease = runtime.reserve(&candidate).unwrap();
        let mut other = projection.owner.clone();
        other.member_id = "different".into();
        registry
            .reserve_resource(&scope)
            .unwrap()
            .register("same-id", &other)
            .unwrap();
        projection.resource = Some(registry.reserve_resource(&scope).unwrap());
        let mut value =
            json!({"type":"response.completed", "response":{"id":"same-id","model":"wire"}});
        assert_eq!(
            projection.value(&mut value),
            Err("response_ownership_conflict")
        );
        assert!(matches!(
            runtime.reserve(&candidate),
            Err(aggregation::runtime::SelectionError::ConcurrencyLimit)
        ));
    }
}
