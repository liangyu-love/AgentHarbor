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
    assert!(stream.completed_response().is_none());
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
