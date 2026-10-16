use super::*;

fn native_answer(protocol: ModelProtocol, search: bool) -> Value {
    match protocol {
        ModelProtocol::Responses => json!({"id":"resp-selected","object":"response","model":"search-model","status":"completed",
            "output":if search {json!([
                {"id":"actual-search","type":"web_search_call","status":"completed","action":{"type":"search","query":"Rust","sources":[{"type":"url","url":"https://www.rust-lang.org/releases","title":"Rust"},{"type":"url","url":"javascript:bad"}]}},
                {"id":"search-summary","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Rust 1.99 fixture","annotations":[]}]}])}
            else {json!([{ "id":"answer","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Rust answer","annotations":[]}]}])},
            "usage":{"input_tokens":5,"output_tokens":6,"total_tokens":11}}),
        ModelProtocol::AnthropicMessages => json!({"id":"msg-selected","type":"message","role":"assistant","model":"search-model","stop_reason":"end_turn",
            "content":if search {json!([
                {"type":"server_tool_use","id":"actual-search","name":"web_search","input":{"query":"Rust"}},
                {"type":"web_search_tool_result","tool_use_id":"actual-search","content":[{"type":"web_search_result","url":"https://www.rust-lang.org/releases","title":"Rust","encrypted_content":"search-opaque"}]},
                {"type":"text","text":"Rust 1.99 fixture"}])} else {json!([{"type":"text","text":"Rust answer"}])},
            "usage":{"input_tokens":5,"output_tokens":6,"server_tool_use":{"web_search_requests":if search {1} else {0}}}}),
        ModelProtocol::ChatCompletions => chat_reply(json!({"role":"assistant","content":"Rust answer"}),"stop"),
    }
}

fn inference_reply(protocol: ModelProtocol, body: &Value) -> Value {
    let has_result = body[if protocol == ModelProtocol::Responses {"input"} else {"messages"}].to_string().contains("Rust 1.99 fixture");
    if has_result {let mut answer = native_answer(protocol,false);answer["model"] = json!("inference-model");return answer}
    let tool = body["tools"].as_array().unwrap().iter().find(|tool|
        tool["name"].as_str().or_else(|| tool["function"]["name"].as_str()).is_some_and(|name| name.starts_with("__agentharbor_web_search"))).unwrap();
    let name = tool["name"].as_str().or_else(|| tool["function"]["name"].as_str()).unwrap();
    match protocol {
        ModelProtocol::Responses => json!({"id":"resp-inference","object":"response","status":"completed","model":"inference-model",
            "output":[{"type":"reasoning","id":"rs-original","encrypted_content":"response-private","summary":[{"type":"summary_text","text":"private inference"}]},
                {"type":"function_call","id":"fc-search","call_id":"local-search","name":name,"arguments":"{\"query\":\"Rust\"}","status":"completed"}],
            "usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}),
        ModelProtocol::AnthropicMessages => json!({"id":"msg-inference","type":"message","role":"assistant","model":"inference-model","stop_reason":"tool_use",
            "content":[{"type":"thinking","thinking":"private inference","signature":"inference-signature"},
                {"type":"tool_use","id":"local-search","name":name,"input":{"query":"Rust"}}],"usage":{"input_tokens":3,"output_tokens":4}}),
        ModelProtocol::ChatCompletions => chat_reply(json!({"role":"assistant","content":null,"reasoning_content":"private inference",
            "tool_calls":[{"id":"local-search","type":"function","function":{"name":name,"arguments":"{\"query\":\"Rust\"}"}}]}),"tool_calls"),
    }
}

fn card(id: &str, url: &str, model: &str, protocol: ModelProtocol, mode: protocol::ChatSearch) -> router::Provider {
    router::Provider {id:id.into(),name:id.into(),base_url:url.into(),api_key:format!("{id}-only"),model:model.into(),protocol,
        chat_search:mode,enabled:true,network_mode:"direct".into(),..Default::default()}
}

fn store_file(paths: &CodexPaths, store: &router::Store) {
    crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),&serde_json::to_vec(store).unwrap()).unwrap();
}

struct Cleanup;
impl Drop for Cleanup {fn drop(&mut self) {
    account_sources::set_endpoint_override(account_sources::SourceKind::Kiro,None);
    account_sources::set_endpoint_override(account_sources::SourceKind::ChatgptApi,None);
    stop_unchecked();
}}

