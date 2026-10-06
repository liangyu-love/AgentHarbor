use super::*;
use serde_json::{json, Value};

fn pair(to: ModelProtocol) -> Translation {
    Translation {
        from: ModelProtocol::ChatCompletions,
        to,
    }
}
fn basic() -> Value {
    json!({"model":"fixture","messages":[{"role":"user","content":"hello"}]})
}
fn data(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn event(kind: &str, mut value: Value) -> String {
    value["type"] = json!(kind);
    format!("event: {kind}\ndata: {value}\n\n")
}
fn decoded(frames: &[String]) -> Vec<Value> {
    frames
        .iter()
        .filter_map(|frame| {
            let data = frame.lines().find_map(|line| line.strip_prefix("data: "))?;
            (data != "[DONE]").then(|| serde_json::from_str(data).unwrap())
        })
        .collect()
}
fn response(output: Value, status: &str) -> Value {
    json!({"id":"response_chat_fixture","object":"response","model":"fixture","created_at":1720000000,
        "status":status,"output":output,"usage":{"input_tokens":20,"output_tokens":8,"total_tokens":28,
            "input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":2}}})
}
fn output() -> Value {
    json!([
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"检查"}]},
        {"type":"function_call","call_id":"call_a","name":"lookup","arguments":"{\"q\":\"a\"}"},
        {"type":"function_call","call_id":"call_b","name":"lookup","arguments":"{\"q\":\"b\"}"}
    ])
}

#[test]
fn chat_ingress_maps_history_tools_choices_and_limits_to_both_targets() {
    let value = json!({"model":"fixture","stream":true,"max_completion_tokens":500,"temperature":0.2,"top_p":0.9,
        "parallel_tool_calls":false,"stream_options":{"include_usage":true},
        "messages":[{"role":"developer","content":"be helpful"},{"role":"user","content":"question"},
            {"role":"assistant","content":"checking","tool_calls":[
                {"id":"a","type":"function","function":{"name":"lookup","arguments":"{\"q\":1}"}},
                {"id":"b","type":"function","function":{"name":"lookup","arguments":"{\"q\":2}"}}]},
            {"role":"tool","tool_call_id":"b","content":"second"},{"role":"tool","tool_call_id":"a","content":"first"},
            {"role":"user","content":"finish"}],
        "tools":[{"type":"function","function":{"name":"lookup","description":"look up","strict":true,"parameters":{"type":"object","properties":{"q":{"type":"integer"}}}}}],
        "tool_choice":{"type":"function","function":{"name":"lookup"}}});
    let responses = request_body(pair(ModelProtocol::Responses), &value).unwrap();
    assert_eq!(responses.body["max_output_tokens"], 500);
    assert_eq!(responses.body["input"][0]["role"], "developer");
    assert_eq!(responses.body["input"][3]["type"], "function_call");
    assert_eq!(responses.body["input"][4]["call_id"], "b");
    assert_eq!(responses.body["input"][5]["call_id"], "b");
    assert_eq!(responses.body["tools"][0]["name"], "lookup");
    assert_eq!(responses.body["tools"][0]["strict"], true);
    assert_eq!(
        responses.body["tool_choice"],
        json!({"type":"function","name":"lookup"})
    );
    assert_eq!(responses.body["store"], false);
    assert!(responses.response_options.chat_include_usage);
    let messages = request_body(pair(ModelProtocol::AnthropicMessages), &value).unwrap();
    assert_eq!(messages.body["system"], "be helpful");
    assert_eq!(messages.body["max_tokens"], 500);
    assert_eq!(
        messages.body["messages"][1]["content"][0]["text"],
        "checking"
    );
    assert_eq!(messages.body["messages"][1]["content"][1]["id"], "a");
    assert_eq!(messages.body["messages"][1]["content"][2]["id"], "b");
    assert_eq!(
        messages.body["messages"][2]["content"][0]["tool_use_id"],
        "b"
    );
    assert_eq!(
        messages.body["tool_choice"],
        json!({"type":"tool","name":"lookup","disable_parallel_tool_use":true})
    );
    assert_eq!(messages.body["tools"][0]["input_schema"]["type"], "object");
}

