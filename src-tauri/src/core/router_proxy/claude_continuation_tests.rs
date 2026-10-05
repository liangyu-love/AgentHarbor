//! Real localhost HTTP requests; no external upstream or user credentials.
use super::*;
use std::convert::Infallible;

#[test]
fn gemini_nested_array_schema_survives_messages_chat_and_responses_routes() {
    let _lock = global_router_test_lock();
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let dir=tempfile::tempdir().unwrap();
        let paths=CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let observed=Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured=observed.clone();
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let port=listener.local_addr().unwrap().port();
        let server=tokio::spawn(async move {
            while let Ok((socket,_))=listener.accept().await {
                let captured=captured.clone();
                tokio::spawn(async move {
                    let service=service_fn(move |req:Request<Incoming>| {
                        let captured=captured.clone();
                        async move {
                            let path=req.uri().path().to_owned();
                            let body:Value=serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
                            let schema=match path.as_str() {
                                "/v1/messages"=>&body["tools"][0]["input_schema"],
                                "/v1/chat/completions"=>&body["tools"][0]["function"]["parameters"],
                                "/v1/responses"=>&body["tools"][0]["parameters"],
                                _=>panic!("unexpected fixture path"),
                            };
                            assert_eq!(schema["type"],"object");
                            assert_eq!(schema["properties"]["__agentharbor_arguments_json"]["type"],"string", "restricted Google Schema requires a concrete type");
                            assert!(schema["properties"]["__agentharbor_arguments_json"]["description"].as_str().unwrap().contains("where"));
                            let input=&body[if path=="/v1/responses" {"input"} else {"messages"}];
                            let continued=input.to_string().contains("tool_result") || input.to_string().contains("function_call_output") || input.as_array().unwrap().iter().any(|m|m["role"]=="tool");
                            captured.lock().unwrap().push(json!({"path":path,"continued":continued,"schema":schema}));
                            if continued { assert!(input.to_string().contains("__agentharbor_arguments_json"), "tool replay must use the upstream envelope"); }
                            let args=json!({"__agentharbor_arguments_json":json!({"query":{"where":[1,true,{"x":"y"}]}}).to_string()});
                            let canonical=json!({"object":"response","id":"resp-schema","status":"completed","model":"gemini-3.8-flash-high","output":if continued {
                                vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"schema fixture passed"}]})]
                            } else {vec![json!({"type":"function_call","id":"fc-schema","call_id":"lookup1","name":"lookup","arguments":args.to_string(),"status":"completed"})]}});
                            let response=match path.as_str() {
                                "/v1/responses"=>canonical,
                                "/v1/chat/completions"=>json!({"model":"gemini-3.8-flash-high","choices":[{"finish_reason":if continued {"stop"} else {"tool_calls"},"message":if continued {json!({"role":"assistant","content":"schema fixture passed"})} else {json!({"role":"assistant","content":null,"tool_calls":[{"type":"function","id":"lookup1","function":{"name":"lookup","arguments":args.to_string()}}]})}}]}),
                                _=>json!({"type":"message","id":"msg-schema","role":"assistant","model":"gemini-3.8-flash-high","stop_reason":if continued {"end_turn"} else {"tool_use"},"content":if continued {vec![json!({"type":"text","text":"schema fixture passed"})]} else {vec![json!({"type":"tool_use","id":"lookup1","name":"lookup","input":args})]},"usage":{"input_tokens":1,"output_tokens":1}}),
                            };
                            let (content_type, reply) = if body["stream"] == true {
                                let frames = match path.as_str() {
                                    "/v1/responses" => vec![json!({"type":"response.completed","response":response})],
                                    "/v1/chat/completions" => if continued {
                                        vec![json!({"model":"gemini-3.8-flash-high","choices":[{"index":0,"delta":{"role":"assistant","content":"schema fixture passed"},"finish_reason":null}]}), json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})]
                                    } else {
                                        vec![json!({"model":"gemini-3.8-flash-high","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"lookup1","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]}),json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":args.to_string()}}]},"finish_reason":null}]}),json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})]
                                    },
                                    _ => {
                                        let mut frames=vec![json!({"type":"message_start","message":{"id":"msg-schema","type":"message","role":"assistant","model":"gemini-3.8-flash-high","content":[],"usage":{"input_tokens":1,"output_tokens":0}}})];
                                        if continued {
                                            frames.extend([json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"schema fixture passed"}})]);
                                        } else {
                                            frames.extend([json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"lookup1","name":"lookup","input":{}}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":args.to_string()}})]);
                                        }
                                        frames.extend([json!({"type":"content_block_stop","index":0}),json!({"type":"message_delta","delta":{"stop_reason":if continued {"end_turn"} else {"tool_use"}},"usage":{"output_tokens":1}}),json!({"type":"message_stop"})]); frames
                                    }
                                };
                                ("text/event-stream",sse(frames,path=="/v1/chat/completions"))
                            } else { ("application/json",response.to_string()) };
                            Ok::<_,Infallible>(Response::builder().header(CONTENT_TYPE,content_type).body(Full::new(Bytes::from(reply))).unwrap())
                        }
                    });
                    http1::Builder::new().serve_connection(TokioIo::new(socket),service).await.unwrap();
                });
            }
        });
        let protocols=[ModelProtocol::ChatCompletions,ModelProtocol::Responses,ModelProtocol::AnthropicMessages];
        let providers=protocols.into_iter().enumerate().map(|(i,protocol)|Provider {id:format!("schema{i}"),name:"fixture".into(),base_url:format!("http://127.0.0.1:{port}"),api_key:"fixture-key".into(),model:"gemini-3.8-flash-high".into(),protocol,enabled:true,network_mode:"direct".into(),..Default::default()}).collect();
        let store=Store {providers,router:RouterSettings {bearer_token:"local-fixture".into(),..Default::default()},..Default::default()};
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),&serde_json::to_vec(&store).unwrap()).unwrap();
        let port=ensure_running(0,paths,ApiProxyConfigPayload::default()).await.unwrap();
        struct Stop;
        impl Drop for Stop {fn drop(&mut self){stop();}}
        let _stop=Stop;
        let client=Client::builder().no_proxy().build().unwrap();
        for streaming in [false,true] { for i in 0..3 {
            let mut body=json!({"model":format!("schema{i}::gemini-3.8-flash-high"),"max_tokens":128,"tools":[{"name":"lookup","input_schema":{"type":"object","properties":{"query":{"type":"object","properties":{"where":{"type":"array"}}},"typed":{"type":"array","items":{"type":"integer"}}}}}],"messages":[{"role":"user","content":"use lookup"}]});
            body["stream"] = json!(streaming);
            let url=format!("http://127.0.0.1:{port}/v1/messages");
            let response=client.post(&url).header("x-api-key","local-fixture").json(&body).send().await.unwrap();
            assert!(response.status().is_success());
            let content = if streaming {
                let text=response.text().await.unwrap();
                assert!(!text.contains("__agentharbor_arguments_json"), "internal envelope must not reach Claude");
                let values:Vec<Value>=text.lines().filter_map(|line|line.strip_prefix("data: ")).map(|line|serde_json::from_str(line).unwrap()).collect();
                assert_eq!(values.last().unwrap()["type"],"message_stop");
                let input=values.iter().filter(|v|v["type"]=="content_block_delta" && v["delta"]["type"]=="input_json_delta").filter_map(|v|v["delta"]["partial_json"].as_str()).collect::<String>();
                let input:Value=serde_json::from_str(&input).unwrap();
                json!([{"type":"tool_use","id":"lookup1","name":"lookup","input":input}])
            } else { response.json::<Value>().await.unwrap()["content"].clone() };
            assert_eq!(content[0]["input"]["query"]["where"],json!([1,true,{"x":"y"}]));
            body["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":content}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"lookup1","content":"result"}]})]);
            let response=client.post(&url).header("x-api-key","local-fixture").json(&body).send().await.unwrap();
            assert!(response.status().is_success());
            assert!(response.text().await.unwrap().contains("schema fixture passed"));
        }
        }
        assert_eq!(observed.lock().unwrap().len(),12);
        server.abort();
    });
}
use crate::core::router::{ModelProtocol, Provider, RouterSettings, Store};

