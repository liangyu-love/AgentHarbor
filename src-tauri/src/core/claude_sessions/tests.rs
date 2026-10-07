use super::*;
use serde_json::json;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

fn record(role: &str, text: &str, index: usize) -> Value {
    json!({"type":role,"sessionId":ID,"uuid":format!("record-{index}"),"cwd":"D:\\workspace\\project",
        "timestamp":"2026-10-07T08:30:00Z","version":"2.1.3","message":{"role":role,"content":text}})
}
fn fixture(records: &[Value]) -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("projects");
    let project = root.join("D--workspace-project");
    fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{ID}.jsonl"));
    write_records(&path, records);
    let key = format!("claude:D--workspace-project/{ID}.jsonl");
    (directory, root, path, key)
}
fn write_records(path: &Path, records: &[Value]) {
    fs::write(
        path,
        records
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>(),
    )
    .unwrap();
}

#[test]
fn catalog_discovers_only_main_sessions_and_degrades_bad_files() {
    let (_directory, root, path, _) = fixture(&[record("user", "first prompt", 1)]);
    let project = path.parent().unwrap();
    fs::write(project.join(format!("{OTHER}.jsonl")), b"not json\n\xff\n").unwrap();
    fs::write(
        project.join("agent-extra.jsonl"),
        record("user", "subagent", 1).to_string(),
    )
    .unwrap();
    fs::create_dir_all(project.join(ID).join("subagents")).unwrap();
    fs::write(
        project
            .join(ID)
            .join("subagents")
            .join(format!("{OTHER}.jsonl")),
        record("user", "subagent", 1).to_string(),
    )
    .unwrap();
    let scan = scan_threads(&root).unwrap();
    assert_eq!(scan.items.len(), 1);
    assert_eq!(scan.warnings.len(), 1);
    assert_eq!(scan.items[0].cwd, "D:\\workspace\\project");
    assert_eq!(scan.items[0].folder_name, "project");
    assert_eq!(scan.items[0].source, "claudeCode");
    assert_eq!(scan.items[0].thread_source, "claudeJsonl");
    assert_eq!(scan.items[0].model_provider, "");
    assert!(scan_threads(&root.join("missing"))
        .unwrap()
        .items
        .is_empty());
}

#[test]
fn custom_title_and_streamed_assistant_usage_are_not_double_counted() {
    let usage = json!({"input_tokens":10,"cache_read_input_tokens":50,"cache_creation_input_tokens":20,"output_tokens":3});
    let first = json!({"sessionId":ID,"uuid":"assistant-chunk-1","timestamp":"2026-10-07T08:31:00Z","message":{
        "id":"msg-1","role":"assistant","model":"deepseek-flash","usage":usage,
        "content":[{"type":"thinking","thinking":"consider"}]}});
    let mut second = first.clone();
    second["uuid"] = json!("assistant-chunk-2");
    second["message"]["content"] = json!([{"type":"text","text":"answer"}]);
    second["message"]["usage"]["output_tokens"] = json!(8);
    let records = [
        record("user", "first prompt", 1),
        first,
        second.clone(),
        second,
        json!({"type":"custom-title","customTitle":"My title","sessionId":ID}),
    ];
    let (_directory, root, _path, key) = fixture(&records);
    let scan = scan_threads(&root).unwrap();
    let summary = &scan.items[0];
    assert_eq!(summary.title, "My title");
    assert_eq!(summary.user_messages, 1);
    assert_eq!(summary.agent_messages, 1);
    assert_eq!(summary.model, "deepseek-flash");
    assert_eq!(summary.tokens_used, 88);
    assert_eq!(summary.input_tokens, Some(80));
    assert_eq!(summary.cached_tokens, Some(50));
    assert_eq!(summary.output_tokens, Some(8));
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.usage.unwrap().total, 88);
    assert_eq!(details.by_model[0].usage.total, 88);
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[1].role, "reasoning");
    assert_eq!(page.messages[2].text, "answer");
}

#[test]
fn pages_are_bounded_append_compatible_and_reject_same_length_replacements() {
    let records: Vec<_> = (0..120)
        .map(|index| record("user", &format!("message {index:03}"), index))
        .collect();
    let (_directory, root, path, key) = fixture(&records);
    let details = read_details(&root, &key, ID).unwrap();
    let first = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(first.messages.len(), PAGE_SIZE);
    assert_eq!(first.next_offset, Some(PAGE_SIZE));
    let last = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        100,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(last.messages.len(), 20);
    assert!(last.next_offset.is_none());
    writeln!(
        OpenOptions::new().append(true).open(&path).unwrap(),
        "{}",
        record("user", "new", 121)
    )
    .unwrap();
    let old = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        100,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(old.messages.len(), 20);
    let original = fs::read_to_string(&path).unwrap();
    let changed = original.replace("message 100", "CHANGED 100");
    assert_eq!(changed.len(), original.len());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, changed).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        100,
        Some(&details.snapshot_id)
    )
    .is_err());
    fs::write(&path, "{}").unwrap();
    assert!(read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id)
    )
    .is_err());
}

