//! Repeatable P7 fixtures. The reference path reconstructs the former string
//! bridge using the current Chat decoder and Messages encoder; it is not a
//! separate old executable. Only generated IDs are normalized for comparison.
use super::{anthropic_sse, message_history, request_body, sse, ModelProtocol, Translation};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

struct ByteCounter(usize);
impl std::io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_bytes(value: &Value) -> usize {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value).unwrap();
    counter.0
}

// This module is compiled only by the library's cfg(test) module declaration.
// Counting is disabled by default and is scoped to the current benchmark
// thread. Other tests and production binaries keep System allocation behavior.
struct TestAllocator;

#[derive(Clone, Copy, Debug, Default)]
struct AllocationCounts {
    allocation_calls: u64,
    reallocation_calls: u64,
    requested_bytes: u64,
}

thread_local! {
    static ALLOCATION_COUNTS: Cell<Option<AllocationCounts>> = const { Cell::new(None) };
}

fn account(size: usize, reallocation: bool) {
    // try_with also handles allocator calls while TLS is being destroyed.
    let _ = ALLOCATION_COUNTS.try_with(|counter| {
        if let Some(mut counts) = counter.get() {
            if reallocation {
                counts.reallocation_calls += 1;
            } else {
                counts.allocation_calls += 1;
            }
            counts.requested_bytes = counts.requested_bytes.saturating_add(size as u64);
            counter.set(Some(counts));
        }
    });
}

unsafe impl GlobalAlloc for TestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's valid layout is forwarded unchanged to System.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            account(layout.size(), false);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's valid layout is forwarded unchanged to System.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            account(layout.size(), false);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the original allocation and new size are forwarded to System.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            account(size, true);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: this allocator exclusively delegates storage to System.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static TEST_ALLOCATOR: TestAllocator = TestAllocator;

struct AllocationScope;
impl AllocationScope {
    fn start() -> Self {
        ALLOCATION_COUNTS.with(|counter| {
            assert!(counter.get().is_none(), "nested allocation measurement");
            counter.set(Some(AllocationCounts::default()));
        });
        Self
    }
    fn finish(self) -> AllocationCounts {
        ALLOCATION_COUNTS.with(|counter| counter.take().unwrap())
    }
}
impl Drop for AllocationScope {
    fn drop(&mut self) {
        let _ = ALLOCATION_COUNTS.try_with(|counter| counter.set(None));
    }
}

fn measured<T>(operation: impl FnOnce() -> T) -> (T, Duration, AllocationCounts) {
    let counter = AllocationScope::start();
    let started = Instant::now();
    let result = operation();
    let elapsed = started.elapsed();
    (result, elapsed, counter.finish())
}

fn chat_frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"chat_upstream_fixture","object":"chat.completion.chunk",
            "created":1720000000,"model":"fixture","choices":[{"index":0,
                "delta":delta,"finish_reason":finish}]})
    )
}

fn tool_fixture() -> Vec<String> {
    vec![
        chat_frame(json!({"content":"开始🙂"}), None),
        chat_frame(json!({"reasoning_content":"检查工具"}), None),
        chat_frame(
            json!({"tool_calls":[{"index":0,"id":"call_alpha",
            "function":{"name":"look_","arguments":""}}]}),
            None,
        ),
        chat_frame(
            json!({"tool_calls":[{"index":1,"id":"call_beta",
            "function":{"name":"read_file","arguments":"{\"path\":"}}]}),
            None,
        ),
        chat_frame(json!({"content":"处理中"}), None),
        chat_frame(
            json!({"tool_calls":[{"index":0,
            "function":{"name":"up","arguments":"{\"q\":"}}]}),
            None,
        ),
        chat_frame(
            json!({"tool_calls":[{"index":1,
            "function":{"arguments":"\"文件🙂\"}"}}]}),
            None,
        ),
        chat_frame(
            json!({"tool_calls":[{"index":0,
            "function":{"arguments":"\"研究\"}"}}]}),
            Some("tool_calls"),
        ),
        format!(
            "data: {}\n\n",
            json!({"choices":[],"usage":{
            "prompt_tokens":120,"completion_tokens":19,"total_tokens":139,
            "prompt_tokens_details":{"cached_tokens":32},
            "completion_tokens_details":{"reasoning_tokens":5}}})
        ),
        "data: [DONE]\n\n".into(),
    ]
}

fn limited_fixture() -> Vec<String> {
    vec![
        chat_frame(json!({"content":"部分回答🙂"}), None),
        chat_frame(json!({}), Some("length")),
        "data: [DONE]\n\n".into(),
    ]
}

