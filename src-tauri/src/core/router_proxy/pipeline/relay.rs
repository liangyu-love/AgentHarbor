//! The upstream reply back to the client: account-source refusals explained,
//! streams translated or adapted, usage observed, and errors re-encoded in
//! the client's protocol.
use super::super::*;

/// An account source's refusal with what the user can do about it added to
/// its message, and whether there was anything to add.
pub(in crate::core::router_proxy) async fn explain_account_refusal(kind: account_sources::SourceKind, response: reqwest::Response) -> (reqwest::Response, bool) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.unwrap_or_default();
    let hint = account_sources::explain(kind, status.as_u16(), &bytes);
    let body = match &hint {
        Some(hint) => {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let said = value["error"]["message"].as_str().or_else(|| value["error"].as_str())
                .or_else(|| value["msg"].as_str()).or_else(|| value["message"].as_str())
                .map(str::trim).filter(|said| !said.is_empty());
            let message = match said {
                Some(said) => format!("{said} — {hint}"),
                None => hint.clone(),
            };
            let mut error = value["error"].as_object().cloned().unwrap_or_default();
            error.insert("message".into(), json!(message));
            error.entry("type").or_insert_with(|| json!("invalid_request_error"));
            Bytes::from(json!({"error": error}).to_string())
        }
        None => bytes,
    };
    let mut rebuilt = hyper::Response::builder().status(status);
    for (name, value) in &headers {
        if name != CONTENT_LENGTH && name != TRANSFER_ENCODING && (hint.is_none() || name != CONTENT_TYPE) {
            rebuilt = rebuilt.header(name, value);
        }
    }
    if hint.is_some() {
        rebuilt = rebuilt.header(CONTENT_TYPE, "application/json");
    }
    (reqwest::Response::from(rebuilt.body(body).expect("response parts")), hint.is_some())
}

