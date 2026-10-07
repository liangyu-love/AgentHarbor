use super::*;
use std::fs;

const ORIGINAL: &str = r#"model = "original"
model_provider = "custom"
theme = "light"
[agents]
enabled = true
max_threads = 8 # user limit
default_subagent_model = "user-agent" # user model
default_subagent_reasoning_effort = "medium"
[agents.reviewer]
config_file = "review.toml"
[model_providers.custom]
base_url = "https://custom.invalid"
"#;

fn fixture() -> (tempfile::TempDir, CodexPaths, String) {
    let dir = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(dir.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.config_path, ORIGINAL).unwrap();
    let id = "relay".to_owned();
    save_store_unlocked(
        &paths,
        &Store {
            providers: vec![Provider {
                id: id.clone(),
                name: "Relay".into(),
                base_url: "https://relay.invalid".into(),
                api_key: "synthetic-relay-key".into(),
                model: "gpt-6-luna".into(),
                models: vec![newapi::Model {
                    id: "gpt-6-luna".into(),
                    overrides: newapi::ModelOverrides {
                        reasoning: Some(
                            ["high", "xhigh"]
                                .map(|effort| newapi::ReasoningLevel {
                                    effort: effort.into(),
                                    description: effort.into(),
                                })
                                .into(),
                        ),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                enabled: true,
                protocol: ModelProtocol::Responses,
                ..Default::default()
            }],
            active_provider_id: Some(id.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    (dir, paths, id)
}

fn fixed(id: &str) -> Settings {
    Settings {
        model: Some(format!("{id}::gpt-6-luna")),
        reasoning_effort: Some("xhigh".into()),
    }
}

fn document(paths: &CodexPaths) -> DocumentMut {
    fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn defaults_are_saved_until_enabled_and_restore_the_first_user_baseline() {
    let (_dir, paths, id) = fixture();
    assert!(!load(&paths).unwrap().managed);
    save(&paths, Some(fixed(&id))).unwrap();
    assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), ORIGINAL);
    set_smart_routing(&paths, true).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), fixed(&id));
    assert!(load(&paths).unwrap().applied);
    let root_model = document(&paths)["model"].as_str().unwrap().to_owned();
    let mut updated = fixed(&id);
    updated.reasoning_effort = Some("high".into());
    save(&paths, Some(updated)).unwrap();
    assert_eq!(
        document(&paths)["model"].as_str(),
        Some(root_model.as_str())
    );
    save(&paths, Some(Settings::default())).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), Settings::default());
    let user = document(&paths);
    assert_eq!(user["agents"]["max_threads"].as_integer(), Some(8));
    assert_eq!(
        user["agents"]["reviewer"]["config_file"].as_str(),
        Some("review.toml")
    );
    assert!(fs::read_to_string(&paths.config_path)
        .unwrap()
        .contains("# user limit"));
    set_smart_routing(&paths, false).unwrap();
    assert_eq!(
        document(&paths).to_string().parse::<toml::Value>().unwrap(),
        ORIGINAL.parse::<toml::Value>().unwrap()
    );
    set_smart_routing(&paths, true).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), Settings::default());
    save(&paths, None).unwrap();
    assert_eq!(
        read(&document(&paths)).unwrap().model.as_deref(),
        Some("user-agent")
    );
    assert!(!load(&paths).unwrap().managed);
}

#[test]
fn changing_modes_cannot_send_namespaced_child_models_to_native() {
    let (_dir, paths, id) = fixture();
    save(&paths, Some(fixed(&id))).unwrap();
    set_smart_routing(&paths, true).unwrap();
    let before = fs::read(&paths.config_path).unwrap();
    assert_eq!(
        set_access_mode(&paths, CodexAccessMode::Native).unwrap_err(),
        "newapi.errors.subagentModel"
    );
    assert_eq!(fs::read(&paths.config_path).unwrap(), before);
    save(&paths, Some(Settings::default())).unwrap();
    set_access_mode(&paths, CodexAccessMode::Native).unwrap();
    let native = Settings {
        model: Some("gpt-6-luna".into()),
        reasoning_effort: Some("xhigh".into()),
    };
    save(&paths, Some(native.clone())).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), native);
    assert!(load(&paths)
        .unwrap()
        .models
        .iter()
        .all(|choice| !choice.id.contains("::")));
    assert!(native_config::intact(&paths, &load_store(&paths).unwrap()).unwrap());
    save(&paths, None).unwrap();
    assert_eq!(
        read(&document(&paths)).unwrap().model.as_deref(),
        Some("user-agent")
    );
    // Reacquire even if the user's current value already equals the desired one.
    let same = read(&document(&paths)).unwrap();
    save(&paths, Some(Settings::default())).unwrap();
    save(&paths, None).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), same);
    set_smart_routing(&paths, false).unwrap();
    assert_eq!(
        read(&document(&paths)).unwrap().model.as_deref(),
        Some("user-agent")
    );
}

