use super::*;
use serde_json::{json, Value};

fn pair(to: ModelProtocol) -> Translation {
    Translation {
        from: ModelProtocol::AnthropicMessages,
        to,
    }
}
fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn values(events: &[String]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| event.lines().find_map(|line| line.strip_prefix("data: ")))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}
fn tool() -> Value {
    json!({"type":"function_call","id":"fc1","call_id":"c1","name":"read_file","arguments":"{\"path\":\"a\"}","status":"completed"})
}

fn search_request() -> Value {
    json!({"model":"m","max_tokens":128,"messages":[{"role":"user","content":"Search Rust"}],
        "tools":[{"type":"web_search_20250305","name":"web_search_next","max_uses":2,
            "allowed_domains":["rust-lang.org"],"user_location":{"type":"approximate","country":"CN"}}],
        "tool_choice":{"type":"tool","name":"web_search_next"}})
}

fn search_output() -> Vec<Value> {
    vec![json!({"type":"web_search_call","id":"ws1","status":"completed",
        "action":{"type":"search","queries":["Rust 新版"],"sources":[{"type":"url","url":"https://rust-lang.org","title":"Rust"}]}}),
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"新版已发布。",
            "annotations":[{"type":"url_citation","start_index":0,"end_index":6,"url":"https://rust-lang.org","title":"Rust"}]}]})]
}

#[test]
fn hosted_search_maps_native_tool_options_and_forced_choice() {
    let request = request_body(pair(ModelProtocol::Responses), &search_request()).unwrap();
    assert_eq!(request.body["tools"][0], json!({"type":"web_search","filters":{"allowed_domains":["rust-lang.org"]},"user_location":{"type":"approximate","country":"CN"}}));
    assert_eq!(request.body["max_tool_calls"], 2);
    assert_eq!(request.body["tool_choice"], json!("required"));
    assert_eq!(request.body["include"], json!(["web_search_call.action.sources"]));
    assert_eq!(request.response_options.search.unwrap().name, "web_search_next");
    assert_eq!(request_body(pair(ModelProtocol::ChatCompletions), &search_request()).err().unwrap(), "protocol.errors.unsupportedHostedTool");
    let mut custom = search_request();
    custom["tools"] = json!([{"name":"web_search_next","input_schema":{"type":"object"}}]);
    assert_eq!(request_body(pair(ModelProtocol::Responses), &custom).unwrap().body["tool_choice"], json!({"type":"function","name":"web_search_next"}));
}

#[test]
fn forced_hosted_search_does_not_allow_a_different_tool_to_satisfy_required() {
    let mut value = search_request();
    value["tools"].as_array_mut().unwrap().push(json!({
        "type":"custom","name":"read_file","input_schema":{"type":"object"}
    }));
    let request = request_body(pair(ModelProtocol::Responses), &value).unwrap();
    assert_eq!(request.body["tool_choice"], "required");
    assert_eq!(request.body["tools"], json!([{"type":"web_search",
        "filters":{"allowed_domains":["rust-lang.org"]},
        "user_location":{"type":"approximate","country":"CN"}}]));
}

#[test]
fn hosted_search_rejects_lossy_options_and_invalid_limits() {
    for changes in [json!({"type":"web_search_20991231"}), json!({"blocked_domains":["example.org"]}),
        json!({"allowed_callers":["code_execution_20260120"]}),
        json!({"response_inclusion":"excluded"}), json!({"defer_loading":true}),
        json!({"max_uses":0}), json!({"max_uses":1.5}), json!({"allowed_domains":5})] {
        let mut request = search_request();
        request["tools"][0].as_object_mut().unwrap().extend(changes.as_object().unwrap().clone());
        assert!(request_body(pair(ModelProtocol::Responses), &request).is_err(), "{changes}");
    }
    let mut request = search_request();
    request["tools"][0]["type"] = json!("web_search_20260318");
    request["tools"][0]["allowed_callers"] = json!(["direct"]);
    assert!(request_body(pair(ModelProtocol::Responses), &request).is_ok());
    // Newer versions default to programmatic callers; without a code
    // execution tool in the request every call is still direct.
    for version in ["web_search_20260209", "web_search_20260318"] {
        let tool = json!({"type":version,"name":"web_search"});
        assert!(web_search::prepare(ModelProtocol::Responses, &tool, false).is_ok());
        assert!(web_search::prepare(ModelProtocol::Responses, &tool, true).is_err());
        let mut request = search_request();
        request["tools"][0]["type"] = json!(version);
        request["tools"][0].as_object_mut().unwrap().remove("allowed_callers");
        assert!(request_body(pair(ModelProtocol::Responses), &request).is_ok(), "{version}");
    }
    assert!(web_search::programmatic_callers(&[json!({"type":"code_execution_20260120","name":"code_execution"})]));
}