/// Streams the upstream response back, translating it into the caller's protocol
/// when needed and recording relay usage along the way.
pub(in crate::core::router_proxy) async fn relay_response(
    response: reqwest::Response,
    translation: Option<protocol::Translation>,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    translated_input: Vec<Value>,
    trace: route_trace::Trace,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    message_context: Option<protocol::message_history::Context>,
    tool_adapter: tool_schema::Adapter,
    tool_namespaces: protocol::tool_namespaces::ToolNamespaces,
    response_options: protocol::ResponseOptions,
    health: Option<crate::core::aggregation::runtime::Health>,
) -> ForwardResult {
    let response_status = response.status();
    let is_event_stream = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    let mut response_headers = response.headers().clone();
    remove_hop_by_hop_headers(&mut response_headers);
    response_headers.remove(CONTENT_LENGTH);

    // A translated stream is rebuilt as Responses events, so its length and any
    // upstream content type no longer describe the bytes being sent.
    let mut builder = Response::builder().status(response_status);
    for (name, value) in &response_headers {
        if translation.is_some() && name == CONTENT_TYPE {
            continue;
        }
        builder = builder.header(name, value);
    }
    if translation.is_some() {
        // A translated stream is rebuilt as Responses events; a translated
        // single body (including an error) is JSON in the caller's shape.
        builder = builder.header(
            CONTENT_TYPE,
            if is_event_stream && response_status.is_success() {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
    }

    let response_body = match translation {
        Some(translation) if response_status.is_success() && is_event_stream => translated_stream(
            response,
            translation,
            relay_request,
            translated_input,
            trace,
            upstream_protocol,
            upstream_model,
            attempt_id,
            message_context,
            tool_adapter,
            tool_namespaces,
            response_options,
            health,
        )?,
        Some(translation) if response_status.is_success() => {
            let mut diagnostic = ResponseObservation::new(
                trace.clone(),
                relay_request,
                upstream_protocol,
                &upstream_model,
                &attempt_id,
                response_status.as_u16(),
                false,
            ).with_health(health);
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    diagnostic.error();
                    diagnostic.finalize(false);
                    return Err((StatusCode::BAD_GATEWAY, "cannot read provider response"));
                }
            };
            diagnostic.push(&bytes);
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                diagnostic.translation_error("invalid_provider_response");
                (StatusCode::BAD_GATEWAY, "invalid provider response")
            })?;
            diagnostic.value(&value);
            tool_adapter.response(&mut value, upstream_protocol).map_err(|code| {
                diagnostic.translation_error(code);
                (StatusCode::BAD_GATEWAY, "invalid Gemini tool arguments")
            })?;
            let mut converted = WireAdapter::new(translation.from)
                .encode_response(upstream_protocol, &value, &response_options).map_err(|_| {
                diagnostic.translation_error("invalid_upstream_response");
                (
                    StatusCode::BAD_GATEWAY,
                    "cannot translate provider response",
                )
            })?;
            tool_namespaces.restore_response(&mut converted);
            diagnostic.finish();
            if translation.from == router::ModelProtocol::Responses {
                protocol::history::remember(&converted, translated_input);
            } else if let Some(history) = message_context {
                if let Ok(canonical) = protocol::canonical_response(translation, &value) {
                    history.remember(&canonical);
                }
            }
            BodyExt::boxed(
                Full::new(Bytes::from(converted.to_string())).map_err(|never| match never {}),
            )
        }
        Some(translation) => {
            // An upstream error must reach the caller in the caller's own error shape,
            // otherwise SDKs report a parse failure instead of the real message.
            let bytes = response.bytes().await.unwrap_or_default();
            let mut diagnostic = ResponseObservation::new(
                trace.clone(),
                relay_request,
                upstream_protocol,
                &upstream_model,
                &attempt_id,
                response_status.as_u16(),
                false,
            );
            diagnostic.push(&bytes);
            diagnostic.finish();
            let body = translated_error_body(translation.from, response_status, &bytes);
            trace.fail("headers", "upstream_http_error");
            BodyExt::boxed(Full::new(Bytes::from(body)).map_err(|never| match never {}))
        }
        None if response_status.is_success() && !tool_adapter.is_empty() && !is_event_stream => {
            let mut diagnostic = ResponseObservation::new(trace, relay_request, upstream_protocol,
                &upstream_model, &attempt_id, response_status.as_u16(), false,
            ).with_health(health);
            let bytes = response.bytes().await.map_err(|_| {
                diagnostic.error();
                (StatusCode::BAD_GATEWAY, "cannot read provider response")
            })?;
            diagnostic.push(&bytes);
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                diagnostic.translation_error("invalid_provider_response");
                (StatusCode::BAD_GATEWAY, "invalid provider response")
            })?;
            diagnostic.value(&value);
            tool_adapter.response(&mut value, upstream_protocol).map_err(|code| {
                diagnostic.translation_error(code);
                (StatusCode::BAD_GATEWAY, "invalid Gemini tool arguments")
            })?;
            diagnostic.finish();
            BodyExt::boxed(Full::new(Bytes::from(value.to_string())).map_err(|never| match never {}),
            )
        }
        None if response_status.is_success() && !tool_adapter.is_empty() => adapted_native_stream(response, trace, relay_request, upstream_protocol, upstream_model, attempt_id, tool_adapter, health,
        ),
        None => {
            let stream = observed_stream(
                response,
                trace,
                relay_request,
                upstream_protocol,
                upstream_model,
                attempt_id,
                is_event_stream,
                health,
            )
            .map(move |chunk| {
                chunk
                    .map(Frame::data)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
            });
            if is_event_stream {
                BodyExt::boxed(StreamBody::new(sse_keepalive(stream, Duration::from_secs(15),
                )))
            } else { BodyExt::boxed(StreamBody::new(stream)) }
        }
    };
    builder
        .body(response_body)
        .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid upstream response"))
}

