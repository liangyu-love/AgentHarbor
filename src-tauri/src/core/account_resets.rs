//! Read-only reset-credit metadata. Count comes from wham/usage; optional
//! details must not turn a healthy quota refresh into a login/error state.
use super::models::{AccountResetCredit, AccountResetCredits};
use serde_json::Value;

pub fn parse_count(payload: &Value) -> Option<AccountResetCredits> {
    let count = payload
        .pointer("/rate_limit_reset_credits/available_count")?
        .as_u64()?;
    Some(AccountResetCredits {
        available_count: u32::try_from(count).ok()?,
        credits: None,
    })
}

fn timestamp(value: Option<&Value>) -> Result<Option<i64>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => chrono::DateTime::parse_from_rfc3339(text)
            .map(|time| Some(time.timestamp()))
            .map_err(|_| ()),
        _ => Err(()),
    }
}

pub fn apply_details(credits: &mut AccountResetCredits, payload: &Value) -> Result<(), ()> {
    let count = payload
        .get("available_count")
        .and_then(Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(())?;
    let items = payload.get("credits").and_then(Value::as_array).ok_or(())?;
    if items.len() > 1000 {
        return Err(());
    }
    let mut parsed = Vec::new();
    for item in items {
        if item
            .get("reset_type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "codex_rate_limits")
        {
            continue;
        }
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 40)
            .ok_or(())?;
        parsed.push(AccountResetCredit {
            status: status.into(),
            granted_at: timestamp(item.get("granted_at"))?,
            expires_at: timestamp(item.get("expires_at"))?,
        });
    }
    // Publish atomically only after every item validates; keep the usage count
    // on failure, and use the newer detail count on success.
    credits.available_count = count;
    credits.credits = Some(parsed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_zero_and_malformed_counts_are_distinct() {
        for value in [
            json!({}),
            json!({"rate_limit_reset_credits":null}),
            json!({"rate_limit_reset_credits":{"available_count":-1}}),
            json!({"rate_limit_reset_credits":{"available_count":1.5}}),
        ] {
            assert!(parse_count(&value).is_none());
        }
        assert_eq!(
            parse_count(&json!({"rate_limit_reset_credits":{"available_count":0}}))
                .unwrap()
                .available_count,
            0
        );
    }

    #[test]
    fn details_preserve_expiry_history_and_newer_count_without_ids() {
        let mut credits =
            parse_count(&json!({"rate_limit_reset_credits":{"available_count":2}})).unwrap();
        apply_details(&mut credits, &json!({"available_count":1,"credits":[
            {"id":"not-persisted","reset_type":"codex_rate_limits","status":"available","granted_at":"2026-09-30T00:00:00Z","expires_at":"2026-10-30T00:00:00Z"},
            {"status":"redeemed","granted_at":"2026-09-01T00:00:00Z","expires_at":null},
            {"reset_type":"other","status":"available"}
        ]})).unwrap();
        assert_eq!(credits.available_count, 1);
        let items = credits.credits.as_ref().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].expires_at.unwrap() > items[0].granted_at.unwrap());
        assert_eq!(items[1].status, "redeemed");
        assert_eq!(items[1].expires_at, None);
        assert!(!serde_json::to_string(&credits)
            .unwrap()
            .contains("not-persisted"));
        let previous = credits.clone();
        assert!(apply_details(
            &mut credits,
            &json!({"available_count":9,"credits":[{"status":"available","expires_at":"bad-date"}]})
        )
        .is_err());
        assert_eq!(credits, previous);
    }
}
