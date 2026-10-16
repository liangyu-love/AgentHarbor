//! Account-source flows against loopback stand-ins for WorkBuddy and OpenAI.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::{body::Bytes, server::conn::http1, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub(crate) struct Mock {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A loopback HTTP server answering each request with `respond`, as JSON.
pub(crate) async fn mock(respond: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static) -> Mock {
    mock_raw(move |seen| {
        let (status, body) = respond(seen);
        (status, "application/json", body.into_bytes())
    }).await
}

/// A loopback HTTP server answering each request with `respond`: status,
/// content type and raw bytes (an AWS event stream, an SSE stream).
pub(crate) async fn mock_raw(respond: impl Fn(&Seen) -> (u16, &'static str, Vec<u8>) + Send + Sync + 'static) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let respond = Arc::new(respond);
    let record = seen.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let respond = respond.clone();
            let record = record.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let respond = respond.clone();
                    let record = record.clone();
                    async move {
                        let (parts, body) = request.into_parts();
                        let body = body.collect().await.unwrap().to_bytes();
                        let seen = Seen {
                            method: parts.method.to_string(),
                            path: parts.uri.to_string(),
                            headers: parts.headers.iter()
                                .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or_default().to_owned())).collect(),
                            body: String::from_utf8_lossy(&body).into_owned(),
                        };
                        let (status, content_type, reply) = respond(&seen);
                        record.lock().unwrap().push(seen);
                        Ok::<_, std::convert::Infallible>(Response::builder().status(status)
                            .header("content-type", content_type).body(Full::new(Bytes::from(reply))).unwrap())
                    }
                });
                let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
            });
        }
    });
    Mock { base, seen, task }
}

fn client() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

fn temp_paths() -> (tempfile::TempDir, CodexPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(dir.path().join("codex"));
    paths.ensure_directories().unwrap();
    (dir, paths)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

fn stored(paths: &CodexPaths, kind: SourceKind, credentials: Credentials) -> String {
    store::mutate(&paths.app_data_dir, |store| Ok(store.keep_sign_in(Account {
        id: uuid::Uuid::new_v4().to_string(), kind, name: "Stored".into(), subject: "subject-1".into(), plan: None,
        active: false, credentials, added_at: 1, lapsed: None, quota: None, checkin: None, checkin_off: false,
    }))).unwrap()
}

#[test]
fn workbuddy_sign_in_waits_for_the_browser_then_keeps_the_account() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let token_polls = Arc::new(AtomicUsize::new(0));
        let polls = token_polls.clone();
        let upstream = mock(move |seen| {
            let path = seen.path.as_str();
            let reply = if path == "/v2/plugin/auth/state?platform=workbuddy" {
                assert_eq!(seen.method, "POST");
                assert_eq!(seen.headers["x-no-authorization"], "true");
                json!({"code":0,"data":{"state":"st-1","authUrl":"https://www.codebuddy.cn/login?from=app"}})
            } else if path == "/v2/plugin/auth/token?state=st-1" {
                if polls.fetch_add(1, Ordering::SeqCst) == 0 {
                    json!({"code":11217,"msg":"not ready"})
                } else {
                    json!({"code":0,"data":{"accessToken":"at-1","refreshToken":"rt-1","expiresIn":3600,"domain":"tenant.example"}})
                }
            } else if path == "/v2/plugin/login/account?state=st-1" {
                json!({"code":0,"data":{"uid":"uid-1","nickname":" Harbor ","phoneNumber":"138"}})
            } else {
                return (404, "{}".into());
            };
            (200, reply.to_string())
        }).await;
        set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let opened = Arc::new(Mutex::new(String::new()));
        let page = opened.clone();
        let account = sign_in(&paths, &client(), SourceKind::Workbuddy, None, None, cancel, move |prompt| {
            assert!(prompt.code.is_none());
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        }).await.unwrap();
        set_endpoint_override(SourceKind::Workbuddy, None);

        let url = reqwest::Url::parse(&opened.lock().unwrap()).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!((query["from"].as_str(), query["version"].as_str()), ("app", "2.0.0"));
        assert_eq!(query["loginSessionId"].len(), 32);
        assert_eq!((account.name.as_str(), account.subject.as_str()), ("Harbor", "uid-1"));
        let Credentials::WorkBuddy(creds) = &account.credentials else { panic!("WorkBuddy credentials") };
        assert_eq!((creds.access.as_str(), creds.refresh.as_str(), creds.domain.as_str()), ("at-1", "rt-1", "tenant.example"));
        assert_eq!(token_polls.load(Ordering::SeqCst), 2, "a not-ready answer is asked again");
        let account_poll = upstream.seen.lock().unwrap().iter().find(|seen| seen.path.starts_with("/v2/plugin/login/account")).cloned().unwrap();
        assert_eq!(account_poll.headers["authorization"], "Bearer at-1");
        assert_eq!(account_poll.headers["x-domain"], "tenant.example");

        assert!(keep(&paths, account).unwrap(), "the first account of its source");
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Workbuddy).unwrap();
        assert_eq!(source.accounts.len(), 1);
        assert!(source.accounts[0].active && !source.exposed);
        assert!(!serde_json::to_string(&listed).unwrap().contains("at-1"), "views never carry tokens");
    });
}

#[test]
fn a_cancelled_workbuddy_sign_in_stops_polling() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock(|seen| {
            if seen.path.starts_with("/v2/plugin/auth/state") {
                (200, json!({"code":0,"data":{"state":"s","authUrl":"https://www.codebuddy.cn/login"}}).to_string())
            } else {
                (200, json!({"code":11217}).to_string())
            }
        }).await;
        set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (cancel_tx, cancel) = watch::channel(false);
        let http = client();
        let attempt = sign_in(&paths, &http, SourceKind::Workbuddy, None, None, cancel, |_| Ok(()));
        let cancel_later = async {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            cancel_tx.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(attempt, cancel_later);
        set_endpoint_override(SourceKind::Workbuddy, None);
        assert_eq!(result.err().as_deref(), Some("accounts.loginCancelled"));
    });
}

#[test]
fn a_near_expired_workbuddy_token_is_refreshed_once_for_concurrent_requests() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let refreshes = Arc::new(AtomicUsize::new(0));
        let count = refreshes.clone();
        let upstream = mock(move |seen| {
            assert_eq!(seen.path, "/v2/plugin/auth/token/refresh");
            assert_eq!(seen.headers["x-refresh-token"], "rt-old");
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(30));
            (200, json!({"code":0,"data":{"accessToken":"at-new","refreshToken":"rt-new","expiresIn":3600}}).to_string())
        }).await;
        set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let id = stored(&paths, SourceKind::Workbuddy, Credentials::WorkBuddy(workbuddy::Creds {
            uid: "uid-1".into(), access: "at-old".into(), refresh: "rt-old".into(),
            expires_at: chrono::Utc::now().timestamp_millis(), ..Default::default()
        }));
        let client = client();
        let (first, second) = tokio::join!(
            credentials(&paths.app_data_dir, &client, SourceKind::Workbuddy),
            credentials(&paths.app_data_dir, &client, SourceKind::Workbuddy),
        );
        set_endpoint_override(SourceKind::Workbuddy, None);
        for auth in [first.ok().unwrap(), second.ok().unwrap()] {
            assert!(auth.headers.iter().any(|(name, value)| *name == "authorization" && value == "Bearer at-new"));
            assert!(auth.base.ends_with("/v2"));
        }
        assert_eq!(refreshes.load(Ordering::SeqCst), 1, "a rotated refresh token is spent once");
        let saved = store::load(&paths.app_data_dir).unwrap();
        let Credentials::WorkBuddy(creds) = &saved.accounts.iter().find(|account| account.id == id).unwrap().credentials else { panic!() };
        assert_eq!((creds.access.as_str(), creds.refresh.as_str()), ("at-new", "rt-new"));
    });
}

#[test]
fn a_refused_refresh_marks_the_account_for_a_new_sign_in() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock(|_| (401, json!({"code":401,"msg":"token invalid"}).to_string())).await;
        set_endpoint_override(SourceKind::WorkbuddyAi, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        stored(&paths, SourceKind::WorkbuddyAi, Credentials::WorkBuddy(workbuddy::Creds {
            uid: "uid".into(), access: "at".into(), refresh: "rt".into(), expires_at: 1, ..Default::default()
        }));
        let refused = credentials(&paths.app_data_dir, &client(), SourceKind::WorkbuddyAi).await;
        assert_eq!(refused.err(), Some(SIGNED_OUT));
        let again = credentials(&paths.app_data_dir, &client(), SourceKind::WorkbuddyAi).await;
        assert_eq!(again.err(), Some(SIGNED_OUT), "a lapsed account is not retried");
        assert_eq!(upstream.seen.lock().unwrap().len(), 1);
        set_endpoint_override(SourceKind::WorkbuddyAi, None);
        let listed = list(&paths).unwrap();
        assert!(listed.sources.iter().any(|source| source.accounts.iter().any(|account| account.lapsed.is_some())));
        assert_eq!(credentials(&paths.app_data_dir, &client(), SourceKind::Workbuddy).await.err(), Some(NO_ACCOUNT));
    });
}

fn jwt(claims: Value) -> String {
    use base64::Engine as _;
    let part = |value: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string());
    format!("{}.{}.signature", part(json!({"alg":"RS256"})), part(claims))
}

fn form(body: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://local/?{body}")).unwrap().query_pairs().into_owned().collect()
}