/// Translate an upstream event stream frame by frame; the translator emits a
/// terminal event if the upstream stops without one, so Codex never waits forever.
pub(in crate::core::router_proxy) fn translated_stream(
    response: reqwest::Response,
    translation: protocol::Translation,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    translated_input: Vec<Value>,
    trace: route_trace::Trace,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    message_context: Option<protocol::message_history::Context>,
    tool_adapter: tool_schema::Adapter,
    tool_namespaces: protocol::tool_namespaces::ToolNamespaces,
    response_options: protocol::ResponseOptions,
    health: Option<crate::core::aggregation::runtime::Health>,
) -> ForwardResult<ResponseBody> {
    let translator = WireAdapter::new(translation.from)
        .stream_decoder(upstream_protocol, response_options)
        .map_err(|_| (StatusCode::BAD_GATEWAY, "cannot translate provider stream"))?
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "invalid provider stream translation",
        ))?;
    let observer = ResponseObservation::new(
        trace,
        relay_request,
        upstream_protocol,
        &upstream_model,
        &attempt_id,
        response.status().as_u16(),
        true,
    ).with_health(health);
    // One owner finalizes usage after translation. Dropping this stream also
    // drops the observer, marking an interrupted delivery without losing usage.
    let stream = futures_util::stream::unfold(
        (
            response.bytes_stream(),
            observer,
            translator,
            Some(translated_input),
            message_context,
            tool_adapter.stream(upstream_protocol),
            tool_namespaces,
            false,
        ),
        move |(mut source, mut observer, mut translator, mut recorded, history, mut tools, namespaces, done,
        )| async move {
            if done {
                return None;
            }
            let (events, done) = match source.next().await {
                Some(Ok(bytes)) => {
                    observer.push(&bytes);
                    (match tools.push(&bytes) {
                        Ok(decoded) => translator.push(&decoded),
                        Err(code) => translator.fail(code),
                    }, false,
                    )
                }
                Some(Err(error)) => {
                    observer.error();
                    (translator.fail(if error.is_timeout() { "upstream_stream_timeout" } else { "upstream_stream_error" }), true,
                    )
                }
                None => (match tools.finish() {
                    Ok(()) => translator.finish(),
                    Err(code) => translator.fail(code),
                }, true,
                ),
            };
            if let Some(code) = translator.failure() {
                observer.translation_error(code);
            }
            let done = done || translator.is_finished();
            // Commit before exposing terminal events: a follow-up may arrive
            // before the upstream connection reaches EOF.
            if let Some(response) = translator.completed_response() {
                if let Some(input) = recorded.take() {
                    if translation.from == router::ModelProtocol::Responses {
                        let mut response = response.clone();
                        namespaces.restore_response(&mut response);
                        protocol::history::remember(&response, input);
                    } else if let Some(history) = &history {
                        history.remember(response);
                    }
                }
            }
            if done {
                observer.finish();
            }
            let events: Vec<_> = events.into_iter().map(|event| namespaces.restore_event(event)).collect();
            let frames = futures_util::stream::iter(events.into_iter().map(|event| {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Frame::data(Bytes::from(event)))
            }));
            Some((
                frames,
                (source, observer, translator, recorded, history, tools, namespaces, done,
                ),
            ))
        },
    )
    .flatten();
    Ok(BodyExt::boxed(StreamBody::new(sse_keepalive(stream, Duration::from_secs(15),
    ))))
}

pub(in crate::core::router_proxy) fn adapted_native_stream(
    response: reqwest::Response,
    trace: route_trace::Trace,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    protocol: router::ModelProtocol,
    model: String,
    attempt_id: String,
    adapter: tool_schema::Adapter,
    health: Option<crate::core::aggregation::runtime::Health>,
) -> ResponseBody {
    let observer = ResponseObservation::new(
        trace,
        relay_request,
        protocol,
        &model,
        &attempt_id,
        response.status().as_u16(),
        true,
    ).with_health(health);
    let stream = futures_util::stream::unfold(
        (
            response.bytes_stream(),
            observer,
            adapter.stream(protocol),
            native_sse::TerminalGate::new(protocol),
            false,
        ),
        move |(mut source, mut observer, mut adapter, mut gate, done)| async move {
            if done {
                return None;
            }
            let (result, done) = match source.next().await {
                Some(Ok(bytes)) => {
                    observer.push(&bytes);
                    (
                        gate.push(&bytes).and_then(|bytes| adapter.push(&bytes)),
                        false,
                    )
                }
                Some(Err(error)) => {
                    observer.error();
                    (
                        Err(if error.is_timeout() {
                            "upstream_stream_timeout"
                        } else {
                            "upstream_stream_error"
                        }),
                        true,
                    )
                }
                None => (
                    gate.flush().and_then(|bytes| {
                        let bytes = adapter.push(&bytes)?;
                        adapter.finish()?;
                        observer.stream_ended().map_or(Ok(bytes), Err)
                    }),
                    true,
                ),
            };
            let (bytes, done) = match result {
                Ok(bytes) => {
                    let done = done || adapter.is_finished();
                    if done {
                        if let Some(code) = observer.stream_ended() {
                            observer.translation_error(code);
                            return Some((
                                Ok::<Frame<Bytes>, Box<dyn std::error::Error + Send + Sync>>(
                                    Frame::data(Bytes::from(stream_error_event(protocol, code))),
                                ),
                                (source, observer, adapter, gate, true),
                            ));
                        }
                    }
                    if done {
                        observer.finish();
                    }
                    (bytes, done)
                }
                Err(code) => {
                    observer.translation_error(code);
                    observer.finalize(false);
                    // End the body with an error, never with an executable tool
                    // or a success terminal frame after invalid JSON arguments.
                    return Some((
                        Ok::<Frame<Bytes>, Box<dyn std::error::Error + Send + Sync>>(Frame::data(
                            Bytes::from(stream_error_event(protocol, code)),
                        )),
                        (source, observer, adapter, gate, true),
                    ));
                }
            };
            Some((
                Ok(Frame::data(bytes)),
                (source, observer, adapter, gate, done),
            ))
        },
    );
    BodyExt::boxed(StreamBody::new(sse_keepalive(
        stream,
        Duration::from_secs(15),
    )))
}

