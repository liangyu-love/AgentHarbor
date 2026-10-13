//! Explicit search capability check for one relay model. Each search format
//! is one small billable request; the verdict comes only from what the
//! provider returned, never from its endpoint, brand or model name.
use super::{
    models::ApiProxyConfigPayload,
    newapi,
    protocol::{self, ChatSearch, Translation},
    provider_probe,
    router::ModelProtocol,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::HashSet, io::Read, time::Duration};

const PROMPT: &str = "Use web search to find the current stable Rust version. Answer in one short sentence and cite the source.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProbeAttempt {
    /// `responses`, `messages`, `mimoChat` or `openAiChat`.
    pub format: String,
    /// `executed`, `failed`, `notExecuted`, `misclassified`, `rejected` or `error`.
    pub outcome: String,
    pub status: Option<u16>,
    pub sources: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProbeReport {
    pub model: String,
    pub protocol: ModelProtocol,
    pub attempts: Vec<SearchProbeAttempt>,
    /// The Chat search contract that actually executed a search.
    pub suggestion: Option<ChatSearch>,
}

pub fn run(
    config: &newapi::Connection,
    model: &str,
    protocol: ModelProtocol,
    proxy: &ApiProxyConfigPayload,
) -> Result<SearchProbeReport, String> {
    let model = newapi::validated_test_model(model)?.to_owned();
    let formats: &[&str] = match protocol {
        ModelProtocol::Responses => &["responses"],
        ModelProtocol::AnthropicMessages => &["messages"],
        // No Chat search contract is assumed; stop at the first that works.
        ModelProtocol::ChatCompletions => &["mimoChat", "openAiChat"],
    };
    let mut attempts = Vec::new();
    for format in formats {
        let attempt = attempt(config, &model, protocol, format, proxy);
        let executed = attempt.outcome == "executed";
        attempts.push(attempt);
        if executed {
            break;
        }
    }
    let suggestion = attempts
        .iter()
        .find(|attempt| attempt.outcome == "executed")
        .and_then(|attempt| match attempt.format.as_str() {
            "mimoChat" => Some(ChatSearch::Mimo),
            "openAiChat" => Some(ChatSearch::OpenAi),
            _ => None,
        });
    Ok(SearchProbeReport { model, protocol, attempts, suggestion })
}

fn request(format: &str, model: &str) -> (&'static str, Value) {
    let messages = json!([{"role":"user","content":PROMPT}]);
    match format {
        "responses" => ("v1/responses", json!({"model":model,"input":PROMPT,
            "tools":[{"type":"web_search"}],"max_output_tokens":1024,"stream":false})),
        "messages" => ("v1/messages", json!({"model":model,"messages":messages,"max_tokens":1024,"stream":false,
            "tools":[{"type":"web_search_20250305","name":"web_search","max_uses":1}],
            "tool_choice":{"type":"tool","name":"web_search"}})),
        // Chat search models may refuse `max_tokens`; the prompt keeps it short.
        "mimoChat" => ("v1/chat/completions", json!({"model":model,"messages":messages,"stream":false,
            "tools":[{"type":"web_search","force_search":true,"max_keyword":1}],"tool_choice":"auto"})),
        _ => ("v1/chat/completions", json!({"model":model,"messages":messages,"stream":false,
            "web_search_options":{}})),
    }
}