#[test]
fn chatgpt_sign_in_registers_the_app_and_checks_the_id_token() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let nonce = Arc::new(Mutex::new(String::new()));
        let expected_nonce = nonce.clone();
        let upstream = mock(move |seen| {
            assert_eq!(seen.path, "/api/accounts/oauth/token");
            let fields = form(&seen.body);
            assert_eq!(fields["grant_type"], "authorization_code");
            assert_eq!(fields["client_id"], "issued-client");
            assert_eq!(fields["code"], "code-1");
            assert_eq!(fields["resource"], "https://api.openai.com/v1");
            assert!(fields["code_verifier"].len() >= 43);
            let id = jwt(json!({"iss":"https://auth.openai.com","aud":"issued-client","nonce":*expected_nonce.lock().unwrap(),
                "sub":"user-sub","email":"me@example.com","exp":chrono::Utc::now().timestamp() + 600,
                "https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}));
            (200, json!({"access_token":"oa-access","refresh_token":"oa-refresh","id_token":id,"expires_in":3600,
                "scope":"openid email offline_access chatgpt.tokens.use.direct"}).to_string())
        }).await;
        set_endpoint_override(SourceKind::ChatgptApi, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let page_nonce = nonce.clone();
        let account = sign_in(&paths, &client(), SourceKind::ChatgptApi, None, None, cancel, move |prompt| {
            let url = reqwest::Url::parse(prompt.url).unwrap();
            assert_eq!(url.path(), "/api/accounts/authorize");
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(query["client_id"], "dynamic_agent_client");
            assert_eq!(query["code_challenge_method"], "S256");
            assert!(query["scope"].contains("chatgpt.tokens.use.direct"));
            assert!(query["ext_agent_host_id"].starts_with("urn:uuid:"));
            *page_nonce.lock().unwrap() = query["nonce"].clone();
            let back = format!("{}?code=code-1&state={}&client_id=issued-client", query["redirect_uri"], query["state"]);
            tokio::spawn(async move {
                let page = Client::builder().no_proxy().build().unwrap().get(back).send().await.unwrap();
                assert!(page.text().await.unwrap().contains("Signed in"));
            });
            Ok(())
        }).await.unwrap();
        set_endpoint_override(SourceKind::ChatgptApi, None);
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("me@example.com", "user-sub", Some("plus")));
        let Credentials::ChatgptApi(creds) = &account.credentials else { panic!("ChatGPT credentials") };
        assert_eq!((creds.client_id.as_str(), creds.access.as_str(), creds.refresh.as_str()), ("issued-client", "oa-access", "oa-refresh"));
        let host = store::load(&paths.app_data_dir).unwrap().host_id.unwrap();
        assert_eq!(creds.host_id, host, "the machine id is made once and kept");
    });
}

#[test]
fn a_chatgpt_callback_from_another_sign_in_is_refused() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock(|_| panic!("no code is exchanged for a foreign state")).await;
        set_endpoint_override(SourceKind::ChatgptApi, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let result = sign_in(&paths, &client(), SourceKind::ChatgptApi, None, None, cancel, |prompt| {
            let url = reqwest::Url::parse(prompt.url).unwrap();
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            let back = format!("{}?code=c&state=forged&client_id=x", query["redirect_uri"]);
            tokio::spawn(async move { let _ = Client::builder().no_proxy().build().unwrap().get(back).send().await; });
            Ok(())
        }).await;
        set_endpoint_override(SourceKind::ChatgptApi, None);
        assert_eq!(result.err().as_deref(), Some("accounts.loginFailed"));
    });
}

#[test]
fn a_chatgpt_refresh_rotates_the_token_and_a_dead_one_lapses() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let dead = Arc::new(Mutex::new(false));
        let refuse = dead.clone();
        let upstream = mock(move |seen| {
            let fields = form(&seen.body);
            assert_eq!((fields["grant_type"].as_str(), fields["client_id"].as_str()), ("refresh_token", "issued"));
            if *refuse.lock().unwrap() {
                return (400, json!({"error":"invalid_grant","error_description":"reused"}).to_string());
            }
            assert_eq!(fields["refresh_token"], "r-1");
            (200, json!({"access_token":"a-2","refresh_token":"r-2","expires_in":3600,"scope":"openid chatgpt.tokens.use.direct"}).to_string())
        }).await;
        set_endpoint_override(SourceKind::ChatgptApi, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        stored(&paths, SourceKind::ChatgptApi, Credentials::ChatgptApi(chatgpt_api::Creds {
            client_id: "issued".into(), access: "a-1".into(), refresh: "r-1".into(), expires_at: 0, ..Default::default()
        }));
        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::ChatgptApi).await.ok().unwrap();
        assert_eq!(auth.headers, vec![("authorization", "Bearer a-2".to_owned())]);
        assert!(auth.base.ends_with("/v1"));
        store::mutate(&paths.app_data_dir, |store| {
            if let Credentials::ChatgptApi(creds) = &mut store.accounts[0].credentials {
                creds.expires_at = 0;
            }
            Ok(())
        }).unwrap();
        *dead.lock().unwrap() = true;
        assert_eq!(credentials(&paths.app_data_dir, &client(), SourceKind::ChatgptApi).await.err(), Some(SIGNED_OUT));
        set_endpoint_override(SourceKind::ChatgptApi, None);
        assert!(store::load(&paths.app_data_dir).unwrap().accounts[0].lapsed.as_deref().unwrap().contains("invalid_grant"));
    });
}

#[test]
fn sharing_a_source_publishes_a_router_only_provider_and_keeps_model_choices() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    let (_dir, paths) = temp_paths();
    set_exposed(&paths, SourceKind::Workbuddy, true).unwrap();
    let store = router::load_store(&paths).unwrap();
    let provider = store.providers.iter().find(|provider| provider.id == "account-workbuddy").unwrap();
    assert_eq!(provider.account_source, Some(SourceKind::Workbuddy));
    assert!(provider.api_key.is_empty() && provider.enabled && provider.has_route_credentials() && provider.router_only());
    assert_eq!(provider.selected_protocol(), router::ModelProtocol::ChatCompletions);
    assert!(provider.base_url.ends_with("copilot.tencent.com/v2"));
    assert_eq!(provider.models.len(), workbuddy::models(SourceKind::Workbuddy).len());
    assert!(router::ensure_relay(&paths, "account-workbuddy").is_err(), "relay commands leave it alone");

    router::mutate_store(&paths, |store| {
        let provider = store.providers.iter_mut().find(|provider| provider.id == "account-workbuddy").unwrap();
        provider.models.iter_mut().find(|model| model.id == "glm-5.3").unwrap().included = false;
        provider.model = "glm-5.3-flash".into();
        Ok(())
    }).unwrap();
    set_exposed(&paths, SourceKind::Workbuddy, true).unwrap();
    let store = router::load_store(&paths).unwrap();
    let provider = store.providers.iter().find(|provider| provider.id == "account-workbuddy").unwrap();
    assert!(!provider.models.iter().find(|model| model.id == "glm-5.3").unwrap().included);
    assert_eq!(provider.model, "glm-5.3-flash");

    assert_eq!(set_exposed(&paths, SourceKind::ChatgptApi, true).unwrap_err(), "accountSources.errors.noModels");
    set_exposed(&paths, SourceKind::Workbuddy, false).unwrap();
    assert!(router::load_store(&paths).unwrap().providers.is_empty());
    assert!(list(&paths).unwrap().sources.iter().all(|source| !source.exposed));
}

#[test]
fn copilot_device_sign_in_reads_the_account_its_models_and_allowance() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let polls = Arc::new(AtomicUsize::new(0));
        let count = polls.clone();
        let upstream = mock(move |seen| {
            let reply = match (seen.method.as_str(), seen.path.as_str()) {
                ("POST", "/login/device/code") => {
                    assert_eq!(form(&seen.body)["client_id"], "Iv1.b507a08c87ecfe98");
                    json!({"device_code":"dc-1","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","interval":0,"expires_in":900})
                }
                ("POST", "/login/oauth/access_token") => {
                    assert_eq!(form(&seen.body)["device_code"], "dc-1");
                    if count.fetch_add(1, Ordering::SeqCst) == 0 { json!({"error":"authorization_pending"}) } else { json!({"access_token":"gho-1"}) }
                }
                ("GET", "/user") => {
                    assert_eq!(seen.headers["authorization"], "token gho-1");
                    json!({"login":"Octo"})
                }
                ("GET", "/copilot_internal/user") => json!({"copilot_plan":"individual",
                    "quota_snapshots":{"premium_interactions":{"entitlement":300,"remaining":250,"unlimited":false}}}),
                ("GET", "/copilot_internal/v2/token") => {
                    assert_eq!(seen.headers["editor-version"], "vscode/1.140.0");
                    json!({"token":"cop-1","expires_at":chrono::Utc::now().timestamp() + 1800,"endpoints":{"api":"https://elsewhere.invalid"}})
                }
                ("GET", "/models") => {
                    assert_eq!(seen.headers["authorization"], "Bearer cop-1");
                    assert_eq!(seen.headers["copilot-integration-id"], "vscode-chat");
                    json!({"data":[
                        {"id":"claude-sonnet-4.6","name":"Claude Sonnet 4.6","model_picker_enabled":true,"capabilities":{"type":"chat"},
                         "supported_endpoints":["/chat/completions","/v1/messages"],"policy":{"state":"enabled"}},
                        {"id":"gpt-4.1","name":"GPT-4.1","model_picker_enabled":true,"capabilities":{"type":"chat"}}
                    ]})
                }
                _ => return (404, "{}".into()),
            };
            (200, reply.to_string())
        }).await;
        set_endpoint_override(SourceKind::Copilot, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let shown = Arc::new(Mutex::new(None));
        let prompt_seen = shown.clone();
        let account = sign_in(&paths, &client(), SourceKind::Copilot, None, None, cancel, move |prompt| {
            *prompt_seen.lock().unwrap() = Some((prompt.url.to_owned(), prompt.code.map(str::to_owned)));
            Ok(())
        }).await.unwrap();
        assert_eq!(*shown.lock().unwrap(), Some(("https://github.com/login/device".to_owned(), Some("ABCD-1234".to_owned()))),
            "the device code is shown with GitHub's page");
        assert_eq!(polls.load(Ordering::SeqCst), 2, "a pending code is asked again");
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("Octo", "octo", Some("Pro")));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::Copilot, first).await.unwrap();
        set_endpoint_override(SourceKind::Copilot, None);

        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Copilot).unwrap();
        assert!(source.exposed && source.has_quota);
        let quota = source.accounts[0].quota.clone().unwrap();
        assert_eq!((quota.used, quota.limit, quota.unit.as_str()), (50.0, 300.0, "premium"));
        assert!(!serde_json::to_string(&listed).unwrap().contains("gho-1"), "views never carry tokens");
        let store = router::load_store(&paths).unwrap();
        let provider = store.providers.iter().find(|provider| provider.id == "account-copilot").unwrap();
        assert_eq!(provider.protocol_for_model("claude-sonnet-4.6"), router::ModelProtocol::AnthropicMessages);
        assert_eq!(provider.protocol_for_model("gpt-4.1"), router::ModelProtocol::ChatCompletions);
    });
}