#[test]
fn client_search_argument_types_and_sse_rejection_survive_router_translation() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        let schema = json!({"type":"object","properties":{"system2_search_query":{"type":["array","null"],"items":{"type":"object",
            "properties":{"q":{"type":"string"},"recency":{"type":["integer","null"]},"domains":{"type":["array","null"],"items":{"type":"string"}}}}}}});
        let expected = json!({"system2_search_query":[{"q":"Rust","recency":7,"domains":null}]});
        for target in [ModelProtocol::AnthropicMessages,ModelProtocol::ChatCompletions,ModelProtocol::Responses] {
        for streaming in [false,true] {
            let upstream = Mock::new(move |_,body| {
                if body["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {
                    let error = json!({"error":{"code":"responses_feature_not_supported","message":"tool type 'web_search' is not supported by this gateway phase.","type":"unsupported_feature"}});
                    return Reply {status:400,body:json!(format!("data:{error}\n\n")),delay:Duration::ZERO};
                }
                let args = json!({"system2_search_query":"[{\"q\":\"Rust\",\"recency\":7,\"domains\":null}]"});
                match target {
                    ModelProtocol::Responses=>Reply::ok(json!({"id":"search-client","object":"response","model":"mimo-v2.6-pro","status":"completed",
                        "output":[{"type":"function_call","id":"fc-client","call_id":"client-search","name":"_web_run","arguments":args.to_string(),"status":"completed"}],"usage":{"input_tokens":2,"output_tokens":3}})),
                    ModelProtocol::AnthropicMessages=>Reply::ok(if streaming {json!(format!(
                        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"search-client\",\"model\":\"mimo-v2.6-pro\",\"usage\":{{\"input_tokens\":2}}}}}}\n\n\
                        event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"client-search\",\"name\":\"_web_run\",\"input\":{{}}}}}}\n\n\
                        event: content_block_delta\ndata: {}\n\n\
                        event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
                        event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"tool_use\"}},\"usage\":{{\"output_tokens\":3}}}}\n\n\
                        event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
                        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":args.to_string()}})))} else {
                        json!({"id":"search-client","type":"message","model":"mimo-v2.6-pro","role":"assistant","stop_reason":"tool_use",
                            "content":[{"type":"tool_use","id":"client-search","name":"_web_run","input":args}],"usage":{"input_tokens":2,"output_tokens":3}})}),
                    ModelProtocol::ChatCompletions=>Reply::ok(if streaming {json!(format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"client-search","type":"function","function":{"name":"_web_run","arguments":args.to_string()}}]},"finish_reason":""}]}),
                        json!({"choices":[{"index":0,"delta":{"function_call":{"name":"","arguments":""}},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}})))} else {
                        chat_reply(json!({"role":"assistant","tool_calls":[{"id":"client-search","type":"function","function":{"name":"_web_run","arguments":args.to_string()}}]}),"tool_calls")}),
                }
            }).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            let store = router::Store {providers:vec![card("inference",&upstream.url,"mimo-v2.6-pro",target,protocol::ChatSearch::Auto)],
                router:router::RouterSettings {bearer_token:"router-local".into(),..Default::default()},..Default::default()};
            store_file(&paths,&store);
            let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
            let mut tools = vec![json!({"type":"function","name":"_web_run","parameters":schema})];
            if target == ModelProtocol::Responses {tools.push(json!({"type":"web_search"}));}
            let mut input = json!({"model":"inference::mimo-v2.6-pro","input":"Search Rust","stream":streaming,"tools":tools});
            let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
            let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").header("idempotency-key","original").json(&input).send().await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            let text = response.text().await.unwrap();
            let value: Value = if target != ModelProtocol::Responses && streaming {
                text.lines().filter_map(|line| line.strip_prefix("data:")).filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                    .find(|event| event["type"] == "response.completed").unwrap()["response"].clone()
            } else {serde_json::from_str(&text).unwrap()};
            assert_eq!(value["status"],"completed","{text}");
            let call = value["output"].as_array().unwrap().iter().find(|item| item["type"] == "function_call").unwrap();
            assert_eq!(serde_json::from_str::<Value>(call["arguments"].as_str().unwrap()).unwrap(),expected);
            assert!(value["output"].as_array().unwrap().iter().all(|item| item["type"] != "web_search_call"),"ordinary tool invocation is not native search");
            let captured = upstream.captured();
            assert_eq!(captured.len(),if target == ModelProtocol::Responses {2} else {1});
            if target == ModelProtocol::Responses {assert_ne!(captured[0]["idempotencyKey"],captured[1]["idempotencyKey"]);}
            input["previous_response_id"] = value["id"].clone();
            input["input"] = json!([{"type":"function_call_output","call_id":"client-search","output":"Real search result"}]);
            input["tools"] = json!([tools[0]]);
            let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&input).send().await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            response.bytes().await.unwrap();
            if target != ModelProtocol::Responses {
                let captured = upstream.captured();
                let messages = captured.last().unwrap()["body"]["messages"].as_array().unwrap();
                let replayed = if target == ModelProtocol::AnthropicMessages {
                    messages.iter().flat_map(|message| message["content"].as_array().into_iter().flatten()).find(|block| block["type"] == "tool_use").unwrap()["input"].clone()
                } else {
                    let function = &messages.iter().find(|message| message["role"] == "assistant").unwrap()["tool_calls"][0]["function"];
                    serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap()
                };
                assert_eq!(replayed,expected,"history must retain the arguments delivered to the tool");
            }
            stop_unchecked();
        }
        }
    });
}

#[test]
fn native_search_recovers_misclassified_hosted_calls_on_the_original_provider() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        for target in [ModelProtocol::AnthropicMessages,ModelProtocol::Responses] {
        for streaming in [false,true] {
        for mixed_error in [false,true] {
            let upstream = Mock::new(move |_,body| {
                if body["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {
                    if body["tools"].as_array().unwrap().len() == 1 {
                        return Reply::ok(native_answer(target,true));
                    }
                    if mixed_error {
                        return Reply {status:400,body:json!({"error":{"message":"Please enable tool_config.include_server_side_tool_invocations to use Built-in tools with Function calling."}}),delay:Duration::ZERO};
                    }
                    // Observed relay behavior: hosted search becomes an ordinary
                    // client call with no query when mixed with function tools.
                    if target == ModelProtocol::Responses {
                        let response = json!({"id":"bad-search","object":"response","status":"completed","model":"inference-model",
                            "output":[{"type":"function_call","id":"bad-search-call","call_id":"bad-search-call","name":"web_search","arguments":"{}","status":"completed"}],
                            "usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}});
                        return Reply::ok(if streaming {json!(format!("event: response.completed\ndata: {}\n\n",json!({"type":"response.completed","response":response})))} else {response});
                    }
                    return Reply::ok(if streaming {json!(concat!(
                        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"bad-search\",\"model\":\"inference-model\",\"usage\":{\"input_tokens\":3}}}\n\n",
                        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"bad-search-call\",\"name\":\"web_search\",\"input\":{}}}\n\n",
                        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":4}}\n\n",
                        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"))} else {
                        json!({"id":"bad-search","model":"inference-model","type":"message","role":"assistant","stop_reason":"tool_use",
                            "content":[{"type":"tool_use","id":"bad-search-call","name":"web_search","input":{}}],"usage":{"input_tokens":3,"output_tokens":4}})
                    });
                }
                Reply::ok(inference_reply(target,body))
            }).await;
            let store = router::Store {providers:vec![card("inference",&upstream.url,"inference-model",target,protocol::ChatSearch::Auto)],
                router:router::RouterSettings {bearer_token:"router-local".into(),..Default::default()},..Default::default()};
            store_file(&paths,&store);
            let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
            let request = json!({"model":"inference::inference-model","input":"private caller context","stream":streaming,
                "max_tool_calls":1,"tool_choice":if mixed_error {json!({"type":"web_search"})} else {json!("auto")},
                "tools":[{"type":"web_search","external_web_access":true},{"type":"function","name":"read_file","parameters":{"type":"object"}}]});
            let send = |key: &'static str| client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
                .header("idempotency-key",key).json(&request).send();
            let response = send("caller-native").await.unwrap();
            let status = response.status();
            let mut text = response.text().await.unwrap();
            assert_eq!(status,StatusCode::OK,"{text}");
            if !mixed_error {
                // The reply streams through untouched (never buffered for an
                // inspection); its misclassified call teaches the next request.
                assert!(text.contains("bad-search-call") && !text.contains("Rust answer"),"{text}");
                assert_eq!(upstream.captured().len(),1);
                let response = send("caller-next").await.unwrap();
                assert_eq!(response.status(),StatusCode::OK);
                text = response.text().await.unwrap();
            }
            assert!(text.contains("web_search_call") && text.contains("https://www.rust-lang.org/releases") && text.contains("Rust answer"),"{text}");
            assert!(!text.contains("bad-search-call") && !text.contains("__agentharbor"));
            let requests = upstream.captured();
            assert_eq!(requests.len(),4);
            assert!(requests.iter().all(|request| request["body"]["model"] == "inference-model"
                && if target == ModelProtocol::AnthropicMessages {request["path"] == "/v1/messages" && request["apiKey"] == "inference-only"}
                    else {request["path"] == "/v1/responses" && request["authorization"] == "Bearer inference-only"}));
            assert_ne!(requests[0]["idempotencyKey"],requests[1]["idempotencyKey"]);
            assert_eq!(if target == ModelProtocol::Responses {&requests[2]["body"]["input"]} else {&requests[2]["body"]["messages"][0]["content"]},"Rust");
            assert_eq!(requests[2]["body"]["tools"].as_array().unwrap().len(),1);
            assert_eq!(if target == ModelProtocol::Responses {&requests[2]["body"]["max_tool_calls"]}
                else {&requests[2]["body"]["tools"][0]["max_uses"]},1);
            assert!(!requests[2]["body"].to_string().contains("private caller context"));
            stop_unchecked();
        }
        }
        }
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.request_count,32);
        assert!(ledger.recent.iter().all(|record| record.provider_id == "inference" && record.model == "inference-model"));
    });
}