fn attempt(
    config: &newapi::Connection,
    model: &str,
    protocol: ModelProtocol,
    format: &str,
    proxy: &ApiProxyConfigPayload,
) -> SearchProbeAttempt {
    let mut attempt = SearchProbeAttempt {
        format: format.into(),
        outcome: "error".into(),
        status: None,
        sources: 0,
        detail: String::new(),
    };
    let (path, body) = request(format, model);
    let sent = (|| -> Result<(), String> {
        let url = newapi::endpoint(config, path)?;
        let (name, value) = newapi::model_auth_header(&config.base_url, protocol, &config.api_key)?;
        let mut request = newapi::client_with_timeout(proxy, Duration::from_secs(90))?
            .post(url)
            .header("User-Agent", "AgentHarbor/1.0")
            .header(name, value)
            .json(&body);
        if protocol == ModelProtocol::AnthropicMessages {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let response = request.send().map_err(newapi::request_error)?;
        let status = response.status();
        attempt.status = Some(status.as_u16());
        let mut bytes = Vec::new();
        response
            .take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|_| "newapi.errors.network")?;
        if !status.is_success() {
            // A refused declaration is evidence; auth, quota and outages are not.
            attempt.outcome = if matches!(status.as_u16(), 400 | 404 | 422) { "rejected" } else { "error" }.into();
            attempt.detail = provider_probe::error_message(&bytes).unwrap_or_default();
            return Ok(());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.response")?;
        let canonical = protocol::canonical_response(Translation { from: ModelProtocol::Responses, to: protocol }, &value)
            .map_err(|_| "newapi.errors.response")?;
        classify(&canonical, &mut attempt);
        Ok(())
    })();
    if let Err(error) = sent {
        attempt.outcome = "error".into();
        attempt.detail = error;
    }
    attempt.detail = provider_probe::redact(&attempt.detail, config).chars().take(600).collect();
    attempt
}

fn classify(canonical: &Value, attempt: &mut SearchProbeAttempt) {
    let output = canonical["output"].as_array().map(Vec::as_slice).unwrap_or_default();
    let calls: Vec<_> = output.iter().filter(|item| item["type"] == "web_search_call").collect();
    let mut urls = HashSet::new();
    let mut text = String::new();
    for item in output {
        for source in item["action"]["sources"].as_array().into_iter().flatten() {
            urls.extend(source["url"].as_str().map(str::to_owned));
        }
        for part in item["content"].as_array().into_iter().flatten() {
            text.push_str(part["text"].as_str().unwrap_or_default());
            for annotation in part["annotations"].as_array().into_iter().flatten() {
                urls.extend(annotation["url"].as_str().map(str::to_owned));
            }
        }
    }
    attempt.sources = urls.len();
    // Chat search reports no call items: native citations or search usage
    // are its evidence. A Responses/Messages answer needs an actual call.
    let chat_evidence = attempt.format.ends_with("Chat")
        && (!urls.is_empty() || canonical["usage"]["web_search_usage"]["tool_usage"].as_u64().unwrap_or(0) > 0);
    attempt.outcome = if calls.iter().any(|call| call["status"] == "completed") || (calls.is_empty() && chat_evidence) {
        "executed"
    } else if let Some(call) = calls.first() {
        text = call["error"]["code"].as_str().unwrap_or("search_failed").to_owned();
        "failed"
    } else if output.iter().any(|item| item["type"] == "function_call" && item["name"] == "web_search") {
        "misclassified"
    } else {
        "notExecuted"
    }
    .into();
    attempt.detail = text;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    /// Answer each request with the reply chosen from its body.
    fn serve(replies: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (status, reply) in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                bodies.push(serde_json::from_slice::<Value>(&body).unwrap());
                let reply = reply.to_string();
                write!(socket, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
            bodies
        });
        (format!("http://{address}"), server)
    }

    fn probe(protocol: ModelProtocol, replies: Vec<(u16, Value)>) -> (SearchProbeReport, Vec<Value>) {
        let (base_url, server) = serve(replies);
        let config = newapi::Connection { base_url, api_key: "sk-secret".into(), ..Default::default() };
        let proxy = newapi::effective_proxy("direct", &ApiProxyConfigPayload::default());
        let report = run(&config, "fixture-model", protocol, &proxy).unwrap();
        (report, server.join().unwrap())
    }

    #[test]
    fn chat_probe_suggests_only_a_contract_that_actually_searched() {
        let cited = json!({"id":"c","object":"chat.completion","model":"m","choices":[{"index":0,"finish_reason":"stop",
            "message":{"role":"assistant","content":"Rust 1.99.","annotations":[{"type":"url_citation","url":"https://www.rust-lang.org/","title":"Rust"}]}}],
            "usage":{"prompt_tokens":1,"completion_tokens":2,"web_search_usage":{"tool_usage":1,"page_usage":1}}});
        let plain = json!({"id":"c","object":"chat.completion","model":"m","choices":[{"index":0,"finish_reason":"stop",
            "message":{"role":"assistant","content":"Probably Rust 1.80."}}],"usage":{"prompt_tokens":1,"completion_tokens":2}});
        let refused = json!({"error":{"code":"400","message":"Param Incorrect","param":"webSearchEnabled is false sk-secret"}});
        // MiMo format refused, OpenAI format answered with real citations.
        let (report, bodies) = probe(ModelProtocol::ChatCompletions, vec![(400, refused.clone()), (200, cited.clone())]);
        assert_eq!(report.attempts.iter().map(|a| (a.format.as_str(), a.outcome.as_str())).collect::<Vec<_>>(),
            vec![("mimoChat", "rejected"), ("openAiChat", "executed")]);
        assert_eq!(report.suggestion, Some(ChatSearch::OpenAi));
        assert_eq!(report.attempts[1].sources, 1);
        assert!(!serde_json::to_string(&report).unwrap().contains("sk-secret"));
        assert_eq!(bodies[0]["tools"], json!([{"type":"web_search","force_search":true,"max_keyword":1}]));
        assert_eq!(bodies[1]["web_search_options"], json!({}));
        // A first success stops the probe; an answer without evidence is not search.
        let (report, bodies) = probe(ModelProtocol::ChatCompletions, vec![(200, cited)]);
        assert_eq!(report.suggestion, Some(ChatSearch::Mimo));
        assert_eq!(bodies.len(), 1);
        let (report, _) = probe(ModelProtocol::ChatCompletions, vec![(200, plain.clone()), (200, plain)]);
        assert!(report.attempts.iter().all(|a| a.outcome == "notExecuted") && report.suggestion.is_none());
    }

    #[test]
    fn native_probes_require_an_actual_completed_search_call() {
        let search = json!({"id":"m","type":"message","model":"m","role":"assistant","stop_reason":"end_turn",
            "content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"Rust"}},
                {"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[{"type":"web_search_result","url":"https://www.rust-lang.org/","title":"Rust","encrypted_content":"x"}]},
                {"type":"text","text":"Rust 1.99."}],"usage":{"input_tokens":1,"output_tokens":2}});
        let (report, bodies) = probe(ModelProtocol::AnthropicMessages, vec![(200, search)]);
        assert_eq!((report.attempts[0].outcome.as_str(), report.attempts[0].sources), ("executed", 1));
        assert_eq!(bodies[0]["tool_choice"], json!({"type":"tool","name":"web_search"}));
        assert!(report.suggestion.is_none());
        let misclassified = json!({"id":"m","type":"message","model":"m","role":"assistant","stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"t","name":"web_search","input":{}}],"usage":{"input_tokens":1,"output_tokens":2}});
        let (report, _) = probe(ModelProtocol::AnthropicMessages, vec![(200, misclassified)]);
        assert_eq!(report.attempts[0].outcome, "misclassified");
        let failed = json!({"id":"r","object":"response","status":"completed","model":"m","output":[
            {"type":"web_search_call","id":"ws_1","status":"failed","error":{"code":"search_not_executed"}},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"No.","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":2}});
        let (report, _) = probe(ModelProtocol::Responses, vec![(200, failed)]);
        assert_eq!((report.attempts[0].outcome.as_str(), report.attempts[0].detail.as_str()), ("failed", "search_not_executed"));
        let (report, _) = probe(ModelProtocol::Responses, vec![(429, json!({"error":{"message":"rate limited"}}))]);
        assert_eq!((report.attempts[0].outcome.as_str(), report.attempts[0].status), ("error", Some(429)));
    }
}