#[test]
fn hosted_and_function_tools_cannot_share_a_name() {
    let mut request = search_request();
    request["tools"].as_array_mut().unwrap().push(json!({
        "name":"web_search_next","input_schema":{"type":"object"}
    }));
    assert_eq!(request_body(pair(ModelProtocol::Responses), &request).err().unwrap(), "protocol.errors.invalidRequest");
}

#[test]
fn hosted_search_nonstream_retains_sources_errors_and_actual_usage() {
    let request = request_body(pair(ModelProtocol::Responses), &search_request()).unwrap();
    let mut output = search_output();
    let mut failed = output[0].clone();
    failed["id"] = json!("ws2"); failed["status"] = json!("failed");
    output.insert(1, failed);
    let converted = response_body_with_options(pair(ModelProtocol::Responses), &response(output), &request.response_options).unwrap();
    assert_eq!(converted["content"][0]["type"], "server_tool_use");
    assert_eq!(converted["content"][0]["name"], "web_search_next");
    assert_eq!(converted["content"][1]["content"][0]["url"], "https://rust-lang.org");
    assert_eq!(converted["content"][3]["content"]["error_code"], "unavailable");
    assert!(converted["content"][4]["text"].as_str().unwrap().contains("[1](https://rust-lang.org)"));
    assert_eq!(converted["stop_reason"], "end_turn");
    assert_eq!(converted["usage"]["server_tool_use"]["web_search_requests"], 2);
}

#[test]
fn hosted_search_stream_matches_nonstream_under_every_byte_split() {
    let request = request_body(pair(ModelProtocol::Responses), &search_request()).unwrap();
    let output = search_output();
    let final_response = response(output.clone());
    let frames = frame(json!({"type":"response.created","response":{"model":"m"}}))
        + &frame(json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws1","status":"in_progress"}}))
        + &frame(json!({"type":"response.web_search_call.searching","output_index":0,"item_id":"ws1"}))
        + &frame(json!({"type":"response.output_item.done","output_index":0,"item":output[0]}))
        + &frame(json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"新版已发布。"}))
        // Citations may arrive only with the terminal response.
        + &frame(json!({"type":"response.output_item.done","output_index":1,"item":{"type":"message","content":[{"type":"output_text","text":"新版已发布。"}]}}))
        + &frame(json!({"type":"response.completed","response":final_response}));
    let expected = response_body_with_options(pair(ModelProtocol::Responses), &final_response, &request.response_options).unwrap();
    for split in 0..=frames.len() {
        let mut stream = stream_translator_with_options(pair(ModelProtocol::Responses), request.response_options.clone());
        let mut events = stream.push(&frames.as_bytes()[..split]);
        events.extend(stream.push(&frames.as_bytes()[split..]));
        assert_eq!(stream.failure(), None, "split {split}: {}", events.concat());
        let values = values(&events);
        assert_eq!(values.last().unwrap()["type"], "message_stop");
        let blocks: Vec<_> = values.iter().filter(|v| v["type"] == "content_block_start").collect();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["content_block"], expected["content"][0]);
        assert_eq!(blocks[1]["content_block"], expected["content"][1]);
        let text: String = values.iter().filter_map(|v| v["delta"]["text"].as_str()).collect();
        assert_eq!(text, expected["content"][2]["text"]);
        assert_eq!(stream.completed_response(), Some(&final_response));
    }
}

