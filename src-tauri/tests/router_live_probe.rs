//! Opt-in, bounded upstream experiments. Never run in the ordinary test suite.
//! Set AGENTHARBOR_PROBE_STORE to an existing encrypted provider store,
//! AGENTHARBOR_PROBE_PROVIDER to an enabled provider ID, AGENTHARBOR_PROBE_MODEL
//! to one of its configured models, and optionally AGENTHARBOR_PROBE_PROXY.
//! Only synthetic text is sent. The store is read without migrations/writes;
//! credentials stay in memory and are not included in the report.

use agentharbor_lib::core::{protocol, router};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
#[ignore = "sends small billable requests to an explicitly selected configured provider"]
fn capture_live_reasoning_compatibility() {
    let path =
        PathBuf::from(std::env::var_os("AGENTHARBOR_PROBE_STORE").expect("explicit store path"));
    let before = std::fs::read(&path).unwrap();
    let store = router::read_store(&path).expect("read-only store");
    let id = std::env::var("AGENTHARBOR_PROBE_PROVIDER").expect("explicit provider ID");
    let model = std::env::var("AGENTHARBOR_PROBE_MODEL").expect("explicit model");
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == id && p.enabled)
        .expect("enabled provider");
    assert!(provider.models.iter().any(|m| m.id == model && m.included));
    let wire = provider.protocol_for_model(&model);
    assert!(matches!(
        wire,
        router::ModelProtocol::Responses | router::ModelProtocol::AnthropicMessages
    ));
    let mut url = reqwest::Url::parse(&provider.base_url).expect("stored URL");
    assert_eq!(url.scheme(), "https", "live probes require TLS");
    assert!(url.username().is_empty() && url.password().is_none());
    let base = url.path().trim_end_matches('/').trim_end_matches("/v1");
    let endpoint = if wire == router::ModelProtocol::Responses {
        "responses"
    } else {
        "messages"
    };
    url.set_path(&format!("{base}/v1/{endpoint}"));
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none());
    if provider.network_mode == "direct" {
        builder = builder.no_proxy();
    } else if let Ok(proxy) = std::env::var("AGENTHARBOR_PROBE_PROXY") {
        builder = builder
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy).unwrap());
    }
    let client = builder.build().unwrap();
    let mut cases = Vec::new();
    for effort in ["none", "low", "high"] {
        let canonical = json!({
            "model":model, "input":[{"role":"user","content":"Reply with the single word OK. Do not use any tools."}],
            "reasoning":{"effort":effort}, "max_output_tokens":8192, "stream":false
        });
        let body = if wire == router::ModelProtocol::AnthropicMessages {
            protocol::request_body(
                protocol::Translation {
                    from: router::ModelProtocol::Responses,
                    to: wire,
                },
                &canonical,
            )
            .unwrap()
            .body
        } else {
            canonical
        };
        cases.push((format!("current_{effort}"), body));
    }
    if wire == router::ModelProtocol::AnthropicMessages {
        for effort in ["low", "high"] {
            cases.push((format!("adaptive_{effort}"), json!({
                "model":model, "messages":[{"role":"user","content":"Reply with the single word OK. Do not use any tools."}],
                "max_tokens":8192, "thinking":{"type":"adaptive"}, "output_config":{"effort":effort}
            })));
        }
    }
    if let Ok(selected) = std::env::var("AGENTHARBOR_PROBE_CASES") {
        cases.retain(|(name, _)| selected.split(',').any(|case| case == name));
        assert!(!cases.is_empty(), "select at least one known case");
    }
    let mut report = Vec::new();
    for (case, body) in cases {
        let started = Instant::now();
        let mut request = client.post(url.clone()).json(&body);
        if wire == router::ModelProtocol::AnthropicMessages {
            request = request
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            request = request.bearer_auth(&provider.api_key);
        }
        let mut row = json!({"case":case,"model":model,"wire":wire,"sent":{
            "reasoning":body.get("reasoning"),"thinking":body.get("thinking"),"output_config":body.get("output_config")
        }});
        match request.send() {
            Ok(response) => {
                let status = response.status();
                row["status"] = json!(status.as_u16());
                let mut bytes = Vec::new();
                let read = response.take(256 * 1024).read_to_end(&mut bytes);
                row["body_read_ok"] = json!(read.is_ok());
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    row["usage"] = json!({
                        "input_tokens":value.pointer("/usage/input_tokens"),
                        "output_tokens":value.pointer("/usage/output_tokens"),
                        "reasoning_tokens":value.pointer("/usage/output_tokens_details/reasoning_tokens")
                    });
                    row["reported_model"] = value.get("model").cloned().unwrap_or(Value::Null);
                    row["reported_reasoning"] =
                        value.get("reasoning").cloned().unwrap_or(Value::Null);
                    row["stop_reason"] = value
                        .get("stop_reason")
                        .or_else(|| value.get("status"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    if !status.is_success() {
                        let error = value
                            .get("error")
                            .or_else(|| value.get("detail"))
                            .unwrap_or(&Value::Null)
                            .to_string();
                        let redacted = error.replace(&provider.api_key, "[redacted]");
                        row["error"] = json!(redacted.chars().take(700).collect::<String>());
                    }
                    let text = value
                        .get("content")
                        .or_else(|| value.get("output"))
                        .map(Value::to_string)
                        .unwrap_or_default();
                    row["contains_ok"] = json!(text.contains("OK"));
                } else {
                    row["json"] = json!(false);
                }
            }
            Err(error) => {
                row["transport_error"] = json!(if error.is_timeout() {
                    "timeout"
                } else {
                    "network"
                })
            }
        }
        row["ms"] = json!(started.elapsed().as_millis() as u64);
        eprintln!("LIVE_PROBE {row}");
        let stop = matches!(row["status"].as_u64(), Some(401 | 402 | 403 | 429))
            || row.get("transport_error").is_some();
        report.push(row);
        if stop {
            break;
        }
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "source store was modified"
    );
    if let Some(output) = std::env::var_os("AGENTHARBOR_PROBE_REPORT") {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .expect("choose a new report path; never overwrite an existing file");
        serde_json::to_writer_pretty(file, &report).unwrap();
    }
}
