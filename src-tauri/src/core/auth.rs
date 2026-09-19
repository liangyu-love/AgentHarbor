//! Local identity metadata only. Tokens never leave this module through an IPC payload.
use super::models::{AuthMode, CoreError, PlanType};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Serialize, Deserialize, Default)]
pub struct AuthTokens {
    pub id_token: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct AuthFile {
    pub auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub tokens: AuthTokens,
    pub last_refresh: Option<String>,
}

pub struct AuthSnapshot {
    pub account_key: String,
    pub email: String,
    pub account_name: Option<String>,
    pub workspace_name: Option<String>,
    pub profile_name: Option<String>,
    pub plan: PlanType,
    pub auth_mode: AuthMode,
    pub created_at: i64,
}

pub fn current_timestamp() -> i64 { chrono::Utc::now().timestamp() }

pub fn load_auth_file(path: &Path) -> Result<AuthFile, CoreError> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn claims(token: Option<&str>) -> Value {
    token.and_then(|s| s.split('.').nth(1))
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or(Value::Null)
}

pub fn make_auth_snapshot(auth: &AuthFile, path: &Path) -> Result<AuthSnapshot, CoreError> {
    let identity = claims(auth.tokens.id_token.as_deref());
    let metadata = &identity["https://api.openai.com/auth"];
    let email = identity["email"].as_str().unwrap_or("").to_owned();
    // JWT claims are display metadata, never proof of authentication.
    let account_id = auth.tokens.account_id.as_deref()
        .or_else(|| metadata["chatgpt_account_id"].as_str());
    let api_key = auth.openai_api_key.as_deref().filter(|s| !s.trim().is_empty());
    let identity_key = account_id.or_else(|| identity["sub"].as_str())
        .or_else(|| (!email.is_empty()).then_some(email.as_str()))
        .or(api_key)
        .ok_or_else(|| CoreError::InvalidData("Auth file does not contain an account identity".into()))?;
    let account_key = format!("{:x}", Sha256::digest(identity_key.as_bytes()));
    let plan = match metadata["chatgpt_plan_type"].as_str().unwrap_or("") {
        "free" => PlanType::Free,
        "plus" => PlanType::Plus,
        "team" => PlanType::Team,
        "business" => PlanType::Business,
        "enterprise" => PlanType::Enterprise,
        "edu" => PlanType::Edu,
        _ => PlanType::Unknown,
    };
    let created_at = std::fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| t.as_secs() as i64).unwrap_or_else(current_timestamp);
    Ok(AuthSnapshot {
        account_key, email, plan, created_at,
        auth_mode: if api_key.is_some() { AuthMode::Apikey } else { AuthMode::Chatgpt },
        account_name: None, workspace_name: None,
        profile_name: identity["name"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_identity_is_hashed_and_missing_tokens_are_supported() {
        let auth: AuthFile = serde_json::from_str(r#"{"OPENAI_API_KEY":"test-key"}"#).unwrap();
        let snapshot = make_auth_snapshot(&auth, Path::new("missing.json")).unwrap();
        assert_eq!(snapshot.account_key.len(), 64);
        assert!(!snapshot.account_key.contains("test-key"));
        assert_eq!(snapshot.auth_mode, AuthMode::Apikey);
    }

    #[test]
    fn empty_auth_is_not_an_account() {
        let auth: AuthFile = serde_json::from_str("{}").unwrap();
        assert!(make_auth_snapshot(&auth, Path::new("missing.json")).is_err());
    }
}
