//! Opt-in installed CLI check. Every upstream and credential is a local fixture.
use super::*;
use crate::core::{
    newapi::Model,
    router::{ModelProtocol, Provider, RouterSettings, Store},
};
use std::convert::Infallible;

#[test]
#[ignore = "requires CLAUDE_ROUTER_TEST_EXE pointing to an installed Claude Code executable"]
fn installed_claude_cli_receives_translated_streams() {
    run_installed_cli(false, false);
}

#[test]
#[ignore = "requires CLAUDE_ROUTER_TEST_EXE pointing to an installed Claude Code executable"]
fn installed_claude_cli_executes_tool_and_replays_reasoning() {
    run_installed_cli(true, false);
}

#[test]
#[ignore = "requires CLAUDE_ROUTER_TEST_EXE pointing to an installed Claude Code executable"]
fn installed_claude_cli_replays_greeting_before_deepseek_tool_continuation() {
    run_installed_cli(true, true);
}

#[test]
#[ignore = "requires CLAUDE_ROUTER_TEST_EXE pointing to an installed Claude Code executable"]
fn installed_claude_cli_deepseek_messages_supports_direct_and_routed_tools() {
    let executable =
        std::env::var_os("CLAUDE_ROUTER_TEST_EXE").expect("set CLAUDE_ROUTER_TEST_EXE");
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let dir=tempfile::tempdir().unwrap();
        let paths=CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let file=dir.path().join("messages-read-fixture.txt");
        std::fs::write(&file,"native messages fixture content").unwrap();
        let args=json!({"file_path":file}).to_string();
        let observed=Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured=observed.clone();
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let upstream_port=listener.local_addr().unwrap().port();
        let server=tokio::spawn(async move {
            while let Ok((socket,_))=listener.accept().await {
                let captured=captured.clone();let args=args.clone();
                tokio::spawn(async move {
                    let service=service_fn(move |req:Request<Incoming>| {
                        let captured=captured.clone();let args=args.clone();
                        async move {
                            // Optional Claude Code server probe; this fixture
                            // deliberately exposes only the Messages endpoint.
                            if req.uri().path() == "/anthropic/api/hello" {
                                return Ok::<_,Infallible>(Response::builder().status(StatusCode::NOT_FOUND).header(CONTENT_TYPE,"application/json").body(Full::new(Bytes::from_static(br#"{"error":{"message":"fixture only supports Messages"}}"#))).unwrap());
                            }
                            assert_eq!(req.uri().path(),"/anthropic/v1/messages");
                            assert_eq!(req.headers()["x-api-key"],"fixture-native-key");
                            let body:Value=serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
                            assert_eq!(body["model"],"deepseek-flash","[1m] must not reach the upstream model ID");
                            let continued=body["messages"].to_string().contains("native messages fixture content");
                            captured.lock().unwrap().push(json!({"model":body["model"],"continued":continued}));
                            let frames=vec![
                                ("message_start",json!({"message":{"id":"msg-native","type":"message","role":"assistant","model":"deepseek-flash","content":[],"stop_reason":null,"usage":{"input_tokens":2,"output_tokens":0}}})),
                                ("content_block_start",json!({"index":0,"content_block":if continued {json!({"type":"text","text":""})} else {json!({"type":"tool_use","id":"native-read","name":"Read","input":{}})}})),
                                ("content_block_delta",json!({"index":0,"delta":if continued {json!({"type":"text_delta","text":"Native Messages fixture passed"})} else {json!({"type":"input_json_delta","partial_json":args})}})),
                                ("content_block_stop",json!({"index":0})),
                                ("message_delta",json!({"delta":{"stop_reason":if continued {"end_turn"} else {"tool_use"},"stop_sequence":null},"usage":{"output_tokens":1}})),
                                ("message_stop",json!({})),
                            ];
                            let mut sse=String::new();
                            for (kind,mut value) in frames {value["type"]=json!(kind);sse.push_str(&format!("event: {kind}\ndata: {value}\n\n"));}
                            Ok::<_,Infallible>(Response::builder().header(CONTENT_TYPE,"text/event-stream").body(Full::new(Bytes::from(sse))).unwrap())
                        }
                    });
                    http1::Builder::new().serve_connection(TokioIo::new(socket),service).await.unwrap();
                });
            }
        });
        let store=Store {providers:vec![Provider {id:"native-fixture".into(),name:"fixture".into(),base_url:format!("http://127.0.0.1:{upstream_port}/anthropic"),api_key:"fixture-native-key".into(),model:"deepseek-flash".into(),protocol:ModelProtocol::AnthropicMessages,enabled:true,network_mode:"direct".into(),..Default::default()}],router:RouterSettings {bearer_token:"fixture-router-key".into(),..Default::default()},..Default::default()};
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),&serde_json::to_vec(&store).unwrap()).unwrap();
        let router_port=ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        struct Stop;impl Drop for Stop {fn drop(&mut self){stop();}}
        let _stop=Stop;
        for routed in [false,true] {
            let mut command=tokio::process::Command::new(&executable);
            for (key,_) in std::env::vars_os() {
                let name=key.to_string_lossy().to_uppercase();
                if ["ANTHROPIC","CLAUDE","HTTP_PROXY","HTTPS_PROXY","ALL_PROXY"].iter().any(|prefix|name.starts_with(prefix)){command.env_remove(key);}
            }
            command.current_dir(dir.path()).kill_on_drop(true)
                .env("ANTHROPIC_BASE_URL",if routed {format!("http://127.0.0.1:{router_port}/p/native-fixture")} else {format!("http://127.0.0.1:{upstream_port}/anthropic")})
                .env("ANTHROPIC_API_KEY",if routed {"fixture-router-key"} else {"fixture-native-key"})
                .env("CLAUDE_CONFIG_DIR",dir.path().join(format!("native-{routed}")))
                .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC","1").env("NO_PROXY","127.0.0.1,localhost")
                .args(["-p","Read the fixture file","--model","deepseek-flash[1m]","--system-prompt","Local native Messages diagnostic","--tools","Read","--allowedTools","Read","--max-turns","3","--setting-sources","","--strict-mcp-config","--mcp-config","{\"mcpServers\":{}}","--no-session-persistence"]);
            #[cfg(windows)] command.creation_flags(0x08000000);
            let output=tokio::time::timeout(Duration::from_secs(45),command.output()).await.expect("CLI timeout").unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stdout));
            assert!(String::from_utf8_lossy(&output.stdout).contains("Native Messages fixture passed"));
        }
        let captured=observed.lock().unwrap();
        assert_eq!(captured.len(),4);assert_eq!(captured.iter().filter(|v|v["continued"]==true).count(),2);
        let usage=relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.request_count,2);assert_eq!(usage.all_time.successful_count,2);
        server.abort();
    });
}

