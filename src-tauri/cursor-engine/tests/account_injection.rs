//! Local account bootstrap, subscription checks and model discovery through the real front.
use cursor_engine::{
    cursor::services::account::local_token,
    model::{ModelConfig, ModelConfigInput},
    Engine, EngineConfig, Host,
};
use serde_json::json;

#[tokio::test]
async fn local_account_discovers_injected_models_and_passes_plan_checks_over_https() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::new();
    let input: ModelConfigInput = serde_json::from_value(json!({
        "display_name": "Injected Model",
        "type": "openai",
        "base_url": "http://127.0.0.1:1",
        "api_key": "local-provider-key",
        "tooltip_data": "test",
        "model_id": "test-model"
    }))
    .unwrap();
    let model = ModelConfig::from_input(&input).unwrap();
    let model_id = model.model_hash.clone();
    host.set_models(vec![model]);
    let engine = Engine::start(
        EngineConfig {
            data_dir: directory.path().to_owned(),
            port: 0,
        },
        host,
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(engine.url()).unwrap())
        .add_root_certificate(
            reqwest::Certificate::from_pem(engine.authority().certificate_pem().as_bytes())
                .unwrap(),
        )
        .build()
        .unwrap();
    let auth = format!("Bearer {}", local_token());
    for path in [
        "/aiserver.v1.AuthService/GetEmail",
        "/aiserver.v1.AuthService/GetUserMeta",
        "/aiserver.v1.DashboardService/GetMe",
        "/aiserver.v1.DashboardService/GetTeams",
        "/aiserver.v1.DashboardService/GetUserProfile",
        "/aiserver.v1.DashboardService/GetCurrentPeriodUsage",
        "/aiserver.v1.DashboardService/GetUsageLimitStatusAndActiveGrants",
        "/aiserver.v1.AiService/GetServerConfig",
        "/aiserver.v1.ServerConfigService/GetServerConfig",
        "/aiserver.v1.AiService/AvailableModels",
        "/agent.v1.AgentService/GetUsableModels",
        "/aiserver.v1.AiService/GetUsableModels",
        "/aiserver.v1.AiService/GetDefaultModelNudgeData",
    ] {
        let response = client
            .post(format!("https://api2.cursor.sh{path}"))
            .header("authorization", &auth)
            .header("content-type", "application/proto")
            .body(Vec::new())
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{path}: {}",
            response.status()
        );
        assert_eq!(response.headers()["content-type"], "application/proto");
        let body = response.bytes().await.unwrap();
        if path.ends_with("AvailableModels") || path.ends_with("GetUsableModels") {
            assert!(
                body.windows(model_id.len())
                    .any(|bytes| bytes == model_id.as_bytes()),
                "{path} includes the injected model"
            );
        }
    }
    for origin in ["api2.cursor.sh", "cursor.com"] {
        for path in ["/auth/full_stripe_profile", "/auth/stripe_profile"] {
            let response = client
                .get(format!("https://{origin}{path}"))
                .header("authorization", &auth)
                .header("origin", "vscode-file://vscode-app")
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            assert_eq!(
                response.headers()["access-control-allow-origin"],
                "vscode-file://vscode-app"
            );
            let profile: serde_json::Value = response.json().await.unwrap();
            assert_eq!(profile["membershipType"], "ultra");
            assert_eq!(profile["subscriptionStatus"], "active");
        }
    }
    engine.stop().await;
}
