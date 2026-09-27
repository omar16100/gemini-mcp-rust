//! Bounded in-memory TTL cache, used by `gemini-search-v2` to reuse Gemini
//! generations for identical requests.
//!
//! - Keys are SHA-256 fingerprints of every input that determines the
//!   generation (see [`fingerprint`]), so a change to any of them is a miss.
//! - Entries expire after `ttl` (default 300 s, `GEMINI_CACHE_TTL_SECS`).
//! - At most `max_entries` entries are kept (default 100,
//!   `GEMINI_CACHE_MAX_ENTRIES`). Every insert drops expired entries; if the
//!   cache is still full, the oldest entry is evicted. Callers bound the size
//!   of each value (search-v2 skips answers over 256 KiB), so memory is at
//!   most about `max_entries` times that size.
//! - A TTL or capacity of 0 disables the cache.
//! - Expiry uses the monotonic clock (`Instant`), so wall-clock changes do not
//!   extend or shorten entries.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Default TTL for cache entries (5 minutes)
pub const DEFAULT_TTL_SECS: u64 = 300;
/// Default maximum number of cached entries
pub const DEFAULT_MAX_ENTRIES: usize = 100;
/// Largest accepted TTL (1 day)
pub const MAX_TTL_SECS: u64 = 86_400;
/// Largest accepted capacity
pub const MAX_ENTRIES_LIMIT: usize = 10_000;

pub const TTL_ENV: &str = "GEMINI_CACHE_TTL_SECS";
pub const MAX_ENTRIES_ENV: &str = "GEMINI_CACHE_MAX_ENTRIES";

#[derive(Debug)]
struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
    expires_at: Instant,
}

/// Thread-safe, bounded, in-memory TTL cache.
#[derive(Debug)]
pub struct QueryCache<T> {
    store: Mutex<HashMap<String, CacheEntry<T>>>,
    ttl: Duration,
    max_entries: usize,
}

impl<T: Clone> QueryCache<T> {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        let ttl = ttl.min(Duration::from_secs(MAX_TTL_SECS));
        let max_entries = max_entries.min(MAX_ENTRIES_LIMIT);
        info!(
            "Initializing QueryCache: ttl={}s, max_entries={}, enabled={}",
            ttl.as_secs(),
            max_entries,
            !ttl.is_zero() && max_entries > 0
        );
        Self {
            store: Mutex::new(HashMap::new()),
            ttl,
            max_entries,
        }
    }

    /// Cache configured from `GEMINI_CACHE_TTL_SECS` and `GEMINI_CACHE_MAX_ENTRIES`.
    pub fn from_env() -> Self {
        let ttl = parse_env_number(
            TTL_ENV,
            std::env::var(TTL_ENV).ok().as_deref(),
            DEFAULT_TTL_SECS,
            MAX_TTL_SECS,
        );
        let max_entries = parse_env_number(
            MAX_ENTRIES_ENV,
            std::env::var(MAX_ENTRIES_ENV).ok().as_deref(),
            DEFAULT_MAX_ENTRIES as u64,
            MAX_ENTRIES_LIMIT as u64,
        );
        Self::new(Duration::from_secs(ttl), max_entries as usize)
    }

    pub fn is_enabled(&self) -> bool {
        !self.ttl.is_zero() && self.max_entries > 0
    }

    /// Get a value if present and not expired. Expired entries are removed.
    pub fn get(&self, key: &str) -> Option<T> {
        self.get_at(key, Instant::now())
    }

    /// Insert a value with the cache TTL. No-op when the cache is disabled.
    pub fn insert(&self, key: String, value: T) {
        self.insert_at(key, value, Instant::now());
    }

    /// Number of stored entries (may include expired entries not yet purged).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, CacheEntry<T>>> {
        // A panic while holding the lock cannot leave the map half-updated in
        // a way that matters for a cache, so recover from poisoning.
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get_at(&self, key: &str, now: Instant) -> Option<T> {
        let mut store = self.lock();
        match store.get(key) {
            Some(entry) if now < entry.expires_at => {
                debug!("Cache hit for key {}", short_key(key));
                Some(entry.value.clone())
            }
            Some(_) => {
                debug!("Cache entry expired for key {}", short_key(key));
                store.remove(key);
                None
            }
            None => {
                debug!("Cache miss for key {}", short_key(key));
                None
            }
        }
    }

    fn insert_at(&self, key: String, value: T, now: Instant) {
        if !self.is_enabled() {
            return;
        }
        let Some(expires_at) = now.checked_add(self.ttl) else {
            warn!("Cache TTL overflowed the clock; not caching");
            return;
        };

        let mut store = self.lock();
        // Drop expired entries on every insert so they do not linger until
        // read. O(n) with n <= max_entries, once per Gemini API call.
        let before = store.len();
        store.retain(|_, entry| now < entry.expires_at);
        if before > store.len() {
            debug!("Purged {} expired cache entries", before - store.len());
        }
        if !store.contains_key(&key) && store.len() >= self.max_entries {
            let oldest = store
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                debug!("Cache full, evicting oldest key {}", short_key(&oldest));
                store.remove(&oldest);
            }
        }

        let short = short_key(&key).to_string();
        store.insert(
            key,
            CacheEntry {
                value,
                inserted_at: now,
                expires_at,
            },
        );
        debug!(
            "Cached value for key {} (ttl={}s, size={})",
            short,
            self.ttl.as_secs(),
            store.len()
        );
    }
}