fn run_installed_cli(tool_mode: bool, conversation: bool) {
    let executable =
        std::env::var_os("CLAUDE_ROUTER_TEST_EXE").expect("set CLAUDE_ROUTER_TEST_EXE");
    let _lock = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let fixture_file = dir.path().join("router-read-fixture.txt");
        std::fs::write(&fixture_file, "local fixture content").unwrap();
        let tool_arguments = json!({"file_path":fixture_file}).to_string();
        let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_port = listener.local_addr().unwrap().port();
        let captured = observed.clone();
        let upstream = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let captured = captured.clone();
                let tool_arguments = tool_arguments.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let captured = captured.clone();
                        let tool_arguments = tool_arguments.clone();
                        async move {
                            assert_eq!(req.headers()[AUTHORIZATION], "Bearer fixture-upstream-key");
                            assert!(req.headers().get("x-api-key").is_none());
                            let is_chat = req.uri().path() == "/v1/chat/completions";
                            assert!(is_chat || req.uri().path() == "/v1/responses");
                            let bytes = req.into_body().collect().await.unwrap().to_bytes();
                            let body: Value = serde_json::from_slice(&bytes).unwrap();
                            let model = body["model"].as_str().unwrap();
                            let messages = if is_chat { &body["messages"] } else { &body["input"] };
                            assert!(messages.as_array().unwrap().iter().any(|m| m["role"] == "user"));
                            assert!(messages.as_array().unwrap().iter().any(|m| m["role"] == "system"));
                            assert!(!body["tools"].as_array().unwrap().is_empty());
                            let continued = messages.as_array().unwrap().iter().any(|m| m["role"] == "tool" || m["type"] == "function_call_output");
                            let replayed = if is_chat {messages.as_array().unwrap().iter().any(|m|m["reasoning_content"]=="fixture private reasoning")}
                                else {messages.as_array().unwrap().iter().any(|m|m["type"]=="reasoning" && (m["encrypted_content"]=="fixture opaque reasoning" || m["content"][0]["text"]=="fixture private reasoning"))};
                            let previous_assistant = messages.as_array().unwrap().iter().any(|m|m["role"]=="assistant" || m["type"]=="function_call");
                            let greeting_replayed = if is_chat { messages.as_array().unwrap().iter().any(|m|m["reasoning_content"]=="fixture greeting reasoning") }
                                else { messages.as_array().unwrap().iter().any(|m|m["id"]=="rs-greeting" && m["content"][0]["type"]=="reasoning_text" && m["content"][0]["text"]=="fixture greeting reasoning") };
                            let read_ok = messages.to_string().contains("local fixture content");
                            // Retain only structure, never request content or credentials.
                            captured.lock().unwrap().push(json!({
                                "continued":continued,"replayed":replayed,"readOk":read_ok,"previousAssistant":previous_assistant,"greetingReplayed":greeting_replayed,
                                "model":model,"protocol":if is_chat {"chat"} else {"responses"},
                                "effort":if is_chat {&body["reasoning_effort"]} else {&body["reasoning"]["effort"]},
                                "roles":messages.as_array().unwrap().iter().filter_map(|m|m["role"].as_str()).collect::<Vec<_>>()
                            }));
                            let text = "Local router fixture passed";
                            let frames = if tool_mode && !continued && (!conversation || previous_assistant) {
                                if is_chat { vec![
                                    json!({"model":model,"choices":[{"delta":{"reasoning_content":"fixture private reasoning","tool_calls":[{"index":0,"id":"fixture-read","function":{"name":"Read","arguments":tool_arguments}}]},"finish_reason":"tool_calls"}]}),
                                    json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5}})
                                ] } else { vec![json!({"type":"response.completed","response":{"object":"response","id":"resp-fixture-tool","model":model,"status":"completed",
                                    "output":[{"type":"reasoning","id":"rs-fixture","encrypted_content":if conversation {Value::Null} else {json!("fixture opaque reasoning")},"content":if conversation {json!([{"type":"reasoning_text","text":"fixture private reasoning"}])} else {Value::Null},"summary":[]},
                                        {"type":"function_call","id":"fc-fixture","call_id":"fixture-read","name":"Read","arguments":tool_arguments,"status":"completed"}],
                                    "usage":{"input_tokens":20,"output_tokens":5}}})] }
                            } else if conversation && !previous_assistant {
                                if is_chat {vec![json!({"model":model,"choices":[{"delta":{"content":text,"reasoning_content":"fixture greeting reasoning"},"finish_reason":"stop"}]}),
                                    json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5}})]}
                                else {vec![json!({"type":"response.completed","response":{"object":"response","id":"resp-greeting","model":model,"status":"completed",
                                    "output":[{"type":"reasoning","id":"rs-greeting","status":"completed","content":[{"type":"reasoning_text","text":"fixture greeting reasoning"}],"summary":[]},
                                        {"type":"message","id":"msg-greeting","status":"completed","role":"assistant","content":[{"type":"output_text","text":text,"annotations":[]}]}],
                                    "usage":{"input_tokens":20,"output_tokens":5}}})]}
                            } else if is_chat {
                                vec![
                                    json!({"id":"chat-fixture","model":model,"choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}),
                                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                                    json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5}}),
                                ]
                            } else {
                                vec![
                                    json!({"type":"response.created","response":{"id":"resp-fixture","model":model}}),
                                    json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":text}),
                                    json!({"type":"response.completed","response":{"id":"resp-fixture","model":model,"status":"completed",
                                        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],
                                        "usage":{"input_tokens":20,"output_tokens":5}}}),
                                ]
                            };
                            let mut stream = frames.iter().map(|frame|format!("data: {frame}\n\n")).collect::<String>();
                            if is_chat { stream.push_str("data: [DONE]\n\n"); }
                            Ok::<_, std::convert::Infallible>(Response::builder().header(CONTENT_TYPE,"text/event-stream").body(Full::new(Bytes::from(stream))).unwrap())
                        }
                    });
                    http1::Builder::new().serve_connection(TokioIo::new(socket),service).await.unwrap();
                });
            }
        });
        let models = ["deepseek-flash", "gpt-6-luna", "glm-5.3-flash", "gemini-3.8-flash"];
        let providers = [ModelProtocol::ChatCompletions, ModelProtocol::Responses].into_iter().enumerate().map(|(index, protocol)|Provider {
            id:format!("fixture{index}"),name:format!("Local {index}"),
            base_url:format!("http://127.0.0.1:{upstream_port}/"),api_key:"fixture-upstream-key".into(),
            enabled:true,model:models[0].into(),protocol,network_mode:"direct".into(),
            models:models.iter().map(|id|Model {id:(*id).into(),..Default::default()}).collect(),
            ..Default::default()
        }).collect();
        let store = Store {
            providers, router:RouterSettings {bearer_token:"fixture-router-key".into(),..Default::default()},
            ..Default::default()
        };
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),&serde_json::to_vec(&store).unwrap()).unwrap();
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        struct StopRouter;
        impl Drop for StopRouter { fn drop(&mut self) { stop(); } }
        let _stop = StopRouter;
        for index in 0..2 {
            for model in models {
                let session_id=uuid::Uuid::new_v4().to_string();
                for phase in 0..if conversation {2} else {1} {
                let mut command = tokio::process::Command::new(&executable);
                for (key, _) in std::env::vars_os() {
                    let name = key.to_string_lossy().to_uppercase();
                    if ["ANTHROPIC", "CLAUDE", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"].iter().any(|prefix|name.starts_with(prefix)) { command.env_remove(key); }
                }
                command.current_dir(dir.path()).kill_on_drop(true)
                    .env("ANTHROPIC_BASE_URL",format!("http://127.0.0.1:{port}"))
                    .env("ANTHROPIC_API_KEY","fixture-router-key")
                    .env("CLAUDE_CONFIG_DIR",dir.path().join(format!("claude-{index}-{model}")))
                    .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC","1")
                    .env("NO_PROXY","127.0.0.1,localhost")
                    .args(["-p",if phase==0 {"Hello"} else {"Read the fixture file"},"--system-prompt","Local router diagnostic","--model",&format!("fixture{index}::{model}"),
                        "--effort","max","--setting-sources","","--strict-mcp-config","--mcp-config","{\"mcpServers\":{}}"]);
                if conversation { command.args([if phase==0 {"--session-id"} else {"--resume"}, &session_id]); }
                else { command.arg("--no-session-persistence"); }
                if tool_mode { command.args(["--tools","Read","--allowedTools","Read","--max-turns","3"]); }
                #[cfg(windows)]
                command.creation_flags(0x08000000);
                let output = tokio::time::timeout(Duration::from_secs(45),command.output()).await.expect("CLI timeout").unwrap();
                assert!(output.status.success(),"CLI failed: {}",String::from_utf8_lossy(&output.stdout));
                assert!(String::from_utf8_lossy(&output.stdout).contains("Local router fixture passed"));
                }
            }
        }
        let captured = observed.lock().unwrap();
        for index in 0..2 {
            for model in models {
                let entry = captured.iter().find(|entry|entry["model"] == model && entry["protocol"] == if index == 0 {"chat"} else {"responses"}).expect("upstream request captured");
                assert_eq!(entry["effort"],if model == "gemini-3.8-flash" {"high"} else {"max"},"{entry}");
            }
        }
        if tool_mode {
            let continued: Vec<_> = captured.iter().filter(|e| e["continued"] == true).collect();
            assert_eq!(continued.len(),8,"each CLI must execute the tool and send a second request");
            assert!(continued.iter().all(|e|e["replayed"]==true),"missing reasoning: {continued:?}");
            assert!(continued.iter().all(|e|e["readOk"]==true),"fixture file was not read: {continued:?}");
        }
        if conversation {
            let followups:Vec<_>=captured.iter().filter(|e|e["previousAssistant"]==true).collect();
            assert_eq!(followups.len(),16);
            assert!(followups.iter().all(|e|e["greetingReplayed"]==true),"greeting reasoning was dropped: {followups:?}");
        }
        let factor = if conversation {3} else if tool_mode {2} else {1};
        let usage = relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.request_count, 8 * factor);
        assert_eq!(usage.all_time.successful_count, 8 * factor);
        assert_eq!(usage.all_time.input_tokens, 160 * factor);
        assert_eq!(usage.all_time.output_tokens, 40 * factor);
        let routes = route_trace::recent();
        for index in 0..2 {
            for model in models {
                let route = routes.iter().find(|route|route.requested_model == format!("fixture{index}::{model}")).unwrap();
                assert_eq!(route.status,Some(200));
                assert!(route.done);
                let sent_key = if index == 0 {"reasoning_effort"} else {"reasoning.effort"};
                assert_eq!(route.attempts.last().unwrap().sent[sent_key],if model == "gemini-3.8-flash" {"high"} else {"max"});
            }
        }
        println!("{}",serde_json::to_string(&*captured).unwrap());
        upstream.abort();
    });
}

