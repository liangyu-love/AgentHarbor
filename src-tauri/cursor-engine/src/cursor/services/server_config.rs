//! Cursor's server configuration, passed through with two changes: Agent
//! traffic stays on HTTP/1.1 (BidiAppend + RunSSE, which the local front
//! serves), and for `cursor-agent` pointed at the local endpoint, the Agent
//! servers Cursor names are dropped so the CLI keeps using that endpoint.
use axum::{
    body::Body,
    extract::Extension,
    http::{Request, Response},
};
use bytes::{BufMut, BytesMut};
use prost::Message;

use crate::{
    api::cursor::proxy::{self, CursorProxy, UPSTREAM_URL_HEADER},
    Result,
};

use super::model_catalog::{local_response, unary_payload};

const HTTP2_CONFIG: u64 = 7;
const HTTP2_CONFIG_FORCE_ALL_DISABLED: i32 = 1;
const AGENT_URL_CONFIG: u64 = 27;

#[derive(Clone, PartialEq, Message)]
struct ServerConfigResponse {
    #[prost(string, tag = "6")]
    config_version: String,
    #[prost(int32, tag = "7")]
    http2_config: i32,
    #[prost(bool, optional, tag = "28")]
    cli_sandbox_default_enabled: Option<bool>,
}

pub async fn get(Extension(proxy): Extension<CursorProxy>, request: Request<Body>) -> Result<Response<Body>> {
    // Read inside the IDE's CONNECT, a request names the server it was for;
    // the CLI talks to the local endpoint directly.
    let direct = !request.headers().contains_key(UPSTREAM_URL_HEADER);
    let upstream = match proxy::forward_buffered(&proxy, request).await {
        Ok(upstream) if upstream.status.is_success() => upstream,
        Ok(upstream) => {
            tracing::warn!(status = %upstream.status, "Cursor GetServerConfig refused; answering locally");
            return Ok(local_response(fallback().encode_to_vec()));
        }
        Err(error) => {
            tracing::warn!(%error, "Cursor GetServerConfig unavailable; answering locally");
            return Ok(local_response(fallback().encode_to_vec()));
        }
    };
    let (framed, payload) = unary_payload(&upstream.body)?;
    let patched = patch(payload, direct);
    let body = if framed {
        let mut body = BytesMut::with_capacity(5 + patched.len());
        body.put_u8(0);
        body.put_u32(patched.len() as u32);
        body.extend_from_slice(&patched);
        body.freeze()
    } else {
        patched.into()
    };
    Ok(upstream.with_body(body))
}

fn fallback() -> ServerConfigResponse {
    ServerConfigResponse {
        config_version: "agentharbor_local".into(),
        http2_config: HTTP2_CONFIG_FORCE_ALL_DISABLED,
        cli_sandbox_default_enabled: Some(true),
    }
}

/// The configuration with HTTP/2 off and, when `direct`, no Agent servers.
fn patch(payload: &[u8], direct: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 2);
    let mut at = 0;
    while at < payload.len() {
        let start = at;
        let Some(key) = varint(payload, &mut at) else { return untouched(payload) };
        let skipped = match key & 7 {
            0 => varint(payload, &mut at).map(|_| ()),
            1 => advance(payload, &mut at, 8),
            2 => varint(payload, &mut at).and_then(|length| advance(payload, &mut at, length as usize)),
            5 => advance(payload, &mut at, 4),
            _ => None,
        };
        if skipped.is_none() {
            return untouched(payload);
        }
        let field = key >> 3;
        if field == HTTP2_CONFIG || (direct && field == AGENT_URL_CONFIG) {
            continue;
        }
        out.extend_from_slice(&payload[start..at]);
    }
    out.push((HTTP2_CONFIG << 3) as u8);
    out.push(HTTP2_CONFIG_FORCE_ALL_DISABLED as u8);
    out
}

/// A configuration that could not be read keeps everything but gets HTTP/2
/// off appended: a later scalar field wins in protobuf.
fn untouched(payload: &[u8]) -> Vec<u8> {
    let mut out = payload.to_vec();
    out.push((HTTP2_CONFIG << 3) as u8);
    out.push(HTTP2_CONFIG_FORCE_ALL_DISABLED as u8);
    out
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn advance(bytes: &[u8], at: &mut usize, length: usize) -> Option<()> {
    let end = at.checked_add(length)?;
    (end <= bytes.len()).then(|| *at = end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Message)]
    struct AgentUrlConfig {
        #[prost(string, tag = "1")]
        agent_url: String,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Config {
        #[prost(string, tag = "6")]
        config_version: String,
        #[prost(int32, tag = "7")]
        http2_config: i32,
        #[prost(message, optional, tag = "27")]
        agent_url_config: Option<AgentUrlConfig>,
        #[prost(bool, optional, tag = "28")]
        cli_sandbox_default_enabled: Option<bool>,
    }

    fn official() -> Vec<u8> {
        Config {
            config_version: "official".into(),
            http2_config: 2,
            agent_url_config: Some(AgentUrlConfig { agent_url: "https://agent.cursor.sh".into() }),
            cli_sandbox_default_enabled: Some(false),
        }
        .encode_to_vec()
    }

    #[test]
    fn the_cli_keeps_its_endpoint_and_both_stay_on_http1() {
        let cli = Config::decode(patch(&official(), true).as_slice()).unwrap();
        assert_eq!(cli.config_version, "official");
        assert_eq!(cli.http2_config, HTTP2_CONFIG_FORCE_ALL_DISABLED);
        assert_eq!(cli.agent_url_config, None);
        assert_eq!(cli.cli_sandbox_default_enabled, Some(false));

        let ide = Config::decode(patch(&official(), false).as_slice()).unwrap();
        assert_eq!(ide.http2_config, HTTP2_CONFIG_FORCE_ALL_DISABLED);
        assert_eq!(ide.agent_url_config.unwrap().agent_url, "https://agent.cursor.sh");
    }

    #[test]
    fn unreadable_configurations_still_turn_http2_off() {
        let broken = [0x0a, 0x10, 0x01];
        let patched = patch(&broken, true);
        assert_eq!(&patched[..3], &broken);
        assert_eq!(&patched[3..], &[0x38, 0x01]);
    }
}