enum Pipeline {
    Events(anthropic_sse::StreamTranslator),
    FormerStringBridge {
        decoder: sse::StreamTranslator,
        encoder: anthropic_sse::StreamTranslator,
        bridge_json_passes: usize,
        bridge_bytes: usize,
    },
}
impl Pipeline {
    fn new(staging: bool) -> Self {
        if staging {
            Self::FormerStringBridge {
                decoder: sse::StreamTranslator::new(Translation {
                    from: ModelProtocol::Responses,
                    to: ModelProtocol::ChatCompletions,
                })
                .with_tool_metadata(),
                encoder: anthropic_sse::StreamTranslator::new(ModelProtocol::Responses),
                bridge_json_passes: 0,
                bridge_bytes: 0,
            }
        } else {
            Self::Events(anthropic_sse::StreamTranslator::new(
                ModelProtocol::ChatCompletions,
            ))
        }
    }
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        match self {
            Self::Events(encoder) => encoder.push(chunk),
            Self::FormerStringBridge {
                decoder,
                encoder,
                bridge_json_passes,
                bridge_bytes,
            } => {
                let events = decoder.push(chunk);
                // Each emitted event is serialized by push() and reparsed once
                // by encoder.push(), exactly as in the former bytes bridge.
                *bridge_json_passes += events.len();
                let staged = events.concat();
                *bridge_bytes += staged.len();
                encoder.push(staged.as_bytes())
            }
        }
    }
    fn finish(&mut self) -> Vec<String> {
        match self {
            Self::Events(encoder) => encoder.finish(),
            Self::FormerStringBridge {
                decoder,
                encoder,
                bridge_json_passes,
                bridge_bytes,
            } => {
                let events = decoder.finish();
                *bridge_json_passes += events.len();
                let staged = events.concat();
                *bridge_bytes += staged.len();
                let mut output = encoder.push(staged.as_bytes());
                output.extend(encoder.finish());
                output
            }
        }
    }
    fn encoder(&self) -> &anthropic_sse::StreamTranslator {
        match self {
            Self::Events(encoder) | Self::FormerStringBridge { encoder, .. } => encoder,
        }
    }
    fn bridge_work(&self) -> (usize, usize) {
        match self {
            Self::Events(_) => (0, 0),
            Self::FormerStringBridge {
                bridge_json_passes,
                bridge_bytes,
                ..
            } => (*bridge_json_passes, *bridge_bytes),
        }
    }
}

#[derive(Default)]
struct StreamRun {
    output: Vec<String>,
    history: Option<Value>,
    failure: Option<&'static str>,
    first_text: Option<Duration>,
    first_text_input_bytes: Option<usize>,
    output_bytes: usize,
    output_events: usize,
    bridge_json_passes: usize,
    bridge_bytes: usize,
}

fn run_stream<'a>(
    staging: bool,
    chunks: impl IntoIterator<Item = &'a [u8]>,
    collect: bool,
) -> StreamRun {
    let started = Instant::now();
    let mut pipeline = Pipeline::new(staging);
    let mut run = StreamRun::default();
    let mut input_bytes = 0;
    for chunk in chunks {
        input_bytes += chunk.len();
        let events = pipeline.push(chunk);
        if run.first_text.is_none() && events.iter().any(|event| event.contains("\"text_delta\"")) {
            run.first_text = Some(started.elapsed());
            run.first_text_input_bytes = Some(input_bytes);
        }
        run.output_bytes += events.iter().map(String::len).sum::<usize>();
        run.output_events += events.len();
        if collect {
            run.output.extend(events);
        } else {
            black_box(events);
        }
    }
    let last = pipeline.finish();
    run.output_bytes += last.iter().map(String::len).sum::<usize>();
    run.output_events += last.len();
    if collect {
        run.output.extend(last);
        run.history = pipeline.encoder().completed_response().cloned();
    }
    run.failure = pipeline.encoder().failure();
    (run.bridge_json_passes, run.bridge_bytes) = pipeline.bridge_work();
    run
}

fn normalize_ids(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "id" | "item_id" | "response_id") {
                    if let Value::String(id) = value {
                        if let Some(position) = id.rfind("resp_") {
                            let random = &id[position + 5..];
                            if random.len() == 32
                                && random.bytes().all(|byte| byte.is_ascii_hexdigit())
                            {
                                id.replace_range(position + 5.., "GENERATED_RESPONSE");
                            }
                        } else if id.strip_prefix("msg_").is_some_and(|random| {
                            random.len() == 32
                                && random.bytes().all(|byte| byte.is_ascii_hexdigit())
                        }) {
                            *id = "msg_GENERATED_MESSAGE".into();
                        }
                    }
                }
                normalize_ids(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_ids),
        _ => {}
    }
}

