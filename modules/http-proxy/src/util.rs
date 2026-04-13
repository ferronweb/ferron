//! Utility types for the reverse proxy module.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A TTL (time-to-live) cache.
pub struct TtlCache<K, V> {
    cache: HashMap<K, (V, Instant)>,
    ttl: Duration,
}

impl<K, V> TtlCache<K, V>
where
    K: std::cmp::Eq + std::hash::Hash + Clone,
    V: Clone,
{
    /// Creates a new TTL cache.
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: HashMap::new(),
            ttl,
        }
    }

    /// Inserts a value into the cache.
    pub fn insert(&mut self, key: K, value: V) {
        self.cache.insert(key, (value, Instant::now()));
    }

    /// Gets a value from the cache, returning `None` if expired.
    pub fn get(&self, key: &K) -> Option<V> {
        self.cache.get(key).and_then(|(value, timestamp)| {
            if timestamp.elapsed() < self.ttl {
                Some(value.clone())
            } else {
                None
            }
        })
    }

    /// Removes a value from the cache.
    #[allow(dead_code)]
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.cache.remove(key).map(|(value, _)| value)
    }

    /// Removes all expired entries.
    #[allow(dead_code)]
    pub fn cleanup(&mut self) {
        self.cache
            .retain(|_, (_, timestamp)| timestamp.elapsed() < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_insert_and_get() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        assert_eq!(cache.get(&"key1"), Some("value1"));
    }

    #[test]
    fn test_get_nonexistent_key() {
        let cache: TtlCache<&str, &str> = TtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.get(&"missing"), None);
    }

    #[test]
    fn test_expired_entry() {
        let mut cache = TtlCache::new(Duration::from_millis(50));
        cache.insert("key1", "value1");
        assert_eq!(cache.get(&"key1"), Some("value1"));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_overwrite_key() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        cache.insert("key1", "value2");
        assert_eq!(cache.get(&"key1"), Some("value2"));
    }

    #[test]
    fn test_remove() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key1", "value1");
        assert_eq!(cache.remove(&"key1"), Some("value1"));
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut cache: TtlCache<&str, &str> = TtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.remove(&"missing"), None);
    }

    #[test]
    fn test_cleanup() {
        let mut cache = TtlCache::new(Duration::from_millis(50));
        cache.insert("key1", "value1");
        cache.insert("key2", "value2");

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        cache.insert("key3", "value3");
        cache.cleanup();

        assert_eq!(cache.get(&"key1"), None);
        assert_eq!(cache.get(&"key2"), None);
        assert_eq!(cache.get(&"key3"), Some("value3"));
    }

    #[test]
    fn test_multiple_entries() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        for i in 0..100 {
            cache.insert(format!("key{}", i), i);
        }
        for i in 0..100 {
            assert_eq!(cache.get(&format!("key{}", i)), Some(i));
        }
        assert_eq!(cache.cache.len(), 100);
    }

    #[test]
    fn bench_ttlcache_insert_get() {
        use std::time::Instant;
        let mut cache = TtlCache::new(Duration::from_secs(60));
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
        println!("ttlcache insert for {} items: {:?}, get: {:?}", n, insert_elapsed, get_elapsed);
    }
}