fn sse(frames: Vec<Value>, chat: bool) -> String {
    let mut out = frames
        .iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect::<String>();
    if chat {
        out.push_str("data: [DONE]\n\n");
    }
    out
}
pub(super) fn reply(body: &Value, chat: bool, mode: &str) -> (String, String) {
    if mode == "deepseek-history" {
        return deepseek_history_reply(body, chat);
    }
    let streaming = body["stream"] == true;
    let continued = if chat {
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool")
    } else {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["type"] == "function_call_output")
    };
    let arguments = if mode == "bad-tool" { "{" } else { "{}" };
    let canonical = json!({"object":"response","id":"resp-fixture","model":"fixture-model","status":"completed",
        "output":if continued {vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]})]}
        else {vec![json!({"type":"reasoning","id":"rs-fixture","encrypted_content":"opaque-fixture","summary":[{"type":"summary_text","text":"required reasoning"}]}),
            json!({"type":"function_call","id":"fc-fixture","call_id":"c1","name":"lookup","arguments":arguments,"status":"completed"})]},
        "usage":{"input_tokens":3,"output_tokens":4}});
    if mode == "bad-json" {
        return (
            "application/json".into(),
            json!({"unusable":true,"usage":{"prompt_tokens":3,"completion_tokens":4}}).to_string(),
        );
    }
    if streaming {
        let mut frames = if chat {
            vec![
                json!({"model":"fixture-model","choices":[{"delta":if continued {json!({"content":"done"})} else {
            json!({"reasoning_content":"required reasoning","tool_calls":[{"index":0,"id":"c1","function":{"name":"lookup","arguments":arguments}}]})},
            "finish_reason":if continued {"stop"} else {"tool_calls"}}]}),
                json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}),
            ]
        } else {
            vec![json!({"type":"response.completed","response":canonical})]
        };
        if mode == "upstream-error" {
            frames = vec![json!({"error":{"message":"fixture failure"}})];
        }
        return (
            "text/event-stream".into(),
            sse(frames, chat && mode != "truncated"),
        );
    }
    let value = if chat {
        json!({"model":"fixture-model","choices":[{"finish_reason":if continued {"stop"} else {"tool_calls"},"message":if continued {
        json!({"role":"assistant","content":"done"})} else {json!({"role":"assistant","content":null,"reasoning_content":"required reasoning",
            "tool_calls":[{"id":"c1","type":"function","function":{"name":"lookup","arguments":arguments}}]})}}],"usage":{"prompt_tokens":3,"completion_tokens":4}})
    } else {
        canonical
    };
    ("application/json".into(), value.to_string())
}