#[test]
fn hosted_search_history_replays_exact_output_and_isolates_different_searches() {
    use super::message_history::Context;
    let scope = uuid::Uuid::new_v4().to_string();
    let original = search_request();
    let request = request_body(pair(ModelProtocol::Responses), &original).unwrap();
    let upstream = response(search_output());
    let visible = response_body_with_options(pair(ModelProtocol::Responses), &upstream, &request.response_options).unwrap();
    Context::new(&[&scope], ModelProtocol::Responses, original.to_string().as_bytes()).unwrap().remember(&upstream);
    let mut next = original;
    next["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":visible["content"]}), json!({"role":"user","content":"More details"})]);
    let mut body = request_body(pair(ModelProtocol::Responses), &next).unwrap().body;
    Context::new(&[&scope], ModelProtocol::Responses, next.to_string().as_bytes()).unwrap().restore(&mut body);
    assert_eq!(body["input"][1], upstream["output"][0]);
    assert_eq!(body["input"][2], upstream["output"][1]);
    next["messages"][1]["content"][0]["input"]["query"] = json!("Different search");
    let mut other = request_body(pair(ModelProtocol::Responses), &next).unwrap().body;
    Context::new(&[&scope], ModelProtocol::Responses, next.to_string().as_bytes()).unwrap().restore(&mut other);
    assert_eq!(other["input"][1]["action"]["query"], "Different search");
}

#[test]
fn hosted_search_waits_for_late_sources_and_citation_only_gateways() {
    let request = request_body(pair(ModelProtocol::Responses), &search_request()).unwrap();
    for late_sources in [false, true] {
        let mut output = search_output();
        output[0]["action"].as_object_mut().unwrap().remove("sources");
        let early = output[0].clone();
        if late_sources {
            output[0]["action"]["sources"] = json!([{"type":"url","url":"https://rust-lang.org"}]);
        }
        let final_response = response(output);
        let expected = response_body_with_options(pair(ModelProtocol::Responses), &final_response, &request.response_options).unwrap();
        assert_eq!(expected["content"][1]["content"][0]["title"], "Rust");
        let mut stream = stream_translator_with_options(pair(ModelProtocol::Responses), request.response_options.clone());
        let mut events = stream.push(frame(json!({"type":"response.created","response":{"model":"m"}})).as_bytes());
        events.extend(stream.push(frame(json!({"type":"response.output_item.done","output_index":0,"item":early})).as_bytes()));
        events.extend(stream.push(frame(json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"新版已发布。"})).as_bytes()));
        assert!(!events.concat().contains("content_block_start"));
        assert!(events.concat().contains("event: ping"));
        events.extend(stream.push(frame(json!({"type":"response.completed","response":final_response})).as_bytes()));
        assert_eq!(stream.failure(), None);
        let blocks: Vec<Value> = values(&events).into_iter().filter(|v| v["type"] == "content_block_start").collect();
        assert_eq!(blocks[0]["index"], 0);
        assert_eq!(blocks[1]["index"], 1);
        assert_eq!(blocks[1]["content_block"], expected["content"][1]);
        assert_eq!(blocks[2]["index"], 2);
    }
}

