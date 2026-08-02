//! Per-configuration caches and the spawned-task registry.
//!
//! State that must be shared across requests for one configuration
//! generation is keyed by the byte representation of the configuration
//! layer Arc pointers (see `stage.rs` for how the key is built). A config
//! reload allocates new Arc pointers, so reloaded configs arrive under
//! fresh keys. The old generation is discarded by `ProxyState::on_reload()`:
//! `PerConfigCache` entries are cleared and `TaskRegistry` tasks are aborted.

use dashmap::DashMap;
use rustc_hash::FxBuildHasher;

/// A cache keyed by configuration pointer identity.
///
/// Values are shared across requests for one configuration generation and
/// invalidated wholesale on reload via [`clear`](PerConfigCache::clear).
pub struct PerConfigCache<T> {
    inner: DashMap<Vec<usize>, T, FxBuildHasher>,
}

impl<T> Default for PerConfigCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PerConfigCache<T> {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_hasher(FxBuildHasher),
        }
    }

    /// Get the entry for `key`, or insert it from `init`.
    ///
    /// `init` runs at most once per key; concurrent callers receive the
    /// same value.
    pub fn get_or_insert_with(&self, key: &[usize], init: impl FnOnce() -> T) -> T
    where
        T: Clone,
    {
        self.inner.entry(key.to_vec()).or_insert_with(init).clone()
    }

    /// Get a clone of the entry for `key`, if present.
    pub fn get(&self, key: &[usize]) -> Option<T>
    where
        T: Clone,
    {
        self.inner.get(key).map(|entry| entry.clone())
    }

    /// Insert `value` for `key`.
    pub fn insert(&self, key: &[usize], value: T) {
        self.inner.insert(key.to_vec(), value);
    }

    /// Remove all entries.
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Number of entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Registry of spawned per-config tasks, aborted on reload.
///
/// Health check probe loops (and any future per-config background task)
/// are registered here so that a config reload cancels the old generation
/// instead of leaking one infinite loop per reload.
pub struct TaskRegistry {
    inner: DashMap<Vec<usize>, tokio::task::AbortHandle, FxBuildHasher>,
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_hasher(FxBuildHasher),
        }
    }

    /// Spawn and register the task for `key` if none is registered yet.
    ///
    /// `spawn` runs only when `key` is unregistered, so the operation is
    /// idempotent per configuration generation. Returns `true` when a new
    /// task was spawned.
    pub fn ensure(&self, key: &[usize], spawn: impl FnOnce() -> tokio::task::AbortHandle) -> bool {
        match self.inner.entry(key.to_vec()) {
            dashmap::Entry::Occupied(_) => false,
            dashmap::Entry::Vacant(entry) => {
                entry.insert(spawn());
                true
            }
        }
    }

    /// Whether `key` has a registered task.
    pub fn contains_key(&self, key: &[usize]) -> bool {
        self.inner.contains_key(key)
    }

    /// Abort every registered task and remove it from the registry.
    ///
    /// Handles are collected before aborting so no shard lock is held
    /// while a task is cancelled.
    pub fn abort_all(&self) {
        let handles: Vec<_> = self
            .inner
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        for handle in handles {
            handle.abort();
        }
        self.inner.clear();
    }

    /// Number of registered tasks.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{PerConfigCache, TaskRegistry};

    #[test]
    fn per_config_cache_keeps_keys_distinct() {
        let cache = PerConfigCache::new();
        let key_a = vec![1usize, 2, 3];
        let key_b = vec![4usize, 5, 6];

        assert_eq!(cache.get_or_insert_with(&key_a, || 10), 10);
        assert_eq!(cache.get_or_insert_with(&key_b, || 20), 20);
        assert_eq!(cache.len(), 2);

        assert_eq!(cache.get(&key_a), Some(10));
        assert_eq!(cache.get(&key_b), Some(20));
    }

    #[test]
    fn per_config_cache_init_runs_once_per_key() {
        let cache = PerConfigCache::new();
        let key = vec![1usize, 2, 3];
        let calls = Arc::new(AtomicUsize::new(0));

        let init = || {
            calls.fetch_add(1, Ordering::Relaxed);
            42
        };
        assert_eq!(cache.get_or_insert_with(&key, &init), 42);
        assert_eq!(cache.get_or_insert_with(&key, &init), 42);
        assert_eq!(cache.get_or_insert_with(&key, &init), 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn per_config_cache_clear_invalidates() {
        let cache = PerConfigCache::new();
        let key = vec![1usize, 2, 3];
        cache.insert(&key, 7);
        assert_eq!(cache.get(&key), Some(7));

        cache.clear();
        assert_eq!(cache.get(&key), None);
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn task_registry_ensure_is_idempotent() {
        let registry = TaskRegistry::new();
        let key = vec![1usize, 2, 3];
        let spawns = Arc::new(AtomicUsize::new(0));

        let spawn = || {
            spawns.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async {}).abort_handle()
        };
        assert!(registry.ensure(&key, spawn));
        assert!(!registry.ensure(&key, spawn));
        assert!(!registry.ensure(&key, spawn));
        assert_eq!(registry.len(), 1);
        assert_eq!(spawns.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn task_registry_abort_all_stops_tasks() {
        let registry = TaskRegistry::new();
        let key_a = vec![1usize, 2, 3];
        let key_b = vec![4usize, 5, 6];
        let count = Arc::new(AtomicUsize::new(0));

        let spawn = || {
            let counter = Arc::clone(&count);
            tokio::spawn(async move {
                loop {
                    counter.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .abort_handle()
        };
        assert!(registry.ensure(&key_a, spawn));
        assert!(registry.ensure(&key_b, spawn));

        tokio::time::sleep(Duration::from_millis(5)).await;
        registry.abort_all();
        assert_eq!(registry.len(), 0);

        let before = count.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let after = count.load(Ordering::Relaxed);
        assert_eq!(
            before, after,
            "aborted task must stop incrementing the counter"
        );
    }
}