/// Follow a sign-in's page back to its own loopback callback, as the browser would.
fn come_back(url: &str, path: &str, extra: &str) {
    let url = reqwest::Url::parse(url).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let base = query["redirect_uri"].clone();
    let back = if base.ends_with(path) { format!("{base}?{extra}&state={}", query["state"]) } else { format!("{base}{path}?{extra}&state={}", query["state"]) };
    tokio::spawn(async move {
        let page = Client::builder().no_proxy().build().unwrap().get(back).send().await.unwrap();
        assert!(page.text().await.unwrap().contains("Signed in"));
    });
}

#[test]
fn antigravity_sign_in_sets_the_account_up_and_reads_its_models_and_allowance() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock(|seen| {
            let reply = match seen.path.as_str() {
                "/token" => {
                    let fields = form(&seen.body);
                    assert_eq!(fields["grant_type"], "authorization_code");
                    assert_eq!(fields["client_id"], "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com");
                    assert!(fields["code_verifier"].len() >= 43);
                    json!({"access_token":"ya29-a","refresh_token":"1//r","expires_in":3600})
                }
                "/userinfo" => json!({"email":"Me@Gmail.com"}),
                "/v1internal:loadCodeAssist" => {
                    assert_eq!(seen.headers["authorization"], "Bearer ya29-a");
                    assert!(seen.headers["user-agent"].starts_with("antigravity/hub/"));
                    json!({"currentTier":{"id":"free-tier","name":"Antigravity Free"},"allowedTiers":[{"id":"free-tier","isDefault":true}]})
                }
                "/v1internal:onboardUser" => {
                    assert_eq!(serde_json::from_str::<Value>(&seen.body).unwrap()["metadata"]["ide_type"], "ANTIGRAVITY");
                    json!({"done":true,"response":{"cloudaicompanionProject":{"id":"proj-9","name":"x"}}})
                }
                "/v1internal:fetchAvailableModels" => {
                    assert_eq!(serde_json::from_str::<Value>(&seen.body).unwrap()["project"], "proj-9");
                    json!({"models":{
                        "gemini-3-flash":{"displayName":"Gemini 3 Flash","maxTokens":1048576,"maxOutputTokens":65536,"quotaInfo":{"remainingFraction":0.8}},
                        "claude-sonnet-4-6":{"displayName":"Claude Sonnet 4.6","maxTokens":200000,"quotaInfo":{"remainingFraction":0.25}},
                        "tab_flash_lite_preview":{"displayName":"tab"}},
                      "agentModelSorts":[{"groups":[{"modelIds":["claude-sonnet-4-6","gemini-3-flash"]}]}]})
                }
                _ => return (404, "{}".into()),
            };
            (200, reply.to_string())
        }).await;
        set_endpoint_override(SourceKind::Antigravity, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let account = sign_in(&paths, &client(), SourceKind::Antigravity, None, None, cancel, |prompt| {
            assert!(prompt.url.contains("/authorize?") && prompt.url.contains("access_type=offline"));
            assert!(prompt.url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A"), "the Antigravity client comes back to localhost");
            come_back(prompt.url, "/oauth-callback", "code=g-code");
            Ok(())
        }).await.unwrap();
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("Me@Gmail.com", "me@gmail.com", Some("Antigravity Free")));
        let Credentials::Google(creds) = &account.credentials else { panic!("Google credentials") };
        assert_eq!(creds.project, "proj-9");
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::Antigravity, first).await.unwrap();
        set_endpoint_override(SourceKind::Antigravity, None);

        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Antigravity).unwrap();
        assert!(source.exposed && source.risk.is_some());
        let ids: Vec<_> = source.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["claude-sonnet-4-6", "gemini-3-flash"], "the Antigravity picker order, chat models only");
        assert_eq!(source.models[1].max_output_tokens, Some(65536));
        let quota = source.accounts[0].quota.clone().unwrap();
        assert_eq!((quota.used, quota.limit, quota.unit.as_str()), (75.0, 100.0, "percent"), "the tightest model allowance");
        assert_eq!(source.accounts[0].project.as_deref(), Some("proj-9"));
        assert!(!serde_json::to_string(&listed).unwrap().contains("1//r"), "views never carry tokens");
    });
}

#[test]
fn kiro_sign_in_with_github_reads_the_profile_plan_and_models() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock(|seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            let reply = match path {
                "/oauth/token" => {
                    let body: Value = serde_json::from_str(&seen.body).unwrap();
                    assert_eq!(body["code"], "k-code");
                    assert!(body["redirect_uri"].as_str().unwrap().ends_with("/oauth/callback?login_option=github"));
                    assert!(seen.headers["user-agent"].starts_with("KiroIDE-"));
                    json!({"accessToken":"kat","refreshToken":"krt","expiresIn":3600,"profileArn":"arn:aws:codewhisperer:us-east-1:7:profile/P"})
                }
                "/Get-Usage-Limits" => {
                    assert_eq!(seen.headers["authorization"], "Bearer kat");
                    assert!(seen.path.contains("profileArn=arn"));
                    json!({"userInfo":{"email":"kiro@example.com"},"subscriptionInfo":{"subscriptionTitle":"KIRO PRO+"},
                        "usageBreakdownList":[{"currentUsageWithPrecision":120.5,"usageLimitWithPrecision":1000}]})
                }
                "/List-Available-Models" => json!({"models":[
                    {"modelId":"claude-sonnet-4.5","modelName":"Claude Sonnet 4.5","tokenLimits":{"maxInputTokens":200000,"maxOutputTokens":64000}},
                    {"modelId":"auto"}],"defaultModel":{"modelId":"auto"}}),
                _ => return (404, "{}".into()),
            };
            (200, reply.to_string())
        }).await;
        set_endpoint_override(SourceKind::Kiro, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let account = sign_in(&paths, &client(), SourceKind::Kiro, None, None, cancel, |prompt| {
            assert!(prompt.url.contains("/signin?") && prompt.url.contains("redirect_from=KiroIDE"));
            come_back(prompt.url, "/oauth/callback", "login_option=github&code=k-code");
            Ok(())
        }).await.unwrap();
        assert_eq!((account.name.as_str(), account.plan.as_deref()), ("kiro@example.com", Some("Kiro Pro+")));
        let Credentials::Kiro(creds) = &account.credentials else { panic!("Kiro credentials") };
        assert_eq!((creds.method.as_str(), creds.provider.as_str(), creds.api_region().as_str()), ("social", "Github", "us-east-1"));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::Kiro, first).await.unwrap();
        set_endpoint_override(SourceKind::Kiro, None);
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Kiro).unwrap();
        let names: Vec<_> = source.models.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(names, ["Auto", "Claude Sonnet 4.5"], "the Kiro default first");
        let quota = source.accounts[0].quota.clone().unwrap();
        assert_eq!((quota.used, quota.limit, quota.unit.as_str()), (120.5, 1000.0, "credits"));
        assert!(source.exposed);
    });
}

/// A stand-in Grok CLI: `login` prints its device link and signs in to
/// GROK_HOME; anything else does nothing.
fn fake_grok(dir: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let script = dir.join("grok.cmd");
        std::fs::write(&script, "@echo off\r\nif not \"%1\"==\"login\" exit /b 0\r\necho Open https://accounts.x.ai/device?user_code=GROK-1234 to sign in\r\necho {\"https://auth.x.ai\":{\"key\":\"grok-key\",\"email\":\"Me@X.ai\",\"expires_at\":\"2099-01-01T00:00:00Z\"}}> \"%GROK_HOME%/auth.json\"\r\necho Signed in.\r\nexit /b 0\r\n").unwrap();
        script
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("grok");
        std::fs::write(&script, "#!/bin/sh\n[ \"$1\" = login ] || exit 0\necho 'Open https://accounts.x.ai/device?user_code=GROK-1234 to sign in'\necho '{\"https://auth.x.ai\":{\"key\":\"grok-key\",\"email\":\"Me@X.ai\",\"expires_at\":\"2099-01-01T00:00:00Z\"}}' > \"$GROK_HOME/auth.json\"\necho 'Signed in.'\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }
}

#[test]
fn grok_signs_in_through_its_cli_in_a_home_of_its_own_and_forgets_it_on_removal() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let (dir, paths) = temp_paths();
        *grok::EXECUTABLE.lock().unwrap() = Some(fake_grok(dir.path()));
        let (_cancel, cancel) = watch::channel(false);
        let opened = Arc::new(Mutex::new(String::new()));
        let page = opened.clone();
        let account = sign_in(&paths, &client(), SourceKind::Grok, None, None, cancel, move |prompt| {
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        }).await.unwrap();
        *grok::EXECUTABLE.lock().unwrap() = None;
        assert_eq!(opened.lock().unwrap().as_str(), "https://accounts.x.ai/device?user_code=GROK-1234");
        assert_eq!((account.name.as_str(), account.subject.as_str()), ("Me@X.ai", "me@x.ai"));
        let Credentials::Grok(creds) = account.credentials.clone() else { panic!("Grok credentials") };
        let home = std::path::PathBuf::from(&creds.home);
        assert!(home.starts_with(paths.app_data_dir.join("account-sources").join("grok")), "a home of AgentHarbor's, never ~/.grok");
        assert!(matches!(grok::token(&paths.app_data_dir, &creds).await, Ok(token) if token == "grok-key"));
        assert!(!serde_json::to_string(&account.credentials).unwrap().contains("grok-key"), "the token stays in the CLI's home");
        keep(&paths, account).unwrap();
        let id = store::load(&paths.app_data_dir).unwrap().accounts[0].id.clone();
        remove(&paths, &client(), &id).await.unwrap();
        assert!(!home.exists(), "the home made for it goes with it");
    });
}