fn values(run: &StreamRun) -> Vec<Value> {
    run.output
        .iter()
        .map(|frame| {
            let data = frame
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap();
            let mut value: Value = serde_json::from_str(data).unwrap();
            normalize_ids(&mut value);
            value
        })
        .collect()
}

fn normalized_history(run: &StreamRun) -> Option<Value> {
    run.history.clone().map(|mut value| {
        normalize_ids(&mut value);
        value
    })
}

#[test]
fn p7_event_bridge_matches_string_bridge_under_every_byte_split() {
    for frames in [tool_fixture(), limited_fixture()] {
        let wire = frames.concat();
        let baseline = run_stream(true, [wire.as_bytes()], true);
        assert!(baseline.failure.is_none());
        let expected = values(&baseline);
        let expected_history = normalized_history(&baseline);
        for split in 0..=wire.len() {
            let chunks = [&wire.as_bytes()[..split], &wire.as_bytes()[split..]];
            let direct = run_stream(false, chunks, true);
            assert_eq!(direct.failure, None, "split {split}");
            assert_eq!(values(&direct), expected, "events at split {split}");
            assert_eq!(
                normalized_history(&direct),
                expected_history,
                "history at split {split}"
            );
            assert_eq!(direct.bridge_json_passes, 0);
        }
    }
}

#[test]
fn p7_unicode_tools_keep_identity_and_first_text_under_single_byte_frames() {
    let frames = tool_fixture();
    let wire = frames.concat();
    let direct = run_stream(false, wire.as_bytes().chunks(1), true);
    let former = run_stream(true, wire.as_bytes().chunks(1), true);
    assert_eq!(values(&direct), values(&former));
    assert_eq!(normalized_history(&direct), normalized_history(&former));
    assert_eq!(direct.first_text_input_bytes, Some(frames[0].len()));
    assert_eq!(former.first_text_input_bytes, Some(frames[0].len()));
    assert!(former.bridge_json_passes > 0);
    assert!(former.bridge_bytes > 0);
    let history = direct.history.unwrap();
    let tools: Vec<_> = history["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["call_id"], "call_alpha");
    assert_eq!(tools[0]["name"], "look_up");
    assert_eq!(tools[0]["arguments"], "{\"q\":\"研究\"}");
    assert_eq!(tools[1]["call_id"], "call_beta");
    assert_eq!(tools[1]["arguments"], "{\"path\":\"文件🙂\"}");
    assert_eq!(
        history["usage"]["input_tokens_details"]["cached_tokens"],
        32
    );
    assert_eq!(history["usage"]["output_tokens"], 19);
}

#[test]
fn p7_pending_frame_limit_stops_unterminated_input_without_success_history() {
    const LIMIT: usize = 8 * 1024 * 1024;
    // Chunks cap overshoot to one byte; no delimiter means no valid JSON frame.
    let block = vec![b'x'; 64 * 1024];
    for staging in [false, true] {
        let mut pipeline = Pipeline::new(staging);
        for _ in 0..LIMIT / block.len() {
            assert!(pipeline.push(&block).is_empty());
            assert_eq!(pipeline.encoder().failure(), None);
        }
        let error = pipeline.push(b"x");
        assert!(pipeline.encoder().failure().is_some());
        assert!(pipeline.encoder().completed_response().is_none());
        assert!(!error.concat().contains("message_stop"));
        assert!(pipeline.push(b"\n\n").is_empty());
    }
}

#[test]
fn p7_many_small_frames_share_one_retained_output_budget() {
    let text = "x".repeat(512 * 1024);
    for caller in [
        ModelProtocol::AnthropicMessages,
        ModelProtocol::ChatCompletions,
    ] {
        let mut stream = super::stream_translator(Translation {
            from: caller,
            to: ModelProtocol::Responses,
        });
        let created = format!(
            "data: {}\n\n",
            json!({"type":"response.created",
            "response":{"model":"fixture","status":"in_progress","output":[]}})
        );
        stream.push(created.as_bytes());
        let mut accepted_text = 0;
        let mut final_events = Vec::new();
        // Each individual frame and block is far below the frame/block limit.
        // Only the shared budget may stop this sequence of distinct blocks.
        for index in 0..=super::canonical::MAX_RETAINED_BYTES / text.len() {
            let wire = format!(
                "data: {}\n\n",
                json!({"type":"response.output_text.delta",
                "output_index":index,"content_index":0,"delta":text})
            );
            let events = stream.push(wire.as_bytes());
            if stream.failure().is_some() {
                final_events = events;
                break;
            }
            accepted_text += text.len();
        }
        assert_eq!(
            stream.failure(),
            Some("stream_output_limit"),
            "caller {caller:?}"
        );
        assert!(accepted_text <= super::canonical::MAX_RETAINED_BYTES);
        assert!(stream.completed_response().is_none());
        assert!(final_events
            .iter()
            .any(|event| event.contains("stream_output_limit")));
        assert!(!final_events.concat().contains("message_stop"));
        assert!(!final_events.concat().contains("data: [DONE]"));
        assert!(stream.finish().is_empty());
    }
}

#[test]
fn p7_allocation_counter_is_local_and_disabled_outside_measurement() {
    let ((), _, counts) = measured(|| {
        let allocation = black_box(vec![0u8; 1234]);
        // A child thread is intentionally outside this thread's measurement.
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(ALLOCATION_COUNTS.with(Cell::get).is_none());
                black_box(vec![0u8; 256]);
            });
        });
        black_box(allocation);
    });
    assert!(counts.allocation_calls > 0);
    assert!(counts.requested_bytes >= 1234);
    assert!(ALLOCATION_COUNTS.with(Cell::get).is_none());
}

