use super::*;
use crate::core::{
    aggregation,
    model_targets::{ClientModelSelection, ModelTargetRef},
    newapi,
};

/// A Desktop profile as Harness's own Models page leaves it: the user's own
/// gateway route, their default model, comments and a `!!js` expression.
const DESKTOP: &str = "# Your patch layer for this dsh profile\n# (id-targeted config overrides)\n- id: ui-chat\n  name: \"@deepseek-ai/dsh-client-ui-chat\"\n  config:\n    transcriptView: standard\n- id: agent-default-model\n  name: \"@deepseek-ai/dsh-agent-default-model\"\n  config:\n    provider: harbor # picked in Harness\n    model: workbuddy-ai/primary-model\n    reasoningEffort: high\n- id: llm-pi-ai\n  name: \"@deepseek-ai/dsh-llm-pi-ai\"\n  config:\n    providers:\n      harbor:\n        apiKeyEnv: HARBOR_API_KEY\n        api: openai-completions\n        baseURL: http://127.0.0.1:43127/gateway/v1\n        models:\n          - id: deepseek-flash\n            contextWindow: 1000000\n- id: tool\n  config:\n    cwd: !!js process.cwd()\n";

const CREDENTIALS: &str = "version: 1\nrefs:\n  # the user's own key\n  HARBOR_API_KEY: user-key\nrecords:\n  deepseek-account-platform/default:\n    kind: grant\n    payload:\n      token: keep-login\n";

/// A freshly initialized profile.
const FRESH: &str = "# Your patch layer for this dsh profile, applied after every bundle layer:\n# a top-level YAML array of loader patch entries.\n[]\n";

struct Fixture {
    _temp: tempfile::TempDir,
    manager: ConfigManager,
    store: Store,
    request: ConfigRequest,
}

