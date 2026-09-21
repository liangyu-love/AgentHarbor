use super::{models::ApiProxyConfigPayload, newapi};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountPlatform {
    NewApi,
    Sub2Api,
    Unknown,
}

/// Query a balance without mixing credential protocols. Official hosts are
/// handled explicitly. Custom domains are identified through unauthenticated
/// public metadata before an account access token is sent.
pub fn query(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<(newapi::Balance, &'static str), String> {
    let host = host(connection)?;

    if host == "api.deepseek.com" {
        if connection.api_key.is_empty() {
            return Err("newapi.errors.apiKeyRequired".into());
        }
        return deepseek_balance(connection, proxy).map(|balance| (balance, "deepseek"));
    }

    // These are first-party endpoints with no documented API-key balance
    // endpoint. Never probe them with NewAPI/Sub2API account credentials.
    if is_known_official_host(&host) {
        return Err("newapi.errors.balanceUnsupported".into());
    }

    if connection.access_token.is_empty() {
        return Err("newapi.errors.accessTokenRequired".into());
    }

    match detect_account_platform(connection, proxy)? {
        AccountPlatform::NewApi => {
            if connection.user_id.is_none() {
                return Err("newapi.errors.userId".into());
            }
            newapi::balance(connection, proxy).map(|balance| (balance, "newapi"))
        }
        AccountPlatform::Sub2Api => {
            sub2api_balance(connection, proxy).map(|balance| (balance, "sub2api"))
        }
        AccountPlatform::Unknown => Err("newapi.errors.balanceDetect".into()),
    }
}

fn host(connection: &newapi::Connection) -> Result<String, String> {
    Ok(newapi::base_url(&connection.base_url)?
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase())
}

fn is_known_official_host(host: &str) -> bool {
    matches!(
        host,
        "api.openai.com"
            | "api.anthropic.com"
            | "generativelanguage.googleapis.com"
            | "api.cohere.com"
            | "api.mistral.ai"
            | "api.x.ai"
            | "opencode.ai"
            | "api.commandcode.ai"
            | "token-plan-cn.xiaomimimo.com"
    )
}

fn detect_account_platform(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<AccountPlatform, String> {
    let client = newapi::client(proxy)?;
    let root = newapi::base_url(&connection.base_url)?;

    let newapi_status = root
        .join("api/status")
        .ok()
        .and_then(|url| newapi::get(&client, url, HeaderMap::new()).ok());
    if newapi_status.as_ref().is_some_and(is_newapi_public_status) {
        return Ok(AccountPlatform::NewApi);
    }

    let sub2api_settings = root
        .join("api/v1/settings/public")
        .ok()
        .and_then(|url| newapi::get(&client, url, HeaderMap::new()).ok());
    if sub2api_settings.as_ref().is_some_and(is_sub2api_public_settings) {
        return Ok(AccountPlatform::Sub2Api);
    }

    Ok(AccountPlatform::Unknown)
}

fn is_newapi_public_status(value: &Value) -> bool {
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return false;
    };
    value.get("success").and_then(Value::as_bool) == Some(true)
        && [
            "system_name",
            "version",
            "start_time",
            "quota_per_unit",
            "quota_display_type",
            "display_in_currency",
        ]
        .iter()
        .any(|key| data.contains_key(*key))
}

fn is_sub2api_public_settings(value: &Value) -> bool {
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return false;
    };
    value.get("code").and_then(Value::as_i64) == Some(0)
        && data.get("site_name").and_then(Value::as_str).is_some()
        && data.get("registration_enabled").and_then(Value::as_bool).is_some()
}

fn sub2api_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, newapi::auth_header(&connection.access_token)?);
    let url = newapi::base_url(&connection.base_url)?
        .join("api/v1/auth/me")
        .map_err(|_| "newapi.errors.url".to_owned())?;
    let value = newapi::get(&client, url, headers)?;
    parse_sub2api_balance(&value, connection.user_id)
}

