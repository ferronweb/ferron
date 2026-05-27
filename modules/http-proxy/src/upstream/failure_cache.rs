use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::types::upstream::UpstreamInner;

/// A concurrent TTL (time-to-live) cache backed by a sharded, lock-free map.
pub struct ConcurrentTtlCache<K, V> {
    cache: DashMap<K, (V, Instant)>,
    ttl: Duration,
}

impl<K, V> ConcurrentTtlCache<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
    V: Clone,
{
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: DashMap::new(),
            ttl,
        }
    }

    pub fn insert(&self, key: K, value: V) {
        self.cache.insert(key, (value, Instant::now()));
    }

    pub fn get(&self, key: &K) -> Option<V> {
        if let Some(entry) = self.cache.get(key) {
            let (value, timestamp) = entry.value();
            if timestamp.elapsed() < self.ttl {
                return Some(value.clone());
            }
            drop(entry);
            self.cache.remove(key);
        }
        None
    }

    #[allow(dead_code)]
    pub fn remove(&self, key: &K) -> Option<V> {
        self.cache.remove(key).map(|(_, (value, _))| value)
    }

    #[allow(dead_code)]
    pub fn cleanup(&self) {
        self.cache
            .retain(|_, (_, timestamp)| timestamp.elapsed() < self.ttl);
    }
}

/// Cache for tracking failed backends, shared across all proxy requests.
pub(crate) type FailureCache = ConcurrentTtlCache<Arc<UpstreamInner>, u64>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_insert_and_get() {
        let cache = ConcurrentTtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        assert_eq!(cache.get(&"key1"), Some("value1"));
    }

    #[test]
    fn test_get_nonexistent_key() {
        let cache: ConcurrentTtlCache<&str, &str> =
            ConcurrentTtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.get(&"missing"), None);
    }

    #[test]
    fn test_expired_entry() {
        let cache = ConcurrentTtlCache::new(Duration::from_millis(50));
        cache.insert("key1", "value1");
        assert_eq!(cache.get(&"key1"), Some("value1"));

        sleep(Duration::from_millis(60));
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_overwrite_key() {
        let cache = ConcurrentTtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        cache.insert("key1", "value2");
        assert_eq!(cache.get(&"key1"), Some("value2"));
    }

    #[test]
    fn test_remove() {
        let cache = ConcurrentTtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        assert_eq!(cache.remove(&"key1"), Some("value1"));
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_remove_nonexistent() {
        let cache: ConcurrentTtlCache<&str, &str> =
            ConcurrentTtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.remove(&"missing"), None);
    }

    #[test]
    fn test_cleanup() {
        let cache = ConcurrentTtlCache::new(Duration::from_millis(50));
        cache.insert("key1", "value1");
        cache.insert("key2", "value2");

        sleep(Duration::from_millis(60));

        cache.insert("key3", "value3");
        cache.cleanup();

        assert_eq!(cache.get(&"key1"), None);
        assert_eq!(cache.get(&"key2"), None);
        assert_eq!(cache.get(&"key3"), Some("value3"));
    }

    #[test]
    fn test_multiple_entries() {
        let cache = ConcurrentTtlCache::new(Duration::from_secs(60));
        for i in 0..100 {
            cache.insert(format!("key{}", i), i);
        }
        for i in 0..100 {
            assert_eq!(cache.get(&format!("key{}", i)), Some(i));
        }
    }

    #[test]
    fn bench_concurrent_ttlcache_insert_get() {
        use std::time::Instant;
        let cache = ConcurrentTtlCache::new(Duration::from_secs(60));
        let n = 100_000usize;
        let start = Instant::now();
        for i in 0..n {
            cache.insert(format!("key{}", i), i);
        }
        let insert_elapsed = start.elapsed();
        let start = Instant::now();
        for i in 0..n {
            let _ = cache.get(&format!("key{}", i));
        }
        let get_elapsed = start.elapsed();
        println!(
            "concurrent_ttlcache insert for {} items: {:?}, get: {:?}",
            n, insert_elapsed, get_elapsed
        );
    }
}
