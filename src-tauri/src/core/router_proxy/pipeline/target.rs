//! Where an attempt goes and how it is signed: a relay's own key, an account
//! source's fresh account token, or Codex's own sign-in for official models.
use super::super::*;

pub(in crate::core::router_proxy) fn provider_query(query: Option<&str>, translation: Option<protocol::Translation>) -> String {
    let Some(query) = query else {
        return String::new();
    };
    let query =
        if translation.is_some_and(|pair| pair.from == router::ModelProtocol::AnthropicMessages) {
            // The Anthropic SDK's beta flag does not belong to OpenAI endpoints.
            query
                .split('&')
                .filter(|part| part.split('=').next() != Some("beta"))
                .collect::<Vec<_>>()
                .join("&")
        } else {
            query.to_owned()
        };
    if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    }
}

/// URL and headers for a relay provider, carrying its own API key.
pub(in crate::core::router_proxy) fn provider_target(
    provider: &router::Provider,
    upstream_protocol: router::ModelProtocol,
    translation: Option<protocol::Translation>,
    suffix: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> ForwardResult<(String, HeaderMap)> {
    let endpoint = router::ModelEndpoint {
        base_url: &provider.base_url,
        protocol: upstream_protocol,
    };
    let credential = provider.model_credential();
    let (auth_name, auth_value) = crate::core::newapi::model_auth_header(
        endpoint.base_url,
        endpoint.protocol,
        credential.api_key)
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid provider configuration",
                )
            })?;
    headers.remove(AUTHORIZATION);
    headers.remove("x-api-key");
    headers.insert(auth_name, auth_value);
    match endpoint.protocol {
        router::ModelProtocol::Responses | router::ModelProtocol::ChatCompletions => {
            headers.remove("anthropic-version");
            headers.remove("anthropic-beta");
        }
        router::ModelProtocol::AnthropicMessages => {
            if !headers.contains_key("anthropic-version") {
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
        }
    }
    Ok((
        format!(
            "{}{}",
            crate::core::newapi::provider_endpoint(endpoint.base_url, &format!("v1/{suffix}"))
                .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid provider URL"))?,
            provider_query(query, translation)
        ),
        headers,
    ))
}

/// An account source's credentials for this destination, refreshed when about
/// to lapse; relays carry their own key and need none.
pub(in crate::core::router_proxy) async fn account_auth(context: &ArcContext, destination: &Destination) -> ForwardResult<Option<account_sources::RequestAuth>> {
    let Destination::Provider { provider, .. } = destination else { return Ok(None) };
    let Some(kind) = provider.account_source else { return Ok(None) };
    let client = client_for(context, destination)?;
    account_sources::credentials(&context.app_data_dir, &client, kind).await.map(Some)
}

/// URL and headers for an account source: its own endpoint, signed as the
/// active account. Only what the source serves (inference, ChatGPT's model
/// list) is asked.
pub(in crate::core::router_proxy) fn account_target(
    auth: &account_sources::RequestAuth,
    upstream_protocol: router::ModelProtocol,
    translation: Option<protocol::Translation>,
    suffix: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> ForwardResult<(String, HeaderMap)> {
    if !auth.serves(upstream_protocol, suffix) {
        return Err((StatusCode::NOT_FOUND, "this account source serves only model requests"));
    }
    if auth.exact {
        headers.clear();
    }
    headers.remove(AUTHORIZATION);
    headers.remove("x-api-key");
    headers.remove("anthropic-version");
    headers.remove("anthropic-beta");
    for (name, value) in &auth.headers {
        let value = HeaderValue::from_str(value)
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid account source credentials"))?;
        headers.insert(hyper::header::HeaderName::from_static(name), value);
    }
    Ok((format!("{}/{}{}", auth.base.trim_end_matches('/'), auth.path(suffix), provider_query(query, translation)), headers))
}

/// URL and headers for official OpenAI models, using Codex's own credentials.
pub(in crate::core::router_proxy) fn official_target(
    context: &ArcContext,
    codex_auth: Option<(HeaderValue, Option<HeaderValue>)>,
    suffix: &str,
    query: Option<&str>,
    mut headers: HeaderMap,
) -> ForwardResult<(String, HeaderMap)> {
    let query = query.map(|q| format!("?{q}")).unwrap_or_default();
    if let Some((authorization, account_id)) = codex_auth {
        // Codex may have refreshed its token in memory or use a keyring.
        // Its current credentials take precedence over a stale auth.json.
        headers.insert(AUTHORIZATION, authorization);
        let is_chatgpt = account_id.is_some()
            || bearer_matches_chatgpt_auth(headers.get(AUTHORIZATION).unwrap(), &context.auth_path);
        if let Some(account_id) = account_id {
            headers.insert("chatgpt-account-id", account_id);
        }
        let base = if is_chatgpt {
            CHATGPT_CODEX_BASE
        } else {
            OPENAI_API_BASE
        };
        return Ok((format!("{}{suffix}{query}", context.official_base(base)), headers));
    }
    let auth = crate::core::auth::load_auth_file(&context.auth_path).ok();
    let access_token = auth
        .as_ref()
        .and_then(|a| a.tokens.access_token.as_deref())
        .map(str::to_owned)
        .filter(|t| !t.trim().is_empty());
    if let Some(token) = access_token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid token"))?,
        );
        if let Some(account_id) = auth
            .as_ref()
            .and_then(|a| a.tokens.account_id.as_deref())
            .filter(|s| !s.is_empty())
        {
            if let Ok(value) = HeaderValue::from_str(account_id) {
                headers.insert("chatgpt-account-id", value);
            }
        }
        Ok((format!("{}{suffix}{query}", context.official_base(CHATGPT_CODEX_BASE)), headers))
    } else if let Some(key) = auth
        .and_then(|a| a.openai_api_key)
        .filter(|k| !k.trim().is_empty())
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "invalid key"))?,
        );
        Ok((format!("{}{suffix}{query}", context.official_base(OPENAI_API_BASE)), headers))
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "no OpenAI credentials found for official models",
        ))
    }
}

/// Direct-mode providers bypass the configured proxy; everything else uses it.
pub(in crate::core::router_proxy) fn client_for(context: &ArcContext, destination: &Destination) -> ForwardResult<Client> {
    Ok(match destination {
        Destination::Provider { provider, .. } if provider.network_mode() == "direct" => {
            context.outbound_direct.clone()
        }
        _ => context
            .outbound
            .read()
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "router network state unavailable",
                )
            })?
            .clone(),
    })
}

pub(in crate::core::router_proxy) fn bearer_matches_chatgpt_auth(authorization: &HeaderValue, auth_path: &std::path::Path) -> bool {
    crate::core::auth::load_auth_file(auth_path)
        .ok()
        .and_then(|auth| auth.tokens.access_token)
        .filter(|token| !token.trim().is_empty())
        .is_some_and(|token| {
            authorization.to_str().ok().and_then(|v| v.strip_prefix("Bearer "))
                .is_some_and(|candidate| token_matches(candidate, &token))
        })
}