fn parse_sub2api_balance(value: &Value, expected_user_id: Option<u64>) -> Result<newapi::Balance, String> {
    if value.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("newapi.errors.rejected".into());
    }
    let data = value.get("data").and_then(Value::as_object).ok_or("newapi.errors.response")?;
    if let Some(expected) = expected_user_id {
        let actual = data.get("id").and_then(Value::as_u64).ok_or("newapi.errors.response")?;
        if actual != expected {
            return Err("newapi.errors.userMismatch".into());
        }
    }
    let remaining = parse_number(data.get("balance")).ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        remaining,
        used: None,
        unit: "USD".into(),
        raw_quota: remaining,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
    })
}

fn parse_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse::<f64>().ok()))
        })
        .filter(|value| value.is_finite())
}

fn deepseek_balance(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, newapi::auth_header(&connection.api_key)?);
    let url = newapi::base_url(&connection.base_url)?
        .join("api/user/balance")
        .map_err(|_| "newapi.errors.url".to_owned())?;
    let value = newapi::get(&client, url, headers)?;
    parse_deepseek_balance(&value)
}

fn parse_deepseek_balance(value: &Value) -> Result<newapi::Balance, String> {
    let info = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or("newapi.errors.response")?;
    let total = parse_number(info.get("total_balance")).ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        remaining: total,
        used: None,
        unit: info
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        raw_quota: total,
        quota_per_unit: None,
        request_count: None,
        checked_at: chrono::Utc::now().timestamp(),
        conversion_available: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        sync::{Arc, Mutex},
        time::Duration,
    };

    fn mock_server(responses: Vec<(&'static str, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut bytes = [0u8; 16 * 1024];
                let count = stream.read(&mut bytes).unwrap();
                captured.lock().unwrap().push(String::from_utf8_lossy(&bytes[..count]).to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}/"), requests, handle)
    }

    #[test]
    fn identifies_newapi_and_sub2api_public_metadata() {
        assert!(is_newapi_public_status(&json!({
            "success": true,
            "data": {"system_name": "Relay", "quota_per_unit": 500000}
        })));
        assert!(!is_newapi_public_status(&json!({"success": true, "data": {}})));

        assert!(is_sub2api_public_settings(&json!({
            "code": 0,
            "data": {"site_name": "Sub2API", "registration_enabled": true}
        })));
        assert!(!is_sub2api_public_settings(&json!({
            "code": 0,
            "data": {"site_name": "unrelated"}
        })));
    }

    #[test]
    fn parses_sub2api_balance_and_checks_optional_user_id() {
        let value = json!({"code": 0, "data": {"id": 37, "balance": "12.50"}});
        let balance = parse_sub2api_balance(&value, Some(37)).unwrap();
        assert_eq!(balance.remaining, 12.5);
        assert_eq!(balance.unit, "USD");
        assert_eq!(
            parse_sub2api_balance(&value, Some(38)).unwrap_err(),
            "newapi.errors.userMismatch"
        );
    }

    #[test]
    fn parses_deepseek_balance_without_newapi_fields() {
        let value = json!({
            "is_available": true,
            "balance_infos": [{"currency": "CNY", "total_balance": "23.75"}]
        });
        let balance = parse_deepseek_balance(&value).unwrap();
        assert_eq!(balance.remaining, 23.75);
        assert_eq!(balance.unit, "CNY");
    }

    #[test]
    fn official_hosts_never_enter_account_protocol_detection() {
        for host in [
            "api.openai.com",
            "api.anthropic.com",
            "opencode.ai",
            "api.commandcode.ai",
            "token-plan-cn.xiaomimimo.com",
        ] {
            assert!(is_known_official_host(host));
        }
        assert!(!is_known_official_host("relay.example.com"));

        for base_url in [
            "https://api.openai.com/v1",
            "https://opencode.ai/zen/go/v1",
            "https://api.commandcode.ai/provider/v1",
        ] {
            let connection = newapi::Connection {
                base_url: base_url.into(),
                user_id: Some(37),
                access_token: "account-token-must-not-be-used".into(),
                api_key: "model-key-must-not-be-used".into(),
                ..Default::default()
            };
            assert_eq!(
                query(&connection, &ApiProxyConfigPayload::default()).unwrap_err(),
                "newapi.errors.balanceUnsupported"
            );
        }
    }

    #[test]
    fn newapi_adapter_detects_public_metadata_before_sending_account_credentials() {
        let (base_url, requests, server) = mock_server(vec![
            ("200 OK", r#"{"success":true,"data":{"system_name":"Relay","quota_per_unit":500000}}"#),
            ("200 OK", r#"{"success":true,"data":{"id":37,"quota":750000,"used_quota":250000}}"#),
            ("200 OK", r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "account-token".into(),
            api_key: "model-key".into(),
            ..Default::default()
        };
        let (balance, source) = query(&connection, &ApiProxyConfigPayload::default()).unwrap();
        server.join().unwrap();
        assert_eq!(source, "newapi");
        assert_eq!(balance.remaining, 1.5);
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /api/status "));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(requests[1].starts_with("GET /api/user/self "));
        assert!(requests[1].to_ascii_lowercase().contains("authorization: bearer account-token"));
        assert!(requests[1].to_ascii_lowercase().contains("new-api-user: 37"));
    }

    #[test]
    fn sub2api_adapter_uses_auth_me_and_never_uses_the_model_key() {
        let (base_url, requests, server) = mock_server(vec![
            ("404 Not Found", r#"{}"#),
            ("200 OK", r#"{"code":0,"data":{"site_name":"Sub2API","registration_enabled":true}}"#),
            ("200 OK", r#"{"code":0,"data":{"id":37,"balance":9.25}}"#),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "sub2-account-token".into(),
            api_key: "model-key-must-not-be-used".into(),
            ..Default::default()
        };
        let (balance, source) = query(&connection, &ApiProxyConfigPayload::default()).unwrap();
        server.join().unwrap();
        assert_eq!(source, "sub2api");
        assert_eq!(balance.remaining, 9.25);
        let requests = requests.lock().unwrap();
        assert!(requests[2].starts_with("GET /api/v1/auth/me "));
        assert!(requests[2].to_ascii_lowercase().contains("authorization: bearer sub2-account-token"));
        assert!(requests.iter().all(|request| !request.contains("model-key-must-not-be-used")));
    }

    #[test]
    fn unknown_sites_receive_only_credential_free_detection_requests() {
        let (base_url, requests, server) = mock_server(vec![
            ("200 OK", r#"{"hello":"world"}"#),
            ("200 OK", r#"{"code":0,"data":{"site_name":"Other"}}"#),
        ]);
        let connection = newapi::Connection {
            base_url,
            user_id: Some(37),
            access_token: "account-token-must-not-leak".into(),
            api_key: "model-key-must-not-leak".into(),
            ..Default::default()
        };
        assert_eq!(
            query(&connection, &ApiProxyConfigPayload::default()).unwrap_err(),
            "newapi.errors.balanceDetect"
        );
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| !request.to_ascii_lowercase().contains("authorization:")));
        assert!(requests.iter().all(|request| !request.contains("must-not-leak")));
    }

    #[test]
    fn deepseek_adapter_uses_only_the_official_balance_shape_and_model_key() {
        let (base_url, requests, server) = mock_server(vec![(
            "200 OK",
            r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"8.50"}]}"#,
        )]);
        let connection = newapi::Connection {
            base_url,
            access_token: "account-token-must-not-be-used".into(),
            api_key: "deepseek-model-key".into(),
            ..Default::default()
        };
        let balance = deepseek_balance(&connection, &ApiProxyConfigPayload::default()).unwrap();
        server.join().unwrap();
        assert_eq!(balance.remaining, 8.5);
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /api/user/balance "));
        assert!(requests[0].to_ascii_lowercase().contains("authorization: bearer deepseek-model-key"));
        assert!(!requests[0].contains("account-token-must-not-be-used"));
    }
}
