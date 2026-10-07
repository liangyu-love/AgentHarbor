//! Explicitly opt-in live aggregation acceptance. Ordinary test runs never send
//! upstream requests. The source store is read without migration and never saved;
//! the router, credentials, ledger and temporary aggregate live under a TempDir.
//! AGENTHARBOR_AGGREGATION_LIVE_STORE selects the existing encrypted card store.
//! AGENTHARBOR_AGGREGATION_LIVE_REPORT must name a new, redacted report file.

use crate::{
    core::{
        aggregation, models_dev, newapi, relay_usage, route_trace, router, router_proxy,
        secret_file,
    },
    platform::paths::CodexPaths,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

const PUBLIC_MODEL: &str = "deepseek-v4.1-flash";
const BODY_LIMIT: usize = 512 * 1024;
const OUTPUT_LIMIT: u64 = 1024;

struct Stop;
impl Drop for Stop {
    fn drop(&mut self) {
        router_proxy::stop_unchecked();
    }
}

#[test]
fn live_acceptance_does_not_count_truncated_messages_as_complete() {
    let values = [
        json!({"type":"message_start","message":{"model":PUBLIC_MODEL}}),
        json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"OK"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
        json!({"type":"message_stop"}),
    ];
    let stream: String = values
        .iter()
        .map(|value| format!("data: {value}\r\n\r\n"))
        .collect();
    let (facts, _) = inspect(
        router::ModelProtocol::AnthropicMessages,
        true,
        stream.as_bytes(),
    );
    assert_eq!(facts["containsOk"], true);
    assert_eq!(facts["stopReason"], "max_tokens");
    assert_eq!(facts["ok"], false);
    let complete = stream.replace("max_tokens", "end_turn");
    let (facts, _) = inspect(
        router::ModelProtocol::AnthropicMessages,
        true,
        complete.as_bytes(),
    );
    assert_eq!(facts["ok"], true);
}

fn card(store: &router::Store, host: &str, model: &str) -> router::Provider {
    let candidates: Vec<_> = store
        .providers
        .iter()
        .filter(|p| {
            reqwest::Url::parse(&p.base_url).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str() == Some(host)
                    && url.username().is_empty()
                    && url.password().is_none()
            }) && p.enabled
                && !p.api_key.trim().is_empty()
                && p.models.iter().any(|m| m.id == model && m.included)
        })
        .collect();
    assert_eq!(
        candidates.len(),
        1,
        "select exactly one enabled {host} card containing {model}"
    );
    let mut provider = candidates[0].clone();
    provider.model = model.into();
    provider.models.retain(|entry| entry.id == model);
    // Account/balance credentials play no part in inference acceptance.
    provider.access_token.clear();
    provider.user_id = None;
    provider.balance = None;
    provider.balance_script.clear();
    provider
}

fn text_request(protocol: router::ModelProtocol, stream: bool) -> Value {
    use router::ModelProtocol::*;
    let mut body = json!({"model":PUBLIC_MODEL,"stream":stream});
    let prompt = "Reply with the single word OK. Do not call any tools.";
    match protocol {
        Responses => {
            body["input"] = json!(prompt);
            body["max_output_tokens"] = json!(OUTPUT_LIMIT);
            body["reasoning"] = json!({"effort":"none"});
        }
        AnthropicMessages => {
            body["messages"] = json!([{"role":"user","content":prompt}]);
            body["max_tokens"] = json!(OUTPUT_LIMIT);
            body["thinking"] = json!({"type":"disabled"});
        }
        ChatCompletions => {
            body["messages"] = json!([{"role":"user","content":prompt}]);
            body["max_tokens"] = json!(OUTPUT_LIMIT);
            if stream {
                body["stream_options"] = json!({"include_usage":true});
            }
        }
    }
    body
}

fn sse_values(bytes: &[u8]) -> Result<(Vec<Value>, bool), &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid_utf8")?;
    let mut values = Vec::new();
    let mut data = Vec::new();
    let mut done = false;
    for line in text
        .trim_start_matches('\u{feff}')
        .lines()
        .chain(std::iter::once(""))
    {
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start());
        }
        if line.is_empty() && !data.is_empty() {
            let payload = data.join("\n");
            data.clear();
            if payload == "[DONE]" {
                done = true;
            } else {
                values.push(serde_json::from_str(&payload).map_err(|_| "invalid_sse_json")?);
            }
        }
    }
    Ok((values, done))
}