/// A stand-in Cursor CLI: `login` prints a link it wraps and signs in,
/// `about` says who is signed in, `models` lists two.
fn fake_cursor(dir: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let script = dir.join("cursor-agent.cmd");
        let lines = [
            "@echo off",
            "if \"%1\"==\"about\" goto about",
            "if \"%1\"==\"login\" goto login",
            "if \"%1\"==\"models\" goto models",
            "exit /b 0",
            ":about",
            "if not exist \"%~dp0signed-in\" (echo Not logged in& exit /b 1)",
            "echo {\"userEmail\":\"Me@Cursor.sh\",\"subscriptionTier\":\"Pro\"}",
            "exit /b 0",
            ":login",
            "echo Open this link to sign in:",
            "echo https://cursor.com/loginDeepControl?challenge=abc^&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427",
            "echo ^&mode=login^&redirectTarget=cli^&supportsSelectedTeamLogin=true",
            "echo Waiting for sign-in...",
            "type nul > \"%~dp0signed-in\"",
            "exit /b 0",
            ":models",
            "echo auto - Auto",
            "echo gpt-6-high - GPT-6 High 1M",
            "exit /b 0",
        ];
        std::fs::write(&script, lines.join("\r\n") + "\r\n").unwrap();
        script
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("cursor-agent");
        let body = r#"#!/bin/sh
here=$(dirname "$0")
case "$1" in
about) [ -f "$here/signed-in" ] || { echo 'Not logged in'; exit 1; }
  echo '{"userEmail":"Me@Cursor.sh","subscriptionTier":"Pro"}' ;;
login) echo 'Open this link to sign in:'
  echo 'https://cursor.com/loginDeepControl?challenge=abc&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427'
  echo '&mode=login&redirectTarget=cli&supportsSelectedTeamLogin=true'
  echo 'Waiting for sign-in...'
  touch "$here/signed-in" ;;
models) echo 'auto - Auto'; echo 'gpt-6-high - GPT-6 High 1M' ;;
esac
"#;
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }
}

#[test]
fn cursor_signs_in_through_its_cli_and_keeps_the_one_sign_in_the_cli_has() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let (dir, paths) = temp_paths();
        *cursor::EXECUTABLE.lock().unwrap() = Some(fake_cursor(dir.path()));
        *cursor::TOKEN.lock().unwrap() = Some(String::new());
        let (_cancel, cancel) = watch::channel(false);
        let opened = Arc::new(Mutex::new(String::new()));
        let page = opened.clone();
        let account = sign_in(&paths, &client(), SourceKind::Cursor, None, None, cancel.clone(), move |prompt| {
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        }).await.unwrap();
        assert_eq!(opened.lock().unwrap().as_str(), "https://cursor.com/loginDeepControl?challenge=abc&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427&mode=login&redirectTarget=cli&supportsSelectedTeamLogin=true",
            "the link the CLI wrapped is handed on whole");
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("Me@Cursor.sh", "me@cursor.sh", Some("Pro")));
        keep(&paths, account).unwrap();

        // Signed in already: kept as it is, without a page to open.
        *cursor::TOKEN.lock().unwrap() = Some("tok".into());
        let again = sign_in(&paths, &client(), SourceKind::Cursor, None, None, cancel, |_| panic!("no page for a CLI signed in already")).await.unwrap();
        keep(&paths, again).unwrap();
        let mut other = store::load(&paths.app_data_dir).unwrap().accounts[0].clone();
        other.id = "other".into();
        other.subject = "someone@else.sh".into();
        keep(&paths, other).unwrap();
        let kept: Vec<_> = store::load(&paths.app_data_dir).unwrap().accounts.into_iter().filter(|account| account.kind == SourceKind::Cursor).collect();
        assert_eq!(kept.len(), 1, "the CLI keeps one sign-in, and so does its source");
        assert_eq!(kept[0].subject, "someone@else.sh");

        let models = cursor_models_for_test(&paths).await;
        assert_eq!(models, vec![("auto".to_owned(), 200_000), ("gpt-6-high".to_owned(), 1_000_000)]);
        *cursor::EXECUTABLE.lock().unwrap() = None;
        *cursor::TOKEN.lock().unwrap() = None;
    });
}

async fn cursor_models_for_test(paths: &CodexPaths) -> Vec<(String, u64)> {
    cursor::fetch_models(&paths.app_data_dir).await.unwrap().into_iter()
        .map(|model| (model.id, model.context_window.unwrap())).collect()
}

#[test]
fn models_left_out_of_a_share_never_reach_the_router_and_leaving_all_out_stops_it() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    let (_dir, paths) = temp_paths();
    store::mutate(&paths.app_data_dir, |store| {
        store.models.insert(SourceKind::Grok, ["grok-4.7", "grok-4.7-fast", "grok-code"].iter()
            .map(|id| SourceModel { id: (*id).into(), name: (*id).into(), ..Default::default() }).collect());
        Ok(())
    }).unwrap();
    // Before it is shared, every model may be left out.
    set_hidden_models(&paths, SourceKind::Grok, vec!["grok-4.7".into(), "grok-4.7-fast".into(), "grok-code".into()]).unwrap();
    set_hidden_models(&paths, SourceKind::Grok, vec!["grok-4.7-fast".into(), "unknown".into()]).unwrap();
    set_exposed(&paths, SourceKind::Grok, true).unwrap();
    let shared = |paths: &CodexPaths| -> Vec<String> {
        router::load_store(paths).unwrap().providers.into_iter().find(|provider| provider.account_source == Some(SourceKind::Grok))
            .unwrap().models.into_iter().map(|model| model.id).collect()
    };
    assert_eq!(shared(&paths), ["grok-4.7", "grok-code"]);
    let source = list(&paths).unwrap().sources.into_iter().find(|source| source.kind == SourceKind::Grok).unwrap();
    assert_eq!(source.hidden_models, ["grok-4.7-fast"], "an unknown id is not kept");
    assert_eq!(source.models.len(), 3, "the page still lists every model");
    // Leaving every model out stops the share instead of being refused.
    set_hidden_models(&paths, SourceKind::Grok, vec!["grok-4.7".into(), "grok-4.7-fast".into(), "grok-code".into()]).unwrap();
    let exposed = |paths: &CodexPaths| list(paths).unwrap().sources.into_iter().find(|source| source.kind == SourceKind::Grok).unwrap().exposed;
    assert!(!exposed(&paths));
    assert!(!router::load_store(&paths).unwrap().providers.iter().any(|provider| provider.account_source == Some(SourceKind::Grok)));
    assert_eq!(set_exposed(&paths, SourceKind::Grok, true).unwrap_err(), "accountSources.errors.noModelsSelected",
        "a source with every model left out cannot be shared");
    set_hidden_models(&paths, SourceKind::Grok, Vec::new()).unwrap();
    assert!(!exposed(&paths), "bringing models back does not share the source by itself");
    set_exposed(&paths, SourceKind::Grok, true).unwrap();
    assert_eq!(shared(&paths), ["grok-4.7", "grok-4.7-fast", "grok-code"]);
}

fn workbuddy_creds() -> Credentials {
    Credentials::WorkBuddy(workbuddy::Creds { uid: "uid-1".into(), access: "at-1".into(), domain: "copilot.tencent.com".into(), ..Default::default() })
}

fn checkin_of(paths: &CodexPaths, id: &str) -> Option<checkin::Checkin> {
    store::load(&paths.app_data_dir).unwrap().accounts.into_iter().find(|account| account.id == id).unwrap().checkin
}

#[test]
fn workbuddy_check_in_claims_once_a_day_and_only_while_on() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let claims = Arc::new(AtomicUsize::new(0));
        let claimed = claims.clone();
        let upstream = mock(move |seen| {
            assert_eq!(seen.headers["authorization"], "Bearer at-1");
            assert_eq!(seen.headers["x-user-id"], "uid-1");
            match seen.path.as_str() {
                "/v2/billing/meter/checkin-activity-status" => (200, json!({"code":0,"data":{
                    "active":true,"today_checked_in":false,"streak_days":"2","daily_credit":5}}).to_string()),
                "/v2/billing/meter/daily-checkin" => {
                    claimed.fetch_add(1, Ordering::SeqCst);
                    (200, json!({"code":0,"data":{"credit":"5","streak_days":3,"message":"签到成功"}}).to_string())
                }
                other => panic!("unexpected {other}"),
            }
        }).await;
        set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let id = stored(&paths, SourceKind::Workbuddy, workbuddy_creds());

        assert_eq!(checkin::run(&paths.app_data_dir, &client(), None, false).await.unwrap(), 0, "auto check-in is off until turned on");
        checkin::set_auto(&paths.app_data_dir, SourceKind::Workbuddy, true).unwrap();
        assert_eq!(checkin::run(&paths.app_data_dir, &client(), None, false).await.unwrap(), 1);
        let result = checkin_of(&paths, &id).unwrap();
        assert_eq!((result.outcome, result.credit, result.streak), (checkin::Outcome::Claimed, 5.0, 3));
        assert_eq!(result.day, checkin::today());
        assert_eq!(result.message.as_deref(), Some("签到成功"));

        assert_eq!(checkin::run(&paths.app_data_dir, &client(), None, false).await.unwrap(), 0, "an answer holds for the day");
        assert_eq!(checkin::run(&paths.app_data_dir, &client(), Some(SourceKind::Workbuddy), true).await.unwrap(), 0, "a press too");
        assert_eq!(claims.load(Ordering::SeqCst), 1);

        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Workbuddy).unwrap();
        assert!(source.checks_in && source.auto_checkin);
        assert_eq!(source.accounts[0].checkin.as_ref().unwrap().outcome, checkin::Outcome::Claimed);
        assert_eq!(listed.today, checkin::today());
        assert!(!listed.sources.iter().find(|source| source.kind == SourceKind::WorkbuddyAi).unwrap().checks_in, "WorkBuddy AI has no event");
        set_endpoint_override(SourceKind::Workbuddy, None);
    });
}

