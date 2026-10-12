use super::*;

fn proxied(manager: ConfigManager, mode: ApiProxyMode, url: Option<&str>) -> ConfigManager {
    manager.with_network(Network {
        proxy: ApiProxyConfigPayload {
            mode,
            url: url.map(str::to_owned),
        },
        ..Default::default()
    })
}

fn direct() -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let (temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    let manager = proxied(manager, ApiProxyMode::Manual, Some("http://127.0.0.1:7890"));
    (temp, manager, store, request)
}

#[test]
fn a_direct_configuration_carries_the_manual_http_proxy_until_restored() {
    let (_temp, manager, store, request) = direct();
    write(&manager.paths.claude, &json!({"env": {"CUSTOM": "kept"}}));
    apply(&manager, &store, &request);
    let env = &read(&manager.paths.claude)["env"];
    assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:7890");
    assert_eq!(env["NO_PROXY"], "localhost,127.0.0.1,::1");
    manager.restore(request.target_id).unwrap();
    let env = &read(&manager.paths.claude)["env"];
    assert!(env.get("HTTPS_PROXY").is_none() && env.get("NO_PROXY").is_none());
    assert_eq!(env["CUSTOM"], "kept");
}

#[test]
fn the_users_own_proxy_variables_stay_theirs() {
    let (_temp, manager, store, request) = direct();
    write(&manager.paths.claude, &json!({"env": {"https_proxy": "http://corp:3128"}}));
    apply(&manager, &store, &request);
    let env = &read(&manager.paths.claude)["env"];
    assert_eq!(env["https_proxy"], "http://corp:3128");
    assert!(env.get("HTTPS_PROXY").is_none() && env.get("NO_PROXY").is_none());

    let (_temp, manager, store, request) = direct();
    write(&manager.paths.claude, &json!({"env": {"no_proxy": "corp.internal"}}));
    apply(&manager, &store, &request);
    let env = &read(&manager.paths.claude)["env"];
    assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:7890");
    assert_eq!(env["no_proxy"], "corp.internal");
    assert!(env.get("NO_PROXY").is_none());
}

#[test]
fn nothing_is_written_for_the_router_a_direct_card_or_an_active_terminal_proxy() {
    let (_temp, manager, store, mut request) = direct();
    request.options.access_mode = ClientAccessMode::Router;
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["env"].get("HTTPS_PROXY").is_none());

    let (_temp, manager, mut store, request) = direct();
    store.providers[0].network_mode = "direct".into();
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["env"].get("HTTPS_PROXY").is_none());

    let (_temp, manager, store, request) = direct();
    let manager = manager.with_network(Network {
        proxy: ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some("http://127.0.0.1:7890".into()),
        },
        terminal: Some("http://127.0.0.1:7890/".into()),
        system: false,
    });
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["env"].get("HTTPS_PROXY").is_none());
}

#[test]
fn socks_and_system_proxies_warn_instead_of_writing() {
    let (_temp, manager, store, request) = direct();
    let manager = proxied(manager, ApiProxyMode::Manual, Some("socks5://127.0.0.1:7891"));
    let preview = manager.preview(&store, &request).unwrap();
    assert!(preview.warnings.contains(&"clients.warnings.proxySocks".to_owned()));
    assert!(!preview.content.contains("HTTPS_PROXY"));

    let (_temp, manager, store, request) = direct();
    let manager = manager.with_network(Network {
        system: true,
        ..Default::default()
    });
    let preview = manager.preview(&store, &request).unwrap();
    assert!(preview.warnings.contains(&"clients.warnings.systemProxy".to_owned()));
    assert!(!preview.content.contains("HTTPS_PROXY"));

    let (_temp, manager, store, request) = direct();
    let manager = proxied(manager, ApiProxyMode::Direct, None);
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview.warnings.iter().any(|warning| warning.contains("roxy")));
}

#[test]
fn a_proxy_address_with_credentials_is_redacted_in_the_preview() {
    let (_temp, manager, store, request) = direct();
    let manager = proxied(manager, ApiProxyMode::Manual, Some("http://user:pass@proxy.example:8080"));
    let preview = manager.preview(&store, &request).unwrap();
    assert!(preview.content.contains("HTTPS_PROXY"));
    assert!(!preview.content.contains("pass@"));
    assert!(preview.content.contains("localhost,127.0.0.1,::1"), "a bypass list is no secret");
}

#[test]
fn a_changed_proxy_marks_a_direct_configuration_stale() {
    let (temp, manager, store, request) = direct();
    apply(&manager, &store, &request);
    assert!(!manager.list_with(Some(&store))[0].stale);
    let reopen = |network: Network| {
        ConfigManager::new(&temp.path().join("data"), ClientPaths::from_home(temp.path())).with_network(network)
    };
    let manual = |url: &str| Network {
        proxy: ApiProxyConfigPayload {
            mode: ApiProxyMode::Manual,
            url: Some(url.into()),
        },
        ..Default::default()
    };
    assert!(reopen(manual("http://127.0.0.1:7899")).list_with(Some(&store))[0].stale, "the file holds the old port");
    assert!(
        reopen(Network {
            terminal: Some("http://127.0.0.1:7890/".into()),
            ..manual("http://127.0.0.1:7890")
        })
        .list_with(Some(&store))[0]
            .stale,
        "the terminal proxy now carries it: the written one should go"
    );
    assert!(!reopen(manual("http://127.0.0.1:7890")).list_with(Some(&store))[0].stale);

    // Written without a proxy, a configuration keeps its earlier fingerprint.
    let (temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    let reopened = ConfigManager::new(&temp.path().join("data"), ClientPaths::from_home(temp.path()));
    assert!(!reopened.list_with(Some(&store))[0].stale);
    assert!(
        reopened.with_network(manual("http://127.0.0.1:7890")).list_with(Some(&store))[0].stale,
        "a proxy set since then is missing from the file"
    );
}