fn inspect(protocol: router::ModelProtocol, stream: bool, bytes: &[u8]) -> (Value, Option<Value>) {
    use router::ModelProtocol::*;
    let parsed = if stream {
        sse_values(bytes)
    } else {
        serde_json::from_slice::<Value>(bytes)
            .map(|value| (vec![value], false))
            .map_err(|_| "invalid_json")
    };
    let (values, done) = match parsed {
        Ok(values) => values,
        Err(code) => return (json!({"parseError":code,"ok":false}), None),
    };
    let mut models = Vec::new();
    let mut visible = String::new();
    let mut terminal = false;
    let mut stop_reason = None;
    let mut protocol_error = false;
    let mut tool = None;
    let mut usage = Value::Null;
    for value in &values {
        protocol_error |= value.get("error").is_some_and(|v| !v.is_null())
            || matches!(value["type"].as_str(), Some("error" | "response.failed"));
        match protocol {
            Responses => {
                let response = value.get("response").unwrap_or(value);
                if let Some(model) = response["model"].as_str() {
                    models.push(model);
                }
                terminal |= response["status"] == "completed"
                    && (!stream || value["type"] == "response.completed");
                if stream && value["type"] == "response.output_text.delta" {
                    visible.push_str(value["delta"].as_str().unwrap_or_default());
                }
                if !stream {
                    for item in response["output"].as_array().into_iter().flatten() {
                        for part in item["content"].as_array().into_iter().flatten() {
                            if part["type"] == "output_text" {
                                visible.push_str(part["text"].as_str().unwrap_or_default());
                            }
                        }
                    }
                }
                if response["usage"].is_object() {
                    usage = response["usage"].clone();
                }
            }
            AnthropicMessages => {
                let message = value.get("message").unwrap_or(value);
                if let Some(model) = message["model"].as_str() {
                    models.push(model);
                }
                terminal |= if stream {
                    value["type"] == "message_stop"
                } else {
                    matches!(value["stop_reason"].as_str(), Some("end_turn" | "tool_use"))
                };
                if let Some(reason) = value
                    .pointer("/delta/stop_reason")
                    .or_else(|| value.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    stop_reason = Some(reason);
                }
                if stream
                    && value.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta")
                {
                    visible.push_str(value["delta"]["text"].as_str().unwrap_or_default());
                }
                if !stream {
                    for part in value["content"].as_array().into_iter().flatten() {
                        if part["type"] == "text" {
                            visible.push_str(part["text"].as_str().unwrap_or_default());
                        }
                        if part["type"] == "tool_use" {
                            tool = Some(part.clone());
                        }
                    }
                }
                for source in [message.get("usage"), value.get("usage")]
                    .into_iter()
                    .flatten()
                {
                    if let Some(fields) = source.as_object() {
                        if !usage.is_object() {
                            usage = json!({});
                        }
                        usage.as_object_mut().unwrap().extend(fields.clone());
                    }
                }
            }
            ChatCompletions => {
                if let Some(model) = value["model"].as_str() {
                    models.push(model);
                }
                for choice in value["choices"].as_array().into_iter().flatten() {
                    terminal |= choice["finish_reason"] == "stop";
                    let message = if stream {
                        &choice["delta"]
                    } else {
                        &choice["message"]
                    };
                    visible.push_str(message["content"].as_str().unwrap_or_default());
                }
                if value["usage"].is_object() {
                    usage = value["usage"].clone();
                }
            }
        }
    }
    if stream && protocol == ChatCompletions {
        terminal &= done;
    }
    if protocol == AnthropicMessages {
        terminal &= matches!(stop_reason, Some("end_turn" | "tool_use"));
    }
    let public = !models.is_empty() && models.iter().all(|model| *model == PUBLIC_MODEL);
    let tool_valid = tool.as_ref().is_some_and(|tool| {
        tool["name"] == "probe_echo"
            && tool["input"]["value"] == "OK"
            && tool["id"].as_str().is_some_and(|id| !id.is_empty())
    });
    let facts = json!({"publicModelMatches":public,"terminal":terminal,"stopReason":stop_reason,"protocolError":protocol_error,
        "containsOk":visible.contains("OK"),"toolCallValid":tool_valid,"usage":usage,
        "ok":public && terminal && !protocol_error && (visible.contains("OK") || tool_valid)});
    (
        facts,
        if !stream {
            values.first().cloned()
        } else {
            None
        },
    )
}

