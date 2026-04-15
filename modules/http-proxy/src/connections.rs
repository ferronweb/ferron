//! Connection pool using thread-local storage.
//!
//! This replaces the concurrent `connpool` with a simple, single-threaded pool
//! stored in thread-local storage. Each thread owns its own pool exclusively,
//! eliminating synchronization overhead entirely.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use rustc_hash::FxHasher;

use crate::connpool_single::{PoolItem, SingleThreadPool};
use crate::send_request::SendRequestWrapper;
use crate::upstream::UpstreamInner;

/// Connection pool key type: (upstream via Arc for cheap cloning, optional client IP for PROXY protocol).
pub type PoolKey = (Arc<UpstreamInner>, Option<IpAddr>);

/// Thread-local pool storage.
///
/// Since we use a thread-per-core runtime, each thread gets its own pool.
/// The pools are stored in `RefCell` for interior mutability within the thread.
struct ThreadLocalPools {
    /// Sharded TCP connection pools (for distributing global limit across threads).
    tcp_shards: Vec<RefCell<SingleThreadPool<PoolKey, SendRequestWrapper>>>,
    /// Unix socket pool (unbounded, separate from TCP pools).
    #[cfg(unix)]
    unix_pool: RefCell<SingleThreadPool<PoolKey, SendRequestWrapper>>,
}

// Thread-local storage for connection pools.
thread_local! {
    static TLS_POOLS: RefCell<Option<ThreadLocalPools>> = const { RefCell::new(None) };
}

/// Connection pool manager for the reverse proxy.
///
/// This manager coordinates thread-local pools with a global concurrent limit.
/// Each thread owns its own pool instance, eliminating cross-thread contention.
pub struct ConnectionManager {
    /// Global limit shared across all threads. Uses `AtomicUsize` for thread-safe interior mutability.
    global_limit: AtomicUsize,
    /// Number of shards (typically matches CPU count).
    shards: usize,
    /// Per-upstream local limit indices (read-only after initialization).
    local_limits: RwLock<HashMap<UpstreamInner, usize>>,
}

impl ConnectionManager {
    pub fn with_global_limit(global_limit: usize) -> Self {
        Self::with_global_limit_and_shards(global_limit, std::cmp::max(1, num_cpus::get()))
    }

    /// Like with_global_limit but allows overriding the number of shards (useful for tests/benches).
    pub fn with_global_limit_and_shards(global_limit: usize, shards: usize) -> Self {
        let shards = std::cmp::max(1, shards);

        // Initialize thread-local pools lazily on first access
        Self {
            global_limit: AtomicUsize::new(global_limit),
            shards,
            local_limits: RwLock::new(HashMap::new()),
        }
    }

    /// Ensures thread-local pools are initialized for the current thread.
    fn ensure_tls_pools(&self) {
        TLS_POOLS.with(|tls| {
            let mut guard = tls.borrow_mut();
            if guard.is_some() {
                return;
            }

            let limit = self.global_limit.load(Ordering::Relaxed);
            let per_shard = if self.shards > 0 {
                limit.div_ceil(self.shards)
            } else {
                limit
            };

            let mut tcp_shards = Vec::with_capacity(self.shards);
            for _ in 0..self.shards {
                tcp_shards.push(RefCell::new(SingleThreadPool::new(per_shard)));
            }

            *guard = Some(ThreadLocalPools {
                tcp_shards,
                #[cfg(unix)]
                unix_pool: RefCell::new(SingleThreadPool::new_unbounded()),
            });
        });
    }

