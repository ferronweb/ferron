//! Token bucket registry with per-key management and TTL-based eviction.
//!
//! Uses `DashMap` for concurrent sharded access to per-key token buckets.
//! Stale buckets are evicted based on last-access time to prevent unbounded
//! memory growth from one-shot clients.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::token_bucket::ConcurrentTokenBucket;

/// Entry wrapping a token bucket with metadata for eviction.
struct BucketEntry {
    bucket: ConcurrentTokenBucket,
    /// Instant of the last successful access (used for TTL eviction).
    last_accessed: AtomicU64,
}

impl BucketEntry {
    fn new(bucket: ConcurrentTokenBucket) -> Self {
        Self {
            bucket,
            last_accessed: AtomicU64::new(now_secs()),
        }
    }

    fn touch(&self) {
        self.last_accessed.store(now_secs(), Ordering::Relaxed);
    }

    fn last_accessed(&self) -> u64 {
        self.last_accessed.load(Ordering::Relaxed)
    }

    fn is_stale(&self, ttl_secs: u64) -> bool {
        now_secs().saturating_sub(self.last_accessed()) > ttl_secs
    }
}

// Use a process-start-relative counter for TTL.
static PROCESS_START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn now_secs() -> u64 {
    PROCESS_START.elapsed().as_secs()
}

/// A registry of per-key token buckets with TTL-based eviction.
///
/// Buckets are created on-demand when `get_or_create` is called.
/// Stale buckets (not accessed within the TTL window) are evicted during
/// periodic `evict_stale` calls.
#[derive(Clone)]
pub struct TokenBucketRegistry {
    /// Sharded concurrent map from key → bucket entry.
    buckets: Arc<DashMap<String, BucketEntry>>,
    /// Parameters for creating new buckets.
    capacity: u64,
    refill_rate: f64,
    /// TTL in seconds for evicting stale buckets.
    ttl_secs: u64,
    /// Maximum number of buckets. When exceeded, stale entries are evicted
    /// before new ones are created.
    max_buckets: usize,
}

impl TokenBucketRegistry {
    /// Create a new registry with the given parameters.
    pub fn new(capacity: u64, refill_rate: f64, ttl_secs: u64, max_buckets: usize) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            capacity,
            refill_rate,
            ttl_secs,
            max_buckets,
        }
    }

    /// Get or create a token bucket for the given key.
    ///
    /// If the registry is full, evicts stale entries first. If still full
    /// after eviction, returns `None` (backpressure).
    pub fn get_or_create(&self, key: &str) -> Option<ConcurrentTokenBucket> {
        // Fast path: bucket exists
        if let Some(entry) = self.buckets.get(key) {
            entry.touch();
            return Some(entry.bucket.clone());
        }

        // Slow path: need to create a new bucket
        // First try eviction if we're at capacity
        if self.buckets.len() >= self.max_buckets {
            self.evict_stale();
        }

        // Try again after eviction
        if self.buckets.len() >= self.max_buckets {
            // Still at capacity — backpressure
            return None;
        }

        // Create the new bucket
        let bucket = ConcurrentTokenBucket::new(self.capacity, self.refill_rate);
        let entry = BucketEntry::new(bucket.clone());
        self.buckets.insert(key.to_string(), entry);

        Some(bucket)
    }

    /// Evict stale buckets that haven't been accessed within the TTL window.
    pub fn evict_stale(&self) {
        self.buckets
            .retain(|_, entry| !entry.is_stale(self.ttl_secs));
    }

    /// Get the current number of active buckets.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// Check if the registry is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn creates_bucket_on_demand() {
        let registry = TokenBucketRegistry::new(10, 1.0, 60, 100);
        let bucket = registry.get_or_create("192.0.2.1");
        assert!(bucket.is_some());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn returns_same_bucket_for_same_key() {
        let registry = TokenBucketRegistry::new(10, 1.0, 60, 100);
        let b1 = registry.get_or_create("192.0.2.1").unwrap();
        let b2 = registry.get_or_create("192.0.2.1").unwrap();

        // Both should share the same underlying bucket
        b1.try_consume(10);
        assert!(!b2.try_consume(1)); // bucket is now empty
    }

    #[test]
    fn different_keys_get_different_buckets() {
        let registry = TokenBucketRegistry::new(5, 0.0, 60, 100);
        let b1 = registry.get_or_create("key1").unwrap();
        let b2 = registry.get_or_create("key2").unwrap();

        // Draining one should not affect the other
        b1.try_consume(5);
        assert!(b2.try_consume(1));
    }

    #[test]
    fn evicts_stale_buckets() {
        // Use a very short TTL (1 second)
        let registry = TokenBucketRegistry::new(10, 1.0, 1, 100);
        registry.get_or_create("stale_key");
        assert_eq!(registry.len(), 1);

        // Wait for TTL to expire
        thread::sleep(Duration::from_secs(2));
        registry.evict_stale();

        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn respects_max_buckets() {
        let registry = TokenBucketRegistry::new(10, 0.0, 60, 3);
        assert!(registry.get_or_create("k1").is_some());
        assert!(registry.get_or_create("k2").is_some());
        assert!(registry.get_or_create("k3").is_some());

        // 4th should fail (no eviction possible with fresh entries)
        assert!(registry.get_or_create("k4").is_none());
    }

    #[test]
    fn evicts_before_creating_new_when_at_capacity() {
        // Use short TTL so entries become stale quickly
        let registry = TokenBucketRegistry::new(10, 0.0, 1, 2);
        let b1 = registry.get_or_create("k1").unwrap();
        let _b2 = registry.get_or_create("k2").unwrap();

        // Access k1 to keep it fresh, wait for k2 to go stale
        b1.try_consume(1); // touches k1
        thread::sleep(Duration::from_secs(2));

        // k2 is stale, evict should free a slot
        let b3 = registry.get_or_create("k3");
        assert!(b3.is_some());
    }

    #[test]
    fn concurrent_access() {
        let registry = Arc::new(TokenBucketRegistry::new(1000, 0.0, 60, 1000));
        let mut handles = Vec::new();

        for i in 0..10 {
            let r = registry.clone();
            handles.push(thread::spawn(move || {
                let key = format!("key-{}", i);
                let bucket = r.get_or_create(&key).unwrap();
                bucket.try_consume(1);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(registry.len(), 10);
    }
}
