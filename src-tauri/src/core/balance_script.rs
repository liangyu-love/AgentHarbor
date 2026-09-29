//! Small, isolated JS extractor. No host functions, filesystem or network are
//! exposed to JS. Requests are executed by Rust with the provider's proxy policy.
use super::{models::ApiProxyConfigPayload, newapi, router_balance};
use rquickjs::{Context, Runtime};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const SCRIPT_ERROR: &str = "newapi.errors.balanceScript";

fn evaluate(script: &str, response: Option<&Value>) -> Result<Value, String> {
    if script.len() > 32 * 1024 {
        return Err(SCRIPT_ERROR.into());
    }
    let runtime = Runtime::new().map_err(|_| SCRIPT_ERROR)?;
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(256 * 1024);
    let deadline = Instant::now() + Duration::from_millis(300);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let context = Context::full(&runtime).map_err(|_| SCRIPT_ERROR)?;
    let expression = match response {
        Some(value) => format!(
            "JSON.stringify(({}).extractor({value}))",
            script.trim().trim_end_matches(';')
        ),
        None => format!(
            "JSON.stringify(({}).request)",
            script.trim().trim_end_matches(';')
        ),
    };
    let json = context
        .with(|ctx| ctx.eval::<String, _>(expression))
        .map_err(|_| SCRIPT_ERROR)?;
    if json.len() > 2 * 1024 * 1024 {
        return Err(SCRIPT_ERROR.into());
    }
    serde_json::from_str(&json).map_err(|_| SCRIPT_ERROR.into())
}

#[derive(Deserialize)]
struct Request {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<Value>,
}

pub fn validate(script: &str) -> Result<(), String> {
    if script.trim().is_empty() {
        return Ok(());
    }
    let request: Request =
        serde_json::from_value(evaluate(script, None)?).map_err(|_| SCRIPT_ERROR)?;
    if request.url.is_empty()
        || !matches!(request.method.as_deref().unwrap_or("GET"), "GET" | "POST")
    {
        return Err(SCRIPT_ERROR.into());
    }
    Ok(())
}

pub fn query(
    connection: &newapi::Connection,
    proxy: &ApiProxyConfigPayload,
    script: &str,
) -> Result<newapi::Balance, String> {
    let request: Request =
        serde_json::from_value(evaluate(script, None)?).map_err(|_| SCRIPT_ERROR)?;
    let root = newapi::base_url(&connection.base_url)?;
    // Secret placeholders are resolved only after the destination is validated.
    let url_text = request
        .url
        .replace("{{baseUrl}}", root.as_str().trim_end_matches('/'));
    let url = reqwest::Url::parse(&url_text).map_err(|_| "newapi.errors.url")?;
    if url.origin() != root.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("newapi.errors.balanceScriptOrigin".into());
    }
    let expand = |text: &str| -> Result<String, String> {
        for (placeholder, value) in [
            ("{{apiKey}}", connection.api_key.as_str()),
            ("{{accessToken}}", connection.access_token.as_str()),
        ] {
            if text.contains(placeholder) && value.is_empty() {
                return Err("newapi.errors.token".into());
            }
        }
        Ok(text
            .replace("{{apiKey}}", &connection.api_key)
            .replace("{{accessToken}}", &connection.access_token)
            .replace(
                "{{userId}}",
                &connection
                    .user_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
            ))
    };
    let client = newapi::client(proxy)?;
    let mut builder = match request.method.as_deref().unwrap_or("GET") {
        "GET" => client.get(url),
        "POST" => client.post(url),
        _ => return Err(SCRIPT_ERROR.into()),
    };
    for (key, value) in request.headers {
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "host" | "content-length" | "transfer-encoding"
        ) {
            return Err(SCRIPT_ERROR.into());
        }
        let name =
            reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|_| SCRIPT_ERROR)?;
        let mut header =
            reqwest::header::HeaderValue::from_str(&expand(&value)?).map_err(|_| SCRIPT_ERROR)?;
        header.set_sensitive(true);
        builder = builder.header(name, header);
    }
    if let Some(body) = request.body {
        builder = builder.json(&body);
    }
    let value = newapi::response_json(builder.send().map_err(newapi::request_error)?)?;
    let extracted = evaluate(script, Some(&value))?;
    router_balance::parse_custom_balance(&extracted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn script_preserves_zero_and_false_and_has_no_host_access() {
        let script = "({request:{url:'{{baseUrl}}/v1/usage'}, extractor:r=>({remaining:r.balance??4,unit:'USD',isValid:r.active??true})})";
        validate(script).unwrap();
        let data = evaluate(
            script,
            Some(&serde_json::json!({"balance":0,"active":false})),
        )
        .unwrap();
        assert_eq!(data["remaining"], 0);
        assert_eq!(data["isValid"], false);
        assert!(evaluate("(()=>{while(true){} })()", None).is_err());
        assert!(evaluate("({request:require('fs')})", None).is_err());
    }
    #[test]
    fn custom_query_resolves_placeholders_and_rejects_cross_origin() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 8192];
            let count = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_lowercase();
            assert!(request.contains("get /v1/usage "));
            assert!(request.contains("authorization: bearer synthetic-key"));
            let body = r#"{"data":{"balance":"12.5","unit":"USD"}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        let connection = newapi::Connection {
            base_url: format!("http://{address}/v1"),
            api_key: "synthetic-key".into(),
            ..Default::default()
        };
        let proxy = newapi::effective_proxy("direct", &ApiProxyConfigPayload::default());
        let script = "({request:{url:'{{baseUrl}}/v1/usage',headers:{Authorization:'Bearer {{apiKey}}'}},extractor:r=>({remaining:r.data.balance,unit:r.data.unit})})";
        assert_eq!(query(&connection, &proxy, script).unwrap().remaining, 12.5);
        server.join().unwrap();
        assert_eq!(
            query(
                &connection,
                &proxy,
                "({request:{url:'https://other.invalid/balance'}})"
            )
            .unwrap_err(),
            "newapi.errors.balanceScriptOrigin"
        );
    }
}