#[test]
fn native_search_recovery_preserves_real_searches_and_client_functions() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let upstream = Mock::new(|_,body| {
            let text = body["messages"].to_string();
            if text.contains("authentication-case") {return Reply {status:401,body:json!({"error":{"message":"web_search authentication failure"}}),delay:Duration::ZERO};}
            if body["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {
                let mut answer = native_answer(ModelProtocol::AnthropicMessages,true);
                answer["content"].as_array_mut().unwrap().push(json!({"type":"tool_use","id":"client-file","name":"read_file","input":{"path":"a"}}));
                answer["stop_reason"] = json!("tool_use");
                return Reply::ok(answer);
            }
            Reply::ok(json!({"id":"client-search","model":"inference-model","stop_reason":"tool_use","type":"message","role":"assistant",
                "content":[{"type":"tool_use","id":"client-owned","name":"web_search","input":{"query":"Rust"}}],"usage":{"input_tokens":3,"output_tokens":4}}))
        }).await;
        let store = router::Store {providers:vec![card("inference",&upstream.url,"inference-model",ModelProtocol::AnthropicMessages,protocol::ChatSearch::Auto)],
            router:router::RouterSettings {bearer_token:"router-local".into(),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        for (index,input,tools) in [
            (0,"native-case",json!([{"type":"web_search"},{"type":"function","name":"read_file","parameters":{"type":"object"}}])),
            (1,"client-case",json!([{"type":"function","name":"web_search","parameters":{"type":"object"}}])),
            (2,"authentication-case",json!([{"type":"web_search"},{"type":"function","name":"read_file","parameters":{"type":"object"}}])),
        ] {
            let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
                .json(&json!({"model":"inference::inference-model","input":input,"tools":tools})).send().await.unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(upstream.captured().len(),index+1,"no duplicate native search or proxy execution of a client function");
            if index == 0 {assert_eq!(status,StatusCode::OK);assert!(text.contains("web_search_call") && text.contains("client-file"));}
            if index == 1 {assert_eq!(status,StatusCode::OK);assert!(text.contains("client-owned") && !text.contains("web_search_call"));}
            if index == 2 {assert_eq!(status,StatusCode::UNAUTHORIZED);}
        }
    });
}