#[test]
fn workbuddy_check_in_tells_its_answers_apart() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let mode = Arc::new(Mutex::new("already"));
        let now = mode.clone();
        let upstream = mock(move |seen| {
            let mode = *now.lock().unwrap();
            match (mode, seen.path.as_str()) {
                (_, "/v2/billing/meter/checkin-activity-status") if mode == "inactive" => (200, json!({"code":0,"data":{"active":false}}).to_string()),
                (_, "/v2/billing/meter/checkin-activity-status") if mode == "unauthorized" => (401, "<html>Unauthorized</html>".into()),
                (_, "/v2/billing/meter/checkin-activity-status") => (200, json!({"code":0,"data":{"active":true,"today_checked_in":false,"streak_days":4}}).to_string()),
                ("already", "/v2/billing/meter/daily-checkin") => (200, json!({"code":1001,"msg":"今日已签到"}).to_string()),
                ("refused", "/v2/billing/meter/daily-checkin") => (502, "bad gateway".into()),
                (_, "/billing/meter/get-user-resource-summary") => (200, json!({"code":0,"data":{"Packages":[]}}).to_string()),
                other => panic!("unexpected {other:?}"),
            }
        }).await;
        set_endpoint_override(SourceKind::Workbuddy, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let id = stored(&paths, SourceKind::Workbuddy, workbuddy_creds());
        let press = || async { checkin::run(&paths.app_data_dir, &client(), Some(SourceKind::Workbuddy), true).await.unwrap() };

        press().await;
        let done = checkin_of(&paths, &id).unwrap();
        assert_eq!((done.outcome, done.streak), (checkin::Outcome::Done, 4), "1001 is in already, with the status's streak");

        let reset = |paths: &CodexPaths| store::mutate(&paths.app_data_dir, |store| { store.accounts[0].checkin = None; Ok(()) }).unwrap();
        *mode.lock().unwrap() = "refused";
        reset(&paths);
        press().await;
        let failed = checkin_of(&paths, &id).unwrap();
        assert_eq!(failed.outcome, checkin::Outcome::Failed);
        assert!(!failed.offline, "a reply, however bad, reached WorkBuddy");

        *mode.lock().unwrap() = "unauthorized";
        press().await;
        assert_eq!(checkin_of(&paths, &id).unwrap().outcome, checkin::Outcome::OwnApp, "a failure is tried again on a press; 401 with readable credits is the app's");

        *mode.lock().unwrap() = "inactive";
        reset(&paths);
        press().await;
        assert_eq!(checkin_of(&paths, &id).unwrap().outcome, checkin::Outcome::Inactive);

        checkin::set_account_off(&paths.app_data_dir, &id, true).unwrap();
        reset(&paths);
        assert_eq!(press().await, 0, "an account left out is not asked, even on a press");
        set_endpoint_override(SourceKind::Workbuddy, None);
    });
}

#[test]
fn minimax_signs_in_by_device_code_reads_its_plan_and_checks_in() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let polls = Arc::new(AtomicUsize::new(0));
        let polled = polls.clone();
        let upstream = mock(move |seen| {
            let path = seen.path.split('?').next().unwrap_or_default().to_owned();
            let signed = |seen: &Seen| {
                let seconds = &seen.headers["x-timestamp"];
                let body = if seen.method == "POST" { seen.body.as_str() } else { "" };
                assert_eq!(seen.headers["x-signature"], oauth::md5_hex(&format!("{seconds}I*7Cf%WZ#S&%1RlZJ&C2{body}")), "{}", seen.path);
            };
            let reply = match (seen.method.as_str(), path.as_str()) {
                ("POST", "/oauth2/device/code") => {
                    assert!(seen.body.contains("client_id=mcode-public") && seen.body.contains("code_challenge_method=S256"));
                    json!({"user_code": "WXYZ-1234", "device_code": "dc-1", "verification_uri_complete": "https://account.minimax.cn/device?code=WXYZ-1234",
                        "expires_in": 600, "interval": 0.01})
                }
                ("POST", "/oauth2/token") if seen.body.contains("grant_type=urn") => {
                    assert!(seen.body.contains("device_code=dc-1") && seen.body.contains("code_verifier="));
                    if polled.fetch_add(1, Ordering::SeqCst) == 0 {
                        json!({"status": "pending"})
                    } else {
                        json!({"access_token": "mm-at-1", "refresh_token": "mm-rt-1", "expires_in": 3600})
                    }
                }
                ("GET", "/v1/api/user/info") => {
                    signed(seen);
                    assert!(seen.headers.contains_key("yy") && seen.path.contains("device_platform=mcode") && seen.path.contains("client=mcode"));
                    json!({"statusInfo": {"code": 0}, "data": {"userInfo": {"realUserID": "mm-u1", "userEmail": "mm@example.com"}}})
                }
                ("GET", "/mavis/api/v1/models") => {
                    assert_eq!(seen.headers["authorization"], "Bearer mm-at-1");
                    json!({"providers": [{"providerId": "minimax", "config": {"models": {
                        "MiniMax-M3": {"name": "MiniMax-M3", "limit": {"context": 512000}, "thinking_config": {"mode": "switchable"}}}}}]})
                }
                ("POST", "/matrix/api/v1/user/get_user_extra_info") => {
                    signed(seen);
                    json!({"workspaces": [{"workspace_type": 0, "workspace_id": 7, "has_token_plan": true, "op_group_id": "g-7"}]})
                }
                ("POST", "/matrix/api/v1/commerce/get_membership_info") => {
                    assert_eq!(seen.body, r#"{"workspace_id":7}"#);
                    json!({"data": {"token_plan_tier": "Plus", "opcredit_balance": "88.5"}})
                }
                ("GET", "/v1/api/openplatform/coding_plan/remains") => {
                    assert_eq!(seen.headers["x-group-id"], "g-7");
                    json!({"base_resp": {"status_code": 0}, "model_remains": [{"model_name": "general",
                        "current_interval_remaining_percent": 75, "current_interval_status": 1}]})
                }
                ("GET", "/minimax-cloud/api/v1/signin/status") => {
                    signed(seen);
                    assert!(seen.path.ends_with("?timezone_id=Asia%2FShanghai"));
                    json!({"data": {"days": [{"day_no": 1, "points": 50, "status": 3, "is_today": false},
                        {"day_no": 2, "points": 60, "status": 2, "is_today": true}]}})
                }
                ("POST", "/minimax-cloud/api/v1/signin/claim") => {
                    signed(seen);
                    json!({"data": {"claim_result": 1, "day_no": 2, "points": 60}, "base_resp": {"status_code": 0}})
                }
                other => panic!("unexpected {other:?}"),
            };
            (200, reply.to_string())
        }).await;
        set_endpoint_override(SourceKind::MinimaxCode, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let shown = Arc::new(Mutex::new(None));
        let code = shown.clone();
        let account = sign_in(&paths, &client(), SourceKind::MinimaxCode, None, None, cancel, move |prompt| {
            *code.lock().unwrap() = prompt.code.map(str::to_owned);
            Ok(())
        }).await.unwrap();
        assert_eq!(shown.lock().unwrap().as_deref(), Some("WXYZ-1234"));
        assert_eq!((account.name.as_str(), account.subject.as_str()), ("mm@example.com", "mm-u1"));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::MinimaxCode, first).await.unwrap();

        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::MinimaxCode).unwrap();
        assert!(source.exposed && source.checks_in && source.risk.is_some());
        assert_eq!(source.protocol, router::ModelProtocol::AnthropicMessages);
        assert_eq!(source.models[0].reasoning, ["none", "high"]);
        let account = &source.accounts[0];
        assert_eq!(account.plan.as_deref(), Some("M Plan Plus"));
        let quota = account.quota.as_ref().unwrap();
        assert_eq!((quota.unit.as_str(), quota.used, quota.limit, quota.remaining), ("percent", 25.0, 100.0, Some(88.5)));

        checkin::run(&paths.app_data_dir, &client(), Some(SourceKind::MinimaxCode), true).await.unwrap();
        let result = checkin_of(&paths, &account.id).unwrap();
        assert_eq!((result.outcome, result.credit, result.streak), (checkin::Outcome::Claimed, 60.0, 2));

        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::MinimaxCode).await.ok().unwrap();
        assert!(auth.serves(router::ModelProtocol::AnthropicMessages, "messages"));
        let header = |name: &str| auth.headers.iter().find(|(key, _)| *key == name).map(|(_, value)| value.clone());
        assert_eq!(header("authorization").as_deref(), Some("Bearer mm-at-1"));
        assert_eq!(header("x-api-key").as_deref(), Some("sk-xxx"));
        assert_eq!(header("user-agent").as_deref(), Some("MiniMaxAgent"));
        assert!(auth.base.ends_with("/mavis/api/v1/llm/v1"));
        set_endpoint_override(SourceKind::MinimaxCode, None);
    });
}