/// Downstream comments keep SDK/proxy connections alive during upstream
/// silence. They never enter usage observation or reset the upstream read
/// timeout; EOF/error/cancellation still terminates the source normally.
pub(in crate::core::router_proxy) fn sse_keepalive<S, E>(source: S, period: Duration,
) -> impl futures_util::Stream<Item = Result<Frame<Bytes>, E>> + Send
where S: futures_util::Stream<Item = Result<Frame<Bytes>, E>> + Send, E: Send,
{
    futures_util::stream::unfold((Box::pin(source), Box::pin(tokio::time::sleep(period)), false, false), move |(mut source, mut timer, mut sent_keepalive, mut source_started)| async move {
        let (item, from_source) = tokio::select! {
            biased;
            item = source.next() => (item?, true),
            _ = timer.as_mut() => {
                sent_keepalive = true;
                (Ok(Frame::data(Bytes::from_static(b": keepalive\n\n"))), false)
            },
        };
        let item = if from_source {
            item.map(|frame| match frame.into_data() {
                Ok(bytes) => {
                    // Native data is already framed. Once our comment precedes
                    // it, its optional BOM must not become a field-name prefix.
                    let remove_bom = !source_started && sent_keepalive && bytes.starts_with(b"\xef\xbb\xbf");
                    source_started |= !bytes.is_empty();
                    Frame::data(if remove_bom { bytes.slice(3..) } else { bytes })
                }
                Err(frame) => frame,
            })
        } else {
            item
        };
        timer.as_mut().reset(tokio::time::Instant::now() + period);
        Some((item, (source, timer, sent_keepalive, source_started)))
    },
    )
}