#[test]
#[ignore = "Explicit native-search probe: small paid upstream requests, isolated local router/config"]
fn live_native_search_mixed_tools_on_original_provider() {
    let _guard = global_router_test_lock();
    let provider_id = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_PROVIDER").expect("select the tested provider explicitly");
    let model = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_MODEL").expect("select the tested model explicitly");
    let source = router::load_store(&CodexPaths::new()).unwrap();
    let mut provider = source.providers.iter().find(|provider| provider.id == provider_id).unwrap().clone();
    provider.model = model.clone();
    provider.chat_search = protocol::ChatSearch::Auto;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        let key = provider.api_key.clone();
        let store = router::Store {providers:vec![provider],router:router::RouterSettings {bearer_token:"isolated-native-probe".into(),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let response = Client::builder().no_proxy().timeout(Duration::from_secs(120)).build().unwrap()
            .post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("isolated-native-probe")
            .json(&json!({"model":format!("{provider_id}::{model}"),"input":"Use web search to find an official Rust release announcement. Reply briefly and cite sources only if provided.",
                "max_output_tokens":512,"tools":[{"type":"web_search","external_web_access":true},
                    {"type":"function","name":"read_file","description":"Read a local file; use web search for internet information.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]})).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap().replace(&key,"[redacted]");
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        println!("{}",json!({"model":model,"status":status.as_u16(),"requests":ledger.all_time.request_count,"response":text}));
        if let Ok(output) = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_OUTPUT") {
            std::fs::write(output,json!({"model":model,"status":status.as_u16(),"requests":ledger.all_time.request_count,"response":text}).to_string()).unwrap();
        }
        assert_eq!(status,StatusCode::OK);
        let reply: Value = serde_json::from_str(&text).unwrap();
        let searches: Vec<_> = reply["output"].as_array().into_iter().flatten().filter(|item| item["type"] == "web_search_call").collect();
        assert_eq!(reply["status"],"completed","{text}");
        assert!(!searches.is_empty() && searches.iter().all(|item| item["status"] == "completed"),"{text}");
        assert!(!text.contains("\"name\":\"web_search\"") && !text.contains("__agentharbor"),"{text}");
    });
}

#[test]
#[ignore = "Explicit client-tool probe: a small paid request on the selected provider; isolated config, no search service execution"]
fn live_client_search_tool_arguments_on_selected_provider() {
    let _guard = global_router_test_lock();
    let id = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_PROVIDER").expect("select a provider");
    let model = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_MODEL").expect("select a model");
    let target = match std::env::var("AGENT_HARBOR_CLIENT_TOOL_PROBE_PROTOCOL").unwrap().as_str() {
        "messages"=>ModelProtocol::AnthropicMessages,"responses"=>ModelProtocol::Responses,_=>panic!("select messages or responses"),
    };
    let source = router::load_store(&CodexPaths::new()).unwrap();
    let saved = source.providers.iter().find(|provider| provider.id == id).unwrap();
    let provider = router::Provider {model:model.clone(),protocol:target,models:vec![],..saved.clone()};
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let key = provider.api_key.clone();
        store_file(&paths,&router::Store {providers:vec![provider],router:router::RouterSettings {bearer_token:"isolated-client-tool".into(),..Default::default()},..Default::default()});
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let query_schema = json!({"type":"object","properties":{"q":{"type":"string"},"recency":{"type":["integer","null"]},
            "domains":{"type":["array","null"],"items":{"type":"string"}}},"required":["q","recency","domains"]});
        let schema = json!({"type":"object","properties":{"system2_search_query":{"anyOf":[{"type":"array","items":query_schema},{"type":"null"}]}},"required":["system2_search_query"]});
        let response = Client::builder().no_proxy().timeout(Duration::from_secs(60)).build().unwrap()
            .post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("isolated-client-tool")
            .json(&json!({"model":format!("{id}::{model}"),"stream":false,"max_output_tokens":1024,
                "input":"Call the ordinary search_news function once with system2_search_query=[{q:Rust,recency:7,domains:null}]. Do not call the built-in web_search. This is a tool parameter compatibility test; do not answer from memory.",
                "tools":[{"type":"web_search"},{"type":"namespace","name":"search_service","tools":[{"type":"function","name":"search_news","description":"An ordinary client function that accepts an array of query objects.","parameters":schema}]}]})).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap().replace(&key,"[redacted]");
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        let report = json!({"model":model,"protocol":target,"status":status.as_u16(),"requests":ledger.all_time.request_count,"response":text});
        if let Ok(output) = std::env::var("AGENT_HARBOR_NATIVE_SEARCH_PROBE_OUTPUT") {std::fs::write(output,report.to_string()).unwrap();}
        println!("{report}");
        assert_eq!(status,StatusCode::OK,"{text}");
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["status"],"completed","{text}");
        let call = reply["output"].as_array().unwrap().iter().find(|call| call["type"] == "function_call" && call["name"] == "search_news").expect("real ordinary tool call");
        assert_eq!(call["namespace"],"search_service");
        let args: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        let query = &args["system2_search_query"].as_array().expect("actual typed query array")[0];
        assert!(query["q"].as_str().is_some_and(|q| !q.is_empty()));
        assert_eq!(query["recency"],7);
        assert!(query["domains"].is_null());
    });
}

#[test]
fn selected_native_search_keeps_all_three_inference_protocols_and_separate_credentials_usage() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        for search_protocol in [ModelProtocol::Responses,ModelProtocol::AnthropicMessages] {
            let search = Mock::new(move |_,_| Reply::ok(native_answer(search_protocol,true))).await;
            for target in [ModelProtocol::ChatCompletions,ModelProtocol::AnthropicMessages,ModelProtocol::Responses] {
                let inference = Mock::new(move |_,body| Reply::ok(inference_reply(target,body))).await;
                let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",target,protocol::ChatSearch::Selected),
                    card("search",&search.url,"search-model",search_protocol,protocol::ChatSearch::Auto)],
                    router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
                store_file(&paths,&store);
                let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
                for caller in [ModelProtocol::Responses,ModelProtocol::AnthropicMessages] {
                    for streaming in [false,true] {
                        let mut inbound = if caller == ModelProtocol::Responses {
                            json!({"model":"inference::inference-model","input":"private caller context","max_tool_calls":2,"tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}})
                        } else {request(false)};
                        inbound["model"] = json!("inference::inference-model");
                        inbound["stream"] = json!(streaming);
                        let suffix = if caller == ModelProtocol::Responses {"responses"} else {"messages"};
                        let response = client.post(format!("http://127.0.0.1:{port}/v1/{suffix}")).bearer_auth("router-local").header("idempotency-key","caller-key").json(&inbound).send().await.unwrap();
                        let status = response.status();
                        let text = response.text().await.unwrap();
                        assert_eq!(status,StatusCode::OK,"{target:?}/{search_protocol:?}/{caller:?}: {text}");
                        assert!(text.contains("https://www.rust-lang.org/releases") && text.contains("Rust answer"),"{text}");
                        assert!(!text.contains("__agentharbor") && !text.contains("javascript:"));
                    }
                }
                let requests = inference.captured();
                assert_eq!(requests.len(),8);
                for pair in requests.chunks_exact(2) {
                    let expected = match target {ModelProtocol::Responses=>"/v1/responses",ModelProtocol::AnthropicMessages=>"/v1/messages",_=>"/v1/chat/completions"};
                    assert!(pair.iter().all(|request| request["path"] == expected && request["body"]["model"] == "inference-model"));
                    assert_ne!(pair[0]["idempotencyKey"],pair[1]["idempotencyKey"]);
                    assert!(pair[1]["body"].to_string().contains("Rust 1.99 fixture"));
                    if target == ModelProtocol::AnthropicMessages {assert!(pair[1]["body"].to_string().contains("inference-signature"));}
                }
                assert!(requests.iter().all(|request| if target == ModelProtocol::AnthropicMessages {request["apiKey"] == "inference-only" && request["authorization"] == ""} else {request["authorization"] == "Bearer inference-only" && request["apiKey"] == ""}));
                stop_unchecked();
            }
            let searches = search.captured();
            assert_eq!(searches.len(),12);
            for request in searches {
                assert_eq!(request["body"]["model"],"search-model");
                assert!(!request["body"].to_string().contains("private caller context") && !request["body"].to_string().contains("private inference"));
                assert!(if search_protocol == ModelProtocol::Responses {request["authorization"] == "Bearer search-only" && request["body"]["max_tool_calls"] == 2}
                    else {request["apiKey"] == "search-only" && request["body"]["tools"][0]["max_uses"] == 2});
            }
        }
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.request_count,72);
        let searches: Vec<_> = ledger.recent.iter().filter(|record| record.provider_id == "search").collect();
        assert!(!searches.is_empty());
        assert!(searches.iter().all(|record| record.model == "search-model"));
        let routes = route_trace::recent();
        let route = routes.iter().find(|route| route.attempts.iter().any(|attempt| !attempt.search_executions.is_empty())).unwrap();
        let attempt = route.attempts.iter().find(|attempt| !attempt.search_executions.is_empty()).unwrap();
        assert_eq!(attempt.provider_id,"inference");
        assert_eq!(attempt.search_executions[0].provider_id,"search");
        assert_ne!(attempt.credential.version_fingerprint,attempt.search_executions[0].credential.version_fingerprint);
    });
}

