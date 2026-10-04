use super::*;
use std::io::{BufRead, Write};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[tokio::test]
async fn cancellation_before_start_and_stale_cancel_are_isolated() {
    let manager = LoginManager::default();
    let first = id();
    assert!(manager.cancel(first.clone()).await.unwrap());
    assert!(matches!(manager.begin(first.clone()), Err(error) if error == CANCELLED));
    let second = manager.begin(id()).unwrap();
    assert!(manager.cancel(first).await.unwrap());
    assert!(!*second.cancel.borrow());
    assert!(matches!(manager.begin(id()), Err(error) if error == "accounts.loginBusy"));
    drop(second);
    assert!(manager.begin(id()).is_ok());
}
#[tokio::test]
async fn cancellation_waits_for_cleanup_and_blocks_registration() {
    let manager = LoginManager::default();
    let attempt_id = id();
    let attempt = manager.begin(attempt_id.clone()).unwrap();
    let canceller = tokio::spawn({
        let manager = manager.clone();
        async move { manager.cancel(attempt_id).await }
    });
    let mut cancellation = attempt.cancellation();
    cancelled(&mut cancellation).await;
    assert!(!canceller.is_finished());
    assert_eq!(attempt.commit(|| Ok(42)).unwrap_err(), CANCELLED);
    drop(attempt);
    assert!(canceller.await.unwrap().unwrap());
    assert!(manager.begin(id()).is_ok());
}
#[tokio::test]
async fn completed_registration_wins_late_cancel() {
    let manager = LoginManager::default();
    let attempt_id = id();
    let attempt = manager.begin(attempt_id.clone()).unwrap();
    assert_eq!(attempt.commit(|| Ok(42)).unwrap(), 42);
    assert!(!manager.cancel(attempt_id.clone()).await.unwrap());
    drop(attempt);
    assert!(!manager.cancel(attempt_id).await.unwrap());
}
#[test]
fn only_official_https_authorization_urls_are_opened() {
    assert!(validate_auth_url("https://auth.openai.com/oauth/authorize?state=synthetic").is_ok());
    for url in [
        "file:///C:/test.cmd",
        "https://auth.openai.com.evil.invalid/",
        "http://auth.openai.com/",
        "https://user@auth.openai.com/",
        "https://auth.openai.com:444/",
    ] {
        assert!(validate_auth_url(url).is_err());
    }
}

