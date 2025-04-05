use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct TtlCache<K, V> {
  cache: HashMap<K, (V, Instant)>,
  ttl: Duration,
}

impl<K, V> TtlCache<K, V>
where
  K: std::cmp::Eq + std::hash::Hash + Clone,
  V: Clone,
{
  pub fn new(ttl: Duration) -> Self {
    Self {
      cache: HashMap::new(),
      ttl,
    }
  }

  pub fn insert(&mut self, key: K, value: V) {
    self.cache.insert(key, (value, Instant::now()));
  }

  pub fn get(&self, key: &K) -> Option<V> {
    self.cache.get(key).and_then(|(value, timestamp)| {
      if timestamp.elapsed() < self.ttl {
        Some(value.clone())
      } else {
        None
      }
    })
  }

  #[allow(dead_code)]
  pub fn remove(&mut self, key: &K) -> Option<V> {
    self.cache.remove(key).map(|(value, _)| value)
  }

  pub fn cleanup(&mut self) {
    self
      .cache
      .retain(|_, (_, timestamp)| timestamp.elapsed() < self.ttl);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::thread::sleep;
  use std::time::Duration;

  #[test]
  fn test_insert_and_get() {
    let mut cache = TtlCache::new(Duration::new(5, 0));
    cache.insert("key1", "value1");

    assert_eq!(cache.get(&"key1"), Some("value1"));
  }

  #[test]
  fn test_get_expired() {
    let mut cache = TtlCache::new(Duration::new(1, 0));
    cache.insert("key1", "value1");

    // Sleep for 2 seconds to ensure the entry expires
    sleep(Duration::new(2, 0));

    assert_eq!(cache.get(&"key1"), None);
  }

  #[test]
  fn test_remove() {
    let mut cache = TtlCache::new(Duration::new(5, 0));
    cache.insert("key1", "value1");
    cache.remove(&"key1");

    assert_eq!(cache.get(&"key1"), None);
  }

  #[test]
  fn test_cleanup() {
    let mut cache = TtlCache::new(Duration::new(1, 0));
    cache.insert("key1", "value1");
    cache.insert("key2", "value2");

    // Sleep for 2 seconds to ensure the entries expire
    sleep(Duration::new(2, 0));

    cache.cleanup();

    assert_eq!(cache.get(&"key1"), None);
    assert_eq!(cache.get(&"key2"), None);
  }

  #[test]
  fn test_get_non_existent() {
    let cache: TtlCache<&str, &str> = TtlCache::new(Duration::new(5, 0));
    assert_eq!(cache.get(&"key1"), None);
  }

  #[test]
  fn test_insert_and_get_multiple() {
    let mut cache = TtlCache::new(Duration::new(5, 0));
    cache.insert("key1", "value1");
    cache.insert("key2", "value2");

    assert_eq!(cache.get(&"key1"), Some("value1"));
    assert_eq!(cache.get(&"key2"), Some("value2"));
  }
}