#[test]
fn command_code_takes_a_studio_approved_key_and_serves_go_plans_through_the_cli_endpoint() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let plan = Arc::new(Mutex::new("individual-pro-monthly"));
        let plan_now = plan.clone();
        let upstream = mock_raw(move |seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            let json_reply = |value: Value| (200, "application/json", value.to_string().into_bytes());
            match (seen.method.as_str(), path) {
                ("GET", "/alpha/whoami") => {
                    assert_eq!(seen.headers["authorization"], "Bearer cc-key-1");
                    json_reply(json!({"user": {"id": "cc-u1", "userName": "harbor", "email": "h@example.com"}}))
                }
                ("GET", "/alpha/billing/subscriptions") => json_reply(json!({"success": true, "data": {"planId": *plan_now.lock().unwrap(), "status": "active"}})),
                ("GET", "/alpha/billing/credits") => json_reply(json!({"credits": {"monthlyCredits": 12, "purchasedCredits": 0, "freeCredits": 0},
                    "windowLimits": {"fiveHour": {"used": 2, "cap": 10}, "weekly": {"used": 30, "cap": 100}}})),
                ("GET", "/provider/v1/models") => json_reply(json!({"data": [
                    {"id": "claude-sonnet-5", "context_length": 1000000, "supported_endpoints": ["/v1/messages"]},
                    {"id": "claude-opus-5-5", "supported_endpoints": ["/v1/messages"]},
                    {"id": "gpt-6-sol", "supported_endpoints": ["/v1/responses"]},
                    {"id": "zai-org/GLM-5.3", "display_name": "GLM-5.3", "supported_endpoints": ["/v1/chat/completions", "/v1/responses"]},
                    {"id": "flux-image-1"},
                ]})),
                ("POST", "/alpha/generate") => {
                    assert_eq!(seen.headers["x-command-code-version"], "1.72.2");
                    assert_eq!(seen.headers["user-agent"], "cli");
                    let body: Value = serde_json::from_str(&seen.body).unwrap();
                    assert_eq!(body["params"]["model"], "zai-org/GLM-5.3");
                    assert_eq!(body["params"]["messages"][0]["content"][0]["text"], "hi");
                    let lines = concat!(
                        r#"{"type":"text-delta","text":"hello"}"#, "\n",
                        r#"{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":3,"outputTokens":1}}"#, "\n",
                    );
                    (200, "application/x-ndjson", lines.as_bytes().to_vec())
                }
                other => panic!("unexpected {other:?}"),
            }
        }).await;
        set_endpoint_override(SourceKind::Commandcode, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let page = Arc::new(Mutex::new(String::new()));
        let opened = page.clone();
        let studio = async {
            let url = loop {
                let url = opened.lock().unwrap().clone();
                if !url.is_empty() {
                    break url;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            };
            let url = reqwest::Url::parse(&url).unwrap();
            assert_eq!(url.path(), "/studio/auth/cli");
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(query["mode"], "redirect");
            let http = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
            let preflight = http.request(reqwest::Method::OPTIONS, &query["callback"]).header("origin", "https://commandcode.ai")
                .header("access-control-request-private-network", "true").send().await.unwrap();
            assert_eq!(preflight.headers()["access-control-allow-private-network"], "true");
            let wrong = http.post(&query["callback"]).form(&[("state", "other"), ("apiKey", "cc-key-1")]).send().await.unwrap();
            assert_eq!(wrong.status(), 403, "a post from another sign-in is turned away");
            let posted = http.post(&query["callback"]).form(&[("state", query["state"].as_str()), ("apiKey", "cc-key-1"), ("userName", "harbor")])
                .send().await.unwrap();
            assert_eq!(posted.status(), 303);
            let done = http.get(format!("{}/complete", query["callback"])).send().await.unwrap().text().await.unwrap();
            assert!(done.contains("signed in"), "{done}");
        };
        let http = client();
        let signing = sign_in(&paths, &http, SourceKind::Commandcode, Some("browser"), None, cancel, move |prompt| {
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        });
        let (account, ()) = tokio::join!(signing, studio);
        let account = account.unwrap();
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("harbor", "cc-u1", Some("Pro")));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::Commandcode, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Commandcode).unwrap();
        assert_eq!(source.methods.iter().map(|method| method.id).collect::<Vec<_>>(), ["browser", "cli", "key"]);
        let ids: Vec<&str> = source.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["claude-sonnet-5", "gpt-6-sol", "zai-org/GLM-5.3"], "Pro is refused Opus; a drawing model is no chat model");
        assert_eq!(source.models[0].protocol, Some(router::ModelProtocol::AnthropicMessages));
        assert_eq!(source.models[1].protocol, Some(router::ModelProtocol::Responses));
        assert_eq!(source.models[2].protocol, None, "chat completions, the source's own API");
        let quota = source.accounts[0].quota.as_ref().unwrap();
        assert_eq!((quota.unit.as_str(), quota.used), ("percent", 30.0), "the tightest window");

        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::Commandcode).await.ok().unwrap();
        assert!(auth.base.ends_with("/provider/v1") && auth.headers.contains(&("x-api-key", "cc-key-1".into())));
        assert!(auth.serves(router::ModelProtocol::AnthropicMessages, "messages") && auth.serves(router::ModelProtocol::Responses, "responses"));

        // A Go plan, read on the next allowance refresh, is asked where its CLI asks.
        *plan.lock().unwrap() = "individual-go-monthly";
        let id = source.accounts[0].id.clone();
        refresh_quota(&paths, &client(), &id).await.unwrap();
        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::Commandcode).await.ok().unwrap();
        let chat = json!({"model": "zai-org/GLM-5.3", "stream": false, "messages": [{"role": "user", "content": "hi"}]});
        let encoded = auth.encode(router::ModelProtocol::ChatCompletions, chat.to_string().as_bytes(), "zai-org/GLM-5.3").unwrap().ok().unwrap();
        assert_eq!(encoded.path, "alpha/generate");
        let mut request = client().post(format!("{}/{}", auth.base, encoded.path)).body(encoded.body);
        for (name, value) in auth.headers.iter().chain(encoded.headers.iter()) {
            request = request.header(*name, value);
        }
        let reply = encoded.decoder.decode(request.send().await.unwrap()).await;
        let body: Value = reply.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
        assert_eq!(body["usage"]["prompt_tokens"], 3);
        set_endpoint_override(SourceKind::Commandcode, None);
    });
}

fn zcode_jwt() -> String {
    use base64::Engine;
    let claims = json!({"exp": chrono::Utc::now().timestamp() + 3600}).to_string();
    format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims))
}

#[test]
fn zcode_signs_in_through_zcode_s_flow_and_sends_trial_entries_to_the_gift() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    let home = tempfile::tempdir().unwrap();
    *zcode::TEST_HOME.lock().unwrap() = Some(home.path().to_path_buf());
    runtime().block_on(async {
        let jwt = zcode_jwt();
        let polls = Arc::new(AtomicUsize::new(0));
        let polled = polls.clone();
        let session = jwt.clone();
        let upstream = mock(move |seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            let ok = |data: Value| (200, json!({"code": 0, "msg": "ok", "data": data}).to_string());
            match (seen.method.as_str(), path) {
                ("POST", "/api/v1/oauth/cli/init") => {
                    assert!(seen.headers.contains_key("x-device-mid"));
                    assert_eq!(seen.body, r#"{"provider":"zai"}"#);
                    ok(json!({"flow_id": "flow-1", "authorize_url": "https://chat.z.ai/oauth?x=1", "poll_interval_sec": 1}))
                }
                ("GET", "/api/v1/oauth/cli/poll/flow-1") => if polled.fetch_add(1, Ordering::SeqCst) == 0 {
                    ok(json!({"status": "pending"}))
                } else {
                    ok(json!({"status": "ready", "zai": {"access_token": "zai-token"}, "token": session, "user": {"email": "dev@z.example"}}))
                },
                ("POST", "/api/auth/z/login") => ok(json!({"access_token": "biz-1"})),
                ("GET", "/api/biz/customer/getCustomerInfo") => {
                    assert_eq!(seen.headers["authorization"], "Bearer biz-1");
                    ok(json!({"organizations": [{"organizationId": "o1", "organizationName": "默认机构",
                        "projects": [{"projectId": "p1", "projectName": "默认项目", "projectType": 1}]}]}))
                }
                ("GET", "/api/biz/v1/organization/o1/projects/p1/api_keys") => ok(json!([])),
                ("POST", "/api/biz/v1/organization/o1/projects/p1/api_keys") => {
                    assert_eq!(seen.body, r#"{"name":"zcode-api-key"}"#);
                    ok(json!({"apiKey": "kid"}))
                }
                ("GET", "/api/biz/v1/organization/o1/projects/p1/api_keys/copy/kid") => ok(json!({"secretKey": "sec"})),
                ("GET", "/api/biz/subscription/list") => {
                    assert_eq!(seen.headers["authorization"], "kid.sec");
                    ok(json!([{"status": "VALID", "productName": "GLM Coding Pro"}]))
                }
                ("GET", "/api/v1/zcode-plan/billing/balance") => ok(json!({"plans": [{"status": "active", "name": "ZCode Trust Build", "user_plan_id": "up1"}],
                    "balances": [{"user_plan_id": "up1", "capabilities": ["model:glm-5.3-flash"], "total_units": 100, "remaining_units": 40}]})),
                ("GET", "/api/v1/client/configs") => (404, "{}".into()),
                ("GET", "/api/monitor/usage/quota/limit") => ok(json!({"level": "pro", "limits": [
                    {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "usage": 100, "remaining": 70}, {"type": "TIME_LIMIT", "percentage": 90}]})),
                other => panic!("unexpected {other:?}"),
            }
        }).await;
        set_endpoint_override(SourceKind::Zcode, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let account = sign_in(&paths, &client(), SourceKind::Zcode, Some("zai"), None, cancel, |prompt| {
            assert!(prompt.url.contains("redirect_uri="), "{}", prompt.url);
            Ok(())
        }).await.unwrap();
        assert_eq!((account.name.as_str(), account.plan.as_deref()), ("dev@z.example", Some("GLM Coding Pro")));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::Zcode, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::Zcode).unwrap();
        assert!(source.models.iter().any(|model| model.id == "GLM-5.3"), "the coding plan's models");
        assert!(source.models.iter().any(|model| model.id == "GLM-5.3-Flash-Trial"), "the gift's trial entry");
        let quota = source.accounts[0].quota.as_ref().unwrap();
        assert_eq!((quota.used, quota.limit), (30.0, 100.0), "MCP calls are set aside");
        assert_eq!(source.accounts[0].plan.as_deref(), Some("GLM Coding Pro"));

        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::Zcode).await.ok().unwrap();
        assert!(auth.base.ends_with("/api/anthropic"));
        assert!(auth.headers.contains(&("x-api-key", "kid.sec".into())));
        assert_eq!(auth.path("messages"), "v1/messages");
        let plain = json!({"model": "GLM-5.3", "messages": [{"role": "user", "content": "hi"}]});
        assert!(auth.reroute(router::ModelProtocol::AnthropicMessages, plain.to_string().as_bytes()).is_none(), "a plain model spends the coding plan");
        assert!(auth.shape_body(router::ModelProtocol::AnthropicMessages, plain.to_string().as_bytes()).is_none());
        let trial = json!({"model": "GLM-5.3-Flash-Trial", "system": "agent", "messages": [{"role": "user", "content": "hi"}]});
        let rerouted = auth.reroute(router::ModelProtocol::AnthropicMessages, trial.to_string().as_bytes()).unwrap().ok().unwrap();
        assert!(rerouted.base.ends_with("/api/v1/zcode-plan/anthropic"));
        assert!(rerouted.headers.contains(&("authorization", format!("Bearer {jwt}"))));
        let body: Value = serde_json::from_slice(&rerouted.body).unwrap();
        assert_eq!(body["model"], "glm-5.3-flash");
        assert_eq!(body["system"][0]["text"], "You are ZCode, an interactive coding agent");
        let unknown = json!({"model": "GLM-9-Trial", "messages": []});
        assert_eq!(auth.reroute(router::ModelProtocol::AnthropicMessages, unknown.to_string().as_bytes()).unwrap().err().unwrap().0, hyper::StatusCode::BAD_REQUEST);
        set_endpoint_override(SourceKind::Zcode, None);
    });
    *zcode::TEST_HOME.lock().unwrap() = None;
}