impl Fixture {
    fn new(mode: ClientAccessMode, patch: Option<&str>, credentials: Option<&str>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let manager = ConfigManager::new(
            &temp.path().join("data"),
            ClientPaths::from_home(temp.path()),
        );
        for profile in ["desktop", "web"] {
            let dir = manager.paths.harness.join("profiles").join(profile);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("package.json"), "{}").unwrap();
        }
        // Not a profile: pnpm's shared store sits next to them.
        std::fs::create_dir_all(manager.paths.harness.join("profiles/node_modules")).unwrap();
        let fixture = Self {
            _temp: temp,
            manager,
            store: Store {
                providers: vec![Provider {
                    id: "test-relay".into(),
                    name: "Relay".into(),
                    base_url: "https://relay.example/v1".into(),
                    enabled: true,
                    model: "chat-model".into(),
                    api_key: "upstream-secret".into(),
                    protocol: ModelProtocol::ChatCompletions,
                    models: [
                        ModelProtocol::ChatCompletions,
                        ModelProtocol::Responses,
                        ModelProtocol::AnthropicMessages,
                    ]
                    .into_iter()
                    .zip(["chat-model", "responses-model", "messages-model"])
                    .map(|(protocol, id)| newapi::Model {
                        id: id.into(),
                        included: true,
                        protocol: Some(protocol),
                        overrides: newapi::ModelOverrides {
                            context_window: Some(272_000),
                            max_output_tokens: Some(8192),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .collect(),
                    ..Default::default()
                }],
                router: super::super::super::router::RouterSettings {
                    bearer_token: "local-secret".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            request: ConfigRequest {
                target_id: TargetId::Harness,
                provider_id: "test-relay".into(),
                model: "chat-model".into(),
                context_window: None,
                max_output_tokens: None,
                options: ClientOptions {
                    access_mode: mode,
                    provider_ids: vec!["test-relay".into()],
                    harness_profile: Some("desktop".into()),
                    ..Default::default()
                },
            },
        };
        if let Some(patch) = patch {
            std::fs::write(fixture.patch(), patch).unwrap();
        }
        if let Some(credentials) = credentials {
            std::fs::write(fixture.credentials(), credentials).unwrap();
        }
        fixture
    }

    fn patch(&self) -> PathBuf {
        self.manager
            .paths
            .harness
            .join("profiles/desktop/cordis.patch.yml")
    }

    fn credentials(&self) -> PathBuf {
        self.manager.paths.harness.join(".credentials.yaml")
    }

    fn apply(&self) -> ConfigApplied {
        let preview = self.manager.preview(&self.store, &self.request).unwrap();
        self.manager
            .apply(&self.store, &self.request, &preview.revision)
            .unwrap()
    }

    fn patch_text(&self) -> String {
        std::fs::read_to_string(self.patch()).unwrap()
    }

    fn patch_doc(&self) -> Value {
        yaml::read(Some(&self.patch_text())).unwrap()
    }

    fn credentials_doc(&self) -> Value {
        yaml::read(Some(&std::fs::read_to_string(self.credentials()).unwrap())).unwrap()
    }

    fn status(&self) -> ClientStatus {
        self.manager
            .list()
            .into_iter()
            .find(|status| status.target_id == TargetId::Harness)
            .unwrap()
    }
}

fn route<'a>(document: &'a Value, key: &str) -> &'a Value {
    yaml::patch_value(document, PROVIDERS_ROW, &["providers".into(), key.into()])
        .unwrap_or(&Value::Null)
}

fn default_model(document: &Value) -> Value {
    yaml::patch_value(document, DEFAULT_ROW, &[])
        .cloned()
        .unwrap_or(Value::Null)
}

// ---- the YAML layer -------------------------------------------------------------

#[test]
fn routes_join_the_users_row_and_leave_comments_tags_and_other_providers_alone() {
    let path = ["providers".to_owned(), "agentharbor-x".to_owned()];
    let value = json!({"api": "openai-completions", "baseURL": "http://127.0.0.1:1/v1", "models": [{"id": "m", "input": ["text", "image"]}]});
    let edited = yaml::set_patch(Some(DESKTOP), PROVIDERS_ROW, &path, Some(&value))
        .unwrap()
        .unwrap();
    for kept in [
        "# (id-targeted config overrides)",
        "provider: harbor # picked in Harness",
        "cwd: !!js process.cwd()",
        "      harbor:\n",
    ] {
        assert!(edited.contains(kept), "{kept:?} lost:\n{edited}");
    }
    let document = yaml::read(Some(&edited)).unwrap();
    assert_eq!(route(&document, "agentharbor-x"), &value);
    assert_eq!(route(&document, "harbor")["apiKeyEnv"], "HARBOR_API_KEY");
    // Block layout in a block file.
    assert!(
        edited.contains("      agentharbor-x:\n        api: openai-completions\n"),
        "{edited}"
    );
    let removed = yaml::set_patch(Some(&edited), PROVIDERS_ROW, &path, None)
        .unwrap()
        .unwrap();
    assert_eq!(removed, DESKTOP);
}

#[test]
fn a_fresh_profile_gives_up_its_empty_list_and_gets_it_back() {
    let path = ["providers".to_owned(), "agentharbor-x".to_owned()];
    let value = json!({"api": "openai-responses", "models": [{"id": "m"}]});
    let edited = yaml::set_patch(Some(FRESH), PROVIDERS_ROW, &path, Some(&value))
        .unwrap()
        .unwrap();
    assert!(edited.starts_with("# Your patch layer"));
    assert!(!edited.contains("[]"), "{edited}");
    assert!(
        edited.contains("- id: llm-pi-ai\n  config:\n    providers:\n"),
        "{edited}"
    );
    let emptied = yaml::remove_row(Some(&edited), PROVIDERS_ROW)
        .unwrap()
        .unwrap();
    assert_eq!(yaml::read(Some(&emptied)).unwrap(), json!([]), "{emptied}");
    assert!(
        emptied.trim_end().ends_with("[]"),
        "Harness refuses an empty patch:\n{emptied}"
    );
    // An absent file is an empty list too.
    let created = yaml::set_patch(None, PROVIDERS_ROW, &path, Some(&value))
        .unwrap()
        .unwrap();
    assert_eq!(
        route(&yaml::read(Some(&created)).unwrap(), "agentharbor-x"),
        &value
    );
}

#[test]
fn flow_lists_and_flow_configs_take_flow_values() {
    let path = ["providers".to_owned(), "agentharbor-x".to_owned()];
    let value = json!({"api": "anthropic-messages", "models": [{"id": "m", "name": "M: one"}]});
    for source in [
        "# plugin manager\n[ { id: dsh-quota, name: dsh-deepseek-quota, disabled: true }, { id: gpt-image, disabled: true } ]\n",
        "- id: llm-pi-ai\n  config: { providers: { harbor: { api: openai-completions } } }\n",
        "- id: llm-pi-ai\n  name: x\n- id: other\n",
        "- id: llm-pi-ai\n  config:\n    retry: 1\n",
    ] {
        let edited = yaml::set_patch(Some(source), PROVIDERS_ROW, &path, Some(&value)).unwrap().unwrap();
        let document = yaml::read(Some(&edited)).unwrap();
        assert_eq!(route(&document, "agentharbor-x"), &value, "{edited}");
        let before = yaml::read(Some(source)).unwrap();
        for (index, row) in before.as_array().unwrap().iter().enumerate() {
            if row["id"] != PROVIDERS_ROW {
                assert_eq!(&document[index], row);
            }
        }
    }
}

#[test]
fn ambiguous_or_foreign_shapes_are_refused_without_guessing() {
    let path = ["providers".to_owned(), "k".to_owned()];
    let value = json!({"api": "openai-completions"});
    assert_eq!(
        yaml::set_patch(
            Some("- id: llm-pi-ai\n- id: llm-pi-ai\n"),
            PROVIDERS_ROW,
            &path,
            Some(&value)
        )
        .unwrap_err(),
        "clients.errors.harnessDuplicateRow"
    );
    assert_eq!(
        yaml::set_patch(Some("llm-pi-ai: {}\n"), PROVIDERS_ROW, &path, Some(&value)).unwrap_err(),
        "clients.errors.harnessPatchShape"
    );
    assert_eq!(
        yaml::set_patch(
            Some("- id: llm-pi-ai\n  config: !!js load()\n"),
            PROVIDERS_ROW,
            &path,
            Some(&value)
        )
        .unwrap_err(),
        "clients.errors.harnessPatchShape"
    );
    assert_eq!(
        yaml::set_credential(Some("- a\n"), "K", Some("v")).unwrap_err(),
        "clients.errors.harnessCredentialsShape"
    );
}

#[test]
fn credential_refs_are_set_next_to_the_users_and_withdrawn_exactly() {
    let edited = yaml::set_credential(Some(CREDENTIALS), ROUTER_CREDENTIAL, Some("tok: $x #y"))
        .unwrap()
        .unwrap();
    assert!(edited.contains("  # the user's own key\n  HARBOR_API_KEY: user-key\n"));
    let document = yaml::read(Some(&edited)).unwrap();
    assert_eq!(
        yaml::credential(&document, ROUTER_CREDENTIAL),
        Some(&json!("tok: $x #y"))
    );
    assert_eq!(
        document["records"]["deepseek-account-platform/default"]["payload"]["token"],
        "keep-login"
    );
    assert_eq!(
        yaml::set_credential(Some(&edited), ROUTER_CREDENTIAL, None)
            .unwrap()
            .unwrap(),
        CREDENTIALS
    );
    // A new store, a store without refs and an inline one.
    for source in [
        None,
        Some("version: 1\nrecords: {}\n"),
        Some("version: 1\nrefs: {}\n"),
        Some("{version: 1}\n"),
    ] {
        let edited = yaml::set_credential(source, ROUTER_CREDENTIAL, Some("k"))
            .unwrap()
            .unwrap();
        assert_eq!(
            yaml::credential(&yaml::read(Some(&edited)).unwrap(), ROUTER_CREDENTIAL),
            Some(&json!("k")),
            "{edited}"
        );
    }
}

#[test]
fn block_text_reads_back_as_the_same_values() {
    let value = json!({
        "yes": "no", "a: b": "@at", "x": "#hash", "n": "123", "t": "true", "e": "", "u": "ünïcode\n\"quoted\"",
        "list": ["text", "image"], "empty": {}, "none": [], "nested": [{"id": "m", "levels": {"off": "none", "high": "high"}}, {"id": "n"}],
        "flag": false, "number": 1.5,
    });
    let text = yaml::block(&value, 0);
    assert_eq!(yaml::read(Some(&text)).unwrap(), value, "{text}");
}

#[test]
fn profiles_are_listed_desktop_and_web_first_without_pnpm_folders() {
    let fixture = Fixture::new(ClientAccessMode::Router, None, None);
    let dir = fixture.manager.paths.harness.join("profiles/dsh-tui");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cordis.patch.yml"), "[]\n").unwrap();
    std::fs::create_dir_all(fixture.manager.paths.harness.join("profiles/empty")).unwrap();
    assert_eq!(
        profiles(&fixture.manager.paths.harness),
        ["desktop", "web", "dsh-tui"]
    );
    let home = Path::new("/users/me");
    assert_eq!(super::home(home, None), home.join(".dsh"));
    assert_eq!(super::home(home, Some("  ".into())), home.join(".dsh"));
    assert_eq!(
        super::home(home, Some("~/harness".into())),
        home.join("harness")
    );
    assert_eq!(
        super::home(home, Some("/opt/dsh".into())),
        PathBuf::from("/opt/dsh")
    );
}

// ---- the session ----------------------------------------------------------------

#[test]
fn router_routes_per_protocol_reference_the_local_token_and_restore_byte_for_byte() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    let preview = fixture
        .manager
        .preview(&fixture.store, &fixture.request)
        .unwrap();
    assert!(
        !preview.content.contains("local-secret") && !preview.content.contains("upstream-secret"),
        "{}",
        preview.content
    );
    assert_eq!(
        preview.removed_fields,
        ["agent-default-model.reasoningEffort"]
    );
    fixture
        .manager
        .apply(&fixture.store, &fixture.request, &preview.revision)
        .unwrap();

    let document = fixture.patch_doc();
    for (api, model, suffix) in [
        ("openai-completions", "chat-model", "/v1"),
        ("openai-responses", "responses-model", "/v1"),
        ("anthropic-messages", "messages-model", ""),
    ] {
        let node = route(&document, &format!("agentharbor-test-relay-{api}"));
        assert_eq!(node["api"], api);
        assert_eq!(node["apiKeyEnv"], ROUTER_CREDENTIAL);
        assert_eq!(
            node["baseURL"],
            format!("http://127.0.0.1:43127/clients/harness/p/test-relay{suffix}")
        );
        assert_eq!(
            node["displayName"],
            format!(
                "Relay · {}",
                match api {
                    "openai-completions" => "Chat",
                    "openai-responses" => "Responses",
                    _ => "Messages",
                }
            )
        );
        assert_eq!(node["models"][0]["id"], model);
        assert_eq!(node["models"][0]["contextWindow"], 272000);
        assert_eq!(node["models"][0]["maxTokens"], 8192);
    }
    let chat = route(&document, "agentharbor-test-relay-openai-completions");
    assert_eq!(chat["compat"]["maxTokensField"], "max_tokens");
    assert_eq!(
        chat["models"][0]["compat"]["supportsReasoningEffort"],
        false
    );
    assert_eq!(route(&document, "harbor")["apiKeyEnv"], "HARBOR_API_KEY");
    assert_eq!(
        default_model(&document),
        json!({"provider": "agentharbor-test-relay-openai-completions", "model": "chat-model"})
    );
    let credentials = fixture.credentials_doc();
    assert_eq!(credentials["refs"][ROUTER_CREDENTIAL], "local-secret");
    assert_eq!(credentials["refs"]["HARBOR_API_KEY"], "user-key");
    assert!(!fixture.patch_text().contains("local-secret"));

    let status = fixture.status();
    assert!(status.configured && !status.conflict && status.error.is_none());
    assert_eq!(
        status
            .harness
            .unwrap()
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        ["desktop", "web"]
    );
    assert!(has_router_configs(&fixture.manager.app_data_dir).unwrap());

    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), DESKTOP);
    assert_eq!(
        std::fs::read_to_string(fixture.credentials()).unwrap(),
        CREDENTIALS
    );
    assert!(!fixture.status().configured);
    assert!(!has_router_configs(&fixture.manager.app_data_dir).unwrap());
}