pub(in crate::core::router_proxy) fn observed_stream(
    response: reqwest::Response,
    trace: route_trace::Trace,
    relay_request: Option<relay_usage::RelayUsageRequest>,
    upstream_protocol: router::ModelProtocol,
    upstream_model: String,
    attempt_id: String,
    sse: bool,
    health: Option<crate::core::aggregation::runtime::Health>,
) -> impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send {
    let gate = (sse && response.status().is_success())
        .then(|| native_sse::TerminalGate::new(upstream_protocol));
    let observer = ResponseObservation::new(
        trace,
        relay_request,
        upstream_protocol,
        &upstream_model,
        &attempt_id,
        response.status().as_u16(),
        sse,
    ).with_health(health);
    futures_util::stream::unfold(
        (response.bytes_stream(), observer, gate, false),
        move |(mut stream, mut observer, mut gate, done)| async move {
            if done {
                return None;
            }
            match stream.next().await {
                Some(chunk) => {
                    if let Err(error) = &chunk {
                        observer.error();
                        observer.finalize(false);
                        let chunk = if sse {
                            Ok(Bytes::from(stream_error_event(
                                upstream_protocol,
                                if error.is_timeout() {
                                    "upstream_stream_timeout"
                                } else {
                                    "upstream_stream_error"
                                },
                            )))
                        } else {
                            chunk
                        };
                        return Some((chunk, (stream, observer, gate, true)));
                    }
                    match &chunk {
                        Ok(bytes) => observer.push(bytes),
                        Err(_) => observer.error(),
                    }
                    let chunk = match (chunk, gate.as_mut()) {
                        (Ok(bytes), Some(parser)) => match parser.push(&bytes) {
                            Ok(bytes) => Ok(bytes),
                            Err(code) => {
                                observer.translation_error(code);
                                observer.finalize(false);
                                return Some((
                                    Ok(Bytes::from(stream_error_event(upstream_protocol, code))),
                                    (stream, observer, gate, true),
                                ));
                            }
                        },
                        (chunk, _) => chunk,
                    };
                    Some((chunk, (stream, observer, gate, false)))
                }
                None => {
                    let (bytes, gate_error) =
                        match gate.as_mut().map(native_sse::TerminalGate::flush) {
                            Some(Ok(bytes)) => (bytes, None),
                            Some(Err(code)) => (Bytes::new(), Some(code)),
                            None => (Bytes::new(), None),
                        };
                    let error =
                        gate_error.or_else(|| if sse { observer.stream_ended() } else { None });
                    if let Some(code) = error {
                        observer.translation_error(code);
                    }
                    observer.finish();
                    let mut output = bytes.to_vec();
                    if let Some(code) = error {
                        output.extend_from_slice(
                            stream_error_event(upstream_protocol, code).as_bytes(),
                        );
                    }
                    if output.is_empty() {
                        None
                    } else {
                        Some((Ok(Bytes::from(output)), (stream, observer, gate, true)))
                    }
                }
            }
        },
    )
}

pub(in crate::core::router_proxy) fn stream_error_event(protocol: router::ModelProtocol, code: &str) -> String {
    let error = match protocol {
        router::ModelProtocol::Responses => json!({"type":"error","code":code,"message":code}),
        router::ModelProtocol::AnthropicMessages => {
            json!({"type":"error","error":{"type":"api_error","message":code}})
        }
        router::ModelProtocol::ChatCompletions => {
            return format!(
                "\n\ndata: {}\n\n",
                json!({"error":{"type":"api_error","code":code,"message":code}})
            );
        }
    };
    // A raw upstream may fail halfway through a frame. Start a fresh event so
    // the error remains parseable after any bytes already sent downstream.
    format!("\n\nevent: error\ndata: {error}\n\n")
}

/// One raw-byte observation per attempt. The same bounded model result is
/// delivered to the memory trace and the provider ledger before either closes.
pub(in crate::core::router_proxy) struct ResponseObservation {
    pub(in crate::core::router_proxy) trace: route_trace::Trace,
    pub(in crate::core::router_proxy) diagnostic: route_trace::Observation,
    pub(in crate::core::router_proxy) usage: Option<relay_usage::RelayUsageObserver>,
    pub(in crate::core::router_proxy) served: request_diagnostics::ServedModelObserver,
    pub(in crate::core::router_proxy) attempt_id: String,
    pub(in crate::core::router_proxy) finalized: bool,
    pub(in crate::core::router_proxy) delivered: bool,
    pub(in crate::core::router_proxy) health: Option<crate::core::aggregation::runtime::Health>,
}