fn fixture(mode: &str, home: &std::path::Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "core::account_login::tests::mock_app_server",
            "--ignored",
            "--nocapture",
        ])
        .env("AGENT_HARBOR_LOGIN_FIXTURE", mode)
        .env("AGENT_HARBOR_LOGIN_FIXTURE_HOME", home);
    command
}
fn temp_home(root: &tempfile::TempDir) -> TemporaryHome {
    let home = root.path().join("login");
    std::fs::create_dir(&home).unwrap();
    TemporaryHome(home)
}
#[tokio::test]
async fn mock_oauth_success_validates_credentials_and_cleans_home() {
    let root = tempfile::tempdir().unwrap();
    let home = temp_home(&root);
    let command = fixture("success", &home.0);
    let (_sender, cancel) = watch::channel(false);
    let artifact = run_login(command, home, cancel, Duration::from_secs(10), |_| Ok(()))
        .await
        .unwrap();
    assert!(artifact.auth_path.is_file());
    drop(artifact);
    assert!(!root.path().join("login").exists());
}
#[tokio::test]
async fn mock_oauth_failure_timeout_and_browser_failure_clean_home() {
    for mode in [
        "failure",
        "timeout",
        "browser",
        "invalid-auth",
        "invalid-url",
    ] {
        let root = tempfile::tempdir().unwrap();
        let home = temp_home(&root);
        let command = fixture(mode, &home.0);
        let (sender, cancel) = watch::channel(false);
        let result = run_login(command, home, cancel, Duration::from_millis(800), |_| {
            if mode == "browser" {
                Err("accounts.browserFailed".into())
            } else {
                Ok(())
            }
        })
        .await;
        drop(sender);
        assert!(result.is_err(), "{mode}");
        assert!(!root.path().join("login").exists(), "{mode}");
        if mode == "timeout" {
            assert_eq!(result.err().unwrap(), "accounts.loginTimeout");
        }
    }
}
#[tokio::test]
async fn mock_oauth_cancel_releases_callback_and_retries() {
    let root = tempfile::tempdir().unwrap();
    let home = temp_home(&root);
    let command = fixture("wait", &home.0);
    let (sender, cancel) = watch::channel(false);
    let result = run_login(command, home, cancel, Duration::from_secs(10), |url| {
        let url = reqwest::Url::parse(url).unwrap();
        let port: u16 = url
            .query_pairs()
            .find(|(name, _)| name == "port")
            .unwrap()
            .1
            .parse()
            .unwrap();
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        std::fs::write(root.path().join("port"), port.to_string()).unwrap();
        sender.send(true).unwrap();
        Ok(())
    })
    .await;
    assert_eq!(result.err().unwrap(), CANCELLED);
    assert!(root.path().join("cancel-observed").is_file());
    let port: u16 = std::fs::read_to_string(root.path().join("port"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(!root.path().join("login").exists());
    let home = temp_home(&root);
    let command = fixture("success", &home.0);
    let (_sender, cancel) = watch::channel(false);
    assert!(
        run_login(command, home, cancel, Duration::from_secs(10), |_| Ok(()))
            .await
            .is_ok()
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cancellation_reaps_owned_descendant_processes_before_returning() {
    let root = tempfile::tempdir().unwrap();
    let home = temp_home(&root);
    let home_path = home.0.clone();
    let command = fixture("descendants", &home.0);
    let (sender, cancel) = watch::channel(false);
    let port = std::sync::Mutex::new(0u16);
    let result = run_login(command, home, cancel, Duration::from_secs(10), |_| {
        *port.lock().unwrap() = std::fs::read_to_string(home_path.join("child-port"))
            .unwrap()
            .parse()
            .unwrap();
        sender.send(true).unwrap();
        Ok(())
    })
    .await;
    assert_eq!(result.err().unwrap(), CANCELLED);
    assert!(std::net::TcpListener::bind(("127.0.0.1", *port.lock().unwrap())).is_ok());
    assert!(!home_path.exists());
}

#[tokio::test]
#[ignore = "Installed CLI handshake only; no authorization is started"]
async fn installed_cli_supports_login_rpc_and_file_credentials() {
    let root = tempfile::tempdir().unwrap();
    let mut command = account_monitor::codex_login_command(&[
        "-c",
        "cli_auth_credentials_store='file'",
        "app-server",
    ])
    .unwrap();
    command
        .env("CODEX_HOME", root.path())
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    #[cfg(windows)]
    let job = WorkerJob::attach(&child).unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(Duration::from_secs(15), async {
        request(
            &mut stdin,
            &mut reader,
            1,
            "initialize",
            json!({"clientInfo":{"name":"agent-harbor-test","version":"1.0.0"}}),
        )
        .await
        .unwrap();
        send(&mut stdin, json!({"method":"initialized"}))
            .await
            .unwrap();
        let result = request(
            &mut stdin,
            &mut reader,
            2,
            "account/login/cancel",
            json!({"loginId":uuid::Uuid::new_v4().to_string()}),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "notFound");
    })
    .await
    .unwrap();
    #[cfg(windows)]
    job.stop().await.unwrap();
    let _ = child.kill().await;
    let _ = child.wait().await;
    assert!(!root.path().join("auth.json").exists());
}

#[test]
#[ignore = "Child process RPC fixture, never contacts OAuth services"]
fn mock_app_server() {
    let Ok(mode) = std::env::var("AGENT_HARBOR_LOGIN_FIXTURE") else {
        return;
    };
    let home = PathBuf::from(std::env::var_os("AGENT_HARBOR_LOGIN_FIXTURE_HOME").unwrap());
    if mode == "descendant" {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write(
            home.join("child-port"),
            listener.local_addr().unwrap().port().to_string(),
        )
        .unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let reply = match value["method"].as_str().unwrap() {
            "initialize" => json!({"id":value["id"],"result":{}}),
            "initialized" => continue,
            "account/login/start" => {
                if mode == "descendants" {
                    let mut command = fixture("descendant", &home);
                    command
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    #[cfg(windows)]
                    command.creation_flags(crate::platform::windows::CREATE_NO_WINDOW);
                    let _child = command.as_std_mut().spawn().unwrap();
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while !home.join("child-port").is_file() {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                let url = if mode == "invalid-url" {
                    "file:///test.cmd".into()
                } else {
                    format!("https://auth.openai.com/authorize?port={port}")
                };
                json!({"id":value["id"],"result":{"type":"chatgpt","loginId":"fixture-login","authUrl":url}})
            }
            "account/login/cancel" => {
                std::fs::write(home.parent().unwrap().join("cancel-observed"), "yes").unwrap();
                json!({"id":value["id"],"result":{}})
            }
            _ => panic!("unexpected RPC method"),
        };
        writeln!(stdout, "\n{reply}").unwrap();
        stdout.flush().unwrap();
        if value["method"] == "account/login/start" {
            if ["success", "failure", "invalid-auth"].contains(&mode.as_str()) {
                if mode == "success" {
                    std::fs::write(home.join("auth.json"), r#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access","account_id":"synthetic-account"}}"#).unwrap();
                }
                if mode == "invalid-auth" {
                    std::fs::write(home.join("auth.json"), "{}").unwrap();
                }
                // An unrelated completion must never complete the current attempt.
                writeln!(stdout,"{}",json!({"method":"account/login/completed","params":{"loginId":"other-login","success":true}})).unwrap();
                writeln!(stdout,"{}",json!({"method":"account/login/completed","params":{"loginId":"fixture-login","success":mode != "failure"}})).unwrap();
                stdout.flush().unwrap();
            }
        }
    }
}