#[test]
fn router_routes_withdrawn_on_exit_resume_into_the_same_profile() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.apply();
    let installed = fixture.patch_text();
    assert!(fixture.manager.restore_router_configs_on_exit().is_empty());
    assert_eq!(fixture.patch_text(), DESKTOP);
    let waiting = fixture.status();
    assert!(!waiting.configured && waiting.resume_pending);
    assert_eq!(waiting.request.as_ref().and_then(|request| request.options.harness_profile.as_deref()), Some("desktop"));
    assert!(fixture.manager.resume_parked(&fixture.store).is_empty());
    assert!(fixture.status().configured);
    assert_eq!(fixture.patch_text(), installed);
    assert_eq!(fixture.credentials_doc()["refs"][ROUTER_CREDENTIAL], "local-secret");
}

#[test]
fn a_fresh_profile_and_store_are_created_and_withdrawn_completely() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(FRESH), None);
    fixture.apply();
    assert!(fixture.credentials().exists());
    let document = fixture.patch_doc();
    assert!(route(&document, "agentharbor-test-relay-openai-responses").is_object());
    assert_eq!(default_model(&document)["model"], "chat-model");
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), FRESH);
    assert!(!fixture.credentials().exists());
}

/// Harness 0.2.1's headless bundle has no credential store: keys go to
/// `$DSH_HOME/.env` next to the user's own lines, and come back out exactly.
#[test]
fn a_headless_profile_reads_keys_from_the_harness_home_env_file() {
    let mut fixture = Fixture::new(ClientAccessMode::Router, None, Some(CREDENTIALS));
    let home = fixture.manager.paths.harness.clone();
    let headless = home.join("profiles/headless");
    std::fs::create_dir_all(&headless).unwrap();
    std::fs::write(
        headless.join("package.json"),
        r#"{"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-headless"]}}}"#,
    )
    .unwrap();
    std::fs::write(headless.join(PATCH_FILE), FRESH).unwrap();
    let env = "# mine
DEEPSEEK_API_KEY=sk-user
";
    std::fs::write(home.join(DOTENV_FILE), env).unwrap();
    let info = info(&home);
    let listed = |name: &str| info.profiles.iter().find(|profile| profile.name == name).unwrap().dotenv;
    assert!(listed("headless"));
    assert!(!listed("desktop"));

    fixture.request.options.harness_profile = Some("headless".into());
    let preview = fixture.manager.preview(&fixture.store, &fixture.request).unwrap();
    assert!(preview.warnings.contains(&"clients.warnings.harnessDotenv".to_owned()));
    assert!(preview.content.contains(&format!("\"{ROUTER_CREDENTIAL}\": \"[managed locally]\"")), "{}", preview.content);
    assert!(!preview.content.contains("local-secret"));
    fixture.manager.apply(&fixture.store, &fixture.request, &preview.revision).unwrap();
    let written = std::fs::read_to_string(home.join(DOTENV_FILE)).unwrap();
    assert_eq!(written, format!("{env}{ROUTER_CREDENTIAL}=local-secret
"));
    assert_eq!(std::fs::read_to_string(fixture.credentials()).unwrap(), CREDENTIALS);
    let patch = yaml::read(Some(&std::fs::read_to_string(headless.join(PATCH_FILE)).unwrap())).unwrap();
    assert_eq!(route(&patch, "agentharbor-test-relay-openai-responses")["apiKeyEnv"], ROUTER_CREDENTIAL);
    assert!(!fixture.status().conflict);

    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(std::fs::read_to_string(home.join(DOTENV_FILE)).unwrap(), env);
    assert_eq!(std::fs::read_to_string(headless.join(PATCH_FILE)).unwrap(), FRESH);

    // The README's rows turn the store back on, and the keys follow it.
    std::fs::write(headless.join(PATCH_FILE), "- id: credentials
  disabled: false
").unwrap();
    assert!(!reads_dotenv(&home, "headless"));
}

#[test]
fn direct_routes_keep_each_relays_key_in_the_credential_store() {
    let mut fixture = Fixture::new(ClientAccessMode::Native, Some(DESKTOP), Some(CREDENTIALS));
    fixture.request.options.harness_default_model = Some(false);
    fixture.apply();
    let document = fixture.patch_doc();
    let reference = routes::relay_credential("test-relay");
    assert!(reference.starts_with("AGENTHARBOR_RELAY_TEST_RELAY_"));
    let chat = route(&document, "agentharbor-test-relay-openai-completions");
    assert_eq!(chat["baseURL"], "https://relay.example/v1");
    assert_eq!(chat["apiKeyEnv"], reference.as_str());
    assert_eq!(
        route(&document, "agentharbor-test-relay-anthropic-messages")["baseURL"],
        "https://relay.example"
    );
    assert_eq!(
        fixture.credentials_doc()["refs"][reference.as_str()],
        "upstream-secret"
    );
    assert!(fixture.credentials_doc()["refs"]
        .get(ROUTER_CREDENTIAL)
        .is_none());
    // Without the default model switch the user's choice stays untouched.
    assert_eq!(default_model(&document)["provider"], "harbor");
    assert!(!fixture.patch_text().contains("upstream-secret"));
    assert!(!has_router_configs(&fixture.manager.app_data_dir).unwrap());
    // A changed card shows as stale.
    fixture.store.providers[0].api_key = "rotated".into();
    assert!(fixture
        .manager
        .list_with(Some(&fixture.store))
        .into_iter()
        .any(|status| status.target_id == TargetId::Harness && status.stale));
    fixture
        .manager
        .reapply(&fixture.store, TargetId::Harness, false)
        .unwrap();
    assert_eq!(
        fixture.credentials_doc()["refs"][reference.as_str()],
        "rotated"
    );
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), DESKTOP);
    assert_eq!(
        std::fs::read_to_string(fixture.credentials()).unwrap(),
        CREDENTIALS
    );
}

#[test]
fn a_bearer_only_messages_relay_is_reached_through_the_router() {
    let mut fixture = Fixture::new(ClientAccessMode::Native, Some(DESKTOP), None);
    fixture.store.providers[0].base_url = "https://openrouter.ai/api/v1".into();
    let error = fixture
        .manager
        .preview(&fixture.store, &fixture.request)
        .err()
        .unwrap();
    assert_eq!(error, "clients.errors.harnessAnthropicBearer");
    fixture.request.options.access_mode = ClientAccessMode::Router;
    fixture.apply();
}

#[test]
fn the_default_model_starts_on_its_default_effort_or_the_chosen_level() {
    let mut fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.store.providers[0].models[0].overrides.reasoning = Some(
        ["none", "low", "high", "max"]
            .into_iter()
            .map(|effort| crate::core::model_metadata::ReasoningLevel {
                effort: effort.into(),
                description: String::new(),
            })
            .collect(),
    );
    let offered = fixture
        .manager
        .harness_reasoning(&fixture.store, &fixture.request)
        .unwrap();
    assert_eq!(offered.levels, ["off", "low", "high", "max"]);
    // No declared default: the resolver's policy picks high over low and max.
    assert_eq!(offered.model_default.as_deref(), Some("high"));
    let preview = fixture
        .manager
        .preview(&fixture.store, &fixture.request)
        .unwrap();
    assert!(preview.removed_fields.is_empty(), "{:?}", preview.removed_fields);
    fixture.apply();
    assert_eq!(default_model(&fixture.patch_doc())["reasoningEffort"], "high");

    fixture.request.options.harness_reasoning_effort = Some("max".into());
    fixture.apply();
    assert_eq!(default_model(&fixture.patch_doc())["reasoningEffort"], "max");

    fixture.request.options.harness_reasoning_effort = Some("medium".into());
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .as_deref(),
        Some("clients.errors.harnessReasoningEffort")
    );

    // A model with no levels leaves the provider's default.
    fixture.request.options.harness_reasoning_effort = None;
    fixture.store.providers[0].models[0].overrides.reasoning = None;
    fixture.apply();
    assert!(default_model(&fixture.patch_doc())
        .get("reasoningEffort")
        .is_none());
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), DESKTOP);
}