#[test]
fn malformed_chat_tool_histories_are_rejected_without_fabricating_results() {
    let call = json!({"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"read","input":{}}]});
    for messages in [json!([call]), json!([call,{"role":"user","content":"go on"}]),
        json!([call,{"role":"user","content":[{"type":"tool_result","tool_use_id":"wrong","content":"ok"}]}])] {
        let request = json!({"model":"m","max_tokens":128,"messages":messages});
        for target in [ModelProtocol::ChatCompletions, ModelProtocol::Responses] {
            assert_eq!(request_body(pair(target), &request).err().unwrap(), "protocol.errors.unansweredToolCall");
        }
    }
}

#[test]
fn responses_tool_history_pairs_parallel_calls_and_rejects_duplicate_or_orphan_results() {
    let base = json!({"model":"m","max_tokens":128,"messages":[
        {"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"read","input":{}},{"type":"tool_use","id":"c2","name":"read","input":{}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"c2","content":"two"},{"type":"tool_result","tool_use_id":"c1","content":"one"}]}]});
    assert!(request_body(pair(ModelProtocol::Responses), &base).is_ok());
    let mut duplicate = base.clone();
    duplicate["messages"][1]["content"].as_array_mut().unwrap().push(json!({"type":"tool_result","tool_use_id":"c1","content":"again"}));
    assert!(request_body(pair(ModelProtocol::Responses), &duplicate).is_err());
    let mut reused = base.clone();
    reused["messages"][0]["content"][1]["id"] = json!("c1");
    assert!(request_body(pair(ModelProtocol::Responses), &reused).is_err());
    let orphan = json!({"model":"m","max_tokens":128,"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"orphan"}]}]});
    assert!(request_body(pair(ModelProtocol::Responses), &orphan).is_err());
}

#[test]
fn search_limits_are_per_request_and_history_context_does_not_translate_messages() {
    let mut request = search_request();
    request["tools"][0]["max_uses"] = json!(20);
    let visible = response_body_with_options(pair(ModelProtocol::Responses), &response(search_output()), &request_body(pair(ModelProtocol::Responses), &request).unwrap().response_options).unwrap();
    request["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":visible["content"]}), json!({"role":"user","content":"Search again"})]);
    assert_eq!(request_body(pair(ModelProtocol::Responses), &request).unwrap().body["max_tool_calls"], 20);
    // Missing max_tokens proves Context only reads tools, not a second complete
    // request conversion. It still records the configured hosted tool name.
    request.as_object_mut().unwrap().remove("max_tokens");
    let scope = uuid::Uuid::new_v4().to_string();
    let context = super::message_history::Context::new(&[&scope], ModelProtocol::Responses, request.to_string().as_bytes()).unwrap();
    let mut output = search_output();
    output[0]["action"]["opaque_marker"] = json!("must-replay");
    let upstream = response(output);
    let options = request_body(pair(ModelProtocol::Responses), &search_request()).unwrap().response_options;
    let visible = response_body_with_options(pair(ModelProtocol::Responses), &upstream, &options).unwrap();
    context.remember(&upstream);
    request["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":visible["content"]}), json!({"role":"user","content":"Continue"})]);
    request["max_tokens"] = json!(128);
    let mut body = request_body(pair(ModelProtocol::Responses), &request).unwrap().body;
    super::message_history::Context::new(&[&scope], ModelProtocol::Responses, request.to_string().as_bytes()).unwrap().restore(&mut body);
    assert!(body.to_string().contains("must-replay"));
}

#[test]
fn interleaved_text_and_tools_stay_in_one_chat_assistant_turn() {
    let a = json!({"type":"tool_use","id":"a","name":"one","input":{}});
    let b = json!({"type":"tool_use","id":"b","name":"two","input":{}});
    let text = json!({"type":"text","text":"between"});
    for blocks in [
        vec![a.clone(), text.clone(), b.clone()],
        vec![a.clone(), b.clone(), text.clone()],
        vec![
            text.clone(),
            a.clone(),
            text.clone(),
            b.clone(),
            text.clone(),
        ],
    ] {
        let source = json!({"model":"m","max_tokens":128,"messages":[
            {"role":"user","content":"start"}, {"role":"assistant","content":blocks},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"1"},{"type":"tool_result","tool_use_id":"b","content":"2"}]}]});
        let body = request_body(pair(ModelProtocol::ChatCompletions), &source)
            .unwrap()
            .body;
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "{body}");
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[2]["tool_call_id"], "a");
        assert_eq!(messages[3]["tool_call_id"], "b");
    }
}
fn response(output: Vec<Value>) -> Value {
    json!({"object":"response","id":"resp1","status":"completed","model":"upstream-model","output":output,
        "usage":{"input_tokens":20,"output_tokens":4,"input_tokens_details":{"cached_tokens":6}}})
}