impl ResponseObservation {
    pub(in crate::core::router_proxy) fn new(
        trace: route_trace::Trace,
        request: Option<relay_usage::RelayUsageRequest>,
        protocol: router::ModelProtocol,
        model: &str,
        attempt_id: &str,
        status: u16,
        sse: bool,
    ) -> Self {
        Self {
            diagnostic: route_trace::Observation::new(trace.clone(), sse, status),
            trace,
            usage: request.map(|request| request.observe(status, sse)),
            served: request_diagnostics::ServedModelObserver::new(protocol, model, status, sse),
            attempt_id: attempt_id.into(),
            finalized: false,
            delivered: false,
            health: None,
        }
    }
    pub(in crate::core::router_proxy) fn with_health(mut self, health: Option<crate::core::aggregation::runtime::Health>) -> Self {
        self.health = health;
        self
    }
    pub(in crate::core::router_proxy) fn value(&self, value: &Value) {
        if let Some(health) = &self.health {
            if crate::core::aggregation::health::channel_error(value) {
                health.fail(None);
            } else if !value["error"].is_null()
                || !value.pointer("/response/error").unwrap_or(&Value::Null).is_null()
                || matches!(value["type"].as_str(), Some("error" | "response.failed" | "response.cancelled"))
                || matches!(value["status"].as_str(), Some("failed" | "cancelled"))
            {
                health.neutral();
            }
        }
    }
    pub(in crate::core::router_proxy) fn push(&mut self, bytes: &[u8]) {
        self.diagnostic.push(bytes);
        self.served.push(bytes);
        if let Some(health) = &self.health {
            if !self.served.response_error().is_null() {
                if crate::core::aggregation::health::channel_error(
                    &json!({"error":self.served.response_error()}),
                ) {
                    health.fail(None);
                } else {
                    // Preserve the raw upstream category before a translator
                    // can wrap a parameter rejection in a generic API error.
                    health.neutral();
                }
            }
        }
        if let Some(usage) = self.usage.as_mut() {
            usage.observe(bytes);
        }
    }
    pub(in crate::core::router_proxy) fn error(&mut self) {
        if let Some(health) = &self.health {
            health.fail(None);
        }
        self.diagnostic.error();
        self.served.error();
        if let Some(usage) = self.usage.as_mut() {
            usage.fail_delivery();
        }
    }
    pub(in crate::core::router_proxy) fn translation_error(&mut self, code: &'static str) {
        self.trace.fail("translation", code);
        if let Some(health) = &self.health {
            if matches!(code, "upstream_stream_timeout" | "upstream_stream_error" | "upstream_stream_truncated") {
                health.fail(None);
            } else {
                health.neutral();
            }
        }
        if let Some(usage) = self.usage.as_mut() {
            usage.fail_delivery();
        }
    }
    pub(in crate::core::router_proxy) fn finish(&mut self) {
        self.delivered = true;
        self.finalize(true);
        self.diagnostic.finish();
    }
    pub(in crate::core::router_proxy) fn stream_ended(&mut self) -> Option<&'static str> {
        self.finalize(true);
        self.served
            .stream_truncated()
            .then_some("upstream_stream_truncated")
    }
    pub(in crate::core::router_proxy) fn finalize(&mut self, eof: bool) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let observation = self.served.finish(eof);
        if eof {
            if let Some(health) = &self.health {
                if (!self.served.response_error().is_null()
                    && crate::core::aggregation::health::channel_error(
                        &json!({"error":self.served.response_error()}),
                    ))
                    || self.served.stream_truncated()
                {
                    health.fail(None);
                }
            }
        }
        self.trace.observed(&self.attempt_id, observation.clone());
        if let Some(usage) = self.usage.as_mut() {
            usage.set_model_observation(observation);
        }
    }
}

impl Drop for ResponseObservation {
    fn drop(&mut self) {
        if !self.delivered {
            self.trace.fail("stream", "delivery_interrupted");
            if let Some(usage) = self.usage.as_mut() {
                usage.fail_delivery();
            }
        }
        self.finalize(false);
    }
}

/// Re-shape an upstream error body into the caller's protocol.
pub(in crate::core::router_proxy) fn translated_error_body(
    caller: router::ModelProtocol,
    status: StatusCode,
    bytes: &[u8],
) -> String {
    let value: Option<Value> = serde_json::from_slice(bytes).ok().or_else(|| {
        let text = std::str::from_utf8(bytes).ok()?;
        text.lines().filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find(|v| !v["error"].is_null() || v["type"] == "error")
    });
    let error = value.as_ref().and_then(|value| value.get("error"));
    let message = error
        .and_then(|error| error.get("message").or(Some(error)).and_then(Value::as_str))
        .or_else(|| {
            value
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text.is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("upstream error")
                    .to_owned()
            } else {
                text.chars().take(2000).collect()
            }
        });
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            match status.as_u16() {
                400 => "invalid_request_error",
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                529 => "overloaded_error",
                _ => "api_error",
            }
            .to_owned()
        });
    match caller {
        router::ModelProtocol::AnthropicMessages => {
            json!({"type": "error", "error": {"type": kind, "message": message}})
        }
        _ => json!({"error": {"message": message, "type": kind, "code": status.as_u16()}}),
    }
    .to_string()
}
