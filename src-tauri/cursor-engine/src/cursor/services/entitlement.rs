//! Caches a recently confirmed official Cursor Free entitlement.
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{header, HeaderMap};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::account;

const FREE_ENTITLEMENT_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Default)]
pub struct FreeEntitlementCache {
    cached: Arc<Mutex<HashMap<[u8; 32], Instant>>>,
}

impl FreeEntitlementCache {
    pub fn is_confirmed_free(&self, headers: &HeaderMap) -> bool {
        let Some(token_hash) = official_token_hash(headers) else {
            return false;
        };
        let now = Instant::now();
        let mut cached = self.cached.lock();
        cached.retain(|_, expires_at| *expires_at > now);
        cached.contains_key(&token_hash)
    }

    pub fn observe_membership(&self, headers: &HeaderMap, membership_type: &str) -> bool {
        let Some(token_hash) = official_token_hash(headers) else {
            return false;
        };
        let now = Instant::now();
        let mut cached = self.cached.lock();
        cached.retain(|_, expires_at| *expires_at > now);
        if membership_type.eq_ignore_ascii_case("free") {
            cached.insert(token_hash, now + FREE_ENTITLEMENT_TTL);
        } else {
            cached.remove(&token_hash);
        }
        true
    }
}

fn official_token_hash(headers: &HeaderMap) -> Option<[u8; 32]> {
    if account::request_uses_local_cursor_token(headers) {
        return None;
    }
    let token = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    if token.is_empty() {
        return None;
    }
    Some(Sha256::digest(token.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn official_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn caches_only_a_confirmed_free_official_token() {
        let cache = FreeEntitlementCache::default();
        let free = official_headers("official-free-token");
        let other = official_headers("another-token");

        cache.observe_membership(&free, "free");

        assert!(cache.is_confirmed_free(&free));
        assert!(!cache.is_confirmed_free(&other));
    }

    #[test]
    fn confirmed_non_free_membership_clears_the_same_token() {
        let cache = FreeEntitlementCache::default();
        let headers = official_headers("official-token");
        cache.observe_membership(&headers, "free");

        cache.observe_membership(&headers, "pro");

        assert!(!cache.is_confirmed_free(&headers));
    }

    #[test]
    fn local_token_never_enters_the_entitlement_cache() {
        let cache = FreeEntitlementCache::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", account::local_token())
                .parse()
                .unwrap(),
        );

        assert!(!cache.observe_membership(&headers, "free"));

        assert!(!cache.is_confirmed_free(&headers));
        assert!(cache.cached.lock().is_empty());
    }

    #[test]
    fn expired_confirmation_is_removed() {
        let cache = FreeEntitlementCache::default();
        let headers = official_headers("official-token");
        let token_hash = official_token_hash(&headers).unwrap();
        cache
            .cached
            .lock()
            .insert(token_hash, Instant::now() - Duration::from_secs(1));

        assert!(!cache.is_confirmed_free(&headers));
        assert!(cache.cached.lock().is_empty());
    }

    #[test]
    fn ide_and_cli_entitlements_expire_and_invalidate_independently() {
        let cache = FreeEntitlementCache::default();
        let ide = official_headers("ide-free-token");
        let cli = official_headers("cli-free-token");
        cache.observe_membership(&ide, "free");
        cache.observe_membership(&cli, "free");
        assert!(cache.is_confirmed_free(&ide));
        assert!(cache.is_confirmed_free(&cli));

        cache.observe_membership(&ide, "pro");
        assert!(!cache.is_confirmed_free(&ide));
        assert!(cache.is_confirmed_free(&cli));

        cache.observe_membership(&ide, "free");
        cache.cached.lock().insert(
            official_token_hash(&ide).unwrap(),
            Instant::now() - Duration::from_secs(1),
        );
        assert!(!cache.is_confirmed_free(&ide));
        assert!(cache.is_confirmed_free(&cli));

        cache.observe_membership(&ide, "free");
        cache.observe_membership(&ide, "unknown");
        assert!(!cache.is_confirmed_free(&ide));
        assert!(cache.is_confirmed_free(&cli));
    }
}