#[test]
fn unmanaged_defaults_are_preserved_and_unavailable_native_selection_can_be_cleared() {
    let (_dir, paths, _) = fixture();
    set_smart_routing(&paths, true).unwrap();
    assert_eq!(
        read(&document(&paths)).unwrap().model.as_deref(),
        Some("user-agent")
    );
    set_smart_routing(&paths, false).unwrap();
    let mut store = load_store(&paths).unwrap();
    store.codex_access_mode = CodexAccessMode::Native;
    store.providers.clear();
    store.codex_subagents = Some(Settings {
        model: Some("removed".into()),
        ..Default::default()
    });
    save_store_unlocked(&paths, &store).unwrap();
    let view = load(&paths).unwrap();
    assert!(view.managed);
    assert!(view.models.is_empty());
    save(&paths, None).unwrap();
    assert!(!load(&paths).unwrap().managed);
    assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), ORIGINAL);
}

#[test]
fn aggregation_uses_public_models_and_rejects_unavailable_members() {
    use crate::core::aggregation::{AggregateMember, AggregateModel};
    let (_dir, paths, id) = fixture();
    let mut store = load_store(&paths).unwrap();
    store.aggregates = vec![AggregateModel {
        id: "pool".into(),
        public_model_id: "gpt-6-luna".into(),
        display_name: "Luna pool".into(),
        members: vec![AggregateMember {
            id: "member".into(),
            provider_id: id.clone(),
            actual_model_id: "gpt-6-luna".into(),
            ..Default::default()
        }],
        ..Default::default()
    }];
    store.router.aggregate_model_id = Some("pool".into());
    save_store_unlocked(&paths, &store).unwrap();
    let view = load(&paths).unwrap();
    assert_eq!(view.models.len(), 1);
    assert_eq!(view.models[0].id, "gpt-6-luna");
    assert_eq!(
        save(&paths, Some(fixed(&id))).unwrap_err(),
        "newapi.errors.subagentModel"
    );
    let settings = Settings {
        model: Some("gpt-6-luna".into()),
        reasoning_effort: Some("high".into()),
    };
    save(&paths, Some(settings.clone())).unwrap();
    set_smart_routing(&paths, true).unwrap();
    assert_eq!(read(&document(&paths)).unwrap(), settings);
    let before = fs::read(&paths.config_path).unwrap();
    assert_eq!(
        set_enabled(&paths, &id, false).unwrap_err(),
        "newapi.errors.subagentModel"
    );
    assert_eq!(fs::read(&paths.config_path).unwrap(), before);
    assert!(load_store(&paths).unwrap().providers[0].enabled);
}

#[test]
fn missing_models_and_unsupported_efforts_do_not_change_config_or_saved_settings() {
    let (_dir, paths, id) = fixture();
    let before = fs::read(&paths.config_path).unwrap();
    for (settings, error) in [
        (
            Settings {
                model: Some("removed::gpt-6-luna".into()),
                ..Default::default()
            },
            "newapi.errors.subagentModel",
        ),
        (
            Settings {
                reasoning_effort: Some("ultra".into()),
                ..Default::default()
            },
            "newapi.errors.subagentEffort",
        ),
        (
            Settings {
                model: Some(format!("{id}::gpt-6-luna")),
                reasoning_effort: Some("none".into()),
            },
            "newapi.errors.subagentEffort",
        ),
        (
            Settings {
                model: None,
                reasoning_effort: Some("none".into()),
            },
            "newapi.errors.subagentEffort",
        ),
    ] {
        assert_eq!(save(&paths, Some(settings)).unwrap_err(), error);
        assert_eq!(fs::read(&paths.config_path).unwrap(), before);
        assert!(load_store(&paths).unwrap().codex_subagents.is_none());
    }
    save(&paths, Some(fixed(&id))).unwrap();
    set_smart_routing(&paths, true).unwrap();
    let mut store = load_store(&paths).unwrap();
    store.providers.clear();
    assert_eq!(
        apply_routing_config(&paths, &store).unwrap_err(),
        "newapi.errors.subagentModel"
    );
}