/// Parse a numeric env var; invalid or out-of-range values fall back to the default.
fn parse_env_number(name: &str, raw: Option<&str>, default: u64, max: u64) -> u64 {
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().parse::<u64>() {
        Ok(n) if n <= max => n,
        _ => {
            warn!(
                "Ignoring invalid {}={:?} (expected 0-{}), using {}",
                name, raw, max, default
            );
            default
        }
    }
}

/// SHA-256 fingerprint of an ordered list of inputs. Each part is
/// length-prefixed, so `["ab", "c"]` and `["a", "bc"]` differ.
pub fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn short_key(key: &str) -> &str {
    key.get(..12).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(ttl_secs: u64, max_entries: usize) -> QueryCache<i32> {
        QueryCache::new(Duration::from_secs(ttl_secs), max_entries)
    }

    #[test]
    fn test_set_and_get() {
        let cache = cache(60, 10);
        cache.insert("key1".to_string(), 1);
        assert_eq!(cache.get("key1"), Some(1));
        assert_eq!(cache.get("missing"), None);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_entry_expires_after_ttl() {
        let cache = cache(300, 10);
        let t0 = Instant::now();
        cache.insert_at("k".to_string(), 1, t0);

        assert_eq!(cache.get_at("k", t0 + Duration::from_secs(299)), Some(1));
        assert_eq!(cache.get_at("k", t0 + Duration::from_secs(300)), None);
        assert_eq!(cache.len(), 0, "expired entry is removed on read");
    }

    #[test]
    fn test_overwrite_refreshes_ttl() {
        let cache = cache(10, 10);
        let t0 = Instant::now();
        cache.insert_at("k".to_string(), 1, t0);
        cache.insert_at("k".to_string(), 2, t0 + Duration::from_secs(8));
        assert_eq!(cache.get_at("k", t0 + Duration::from_secs(15)), Some(2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_capacity_evicts_oldest() {
        let cache = cache(300, 2);
        let t0 = Instant::now();
        cache.insert_at("a".to_string(), 1, t0);
        cache.insert_at("b".to_string(), 2, t0 + Duration::from_secs(1));
        cache.insert_at("c".to_string(), 3, t0 + Duration::from_secs(2));

        let now = t0 + Duration::from_secs(3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get_at("a", now), None, "oldest entry evicted");
        assert_eq!(cache.get_at("b", now), Some(2));
        assert_eq!(cache.get_at("c", now), Some(3));
    }

    #[test]
    fn test_full_cache_drops_expired_before_evicting() {
        let cache = cache(10, 2);
        let t0 = Instant::now();
        cache.insert_at("old".to_string(), 1, t0);
        cache.insert_at("fresh".to_string(), 2, t0 + Duration::from_secs(9));
        // At t0+11, "old" is expired and "fresh" is live: only "old" goes.
        cache.insert_at("new".to_string(), 3, t0 + Duration::from_secs(11));

        let now = t0 + Duration::from_secs(12);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get_at("fresh", now), Some(2));
        assert_eq!(cache.get_at("new", now), Some(3));
    }

    #[test]
    fn test_insert_purges_expired_entries() {
        let cache = cache(10, 100);
        let t0 = Instant::now();
        cache.insert_at("a".to_string(), 1, t0);
        cache.insert_at("b".to_string(), 2, t0 + Duration::from_secs(1));
        cache.insert_at("c".to_string(), 3, t0 + Duration::from_secs(10));
        // "a" expired at t0+10 and is dropped by the insert of "c".
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_updating_existing_key_in_full_cache_does_not_evict() {
        let cache = cache(300, 2);
        let t0 = Instant::now();
        cache.insert_at("a".to_string(), 1, t0);
        cache.insert_at("b".to_string(), 2, t0);
        cache.insert_at("a".to_string(), 10, t0 + Duration::from_secs(1));

        let now = t0 + Duration::from_secs(2);
        assert_eq!(cache.get_at("a", now), Some(10));
        assert_eq!(cache.get_at("b", now), Some(2));
    }

    #[test]
    fn test_zero_ttl_or_capacity_disables_cache() {
        for disabled in [cache(0, 10), cache(60, 0)] {
            assert!(!disabled.is_enabled());
            disabled.insert("k".to_string(), 1);
            assert_eq!(disabled.get("k"), None);
            assert_eq!(disabled.len(), 0);
        }
    }

    #[test]
    fn test_limits_are_clamped() {
        let cache = cache(u64::MAX, usize::MAX);
        assert_eq!(cache.ttl, Duration::from_secs(MAX_TTL_SECS));
        assert_eq!(cache.max_entries, MAX_ENTRIES_LIMIT);
    }

    #[test]
    fn test_parse_env_number() {
        assert_eq!(parse_env_number("X", None, 300, 1000), 300);
        assert_eq!(parse_env_number("X", Some("0"), 300, 1000), 0);
        assert_eq!(parse_env_number("X", Some(" 60 "), 300, 1000), 60);
        assert_eq!(parse_env_number("X", Some("1001"), 300, 1000), 300);
        assert_eq!(parse_env_number("X", Some("-1"), 300, 1000), 300);
        assert_eq!(parse_env_number("X", Some("five"), 300, 1000), 300);
    }

    #[test]
    fn test_fingerprint_deterministic_and_input_sensitive() {
        let base = fingerprint(&["model", "prompt", "config"]);
        assert_eq!(base, fingerprint(&["model", "prompt", "config"]));
        assert_eq!(base.len(), 64);
        assert_ne!(base, fingerprint(&["model2", "prompt", "config"]));
        assert_ne!(base, fingerprint(&["model", "prompt!", "config"]));
        assert_ne!(base, fingerprint(&["model", "prompt", "config2"]));
    }

    #[test]
    fn test_fingerprint_is_length_prefixed() {
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        assert_ne!(fingerprint(&["a", ""]), fingerprint(&["", "a"]));
    }

    #[test]
    fn test_cache_shared_across_threads() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(cache(60, 100));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for j in 0..20 {
                        cache.insert(format!("{i}-{j}"), i * 100 + j);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(cache.len(), 100, "capacity bound holds under concurrency");
    }
}
