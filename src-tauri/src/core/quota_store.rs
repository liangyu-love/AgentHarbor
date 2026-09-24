use super::models::{CoreError, RateLimitWindow, UsageSource};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuotaStoreFile {
    pub updated_at: i64,
    pub items: Vec<QuotaStoreItem>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaStoreItem {
    pub account_key: String,
    pub captured_at: i64,
    pub usage_source: UsageSource,
    pub primary_window: Option<RateLimitWindow>,
    pub secondary_window: Option<RateLimitWindow>,
    pub token_status: Option<serde_json::Value>,
}

pub fn load_or_default(path: &Path) -> QuotaStoreFile {
    std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn find_item<'a>(store: &'a QuotaStoreFile, key: &str) -> Option<&'a QuotaStoreItem> {
    store.items.iter().find(|item| item.account_key == key)
}

pub fn upsert_item(store: &mut QuotaStoreFile, item: QuotaStoreItem, now: i64) -> bool {
    if let Some(existing) = store
        .items
        .iter_mut()
        .find(|old| old.account_key == item.account_key)
    {
        if *existing == item {
            return false;
        }
        *existing = item;
    } else {
        store.items.push(item);
    }
    store.updated_at = now;
    true
}

pub fn save(path: &Path, store: &QuotaStoreFile) -> Result<(), CoreError> {
    super::storage::write_atomic(path, &serde_json::to_vec_pretty(store)?)
}