#[test]
fn chat_ingress_maps_images_structured_output_and_target_specific_stop() {
    let mut value = basic();
    value["messages"][0]["content"] = json!([{ "type":"text","text":"describe" },
        {"type":"image_url","image_url":{"url":"https://example.test/image.png","detail":"high"}}]);
    value["response_format"] = json!({"type":"json_schema","json_schema":{"name":"result","strict":true,
        "schema":{"type":"object","properties":{"answer":{"type":"string"}}}}});
    let responses = request_body(pair(ModelProtocol::Responses), &value)
        .unwrap()
        .body;
    assert_eq!(responses["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(responses["input"][0]["content"][1]["detail"], "high");
    assert_eq!(responses["text"]["format"]["type"], "json_schema");
    assert_eq!(responses["text"]["format"]["name"], "result");
    value["stop"] = json!(["END"]);
    let messages = request_body(pair(ModelProtocol::AnthropicMessages), &value)
        .unwrap()
        .body;
    assert_eq!(
        messages["messages"][0]["content"][1]["source"]["url"],
        "https://example.test/image.png"
    );
    assert_eq!(messages["output_config"]["format"]["type"], "json_schema");
    assert_eq!(messages["stop_sequences"], json!(["END"]));
    assert_eq!(
        request_body(pair(ModelProtocol::Responses), &value)
            .err()
            .unwrap(),
        "protocol.errors.unsupportedContent"
    );
}

#[test]
fn chat_ingress_rejects_nonportable_features_instead_of_silently_dropping_them() {
    let mut bad = Vec::new();
    for (key, value) in [
        ("n", json!(2)),
        ("audio", json!({"format":"wav"})),
        ("modalities", json!(["text", "audio"])),
        ("logprobs", json!(true)),
        ("top_logprobs", json!(2)),
        ("logit_bias", json!({"1":2})),
        ("frequency_penalty", json!(0.4)),
        ("presence_penalty", json!(0.4)),
        ("functions", json!([])),
        ("function_call", json!("auto")),
        ("prediction", json!({"type":"content","content":"x"})),
    ] {
        let mut request = basic();
        request[key] = value;
        bad.push(request);
    }
    let mut private = basic();
    private["messages"][0]["signature"] = json!("source-bound");
    bad.push(private);
    let mut private = basic();
    private["messages"][0]["content"] =
        json!([{ "type":"text","text":"x","encrypted_content":"opaque" }]);
    bad.push(private);
    let mut audio = basic();
    audio["messages"][0]["content"] =
        json!([{ "type":"input_audio","input_audio":{"data":"abc","format":"wav"} }]);
    bad.push(audio);
    let mut named = basic();
    named["messages"][0]["name"] = json!("another-user");
    bad.push(named);
    let mut hosted = basic();
    hosted["tools"] = json!([{ "type":"web_search" }]);
    bad.push(hosted);
    let mut custom = basic();
    custom["tools"] = json!([{ "type":"custom","custom":{"name":"x"} }]);
    bad.push(custom);
    let mut forced = basic();
    forced["tool_choice"] = json!("required");
    bad.push(forced);
    let mut unknown_tool = basic();
    unknown_tool["tools"] =
        json!([{ "type":"function","function":{"name":"lookup","parameters":{}} }]);
    unknown_tool["tool_choice"] = json!({"type":"function","function":{"name":"missing"}});
    bad.push(unknown_tool);
    let mut invalid_schema = basic();
    invalid_schema["response_format"] =
        json!({"type":"json_schema","json_schema":{"name":"invalid name","schema":{}}});
    bad.push(invalid_schema);
    let mut invalid_schema = basic();
    invalid_schema["response_format"] =
        json!({"type":"json_schema","json_schema":{"name":"valid","schema":[],"strict":true}});
    bad.push(invalid_schema);
    let mut caps = basic();
    caps["max_tokens"] = json!(10);
    caps["max_completion_tokens"] = json!(20);
    bad.push(caps);
    for target in [ModelProtocol::Responses, ModelProtocol::AnthropicMessages] {
        for request in &bad {
            assert!(
                request_body(pair(target), request).is_err(),
                "{target:?} accepted {request}"
            );
        }
    }
}

#[test]
fn chat_ingress_rejects_orphaned_duplicate_and_invalid_function_history() {
    let call = json!({"id":"a","type":"function","function":{"name":"lookup","arguments":"{}"}});
    let histories = [
        json!([{"role":"assistant","content":null,"tool_calls":[call.clone()]}]),
        json!([{"role":"tool","tool_call_id":"a","content":"result"}]),
        json!([{"role":"assistant","content":null,"tool_calls":[call.clone(),call.clone()]},{"role":"tool","tool_call_id":"a","content":"result"}]),
        json!([{"role":"assistant","content":null,"tool_calls":[call.clone()]},{"role":"user","content":"other"},{"role":"tool","tool_call_id":"a","content":"result"}]),
        json!([{"role":"assistant","content":null,"tool_calls":[{"id":"a","type":"function","function":{"name":"lookup","arguments":"[1]"}}]},{"role":"tool","tool_call_id":"a","content":"result"}]),
        json!([{"role":"assistant","content":null,"tool_calls":[call.clone()]},{"role":"tool","tool_call_id":"a","content":"result"},
            {"role":"assistant","content":null,"tool_calls":[call]},{"role":"tool","tool_call_id":"a","content":"again"}]),
    ];
    for target in [ModelProtocol::Responses, ModelProtocol::AnthropicMessages] {
        for messages in &histories {
            assert!(request_body(
                pair(target),
                &json!({"model":"fixture","messages":messages})
            )
            .is_err());
        }
    }
}

#[test]
fn chat_json_response_maps_parallel_tools_usage_incomplete_and_errors() {
    let complete = response(output(), "completed");
    let chat = response_body(pair(ModelProtocol::Responses), &complete).unwrap();
    assert_eq!(chat["object"], "chat.completion");
    assert_eq!(chat["choices"][0]["message"]["content"], "检查");
    assert_eq!(
        chat["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(chat["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(chat["usage"]["prompt_tokens"], 20);
    assert_eq!(chat["usage"]["prompt_tokens_details"]["cached_tokens"], 4);
    assert_eq!(
        chat["usage"]["completion_tokens_details"]["reasoning_tokens"],
        2
    );
    let mut incomplete = response(
        json!([{ "type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}] }]),
        "incomplete",
    );
    incomplete["incomplete_details"] = json!({"reason":"max_output_tokens"});
    assert_eq!(
        response_body(pair(ModelProtocol::Responses), &incomplete).unwrap()["choices"][0]
            ["finish_reason"],
        "length"
    );
    incomplete["incomplete_details"]["reason"] = json!("content_filter");
    assert_eq!(
        response_body(pair(ModelProtocol::Responses), &incomplete).unwrap()["choices"][0]
            ["finish_reason"],
        "content_filter"
    );
    let mut failed = complete.clone();
    failed["status"] = json!("failed");
    failed["error"] = json!({"code":"upstream_error"});
    assert!(response_body(pair(ModelProtocol::Responses), &failed).is_err());
    let mut hosted = complete;
    hosted["output"] = json!([{ "type":"web_search_call","id":"search_private" }]);
    assert!(response_body(pair(ModelProtocol::Responses), &hosted).is_err());
    let messages = json!({"id":"message_fixture","type":"message","role":"assistant","model":"fixture","content":[
        {"type":"text","text":"检查"},{"type":"tool_use","id":"a","name":"lookup","input":{"q":"a"}}],
        "stop_reason":"tool_use","usage":{"input_tokens":12,"output_tokens":3,"cache_read_input_tokens":4,"cache_creation_input_tokens":2}});
    let chat = response_body(pair(ModelProtocol::AnthropicMessages), &messages).unwrap();
    assert_eq!(
        chat["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"q\":\"a\"}"
    );
    assert_eq!(chat["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(chat["usage"]["prompt_tokens"], 18);
    assert_eq!(chat["usage"]["prompt_tokens_details"]["cached_tokens"], 4);
    assert_eq!(chat["usage"]["total_tokens"], 21);
}

fn responses_stream() -> String {
    let mut source = data(
        json!({"type":"response.created","response":{"id":"response_chat_fixture","model":"fixture","created_at":1720000000}}),
    );
    source += &data(
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}),
    );
    source += &data(
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"检"}),
    );
    source += &data(
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_a","name":"lookup","arguments":""}}),
    );
    source += &data(
        json!({"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","call_id":"call_b","name":"lookup","arguments":""}}),
    );
    source += &data(
        json!({"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"q\":"}),
    );
    source += &data(
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"查"}),
    );
    source += &data(
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"q\":\"a\"}"}),
    );
    source += &data(
        json!({"type":"response.function_call_arguments.delta","output_index":2,"delta":"\"b\"}"}),
    );
    source += &data(json!({"type":"response.completed","response":response(output(),"completed")}));
    source
}

fn assert_chat_stream(frames: &[String], expected_calls: usize, usage: bool) {
    let chunks = decoded(frames);
    assert!(frames
        .last()
        .is_some_and(|frame| frame == "data: [DONE]\n\n"));
    assert!(chunks
        .iter()
        .all(|chunk| chunk["object"] == "chat.completion.chunk"));
    let content = chunks
        .iter()
        .filter_map(|chunk| chunk["choices"][0]["delta"]["content"].as_str())
        .collect::<String>();
    assert_eq!(content, "检查");
    let mut calls = std::collections::BTreeMap::<u64, (String, String, String)>::new();
    for chunk in &chunks {
        for call in chunk["choices"][0]["delta"]["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let entry = calls.entry(call["index"].as_u64().unwrap()).or_default();
            if let Some(id) = call["id"].as_str() {
                entry.0.push_str(id);
            }
            if let Some(name) = call["function"]["name"].as_str() {
                entry.1.push_str(name);
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                entry.2.push_str(arguments);
            }
        }
    }
    assert_eq!(calls.len(), expected_calls);
    for (index, (_, name, arguments)) in calls {
        assert_eq!(name, "lookup");
        assert_eq!(
            serde_json::from_str::<Value>(&arguments).unwrap(),
            json!({"q":if index == 0 {"a"} else {"b"}})
        );
    }
    assert_eq!(
        chunks
            .iter()
            .filter_map(|chunk| chunk["choices"][0]["finish_reason"].as_str())
            .collect::<Vec<_>>(),
        vec!["tool_calls"]
    );
    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| chunk["choices"] == json!([]))
            .count(),
        usize::from(usage)
    );
}