#[test]
#[ignore = "requires CLAUDE_ROUTER_TEST_EXE pointing to an installed Claude Code executable"]
fn installed_claude_cli_web_search_runs_on_a_mimo_chat_contract() {
    // Claude Code (also Desktop's bundled agent) runs WebSearch as a separate
    // forced hosted-search request with max_uses 8. A Chat model with a native
    // search contract must answer it as Claude search blocks the CLI accepts.
    let executable = std::env::var_os("CLAUDE_ROUTER_TEST_EXE").expect("set CLAUDE_ROUTER_TEST_EXE");
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_port = listener.local_addr().unwrap().port();
        let captured = observed.clone();
        let upstream = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let captured = captured.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let captured = captured.clone();
                        async move {
                            assert_eq!(req.uri().path(), "/v1/chat/completions");
                            let bytes = req.into_body().collect().await.unwrap().to_bytes();
                            let body: Value = serde_json::from_slice(&bytes).unwrap();
                            let model = body["model"].as_str().unwrap_or_default().to_owned();
                            let tools = body["tools"].as_array().cloned().unwrap_or_default();
                            let search = tools.iter().find(|tool| tool["type"] == "web_search").cloned();
                            let main_loop = tools.iter().any(|tool| tool["function"]["name"] == "WebSearch");
                            let continued = body["messages"].as_array().into_iter().flatten().any(|m| m["role"] == "tool");
                            captured.lock().unwrap().push(json!({"search":search,"mainLoop":main_loop,"continued":continued,
                                "toolResult":body["messages"].as_array().into_iter().flatten().filter(|m| m["role"] == "tool")
                                    .map(|m| m["content"].to_string()).collect::<String>()}));
                            let frames = if search.is_some() {
                                vec![json!({"id":"chat-search","model":model,"choices":[{"index":0,"delta":{"role":"assistant","content":"Rust 1.99 shipped.",
                                        "annotations":[{"type":"url_citation","url":"https://www.rust-lang.org/","title":"Rust","summary":"Release"}]},"finish_reason":null}]}),
                                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                                        "usage":{"prompt_tokens":20,"completion_tokens":5,"web_search_usage":{"tool_usage":1,"page_usage":1}}})]
                            } else if main_loop && !continued {
                                vec![json!({"model":model,"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-search","type":"function",
                                        "function":{"name":"WebSearch","arguments":"{\"query\":\"Rust release\"}"}}]},"finish_reason":"tool_calls"}]}),
                                    json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5}})]
                            } else {
                                vec![json!({"id":"chat-final","model":model,"choices":[{"index":0,"delta":{"role":"assistant","content":"Local search fixture passed"},"finish_reason":null}]}),
                                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":5}})]
                            };
                            let mut stream = frames.iter().map(|frame| format!("data: {frame}\n\n")).collect::<String>();
                            stream.push_str("data: [DONE]\n\n");
                            Ok::<_, std::convert::Infallible>(Response::builder().header(CONTENT_TYPE, "text/event-stream").body(Full::new(Bytes::from(stream))).unwrap())
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(TokioIo::new(socket), service).await;
                });
            }
        });
        let store = Store {
            providers: vec![Provider { id:"mimo".into(), name:"Local MiMo".into(), base_url:format!("http://127.0.0.1:{upstream_port}/"),
                api_key:"fixture-upstream-key".into(), enabled:true, model:"mimo-v2.6-flash".into(), protocol:ModelProtocol::ChatCompletions,
                chat_search:crate::core::protocol::ChatSearch::Mimo, network_mode:"direct".into(),
                models:vec![Model {id:"mimo-v2.6-flash".into(), ..Default::default()}], ..Default::default() }],
            router: RouterSettings { bearer_token:"fixture-router-key".into(), ..Default::default() },
            ..Default::default()
        };
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &serde_json::to_vec(&store).unwrap()).unwrap();
        let port = ensure_running(0, paths.clone(), ApiProxyConfigPayload::default()).await.unwrap();
        struct StopRouter;
        impl Drop for StopRouter { fn drop(&mut self) { stop(); } }
        let _stop = StopRouter;
        let mut command = tokio::process::Command::new(&executable);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_uppercase();
            if ["ANTHROPIC", "CLAUDE", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"].iter().any(|prefix| name.starts_with(prefix)) { command.env_remove(key); }
        }
        command.current_dir(dir.path()).kill_on_drop(true)
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("ANTHROPIC_API_KEY", "fixture-router-key")
            .env("CLAUDE_CONFIG_DIR", dir.path().join("claude-search"))
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .args(["-p", "Search the web for the latest Rust release.", "--system-prompt", "Local router diagnostic",
                "--model", "mimo::mimo-v2.6-flash", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
                "--no-session-persistence", "--tools", "WebSearch", "--allowedTools", "WebSearch", "--max-turns", "3"]);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let output = tokio::time::timeout(Duration::from_secs(60), command.output()).await.expect("CLI timeout").unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let captured = observed.lock().unwrap().clone();
        assert!(output.status.success() && stdout.contains("Local search fixture passed"),
            "CLI failed: {stdout} {} {captured:?}", String::from_utf8_lossy(&output.stderr));
        let search = captured.iter().find(|entry| !entry["search"].is_null()).expect("the CLI sent its WebSearch request");
        // Claude Code forces the search only for Claude model names (Desktop's
        // aliases); a routed non-Claude name gets tool_choice auto.
        assert_eq!(search["search"], json!({"type":"web_search","force_search":false,"max_keyword":8}));
        let result = captured.iter().find(|entry| entry["mainLoop"] == true && entry["continued"] == true).expect("search result returned to the main loop");
        assert!(result["toolResult"].as_str().unwrap().contains("rust-lang.org"), "{result}");
        upstream.abort();
        eprintln!("installed Claude CLI WebSearch through a MiMo Chat contract: {} upstream requests", captured.len());
    });
}
