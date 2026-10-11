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

/// A loopback HTTP server answering each request with `respond`.
pub(crate) async fn mock(respond: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static) -> Mock {
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
                        let (status, reply) = respond(&seen);
                        record.lock().unwrap().push(seen);
                        Ok::<_, std::convert::Infallible>(Response::builder().status(status)
                            .header("content-type", "application/json").body(Full::new(Bytes::from(reply))).unwrap())
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
        let account = sign_in(&paths, &client(), SourceKind::Workbuddy, cancel, move |url| {
            *page.lock().unwrap() = url.to_owned();
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
        let account = sign_in(&paths, &client(), SourceKind::ChatgptApi, cancel, move |url| {
            let url = reqwest::Url::parse(url).unwrap();
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
        let result = sign_in(&paths, &client(), SourceKind::ChatgptApi, cancel, |url| {
            let url = reqwest::Url::parse(url).unwrap();
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
