//! A relay's reply of any shape is translated or refused, never a panic that
//! drops the client's connection mid-request.
use super::*;
use serde_json::json;
use ModelProtocol::*;

const PROTOCOLS: [ModelProtocol; 3] = [Responses, ChatCompletions, AnthropicMessages];

fn shapes() -> Vec<Value> {
    let mut shapes = vec![
        json!(null), json!(true), json!(1), json!(-1), json!(1.5), json!("s"), json!(""),
        json!([]), json!({}), json!([null]), json!([1]), json!(["s"]), json!([{}]), json!([[]]),
        json!({"type":1}), json!({"type":"text","text":1}), json!({"type":"output_text","text":1,"annotations":1}),
        json!({"type":"message","content":"s"}), json!({"type":"message","content":[1]}),
        json!({"type":"function_call","call_id":1,"name":1,"arguments":1}), json!({"type":"reasoning","summary":[1]}),
        json!({"type":"web_search_call","action":1}), json!({"type":"tool_use","id":1,"input":"s"}),
        json!({"type":"thinking","thinking":1}), json!({"type":"server_tool_use","input":1}),
        json!({"type":"web_search_tool_result","content":1}), json!({"role":1,"content":1}),
        json!({"index":"s","message":1,"delta":1}), json!({"message":{"content":[1],"tool_calls":1}}),
        json!({"message":{"tool_calls":[1]}}), json!({"message":{"tool_calls":[{"function":1}]}}),
        json!({"delta":{"tool_calls":[{"index":"s","function":1}]}}), json!({"input_tokens":"s","output_tokens":-1}),
    ];
    let objects: Vec<Value> = shapes.iter().filter(|shape| shape.is_object()).cloned().collect();
    shapes.extend(objects.into_iter().map(|shape| json!([shape])));
    shapes
}

fn base(protocol: ModelProtocol) -> Value {
    match protocol {
        Responses => json!({"id":"r","object":"response","status":"completed","model":"m",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],
            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}),
        ChatCompletions => json!({"id":"c","object":"chat.completion","model":"m",
            "choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
        AnthropicMessages => json!({"id":"m","type":"message","role":"assistant","model":"m",
            "content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}),
    }
}

/// Each top-level field, item and item part of a reply, replaced by each shape.
fn bodies(protocol: ModelProtocol) -> Vec<Value> {
    const KEYS: [&str; 14] = ["id", "object", "type", "status", "model", "output", "choices", "content", "usage",
        "stop_reason", "finish_reason", "incomplete_details", "error", "role"];
    let (list, part) = match protocol {
        Responses => ("output", Some("content")),
        ChatCompletions => ("choices", Some("message")),
        AnthropicMessages => ("content", None),
    };
    let mut bodies = vec![json!([]), json!("s"), json!(1), json!(null)];
    for shape in shapes() {
        for key in KEYS {
            let mut body = base(protocol);
            body[key] = shape.clone();
            bodies.push(body);
        }
        let mut body = base(protocol);
        body[list][0] = shape.clone();
        bodies.push(body);
        if let Some(part) = part {
            let mut body = base(protocol);
            body[list][0][part] = shape.clone();
            bodies.push(body);
            let mut body = base(protocol);
            body[list][0][part] = json!([shape.clone()]);
            bodies.push(body);
        }
        let mut body = base(protocol);
        body["usage"]["input_tokens_details"] = shape.clone();
        bodies.push(body);
    }
    bodies
}

/// A stream of each upstream protocol, and copies with one field of one event reshaped.
fn streams(protocol: ModelProtocol) -> Vec<Vec<Value>> {
    let events = match protocol {
        Responses => vec![
            json!({"type":"response.created","response":{"id":"r","status":"in_progress","model":"m"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"hi"}),
            json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}),
            json!({"type":"response.completed","response":base(Responses)}),
        ],
        ChatCompletions => vec![
            json!({"id":"c","choices":[{"index":0,"delta":{"role":"assistant","content":"hi"}}]}),
            json!({"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"t","function":{"name":"f","arguments":"{}"}}]}}]}),
            json!({"id":"c","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}),
        ],
        AnthropicMessages => vec![
            json!({"type":"message_start","message":{"id":"m","model":"m","content":[],"usage":{"input_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t","name":"f","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ],
    };
    let mut streams = vec![events.clone()];
    for shape in shapes() {
        for at in 0..events.len() {
            for key in ["index", "output_index", "content_index", "delta", "item", "response", "content_block", "message", "choices", "usage"] {
                if events[at].get(key).is_none() {
                    continue;
                }
                let mut reshaped = events.clone();
                reshaped[at][key] = shape.clone();
                streams.push(reshaped);
            }
        }
    }
    streams
}

fn sse(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .map(|event| match event["type"].as_str() {
            Some(kind) => format!("event: {kind}\ndata: {event}\n\n"),
            None => format!("data: {event}\n\n"),
        })
        .collect::<String>()
        .into_bytes()
}

#[test]
fn malformed_relay_replies_are_refused_without_panicking() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut panics = Vec::new();
    for upstream in PROTOCOLS {
        let replies = bodies(upstream);
        let streams = streams(upstream);
        for client in PROTOCOLS {
            let Some(translation) = Translation::between(client, upstream) else {
                continue;
            };
            for reply in &replies {
                if std::panic::catch_unwind(|| response_body(translation, reply)).is_err() {
                    panics.push(format!("{upstream:?}->{client:?} body: {reply}"));
                }
            }
            for events in &streams {
                let bytes = sse(events);
                let run = std::panic::catch_unwind(|| {
                    let mut translator = stream_translator(translation);
                    for chunk in bytes.chunks(7) {
                        translator.push(chunk);
                    }
                    translator.finish();
                });
                if run.is_err() {
                    panics.push(format!("{upstream:?}->{client:?} stream: {}", Value::from(events.clone())));
                }
            }
        }
    }
    std::panic::set_hook(hook);
    assert!(
        panics.is_empty(),
        "{} panics:\n{}",
        panics.len(),
        panics.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}
