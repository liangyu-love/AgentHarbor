use super::super::{gateway, routes, types::ActionRequest};
use super::*;
use crate::core::{aggregation, model_targets::ModelTargetRef};

fn add_aggregate(store: &mut crate::core::router::Store, request: &mut ActionRequest) {
    store.providers[0].models[0].overrides.context_window = Some(128_000);
    store.aggregates.push(aggregation::AggregateModel {
        id: "pool".into(),
        public_model_id: "official-model".into(),
        display_name: "Official model".into(),
        members: vec![aggregation::AggregateMember {
            id: "first-member".into(),
            provider_id: "p1".into(),
            actual_model_id: "gpt-fixture".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    request.routes[0].target_ref = Some(ModelTargetRef::aggregate("pool"));
    request.routes[0].provider_id.clear();
    request.routes[0].model_id.clear();
}

#[test]
fn aggregate_role_keeps_saved_target_reapplies_and_restores_the_original_configuration() {
    let _env = env_lock();
    let (base, paths, manager, mut store, mut request) = router_fixture("aggregateSavedRestore");
    let normal = json!({"deploymentMode":"1p","mcpServers":{"user":{"command":"kept"}}});
    let three_p = json!({"deploymentMode":"1p","userPreference":"kept"});
    let meta = json!({"entries":[{"id":CC_SWITCH_ID,"name":"CC Switch","extra":"kept"}],"appliedId":CC_SWITCH_ID,"custom":"kept"});
    write(&paths.normal_config, &normal.to_string());
    write(&paths.third_party_config, &three_p.to_string());
    write(&paths.meta_path, &meta.to_string());
    add_aggregate(&mut store, &mut request);
    let saved: ActionRequest =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
    apply_router(&manager, &store, &saved);
    let state = manager.load_state().unwrap().unwrap();
    assert_eq!(
        state.router.as_ref().unwrap().routes[0].target_ref,
        Some(ModelTargetRef::aggregate("pool"))
    );
    let installed = read(&paths.profile_path).unwrap();
    assert!(!installed.contains(SENTINEL_KEY));
    let profile: Value = serde_json::from_str(&installed).unwrap();
    assert_eq!(
        profile[profile::KEY_BASE_URL],
        format!("http://127.0.0.1:{}/claude-desktop", store.router.port)
    );
    assert!(profile[profile::KEY_MODELS][0]["name"]
        .as_str()
        .unwrap()
        .starts_with("claude-sonnet-"));
    let snapshot = gateway::committed_snapshot(&base).unwrap().unwrap();
    let resolved = gateway::resolve_message(
        &snapshot,
        &store,
        &models_dev::load(&base),
        "claude-sonnet-4-6",
    )
    .unwrap();
    assert_eq!(resolved.aggregate_id.as_deref(), Some("pool"));
    assert!(resolved.provider.is_none());
    assert_eq!(resolved.view.model_id, "official-model");
    store.aggregates[0].members[0].weight = 7;
    store.providers[0].api_key = "rotated-upstream-key".into();
    let reapply = empty_management_request();
    let prepared = manager
        .prepare(&store, DesktopAction::Reapply, &reapply)
        .unwrap();
    let preview = manager.issue_ticket(&prepared).unwrap();
    manager
        .commit(&store, DesktopAction::Reapply, &preview.revision, &reapply)
        .unwrap();
    assert_eq!(read(&paths.profile_path).unwrap(), installed);
    let restore = manager
        .prepare(&store, DesktopAction::RestorePrevious, &reapply)
        .unwrap();
    let preview = manager.issue_ticket(&restore).unwrap();
    manager
        .commit(
            &store,
            DesktopAction::RestorePrevious,
            &preview.revision,
            &reapply,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&read(&paths.normal_config).unwrap()).unwrap(),
        normal
    );
    assert_eq!(
        serde_json::from_str::<Value>(&read(&paths.third_party_config).unwrap()).unwrap(),
        three_p
    );
    assert_eq!(
        serde_json::from_str::<Value>(&read(&paths.meta_path).unwrap()).unwrap(),
        meta
    );
    assert!(!paths.profile_path.exists());
}

#[test]
fn aggregate_one_m_requires_a_qualified_large_subset_across_every_supported_egress() {
    let _env = env_lock();
    for protocol in [
        ModelProtocol::Responses,
        ModelProtocol::ChatCompletions,
        ModelProtocol::AnthropicMessages,
    ] {
        let (base, _, manager, mut store, mut request) = router_fixture("aggregateOneMSubset");
        add_aggregate(&mut store, &mut request);
        request.routes[0].one_m_mode = OneMMode::Auto;
        let mut large = store.providers[0].clone();
        large.id = "large".into();
        large.models[0].protocol = Some(protocol);
        large.models[0].overrides.context_window = Some(1_000_000);
        store.providers.push(large);
        store.aggregates[0]
            .members
            .push(aggregation::AggregateMember {
                id: "large-member".into(),
                provider_id: "large".into(),
                actual_model_id: "gpt-fixture".into(),
                ..Default::default()
            });
        let catalog = models_dev::load(&base);
        let candidates = routes::aggregate_candidates(&store, &catalog);
        assert!(candidates[0].model.supports1m);
        assert_eq!(candidates[0].model.context_window, Some(128_000));
        apply_router(&manager, &store, &request);
        let snapshot = gateway::committed_snapshot(&base).unwrap().unwrap();
        assert!(
            gateway::resolve_message(&snapshot, &store, &catalog, "claude-sonnet-4-6[1m]")
                .unwrap()
                .view
                .one_m
        );
        store.providers[1].api_key = " \t ".into();
        assert!(
            !routes::aggregate_candidates(&store, &catalog)[0]
                .model
                .supports1m
        );
        assert!(
            gateway::resolve_message(&snapshot, &store, &catalog, "claude-sonnet-4-6[1m]").is_err()
        );
        assert!(gateway::resolve_message(&snapshot, &store, &catalog, "claude-sonnet-4-6").is_ok());
        store.providers[1].api_key = SENTINEL_KEY.into();
        request.routes[0].one_m_mode = OneMMode::Off;
        apply_router(&manager, &store, &request);
        let snapshot = gateway::committed_snapshot(&base).unwrap().unwrap();
        assert!(
            gateway::resolve_message(&snapshot, &store, &catalog, "claude-sonnet-4-6[1m]").is_err()
        );
    }
}

#[test]
fn deleted_aggregate_blocks_reapply_but_keeps_the_restore_path_available() {
    let _env = env_lock();
    let (_, paths, manager, mut store, mut request) = router_fixture("aggregateDeletedRestore");
    write(
        &paths.normal_config,
        r#"{"deploymentMode":"1p","mcpServers":{"kept":true}}"#,
    );
    add_aggregate(&mut store, &mut request);
    apply_router(&manager, &store, &request);
    let installed = read(&paths.profile_path).unwrap();
    store.aggregates.clear();
    let management = empty_management_request();
    assert!(manager
        .prepare(&store, DesktopAction::Reapply, &management)
        .is_err());
    assert_eq!(read(&paths.profile_path).unwrap(), installed);
    let prepared = manager
        .prepare(&store, DesktopAction::RestorePrevious, &management)
        .unwrap();
    let preview = manager.issue_ticket(&prepared).unwrap();
    manager
        .commit(
            &store,
            DesktopAction::RestorePrevious,
            &preview.revision,
            &management,
        )
        .unwrap();
    let normal: Value = serde_json::from_str(&read(&paths.normal_config).unwrap()).unwrap();
    assert_eq!(normal["deploymentMode"], "1p");
    assert_eq!(normal["mcpServers"]["kept"], true);
    assert!(!paths.profile_path.exists());
}