#[test]
fn a_default_model_picked_in_harness_is_activity_and_survives_restore() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.apply();
    // The user picks DeepSeek's own model in Harness and adds a provider of
    // their own next to ours.
    let mut text = fixture.patch_text();
    text = yaml::set_patch(
        Some(&text),
        DEFAULT_ROW,
        &["provider".into()],
        Some(&json!("deepseek-official")),
    )
    .unwrap()
    .unwrap();
    text = yaml::set_patch(
        Some(&text),
        DEFAULT_ROW,
        &["model".into()],
        Some(&json!("deepseek-flash")),
    )
    .unwrap()
    .unwrap();
    text = yaml::set_patch(
        Some(&text),
        PROVIDERS_ROW,
        &["providers".into(), "mine".into()],
        Some(&json!({"api": "openai-completions"})),
    )
    .unwrap()
    .unwrap();
    std::fs::write(fixture.patch(), &text).unwrap();
    let status = fixture.status();
    assert!(!status.conflict, "{:?}", status.conflict_fields);
    fixture.manager.restore(TargetId::Harness).unwrap();
    let document = fixture.patch_doc();
    assert_eq!(
        default_model(&document),
        json!({"provider": "deepseek-official", "model": "deepseek-flash"})
    );
    assert_eq!(
        route(&document, "mine"),
        &json!({"api": "openai-completions"})
    );
    assert!(route(&document, "agentharbor-test-relay-openai-completions").is_null());
    assert!(fixture
        .patch_text()
        .contains("# (id-targeted config overrides)"));
    assert!(fixture.patch_text().contains("cwd: !!js process.cwd()"));
    assert!(fixture.credentials_doc()["refs"]
        .get(ROUTER_CREDENTIAL)
        .is_none());
}