#[test]
fn messages_tools_images_and_results_translate_to_both_upstreams() {
    let source = json!({"model":"m","max_tokens":128,"stream":true,"system":[{"type":"text","text":"Be precise"}],
    "tools":[{"name":"read_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}],
    "tool_choice":{"type":"any","disable_parallel_tool_use":true},
    "messages":[
        {"role":"user","content":[{"type":"text","text":"Inspect"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}}]},
        {"role":"assistant","content":[{"type":"text","text":"Checking"},{"type":"tool_use","id":"c1","name":"read_file","input":{"path":"a"}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","is_error":true,"content":"missing"}]}
    ]});
    let responses = request_body(pair(ModelProtocol::Responses), &source).unwrap();
    assert!(
        responses.input.is_empty(),
        "Claude conversations must not enter Codex continuation history"
    );
    let r = responses.body;
    assert_eq!(r["instructions"], "Be precise");
    assert_eq!(
        r["input"][0]["content"][1]["image_url"],
        "data:image/png;base64,YWJj"
    );
    assert_eq!(r["input"][2]["call_id"], "c1");
    assert!(r["input"][3]["output"]
        .as_str()
        .unwrap()
        .contains("Tool error:"));
    assert_eq!(r["tool_choice"], "required");
    assert_eq!(r["parallel_tool_calls"], false);
    let chat = request_body(pair(ModelProtocol::ChatCompletions), &source)
        .unwrap()
        .body;
    assert_eq!(chat["messages"][0]["content"], "Be precise");
    assert_eq!(
        chat["messages"][1]["content"][1]["image_url"]["url"],
        "data:image/png;base64,YWJj"
    );
    assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "c1");
    assert_eq!(chat["messages"][3]["tool_call_id"], "c1");
    assert_eq!(chat["stream_options"]["include_usage"], true);
}

#[test]
fn current_claude_system_messages_and_effort_translate_without_reordering() {
    // Structure observed from a local-only Claude Code capture.
    for model in [
        "deepseek-flash",
        "gpt-6-luna",
        "glm-5.3-flash",
        "gemini-3.8-flash-high",
    ] {
        let source = json!({"model":model,"max_tokens":64000,"system":[{"type":"text","text":"Leading system"}],
            "thinking":{"type":"adaptive","display":"omitted"},"output_config":{"effort":"high"},
            "messages":[{"role":"user","content":[{"type":"text","text":"Hello"}]},
                {"role":"system","content":[{"type":"text","text":"Dynamic system"}]},
                {"role":"assistant","content":"Hello back"},
                {"role":"developer","content":"Keep this position"}],
            "tools":[{"name":"Read","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}]});
        for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
            let converted = request_body(pair(target), &source).unwrap().body;
            assert_eq!(converted["model"], model);
            if target == ModelProtocol::Responses {
                assert_eq!(converted["input"][1]["role"], "system");
                assert_eq!(
                    converted["input"][1]["content"][0]["text"],
                    "Dynamic system"
                );
                assert_eq!(converted["input"][3]["role"], "developer");
                assert_eq!(converted["reasoning"]["effort"], "high");
            } else {
                assert_eq!(converted["messages"][2]["role"], "system");
                assert_eq!(converted["messages"][2]["content"], "Dynamic system");
                assert_eq!(converted["messages"][4]["role"], "system");
                assert_eq!(converted["reasoning_effort"], "high");
            }
        }
    }
}

#[test]
fn anthropic_document_inputs_map_to_responses_and_chat_file_parts() {
    let source = json!({"model":"m","max_tokens":128,"messages":[{"role":"user","content":[
        {"type":"text","text":"Summarize this"},
        {"type":"document","title":"report.pdf","source":{"type":"base64","media_type":"application/pdf","data":"JVBERi0="}}
    ]}]});
    let responses = request_body(pair(ModelProtocol::Responses), &source)
        .unwrap()
        .body;
    assert_eq!(responses["input"][0]["content"][1]["type"], "input_file");
    assert_eq!(
        responses["input"][0]["content"][1]["filename"],
        "report.pdf"
    );
    assert_eq!(
        responses["input"][0]["content"][1]["file_data"],
        "data:application/pdf;base64,JVBERi0="
    );

    let chat = request_body(pair(ModelProtocol::ChatCompletions), &source)
        .unwrap()
        .body;
    assert_eq!(chat["messages"][0]["content"][1]["type"], "file");
    assert_eq!(
        chat["messages"][0]["content"][1]["file"]["filename"],
        "report.pdf"
    );
    assert_eq!(
        chat["messages"][0]["content"][1]["file"]["file_data"],
        "data:application/pdf;base64,JVBERi0="
    );
}

#[test]
fn system_and_developer_messages_reject_non_text_blocks_and_unknown_roles() {
    for target in [ModelProtocol::ChatCompletions, ModelProtocol::Responses] {
        for role in ["system", "developer"] {
            for block in [
                json!({"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}),
                json!({"type":"tool_use","id":"c1","name":"tool","input":{}}),
                json!({"type":"tool_result","tool_use_id":"c1","content":"result"}),
                json!({"type":"thinking","thinking":"thought"}),
            ] {
                let source = json!({"model":"m","max_tokens":128,"messages":[{"role":role,"content":[block]}]});
                assert_eq!(
                    request_body(pair(target), &source).err().unwrap(),
                    "protocol.errors.unsupportedContent"
                );
            }
        }
        let source =
            json!({"model":"m","max_tokens":128,"messages":[{"role":"unknown","content":"text"}]});
        assert_eq!(
            request_body(pair(target), &source).err().unwrap(),
            "protocol.errors.unsupportedMessageRole"
        );
    }
}

#[test]
fn document_url_is_kept_as_a_responses_file_url() {
    let source = json!({"model":"m","max_tokens":128,"messages":[{"role":"user","content":[
        {"type":"document","source":{"type":"url","url":"https://example.test/report.pdf"}}
    ]}]});
    let responses = request_body(pair(ModelProtocol::Responses), &source)
        .unwrap()
        .body;
    assert_eq!(
        responses["input"][0]["content"][0]["file_url"],
        "https://example.test/report.pdf"
    );
    assert!(request_body(pair(ModelProtocol::ChatCompletions), &source).is_err());
}

#[test]
fn chat_tool_results_stay_contiguous_before_text_and_images() {
    let source = json!({"model":"m","max_tokens":128,"messages":[
        {"role":"assistant","content":[
            {"type":"tool_use","id":"c1","name":"first","input":{}},
            {"type":"tool_use","id":"c2","name":"second","input":{}}
        ]},
        {"role":"user","content":[
            {"type":"text","text":"Tool outputs follow."},
            {"type":"tool_result","tool_use_id":"c1","content":[
                {"type":"text","text":"one"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}}
            ]},
            {"type":"tool_result","tool_use_id":"c2","content":"two"}
        ]}
    ]});

    let chat = request_body(pair(ModelProtocol::ChatCompletions), &source)
        .unwrap()
        .body;
    let messages = chat["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0]["role"], "assistant");
    assert_eq!(messages[1]["role"], "tool");
    assert_eq!(messages[1]["tool_call_id"], "c1");
    assert_eq!(messages[2]["role"], "tool");
    assert_eq!(messages[2]["tool_call_id"], "c2");
    assert_eq!(messages[3]["role"], "user");
    assert_eq!(messages[3]["content"][0]["text"], "Tool outputs follow.");
    assert_eq!(
        messages[3]["content"][1]["image_url"]["url"],
        "data:image/png;base64,YWJj"
    );
}

#[test]
fn nonstream_both_upstreams_return_messages_and_preserve_tools_and_cache_usage() {
    let openai = response(vec![tool()]);
    let converted = response_body(pair(ModelProtocol::Responses), &openai).unwrap();
    assert_eq!(converted["type"], "message");
    assert_eq!(converted["stop_reason"], "tool_use");
    assert_eq!(converted["content"][0]["input"]["path"], "a");
    assert_eq!(converted["usage"]["input_tokens"], 14);
    assert_eq!(converted["usage"]["cache_read_input_tokens"], 6);
    let chat = json!({"model":"m","choices":[{"finish_reason":"tool_calls","message":{"content":null,
        "tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}],
        "usage":{"prompt_tokens":20,"completion_tokens":4,"prompt_tokens_details":{"cached_tokens":6}}});
    let converted = response_body(pair(ModelProtocol::ChatCompletions), &chat).unwrap();
    assert_eq!(converted["content"][0]["id"], "c1");
    assert_eq!(converted["usage"]["input_tokens"], 14);
    assert!(converted.get("object").is_none());
}

#[test]
fn unsupported_content_and_incomplete_tools_are_explicit_errors() {
    let source = json!({"model":"m","max_tokens":12,"messages":[{"role":"user","content":[{"type":"document","source":{}}]}]});
    assert!(request_body(pair(ModelProtocol::Responses), &source).is_err());
    let mut source = json!({"model":"m","max_tokens":12,"messages":[{"role":"user","content":"hi"}],"stop_sequences":["END"]});
    assert_eq!(
        request_body(pair(ModelProtocol::ChatCompletions), &source)
            .unwrap()
            .body["stop"],
        json!(["END"])
    );
    assert!(request_body(pair(ModelProtocol::Responses), &source).is_err());
    source["max_tokens"] = json!(0);
    assert!(request_body(pair(ModelProtocol::ChatCompletions), &source).is_err());
    let mut incomplete = response(vec![tool()]);
    incomplete["status"] = json!("incomplete");
    incomplete["incomplete_details"] = json!({"reason":"max_output_tokens"});
    assert!(response_body(pair(ModelProtocol::Responses), &incomplete).is_err());
}

#[test]
fn chat_sse_streams_text_and_tool_arguments_before_completion() {
    let mut stream = stream_translator(pair(ModelProtocol::ChatCompletions));
    let first = stream.push(
        frame(json!({"model":"m","choices":[{"delta":{"content":"你好"},"finish_reason":null}]}))
            .as_bytes(),
    );
    assert!(first.concat().contains("text_delta"));
    assert!(!first.concat().contains("message_stop"));
    let first_tool = stream.push(frame(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]})).as_bytes());
    assert!(first_tool.concat().contains("input_json_delta"));
    let mut events = stream.push(frame(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a\"}"}}]},"finish_reason":"tool_calls"}]})).as_bytes());
    events.extend(stream.push(
        frame(json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":4}})).as_bytes(),
    ));
    events.extend(stream.push(b"data: [DONE]\n\n"));
    let v = values(&events);
    assert_eq!(v.last().unwrap()["type"], "message_stop");
    assert!(v.iter().any(|v| v["type"] == "message_delta"
        && v["usage"]["input_tokens"] == 10
        && v["delta"]["stop_reason"] == "tool_use"));
    assert!(
        stream.completed_response().is_some(),
        "validated upstream output is retained for Messages reasoning replay"
    );
    assert!(stream.finish().is_empty());
}

#[test]
fn fragmented_tool_names_are_complete_before_a_block_opens() {
    let mut stream = stream_translator(pair(ModelProtocol::ChatCompletions));
    let first = stream.push(frame(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_","arguments":""}}]}}]})).as_bytes());
    assert!(!first.concat().contains("content_block_start"));
    stream.push(
        frame(
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"file"}}]}}]}),
        )
        .as_bytes(),
    );
    let events = stream.push(frame(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]},"finish_reason":"tool_calls"}]})).as_bytes());
    let v = values(&events);
    assert!(v.iter().any(|v| v["content_block"]["name"] == "read_file"));
    assert!(stream
        .push(b"data: [DONE]\n\n")
        .concat()
        .contains("message_stop"));
}

