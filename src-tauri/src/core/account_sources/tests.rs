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
        active: false, credentials, added_at: 1, lapsed: None, quota: None,
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
        let account = sign_in(&paths, &client(), SourceKind::Workbuddy, cancel, move |prompt| {
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
        let attempt = sign_in(&paths, &http, SourceKind::Workbuddy, cancel, |_| Ok(()));
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
        let account = sign_in(&paths, &client(), SourceKind::ChatgptApi, cancel, move |prompt| {
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
        let result = sign_in(&paths, &client(), SourceKind::ChatgptApi, cancel, |prompt| {
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
        let account = sign_in(&paths, &client(), SourceKind::Copilot, cancel, move |prompt| {
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
        let account = sign_in(&paths, &client(), SourceKind::Antigravity, cancel, |prompt| {
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
        let account = sign_in(&paths, &client(), SourceKind::Kiro, cancel, |prompt| {
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
        let account = sign_in(&paths, &client(), SourceKind::Grok, cancel, move |prompt| {
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
        let account = sign_in(&paths, &client(), SourceKind::Cursor, cancel.clone(), move |prompt| {
            *page.lock().unwrap() = prompt.url.to_owned();
            Ok(())
        }).await.unwrap();
        assert_eq!(opened.lock().unwrap().as_str(), "https://cursor.com/loginDeepControl?challenge=abc&uuid=1b4e28ba-2fa1-11d2-883f-0016d3cca427&mode=login&redirectTarget=cli&supportsSelectedTeamLogin=true",
            "the link the CLI wrapped is handed on whole");
        assert_eq!((account.name.as_str(), account.subject.as_str(), account.plan.as_deref()), ("Me@Cursor.sh", "me@cursor.sh", Some("Pro")));
        keep(&paths, account).unwrap();

        // Signed in already: kept as it is, without a page to open.
        *cursor::TOKEN.lock().unwrap() = Some("tok".into());
        let again = sign_in(&paths, &client(), SourceKind::Cursor, cancel, |_| panic!("no page for a CLI signed in already")).await.unwrap();
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
