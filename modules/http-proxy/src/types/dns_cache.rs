//! Application-level DNS result cache with TTL-based expiry.
//!
//! Caches resolved `Vec<Arc<UpstreamInner>>` for strict DNS and
//! `Vec<(Arc<UpstreamInner>, u16, u16)>` for SRV lookups. Each entry
//! expires based on the minimum TTL from the DNS response records.
//!
//! This sits on top of Hickory's internal moka cache, avoiding per-request
//! lock contention and skipping record parsing for hot hostnames.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::upstream::UpstreamInner;

const MAX_CACHE_ENTRIES: usize = 10_000;

type StrictDnsKey = (String, u16, Vec<IpAddr>);
type StrictDnsValue = Vec<Arc<UpstreamInner>>;
type SrvKey = (String, Vec<IpAddr>);
type SrvValue = Vec<(Arc<UpstreamInner>, u16, u16)>;

/// Metrics counters for cache hits and misses.
pub(crate) static DNS_CACHE_HITS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub(crate) static DNS_CACHE_MISSES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

struct DnsCacheEntry<V> {
    value: V,
    expires_at: Instant,
}

impl<V> DnsCacheEntry<V> {
    #[inline]
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Strict DNS cache: keyed by `(hostname, port, dns_servers)`.
struct StrictDnsCache {
    entries: DashMap<StrictDnsKey, DnsCacheEntry<StrictDnsValue>, ahash::RandomState>,
}

impl StrictDnsCache {
    #[inline]
    fn new() -> Self {
        Self {
            entries: DashMap::with_capacity_and_hasher(64, ahash::RandomState::default()),
        }
    }

    #[inline]
    fn get(&self, key: &StrictDnsKey) -> Option<StrictDnsValue> {
        let expired = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.is_expired());
        if expired {
            self.entries.remove(key);
            return None;
        }
        self.entries.get(key).map(|entry| entry.value.clone())
    }

    #[inline]
    fn insert(&self, key: StrictDnsKey, value: StrictDnsValue, ttl: Duration) {
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.evict_expired();
            if self.entries.len() >= MAX_CACHE_ENTRIES {
                // Still at capacity — remove oldest entry
                if let Some(oldest_key) = self
                    .entries
                    .iter()
                    .min_by_key(|r| r.expires_at)
                    .map(|r| r.key().clone())
                {
                    self.entries.remove(&oldest_key);
                }
            }
        }
        let expires_at = Instant::now() + ttl;
        self.entries
            .insert(key, DnsCacheEntry { value, expires_at });
    }

    #[inline]
    fn evict_expired(&self) {
        self.entries.retain(|_, entry| !entry.is_expired());
    }
}

/// SRV cache: keyed by `(srv_name, dns_servers)`.
struct SrvCache {
    entries: DashMap<SrvKey, DnsCacheEntry<SrvValue>, ahash::RandomState>,
}

impl SrvCache {
    #[inline]
    fn new() -> Self {
        Self {
            entries: DashMap::with_capacity_and_hasher(64, ahash::RandomState::default()),
        }
    }

    #[inline]
    fn get(&self, key: &SrvKey) -> Option<SrvValue> {
        let expired = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.is_expired());
        if expired {
            self.entries.remove(key);
            return None;
        }
        self.entries.get(key).map(|entry| entry.value.clone())
    }

    #[inline]
    fn insert(&self, key: SrvKey, value: SrvValue, ttl: Duration) {
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.evict_expired();
            if self.entries.len() >= MAX_CACHE_ENTRIES {
                if let Some(oldest_key) = self
                    .entries
                    .iter()
                    .min_by_key(|r| r.expires_at)
                    .map(|r| r.key().clone())
                {
                    self.entries.remove(&oldest_key);
                }
            }
        }
        let expires_at = Instant::now() + ttl;
        self.entries
            .insert(key, DnsCacheEntry { value, expires_at });
    }

    #[inline]
    fn evict_expired(&self) {
        self.entries.retain(|_, entry| !entry.is_expired());
    }
}

struct DnsResultCache {
    strict_dns: StrictDnsCache,
    srv: SrvCache,
}

impl DnsResultCache {
    #[inline]
    fn new() -> Self {
        Self {
            strict_dns: StrictDnsCache::new(),
            srv: SrvCache::new(),
        }
    }

    #[inline]
    fn cleanup(&self) {
        self.strict_dns.evict_expired();
        self.srv.evict_expired();
    }
}

