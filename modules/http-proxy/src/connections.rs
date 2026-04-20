//! Connection pool using thread-local storage.
//!
//! This replaces the concurrent `connpool` with a simple, single-threaded pool
//! stored in thread-local storage. Each thread owns its own pool exclusively,
//! eliminating synchronization overhead entirely.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

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
    /// TCP connection pool.
    tcp_pool: RefCell<SingleThreadPool<PoolKey, SendRequestWrapper>>,
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
    /// Per-upstream local limit indices (read-only after initialization).
    local_limits: RwLock<HashMap<UpstreamInner, usize>>,
}

impl ConnectionManager {
    /// Creates a new `ConnectionManager` with the given global limit.
    pub fn with_global_limit(global_limit: usize) -> Self {
        // Initialize thread-local pools lazily on first access
        Self {
            global_limit: AtomicUsize::new(global_limit),
            local_limits: RwLock::new(HashMap::new()),
        }
    }

    /// Ensures thread-local pools are initialized for the current thread.
    fn ensure_tls_pools(&self) {
        TLS_POOLS.with(|tls| {
            if tls.borrow().is_some() {
                return;
            }

            let limit = self.global_limit.load(Ordering::Relaxed);
            let available_parallelism = std::thread::available_parallelism()
                .ok()
                .map(|p| p.get())
                .unwrap_or(1);
            let per_thread = if available_parallelism > 0 {
                limit.div_ceil(available_parallelism)
            } else {
                limit
            };

            *tls.borrow_mut() = Some(ThreadLocalPools {
                tcp_pool: RefCell::new(SingleThreadPool::new(per_thread)),
                #[cfg(unix)]
                unix_pool: RefCell::new(SingleThreadPool::new_unbounded()),
            });
        });
    }

    /// Set a per-upstream local connection limit.
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
        let idx = 0usize;

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let available_parallelism = std::thread::available_parallelism()
                .ok()
                .map(|p| p.get())
                .unwrap_or(1);
            let per_thread = if available_parallelism > 0 {
                limit.div_ceil(available_parallelism)
            } else {
                limit
            };
            pools.tcp_pool.borrow_mut().set_local_limit(per_thread);

            #[cfg(unix)]
            let _ = pools.unix_pool.borrow_mut().set_local_limit(per_thread);
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
    /// If the new limit is lower, excess idle connections are evicted from all pools.
    /// If the new limit is higher, pools can grow to the new capacity.
    pub fn update_global_limit(&self, new_limit: usize) {
        // Update the stored global limit
        self.global_limit.store(new_limit, Ordering::Relaxed);

        // Iterate over all thread-local pools and update their capacities
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            if let Some(pools) = guard.as_ref() {
                let available_parallelism = std::thread::available_parallelism()
                    .ok()
                    .map(|p| p.get())
                    .unwrap_or(1);
                let per_thread = if available_parallelism > 0 {
                    new_limit.div_ceil(available_parallelism)
                } else {
                    new_limit
                };

                pools.tcp_pool.borrow_mut().update_capacity(per_thread);
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
                    let available_parallelism = std::thread::available_parallelism()
                        .ok()
                        .map(|p| p.get())
                        .unwrap_or(1);
                    let per_thread = if available_parallelism > 0 {
                        new_limit.div_ceil(available_parallelism)
                    } else {
                        new_limit
                    };

                    {
                        let tcp_mut = pools.tcp_pool.borrow_mut();
                        let local_limits = unsafe { tcp_mut.local_limits_mut() };
                        if idx < local_limits.len() {
                            local_limits[idx] = per_thread;
                        }
                    }

                    #[cfg(unix)]
                    {
                        let unix_mut = pools.unix_pool.borrow_mut();
                        let local_limits = unsafe { unix_mut.local_limits_mut() };
                        if idx < local_limits.len() {
                            local_limits[idx] = per_thread;
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

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();
            let result = pools.tcp_pool.borrow_mut().pull(key);
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

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();
            let result = pools
                .tcp_pool
                .borrow_mut()
                .pull_with_local_limit(key, local_limit_idx);
            result
        })
    }

    /// Select the pool for the given upstream/client key.
    ///
    /// Returns a `PoolRef` which provides access to the thread-local pool
    /// for pulling connections.
    pub fn select_pool(&self, upstream: &UpstreamInner) -> PoolRef {
        self.ensure_tls_pools();

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return PoolRef::Unix;
        }

        PoolRef::Tcp
    }
}

/// A reference to a thread-local pool.
///
/// This type is returned by `ConnectionManager::select_pool()` and provides
/// access to the appropriate pool based on the upstream/client key.
pub enum PoolRef {
    /// TCP pool.
    Tcp,
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
            PoolRef::Tcp => false,
        }
    }

    /// Pull a connection from the referenced pool.
    pub fn pull(&self, key: PoolKey) -> Option<PoolItem<PoolKey, SendRequestWrapper>> {
        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();

            let result = match self {
                PoolRef::Tcp => pools.tcp_pool.borrow_mut().pull(key),
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
                PoolRef::Tcp => pools
                    .tcp_pool
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

/// Return a connection to the thread-local pool.
///
/// This is used by `TrackedBody` to return connections after the response body
/// is fully consumed.
pub fn return_connection_to_pool(
    key: &PoolKey,
    wrapper: SendRequestWrapper,
    local_limit_idx: Option<usize>,
    is_unix: bool,
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
            pools
                .tcp_pool
                .borrow_mut()
                .return_connection_with_local_limit(key.clone(), wrapper, local_limit_idx);
        }
    });
}