#[test]
fn an_edited_route_is_a_conflict_and_the_exit_still_withdraws_router_routes() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.apply();
    let key = "agentharbor-test-relay-openai-responses";
    let mut edited = route(&fixture.patch_doc(), key).clone();
    edited["models"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "extra"}));
    let text = yaml::set_patch(
        Some(&fixture.patch_text()),
        PROVIDERS_ROW,
        &["providers".into(), key.into()],
        Some(&edited),
    )
    .unwrap()
    .unwrap();
    std::fs::write(fixture.patch(), &text).unwrap();

    let status = fixture.status();
    assert!(status.conflict);
    assert_eq!(
        status.conflict_fields,
        [format!("llm-pi-ai.providers.{key}")]
    );
    assert_eq!(
        fixture.manager.restore(TargetId::Harness).err().unwrap(),
        "clients.errors.conflict"
    );
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.conflict"
    );
    assert!(fixture
        .manager
        .preview_with_conflicts(&fixture.store, &fixture.request, true)
        .is_ok());

    // Leaving: a route aimed at this process's router cannot outlive it.
    assert!(fixture.manager.restore_router_configs_on_exit().is_empty());
    let document = fixture.patch_doc();
    assert!(route(&document, key).is_null());
    assert_eq!(default_model(&document)["provider"], "harbor");
    assert_eq!(default_model(&document)["reasoningEffort"], "high");
    assert!(fixture.credentials_doc()["refs"]
        .get(ROUTER_CREDENTIAL)
        .is_none());
}