#[test]
fn selected_native_search_auto_retries_only_tool_rejection_and_requires_actual_search() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let search = Mock::new(|_,_| Reply::ok(native_answer(ModelProtocol::Responses,true))).await;
        let inference = Mock::new(|_,body| {
            if body["input"].to_string().contains("quota-test") {return Reply {status:400,body:json!({"error":{"message":"web_search quota exceeded: unsupported plan"}}),delay:Duration::ZERO};}
            if body["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {
                Reply {status:400,body:json!({"error":{"message":"web_search is unsupported on this model"}}),delay:Duration::ZERO}
            } else {Reply::ok(inference_reply(ModelProtocol::Responses,body))}
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",ModelProtocol::Responses,protocol::ChatSearch::Auto),card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto)],
            router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let _cleanup = Cleanup;
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let response = Client::builder().no_proxy().build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .header("idempotency-key","native-key").json(&json!({"model":"inference::inference-model","input":"test","tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}})).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status,StatusCode::OK,"{text}");
        assert!(text.contains("web_search_call") && text.contains("Rust answer"));
        let requests = inference.captured();
        assert_eq!(requests.len(),3);
        assert_ne!(requests[0]["idempotencyKey"],requests[1]["idempotencyKey"]);
        assert_eq!(search.captured().len(),1);
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.request_count,4);
        assert_eq!(ledger.recent.iter().filter(|record| record.provider_id == "search").count(),1);
        let response = Client::builder().no_proxy().build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .json(&json!({"model":"inference::inference-model","input":"quota-test","tools":[{"type":"web_search"}]})).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_REQUEST);
        assert_eq!(search.captured().len(),1,"quota/auth refusals must not switch search billing accounts");
        let no_search = Mock::new(|_,_| Reply::ok(native_answer(ModelProtocol::Responses,false))).await;
        let mut store = store;
        store.providers[0].chat_search = protocol::ChatSearch::Selected;
        store.providers[1].base_url = no_search.url.clone();
        store_file(&paths,&store);
        let response = Client::builder().no_proxy().build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .json(&json!({"model":"inference::inference-model","input":"test","tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}})).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_GATEWAY);
        assert!(response.text().await.unwrap().contains("search_not_executed"));
        assert_eq!(no_search.captured().len(),1);
        assert_eq!(inference.captured().len(),5,"a failed required search must not become an ordinary answer");
    });
}

fn search_sse(protocol: ModelProtocol) -> String {
    let response = native_answer(protocol,true);
    if protocol == ModelProtocol::Responses {return format!("event: response.completed\ndata: {}\n\n",json!({"type":"response.completed","response":response}))}
    let mut out = format!("event: message_start\ndata: {}\n\n",json!({"type":"message_start","message":{"id":"msg-search-sse","type":"message","role":"assistant","model":"search-model","content":[],"usage":{"input_tokens":5,"output_tokens":0}}}));
    for (index, block) in response["content"].as_array().unwrap().iter().enumerate() {
        out.push_str(&format!("event: content_block_start\ndata: {}\n\n",json!({"type":"content_block_start","index":index,"content_block":block})));
        out.push_str(&format!("event: content_block_stop\ndata: {}\n\n",json!({"type":"content_block_stop","index":index})));
    }
    out.push_str(&format!("event: message_delta\ndata: {}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":6}})));
    out
}

