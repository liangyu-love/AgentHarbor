use super::*;
use crate::{
    core::{model_policy, models_dev, newapi, router, secret_file},
    platform::paths::CodexPaths,
};

fn catalog() -> models_dev::Catalog {
    models_dev::Catalog::from_snapshot(
        models_dev::Snapshot {
            schema_version: models_dev::SCHEMA_VERSION,
            revision: "aggregation-fixture".into(),
            providers: vec![models_dev::Provider {
                id: "fixture".into(),
                name: "Fixture".into(),
                models: vec![models_dev::Model {
                    id: "unrelated-model".into(),
                    name: "Fixture model".into(),
                    limit: models_dev::Limit {
                        context: Some(128_000),
                        input: None,
                        output: Some(8192),
                    },
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
        models_dev::Origin::Embedded,
    )
    .unwrap()
}

fn provider(
    id: &str,
    wire: &str,
    protocol: router::ModelProtocol,
    context: u64,
    input: u64,
) -> router::Provider {
    router::Provider {
        id: id.into(),
        name: format!("Provider {id}"),
        base_url: format!("https://{id}.example/"),
        api_key: "fixture-secret-not-for-dto".into(),
        model: wire.into(),
        protocol,
        enabled: true,
        models: vec![newapi::Model {
            id: wire.into(),
            included: true,
            protocol: Some(protocol),
            overrides: newapi::ModelOverrides {
                context_window: Some(context),
                input_limit: Some(input),
                max_output_tokens: Some(8192),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn aggregate() -> AggregateModel {
    AggregateModel {
        id: "aggregate-a".into(),
        public_model_id: "official-model".into(),
        display_name: "Official model".into(),
        members: vec![
            AggregateMember {
                id: "member-a".into(),
                provider_id: "a".into(),
                actual_model_id: "prefix/model-a".into(),
                ..Default::default()
            },
            AggregateMember {
                id: "member-b".into(),
                provider_id: "b".into(),
                actual_model_id: "platform::model-b".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn store() -> router::Store {
    router::Store {
        providers: vec![
            provider(
                "a",
                "prefix/model-a",
                router::ModelProtocol::Responses,
                128_000,
                100_000,
            ),
            provider(
                "b",
                "platform::model-b",
                router::ModelProtocol::AnthropicMessages,
                1_000_000,
                700_000,
            ),
        ],
        aggregates: vec![aggregate()],
        ..Default::default()
    }
}

#[test]
fn mixed_protocol_members_preserve_full_wire_identity_and_conservative_budget() {
    let store = store();
    let model = &store.aggregates[0];
    let catalog = catalog();
    let a = resolve_member(&store, model, &model.members[0], &catalog).unwrap();
    let b = resolve_member(&store, model, &model.members[1], &catalog).unwrap();
    assert_eq!(a.member.actual_model_id, "prefix/model-a");
    assert_eq!(b.member.actual_model_id, "platform::model-b");
    assert_eq!(b.protocol, router::ModelProtocol::AnthropicMessages);
    let view = project(&store, model, &catalog);
    assert!(view.available);
    let projection = view.projection.unwrap();
    assert_eq!(projection.context_window, 128_000);
    assert_eq!(projection.budget, 100_000);
    assert_eq!(projection.output_limit, Some(8192));
    assert_eq!(projection.compression.threshold, Some(90_000));
}

#[test]
fn custom_public_identity_inherits_real_member_overrides() {
    let mut store = store();
    store.aggregates[0].public_model_id = "my-custom-model".into();
    store.aggregates[0].canonical_ref = None;
    config::validate(&store).unwrap();
    let view = project(&store, &store.aggregates[0], &catalog());
    let projection = view.projection.unwrap();
    assert_eq!(projection.context_window, 128_000);
    assert_eq!(projection.context_source, "aggregate");
    assert!(!projection.context_is_fallback);
    assert_eq!(projection.budget, 100_000);
    assert_eq!(projection.output_limit, Some(8192));
    assert_eq!(projection.output_source, "aggregate");
    assert_eq!(projection.compression.threshold, Some(90_000));
    assert_eq!(
        store.aggregates[0].members[0].actual_model_id,
        "prefix/model-a"
    );
    assert_eq!(
        store.aggregates[0].members[1].actual_model_id,
        "platform::model-b"
    );
}

#[test]
fn unmatched_member_defaults_stay_identifiable_until_capacity_is_configured() {
    let mut store = store();
    store.providers[0].models[0].overrides = newapi::ModelOverrides::default();
    let view = project(&store, &store.aggregates[0], &catalog());
    assert!(view.available);
    let projection = view.projection.unwrap();
    assert_eq!(projection.context_window, 128_000);
    assert!(projection.context_is_fallback);
    assert_eq!(projection.context_source, "fallback");
    assert_eq!(projection.budget_source, "fallback");
    assert_eq!(projection.output_limit, Some(8192));
    assert_eq!(projection.output_source, "fallback");
    let serialized = serde_json::to_value(&projection).unwrap();
    assert_eq!(serialized["contextIsFallback"], true);
    assert_eq!(serialized["contextSource"], "fallback");
    assert_eq!(serialized["outputSource"], "fallback");

    store.providers[0].models[0].overrides.context_window = Some(256_000);
    store.providers[0].models[0].overrides.input_limit = Some(200_000);
    let projection = project(&store, &store.aggregates[0], &catalog())
        .projection
        .unwrap();
    assert_eq!(projection.context_window, 256_000);
    assert!(!projection.context_is_fallback);
    assert_eq!(projection.context_source, "aggregate");
    assert_eq!(projection.budget, 200_000);
    assert_eq!(projection.output_source, "fallback");

    store.providers[0].models[0].overrides.max_output_tokens = Some(16_384);
    let projection = project(&store, &store.aggregates[0], &catalog())
        .projection
        .unwrap();
    assert_eq!(projection.output_limit, Some(8192));
    assert_eq!(projection.output_source, "aggregate");
    assert_eq!(projection.compression.threshold, Some(180_000));

    store.providers[0].models[0].overrides = newapi::ModelOverrides::default();
    store.aggregates[0].members[0].enabled = false;
    let projection = project(&store, &store.aggregates[0], &catalog())
        .projection
        .unwrap();
    assert_eq!(projection.context_window, 1_000_000);
    assert!(!projection.context_is_fallback);
    assert_eq!(projection.context_source, "aggregate");
    assert_eq!(projection.output_source, "aggregate");
}

#[test]
fn disabled_missing_or_excluded_members_remain_visible() {
    let mut store = store();
    store.providers[1].models[0].included = false;
    let view = project(&store, &store.aggregates[0], &catalog());
    assert!(!view.member_states[1].available);
    assert_eq!(
        view.member_states[1].reason.as_deref(),
        Some("model_unavailable")
    );
    store.providers.clear();
    let view = project(&store, &store.aggregates[0], &catalog());
    assert!(!view.available);
    assert_eq!(view.member_states.len(), 2);
    assert!(view
        .member_states
        .iter()
        .all(|m| m.reason.as_deref() == Some("provider_missing")));
    config::validate(&store).unwrap();
}

#[test]
fn physical_identity_is_shared_across_cards_and_stable_for_balance_and_labels() {
    let mut provider = provider(
        "a",
        "prefix/model-a",
        router::ModelProtocol::Responses,
        128_000,
        100_000,
    );
    let first = target_key(
        &provider,
        "prefix/model-a",
        router::ModelProtocol::Responses,
    );
    provider.id = "duplicate-card".into();
    provider.name = "Renamed".into();
    provider.updated_at = 42;
    provider.balance_source = Some("refreshed".into());
    assert_eq!(
        first,
        target_key(
            &provider,
            "prefix/model-a",
            router::ModelProtocol::Responses
        )
    );
    provider.api_key = "rotated-credential".into();
    assert_ne!(
        first,
        target_key(
            &provider,
            "prefix/model-a",
            router::ModelProtocol::Responses
        )
    );
    assert_ne!(
        first,
        target_key(
            &provider,
            "different-model",
            router::ModelProtocol::Responses
        )
    );
    assert_ne!(
        first,
        target_key(
            &provider,
            "prefix/model-a",
            router::ModelProtocol::ChatCompletions
        )
    );
}

#[test]
fn topology_revision_ignores_balance_price_and_display_changes() {
    let mut store = store();
    let revision = projection::route_revision(&store, &store.aggregates[0]);
    store.providers[0].updated_at = 55;
    store.providers[0].name = "Renamed provider".into();
    store.providers[0].price_catalog_provider = Some("different-price-catalog".into());
    store.aggregates[0].display_name = "Renamed aggregate".into();
    assert_eq!(
        revision,
        projection::route_revision(&store, &store.aggregates[0])
    );
    store.aggregates[0].members[0].weight = 2;
    assert_ne!(
        revision,
        projection::route_revision(&store, &store.aggregates[0])
    );
}

#[test]
fn store_save_is_atomic_public_identity_immutable_and_dto_has_no_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let initial = store();
    router::mutate_store(&paths, |store| {
        store.providers = initial.providers.clone();
        Ok(())
    })
    .unwrap();
    let state = save(&paths, aggregate()).unwrap();
    let dto = serde_json::to_string(&state).unwrap();
    assert!(dto.contains("official-model"));
    assert!(!dto.contains("fixture-secret-not-for-dto"));
    assert!(!dto.contains("https://a.example"));
    let path = paths.app_data_dir.join("newapi-providers.dat");
    let before = secret_file::read(&path).unwrap().unwrap();
    let mut changed = aggregate();
    changed.public_model_id = "different-official".into();
    assert_eq!(
        save(&paths, changed).err().as_deref(),
        Some("aggregation.errors.publicIdentityImmutable")
    );
    assert_eq!(before, secret_file::read(&path).unwrap().unwrap());
    let mut duplicate = aggregate();
    duplicate.id = "aggregate-duplicate".into();
    assert_eq!(
        save(&paths, duplicate).err().as_deref(),
        Some("aggregation.errors.duplicateIdentity")
    );
    assert_eq!(before, secret_file::read(&path).unwrap().unwrap());
    set_external_enabled(&paths, true).unwrap();
    assert!(state_from_store_for_test(&paths).external_enabled);
    router::mutate_store(&paths, |store| {
        store.router.aggregate_model_id = Some("aggregate-a".into());
        Ok(())
    })
    .unwrap();
    delete(&paths, "aggregate-a").unwrap();
    let stored = router::load_store(&paths).unwrap();
    assert!(stored.aggregates.is_empty());
    assert_eq!(
        stored.router.aggregate_model_id.as_deref(),
        Some("aggregate-a")
    );
}

fn state_from_store_for_test(paths: &CodexPaths) -> AggregationState {
    state(paths).unwrap()
}

#[test]
fn v8_migration_preserves_credentials_and_first_protected_backup() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let mut store = store();
    store.version = 8;
    store.aggregates.clear();
    store.router.bearer_token = "agentharbor_fixture".into();
    let mut value = serde_json::to_value(&store).unwrap();
    value.as_object_mut().unwrap().remove("aggregates");
    value["router"]["sameModelFailover"] = true.into();
    let original = serde_json::to_vec(&value).unwrap();
    secret_file::write(&paths.app_data_dir.join("newapi-providers.dat"), &original).unwrap();
    let loaded = router::load_store(&paths).unwrap();
    assert_eq!(loaded.version, 9);
    assert!(loaded.aggregates.is_empty());
    assert!(!loaded.router.aggregate_external_enabled);
    assert_eq!(loaded.providers[0].api_key, "fixture-secret-not-for-dto");
    assert_eq!(loaded.router.bearer_token, "agentharbor_fixture");
    let migrated = secret_file::read(&paths.app_data_dir.join("newapi-providers.dat"))
        .unwrap()
        .unwrap();
    let migrated: serde_json::Value = serde_json::from_slice(&migrated).unwrap();
    assert!(migrated["router"].get("sameModelFailover").is_none());
    let backup = paths.app_data_dir.join("newapi-providers.pre-v9.dat");
    assert_eq!(secret_file::read(&backup).unwrap().unwrap(), original);
    router::load_store(&paths).unwrap();
    assert_eq!(secret_file::read(&backup).unwrap().unwrap(), original);
}

#[test]
fn invalid_ranges_missing_refs_and_duplicate_members_fail_without_saving() {
    let mut store = store();
    store.aggregates[0].members[0].weight = 0;
    assert_eq!(
        config::validate(&store).err().as_deref(),
        Some("aggregation.errors.invalidMember")
    );
    store.aggregates[0].members[0].weight = 1;
    let member = store.aggregates[0].members[0].clone();
    store.aggregates[0].members.push(member);
    assert_eq!(
        config::validate(&store).err().as_deref(),
        Some("aggregation.errors.duplicateMember")
    );
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    assert_eq!(
        save(&paths, aggregate()).err().as_deref(),
        Some("aggregation.errors.providerMissing")
    );
    let state = router::load_store(&paths).unwrap();
    assert!(state.aggregates.is_empty());
    let mut model = aggregate();
    model.retry_policy.max_attempts = 3;
    store.aggregates = vec![model];
    assert_eq!(
        config::validate(&store).err().as_deref(),
        Some("aggregation.errors.retryLimit")
    );
    store.aggregates[0].retry_policy.max_attempts = 2;
    store.aggregates[0].compression = model_policy::CompressionPolicy::Tokens { tokens: 0 };
    assert_eq!(
        config::validate(&store).err().as_deref(),
        Some("aggregation.errors.invalidCompression")
    );
}