fn large_history(rounds: usize, characters: usize) -> Value {
    let text = "历史内容🙂 ".repeat(characters / 6);
    let mut messages = Vec::new();
    for round in 0..rounds {
        messages.push(json!({"role":"user","content":format!("question {round}: {text}")}));
        if round % 16 == 0 {
            let id = format!("history_call_{round}");
            messages.push(json!({"role":"assistant","content":[
                {"type":"text","text":text},
                {"type":"tool_use","id":id,"name":"lookup","input":{"q":round}}]}));
            messages.push(json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":id,"content":"found"}]}));
        }
        messages.push(json!({"role":"assistant","content":text}));
    }
    messages.push(json!({"role":"user","content":"finish"}));
    json!({"model":"fixture","system":"Reply using the conversation.","max_tokens":128,
        "messages":messages,"tools":[{"name":"lookup","input_schema":{
            "type":"object","properties":{"q":{"type":"integer"}}}}]})
}

#[test]
fn p7_large_history_conversion_preserves_turns_tools_and_limits() {
    let value = large_history(128, 1024);
    let bytes = value.to_string().into_bytes();
    let context = message_history::Context::new(&["p7-fixture"], ModelProtocol::Responses, &bytes);
    assert!(context.is_some());
    for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        let request = request_body(
            Translation {
                from: ModelProtocol::AnthropicMessages,
                to: target,
            },
            &value,
        )
        .unwrap();
        assert_eq!(request.body["model"], "fixture");
        assert_eq!(
            request.body[if target == ModelProtocol::Responses {
                "max_output_tokens"
            } else {
                "max_tokens"
            }],
            128
        );
        // Messages callers carry their full history in body rather than in
        // TranslatedRequest.input (which belongs to Responses continuation).
        let (calls, results) = if target == ModelProtocol::Responses {
            let input = request.body["input"].as_array().unwrap();
            (
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call")
                    .count(),
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call_output")
                    .count(),
            )
        } else {
            let messages = request.body["messages"].as_array().unwrap();
            (
                messages
                    .iter()
                    .map(|message| message["tool_calls"].as_array().map_or(0, Vec::len))
                    .sum(),
                messages
                    .iter()
                    .filter(|message| message["role"] == "tool")
                    .count(),
            )
        };
        assert_eq!(calls, 8);
        assert_eq!(results, calls);
        assert!(request.body.to_string().len() >= bytes.len() / 2);
    }
}

fn report(
    label: &str,
    run: &StreamRun,
    elapsed: Duration,
    counts: AllocationCounts,
    bytes: usize,
    json_frames: usize,
) {
    println!(
        "{}",
        json!({"case":label,"fixtureBytes":bytes,"upstreamJsonFrames":json_frames,
        "elapsedUs":elapsed.as_micros(),"firstTextUs":run.first_text.map(|t|t.as_micros()),
        "firstTextAfterInputBytes":run.first_text_input_bytes,"outputBytes":run.output_bytes,
        "outputEvents":run.output_events,"intermediateJsonSerializations":run.bridge_json_passes,
        "intermediateJsonParses":run.bridge_json_passes,"intermediateSseBytes":run.bridge_bytes,
        "allocationCalls":counts.allocation_calls,"reallocationCalls":counts.reallocation_calls,
        "allocationRequestedBytes":counts.requested_bytes})
    );
}