#[test]
fn zcode_follows_the_app_s_encrypted_sign_in() {
    use base64::Engine;
    let _guard = crate::core::router_proxy::global_router_test_lock();
    let home = tempfile::tempdir().unwrap();
    *zcode::TEST_HOME.lock().unwrap() = Some(home.path().to_path_buf());
    std::env::set_var("ZCODE_CREDENTIAL_SECRET", "test-seed");
    let key: [u8; 32] = { use sha2::Digest; sha2::Sha256::digest(b"test-seed").into() };
    let seal = |text: &str| {
        let sealing = ring::aead::LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key).unwrap());
        let iv = [9u8; 12];
        let mut data = text.as_bytes().to_vec();
        let tag = sealing.seal_in_place_separate_tag(ring::aead::Nonce::assume_unique_for_key(iv), ring::aead::Aad::empty(), &mut data).unwrap();
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        format!("enc:v1:{}.{}.{}", b64(&iv), b64(tag.as_ref()), b64(&data))
    };
    let dir = home.path().join(".zcode").join("v2");
    std::fs::create_dir_all(&dir).unwrap();
    let jwt = zcode_jwt();
    std::fs::write(dir.join("credentials.json"), json!({
        "account-provider:coding-plan:account:zai-individual-coding-plan:account:u1:api-key": seal("app-key.app-secret"),
        "zcodejwttoken": seal(&format!("Bearer {jwt}")),
        "oauth:zai:user_info": seal(r#"{"email":"app@z.example"}"#),
    }).to_string()).unwrap();
    let own = zcode::own_sign_in().unwrap();
    assert_eq!((own.key.as_str(), own.jwt.as_str(), own.user.as_str(), own.site.as_str()), ("app-key.app-secret", jwt.as_str(), "app@z.example", "zai"));
    assert!(own.from_app);
    // signed in to another account since: the kept sign-in stands
    let kept = zcode::Creds { user: "someone@else".into(), key: "old".into(), from_app: true, ..Default::default() };
    assert_eq!(kept.current().key, "old");
    let same = zcode::Creds { user: "app@z.example".into(), key: "old".into(), from_app: true, ..Default::default() };
    assert_eq!(same.current().key, "app-key.app-secret", "the app's renewed sign-in is followed");
    std::env::remove_var("ZCODE_CREDENTIAL_SECRET");
    *zcode::TEST_HOME.lock().unwrap() = None;
}

#[test]
fn opencode_imports_the_cli_s_key_and_lists_each_model_on_its_api() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("opencode")).unwrap();
    std::fs::write(data.path().join("opencode").join("auth.json"), json!({
        "opencode-go": {"type": "api", "key": "sk-go-1234"}, "anthropic": {"type": "oauth"}}).to_string()).unwrap();
    std::env::set_var("XDG_DATA_HOME", data.path());
    runtime().block_on(async {
        let upstream = mock(|seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            match path {
                "/zen/go/v1/models" => {
                    assert_eq!(seen.headers["authorization"], "Bearer sk-go-1234");
                    (200, json!({"data": [{"id": "glm-5.3"}, {"id": "minimax-m3"}, {"id": "gpt-6-luna"}, {"id": "gemini-3-pro"}]}).to_string())
                }
                "/zen/go/v1/usage" => (200, json!({"usage": {"rolling": {"percent": 12}, "weekly": {"percent": 40}, "monthly": {"percent": "7"}}}).to_string()),
                "/inference/v1/models" => {
                    assert_eq!(seen.headers["authorization"], "Bearer public");
                    (200, json!({"data": [{"id": "big-pickle"}, {"id": "gpt-5.5"}, {"id": "glm-5-free"}, {"id": "jev-1.13-free"}]}).to_string())
                }
                "/docs/models.mdx" => (200, "| Big Pickle | big-pickle | `https://opencode.ai/zen/v1/chat/completions` |".into()),
                other => panic!("unexpected {other}"),
            }
        }).await;
        set_endpoint_override(SourceKind::OpencodeGo, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let account = sign_in(&paths, &client(), SourceKind::OpencodeGo, Some("import"), None, cancel.clone(), |_| Ok(())).await.unwrap();
        assert_eq!(account.name, "Go …1234");
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::OpencodeGo, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let go = listed.sources.iter().find(|source| source.kind == SourceKind::OpencodeGo).unwrap();
        let apis: Vec<(&str, Option<router::ModelProtocol>)> = go.models.iter().map(|model| (model.id.as_str(), model.protocol)).collect();
        assert_eq!(apis, [("glm-5.3", None), ("minimax-m3", Some(router::ModelProtocol::AnthropicMessages)),
            ("gpt-6-luna", Some(router::ModelProtocol::Responses))], "Gemini's own API isn't one the router speaks");
        assert_eq!(go.accounts[0].quota.as_ref().unwrap().used, 40.0);
        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::OpencodeGo).await.ok().unwrap();
        assert!(!auth.exact && auth.base.ends_with("/zen/go/v1"));
        let headers = auth.body_headers(router::ModelProtocol::AnthropicMessages, br#"{"messages":[{"role":"user","content":"hi"}]}"#);
        assert!(headers.iter().any(|(name, value)| *name == "user-agent" && value.contains("provider-utils/4.0.46")));
        set_endpoint_override(SourceKind::OpencodeGo, None);

        set_endpoint_override(SourceKind::OpencodeZen, Some(upstream.base.clone()));
        let account = sign_in(&paths, &client(), SourceKind::OpencodeZen, Some("free"), None, cancel, |_| Ok(())).await.unwrap();
        assert_eq!((account.name.as_str(), account.plan.as_deref()), ("Zen Free", Some("Free")));
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::OpencodeZen, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let zen = listed.sources.iter().find(|source| source.kind == SourceKind::OpencodeZen).unwrap();
        let ids: Vec<&str> = zen.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["big-pickle", "glm-5-free"], "only free models; a SystemOne model isn't one to chat with");
        assert!(!zen.has_quota);
        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::OpencodeZen).await.ok().unwrap();
        assert!(auth.exact && auth.base.ends_with("/inference/v1"));
        set_endpoint_override(SourceKind::OpencodeZen, None);
    });
    std::env::remove_var("XDG_DATA_HOME");
}