#[test]
fn selected_native_search_reads_native_sse_and_account_wire_on_every_inference_round() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        let kiro = Mock::new(|round,body| {
            let current = &body["conversationState"]["currentMessage"]["userInputMessage"];
            assert_eq!(current["modelId"],"claude-sonnet-4.5");
            let event = if round % 2 == 0 {
                let name = current["userInputMessageContext"]["tools"].as_array().unwrap().iter()
                    .find_map(|tool| tool["toolSpecification"]["name"].as_str().filter(|name| name.starts_with("__agentharbor"))).unwrap();
                json!({"event":"toolUseEvent","payload":{"toolUseId":"local-search","name":name,"input":{"query":"Rust"},"stop":true}})
            } else {
                assert!(body.to_string().contains("Rust 1.99 fixture"));
                json!({"event":"assistantResponseEvent","payload":{"content":"Rust answer"}})
            };
            Reply::ok(json!({"__aws_events":[event,{"event":"metadataEvent","payload":{"tokenUsage":{"inputTokens":3,"outputTokens":4}}}]}))
        }).await;
        account_sources::set_endpoint_override(account_sources::SourceKind::Kiro,Some(kiro.url.clone()));
        let mut provider = card("inference",&kiro.url,"claude-sonnet-4.5",ModelProtocol::ChatCompletions,protocol::ChatSearch::Auto);
        provider.account_source = Some(account_sources::SourceKind::Kiro);
        provider.models = vec![crate::core::newapi::Model {id:provider.model.clone(),protocol:Some(ModelProtocol::ChatCompletions),..Default::default()}];
        account_sources::store::mutate(&paths.app_data_dir,|store| {
            store.accounts.push(account_sources::store::Account {id:"kiro-account".into(),kind:account_sources::SourceKind::Kiro,name:"Kiro fixture".into(),subject:"fixture".into(),active:true,
                credentials:account_sources::store::Credentials::Kiro(account_sources::kiro::Creds {access:"kiro-account-only".into(),expires_at:chrono::Utc::now().timestamp()+3600,profile:"arn:aws:codewhisperer:us-east-1:1:profile/P".into(),..Default::default()}),plan:None,added_at:0,lapsed:None,quota:None,checkin:None,checkin_off:false});Ok(())
        }).unwrap();
        for protocol in [ModelProtocol::Responses,ModelProtocol::AnthropicMessages] {
            let search = Mock::new(move |_,_| Reply::ok(json!(search_sse(protocol)))).await;
            let store = router::Store {providers:vec![provider.clone(),card("search",&search.url,"search-model",protocol,protocol::ChatSearch::Auto)],
                router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
            store_file(&paths,&store);
            let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
            let response = Client::builder().no_proxy().build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
                .json(&json!({"model":"inference::claude-sonnet-4.5","input":"Search","tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}})).send().await.unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status,StatusCode::OK,"{text}");
            assert!(text.contains("web_search_call") && text.contains("Rust answer"));
            assert_eq!(search.captured().len(),1);
            stop_unchecked();
        }
        let requests = kiro.captured();
        assert_eq!(requests.len(),4);
        assert!(requests.iter().all(|request| request["path"] == "/generateAssistantResponse" && request["authorization"] == "Bearer kiro-account-only"));
        let ledger = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(ledger.all_time.request_count,6);
    });
}

#[test]
fn selected_native_search_account_source_uses_its_signin_and_rejects_lost_hard_budgets() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let inference = Mock::new(|_,body| Reply::ok(inference_reply(ModelProtocol::ChatCompletions,body))).await;
        let search = Mock::new(|_,body| {
            assert_eq!(body["store"],false);
            assert!(body.get("max_output_tokens").is_none());
            Reply::ok(json!(search_sse(ModelProtocol::Responses)))
        }).await;
        account_sources::set_endpoint_override(account_sources::SourceKind::ChatgptApi,Some(search.url.clone()));
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let _cleanup = Cleanup;
        account_sources::store::mutate(&paths.app_data_dir,|store| {
            store.accounts.push(account_sources::store::Account {id:"search-account".into(),kind:account_sources::SourceKind::ChatgptApi,name:"Search fixture".into(),subject:"fixture".into(),active:true,
                credentials:account_sources::store::Credentials::ChatgptApi(account_sources::chatgpt_api::Creds {access:"search-account-only".into(),expires_at:chrono::Utc::now().timestamp()+3600,..Default::default()}),plan:None,added_at:0,lapsed:None,quota:None,checkin:None,checkin_off:false});Ok(())
        }).unwrap();
        let mut selected = card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto);
        selected.account_source = Some(account_sources::SourceKind::ChatgptApi);
        let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",ModelProtocol::ChatCompletions,protocol::ChatSearch::Auto),selected],
            router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let mut inbound = json!({"model":"inference::inference-model","input":"Search","tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}});
        let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status,StatusCode::OK,"{text}");
        assert_eq!(search.captured()[0]["authorization"],"Bearer search-account-only");
        inbound["max_tool_calls"] = json!(2);
        let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_REQUEST);
        assert!(response.text().await.unwrap().contains("cannot enforce"));
        assert_eq!(search.captured().len(),1);
        assert_eq!(inference.captured().len(),2,"reject a known lost hard limit before paying for inference or search");
    });
}

#[test]
fn selected_native_search_replays_private_native_history_when_search_is_disabled() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        for target in [ModelProtocol::AnthropicMessages,ModelProtocol::Responses] {
            let inference = Mock::new(move |_,body| Reply::ok(inference_reply(target,body))).await;
            let search = Mock::new(|_,_| Reply::ok(native_answer(ModelProtocol::Responses,true))).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",target,protocol::ChatSearch::Selected),card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto)],
                router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
            store_file(&paths,&store);
            let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
            let client = Client::builder().no_proxy().build().unwrap();
            let mut inbound = request(false);
            inbound["model"] = json!("inference::inference-model");
            let response = client.post(format!("http://127.0.0.1:{port}/v1/messages")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            let visible: Value = response.json().await.unwrap();
            inbound.as_object_mut().unwrap().remove("tools");
            inbound["tool_choice"] = json!({"type":"none"});
            inbound["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":visible["content"]}),json!({"role":"user","content":"Explain this source"})]);
            let response = client.post(format!("http://127.0.0.1:{port}/v1/messages")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status,StatusCode::OK,"{target:?}: {text}");
            let requests = inference.captured();
            assert_eq!(requests.len(),3);
            let replay = requests[2]["body"].to_string();
            assert!(replay.contains(if target == ModelProtocol::Responses {"response-private"} else {"inference-signature"}),"{target:?}: {replay}");
            assert!(!replay.contains("web_search_call") && !replay.contains("server_tool_use"));
            assert!(replay.contains("Rust 1.99 fixture"));
            assert_eq!(search.captured().len(),1);
            stop_unchecked();
        }
    });
}

#[test]
fn selected_native_search_does_not_activate_an_unused_expired_search_account() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        let inference = Mock::new(|_,_| Reply::ok(native_answer(ModelProtocol::Responses,true))).await;
        let search = Mock::new(|_,_| panic!("unused fallback must not refresh credentials or call search")).await;
        account_sources::set_endpoint_override(account_sources::SourceKind::ChatgptApi,Some(search.url.clone()));
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        account_sources::store::mutate(&paths.app_data_dir,|store| {
            store.accounts.push(account_sources::store::Account {id:"search-account".into(),kind:account_sources::SourceKind::ChatgptApi,name:"Search fixture".into(),subject:"fixture".into(),active:true,
                credentials:account_sources::store::Credentials::ChatgptApi(account_sources::chatgpt_api::Creds {access:"expired-search-only".into(),refresh:"unused-refresh".into(),expires_at:0,..Default::default()}),plan:None,added_at:0,lapsed:None,quota:None,checkin:None,checkin_off:false});Ok(())
        }).unwrap();
        let mut selected = card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto);
        selected.account_source = Some(account_sources::SourceKind::ChatgptApi);
        let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",ModelProtocol::Responses,protocol::ChatSearch::Auto),selected],
            router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let response = Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .json(&json!({"model":"inference::inference-model","input":"Search","tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"}})).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        assert!(response.text().await.unwrap().contains("web_search_call"));
        assert!(search.captured().is_empty());
        assert_eq!(inference.captured().len(),1);
        let account = account_sources::store::load(&paths.app_data_dir).unwrap().accounts.remove(0);
        let account_sources::store::Credentials::ChatgptApi(creds) = account.credentials else {panic!("fixture account")};
        assert_eq!(creds.access,"expired-search-only");
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count,1);
        let chat = Mock::new(|_,_| Reply::ok(chat_reply(json!({"role":"assistant","content":"Ordinary answer"}),"stop"))).await;
        let mut store = store;
        store.providers[0].base_url = chat.url.clone();
        store.providers[0].protocol = ModelProtocol::ChatCompletions;
        store_file(&paths,&store);
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
        let inbound = json!({"model":"inference::inference-model","input":"Ordinary message","tools":[{"type":"web_search"}]});
        let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status,StatusCode::OK,"{text}");
        assert!(chat.captured()[0]["body"]["tools"].to_string().contains("__agentharbor_web_search"));
        assert!(search.captured().is_empty(),"an offered query function must not activate the account until called");
        store.providers.retain(|provider| provider.id != "search");
        store_file(&paths,&store);
        let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        assert!(chat.captured()[1]["body"].get("tools").is_none(),"an unavailable optional search service retains the ordinary send fallback");
        assert!(search.captured().is_empty());
        let mut forced = inbound;
        forced["tool_choice"] = json!({"type":"web_search"});
        let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local").json(&forced).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_REQUEST);
        assert_eq!(chat.captured().len(),2);
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count,3);
    });
}

