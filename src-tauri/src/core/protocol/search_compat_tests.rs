//! Native search must survive protocol translation without executing a different
//! provider's search service or turning a server tool into a client function.
use super::*;
use serde_json::{json, Value};

fn pair(from: ModelProtocol, to: ModelProtocol) -> Translation {
    Translation::between(from, to).unwrap()
}

#[test]
fn client_search_functions_keep_schema_and_typed_arguments() {
    // Search Service is a client function, separate from the hosted search tool.
    let schema = json!({"type":"object","properties":{
        "system2_search_query":{"type":["array","null"],"items":{"type":"object","properties":{
            "q":{"type":"string"},"domains":{"type":["array","null"],"items":{"type":"string"}},
            "recency":{"type":["integer","null"]}},"required":["q"]}},
        "input":{"type":"string"}},"additionalProperties":false});
    let request = json!({"model":"m","input":"Search Grok Cursor X",
        "tools":[{"type":"function","name":"_web_run","parameters":schema}]});
    for target in [ModelProtocol::ChatCompletions,ModelProtocol::AnthropicMessages] {
        let converted = request_body(pair(ModelProtocol::Responses,target),&request).unwrap();
        let parameters = if target == ModelProtocol::ChatCompletions {
            &converted.body["tools"][0]["function"]["parameters"]
        } else {&converted.body["tools"][0]["input_schema"]};
        assert_eq!(parameters,&schema);
    }
    for input in [
        json!({"system2_search_query":[{"q":"Grok Cursor X 合并订阅","domains":null,"recency":null}]}),
        json!({}),
        json!({"system2_search_query":"[{\"q\":\"Grok Cursor X\"}]"}),
    ] {
        let response = json!({"id":"msg-search","model":"m","stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"call-search","name":"_web_run","input":input}]});
        let converted = response_body(pair(ModelProtocol::Responses,ModelProtocol::AnthropicMessages),&response).unwrap();
        let decoded: Value = serde_json::from_str(converted["output"][0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(decoded,input);
        // Both a complete initial input and fragmented input_json_delta must
        // preserve the upstream value. Malformed client arguments are not guessed.
        for fragmented in [false,true] {
            let mut events = vec![json!({"type":"message_start","message":{"id":"msg-search","model":"m","usage":{"input_tokens":1}}}),
                json!({"type":"content_block_start","index":0,"content_block":{
                    "type":"tool_use","id":"call-search","name":"_web_run","input":if fragmented {json!({})} else {input.clone()}}})];
            if fragmented {
                let arguments = input.to_string();
                let split = arguments.char_indices().nth(arguments.chars().count()/2).map_or(0,|(index,_)| index);
                for partial in [&arguments[..split],&arguments[split..]] {
                    events.push(json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":partial}}));
                }
            }
            events.extend([json!({"type":"content_block_stop","index":0}),
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}),
                json!({"type":"message_stop"})]);
            let frames = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
            let mut decoder = stream_translator(pair(ModelProtocol::Responses,ModelProtocol::AnthropicMessages));
            for chunk in frames.as_bytes().chunks(7) {decoder.push(chunk);}
            let completed = decoder.completed_response().unwrap();
            let decoded: Value = serde_json::from_str(completed["output"][0]["arguments"].as_str().unwrap()).unwrap();
            assert_eq!(decoded,input);
        }
    }
}

#[test]
fn native_chat_contracts_translate_both_clients_without_changing_model() {
    let codex = json!({"model":"gpt-5-search-api","input":"Rust news","tools":[{"type":"web_search"}],"tool_choice":"auto"});
    let chat = request_body_with_chat_search(pair(ModelProtocol::Responses, ModelProtocol::ChatCompletions), &codex, Some(ChatSearch::OpenAi)).unwrap();
    assert_eq!(chat.body["model"], "gpt-5-search-api");
    assert_eq!(chat.body["web_search_options"], json!({}));
    assert!(chat.body.get("tools").is_none());
    let claude = json!({"model":"mimo-v2.6-pro","max_tokens":100,"messages":[{"role":"user","content":"Rust news"}],
        "tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"tool","name":"web_search"}});
    let chat = request_body_with_chat_search(pair(ModelProtocol::AnthropicMessages, ModelProtocol::ChatCompletions), &claude, Some(ChatSearch::Mimo)).unwrap();
    assert_eq!(chat.body["model"], "mimo-v2.6-pro");
    assert_eq!(chat.body["tools"], json!([{"type":"web_search","force_search":true}]));
    assert_eq!(chat.body["tool_choice"], "auto");
    assert!(chat.body.get("web_search_options").is_none());
}

#[test]
fn native_search_does_not_discard_domain_limits_and_runs_codex_cached_mode_live() {
    let request = json!({"model":"m","input":"x","tools":[{"type":"web_search","filters":{"allowed_domains":["example.org"]}}]});
    for format in [ChatSearch::OpenAi, ChatSearch::Mimo] {
        assert_eq!(request_body_with_chat_search(pair(ModelProtocol::Responses, ModelProtocol::ChatCompletions), &request, Some(format)).err().unwrap(), "protocol.errors.unsupportedSearchOptions");
    }
    // Codex's default cached mode, its indexed mode and image content types:
    // no other provider has OpenAI's index, so they run as live text search.
    for tool in [json!({"type":"web_search","external_web_access":false}),
        json!({"type":"web_search","external_web_access":true,"indexed_web_access":true}),
        json!({"type":"web_search","external_web_access":false,"search_content_types":["text","image"]})] {
        let request = json!({"model":"m","input":"x","tools":[tool]});
        let messages = request_body(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages), &request).unwrap();
        assert_eq!(messages.body["tools"], json!([{"type":"web_search_20250305","name":"web_search"}]));
        assert!(messages.response_options.search_cached_as_live);
        for format in [ChatSearch::OpenAi, ChatSearch::Mimo] {
            let chat = request_body_with_chat_search(pair(ModelProtocol::Responses, ModelProtocol::ChatCompletions), &request, Some(format)).unwrap();
            assert!(chat.body.get("web_search_options").is_some() || chat.body["tools"][0]["type"] == "web_search");
            assert!(!chat.body.to_string().contains("external_web_access"));
        }
    }
    let live = json!({"model":"m","input":"x","tools":[{"type":"web_search","external_web_access":true}]});
    assert!(!request_body(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages), &live).unwrap().response_options.search_cached_as_live);
    assert_eq!(ChatSearch::Auto.resolve("https://unknown.example/v1", "gpt-5-search-api"), None);
    assert_eq!(ChatSearch::Mimo.resolve("https://custom-relay.example/v1", "new-model"), Some(ChatSearch::Mimo));
    assert_eq!(ChatSearch::Disabled.resolve("https://api.xiaomimimo.com/v1", "new-model"), None);
}

#[test]
fn codex_search_maps_to_messages_and_returns_real_search_blocks() {
    let request = json!({"model":"claude-sonnet-4-6","input":"Find Rust news","stream":true,
        "tools":[{"type":"web_search","external_web_access":true,
            "filters":{"allowed_domains":["rust-lang.org"]},
            "user_location":{"type":"approximate","country":"CN"}}],
        "tool_choice":{"type":"web_search"},"max_tool_calls":2});
    let translated = request_body(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages), &request).unwrap();
    assert_eq!(translated.body["tools"][0], json!({"type":"web_search_20250305","name":"web_search",
        "allowed_domains":["rust-lang.org"],"user_location":{"type":"approximate","country":"CN"},"max_uses":2}));
    assert_eq!(translated.body["tool_choice"], json!({"type":"tool","name":"web_search"}));
    let answer = messages_search();
    let converted = response_body(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages), &answer).unwrap();
    assert_eq!(converted["output"][0]["type"], "web_search_call");
    assert_eq!(converted["output"][0]["action"]["query"], "Rust news");
    assert_eq!(converted["output"][0]["action"]["sources"][0]["url"], "https://rust-lang.org");
    assert_eq!(converted["output"][1]["content"][0]["annotations"][0]["url"], "https://rust-lang.org");
    assert_eq!(converted["usage"]["server_tool_use"]["web_search_requests"],1);
    let mut failed = answer.clone();
    failed["content"][1]["content"] = json!({"type":"web_search_tool_result_error","error_code":"max_uses_exceeded"});
    let failed = response_body(pair(ModelProtocol::Responses,ModelProtocol::AnthropicMessages),&failed).unwrap();
    assert_eq!(failed["output"][0]["status"],"failed");
    assert_eq!(failed["output"][0]["error"]["code"],"max_uses_exceeded");
}

fn messages_search() -> Value {
    json!({"id":"msg-search","type":"message","model":"claude-sonnet-4-6","stop_reason":"end_turn",
        "content":[
            {"type":"server_tool_use","id":"search-1","name":"web_search","input":{"query":"Rust news"}},
            {"type":"web_search_tool_result","tool_use_id":"search-1","content":[
                {"type":"web_search_result","url":"https://rust-lang.org","title":"Rust","encrypted_content":"opaque"}]},
            {"type":"text","text":"Rust news","citations":[{"type":"web_search_result_location","url":"https://rust-lang.org","title":"Rust","cited_text":"Rust news"}]}],
        "usage":{"input_tokens":2,"output_tokens":3,"server_tool_use":{"web_search_requests":1}}})
}

#[test]
fn review_disabled_claude_search_has_no_native_result() {
    let request = json!({"model":"m","max_tokens":100,"messages":[{"role":"user","content":"hi"}],
        "tools":[{"type":"web_search_20250305","name":"web_search"}],"tool_choice":{"type":"none"}});
    let prepared = request_body_with_chat_search(pair(ModelProtocol::AnthropicMessages,ModelProtocol::ChatCompletions),&request,Some(ChatSearch::Mimo)).unwrap();
    assert!(prepared.response_options.search.is_none());
}

#[test]
fn review_same_source_keeps_distinct_citation_ranges_and_history_queries() {
    let mut text = "First Second".to_owned();
    let citations = search_compat::citations(&mut text,&json!([
        {"type":"url_citation","url":"https://example.org","start_index":0,"end_index":5},
        {"type":"url_citation","url":"https://example.org","start_index":6,"end_index":12},
        {"type":"url_citation","url":"https://example.org","start_index":0,"end_index":5}]));
    assert_eq!(citations.len(),2);
    let history = search_compat::readable_history(&json!({"id":"ws-history","type":"web_search_call","status":"completed",
        "action":{"type":"search","queries":["first query","second query"],"sources":[]}})).unwrap();
    assert!(history.to_string().contains("first query") && history.to_string().contains("second query"));
}

#[test]
fn review_messages_search_preserves_block_order_and_citation_ownership() {
    let mut source = messages_search();
    source["content"].as_array_mut().unwrap().insert(0,json!({"type":"text","text":"Before search",
        "citations":[{"type":"web_search_result_location","url":"https://example.org/earlier","title":"Earlier"}]}));
    let mut frames = format!("event: message_start\ndata: {}\n\n",json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":2}}}));
    for (index,block) in source["content"].as_array().unwrap().iter().enumerate() {
        frames.push_str(&format!("event: content_block_start\ndata: {}\n\nevent: content_block_stop\ndata: {}\n\n",
            json!({"type":"content_block_start","index":index,"content_block":block}),json!({"type":"content_block_stop","index":index})));
    }
    frames.push_str("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
    let mut decoder = stream_translator(pair(ModelProtocol::Responses,ModelProtocol::AnthropicMessages));
    decoder.push(frames.as_bytes());
    let response = decoder.completed_response().unwrap();
    let output = response["output"].as_array().unwrap();
    assert_eq!(output.iter().map(|item| item["type"].as_str().unwrap()).collect::<Vec<_>>(),vec!["message","web_search_call","message"]);
    assert_eq!(output[0]["content"][0]["annotations"][0]["url"],"https://example.org/earlier");
    assert_eq!(output[2]["content"][0]["annotations"][0]["url"],"https://rust-lang.org");
}

#[test]
fn chat_search_keeps_native_citations_and_search_usage() {
    let reply = json!({"id":"chat-search","model":"mimo-v2.6-pro","choices":[{"finish_reason":"stop",
        "message":{"role":"assistant","content":"Rust news","annotations":[
            {"type":"url_citation","url":"https://rust-lang.org","title":"Rust","summary":"Release news"},
            {"type":"url_citation","url":"javascript:alert(1)","title":"unsafe"}]}}],
        "usage":{"prompt_tokens":2,"completion_tokens":3,"web_search_usage":{"tool_usage":1,"page_usage":1}}});
    let converted = response_body(pair(ModelProtocol::Responses, ModelProtocol::ChatCompletions), &reply).unwrap();
    let annotation = &converted["output"][0]["content"][0]["annotations"][0];
    assert_eq!(annotation["url"], "https://rust-lang.org");
    assert!(annotation["start_index"].is_u64() && annotation["end_index"].is_u64());
    assert_eq!(converted["output"][0]["content"][0]["annotations"].as_array().unwrap().len(), 1);
    assert_eq!(converted["usage"]["web_search_usage"]["tool_usage"], 1);
    // Citation evidence alone does not reveal a search query or a call id.
    assert!(converted["output"].as_array().unwrap().iter().all(|item| item["type"] != "web_search_call"));
}

#[test]
fn messages_search_stream_keeps_call_results_and_citations() {
    let blocks = messages_search()["content"].as_array().unwrap().clone();
    let mut frames = String::new();
    let mut frame = |kind: &str, value: Value| {
        frames.push_str(&format!("event: {kind}\ndata: {value}\n\n"));
    };
    frame("message_start", json!({"type":"message_start","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":2}}}));
    for (index, block) in blocks.iter().enumerate() {
        frame("content_block_start", json!({"type":"content_block_start","index":index,"content_block":block}));
        frame("content_block_stop", json!({"type":"content_block_stop","index":index}));
    }
    frame("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}));
    frame("message_stop", json!({"type":"message_stop"}));
    for split in 0..=frames.len() {
        let mut decoder = stream_translator(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages));
        let mut events = decoder.push(&frames.as_bytes()[..split]);
        events.extend(decoder.push(&frames.as_bytes()[split..]));
        assert!(decoder.failure().is_none(), "split {split}: {events:?}");
        let completed = decoder.completed_response().unwrap();
        assert_eq!(completed["output"][0]["type"], "web_search_call", "split {split}");
        assert_eq!(completed["output"][0]["status"], "completed");
        assert_eq!(completed["output"][1]["content"][0]["annotations"][0]["url"], "https://rust-lang.org");
    }
}

#[test]
fn native_chat_result_requires_search_evidence_and_success_in_both_encodings() {
    let options = ResponseOptions { search:Some(web_search::Search {name:"web_search".into(),max_uses:None}), ..Default::default() };
    let upstream = json!({"id":"chat-native","model":"new-model","choices":[{"finish_reason":"stop",
        "message":{"role":"assistant","content":"News","annotations":[{"type":"url_citation","url":"https://example.org","title":"Source"}]}}]});
    for caller in [ModelProtocol::Responses,ModelProtocol::AnthropicMessages] {
        let pair = pair(caller,ModelProtocol::ChatCompletions);
        let result = response_body_with_options(pair,&upstream,&options).unwrap();
        if caller == ModelProtocol::Responses {
            assert_eq!(result["output"][0]["type"],"web_search_call");
            assert!(result["output"][0]["action"].get("query").is_none());
        } else { assert_eq!(result["content"][0]["type"],"server_tool_use"); }
        let frames = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":upstream["choices"][0]["message"],"finish_reason":null}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}));
        for split in 0..=frames.len() {
            let mut decoder = stream_translator_with_options(pair,options.clone());
            let mut events = decoder.push(&frames.as_bytes()[..split]);
            events.extend(decoder.push(&frames.as_bytes()[split..]));
            assert!(decoder.failure().is_none(),"{caller:?}/{split}: {events:?}");
            let completed = decoder.completed_response().unwrap();
            assert!(completed.to_string().contains("web_search_call"),"{completed}");
            assert!(events.join("").contains("https://example.org"));
            assert!(decoder.finish().is_empty());
        }
    }
    let mut incomplete = upstream.clone(); incomplete["choices"][0]["finish_reason"] = json!("length");
    let incomplete = response_body_with_options(pair(ModelProtocol::Responses,ModelProtocol::ChatCompletions), &incomplete,&options).unwrap();
    assert!(incomplete["output"].as_array().unwrap().iter().all(|item| item["type"] != "web_search_call"));
    let mut failed = upstream.clone(); failed["choices"][0]["message"]["error_message"] = json!("search failed");
    let failed = response_body_with_options(pair(ModelProtocol::Responses,ModelProtocol::ChatCompletions),&failed,&options).unwrap();
    assert_eq!(failed["status"],"failed");
    assert!(failed["output"].as_array().unwrap().iter().all(|item| item["type"] != "web_search_call"));
    let mut stream = stream_translator_with_options(pair(ModelProtocol::Responses,ModelProtocol::ChatCompletions),options);
    let failure = stream.push(b"data: {\"choices\":[{\"delta\":{\"error_message\":\"search failed\"}}]}\n\n");
    assert!(failure.join("").contains("response.failed"));
    assert!(stream.completed_response().is_none());
}

#[test]
fn claude_cited_answer_is_one_codex_message_with_sources_listed_once() {
    let cite = |url: &str, title: &str| json!({"type":"web_search_result_location","url":url,"title":title,"cited_text":"x","encrypted_index":"e"});
    let answer = json!({"id":"msg-cited","type":"message","model":"claude-sonnet-4-6","stop_reason":"end_turn",
        "content":[
            {"type":"text","text":"Let me search."},
            {"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"Rust"}},
            {"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[
                {"type":"web_search_result","url":"https://rust-lang.org","title":"Rust","encrypted_content":"opaque"}]},
            {"type":"text","text":"Rust 1.99 shipped","citations":[cite("https://rust-lang.org","Rust")]},
            {"type":"text","text":" with new features"},
            {"type":"text","text":" and fixes.","citations":[cite("https://rust-lang.org","Rust"),cite("https://blog.rust-lang.org/x","Blog [x]")]}],
        "usage":{"input_tokens":2,"output_tokens":3}});
    let expected = "Rust 1.99 shipped with new features and fixes.\n\nSources:\n- [Rust](https://rust-lang.org)\n- [Blog  x](https://blog.rust-lang.org/x)";
    let check = |output: &[Value]| {
        assert_eq!(output.iter().map(|item| item["type"].as_str().unwrap()).collect::<Vec<_>>(), vec!["message","web_search_call","message"]);
        assert_eq!(output[0]["content"][0]["text"], "Let me search.");
        let part = &output[2]["content"][0];
        assert_eq!(part["text"], expected);
        let annotations = part["annotations"].as_array().unwrap();
        assert_eq!(annotations.len(), 2);
        let text: Vec<char> = expected.chars().collect();
        for annotation in annotations {
            let (start, end) = (annotation["start_index"].as_u64().unwrap() as usize, annotation["end_index"].as_u64().unwrap() as usize);
            let linked: String = text[start..end].iter().collect();
            assert!(linked.ends_with(&format!("({})", annotation["url"].as_str().unwrap())), "{linked}");
        }
    };
    let converted = response_body(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages), &answer).unwrap();
    check(converted["output"].as_array().unwrap());
    let mut frames = format!("event: message_start\ndata: {}\n\n", json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":2}}}));
    for (index, block) in answer["content"].as_array().unwrap().iter().enumerate() {
        let mut start = block.clone();
        let citations = start.as_object_mut().unwrap().remove("citations");
        frames.push_str(&format!("event: content_block_start\ndata: {}\n\n", json!({"type":"content_block_start","index":index,"content_block":start})));
        for citation in citations.iter().flat_map(|c| c.as_array().unwrap().clone()) {
            frames.push_str(&format!("event: content_block_delta\ndata: {}\n\n",
                json!({"type":"content_block_delta","index":index,"delta":{"type":"citations_delta","citation":citation}})));
        }
        frames.push_str(&format!("event: content_block_stop\ndata: {}\n\n", json!({"type":"content_block_stop","index":index})));
    }
    frames.push_str("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
    let mut decoder = stream_translator(pair(ModelProtocol::Responses, ModelProtocol::AnthropicMessages));
    let events = decoder.push(frames.as_bytes()).join("");
    assert!(decoder.failure().is_none(), "{events}");
    check(decoder.completed_response().unwrap()["output"].as_array().unwrap());
    // Streamed deltas spell the same text the completed item reports.
    let streamed: String = events.split("\n\n").filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .filter(|event| event["type"] == "response.output_text.delta" && event["output_index"] == 2)
        .map(|event| event["delta"].as_str().unwrap().to_owned()).collect();
    assert_eq!(streamed, expected);
}