    /// Set a per-upstream local connection limit.
    /// This sets the same local-limit index across all shards for compatibility.
    pub fn set_local_limit(&self, upstream: &UpstreamInner, limit: usize) -> usize {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");

        if let Some(&idx) = limits.get(upstream) {
            return idx;
        }

        // Apply local limit to the current thread's pools
        self.ensure_tls_pools();
        let mut idx = 0usize;

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            for (i, shard) in pools.tcp_shards.iter().enumerate() {
                let ret = shard.borrow_mut().set_local_limit(limit);
                if i == 0 {
                    idx = ret;
                }
            }

            #[cfg(unix)]
            let _ = pools.unix_pool.borrow_mut().set_local_limit(limit);
        });

        limits.insert(upstream.clone(), idx);
        idx
    }

    /// Get the local limit index for an upstream.
    pub fn get_local_limit(&self, upstream: &UpstreamInner) -> Option<usize> {
        self.local_limits
            .read()
            .expect("local_limits lock poisoned")
            .get(upstream)
            .copied()
    }

    /// Updates the global concurrent connections limit.
    ///
    /// This recalculates the per-shard capacity and updates all thread-local pools.
    /// If the new limit is lower, excess idle connections are evicted from all pools.
    /// If the new limit is higher, pools can grow to the new capacity.
    pub fn update_global_limit(&self, new_limit: usize) {
        // Update the stored global limit
        self.global_limit.store(new_limit, Ordering::Relaxed);

        // Iterate over all thread-local pools and update their capacities
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            if let Some(pools) = guard.as_ref() {
                let per_shard = if self.shards > 0 {
                    new_limit.div_ceil(self.shards)
                } else {
                    new_limit
                };

                for shard in pools.tcp_shards.iter() {
                    shard.borrow_mut().update_capacity(per_shard);
                }
            }
        });
    }

    /// Updates the local limit for a specific upstream.
    ///
    /// If the upstream already has a local limit index, the limit is updated in place
    /// across all thread-local pools. If it doesn't exist, a new local limit is created.
    pub fn update_local_limit_for_upstream(&self, upstream: &UpstreamInner, new_limit: usize) {
        let limits = self
            .local_limits
            .read()
            .expect("local_limits lock poisoned");

        if let Some(&idx) = limits.get(upstream) {
            // Update existing local limit across all thread-local pools
            TLS_POOLS.with(|tls| {
                let guard = tls.borrow();
                if let Some(pools) = guard.as_ref() {
                    for shard in pools.tcp_shards.iter() {
                        let shard_mut = shard.borrow_mut();
                        // Update the limit value at the existing index
                        let local_limits = unsafe { shard_mut.local_limits_mut() };
                        if idx < local_limits.len() {
                            local_limits[idx] = new_limit;
                        }
                    }

                    #[cfg(unix)]
                    {
                        let unix_mut = pools.unix_pool.borrow_mut();
                        let local_limits = unsafe { unix_mut.local_limits_mut() };
                        if idx < local_limits.len() {
                            local_limits[idx] = new_limit;
                        }
                    }
                }
            });
        } else {
            // Need to create a new local limit
            drop(limits);
            let _ = self.set_local_limit(upstream, new_limit);
        }
    }

    /// Pull a connection from the pool, returning immediately.
    ///
    /// Unlike the old connpool-based version, this is **synchronous** and returns
    /// `None` if the pool is at capacity (caller should establish a new connection).
    #[allow(dead_code)]
    pub fn pull(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
    ) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        self.ensure_tls_pools();

        let key = (Arc::new(upstream.clone()), client_ip);

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return TLS_POOLS.with(|tls| {
                let guard = tls.borrow();
                let pools = guard.as_ref().unwrap();
                let result = pools.unix_pool.borrow_mut().pull(key);
                result
            });
        }

        let shard = self.shard_for_key(&key);

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();
            let result = pools.tcp_shards[shard].borrow_mut().pull(key);
            result
        })
    }

    /// Pull a connection with a local limit applied, returning immediately.
    ///
    /// Unlike the old connpool-based version, this is **synchronous** and returns
    /// `None` if the local or global limit is reached.
    #[allow(dead_code)]
    pub fn pull_with_local_limit(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
        local_limit_idx: Option<usize>,
    ) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        self.ensure_tls_pools();

        let key = (Arc::new(upstream.clone()), client_ip);

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return TLS_POOLS.with(|tls| {
                let guard = tls.borrow();
                let pools = guard.as_ref().unwrap();
                let result = pools
                    .unix_pool
                    .borrow_mut()
                    .pull_with_local_limit(key, local_limit_idx);
                result
            });
        }

        let shard = self.shard_for_key(&key);

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();
            let result = pools.tcp_shards[shard]
                .borrow_mut()
                .pull_with_local_limit(key, local_limit_idx);
            result
        })
    }

    /// Select the sharded pool for the given upstream/client key.
    ///
    /// Returns a `PoolRef` which provides access to the thread-local pool
    /// for pulling connections.
    pub fn select_pool(&self, upstream: &UpstreamInner, client_ip: Option<IpAddr>) -> PoolRef {
        self.ensure_tls_pools();

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return PoolRef::Unix;
        }

        let key = (Arc::new(upstream.clone()), client_ip);
        PoolRef::Tcp(self.shard_for_key(&key))
    }

    /// Select the sharded pool for the given upstream/client key, returning a reference to the underlying pool.
    pub fn select_pool_ref(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
    ) -> PoolRefMut {
        self.ensure_tls_pools();

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return PoolRefMut::Unix;
        }

        let key = (Arc::new(upstream.clone()), client_ip);
        PoolRefMut::Tcp(self.shard_for_key(&key))
    }

    fn shard_for_key(&self, key: &PoolKey) -> usize {
        let mut hasher = FxHasher::default();
        (key, std::thread::current().id()).hash(&mut hasher);
        (hasher.finish() as usize) % self.shards
    }

    /// Returns a reference to the first shard's pool for backwards compatibility.
    #[allow(dead_code)]
    pub fn connections(&self) -> &SingleThreadPool<PoolKey, SendRequestWrapper> {
        self.ensure_tls_pools();
        // This is a bit tricky since we can't return a reference to TLS
        // Instead, we provide a PoolRef that can access it
        panic!("connections() is deprecated - use select_pool() instead");
    }

    #[cfg(unix)]
    #[allow(dead_code)]
    pub fn unix_connections(&self) -> &SingleThreadPool<PoolKey, SendRequestWrapper> {
        self.ensure_tls_pools();
        panic!("unix_connections() is deprecated - use select_pool() instead");
    }
}

