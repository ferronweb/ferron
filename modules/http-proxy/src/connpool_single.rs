//! Single-threaded connection pool.
//!
//! This replaces the concurrent `connpool` with a simple, non-synchronized pool
//! designed for thread-per-core runtimes where each thread owns its pool exclusively.

use std::cell::UnsafeCell;
use std::hash::Hash;

use rustc_hash::FxHashMap;

/// A single-threaded connection pool.
///
/// # Thread Safety
///
/// This type uses `UnsafeCell` internally and must be confined to a single thread.
/// It is marked `!Send` and `!Sync` to enforce this.
pub struct SingleThreadPool<K, L, I> {
    /// Idle connections stored per key (LIFO order for cache locality).
    idle: UnsafeCell<FxHashMap<K, Vec<I>>>,
    /// Number of connections currently outstanding (pulled but not returned).
    outstanding: UnsafeCell<usize>,
    /// Total number of idle connections across all keys.
    idle_total: UnsafeCell<usize>,
    /// Maximum total connections (idle + outstanding).
    max_size: UnsafeCell<usize>,
    /// Whether the pool is unbounded (no max_size limit).
    unbounded: UnsafeCell<bool>,
    /// Per-limit-key outstanding counts.
    local_outstanding: UnsafeCell<FxHashMap<L, usize>>,
    // Prevent Send/Sync auto-implementation.
    _marker: std::marker::PhantomData<*mut ()>,
}

