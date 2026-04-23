//! Connection pool using thread-local storage.
//!
//! This replaces the concurrent `connpool` with a simple, single-threaded pool
//! stored in thread-local storage. Each thread owns its own pool exclusively,
//! eliminating synchronization overhead entirely.

use std::cell::RefCell;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use rustc_hash::FxHashMap;

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
    tcp_pool: RefCell<SingleThreadPool<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    /// Unix socket pool (unbounded, separate from TCP pools).
    #[cfg(unix)]
    unix_pool: RefCell<SingleThreadPool<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    /// Last per-thread TCP capacity that was synced into this TLS pool.
    last_global_limit: usize,
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
    /// Per-upstream local limits, already scaled to the per-thread capacity.
    local_limits: RwLock<FxHashMap<UpstreamInner, usize>>,
    /// Available parallelism for thread-local pool sizing.
    available_parallelism: usize,
}

impl ConnectionManager {
    /// Creates a new `ConnectionManager` with the given global limit.
    #[inline]
    pub fn with_global_limit(global_limit: usize) -> Self {
        // Initialize thread-local pools lazily on first access
        Self {
            global_limit: AtomicUsize::new(global_limit),
            local_limits: RwLock::new(FxHashMap::default()),
            available_parallelism: std::thread::available_parallelism()
                .ok()
                .map(|p| p.get())
                .unwrap_or(1),
        }
    }

    /// Ensures thread-local pools are initialized for the current thread.
    #[inline]
    fn ensure_tls_pools(&self) {
        let limit = self.global_limit.load(Ordering::Relaxed);
        let available_parallelism = self.available_parallelism;
        let per_thread = if available_parallelism > 0 {
            limit.div_ceil(available_parallelism)
        } else {
            limit
        };

        TLS_POOLS.with(|tls| {
            let mut guard = tls.borrow_mut();
            if let Some(pools) = guard.as_mut() {
                if pools.last_global_limit != per_thread {
                    pools.tcp_pool.borrow_mut().update_capacity(per_thread);
                    pools.last_global_limit = per_thread;
                }
                return;
            }

            *guard = Some(ThreadLocalPools {
                tcp_pool: RefCell::new(SingleThreadPool::new(per_thread)),
                #[cfg(unix)]
                unix_pool: RefCell::new(SingleThreadPool::new_unbounded()),
                last_global_limit: per_thread,
            });
        });
    }

    /// Set or update a per-upstream local connection limit.
    #[inline]
    pub fn set_local_limit(&self, upstream: &UpstreamInner, limit: usize) -> usize {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");

        limits.insert(upstream.clone(), limit);
        limit
    }

    /// Get the local limit value for an upstream.
    #[inline]
    pub fn get_local_limit(&self, upstream: &UpstreamInner) -> Option<usize> {
        self.local_limits
            .read()
            .expect("local_limits lock poisoned")
            .get(upstream)
            .copied()
    }

    /// Updates the global concurrent connections limit.
    ///
    /// Existing thread-local pools observe the new capacity on their next access.
    #[inline]
    pub fn update_global_limit(&self, new_limit: usize) {
        // Update the stored global limit
        self.global_limit.store(new_limit, Ordering::Relaxed);
    }

    /// Updates the local limit for a specific upstream.
    #[inline]
    pub fn update_local_limit_for_upstream(&self, upstream: &UpstreamInner, new_limit: usize) {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");
        limits.insert(upstream.clone(), new_limit);
    }

    /// Pull a connection from the pool, returning immediately.
    ///
    /// Unlike the old connpool-based version, this is **synchronous** and returns
    /// `None` if the pool is at capacity (caller should establish a new connection).
    #[allow(dead_code)]
    #[inline]
    pub fn pull(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
    ) -> Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> {
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
    #[inline]
    pub fn pull_with_local_limit(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
        local_limit: Option<usize>,
    ) -> Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> {
        self.ensure_tls_pools();

        let upstream_key = Arc::new(upstream.clone());
        let key = (Arc::clone(&upstream_key), client_ip);
        let limit = local_limit.map(|limit| (upstream_key, limit));

        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return TLS_POOLS.with(|tls| {
                let guard = tls.borrow();
                let pools = guard.as_ref().unwrap();
                let result = pools
                    .unix_pool
                    .borrow_mut()
                    .pull_with_local_limit(key, limit);
                result
            });
        }

        TLS_POOLS.with(|tls| {
            let guard = tls.borrow();
            let pools = guard.as_ref().unwrap();
            let result = pools
                .tcp_pool
                .borrow_mut()
                .pull_with_local_limit(key, limit);
            result
        })
    }
}

/// Return a connection to the thread-local pool.
///
/// This is used by `TrackedBody` to return connections after the response body
/// is fully consumed.
#[inline]
pub fn return_connection_to_pool(
    key: &PoolKey,
    wrapper: SendRequestWrapper,
    local_limit_key: Option<Arc<UpstreamInner>>,
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
                .return_connection_with_local_limit(key.clone(), wrapper, local_limit_key);
        } else {
            pools
                .tcp_pool
                .borrow_mut()
                .return_connection_with_local_limit(key.clone(), wrapper, local_limit_key);
        }
    });
}