/// A reference to a thread-local pool shard.
///
/// This type is returned by `ConnectionManager::select_pool()` and provides
/// access to the appropriate pool shard based on the upstream/client key.
pub enum PoolRef {
    /// TCP pool, index into the shard array.
    Tcp(usize),
    /// Unix socket pool (unbounded).
    #[cfg(unix)]
    Unix,
}

impl PoolRef {
    /// Returns whether this is a Unix socket pool.
    pub fn is_unix(&self) -> bool {
        match self {
            #[cfg(unix)]
            PoolRef::Unix => true,
            PoolRef::Tcp(_) => false,
        }
    }

    /// Returns the TCP shard index (only valid if not a Unix pool).
    pub fn tcp_shard_idx(&self) -> usize {
        match self {
            PoolRef::Tcp(idx) => *idx,
            #[cfg(unix)]
            PoolRef::Unix => 0, // unused
        }
    }

    /// Pull a connection from the referenced pool.
    pub fn pull(&self, key: PoolKey) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let result = match self {
                PoolRef::Tcp(shard_idx) => pools.tcp_shards[*shard_idx].borrow_mut().pull(key),
                #[cfg(unix)]
                PoolRef::Unix => pools.unix_pool.borrow_mut().pull(key),
            };
            result
        })
    }

    /// Pull a connection with a local limit applied.
    pub fn pull_with_local_limit(
        &self,
        key: PoolKey,
        local_limit_idx: Option<usize>,
    ) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let result = match self {
                PoolRef::Tcp(shard_idx) => pools.tcp_shards[*shard_idx]
                    .borrow_mut()
                    .pull_with_local_limit(key, local_limit_idx),
                #[cfg(unix)]
                PoolRef::Unix => pools
                    .unix_pool
                    .borrow_mut()
                    .pull_with_local_limit(key, local_limit_idx),
            };
            result
        })
    }
}

/// A mutable reference to a thread-local pool shard.
///
/// This type is returned by `ConnectionManager::select_pool_ref()` and provides
/// direct access to the underlying `SingleThreadPool` for pulling connections.
pub enum PoolRefMut {
    /// TCP pool, index into the shard array.
    Tcp(usize),
    /// Unix socket pool (unbounded).
    #[cfg(unix)]
    Unix,
}

impl PoolRefMut {
    /// Pull a connection from the referenced pool.
    pub fn pull(&self, key: PoolKey) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let result = match self {
                PoolRefMut::Tcp(shard_idx) => pools.tcp_shards[*shard_idx].borrow_mut().pull(key),
                #[cfg(unix)]
                PoolRefMut::Unix => pools.unix_pool.borrow_mut().pull(key),
            };
            result
        })
    }

    /// Pull a connection with a local limit applied.
    pub fn pull_with_local_limit(
        &self,
        key: PoolKey,
        local_limit_idx: Option<usize>,
    ) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let result = match self {
                PoolRefMut::Tcp(shard_idx) => pools.tcp_shards[*shard_idx]
                    .borrow_mut()
                    .pull_with_local_limit(key, local_limit_idx),
                #[cfg(unix)]
                PoolRefMut::Unix => pools
                    .unix_pool
                    .borrow_mut()
                    .pull_with_local_limit(key, local_limit_idx),
            };
            result
        })
    }
}

/// Return a connection to the thread-local pool.
///
/// This is used by `TrackedBody` to return connections after the response body
/// is fully consumed.
pub fn return_connection_to_pool(
    key: &PoolKey,
    wrapper: SendRequestWrapper,
    local_limit_idx: Option<usize>,
    is_unix: bool,
    tcp_shard_idx: usize,
) {
    TLS_POOLS.with(|tls| {
        let guard = tls.borrow();
        let pools = match guard.as_ref() {
            Some(p) => p,
            None => return, // Pool not initialized, discard connection
        };

        if is_unix {
            #[cfg(unix)]
            pools
                .unix_pool
                .borrow_mut()
                .return_connection_with_local_limit(key.clone(), wrapper, local_limit_idx);
        } else {
            pools.tcp_shards[tcp_shard_idx]
                .borrow_mut()
                .return_connection_with_local_limit(key.clone(), wrapper, local_limit_idx);
        }
    });
}