#[test]
fn chat_responses_stream_survives_every_split_and_preserves_parallel_call_deltas() {
    let source = responses_stream();
    for split in 0..=source.len() {
        let mut translator = stream_translator_with_options(
            pair(ModelProtocol::Responses),
            ResponseOptions {
                chat_include_usage: true,
                ..Default::default()
            },
        );
        let mut frames = translator.push(&source.as_bytes()[..split]);
        frames.extend(translator.push(&source.as_bytes()[split..]));
        frames.extend(translator.finish());
        assert_eq!(translator.failure(), None, "split {split}");
        assert!(translator.completed_response().is_some(), "split {split}");
        assert_chat_stream(&frames, 2, true);
    }
}

#[test]
fn chat_messages_stream_uses_common_events_and_opt_in_usage() {
    let mut source = event(
        "message_start",
        json!({"message":{"id":"msg_fixture","model":"fixture","usage":{"input_tokens":20,"output_tokens":0,"cache_read_input_tokens":4,"cache_creation_input_tokens":2}}}),
    );
    source += &event(
        "content_block_start",
        json!({"index":0,"content_block":{"type":"text","text":""}}),
    );
    source += &event(
        "content_block_delta",
        json!({"index":0,"delta":{"type":"text_delta","text":"检查"}}),
    );
    source += &event("content_block_stop", json!({"index":0}));
    for (index, id, q) in [(1, "call_a", "a"), (2, "call_b", "b")] {
        source += &event(
            "content_block_start",
            json!({"index":index,"content_block":{"type":"tool_use","id":id,"name":"lookup","input":{}}}),
        );
        source += &event(
            "content_block_delta",
            json!({"index":index,"delta":{"type":"input_json_delta","partial_json":format!("{{\"q\":\"{q}\"}}")}}),
        );
        source += &event("content_block_stop", json!({"index":index}));
    }
    source += &event(
        "message_delta",
        json!({"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}),
    );
    source += &event("message_stop", json!({}));
    for usage in [false, true] {
        let mut translator = stream_translator_with_options(
            pair(ModelProtocol::AnthropicMessages),
            ResponseOptions {
                chat_include_usage: usage,
                ..Default::default()
            },
        );
        let mut frames = Vec::new();
        for chunk in source.as_bytes().chunks(1) {
            frames.extend(translator.push(chunk));
        }
        frames.extend(translator.finish());
        assert_eq!(translator.failure(), None);
        assert!(translator.completed_response().is_some());
        assert_chat_stream(&frames, 2, usage);
        if usage {
            let chunks = decoded(&frames);
            let usage = &chunks
                .iter()
                .find(|chunk| chunk["choices"] == json!([]))
                .unwrap()["usage"];
            assert_eq!(usage["prompt_tokens"], 26);
            assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 4);
            assert_eq!(usage["total_tokens"], 34);
        }
    }
}

#[test]
fn chat_stream_does_not_report_success_after_incomplete_failure_or_missing_terminal() {
    let partial_output = json!([{ "type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}] }]);
    let mut partial = response(partial_output, "incomplete");
    partial["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    let frames =
        translator.push(data(json!({"type":"response.incomplete","response":partial})).as_bytes());
    assert_eq!(translator.failure(), None);
    assert!(translator.completed_response().is_none());
    assert!(decoded(&frames)
        .iter()
        .any(|chunk| chunk["choices"][0]["finish_reason"] == "length"));
    assert!(translator.finish().is_empty());
    for target in [ModelProtocol::Responses, ModelProtocol::AnthropicMessages] {
        let mut translator = stream_translator(pair(target));
        let source = if target == ModelProtocol::Responses {
            data(json!({"type":"response.created","response":{"model":"fixture"}}))
        } else {
            event(
                "message_start",
                json!({"message":{"model":"fixture","usage":{}}}),
            )
        };
        let mut frames = translator.push(source.as_bytes());
        frames.extend(translator.finish());
        assert_eq!(translator.failure(), Some("upstream_stream_truncated"));
        assert!(translator.completed_response().is_none());
        assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
        assert!(decoded(&frames)
            .iter()
            .any(|chunk| chunk["error"]["code"] == "upstream_stream_truncated"));
    }
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    let frames = translator.push(data(json!({"type":"response.failed","response":{"status":"failed","error":{"code":"failure"}}})).as_bytes());
    assert_eq!(translator.failure(), Some("upstream_error"));
    assert!(translator.completed_response().is_none());
    assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
}

#[test]
fn chat_stream_rejects_invalid_frames_partial_tools_and_hosted_output() {
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    let frames = translator.push(b"data: not json\n\n");
    assert_eq!(translator.failure(), Some("invalid_stream_frame"));
    assert!(decoded(&frames)[0]["error"].is_object());
    let mut partial = response(
        json!([{ "type":"function_call","call_id":"a","name":"lookup","arguments":"{" }]),
        "completed",
    );
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    translator
        .push(data(json!({"type":"response.completed","response":partial.clone()})).as_bytes());
    assert!(translator.failure().is_some());
    assert!(translator.completed_response().is_none());
    partial["output"] = json!([{ "type":"web_search_call","id":"private-search" }]);
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    let frames =
        translator.push(data(json!({"type":"response.completed","response":partial})).as_bytes());
    assert!(translator.failure().is_some());
    assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
    let mut translator = stream_translator(pair(ModelProtocol::AnthropicMessages));
    let source = event(
        "content_block_start",
        json!({"index":0,"content_block":{"type":"server_tool_use","id":"search_1","name":"web_search","input":{"query":"x"}}}),
    );
    let frames = translator.push(source.as_bytes());
    assert_eq!(translator.failure(), Some("unsupported_content"));
    assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
}

#[test]
fn chat_cross_protocol_private_output_is_rejected_in_json_and_fragmented_sse() {
    let private_blocks = [
        json!({"type":"server_tool_use","id":"search_1","name":"web_search","input":{"query":"x"}}),
        json!({"type":"web_search_tool_result","tool_use_id":"search_1","content":[]}),
        json!({"type":"redacted_thinking","data":"opaque"}),
        json!({"type":"thinking","thinking":"private thought","signature":"signed-by-source"}),
        json!({"type":"new_private_content","data":"opaque"}),
    ];
    for block in private_blocks {
        let body = json!({"type":"message","role":"assistant","model":"fixture","content":[block.clone()],"stop_reason":"end_turn","usage":{}});
        assert_eq!(
            response_body(pair(ModelProtocol::AnthropicMessages), &body)
                .err()
                .unwrap(),
            "protocol.errors.unsupportedContent"
        );
        let source = event(
            "content_block_start",
            json!({"index":0,"content_block":block}),
        );
        let mut translator = stream_translator(pair(ModelProtocol::AnthropicMessages));
        let mut frames = Vec::new();
        for chunk in source.as_bytes().chunks(1) {
            frames.extend(translator.push(chunk));
        }
        assert_eq!(translator.failure(), Some("unsupported_content"));
        assert!(translator.completed_response().is_none());
        assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
    }
    let signed_delta = event(
        "content_block_delta",
        json!({"index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
    );
    let mut translator = stream_translator(pair(ModelProtocol::AnthropicMessages));
    translator.push(signed_delta.as_bytes());
    assert_eq!(translator.failure(), Some("unsupported_content"));
    let mut private_response = response(
        json!([{ "type":"reasoning","summary":[],"encrypted_content":"private" }]),
        "completed",
    );
    assert_eq!(
        response_body(pair(ModelProtocol::Responses), &private_response)
            .err()
            .unwrap(),
        "protocol.errors.unsupportedContent"
    );
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    translator
        .push(data(json!({"type":"response.completed","response":private_response})).as_bytes());
    assert_eq!(translator.failure(), Some("unsupported_content"));
    private_response = response(
        json!([{ "type":"message","role":"assistant","content":[{"type":"output_text","text":"x","signature":"signed"}] }]),
        "completed",
    );
    assert_eq!(
        response_body(pair(ModelProtocol::Responses), &private_response)
            .err()
            .unwrap(),
        "protocol.errors.unsupportedContent"
    );
}

#[test]
fn chat_strict_messages_decoder_does_not_change_legacy_or_same_protocol_extension_behavior() {
    let public_tool_argument = json!({"type":"message","role":"assistant","model":"fixture","content":[
        {"type":"tool_use","id":"a","name":"lookup","input":{"signature":"ordinary function parameter"}}],
        "stop_reason":"tool_use","usage":{}});
    assert!(response_body(
        pair(ModelProtocol::AnthropicMessages),
        &public_tool_argument
    )
    .is_ok());
    let source = event(
        "content_block_start",
        json!({"index":0,"content_block":{"type":"new_private_content","data":"opaque"}}),
    ) + &event(
        "message_delta",
        json!({"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":0}}),
    ) + &event("message_stop", json!({}));
    let mut legacy = stream_translator(Translation {
        from: ModelProtocol::Responses,
        to: ModelProtocol::AnthropicMessages,
    });
    let frames = legacy.push(source.as_bytes());
    assert_eq!(legacy.failure(), None);
    assert!(legacy.completed_response().is_some());
    assert!(frames
        .iter()
        .any(|frame| frame.contains("response.completed")));
    use super::adapters::{InboundAdapter, WireAdapter};
    assert!(WireAdapter::new(ModelProtocol::AnthropicMessages)
        .stream_decoder(ModelProtocol::AnthropicMessages, ResponseOptions::default())
        .unwrap()
        .is_none());
}

#[test]
fn chat_stream_terminal_reconciliation_does_not_accept_disappearing_tool_calls() {
    let source = data(
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"a","name":"lookup","arguments":"{}"}}),
    ) + &data(
        json!({"type":"response.completed","response":response(json!([]),"completed")}),
    );
    let mut translator = stream_translator(pair(ModelProtocol::Responses));
    let frames = translator.push(source.as_bytes());
    assert_eq!(translator.failure(), Some("invalid_stream_frame"));
    assert!(translator.completed_response().is_none());
    assert!(!frames.iter().any(|frame| frame.contains("[DONE]")));
}