#[test]
fn selected_native_search_mixes_client_functions_and_continues_on_the_same_aggregate_member() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        for target in [ModelProtocol::AnthropicMessages,ModelProtocol::Responses] {
            let inference = Mock::new(move |_,body| {
                if body.to_string().contains("Client result") {let mut answer=native_answer(target,false);answer["model"]=json!("inference-model");return Reply::ok(answer)}
                let mut value = inference_reply(target,body);
                let client = if target == ModelProtocol::Responses {json!({"type":"function_call","id":"fc-client","call_id":"client-exec","name":"exec","arguments":"{\"cmd\":\"pwd\"}","status":"completed"})}
                    else {json!({"type":"tool_use","id":"client-exec","name":"exec","input":{"cmd":"pwd"}})};
                value[if target == ModelProtocol::Responses {"output"} else {"content"}].as_array_mut().unwrap().push(client);
                Reply::ok(value)
            }).await;
            let search = Mock::new(|_,_| Reply::ok(native_answer(ModelProtocol::Responses,true))).await;
            let temp = tempfile::tempdir().unwrap();
            let paths = CodexPaths::from_home(temp.path().join("codex"));
            std::fs::create_dir_all(&paths.app_data_dir).unwrap();
            let mut store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",target,protocol::ChatSearch::Selected),card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto)],
                router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
            store.aggregates.push(aggregation::AggregateModel {id:uuid::Uuid::new_v4().to_string(),public_model_id:"mixed-search".into(),display_name:"Mixed search".into(),
                members:vec![aggregation::AggregateMember {id:"member-inference".into(),provider_id:"inference".into(),actual_model_id:"inference-model".into(),..Default::default()}],..Default::default()});
            store_file(&paths,&store);
            let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
            let client = Client::builder().no_proxy().build().unwrap();
            let mut inbound = request(false);
            inbound["model"] = json!("mixed-search");
            inbound["tool_choice"] = json!({"type":"auto"});
            inbound["tools"].as_array_mut().unwrap().push(json!({"name":"exec","input_schema":{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}}));
            let response = client.post(format!("http://127.0.0.1:{port}/v1/messages")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
            let status = response.status();
            let visible: Value = response.json().await.unwrap();
            assert_eq!(status,StatusCode::OK,"{target:?}: {visible}");
            assert_eq!(visible["stop_reason"],"tool_use");
            assert!(visible["content"].as_array().unwrap().iter().any(|block| block["type"] == "tool_use" && block["id"] == "client-exec" && block["name"] == "exec"));
            assert!(visible.to_string().contains("web_search_tool_result"));
            assert_eq!(inference.captured().len(),1,"client functions are never executed or followed up by the broker");
            inbound["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":visible["content"]}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"client-exec","content":"Client result"}]})]);
            let response = client.post(format!("http://127.0.0.1:{port}/v1/messages")).bearer_auth("router-local").json(&inbound).send().await.unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status,StatusCode::OK,"{target:?}: {text}");
            assert!(text.contains("Rust answer"));
            assert_eq!(search.captured().len(),1);
            let requests = inference.captured();
            assert_eq!(requests.len(),2);
            let replay = requests[1]["body"].to_string();
            assert!(replay.contains(if target == ModelProtocol::Responses {"response-private"} else {"inference-signature"}),"{target:?}: {replay}");
            assert!(replay.contains("Client result") && replay.contains("Rust 1.99 fixture"));
            stop_unchecked();
        }
    });
}

#[test]
fn selected_native_search_stops_the_call_group_after_an_incomplete_search() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        let inference = Mock::new(|round,body| if round == 0 {Reply::ok(chat_reply(json!({"role":"assistant","content":null,
            "tool_calls":[search_call(body,"local-first","Rust"),search_call(body,"local-second","release")]}),"tool_calls"))}
            else {Reply::ok(chat_reply(json!({"role":"assistant","content":"Search unavailable"}),"stop"))}).await;
        let search = Mock::new(|_,_| {
            let mut value = native_answer(ModelProtocol::Responses,true);
            value["status"] = json!("incomplete");
            value["incomplete_details"] = json!({"reason":"max_output_tokens"});
            let mut second = value["output"][0].clone();
            second["id"] = json!("actual-second");
            value["output"].as_array_mut().unwrap().insert(1,second);
            Reply::ok(value)
        }).await;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let store = router::Store {providers:vec![card("inference",&inference.url,"inference-model",ModelProtocol::ChatCompletions,protocol::ChatSearch::Selected),card("search",&search.url,"search-model",ModelProtocol::Responses,protocol::ChatSearch::Auto)],
            router:router::RouterSettings {bearer_token:"router-local".into(),web_search_provider_id:Some("search".into()),web_search_model_id:Some("search-model".into()),..Default::default()},..Default::default()};
        store_file(&paths,&store);
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let response = Client::builder().no_proxy().build().unwrap().post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .json(&json!({"model":"inference::inference-model","input":"Search","max_tool_calls":2,"tools":[{"type":"web_search"}]})).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        let value: Value = response.json().await.unwrap();
        let calls: Vec<_> = value["output"].as_array().unwrap().iter().filter(|item| item["type"] == "web_search_call").collect();
        assert_eq!(calls.len(),2);
        assert!(calls.iter().all(|call| call["status"] == "failed"));
        assert_eq!(search.captured().len(),1,"an incomplete search may have consumed the entire native budget");
        assert_eq!(search.captured()[0]["body"]["max_tool_calls"],2);
        let requests = inference.captured();
        assert_eq!(requests.len(),2);
        assert!(requests[1]["body"].to_string().contains("search_stopped_after_failure"));
        assert_eq!(relay_usage::load(&paths.app_data_dir).unwrap().all_time.request_count,3);
    });
}