#[test]
fn tool_results_and_invalid_lines_do_not_appear_as_user_prompts() {
    let assistant = json!({"sessionId":ID,"uuid":"assistant","message":{"role":"assistant","content":[
        {"type":"tool_use","id":"tool-1","name":"Read","input":{"path":"sample.rs"}}]}});
    let result = json!({"sessionId":ID,"uuid":"result","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"tool-1","content":[{"type":"text","text":"file content"}]}]}});
    let (_directory, root, path, key) = fixture(&[
        record("user", "read file", 1),
        assistant.clone(),
        assistant,
        result,
    ]);
    writeln!(
        OpenOptions::new().append(true).open(&path).unwrap(),
        "invalid"
    )
    .unwrap();
    let summary = &scan_threads(&root).unwrap().items[0];
    assert_eq!(summary.user_messages, 1);
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.tools[0].calls, 1);
    assert_eq!(details.diagnostics.invalid_lines, 1);
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        ["user", "toolCall", "toolResult"]
    );
    assert_eq!(page.messages[2].text, "file content");
}

#[test]
fn path_traversal_and_session_identity_mismatch_are_rejected() {
    let (_directory, root, path, key) = fixture(&[record("user", "safe", 1)]);
    for invalid in [
        "claude:../outside.jsonl",
        "active:projects/file.jsonl",
        "claude:D--workspace-project/../../outside.jsonl",
        "claude:D--workspace-project\\file.jsonl",
        "claude:D--workspace-project/agent-extra.jsonl",
    ] {
        assert!(read_details(&root, invalid, ID).is_err(), "{invalid}");
    }
    assert!(read_details(&root, &key, OTHER).is_err());
    let mut mismatch = record("user", "unsafe", 1);
    mismatch["sessionId"] = json!(OTHER);
    write_records(&path, &[mismatch]);
    assert!(read_details(&root, &key, ID).is_err());
    assert!(scan_threads(&root).unwrap().items.is_empty());
}

#[test]
fn exports_preserve_raw_jsonl_and_never_overwrite_or_mutate_sessions() {
    let (directory, root, path, key) = fixture(&[
        record("user", "prompt", 1),
        record("assistant", "response", 2),
    ]);
    let before = fs::read(&path).unwrap();
    let target = directory.path().join("exports");
    fs::create_dir_all(&target).unwrap();
    let original_export = target.join(format!("claude-{ID}.jsonl"));
    fs::write(&original_export, "keep existing").unwrap();
    let jsonl = export_threads(
        &root,
        &[key.clone(), key.clone()],
        &target,
        ThreadExportFormat::Jsonl,
    )
    .unwrap();
    assert_eq!(jsonl.exported, 1);
    assert_eq!(fs::read(target.join(&jsonl.files[0])).unwrap(), before);
    assert_eq!(
        fs::read_to_string(original_export).unwrap(),
        "keep existing"
    );
    let markdown = export_threads(&root, &[key], &target, ThreadExportFormat::Markdown).unwrap();
    let text = fs::read_to_string(target.join(&markdown.files[0])).unwrap();
    assert!(text.contains("## user"));
    assert!(text.contains("prompt"));
    assert!(text.contains("response"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!fs::read_dir(target).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn message_text_and_oversized_jsonl_lines_are_bounded() {
    let (_directory, root, path, key) =
        fixture(&[record("user", &"字".repeat(MAX_TEXT_CHARS + 1), 1)]);
    let mut append = OpenOptions::new().append(true).open(&path).unwrap();
    append.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).unwrap();
    append.write_all(b"\n").unwrap();
    drop(append);
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.diagnostics.oversized_lines, 1);
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(page.messages[0].truncated);
    assert_eq!(page.messages[0].text.chars().count(), MAX_TEXT_CHARS);
}

#[test]
fn partial_transcript_warnings_survive_summary_cache_hits() {
    let (_directory, root, path, _) = fixture(&[record("user", "usable", 1)]);
    writeln!(
        OpenOptions::new().append(true).open(path).unwrap(),
        "{{truncated"
    )
    .unwrap();
    for _ in 0..2 {
        let scan = scan_threads(&root).unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].message.contains("1 invalid"));
    }
}