fn deepseek_history_reply(body: &Value, chat: bool) -> (String, String) {
    let items = body[if chat { "messages" } else { "input" }]
        .as_array()
        .unwrap();
    let assistant_count = items
        .iter()
        .filter(|v| v["role"] == "assistant" || v["type"] == "function_call")
        .count();
    let has_results = items
        .iter()
        .any(|v| v["role"] == "tool" || v["type"] == "function_call_output");
    let valid = if chat {
        items.iter().filter(|v| v["role"] == "assistant").all(|v| {
            v["reasoning_content"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        })
    } else {
        let reasoning: Vec<_> = items.iter().filter(|v| v["type"] == "reasoning").collect();
        reasoning.len() == assistant_count
            && reasoning.iter().all(|v| {
                v["content"][0]["type"] == "reasoning_text"
                    && v["content"][0]["text"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
            })
            && (assistant_count == 0 || items.iter().any(|v| v["id"] == "msg-greeting"))
            && (!has_results || items.iter().any(|v| v["id"] == "fc-tool"))
    };
    if !valid {
        return (
            "application/json".into(),
            json!({"error":{"message":"fixture_missing_reasoning_text"}}).to_string(),
        );
    }
    let greeting = assistant_count == 0;
    let text = if greeting { "hello" } else { "done" };
    let reasoning = if greeting {
        "greeting reasoning"
    } else {
        "tool reasoning"
    };
    let tool = !greeting && !has_results;
    let output = vec![
        json!({"type":"reasoning","id":if greeting {"rs-greeting"} else {"rs-tool"},"status":"completed","content":[{"type":"reasoning_text","text":reasoning}],"summary":[]}),
        if tool {
            json!({"type":"function_call","id":"fc-tool","call_id":"c1","name":"lookup","arguments":"{}","status":"completed"})
        } else {
            json!({"type":"message","id":if greeting {"msg-greeting"} else {"msg-final"},"role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})
        },
    ];
    let canonical = json!({"object":"response","id":"fixture","model":"fixture-model","status":"completed","output":output,"usage":{"input_tokens":3,"output_tokens":4}});
    let chat_reply = json!({"model":"fixture-model","choices":[{"finish_reason":if tool {"tool_calls"} else {"stop"},"message":if tool {
        json!({"role":"assistant","reasoning_content":reasoning,"tool_calls":[{"id":"c1","type":"function","function":{"name":"lookup","arguments":"{}"}}]})}
        else {json!({"role":"assistant","content":text,"reasoning_content":reasoning})}}],"usage":{"prompt_tokens":3,"completion_tokens":4}});
    if body["stream"] != true {
        return (
            "application/json".into(),
            if chat { chat_reply } else { canonical }.to_string(),
        );
    }
    let frames = if chat {
        vec![
            json!({"model":"fixture-model","choices":[{"delta":if tool {
        json!({"reasoning_content":reasoning,"tool_calls":[{"index":0,"id":"c1","function":{"name":"lookup","arguments":"{}"}}]})}
        else {json!({"content":text,"reasoning_content":reasoning})},"finish_reason":if tool {"tool_calls"} else {"stop"}}]}),
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}),
        ]
    } else {
        vec![json!({"type":"response.completed","response":canonical})]
    };
    ("text/event-stream".into(), sse(frames, chat))
}

pub(super) fn streamed_content(text: &str) -> Value {
    let mut blocks = Vec::new();
    let mut arguments = std::collections::HashMap::<usize, String>::new();
    for line in text.lines().filter_map(|s| s.strip_prefix("data: ")) {
        let v: Value = serde_json::from_str(line).unwrap();
        let index = v["index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str() {
            Some("content_block_start") => {
                assert_eq!(index, blocks.len());
                blocks.push(v["content_block"].clone());
            }
            Some("content_block_delta") if v["delta"]["type"] == "input_json_delta" => arguments
                .entry(index)
                .or_default()
                .push_str(v["delta"]["partial_json"].as_str().unwrap()),
            Some("content_block_delta") => {
                let old = blocks[index]["text"].as_str().unwrap().to_owned();
                blocks[index]["text"] = json!(old + v["delta"]["text"].as_str().unwrap());
            }
            Some("error") => panic!("unexpected conversion error: {v}"),
            _ => {}
        }
    }
    for (index, args) in arguments {
        blocks[index]["input"] = serde_json::from_str(&args).unwrap();
    }
    json!(blocks)
}

#[test]
fn messages_tool_continuations_and_translation_failures_over_http() {
    let _lock = global_router_test_lock();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir=tempfile::tempdir().unwrap();
        let paths=CodexPaths::from_home(dir.path().join("codex"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let captured=Arc::new(Mutex::new(Vec::<Value>::new()));
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let port=listener.local_addr().unwrap().port();
        let capture=captured.clone();
        let server=tokio::spawn(async move {
            while let Ok((socket,_))=listener.accept().await {
                let capture=capture.clone();
                tokio::spawn(async move {
                    let service=service_fn(move |request:Request<Incoming>| {
                        let capture=capture.clone();
                        async move {
                            assert_eq!(request.headers()[AUTHORIZATION],"Bearer fixture-key");
                            let chat=request.uri().path().contains("chat/completions");
                            let mode=request.headers().get("x-fixture-mode").and_then(|v|v.to_str().ok()).unwrap_or("normal").to_owned();
                            let bytes=request.into_body().collect().await.unwrap().to_bytes();
                            let body:Value=serde_json::from_slice(&bytes).unwrap();
                            capture.lock().unwrap().push(body.clone());
                            let (content_type,body)=reply(&body,chat,&mode);
                            let status=if body.contains("fixture_missing_reasoning_text") {400} else {200};
                            Ok::<_,std::convert::Infallible>(Response::builder().status(status).header(CONTENT_TYPE,content_type).body(Full::new(Bytes::from(body))).unwrap())
                        }
                    });
                    let _=http1::Builder::new().serve_connection(TokioIo::new(socket),service).await;
                });
            }
        });
        let providers=[ModelProtocol::ChatCompletions,ModelProtocol::Responses].into_iter().enumerate().map(|(i,protocol)|Provider {
            id:format!("p{i}"),name:"fixture".into(),base_url:format!("http://127.0.0.1:{port}"),api_key:"fixture-key".into(),model:"fixture-model".into(),protocol,enabled:true,network_mode:"direct".into(),..Default::default()
        }).collect();
        let store=Store {providers,router:RouterSettings {bearer_token:"local-fixture".into(),..Default::default()},..Default::default()};
        crate::core::secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"),&serde_json::to_vec(&store).unwrap()).unwrap();
        let router_port=ensure_running(0,paths.clone(),ApiProxyConfigPayload::default()).await.unwrap();
        struct Stop;
        impl Drop for Stop {fn drop(&mut self) {stop();}}
        let _stop=Stop;
        let client=Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();
        let url=format!("http://127.0.0.1:{router_port}/v1/messages");
        for i in 0..2 { for streaming in [false,true] {
            let mut request=json!({"model":format!("p{i}::fixture-model"),"max_tokens":128,"stream":streaming,"thinking":{"type":"adaptive"},
                "messages":[{"role":"user","content":format!("case {i}/{streaming}")}]});
            let first=client.post(&url).header("x-api-key","local-fixture").json(&request).send().await.unwrap();
            assert!(first.status().is_success());
            let text=first.text().await.unwrap();
            let content=if streaming {streamed_content(&text)} else {serde_json::from_str::<Value>(&text).unwrap()["content"].clone()};
            request["messages"].as_array_mut().unwrap().extend([
                json!({"role":"assistant","content":content}),
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"result"}]})]);
            let next=client.post(&url).header("x-api-key","local-fixture").json(&request).send().await.unwrap();
            assert!(next.status().is_success());
            assert!(next.text().await.unwrap().contains("done"));
            let sent=captured.lock().unwrap().last().unwrap().clone();
            if i==0 {assert_eq!(sent["messages"][1]["reasoning_content"],"required reasoning");}
            else {
                let reasoning=sent["input"].as_array().unwrap().iter().find(|v|v["type"]=="reasoning").unwrap();
                assert_eq!(reasoning["encrypted_content"],"opaque-fixture");
                assert_eq!(reasoning["id"],"rs-fixture");
            }
        }}
        let usage=relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.successful_count,8);
        for i in 0..2 {for streaming in [false,true] {
            let mut request=json!({"model":format!("p{i}::fixture-model"),"max_tokens":128,"stream":streaming,"thinking":{"type":"adaptive"},
                "tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":format!("greeting {i}/{streaming}")}]});
            for step in 0..3 {
                let reply=client.post(&url).header("x-api-key","local-fixture").header("x-fixture-mode","deepseek-history").json(&request).send().await.unwrap();
                assert!(reply.status().is_success(),"full reasoning history required at step {step}");
                let text=reply.text().await.unwrap();
                let content=if streaming {streamed_content(&text)} else {serde_json::from_str::<Value>(&text).unwrap()["content"].clone()};
                if step==2 {assert!(text.contains("done"));break;}
                request["messages"].as_array_mut().unwrap().push(json!({"role":"assistant","content":content}));
                request["messages"].as_array_mut().unwrap().push(if step==0 {json!({"role":"user","content":"use a tool"})}
                    else {json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"result"}]})});
            }
        }}
        for mode in ["bad-tool","truncated","upstream-error","bad-json"] {
            let request=json!({"model":"p0::fixture-model","max_tokens":128,"stream":mode!="bad-json","messages":[{"role":"user","content":mode}]});
            let result=client.post(&url).header("x-api-key","local-fixture").header("x-fixture-mode",mode).json(&request).send().await.unwrap();
            if mode=="bad-json" {assert_eq!(result.status(),StatusCode::BAD_GATEWAY);} else {assert!(result.text().await.unwrap().contains("event: error"));}
            let routes=route_trace::recent();
            let trace=routes.iter().find(|r|r.requested_model=="p0::fixture-model").unwrap();
            assert!(trace.error.is_some(),"{mode}");
            assert_eq!(trace.failure_stage.as_deref(),Some(if mode=="upstream-error" {"response"} else {"translation"}),"{mode}");
            let usage=relay_usage::load(&paths.app_data_dir).unwrap();
            assert_eq!(usage.all_time.successful_count,20,"{mode}");
        }
        let usage=relay_usage::load(&paths.app_data_dir).unwrap();
        assert_eq!(usage.all_time.request_count,24);
        assert_eq!(usage.all_time.input_tokens,69);
        assert_eq!(usage.all_time.output_tokens,92);
        server.abort();
    });
}

#[test]
fn interrupted_delivery_keeps_usage_but_is_not_successful() {
    let dir = tempfile::tempdir().unwrap();
    let trace = route_trace::Trace::begin("interrupted-fixture", None);
    let request = relay_usage::RelayUsageRequest::new(
        dir.path().to_owned(),
        "fixture".into(),
        "Fixture".into(),
        "model".into(),
        "chatCompletions".into(),
        relay_usage::PricingSnapshot::unknown("unknown", "fixture"),
    );
    {
        let mut observer = ResponseObservation::new(
            trace,
            Some(request),
            ModelProtocol::ChatCompletions,
            "model",
            "attempt",
            200,
            true,
        );
        observer.push(
            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4}}\n\n",
        );
        // Client drops the body before the terminal protocol event.
    }
    let usage = relay_usage::load(dir.path()).unwrap();
    assert_eq!(usage.all_time.successful_count, 0);
    assert_eq!(usage.all_time.total_tokens, 7);
    assert_eq!(usage.recent[0].status_code, 200);
}