#[test]
fn a_confirmed_restore_replaces_edits_and_rebaseline_adopts_them() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.apply();
    let text = yaml::set_credential(
        Some(&std::fs::read_to_string(fixture.credentials()).unwrap()),
        ROUTER_CREDENTIAL,
        Some("theirs"),
    )
    .unwrap()
    .unwrap();
    std::fs::write(fixture.credentials(), &text).unwrap();
    assert!(fixture.status().conflict);
    assert_eq!(
        fixture
            .manager
            .preview_with_conflicts(&fixture.store, &fixture.request, true)
            .unwrap()
            .conflict_fields,
        fixture.status().conflict_fields
    );
    assert!(!fixture.status().conflict_fields.is_empty());
    fixture
        .manager
        .reapply(&fixture.store, TargetId::Harness, true)
        .unwrap();
    assert!(!fixture.status().conflict);
    assert_eq!(
        fixture.credentials_doc()["refs"][ROUTER_CREDENTIAL],
        "local-secret"
    );
    // The adopted value is the new rollback point.
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(
        fixture.credentials_doc()["refs"][ROUTER_CREDENTIAL],
        "theirs"
    );
    assert_eq!(fixture.patch_text(), DESKTOP);

    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture.apply();
    std::fs::write(
        fixture.credentials(),
        &text.replace("local-secret", "theirs"),
    )
    .unwrap();
    fixture
        .manager
        .restore_confirmed(TargetId::Harness, true)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.credentials()).unwrap(),
        CREDENTIALS
    );
}