impl<K, L, I> SingleThreadPool<K, L, I> {
    /// Creates a new connection pool with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            idle: UnsafeCell::new(FxHashMap::default()),
            outstanding: UnsafeCell::new(0),
            idle_total: UnsafeCell::new(0),
            max_size: UnsafeCell::new(capacity),
            unbounded: UnsafeCell::new(false),
            local_outstanding: UnsafeCell::new(FxHashMap::default()),
            _marker: std::marker::PhantomData,
        }
    }

    /// Creates a new connection pool with no maximum capacity.
    pub fn new_unbounded() -> Self {
        Self {
            idle: UnsafeCell::new(FxHashMap::default()),
            outstanding: UnsafeCell::new(0),
            idle_total: UnsafeCell::new(0),
            max_size: UnsafeCell::new(0), // unused when unbounded
            unbounded: UnsafeCell::new(true),
            local_outstanding: UnsafeCell::new(FxHashMap::default()),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<K, L, I> SingleThreadPool<K, L, I>
where
    K: Eq + Hash,
    L: Eq + Hash,
{
    /// Safety: All these methods require single-threaded access.
    /// The caller must ensure no concurrent access occurs.
    #[allow(clippy::mut_from_ref)]
    unsafe fn idle_map(&self) -> &mut FxHashMap<K, Vec<I>> {
        &mut *self.idle.get()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn outstanding(&self) -> &mut usize {
        &mut *self.outstanding.get()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn idle_total(&self) -> &mut usize {
        &mut *self.idle_total.get()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn local_outstanding(&self) -> &mut FxHashMap<L, usize> {
        &mut *self.local_outstanding.get()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn max_size_mut(&self) -> &mut usize {
        &mut *self.max_size.get()
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn unbounded_mut(&self) -> &mut bool {
        &mut *self.unbounded.get()
    }

    /// Updates the pool's maximum capacity.
    ///
    /// - If the capacity is increased, new connections can be established up to the new limit.
    /// - If the capacity is decreased, excess idle connections are evicted (dropped) to fit within the new limit.
    ///   Outstanding (in-flight) connections are not affected — they are allowed to complete normally.
    pub fn update_capacity(&self, new_capacity: usize) {
        let old_max = *unsafe { self.max_size_mut() };
        *unsafe { self.max_size_mut() } = new_capacity;

        // If capacity decreased, evict excess idle connections.
        if new_capacity < old_max {
            self.evict_excess_idle(new_capacity);
        }
    }

    /// Evicts excess idle connections to fit within the given capacity.
    ///
    /// This removes idle connections to fit within the limit.
    /// Outstanding (in-flight) connections are not affected.
    fn evict_excess_idle(&self, max_capacity: usize) {
        let idle = unsafe { self.idle_map() };
        let outstanding = *unsafe { self.outstanding() };
        let current_idle = *unsafe { self.idle_total() };

        // Calculate maximum idle connections allowed.
        let max_idle = max_capacity.saturating_sub(outstanding);

        if current_idle <= max_idle {
            return; // No eviction needed.
        }

        let mut to_evict = current_idle - max_idle;
        let idle_total = unsafe { self.idle_total() };

        // Evict connections across all keys.
        for conns in idle.values_mut() {
            if to_evict == 0 {
                break;
            }

            let evict_from_this = std::cmp::min(to_evict, conns.len());
            if evict_from_this == 0 {
                continue;
            }

            let keep = conns.len() - evict_from_this;
            conns.truncate(keep);
            *idle_total = idle_total.saturating_sub(evict_from_this);
            to_evict -= evict_from_this;
        }

        // Clean up empty keys.
        idle.retain(|_, conns| !conns.is_empty());
    }

    /// Returns the number of idle connections for a given key.
    pub fn idle_count(&self, key: &K) -> usize {
        let idle = unsafe { self.idle_map() };
        idle.get(key).map_or(0, Vec::len)
    }

    /// Returns the total number of idle connections.
    pub fn total_idle_count(&self) -> usize {
        *unsafe { self.idle_total() }
    }

    /// Returns the number of outstanding connections.
    pub fn outstanding_count(&self) -> usize {
        *unsafe { self.outstanding() }
    }

    /// Returns the maximum pool size (if bounded).
    pub fn max_size(&self) -> Option<usize> {
        if *unsafe { self.unbounded_mut() } {
            None
        } else {
            Some(*unsafe { self.max_size_mut() })
        }
    }

    /// Checks if the pool is at its global capacity limit for new connection creation.
    fn is_at_global_limit(&self) -> bool {
        if *unsafe { self.unbounded_mut() } {
            false
        } else {
            (*unsafe { self.outstanding() }).saturating_add(*unsafe { self.idle_total() })
                >= *unsafe { self.max_size_mut() }
        }
    }

    /// Checks if a local limit is reached for a given limit key.
    fn is_at_local_limit(&self, limit_key: &L, local_limit: usize) -> bool {
        let counts = unsafe { self.local_outstanding() };
        counts.get(limit_key).copied().unwrap_or(0) >= local_limit
    }
}

impl<K, L, I> SingleThreadPool<K, L, I>
where
    K: Eq + Hash + Clone,
    L: Eq + Hash + Clone,
{
    /// Increments the local outstanding count for a limit key.
    fn increment_local_outstanding(&self, limit_key: &L) {
        let counts = unsafe { self.local_outstanding() };
        *counts.entry(limit_key.clone()).or_insert(0) += 1;
    }

    /// Decrements the local outstanding count for a limit key.
    fn decrement_local_outstanding(&self, limit_key: &L) {
        if let Some(count) = unsafe { self.local_outstanding() }.get_mut(limit_key) {
            *count = count.saturating_sub(1);
        }
    }

    /// Pulls an item from the pool, returning it immediately.
    ///
    /// Returns `None` if the global limit is reached (caller should establish a new connection).
    pub fn pull(&self, key: K) -> Option<PoolItem<K, L, I>> {
        self.pull_with_local_limit(key, None)
    }

    /// Pulls an item from the pool with a local limit applied.
    ///
    /// Returns `None` if either the global limit or local limit is reached.
    pub fn pull_with_local_limit(
        &self,
        key: K,
        local_limit: Option<(L, usize)>,
    ) -> Option<PoolItem<K, L, I>> {
        let local_limit_key = local_limit.as_ref().map(|(limit_key, _)| limit_key.clone());

        // Check local limit if specified.
        if let Some((limit_key, limit)) = local_limit.as_ref() {
            if self.is_at_local_limit(limit_key, *limit) {
                return None;
            }
        }

        // Try to get an idle connection.
        let inner = unsafe { self.idle_map() }.get_mut(&key).and_then(|conns| {
            let inner = conns.pop();
            if inner.is_some() {
                *unsafe { self.idle_total() } = unsafe { self.idle_total() }.saturating_sub(1);
            }
            inner
        });

        if inner.is_none() && self.is_at_global_limit() {
            return None;
        }

        // Increment outstanding.
        *unsafe { self.outstanding() } += 1;

        // Increment local outstanding if applicable.
        if let Some(ref limit_key) = local_limit_key {
            self.increment_local_outstanding(limit_key);
        }

        Some(PoolItem {
            pool: self,
            key: Some(key),
            inner,
            local_limit_key,
            _marker: std::marker::PhantomData,
        })
    }

    /// Returns a connection to the pool.
    ///
    /// If the pool is at capacity, the connection is dropped instead.
    pub fn return_connection(&self, key: K, inner: I) {
        let outstanding_before = *unsafe { self.outstanding() };
        let idle_total_before = *unsafe { self.idle_total() };

        // Decrement outstanding.
        *unsafe { self.outstanding() } = outstanding_before.saturating_sub(1);

        // Check if we can store the connection.
        let can_store = if *unsafe { self.unbounded_mut() } {
            true
        } else {
            outstanding_before.saturating_add(idle_total_before) <= *unsafe { self.max_size_mut() }
        };

        if can_store {
            unsafe { self.idle_map() }
                .entry(key)
                .or_default()
                .push(inner);
            *unsafe { self.idle_total() } += 1;
        }
        // else: drop the connection (it will be dropped when this function ends)
    }

    /// Returns a connection to the pool with local limit tracking.
    pub fn return_connection_with_local_limit(&self, key: K, inner: I, local_limit_key: Option<L>) {
        if let Some(ref limit_key) = local_limit_key {
            self.decrement_local_outstanding(limit_key);
        }

        self.return_connection(key, inner);
    }
}

/// An item pulled from the connection pool.
///
/// When dropped, the item is automatically returned to the pool (if it still contains a value).
///
/// # Thread Safety
///
/// `PoolItem` holds a raw pointer to the pool and must be dropped on the same thread
/// that created it. It is marked `!Send` to enforce this.
pub struct PoolItem<K: Eq + Hash + Clone, L: Eq + Hash + Clone, I> {
    /// Back-pointer to the pool (safe because PoolItem must be dropped on the same thread).
    pool: *const SingleThreadPool<K, L, I>,
    /// The key this item was pulled for.
    key: Option<K>,
    /// The connection value (may be None if pool was exhausted).
    inner: Option<I>,
    /// Local limit key, if one was applied during pull.
    local_limit_key: Option<L>,
    // Prevent Send auto-implementation.
    _marker: std::marker::PhantomData<*mut ()>,
}

impl<K: Eq + Hash + Clone, L: Eq + Hash + Clone, I> PoolItem<K, L, I> {
    /// Takes the inner value from the item, preventing it from being returned to the pool.
    pub fn take(mut self) -> Option<I> {
        self.inner.take()
    }

    /// Returns a reference to the inner value.
    pub fn inner(&self) -> &Option<I> {
        &self.inner
    }

    /// Returns a mutable reference to the inner value.
    pub fn inner_mut(&mut self) -> &mut Option<I> {
        &mut self.inner
    }

    /// Returns a mutable reference to the inner value, with a shorter name for ergonomics.
    pub fn get_mut(&mut self) -> &mut Option<I> {
        &mut self.inner
    }

    /// Returns a reference to the pool key.
    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
    }

    /// Returns the local limit key, if one was applied.
    pub fn local_limit_key(&self) -> Option<&L> {
        self.local_limit_key.as_ref()
    }

    /// Returns the raw pool pointer (for debugging/advanced use).
    pub fn pool_ptr(&self) -> *const SingleThreadPool<K, L, I> {
        self.pool
    }
}

impl<K: Eq + Hash + Clone, L: Eq + Hash + Clone, I> SingleThreadPool<K, L, I> {
    /// Decrements the outstanding count (used when dropping an item without an inner value).
    fn decrement_outstanding(&self) {
        *unsafe { self.outstanding() } = unsafe { self.outstanding() }.saturating_sub(1);
    }
}

impl<K: Eq + Hash + Clone, L: Eq + Hash + Clone, I> Drop for PoolItem<K, L, I> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            // Safety: PoolItem is guaranteed to be dropped on the same thread as the pool.
            let pool = unsafe { &*self.pool };

            if let Some(inner) = self.inner.take() {
                pool.return_connection_with_local_limit(key, inner, self.local_limit_key.take());
            } else {
                pool.decrement_outstanding();
                if let Some(limit_key) = self.local_limit_key.take() {
                    pool.decrement_local_outstanding(&limit_key);
                }
            }
        }
    }
}

// Note: `PhantomData<*mut ()>` ensures `!Send` and `!Sync` automatically.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_new() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);
        assert_eq!(pool.max_size(), Some(10));
        assert_eq!(pool.outstanding_count(), 0);
    }

    #[test]
    fn test_pool_unbounded() {
        let pool = SingleThreadPool::<String, String, u32>::new_unbounded();
        assert_eq!(pool.max_size(), None);
    }

    #[test]
    fn test_pull_and_return() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        // Pull an item (will be None since no connections stored)
        let item = pool.pull("key1".to_string()).unwrap();
        assert!(item.inner().is_none());
        assert_eq!(pool.outstanding_count(), 1);

        // Item is dropped, should return to pool (but no inner value, so nothing stored)
        drop(item);
        assert_eq!(pool.outstanding_count(), 0);
    }

    #[test]
    fn test_pull_with_inner_value() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        // Manually return a connection
        pool.return_connection("key1".to_string(), 42);
        assert_eq!(pool.idle_count(&"key1".to_string()), 1);

        // Pull it back
        let item = pool.pull("key1".to_string()).unwrap();
        assert_eq!(item.inner(), &Some(42));
        assert_eq!(pool.outstanding_count(), 1);
        assert_eq!(pool.idle_count(&"key1".to_string()), 0);
    }

    #[test]
    fn test_global_limit() {
        let pool = SingleThreadPool::<String, String, u32>::new(2);

        // Fill the pool
        let item1 = pool.pull("key1".to_string()).unwrap();
        let item2 = pool.pull("key2".to_string()).unwrap();

        assert_eq!(pool.outstanding_count(), 2);

        // Should be at limit for new connections.
        let item3 = pool.pull("key3".to_string());
        assert!(item3.is_none());

        drop(item1);
        drop(item2);

        assert_eq!(pool.outstanding_count(), 0);
    }

    #[test]
    fn test_local_limit() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        let limit_key = "upstream-a".to_string();

        // Pull two items with local limit
        let item1 = pool
            .pull_with_local_limit("key1".to_string(), Some((limit_key.clone(), 2)))
            .unwrap();
        let item2 = pool
            .pull_with_local_limit("key1".to_string(), Some((limit_key.clone(), 2)))
            .unwrap();

        // Third should fail local limit
        let item3 = pool.pull_with_local_limit("key1".to_string(), Some((limit_key, 2)));
        assert!(item3.is_none());

        drop(item1);
        drop(item2);
    }

    #[test]
    fn test_take_prevents_return() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        pool.return_connection("key1".to_string(), 42);

        let item = pool.pull("key1".to_string()).unwrap();
        let value = item.take().unwrap();
        assert_eq!(value, 42);

        // Item was taken, should not be in pool
        assert_eq!(pool.idle_count(&"key1".to_string()), 0);
    }

    #[test]
    fn test_unbounded_pool() {
        let pool = SingleThreadPool::<String, String, u32>::new_unbounded();

        // Can pull many items without hitting limit
        let mut items = Vec::new();
        for i in 0..100 {
            let item = pool.pull(format!("key{i}")).unwrap();
            items.push(item);
        }

        assert_eq!(pool.outstanding_count(), 100);
    }

    #[test]
    fn test_update_capacity_increase() {
        let pool = SingleThreadPool::<String, String, u32>::new(2);
        assert_eq!(pool.max_size(), Some(2));

        // Fill the pool
        let item1 = pool.pull("key1".to_string()).unwrap();
        let item2 = pool.pull("key2".to_string()).unwrap();
        assert!(pool.pull("key3".to_string()).is_none()); // At limit

        // Increase capacity
        pool.update_capacity(5);
        assert_eq!(pool.max_size(), Some(5));

        // Should now be able to pull more
        let item3 = pool.pull("key3".to_string()).unwrap();
        drop(item1);
        drop(item2);
        drop(item3);
    }

    #[test]
    fn test_update_capacity_decrease_evicts_idle() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        // Add idle connections
        pool.return_connection("key1".to_string(), 1);
        pool.return_connection("key2".to_string(), 2);
        pool.return_connection("key3".to_string(), 3);
        assert_eq!(pool.total_idle_count(), 3);

        // Decrease capacity to 1
        pool.update_capacity(1);
        assert_eq!(pool.max_size(), Some(1));

        // Should have evicted 2 idle connections (only 1 can fit)
        assert_eq!(pool.total_idle_count(), 1);
    }

    #[test]
    fn test_update_capacity_decrease_no_evict_when_outstanding() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        // Pull 5 connections (all outstanding, no idle)
        let _items: Vec<_> = (0..5)
            .map(|i| pool.pull(format!("key{i}")).unwrap())
            .collect();
        assert_eq!(pool.outstanding_count(), 5);
        assert_eq!(pool.total_idle_count(), 0);

        // Decrease capacity below outstanding
        pool.update_capacity(3);
        assert_eq!(pool.max_size(), Some(3));

        // No idle to evict, outstanding stays at 5
        assert_eq!(pool.outstanding_count(), 5);
    }

    #[test]
    fn test_local_limit_shared_by_limit_key_not_pool_key() {
        let pool = SingleThreadPool::<String, String, u32>::new(10);

        let limit_key = "upstream-a".to_string();
        let item1 = pool
            .pull_with_local_limit("key1".to_string(), Some((limit_key.clone(), 1)))
            .unwrap();

        // Same limit key should be shared across different pool keys.
        assert!(pool
            .pull_with_local_limit("key2".to_string(), Some((limit_key, 1)))
            .is_none());

        drop(item1);
    }

    #[test]
    fn test_global_limit_allows_idle_reuse() {
        let pool = SingleThreadPool::<String, String, u32>::new(1);

        pool.return_connection("key1".to_string(), 7);
        assert_eq!(pool.total_idle_count(), 1);

        // Reusing the idle connection is allowed even though the pool is at capacity.
        let item = pool.pull("key1".to_string()).unwrap();
        assert_eq!(item.inner(), &Some(7));
        assert_eq!(pool.total_idle_count(), 0);

        // But a brand-new connection is still blocked at capacity.
        assert!(pool.pull("key2".to_string()).is_none());

        drop(item);
        assert_eq!(pool.total_idle_count(), 1);
    }
}