#[test]
fn external_child_model_edits_survive_reset_and_connection_removal() {
    for reset in [true, false] {
        let (_dir, paths, id) = fixture();
        set_smart_routing(&paths, true).unwrap();
        save(&paths, Some(fixed(&id))).unwrap();
        let mut edited = document(&paths);
        edited["agents"][MODEL] = toml_edit::value("external-agent");
        edited["agents"]["max_threads"] = toml_edit::value(12);
        fs::write(&paths.config_path, edited.to_string()).unwrap();
        if reset {
            save(&paths, None).unwrap();
        } else {
            set_smart_routing(&paths, false).unwrap();
        }
        let actual = read(&document(&paths)).unwrap();
        assert_eq!(actual.model.as_deref(), Some("external-agent"));
        assert_eq!(actual.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(
            document(&paths)["agents"]["max_threads"].as_integer(),
            Some(12)
        );
    }
}

#[test]
fn provider_takeover_does_not_leave_owned_child_aliases_or_move_their_baseline() {
    for reenable in [false, true] {
        let (_dir, paths, id) = fixture();
        set_smart_routing(&paths, true).unwrap();
        save(&paths, Some(fixed(&id))).unwrap();
        let mut takeover = document(&paths);
        takeover["model"] = toml_edit::value("external-model");
        takeover["model_provider"] = toml_edit::value("external");
        fs::write(&paths.config_path, takeover.to_string()).unwrap();
        assert!(!load(&paths).unwrap().applied);
        if reenable {
            apply_routing_config(&paths, &load_store(&paths).unwrap()).unwrap();
        }
        set_smart_routing(&paths, false).unwrap();
        assert_eq!(
            read(&document(&paths)).unwrap().model.as_deref(),
            Some("user-agent")
        );
        assert_eq!(document(&paths)["model"].as_str(), Some("external-model"));
    }
}

#[test]
fn malformed_agents_and_inline_tables_are_handled_without_replacing_other_settings() {
    for text in ["agents = false", "agents = {default_subagent_model = 5}"] {
        let mut document = text.parse::<DocumentMut>().unwrap();
        let before = document.to_string();
        assert!(Ownership::apply(None, &mut document, &Settings::default()).is_err());
        assert_eq!(document.to_string(), before);
    }
    let mut document =
        r#"agents = {enabled = true, max_threads = 4, default_subagent_model = "prior"}"#
            .parse::<DocumentMut>()
            .unwrap();
    let owned = Ownership::apply(
        None,
        &mut document,
        &Settings {
            model: Some("new".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(document["agents"]["max_threads"].as_integer(), Some(4));
    owned.restore(&mut document).unwrap();
    assert_eq!(read(&document).unwrap().model.as_deref(), Some("prior"));
}

#[test]
fn failed_config_write_keeps_saved_defaults_and_can_retry_without_moving_baseline() {
    let (_dir, paths, id) = fixture();
    save(&paths, Some(fixed(&id))).unwrap();
    let blocker = paths.app_data_dir.join("config-backups");
    fs::write(&blocker, "synthetic backup failure").unwrap();
    assert!(set_smart_routing(&paths, true).is_err());
    assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), ORIGINAL);
    assert!(!load_store(&paths).unwrap().router.enabled);
    assert_eq!(
        load_store(&paths).unwrap().codex_subagents,
        Some(fixed(&id))
    );
    fs::remove_file(blocker).unwrap();
    set_smart_routing(&paths, true).unwrap();
    set_smart_routing(&paths, false).unwrap();
    assert_eq!(
        document(&paths).to_string().parse::<toml::Value>().unwrap(),
        ORIGINAL.parse::<toml::Value>().unwrap()
    );
}
