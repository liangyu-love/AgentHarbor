use super::*;
use std::fs;

fn catalog() -> models_dev::Catalog {
    models_dev::Catalog::parse(
        serde_json::json!({
            "schemaVersion": 1, "sourceUrl": "fixture", "fetchedAt": 1, "revision": "fixture",
            "providers": [
                {"id": "openai", "models": [{"id": "gpt-x", "cost": {"input": 1.0, "output": 4.0, "cacheRead": 0.1}}]},
                {"id": "anthropic", "models": [{"id": "claude-x", "cost": {"input": 3.0, "output": 15.0}}]}
            ]
        })
        .to_string()
        .as_bytes(),
        models_dev::Origin::Embedded,
    )
    .unwrap()
}

fn turn(
    source: &str,
    model: &str,
    timestamp: i64,
    input: u64,
    read: u64,
    output: u64,
) -> UsageRecord {
    UsageRecord {
        request_id: format!("{source}:{model}:{timestamp}"),
        timestamp,
        source_id: source.into(),
        model: model.into(),
        session_id: format!("session-{timestamp}"),
        project: "/work/harbor".into(),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: read,
        total_tokens: input + read + output,
        ..Default::default()
    }
}

fn request(
    client: Option<&str>,
    provider: &str,
    model: &str,
    timestamp: i64,
    input: u64,
    read: u64,
    output: u64,
) -> RelaySample {
    RelaySample {
        timestamp,
        client_source: client.map(str::to_owned),
        provider_id: provider.into(),
        provider_name: format!("Relay {provider}"),
        model: model.into(),
        status_code: 200,
        success: true,
        duration_ms: 5_000,
        usage_reported: true,
        cache_reported: true,
        tokens: TokenCounts {
            input_tokens: input,
            cache_read_tokens: read,
            output_tokens: output,
            total_tokens: input + read + output,
            ..Default::default()
        },
        cost_usd: Some(0.5),
        pricing_status: "priced".into(),
        pricing_source: "manual".into(),
        ..Default::default()
    }
}

fn collected(records: Vec<UsageRecord>, samples: Vec<RelaySample>) -> Collected {
    Collected {
        sources: Vec::new(),
        records,
        warnings: Vec::new(),
        samples,
    }
}

fn official(record: &UsageRecord) -> UsageEntry {
    let catalog = catalog();
    let mut pricer = Pricer {
        catalog: &catalog,
        providers: HashMap::new(),
        known: HashMap::new(),
    };
    local_entry(record, &mut pricer, &HashSet::new(), &HashMap::new())
}

#[test]
fn a_routed_turn_and_the_request_that_carried_it_count_once() {
    let collected = collected(
        vec![
            turn("codex", "relay-a::gpt-x", 1_000, 100, 50, 20),
            turn("codex", "gpt-x", 2_000, 1_000, 0, 100),
        ],
        vec![
            request(Some("codex"), "relay-a", "gpt-x", 990, 100, 50, 20),
            request(Some("opencode"), "relay-b", "glm", 3_000, 10, 0, 5),
        ],
    );
    let entries = join(&collected, None, &catalog());
    assert_eq!(entries.len(), 3);
    let routed = entries.iter().find(|entry| entry.timestamp == 990).unwrap();
    assert_eq!(
        (routed.client.as_str(), routed.channel),
        ("codex", Channel::Relay)
    );
    assert_eq!(routed.provider_name.as_deref(), Some("Relay relay-a"));
    assert_eq!(routed.cost_usd, Some(0.5));
    assert!(!routed.estimated);
    assert_eq!(routed.project.as_deref(), Some("/work/harbor"));
    assert!(routed.request.as_ref().unwrap().matched);

    let official = entries
        .iter()
        .find(|entry| entry.timestamp == 2_000)
        .unwrap();
    assert_eq!(official.channel, Channel::Official);
    assert!(official.estimated);
    assert_eq!(official.pricing_source.as_deref(), Some("catalogReference"));
    assert!((official.cost_usd.unwrap() - 0.0014).abs() < 1e-12);

    let opencode = entries
        .iter()
        .find(|entry| entry.client == "opencode")
        .unwrap();
    assert_eq!(opencode.channel, Channel::Relay);
    assert!(!opencode.request.as_ref().unwrap().matched);
}

#[test]
fn routed_turns_pair_by_time_when_the_two_sides_count_differently() {
    let collected = collected(
        vec![
            turn("codex", "relay-a::gpt-x", 1_010, 90, 0, 20),
            turn("claude", "claude-x", 5_002, 7, 0, 3),
            turn("claude", "claude-x", 9_000, 7, 0, 3),
        ],
        vec![
            request(Some("codex"), "relay-a", "gpt-x", 1_000, 100, 0, 20),
            request(Some("codex"), "relay-z", "gpt-x", 1_000, 90, 0, 20),
            request(Some("claudeCode"), "relay-c", "claude-x", 5_000, 8, 0, 3),
        ],
    );
    let pairs = pair(&collected.records, &collected.samples);
    // relay-z has the exact counts, but Codex named relay-a as the provider.
    assert_eq!(pairs, vec![Some(0), Some(2), None]);
    let entries = join(&collected, None, &catalog());
    let unrouted = entries
        .iter()
        .find(|entry| entry.timestamp == 9_000)
        .unwrap();
    assert_eq!(unrouted.channel, Channel::Official);
    assert!((unrouted.cost_usd.unwrap() - (7.0 * 3.0 + 3.0 * 15.0) / 1e6).abs() < 1e-12);
}