#[test]
fn responses_sse_handles_single_byte_chunks_and_keeps_tool_identity() {
    let mut stream = stream_translator(pair(ModelProtocol::Responses));
    let mut events = Vec::new();
    let mut added = tool();
    added["arguments"] = json!("");
    added["status"] = json!("in_progress");
    let frames = [
        json!({"type":"response.created","response":{"model":"m"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":added}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":\"a\"}"}),
        json!({"type":"response.output_item.done","output_index":0,"item":tool()}),
        json!({"type":"response.completed","response":response(vec![tool()])}),
    ];
    for value in frames {
        for byte in frame(value).as_bytes() {
            events.extend(stream.push(&[*byte]));
        }
    }
    let v = values(&events);
    assert_eq!(v.first().unwrap()["type"], "message_start");
    assert_eq!(v.last().unwrap()["type"], "message_stop");
    assert_eq!(
        v.iter()
            .filter(|v| v["type"] == "content_block_start")
            .count(),
        1
    );
    assert!(v.iter().any(|v| v["content_block"]["id"] == "c1"));
    assert!(v.iter().any(|v| v["usage"]["cache_read_input_tokens"] == 6));
}

#[test]
fn eof_invalid_json_and_upstream_errors_never_complete_partial_tools() {
    for upstream in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        let mut stream = stream_translator(pair(upstream));
        let mut events = stream.push(b"data: {bad json}\n\n");
        events.extend(stream.finish());
        assert_eq!(
            values(&events)
                .iter()
                .filter(|v| v["type"] == "error")
                .count(),
            1
        );
        assert!(!events.concat().contains("message_stop"));
        let mut stream = stream_translator(pair(upstream));
        let events = stream.finish();
        assert!(events.concat().contains("upstream_stream_truncated"));
    }
    let mut stream = stream_translator(pair(ModelProtocol::ChatCompletions));
    stream.push(frame(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"read","arguments":"{"}}]},"finish_reason":"tool_calls"}]})).as_bytes());
    let failed = stream.push(b"data: [DONE]\n\n").concat();
    assert!(failed.contains("event: error"));
    assert!(!failed.contains("content_block_stop"));
    assert!(!failed.contains("message_stop"));
    let mut stream = stream_translator(pair(ModelProtocol::Responses));
    let failed = stream
        .push(
            frame(json!({"type":"response.failed","response":{"error":{"message":"failed"}}}))
                .as_bytes(),
        )
        .concat();
    assert!(failed.contains("event: error"));
}