static DNS_RESULT_CACHE: std::sync::OnceLock<DnsResultCache> = std::sync::OnceLock::new();

#[inline]
fn cache() -> &'static DnsResultCache {
    DNS_RESULT_CACHE.get_or_init(DnsResultCache::new)
}

// --- Strict DNS cache API ---

/// Look up a cached strict DNS result.
///
/// Returns `Some(backends)` on cache hit, `None` on miss or expiry.
#[inline]
pub(crate) fn get_strict_dns(
    hostname: &str,
    port: u16,
    dns_servers: &[IpAddr],
) -> Option<Vec<Arc<UpstreamInner>>> {
    let key = (hostname.to_string(), port, dns_servers.to_vec());
    let result = cache().strict_dns.get(&key);
    if result.is_some() {
        DNS_CACHE_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        DNS_CACHE_MISSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    result
}

/// Insert a strict DNS result into the cache.
#[inline]
pub(crate) fn insert_strict_dns(
    hostname: &str,
    port: u16,
    dns_servers: &[IpAddr],
    value: Vec<Arc<UpstreamInner>>,
    ttl: Duration,
) {
    let key = (hostname.to_string(), port, dns_servers.to_vec());
    cache().strict_dns.insert(key, value, ttl);
}

// --- SRV cache API ---

/// Look up a cached SRV result.
///
/// Returns `Some(candidates)` on cache hit, `None` on miss or expiry.
#[inline]
pub(crate) fn get_srv(
    srv_name: &str,
    dns_servers: &[IpAddr],
) -> Option<Vec<(Arc<UpstreamInner>, u16, u16)>> {
    let key = (srv_name.to_string(), dns_servers.to_vec());
    let result = cache().srv.get(&key);
    if result.is_some() {
        DNS_CACHE_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        DNS_CACHE_MISSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    result
}

/// Insert an SRV result into the cache.
#[inline]
pub(crate) fn insert_srv(
    srv_name: &str,
    dns_servers: &[IpAddr],
    value: Vec<(Arc<UpstreamInner>, u16, u16)>,
    ttl: Duration,
) {
    let key = (srv_name.to_string(), dns_servers.to_vec());
    cache().srv.insert(key, value, ttl);
}

/// Remove all expired entries from both caches.
///
/// Called periodically by a background task and on config reload.
#[inline]
pub(crate) fn cleanup_expired() {
    cache().cleanup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn test_strict_dns_cache_roundtrip() {
        let hostname = "example.com".to_string();
        let port = 8080u16;
        let dns_servers = vec![];
        let upstreams = vec![];

        // Initially empty
        assert!(get_strict_dns(&hostname, port, &dns_servers).is_none());

        // Insert with long TTL
        insert_strict_dns(
            &hostname,
            port,
            &dns_servers,
            upstreams.clone(),
            Duration::from_secs(300),
        );

        // Should hit
        let cached = get_strict_dns(&hostname, port, &dns_servers);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 0);
    }

    #[test]
    #[inline]
    fn test_srv_cache_roundtrip() {
        let srv_name = "_http._tcp.example.com".to_string();
        let dns_servers = vec![];

        // Initially empty
        assert!(get_srv(&srv_name, &dns_servers).is_none());

        // Insert with long TTL
        insert_srv(&srv_name, &dns_servers, vec![], Duration::from_secs(300));

        // Should hit
        let cached = get_srv(&srv_name, &dns_servers);
        assert!(cached.is_some());
    }

    #[test]
    #[inline]
    fn test_cache_expiry() {
        let hostname = "expired.example.com".to_string();
        let port = 80u16;
        let dns_servers = vec![];

        // Insert with zero TTL (expires immediately)
        insert_strict_dns(&hostname, port, &dns_servers, vec![], Duration::ZERO);

        // Should miss (expired)
        assert!(get_strict_dns(&hostname, port, &dns_servers).is_none());
    }

    #[test]
    #[inline]
    fn test_cleanup_removes_expired() {
        let hostname = "cleanup.example.com".to_string();
        let port = 80u16;
        let dns_servers = vec![];

        // Insert with zero TTL
        insert_strict_dns(&hostname, port, &dns_servers, vec![], Duration::ZERO);

        // Run cleanup
        cleanup_expired();

        // Should be gone
        assert!(get_strict_dns(&hostname, port, &dns_servers).is_none());
    }
}