#[test]
fn qoder_cn_signs_in_by_device_chats_through_the_envelope_and_claims_its_credits() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let polls = Arc::new(AtomicUsize::new(0));
        let polled = polls.clone();
        let upstream = mock_raw(move |seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            let reply = |value: Value| (200, "application/json", value.to_string().into_bytes());
            match (seen.method.as_str(), path) {
                ("GET", "/api/v1/deviceToken/poll") => {
                    assert!(seen.path.contains("challenge_method=S256") && seen.path.contains("verifier="));
                    if polled.fetch_add(1, Ordering::SeqCst) == 0 {
                        (404, "application/json", b"{}".to_vec())
                    } else {
                        reply(json!({"token": "dt-1", "refresh_token": "dr-1", "user_id": "u-9", "expires_in": 7200}))
                    }
                }
                // qoder.cn won't trade the device token: chat on it, as its CLI does
                ("POST", "/api/v1/me/jobToken") => (403, "application/json", b"{}".to_vec()),
                ("GET", "/api/v1/userinfo") => reply(json!({"email": "cn@example.com", "name": "CN"})),
                ("GET", "/algo/api/v2/model/list") => {
                    assert!(seen.headers["authorization"].starts_with("Bearer COSY."), "the listing is signed");
                    assert_eq!(seen.headers["cosy-user"], "u-9");
                    reply(json!({"app": [{"key": "auto", "enable": true}, {"key": "efficient", "enable": true, "display_name": "Efficient",
                        "max_input_tokens": 200000}]}))
                }
                ("POST", "/algo/api/v2/service/pro/sse/agent_chat_generation") => {
                    assert_eq!(seen.headers["x-model-key"], "efficient");
                    assert_eq!(seen.headers["accept"], "text/event-stream");
                    let chunk = |delta: Value| format!("data: {}\n\n", json!({"statusCodeValue": 200,
                        "body": json!({"choices": [{"delta": delta}]}).to_string()}));
                    let sse = [chunk(json!({"reasoning_content": "hm"})), chunk(json!({"content": "hello"})),
                        format!("data: {}\n\n", json!({"body": json!({"choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 2}}).to_string()})),
                        format!("data: {}\n\n", json!({"body": "[DONE]"}))].concat();
                    (200, "text/event-stream", sse.into_bytes())
                }
                ("GET", "/sash/api/v2/me/usage") => {
                    assert_eq!(seen.headers["authorization"], "Bearer dt-1");
                    reply(json!({"displayMode": "qoder", "qoderUsage": {"userType": "Pro", "userQuota": {"total": 1000, "used": 250},
                        "addOnQuota": {"total": 100, "remaining": 100}}}))
                }
                ("GET", "/sash/api/v1/me/campaigns") => {
                    assert_eq!(seen.headers["cosy-clienttype"], "10");
                    let now = chrono::Utc::now().timestamp();
                    reply(json!({"campaigns": [{"campaignId": "c-1", "actionType": "CLAIM_BENEFIT", "claimStatus": "CLAIMABLE",
                        "startAt": (now - 60) * 1000, "endAt": (now + 3600) * 1000, "benefit": {"amount": 100}}]}))
                }
                ("POST", "/sash/api/v1/me/campaigns/c-1/claim") => reply(json!({"data": {"status": "CLAIMED", "benefit": {"amount": 100}}})),
                other => panic!("unexpected {other:?}"),
            }
        }).await;
        set_endpoint_override(SourceKind::QoderCn, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let account = sign_in(&paths, &client(), SourceKind::QoderCn, None, None, cancel, |prompt| {
            assert!(prompt.url.contains("/device/selectAccounts?") && !prompt.url.contains("redirect_uri"), "{}", prompt.url);
            Ok(())
        }).await.unwrap();
        assert_eq!(account.name, "cn@example.com");
        let Credentials::Qoder(creds) = &account.credentials else { panic!("Qoder credentials") };
        assert!(creds.device_chat && creds.access == "dt-1", "the device token is the chat token");
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::QoderCn, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::QoderCn).unwrap();
        assert_eq!(source.models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["efficient"]);
        let quota = source.accounts[0].quota.as_ref().unwrap();
        assert_eq!((quota.used, quota.limit), (250.0, 1100.0));
        assert_eq!(source.accounts[0].plan.as_deref(), Some("Pro"));
        assert!(source.checks_in);

        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::QoderCn).await.ok().unwrap();
        let chat = json!({"model": "efficient", "stream": false, "messages": [{"role": "user", "content": "hi"}]});
        let encoded = auth.encode(router::ModelProtocol::ChatCompletions, chat.to_string().as_bytes(), "efficient").unwrap().ok().unwrap();
        let mut request = client().post(format!("{}/{}", auth.base, encoded.path)).body(encoded.body);
        for (name, value) in &encoded.headers {
            request = request.header(*name, value);
        }
        let body: Value = encoded.decoder.decode(request.send().await.unwrap()).await.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
        assert_eq!(body["choices"][0]["message"]["reasoning_content"], "hm");
        assert_eq!(body["usage"]["prompt_tokens"], 7);
        let unknown = json!({"model": "nope", "messages": []});
        assert!(auth.encode(router::ModelProtocol::ChatCompletions, unknown.to_string().as_bytes(), "nope").unwrap().is_err());

        checkin::run(&paths.app_data_dir, &client(), Some(SourceKind::QoderCn), true).await.unwrap();
        let result = checkin_of(&paths, &source.accounts[0].id).unwrap();
        assert_eq!((result.outcome, result.credit), (checkin::Outcome::Claimed, 100.0));
        assert!(result.valid_until.is_some(), "a claimed campaign holds until its window ends");
        set_endpoint_override(SourceKind::QoderCn, None);
    });
}

#[test]
fn trae_cn_signs_in_by_callback_binds_its_device_and_answers_through_the_functions() {
    let _guard = crate::core::router_proxy::global_router_test_lock();
    runtime().block_on(async {
        let upstream = mock_raw(|seen| {
            let path = seen.path.split('?').next().unwrap_or_default();
            let reply = |value: Value| (200, "application/json", value.to_string().into_bytes());
            let body: Value = serde_json::from_str(&seen.body).unwrap_or(Value::Null);
            match path {
                "/trae/api/v3/oauth/ExchangeToken" => {
                    assert_eq!(body["ClientID"], "en1oxy7wnw8j9n", "tried as TRAE SOLO CN first");
                    assert!(body["DeviceProof"]["Signature"].is_string() && body["DeviceInfo"]["DevicePublicKey"].as_str().unwrap().contains("BEGIN PUBLIC KEY"));
                    reply(json!({"Result": {"Token": "bound-jwt", "RefreshToken": "rt-2", "TokenExpireAt": chrono::Utc::now().timestamp() + 3600}}))
                }
                "/api/ide/v1/batch_get_detail_param" => {
                    assert_eq!(seen.headers["authorization"], "Cloud-IDE-JWT bound-jwt");
                    reply(json!({"function_configs": [
                        {"function": "chat_v3", "config_info_list": [{"config_name": "glm-5.2", "display_name": "GLM-5.2", "context_window_tokens": {"dev": 200000}}]},
                        {"function": "solo_agent", "config_info_list": [{"config_name": "glm-5.2", "display_name": "GLM-5.2",
                            "model_detail_list": [{"model_name": "glm-5.2__dev", "max_tokens": 32000}], "context_window_tokens": {"dev": 200000}},
                            {"config_name": "summary", "usage": "summary"}]},
                    ]}))
                }
                "/api/agent/v3/llm_utils_chat" if body["function"] == "solo_agent" => {
                    assert_eq!(body["model_name"], "glm-5.2__dev");
                    (400, "application/json", json!({"code": 4023, "message": "function not available"}).to_string().into_bytes())
                }
                "/api/agent/v3/llm_utils_chat" => {
                    assert_eq!(body["function"], "chat_v3", "a refusing function moves on to the next");
                    let sse = [
                        format!("event: output\ndata: {}\n\n", json!({"response": "hi "})),
                        format!("event: output\ndata: {}\n\n", json!({"response": "<tool_call>{\"name\":\"ls\",\"arguments\":{}}</tool_call>"})),
                        "event: token_usage\ndata: {\"prompt_tokens\":9,\"completion_tokens\":4}\n\n".to_owned(),
                        "event: done\ndata: {}\n\n".to_owned(),
                    ].concat();
                    (200, "text/event-stream", sse.into_bytes())
                }
                "/trae/api/v2/pay/ide_user_ent_usage" => reply(json!({"is_credits_billing": true, "user_entitlement_pack_list": [
                    {"entitlement_base_info": {"quota": {"credits_limit": 1000}}, "usage": {"credits_amount": 150}}]})),
                "/trae/api/v2/ug/checkin_credits/status" => {
                    assert!(seen.headers["user-agent"].contains("TRAE-SOLO-CN/"), "a growth page goes as the IDE's shell");
                    assert_eq!(body, json!({"req_source": 1}));
                    reply(json!({"code": 0, "enable": true, "checked_in": false, "credits": 100}))
                }
                "/trae/api/v2/ug/checkin_credits/claim" => reply(json!({"code": 9074, "message": "当前参与用户太多，请稍后再试"})),
                other => panic!("unexpected {other}"),
            }
        }).await;
        set_endpoint_override(SourceKind::TraeCn, Some(upstream.base.clone()));
        let (_dir, paths) = temp_paths();
        let (_cancel, cancel) = watch::channel(false);
        let page = Arc::new(Mutex::new(String::new()));
        let opened = page.clone();
        let browser = async {
            let url = loop {
                let url = opened.lock().unwrap().clone();
                if !url.is_empty() {
                    break url;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            };
            let url = reqwest::Url::parse(&url).unwrap();
            assert_eq!(url.path(), "/authorization");
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(query["device_id"].len(), 19);
            let mut back = reqwest::Url::parse(&query["auth_callback_url"]).unwrap();
            back.query_pairs_mut()
                .append_pair("userJwt", &json!({"Token": "page-jwt", "RefreshToken": "rt-1", "ClientID": "ono9krqynydwx5"}).to_string())
                .append_pair("userInfo", &json!({"UserID": "7001", "ScreenName": "trae-user"}).to_string());
            let page = reqwest::Client::builder().no_proxy().build().unwrap().get(back).send().await.unwrap().text().await.unwrap();
            assert!(page.contains("signed in"), "{page}");
        };
        let http = client();
        let signing = sign_in(&paths, &http, SourceKind::TraeCn, None, None, cancel, move |prompt| {
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        });
        let (account, ()) = tokio::join!(signing, browser);
        let account = account.unwrap();
        assert_eq!((account.name.as_str(), account.subject.as_str()), ("trae-user", "7001"));
        let Credentials::Trae(creds) = &account.credentials else { panic!("Trae credentials") };
        assert_eq!((creds.token.as_str(), creds.refresh.as_str()), ("bound-jwt", "rt-2"), "the page's token is renewed for this device at once");
        let first = keep(&paths, account).unwrap();
        after_sign_in(&paths, &client(), SourceKind::TraeCn, first).await.unwrap();
        let listed = list(&paths).unwrap();
        let source = listed.sources.iter().find(|source| source.kind == SourceKind::TraeCn).unwrap();
        assert_eq!(source.models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["glm-5.2"], "an IDE helper is no chat model");
        let quota = source.accounts[0].quota.as_ref().unwrap();
        assert_eq!((quota.used, quota.limit, source.accounts[0].plan.as_deref()), (150.0, 1000.0, Some("Credits")));

        let auth = credentials(&paths.app_data_dir, &client(), SourceKind::TraeCn).await.ok().unwrap();
        let chat = json!({"model": "glm-5.2", "stream": false, "messages": [{"role": "user", "content": "list"}],
            "tools": [{"type": "function", "function": {"name": "ls", "parameters": {"type": "object"}}}]});
        let exchange = auth.exchange(router::ModelProtocol::ChatCompletions, chat.to_string().as_bytes(), "glm-5.2").unwrap().ok().unwrap();
        let reply: Value = exchange.run().await.json().await.unwrap();
        assert_eq!(reply["choices"][0]["message"]["content"], "hi ");
        assert_eq!(reply["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "ls");
        assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(reply["usage"]["prompt_tokens"], 9);

        checkin::run(&paths.app_data_dir, &client(), Some(SourceKind::TraeCn), true).await.unwrap();
        let result = checkin_of(&paths, &source.accounts[0].id).unwrap();
        assert_eq!(result.outcome, checkin::Outcome::OwnApp, "Trae pays a claim only to its own app");
        set_endpoint_override(SourceKind::TraeCn, None);
    });
}