#[test]
fn final_usage_fields_are_aggregated_once_and_unknown_caches_stay_unknown() {
    let first = json!({"sessionId":ID,"uuid":"part-1","cwd":"D:\\workspace\\project","message":{
        "id":"msg-1","role":"assistant","usage":{"input_tokens":60,"output_tokens":3},
        "content":[{"type":"text","text":"answer"}]}});
    let mut final_record = first.clone();
    final_record["uuid"] = json!("part-2");
    final_record["timestamp"] = json!("2026-10-07T10:00:00Z");
    final_record["message"]["model"] = json!("deepseek-flash");
    final_record["message"]["usage"] =
        json!({"input_tokens":10,"cache_read_input_tokens":50,"output_tokens":3});
    let (_directory, root, path, key) = fixture(&[first.clone(), final_record.clone()]);
    let summary = &scan_threads(&root).unwrap().items[0];
    assert_eq!(summary.tokens_used, 63);
    assert_eq!(summary.cached_tokens, Some(50));
    assert_eq!(summary.title, "project");
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.usage.unwrap().cached, Some(50));
    assert_eq!(details.by_model[0].label, "deepseek-flash");
    assert_eq!(details.by_model[0].usage.total, 63);
    assert_eq!(details.by_date[0].label, "2026-10-07");
    assert_eq!(details.by_date[0].usage.cached, Some(50));
    assert_eq!(details.diagnostics.undated_usage, 0);
    let mut unknown = first.clone();
    unknown["uuid"] = json!("another");
    unknown["message"]["id"] = json!("msg-2");
    write_records(&path, &[first, final_record, unknown]);
    assert_eq!(
        read_details(&root, &key, ID).unwrap().usage.unwrap().cached,
        None
    );
}

#[test]
fn compact_boundaries_and_tool_calls_keep_their_transcript_roles() {
    let command = record("user", "<command-name>/compact</command-name>", 1);
    let compact = json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted",
        "sessionId":ID,"uuid":"compact"});
    let (_directory, root, _path, key) =
        fixture(&[command, compact, record("user", "real prompt", 2)]);
    assert_eq!(scan_threads(&root).unwrap().items[0].title, "real prompt");
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.diagnostics.compactions, 1);
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(page.messages[1].role, "compaction");
    assert_eq!(page.messages[1].text, "Conversation compacted");
}

#[test]
fn long_record_ids_do_not_alias_and_identity_limits_are_explicit() {
    let prefix = "a".repeat(300);
    let mut first = record("user", "first", 1);
    first["uuid"] = json!(format!("{prefix}1"));
    let mut second = record("user", "second", 2);
    second["uuid"] = json!(format!("{prefix}2"));
    let (_directory, root, _path, key) = fixture(&[first.clone(), second, first]);
    assert_eq!(scan_threads(&root).unwrap().items[0].user_messages, 2);
    let details = read_details(&root, &key, ID).unwrap();
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(page.messages.len(), 2);
    let mut seen = HashSet::new();
    assert!(remember(&mut seen, "one", 2).unwrap());
    assert!(remember(&mut seen, "two", 2).unwrap());
    assert!(!remember(&mut seen, "one", 2).unwrap());
    assert!(remember(&mut seen, "three", 2).is_err());
}

#[test]
fn failed_multi_export_cleans_staging_and_leaves_input_and_existing_exports_intact() {
    let (directory, root, path, key) = fixture(&[record("user", "prompt", 1)]);
    let original = fs::read(&path).unwrap();
    let target = directory.path().join("exports");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("existing.md"), "keep").unwrap();
    assert!(export_threads(
        &root,
        &[key, "claude:../bad.jsonl".into()],
        &target,
        ThreadExportFormat::Markdown
    )
    .is_err());
    let remaining: Vec<_> = fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(remaining, [std::ffi::OsString::from("existing.md")]);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        fs::read_to_string(target.join("existing.md")).unwrap(),
        "keep"
    );
}

#[test]
fn hosted_and_structured_tool_results_are_preserved() {
    let hosted = json!({"sessionId":ID,"uuid":"hosted","message":{"role":"assistant","content":[
        {"type":"server_tool_use","id":"search-1","name":"web_search","input":{"query":"rust"}},
        {"type":"web_search_tool_result","tool_use_id":"search-1","content":[
            {"type":"web_search_result","title":"Rust","url":"https://www.rust-lang.org"}]}]}});
    let error = json!({"sessionId":ID,"uuid":"result","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"local-1","content":{"error":"unavailable"}}]}});
    let (_directory, root, _path, key) = fixture(&[hosted, error]);
    let details = read_details(&root, &key, ID).unwrap();
    assert_eq!(details.tools[0].name, "web_search");
    let page = read_messages_versioned(
        &root,
        &key,
        ID,
        details.snapshot_bytes,
        0,
        Some(&details.snapshot_id),
    )
    .unwrap();
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[1].role, "toolResult");
    assert!(page.messages[1].text.contains("https://www.rust-lang.org"));
    assert!(page.messages[2].text.contains("unavailable"));
}