#[test]
fn reapply_follows_the_store_and_a_users_route_name_is_never_taken() {
    let mut fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(CREDENTIALS));
    fixture
        .request
        .options
        .provider_model_ids
        .insert("test-relay".into(), vec!["chat-model".into()]);
    fixture.apply();
    assert!(route(
        &fixture.patch_doc(),
        "agentharbor-test-relay-openai-responses"
    )
    .is_null());
    fixture.request.options.provider_model_ids.clear();
    fixture.apply();
    assert!(route(
        &fixture.patch_doc(),
        "agentharbor-test-relay-openai-responses"
    )
    .is_object());
    fixture.store.router.bearer_token = "new-token".into();
    fixture
        .manager
        .reapply(&fixture.store, TargetId::Harness, false)
        .unwrap();
    assert_eq!(
        fixture.credentials_doc()["refs"][ROUTER_CREDENTIAL],
        "new-token"
    );
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), DESKTOP);

    let taken = DESKTOP.replace(
        "      harbor:\n",
        "      agentharbor-test-relay-openai-completions:\n",
    );
    let fixture = Fixture::new(ClientAccessMode::Router, Some(&taken), None);
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.harnessProviderConflict"
    );
    assert_eq!(fixture.patch_text(), taken);
}

#[test]
fn a_users_own_key_under_our_name_is_never_overwritten() {
    let taken = format!("version: 1\nrefs:\n  {ROUTER_CREDENTIAL}: someone-elses-token\n");
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), Some(&taken));
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.harnessCredentialConflict"
    );
    assert_eq!(
        fixture.credentials_doc()["refs"][ROUTER_CREDENTIAL],
        "someone-elses-token"
    );
}

