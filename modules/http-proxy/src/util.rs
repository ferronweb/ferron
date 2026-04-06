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