struct Probe<'a> {
    client: Client,
    base: String,
    token: String,
    aggregate_id: String,
    paths: &'a CodexPaths,
    expected: &'a [Value],
}
impl Probe<'_> {
    async fn request(
        &self,
        case: &str,
        ingress: router::ModelProtocol,
        body: &Value,
        session: Option<&str>,
    ) -> (Value, Option<Value>) {
        let known: HashSet<_> = route_trace::recent()
            .into_iter()
            .map(|route| route.id)
            .collect();
        let endpoint = match ingress {
            router::ModelProtocol::Responses => "responses",
            router::ModelProtocol::AnthropicMessages => "messages",
            router::ModelProtocol::ChatCompletions => "chat/completions",
        };
        let mut request = self
            .client
            .post(format!("{}/aggregate/v1/{endpoint}", self.base))
            .bearer_auth(&self.token)
            .header("anthropic-version", "2023-06-01")
            .json(body);
        if let Some(session) = session {
            request = request.header("x-session-id", session);
        }
        let started = Instant::now();
        let result = async {
            let mut response = request.send().await.map_err(|_| "transport_error")?;
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| "body_read_error")? {
                if bytes.len() + chunk.len() > BODY_LIMIT {
                    return Err("body_limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, &'static str>((status, bytes))
        };
        let mut row = json!({"case":case,"ingress":ingress,"stream":body["stream"]});
        let mut reply = None;
        match tokio::time::timeout(Duration::from_secs(75), result).await {
            Ok(Ok((status, bytes))) => {
                row["status"] = json!(status);
                row["bytes"] = json!(bytes.len());
                let (facts, value) = inspect(ingress, body["stream"] == true, &bytes);
                row["response"] = facts;
                row["ok"] = json!(status == 200 && row["response"]["ok"] == true);
                reply = value;
            }
            failure => {
                row["failure"] = json!(match failure {
                    Ok(Err(code)) => code,
                    _ => "timeout",
                });
                row["ok"] = json!(false);
            }
        }
        row["elapsedMs"] = json!(started.elapsed().as_millis() as u64);
        // The accounting observer finishes after the final downstream chunk.
        for _ in 0..200 {
            if aggregation::runtime::shared().stats().in_flight == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let route = route_trace::recent()
            .into_iter()
            .find(|route| !known.contains(&route.id) && route.requested_model == PUBLIC_MODEL);
        let record = route.as_ref().and_then(|route| {
            relay_usage::load(&self.paths.app_data_dir)
                .ok()?
                .recent
                .into_iter()
                .find(|r| r.request_id == route.id)
        });
        if let Some(route) = route {
            row["requestId"] = json!(route.id);
            row["traceDone"] = json!(route.done);
            row["attemptCount"] = json!(route.attempts.len());
            row["traceError"] = json!(route.error);
            if let Some(attempt) = route.attempts.first() {
                // Select individual non-secret facts, never serialize an Attempt
                // wholesale (sent parameters and credential fingerprints omitted).
                row["attempt"] = json!({"provider":attempt.provider_name,"providerId":attempt.provider_id,
                    "memberId":attempt.member_id,"wireModel":attempt.upstream_model,
                    "configuredProtocol":attempt.configured_protocol,"effectiveProtocol":attempt.effective_protocol,
                    "selectionReason":attempt.selection_reason,"status":attempt.status,
                    "servedModel":attempt.model_observation.served_model,
                    "modelMatch":attempt.model_observation.model_match_status,
                    "firstTokenMs":route.first_token_ms});
                let identity_matches = self.expected.iter().any(|expected| {
                    expected["providerId"] == attempt.provider_id
                        && expected["memberId"] == row["attempt"]["memberId"]
                        && expected["wireModel"] == attempt.upstream_model
                        && expected["protocol"] == row["attempt"]["effectiveProtocol"]
                        && expected["protocol"] == row["attempt"]["configuredProtocol"]
                });
                row["identityMatches"] = json!(identity_matches);
                row["ok"] = json!(
                    row["ok"] == true
                        && route.done
                        && route.error.is_none()
                        && identity_matches
                        && attempt.status == Some(200)
                        && attempt.model_observation.served_model.as_deref()
                            == Some(attempt.upstream_model.as_str())
                        && route.attempts.len() == 1
                        && attempt.aggregate_id.as_deref() == Some(&self.aggregate_id)
                );
            } else {
                row["ok"] = json!(false);
            }
        } else {
            row["ok"] = json!(false);
        }
        if let Some(record) = record {
            let attempt = &row["attempt"];
            let consistent = record.provider_id
                == attempt["providerId"].as_str().unwrap_or_default()
                && record.model == attempt["wireModel"].as_str().unwrap_or_default();
            row["ledger"] = json!({"success":record.success,"usageReported":record.usage_reported,
                "inputTokens":record.tokens.input_tokens,"outputTokens":record.tokens.output_tokens,
                "costUsd":record.cost_usd,"pricingStatus":record.pricing_status,"memberMatches":consistent});
            row["ok"] = json!(
                row["ok"] == true
                    && record.success
                    && record.usage_reported
                    && record.tokens.output_tokens > 0
                    && consistent
            );
        } else {
            row["ok"] = json!(false);
        }
        eprintln!("AGGREGATION_LIVE {row}");
        (row, reply)
    }
}

#[test]
#[ignore = "uses explicitly selected Command and DeepSeek cards for at most 16 small billable inference requests"]
fn command_and_deepseek_flash_live_aggregation() {
    let _guard = super::global_router_test_lock();
    let source = PathBuf::from(
        std::env::var_os("AGENTHARBOR_AGGREGATION_LIVE_STORE").expect("explicit source store"),
    );
    let report_path = PathBuf::from(
        std::env::var_os("AGENTHARBOR_AGGREGATION_LIVE_REPORT").expect("explicit new report file"),
    );
    let report_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report_path)
        .expect("new report path");
    let before = std::fs::read(&source).expect("snapshot encrypted source");
    let saved = router::read_store(&source).expect("read source without migrations or writes");
    let providers = vec![
        card(&saved, "api.commandcode.ai", "deepseek/deepseek-v4.1-flash"),
        card(&saved, "api.deepseek.com", "deepseek-flash"),
    ];
    let members = providers
        .iter()
        .enumerate()
        .map(|(index, provider)| aggregation::AggregateMember {
            id: format!("live-member-{index}"),
            provider_id: provider.id.clone(),
            actual_model_id: provider.model.clone(),
            ..Default::default()
        })
        .collect();
    let aggregate = aggregation::AggregateModel {
        id: uuid::Uuid::new_v4().to_string(),
        public_model_id: PUBLIC_MODEL.into(),
        display_name: "DeepSeek 4.1 Flash acceptance".into(),
        members,
        retry_policy: aggregation::RetryPolicy { max_attempts: 1, ..Default::default() },
        ..Default::default()
    };
    let mut store = router::Store {
        version: 9,
        providers,
        aggregates: vec![aggregate],
        ..Default::default()
    };
    store.router.bearer_token = format!("live_{}", uuid::Uuid::new_v4().simple());
    aggregation::config::validate(&store).unwrap();
    let temp = tempfile::tempdir().expect("isolated home");
    let paths = CodexPaths::from_home(temp.path().into());
    paths.ensure_directories().unwrap();
    let source_root = source.parent().expect("store parent");
    let catalog = source_root.join(models_dev::CACHE_FILE);
    if catalog.is_file() {
        std::fs::copy(catalog, paths.app_data_dir.join(models_dev::CACHE_FILE)).unwrap();
    }
    secret_file::write(
        &paths.app_data_dir.join("newapi-providers.dat"),
        &serde_json::to_vec(&store).unwrap(),
    )
    .unwrap();
    let proxy = newapi::proxy_from_settings(&source_root.join("settings.json"));
    let selected: Vec<_> = store.providers.iter().enumerate().map(|(index, provider)| json!({"provider":provider.name,
        "providerId":provider.id,"memberId":format!("live-member-{index}"),
        "wireModel":provider.model,"protocol":provider.selected_protocol(),"networkMode":provider.network_mode()})).collect();
    eprintln!(
        "AGGREGATION_LIVE_SELECTED {}",
        json!({"publicModel":PUBLIC_MODEL,"members":selected,"proxyMode":proxy.mode})
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (rows, catalog_ok, tool_affinity) = runtime.block_on(async {
        let _stop = Stop;
        let port = router_proxy::ensure_running(0, paths.clone(), proxy).await.expect("isolated listener");
        let probe = Probe { client:Client::builder().no_proxy().timeout(Duration::from_secs(75)).build().unwrap(),
            base:format!("http://127.0.0.1:{port}"), token:store.router.bearer_token.clone(),
            aggregate_id:store.aggregates[0].id.clone(), paths:&paths, expected:&selected };
        let catalog = probe.client.get(format!("{}/aggregate/v1/models", probe.base)).bearer_auth(&probe.token)
            .send().await.unwrap().json::<Value>().await.unwrap();
        let catalog_ok = catalog["data"].as_array().is_some_and(|items| items.len() == 1 && items[0]["id"] == PUBLIC_MODEL);
        let mut rows = Vec::new();
        // Differential live coverage chooses the only eligible member in the
        // isolated fixture. Production no longer rotates a healthy channel.
        let choose_member = |chosen: usize| {
            let mut selected = store.clone();
            for (index, member) in selected.aggregates[0].members.iter_mut().enumerate() {
                member.enabled = index == chosen;
            }
            secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),
                &serde_json::to_vec(&selected).unwrap()).unwrap();
            aggregation::reconcile_runtime(&selected);
        };
        for protocol in [router::ModelProtocol::Responses, router::ModelProtocol::AnthropicMessages, router::ModelProtocol::ChatCompletions] {
            for stream in [false, true] {
                for index in 0..2 {
                    choose_member(index);
                    let (mut row, _) = probe.request(&format!("{protocol:?}_{stream}_{index}"), protocol, &text_request(protocol, stream), None).await;
                    let selected_egress = row["attempt"]["memberId"] == selected[index]["memberId"];
                    row["explicitEgressSelection"] = json!(selected_egress);
                    row["ok"] = json!(row["ok"] == true && selected_egress);
                    rows.push(row);
                }
            }
        }
        let mut affinity_ok = true;
        let mut tool_members = HashSet::new();
        if rows.iter().all(|row| row["ok"] == true) {
            for index in 0..2 {
                choose_member(index);
                let session = format!("live-{}-{index}", uuid::Uuid::new_v4());
                let mut request = text_request(router::ModelProtocol::AnthropicMessages, false);
                request["messages"] = json!([{"role":"user","content":"Call probe_echo exactly once with value OK. After its result, reply with the single word OK."}]);
                request["tools"] = json!([{"name":"probe_echo","description":"Returns its value without external side effects.",
                    "input_schema":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}}]);
                request["tool_choice"] = json!({"type":"tool","name":"probe_echo"});
                let (first, reply) = probe.request(&format!("tool_{index}_call"), router::ModelProtocol::AnthropicMessages, &request, Some(&session)).await;
                let member = first["attempt"]["memberId"].as_str().unwrap_or_default().to_owned();
                tool_members.insert(member.clone());
                let usable = first["ok"] == true && first["response"]["toolCallValid"] == true;
                rows.push(first);
                if !usable { affinity_ok = false; continue; }
                let reply = reply.unwrap();
                let tool = reply["content"].as_array().unwrap().iter().find(|part| part["type"] == "tool_use").unwrap();
                request["messages"].as_array_mut().unwrap().extend([
                    json!({"role":"assistant","content":reply["content"]}),
                    json!({"role":"user","content":[{"type":"tool_result","tool_use_id":tool["id"],"content":"OK"}]}),
                ]);
                request["tool_choice"] = json!({"type":"none"});
                let (mut second, _) = probe.request(&format!("tool_{index}_result"), router::ModelProtocol::AnthropicMessages, &request, Some(&session)).await;
                let same = second["attempt"]["memberId"].as_str() == Some(member.as_str());
                affinity_ok &= same && second["ok"] == true;
                second["sameSessionMember"] = json!(same);
                second["ok"] = json!(second["ok"] == true && same);
                rows.push(second);
            }
        } else { affinity_ok = false; }
        affinity_ok &= tool_members.len() == 2 && !tool_members.contains("");
        (rows, catalog_ok, affinity_ok)
    });
    drop(runtime);
    let source_unchanged = std::fs::read(&source).is_ok_and(|bytes| bytes == before);
    let lease_released = aggregation::runtime::shared().stats().in_flight == 0;
    let totals = relay_usage::load(&paths.app_data_dir).unwrap().all_time;
    let explicit_egress_selection = rows.iter().take(12).all(|row| row["explicitEgressSelection"] == true);
    let passed = source_unchanged
        && catalog_ok
        && lease_released
        && tool_affinity
        && explicit_egress_selection
        && rows.len() == 16
        && rows.iter().all(|row| row["ok"] == true)
        && totals.request_count == 16;
    let report = json!({"date":chrono::Utc::now().to_rfc3339(),"publicModel":PUBLIC_MODEL,
        "members":selected,"requests":rows,"catalogOk":catalog_ok,"explicitEgressSelection":explicit_egress_selection,
        "toolAffinity":tool_affinity,"sourceUnchanged":source_unchanged,"leasesReleased":lease_released,
        "ledgerTotals":totals,"passed":passed});
    serde_json::to_writer_pretty(report_file, &report).unwrap();
    temp.close()
        .expect("remove isolated credentials and ledger");
    assert!(
        passed,
        "live aggregation acceptance failed; inspect the redacted report"
    );
}