#[test]
fn untagged_requests_pair_with_either_client_and_aggregates_keep_their_channel() {
    let store = router::Store {
        aggregates: vec![super::super::aggregation::AggregateModel {
            public_model_id: "pool/fast".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut aggregate = request(None, "relay-a", "gpt-x", 1_000, 10, 0, 1);
    aggregate.requested_model = Some("pool/fast".into());
    let collected = collected(
        vec![turn("claude", "claude-x", 1_002, 10, 0, 1)],
        vec![aggregate],
    );
    let entries = join(&collected, Some(&store), &catalog());
    assert_eq!(entries.len(), 1);
    assert_eq!(
        (entries[0].client.as_str(), entries[0].channel),
        ("claudeCode", Channel::Aggregate)
    );
}

#[test]
fn a_range_compares_with_the_same_span_before_it() {
    let now = Local.with_ymd_and_hms(2026, 10, 17, 15, 0, 0).unwrap();
    let at = |days: i64, hours: i64| {
        (now - ChronoDuration::days(days) - ChronoDuration::hours(hours)).timestamp()
    };
    let entries = [at(0, 1), at(1, 1), at(1, -1), at(3, 0), at(9, 0)]
        .into_iter()
        .map(|timestamp| official(&turn("codex", "gpt-x", timestamp, 100, 0, 10)))
        .collect::<Vec<_>>();
    let empty = collected(Vec::new(), Vec::new());
    let today = summarize(&empty, entries.clone(), UsageRange::Today, now);
    // Yesterday counts only up to the same time of day.
    assert_eq!(
        (today.current.request_count, today.previous.request_count),
        (1, 1)
    );
    assert_eq!(today.buckets.len(), 24);
    assert_eq!(today.buckets[14].totals.request_count, 1);
    assert_eq!(today.today.new_tokens, 110);
    let week = summarize(&empty, entries, UsageRange::Week, now);
    assert_eq!(
        (week.current.request_count, week.previous.request_count),
        (4, 1)
    );
    assert_eq!(week.buckets.len(), 7);
    assert_eq!(week.buckets.last().unwrap().key, "2026-10-17");
    assert_eq!(week.all_time.request_count, 5);
}

#[test]
fn rankings_name_projects_and_sessions_and_charts_fold_the_rest() {
    let now = Local.with_ymd_and_hms(2026, 10, 17, 15, 0, 0).unwrap();
    let entries = (0..8_u64)
        .map(|index| {
            let mut record = turn(
                "codex",
                &format!("model-{index}"),
                now.timestamp() - 60,
                100 + index,
                0,
                0,
            );
            record.project = format!("D:\\work\\app-{index}\\");
            record.session_id = format!("s{index}");
            official(&record)
        })
        .collect();
    let payload = summarize(
        &collected(Vec::new(), Vec::new()),
        entries,
        UsageRange::Week,
        now,
    );
    assert_eq!(payload.by_project[0].label, "app-7");
    assert_eq!(
        payload.by_project[0].detail.as_deref(),
        Some("D:\\work\\app-7\\")
    );
    assert_eq!(payload.by_session[0].detail.as_deref(), Some("app-7"));
    assert_eq!(payload.by_model[0].detail.as_deref(), Some("official"));
    let today = payload.buckets.last().unwrap();
    assert_eq!(today.by_model.len(), PART_LIMIT + 1);
    let other = today
        .by_model
        .iter()
        .find(|part| part.key == "other")
        .unwrap();
    assert_eq!(other.new_tokens, 100 + 101);
    assert_eq!(today.by_channel.len(), 1);
}

fn relay_line(timestamp: i64, total: u64) -> String {
    serde_json::json!({
        "attemptId": format!("attempt-{timestamp}"), "timestamp": timestamp, "date": "2026-10-01",
        "providerId": "relay-a", "providerName": "Relay A", "model": "gpt-x", "protocol": "responses",
        "statusCode": 200, "durationMs": 10, "success": true, "usageReported": true,
        "inputTokens": total, "outputTokens": 0, "totalTokens": total, "clientSource": "pi",
        "costUsd": 0.25, "pricingStatus": "priced"
    })
    .to_string()
}

#[test]
fn router_requests_outlive_the_request_log_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, claude) = (dir.path().join("codex"), dir.path().join("claude"));
    let paths = LedgerPaths {
        codex_home: &codex,
        claude_home: &claude,
        data_dir: dir.path(),
    };
    let log = relay_usage::usage_path(dir.path());
    fs::write(
        &log,
        format!("{}\n{}\n", relay_line(1_000, 10), relay_line(2_000, 20)),
    )
    .unwrap();
    record(&paths).unwrap();
    record(&paths).unwrap();
    // The log folded its older request away; the ledger still has it.
    fs::write(&log, format!("{}\n", relay_line(2_000, 20))).unwrap();
    let payload = load(&paths, None, &catalog(), UsageRange::Month).unwrap();
    assert_eq!(payload.sources.relay_count, 2);
    assert_eq!(payload.all_time.request_count, 2);
    assert_eq!(payload.all_time.total_tokens, 30);
    assert_eq!(payload.all_time.cost_micros, 500_000);
    assert_eq!(payload.sources.relay_since, Some(1_000));
}

#[test]
fn a_broken_ledger_is_kept_aside_instead_of_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join(LEDGER_FILE);
    fs::write(&ledger, "not a database").unwrap();
    let (codex, claude) = (dir.path().join("codex"), dir.path().join("claude"));
    let paths = LedgerPaths {
        codex_home: &codex,
        claude_home: &claude,
        data_dir: dir.path(),
    };
    let payload = load(&paths, None, &catalog(), UsageRange::Week).unwrap();
    assert_eq!(payload.all_time.request_count, 0);
    let aside = dir.path().join(format!("{LEDGER_FILE}.broken"));
    assert_eq!(fs::read_to_string(aside).unwrap(), "not a database");
}