#[test]
#[ignore = "repeatable P7 measurements; run with --ignored --nocapture --test-threads=1"]
fn p7_measure_stream_and_large_history_baselines() {
    let small = tool_fixture();
    let mut large = vec![chat_frame(json!({"content":"开始🙂"}), None)];
    for _ in 0..128 {
        large.push(chat_frame(json!({"content":"a".repeat(1024)}), None));
    }
    large.extend(small.into_iter().skip(1));
    for (name, frames, width) in [
        ("tools-single-byte", tool_fixture(), 1usize),
        ("text-and-tools-1KiB", large, 1024usize),
    ] {
        let wire = frames.concat();
        let json_frames = frames
            .iter()
            .filter(|frame| !frame.contains("data: [DONE]"))
            .count();
        for staging in [false, true] {
            black_box(run_stream(staging, wire.as_bytes().chunks(width), false));
            let mut samples = Vec::new();
            for _ in 0..9 {
                samples.push(measured(|| {
                    run_stream(staging, wire.as_bytes().chunks(width), false)
                }));
            }
            samples.sort_by_key(|sample| sample.1);
            let (run, elapsed, counts) = &samples[4];
            assert_eq!(run.failure, None);
            assert!(run.first_text.is_some());
            report(
                &format!(
                    "{name}/{}",
                    if staging {
                        "former-string-bridge"
                    } else {
                        "canonical-events"
                    }
                ),
                run,
                *elapsed,
                *counts,
                wire.len(),
                json_frames,
            );
        }
    }
    let history = large_history(256, 1024);
    let bytes = history.to_string().into_bytes();
    // Reconstruct the removed redundant full conversion used solely to obtain
    // ResponseOptions. Context construction itself still parses inbound bytes
    // once; this benchmark does not claim that parsing was eliminated.
    for redundant_translation in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..9 {
            samples.push(measured(|| {
                let context = message_history::Context::new(
                    &["p7-benchmark"],
                    ModelProtocol::Responses,
                    &bytes,
                )
                .unwrap();
                if redundant_translation {
                    let request = request_body(
                        Translation {
                            from: ModelProtocol::AnthropicMessages,
                            to: ModelProtocol::Responses,
                        },
                        &history,
                    )
                    .unwrap();
                    black_box(request.response_options);
                }
                black_box(context);
            }));
        }
        samples.sort_by_key(|sample| sample.1);
        let (_, elapsed, counts) = &samples[4];
        println!(
            "{}",
            json!({"case":if redundant_translation {"history-context-plus-redundant-conversion"}else{"history-context-tools-only"},
            "fixtureBytes":bytes.len(),"messageCount":history["messages"].as_array().unwrap().len(),
            "inboundJsonParses":1,"redundantFullTranslationCalls":usize::from(redundant_translation),
            "elapsedUs":elapsed.as_micros(),"allocationCalls":counts.allocation_calls,
            "reallocationCalls":counts.reallocation_calls,"allocationRequestedBytes":counts.requested_bytes})
        );
    }
    for target in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        let mut samples = Vec::new();
        for _ in 0..9 {
            let (request, elapsed, counts) = measured(|| {
                request_body(
                    Translation {
                        from: ModelProtocol::AnthropicMessages,
                        to: target,
                    },
                    &history,
                )
                .unwrap()
            });
            let output_bytes = serialized_bytes(&request.body);
            let input_items = request.body[if target == ModelProtocol::Responses {
                "input"
            } else {
                "messages"
            }]
            .as_array()
            .unwrap()
            .len();
            drop(request);
            samples.push((output_bytes, input_items, elapsed, counts));
        }
        samples.sort_by_key(|sample| sample.2);
        let (output_bytes, input_items, elapsed, counts) = &samples[4];
        println!(
            "{}",
            json!({"case":format!("large-history-{target:?}"),"fixtureBytes":bytes.len(),
            "elapsedUs":elapsed.as_micros(),"outputBytes":output_bytes,
            "inputItems":input_items,"allocationCalls":counts.allocation_calls,
            "reallocationCalls":counts.reallocation_calls,"allocationRequestedBytes":counts.requested_bytes})
        );
    }
    // Requested allocation volume counts successful alloc/realloc calls, not
    // resident memory, peak live bytes, tokenizer cost or real network latency.
}