#[test]
fn profiles_are_checked_and_layers_above_the_profile_are_reported() {
    let mut fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), None);
    fixture.request.options.harness_profile = Some("../desktop".into());
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.harnessProfile"
    );
    fixture.request.options.harness_profile = Some("missing".into());
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.harnessProfile"
    );
    fixture.request.options.harness_profile = Some("desktop".into());
    std::fs::write(
        fixture.manager.paths.harness.join("cordis.patch.yml"),
        "- id: llm-pi-ai\n  config:\n    providers: {}\n- id: hmr\n  disabled: true\n",
    )
    .unwrap();
    let preview = fixture
        .manager
        .preview(&fixture.store, &fixture.request)
        .unwrap();
    assert!(preview
        .warnings
        .contains(&"clients.warnings.harnessHomeOverride".to_owned()));
    assert!(preview
        .warnings
        .contains(&"clients.warnings.harnessRestart".to_owned()));
    fixture
        .manager
        .apply(&fixture.store, &fixture.request, &preview.revision)
        .unwrap();
    fixture.request.options.harness_profile = Some("web".into());
    assert_eq!(
        fixture
            .manager
            .preview(&fixture.store, &fixture.request)
            .err()
            .unwrap(),
        "clients.errors.harnessProfileChanged"
    );
}

#[test]
fn a_file_changed_between_preview_and_apply_is_not_overwritten() {
    let fixture = Fixture::new(ClientAccessMode::Router, Some(DESKTOP), None);
    let preview = fixture
        .manager
        .preview(&fixture.store, &fixture.request)
        .unwrap();
    std::fs::write(fixture.patch(), FRESH).unwrap();
    assert_eq!(
        fixture
            .manager
            .apply(&fixture.store, &fixture.request, &preview.revision)
            .err()
            .unwrap(),
        "clients.errors.changed"
    );
    assert_eq!(fixture.patch_text(), FRESH);
    assert!(!fixture.credentials().exists());
}

#[test]
fn aggregates_share_one_route_with_their_public_ids() {
    let mut fixture = Fixture::new(ClientAccessMode::Router, Some(FRESH), None);
    fixture.store.aggregates.push(aggregation::AggregateModel {
        id: "pool".into(),
        public_model_id: "public-deepseek".into(),
        display_name: "DeepSeek".into(),
        members: vec![aggregation::AggregateMember {
            id: "member".into(),
            provider_id: "test-relay".into(),
            actual_model_id: "chat-model".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    fixture.request.options.aggregate_ingress_protocol = Some(ModelProtocol::AnthropicMessages);
    fixture.request.options.model_selection = Some(ClientModelSelection {
        default_target: Some(ModelTargetRef::aggregate("pool")),
        available_targets: vec![
            ModelTargetRef::aggregate("pool"),
            ModelTargetRef::provider("test-relay", "responses-model"),
        ],
    });
    fixture.apply();
    let document = fixture.patch_doc();
    let node = route(&document, "agentharbor-aggregation-anthropic-messages");
    assert_eq!(
        node["baseURL"],
        "http://127.0.0.1:43127/clients/harness/aggregate"
    );
    assert_eq!(node["displayName"], AGGREGATION_NAME);
    assert_eq!(node["models"][0]["id"], "public-deepseek");
    assert_eq!(
        route(&document, "agentharbor-test-relay-openai-responses")["models"][0]["id"],
        "responses-model"
    );
    assert_eq!(
        default_model(&document),
        json!({"provider": "agentharbor-aggregation-anthropic-messages", "model": "public-deepseek"})
    );
    fixture.manager.restore(TargetId::Harness).unwrap();
    assert_eq!(fixture.patch_text(), FRESH);
}

#[test]
fn route_keys_and_credential_names_stay_in_harness_grammar() {
    assert_eq!(
        routes::route_key("test-relay", ModelProtocol::Responses),
        "agentharbor-test-relay-openai-responses"
    );
    let odd = routes::route_key("My_Relay.1", ModelProtocol::ChatCompletions);
    assert!(
        odd.starts_with("agentharbor-my-relay-1-") && odd.ends_with("-openai-completions"),
        "{odd}"
    );
    assert_ne!(
        odd,
        routes::route_key("my-relay-1", ModelProtocol::ChatCompletions)
    );
    assert!(odd
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    let name = routes::relay_credential("account-chatgpt:me@x");
    assert!(
        name.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
        "{name}"
    );
}
