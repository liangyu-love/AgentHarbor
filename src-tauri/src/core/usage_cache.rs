//! Cache analytics use only requests with explicit input and cache-read usage.
//! Missing cache counters are unknown, not cache misses.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CacheUsageTotals {
    pub reported_count: u64,
    pub prompt_tokens: u64,
    pub read_tokens: u64,
    pub write_tokens: u64,
}

impl CacheUsageTotals {
    pub fn add(&mut self, input: u64, read: u64, write: u64) {
        self.reported_count = self.reported_count.saturating_add(1);
        self.prompt_tokens = self
            .prompt_tokens
            .saturating_add(input.saturating_add(read).saturating_add(write));
        self.read_tokens = self.read_tokens.saturating_add(read);
        self.write_tokens = self.write_tokens.saturating_add(write);
    }

    pub fn merge(&mut self, other: &Self) {
        self.reported_count = self.reported_count.saturating_add(other.reported_count);
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.read_tokens = self.read_tokens.saturating_add(other.read_tokens);
        self.write_tokens = self.write_tokens.saturating_add(other.write_tokens);
    }
}