#[test]
fn length_limited_text_is_not_reported_as_a_normal_turn() {
    let mut value = response(vec![
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}]}),
    ]);
    value["status"] = json!("incomplete");
    value["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let mut stream = stream_translator(pair(ModelProtocol::Responses));
    let events =
        stream.push(frame(json!({"type":"response.incomplete","response":value})).as_bytes());
    assert!(values(&events)
        .iter()
        .any(|v| v["delta"]["stop_reason"] == "max_tokens"));
    assert_eq!(values(&events).last().unwrap()["type"], "message_stop");
}

#[test]
fn streamed_token_limited_text_retains_reasoning_for_tool_continuation() {
    use super::message_history::Context;
    for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        let scope = uuid::Uuid::new_v4().to_string();
        let first = json!({"model":"m","max_tokens":12,"tools":[{"name":"read_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}],"messages":[{"role":"user","content":"hi"}]});
        let mut stream = stream_translator(pair(target));
        let frames = if target == ModelProtocol::Responses {
            let value = json!({"object":"response","model":"m","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[
                {"type":"reasoning","id":"rs-partial","summary":[{"type":"summary_text","text":"partial thinking"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}]}]});
            frame(json!({"type":"response.incomplete","response":value}))
        } else {
            frame(json!({"id":"chat-partial","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"partial","reasoning_content":"partial thinking"},"finish_reason":null}]}))
                + &frame(json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]})) + "data: [DONE]\n\n"
        };
        let events = stream.push(frames.as_bytes());
        assert!(stream.failure().is_none());
        assert_eq!(values(&events).last().unwrap()["type"],"message_stop");
        Context::new(&[&scope],target,first.to_string().as_bytes()).unwrap().remember(stream.completed_response().expect("delivered token-limited text must enter history"));
        let mut next = first;
        next["messages"].as_array_mut().unwrap().extend([json!({"role":"assistant","content":"partial"}),json!({"role":"user","content":"now read a file"})]);
        let mut body = request_body(pair(target),&next).unwrap().body;
        Context::new(&[&scope],target,next.to_string().as_bytes()).unwrap().restore(&mut body);
        assert!(body.to_string().contains("partial thinking"));
    }
}

#[test]
fn messages_usage_reaches_responses_and_chat_callers_with_its_cache_counted_in() {
    fn streamed(from: ModelProtocol, delta: Value) -> Value {
        let mut translator = super::sse::StreamTranslator::new(Translation { from, to: ModelProtocol::AnthropicMessages });
        let events = [
            json!({"type":"message_start","message":{"id":"m1","type":"message","role":"assistant","model":"claude","content":[],
                "usage":{"input_tokens":20,"cache_read_input_tokens":80,"cache_creation_input_tokens":5,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":delta}),
            json!({"type":"message_stop"}),
        ];
        for event in events {
            translator.push(format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap()).as_bytes());
        }
        translator.finish();
        translator.completed_response().expect("a completed response")["usage"].clone()
    }
    let expected = json!({"input_tokens":105,"output_tokens":7,"total_tokens":112,"input_tokens_details":{"cached_tokens":80}});
    for from in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        assert_eq!(streamed(from, json!({"output_tokens":7})), expected, "{from:?}");
        // Counts are cumulative: a relay repeating zeros keeps the start's.
        assert_eq!(streamed(from, json!({"input_tokens":0,"cache_read_input_tokens":0,"output_tokens":7})), expected, "{from:?}");
    }
    let reply = json!({"id":"m1","type":"message","role":"assistant","model":"claude","stop_reason":"end_turn",
        "content":[{"type":"text","text":"hi"}],
        "usage":{"input_tokens":20,"cache_read_input_tokens":80,"cache_creation_input_tokens":5,"output_tokens":7}});
    let converted = response_body(Translation { from: ModelProtocol::Responses, to: ModelProtocol::AnthropicMessages }, &reply).unwrap();
    assert_eq!(converted["usage"]["input_tokens"], 105);
    assert_eq!(converted["usage"]["input_tokens_details"]["cached_tokens"], 80);
}