/// Upstream that sends its first SSE frame, then holds the rest of the reply.
async fn held_stream_upstream(first: String, rest: String, hold: Duration) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
    let url = format!("http://{}",listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut socket,_)) = listener.accept().await {
            let (first,rest) = (first.clone(),rest.clone());
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt,AsyncWriteExt};
                let (mut request,mut buffer) = (Vec::new(),[0u8;8192]);
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {return}
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = head.lines().find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok()).unwrap_or(0);
                        if request.len() >= end + 4 + length {break}
                    }
                }
                let chunk = |text: &str| format!("{:x}\r\n{text}\r\n",text.len());
                socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{}",chunk(&first)).as_bytes()).await.unwrap();
                socket.flush().await.unwrap();
                tokio::time::sleep(hold).await;
                let _ = socket.write_all(format!("{}0\r\n\r\n",chunk(&rest)).as_bytes()).await;
            });
        }
    });
    (url,task)
}

#[test]
fn search_capable_requests_still_stream_and_known_rejections_are_not_repeated() {
    let _guard = global_router_test_lock();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _cleanup = Cleanup;
        let temp = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(temp.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let client = Client::builder().no_proxy().timeout(Duration::from_secs(20)).build().unwrap();
        let tools = json!([{"type":"web_search","external_web_access":false},
            {"type":"function","name":"read_file","parameters":{"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"}}}}}]);

        // A same-protocol Codex turn with hosted search and client functions:
        // the first frame must arrive while the upstream is still generating.
        let message = json!({"id":"msg-held","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"held answer","annotations":[]}]});
        let response = json!({"id":"resp-held","object":"response","status":"completed","model":"inference-model","output":[message],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}});
        let first = format!("event: response.created\ndata: {}\n\n",json!({"type":"response.created","response":{"id":"resp-held","status":"in_progress","model":"inference-model"}}));
        let rest = [("response.output_item.added",json!({"type":"response.output_item.added","output_index":0,"item":{"id":"msg-held","type":"message","role":"assistant","status":"in_progress","content":[]}})),
            ("response.output_text.delta",json!({"type":"response.output_text.delta","item_id":"msg-held","output_index":0,"content_index":0,"delta":"held answer"})),
            ("response.output_item.done",json!({"type":"response.output_item.done","output_index":0,"item":message})),
            ("response.completed",json!({"type":"response.completed","response":response}))]
            .iter().map(|(kind,value)| format!("event: {kind}\ndata: {value}\n\n")).collect::<String>();
        let (url,_upstream) = held_stream_upstream(first,rest,Duration::from_secs(3)).await;
        store_file(&paths,&router::Store {providers:vec![card("held",&url,"inference-model",ModelProtocol::Responses,protocol::ChatSearch::Auto)],
            router:router::RouterSettings {bearer_token:"router-local".into(),..Default::default()},..Default::default()});
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        let started = std::time::Instant::now();
        let mut response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
            .json(&json!({"model":"held::inference-model","input":"hi","stream":true,"tools":tools})).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        let first = response.chunk().await.unwrap().unwrap();
        let first_latency = started.elapsed();
        let mut text = String::from_utf8_lossy(&first).into_owned();
        while let Some(chunk) = response.chunk().await.unwrap() {text.push_str(&String::from_utf8_lossy(&chunk));}
        assert!(first_latency < Duration::from_secs(2),"first frame waited for the whole reply: {first_latency:?}");
        assert!(text.starts_with("event: response.created") && text.contains("response.completed") && text.contains("held answer"),"{text}");
        stop_unchecked();

        // MiMo Chat with its search plugin off rejects the declaration with a
        // reason only in `param`. The message still goes through, and the next
        // turn omits the declaration instead of repeating the rejected request.
        let upstream = Mock::new(|_,body| {
            if body["tools"].as_array().is_some_and(|tools| tools.iter().any(protocol::web_search::is_search)) {
                return Reply {status:400,body:json!({"error":{"code":"400","message":"Param Incorrect",
                    "param":"web search tool found in the request body, but webSearchEnabled is false","type":""}}),delay:Duration::ZERO};
            }
            Reply::ok(chat_reply(json!({"role":"assistant","content":"plain answer"}),"stop"))
        }).await;
        store_file(&paths,&router::Store {providers:vec![card("mimo",&upstream.url,"mimo-v2.6-flash",ModelProtocol::ChatCompletions,protocol::ChatSearch::Mimo)],
            router:router::RouterSettings {bearer_token:"router-local".into(),..Default::default()},..Default::default()});
        let port = ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        for (turn,expected) in [(0,2),(1,3)] {
            let response = client.post(format!("http://127.0.0.1:{port}/v1/responses")).bearer_auth("router-local")
                .json(&json!({"model":"mimo::mimo-v2.6-flash","input":format!("turn {turn}"),"tools":tools})).send().await.unwrap();
            assert_eq!(response.status(),StatusCode::OK,"turn {turn}");
            let text = response.text().await.unwrap();
            assert!(text.contains("plain answer") && !text.contains("web_search_call"),"{text}");
            assert_eq!(upstream.captured().len(),expected,"turn {turn}");
        }
        let captured = upstream.captured();
        assert!(captured[0]["body"]["tools"].to_string().contains("force_search"),"the first turn tries the native contract");
        assert!(captured[1..].iter().all(|request| !request["body"].to_string().contains("force_search")
            && request["body"]["tools"].to_string().contains("read_file")),"client functions stay");
        stop_unchecked();
    });
}
