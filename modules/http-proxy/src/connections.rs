//! Connection pool using thread-local storage.
//!
//! This isa simple, single-threaded pool stored in thread-local storage.
//! Each thread owns its own pool exclusively, eliminating synchronization
//! overhead entirely.

use std::cell::UnsafeCell;
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use rustc_hash::FxHashMap;

use crate::connpool_single::SingleThreadPool;
use crate::send_request::SendRequestWrapper;
use crate::types::upstream::UpstreamInner;

/// A unique key for an upstream, used for connection pooling.
#[derive(Clone)]
pub struct UpstreamKey(pub Arc<UpstreamInner>);

impl PartialEq for UpstreamKey {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for UpstreamKey {}

impl std::hash::Hash for UpstreamKey {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // O(1) hashing of the memory address instead of the struct contents
        std::sync::Arc::as_ptr(&self.0).hash(state);
    }
}

/// Connection pool key type: (upstream via Arc for cheap cloning, optional client IP for PROXY protocol).
pub type PoolKey = (UpstreamKey, Option<IpAddr>);

/// Concrete pool item type used throughout the proxy.
pub(crate) type PooledConnection =
    crate::connpool_single::PoolItem<PoolKey, UpstreamKey, SendRequestWrapper>;

/// Thread-local pool storage.
///
/// Since we use a thread-per-core runtime, each thread gets its own pool.
/// The pools are stored in `UnsafeCell` for interior mutability within the thread.
struct ThreadLocalPools {
    /// TCP connection pool.
    tcp_pool: Rc<SingleThreadPool<PoolKey, UpstreamKey, SendRequestWrapper>>,
    /// Unix socket pool (unbounded, separate from TCP pools).
    #[cfg(unix)]
    unix_pool: Rc<SingleThreadPool<PoolKey, UpstreamKey, SendRequestWrapper>>,
    /// Last per-thread TCP capacity that was synced into this TLS pool.
    last_global_limit: usize,
}

// Thread-local storage for connection pools.
thread_local! {
    static TLS_POOLS: UnsafeCell<Option<ThreadLocalPools>> = const { UnsafeCell::new(None) };
}

/// Connection pool manager for the reverse proxy.
///
/// This manager coordinates thread-local pools with a global concurrent limit.
/// Each thread owns its own pool instance, eliminating cross-thread contention.
pub struct ConnectionManager {
    /// Pre-thread global limit. Uses `AtomicUsize` for thread-safe interior mutability.
    global_limit_per_thread: AtomicUsize,
    /// Per-upstream local limits, already scaled to the per-thread capacity.
    local_limits: RwLock<FxHashMap<Arc<UpstreamInner>, usize>>,
    /// Available parallelism for thread-local pool sizing.
    available_parallelism: usize,
}

impl ConnectionManager {
    /// Creates a new `ConnectionManager` with the given global limit.
    #[inline]
    pub fn with_global_limit(global_limit: usize) -> Self {
        // Initialize thread-local pools lazily on first access
        let available_parallelism = std::thread::available_parallelism()
            .ok()
            .map(|p| p.get())
            .unwrap_or(1);
        let per_thread = if available_parallelism > 0 {
            global_limit.div_ceil(available_parallelism)
        } else {
            global_limit
        };
        Self {
            global_limit_per_thread: AtomicUsize::new(per_thread),
            local_limits: RwLock::new(FxHashMap::default()),
            available_parallelism,
        }
    }

    /// Set or update a per-upstream local connection limit.
    #[inline]
    pub fn set_local_limit(&self, upstream: Arc<UpstreamInner>, limit: usize) -> usize {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");

        limits.insert(upstream, limit);
        limit
    }

    /// Get the local limit value for an upstream.
    #[inline]
    pub fn get_local_limit(&self, upstream: Arc<UpstreamInner>) -> Option<usize> {
        self.local_limits
            .read()
            .expect("local_limits lock poisoned")
            .get(&upstream)
            .copied()
    }

    /// Updates the global concurrent connections limit.
    ///
    /// Existing thread-local pools observe the new capacity on their next access.
    #[inline]
    pub fn update_global_limit(&self, new_limit: usize) {
        let available_parallelism = self.available_parallelism;
        let per_thread = if available_parallelism > 0 {
            new_limit.div_ceil(available_parallelism)
        } else {
            new_limit
        };

        // Update the stored global limit
        self.global_limit_per_thread
            .store(per_thread, Ordering::Relaxed);
    }

    /// Updates the local limit for a specific upstream.
    #[allow(dead_code)]
    #[inline]
    pub fn update_local_limit_for_upstream(&self, upstream: Arc<UpstreamInner>, new_limit: usize) {
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
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
    ) -> Option<PooledConnection> {
        let key = (UpstreamKey(upstream), client_ip);
        let per_thread = self.global_limit_per_thread.load(Ordering::Relaxed);

        TLS_POOLS.with(|c| {
            let ptr = c.get();
            let opt = unsafe { &mut *ptr };

            // Fast path: already initialized and limit matches
            if let Some(pools) = opt.as_ref() {
                if pools.last_global_limit == per_thread {
                    #[cfg(unix)]
                    if key.0 .0.proxy_unix.is_some() {
                        return pools.unix_pool.pull(key);
                    }
                    return pools.tcp_pool.pull(key);
                }
            }

            // Slow path: initialize or update capacity
            if opt.is_none() {
                *opt = Some(ThreadLocalPools {
                    tcp_pool: Rc::new(SingleThreadPool::new(per_thread)),
                    #[cfg(unix)]
                    unix_pool: Rc::new(SingleThreadPool::new_unbounded()),
                    last_global_limit: per_thread,
                });
            }
            let pools = opt.as_mut().unwrap();
            if pools.last_global_limit != per_thread {
                pools.tcp_pool.update_capacity(per_thread);
                pools.last_global_limit = per_thread;
            }

            #[cfg(unix)]
            if key.0 .0.proxy_unix.is_some() {
                return pools.unix_pool.pull(key);
            }
            pools.tcp_pool.pull(key)
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
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
        local_limit: Option<usize>,
    ) -> Option<PooledConnection> {
        let upstream_key = UpstreamKey(upstream);
        let key = (upstream_key.clone(), client_ip);
        let limit = local_limit.map(|limit| (upstream_key, limit));
        let per_thread = self.global_limit_per_thread.load(Ordering::Relaxed);

        TLS_POOLS.with(|c| {
            let ptr = c.get();
            let opt = unsafe { &mut *ptr };

            // Fast path: already initialized and limit matches
            if let Some(pools) = opt.as_ref() {
                if pools.last_global_limit == per_thread {
                    #[cfg(unix)]
                    if key.0 .0.proxy_unix.is_some() {
                        return pools.unix_pool.pull_with_local_limit(key, limit);
                    }
                    return pools.tcp_pool.pull_with_local_limit(key, limit);
                }
            }

            // Slow path: initialize or update capacity
            if opt.is_none() {
                *opt = Some(ThreadLocalPools {
                    tcp_pool: Rc::new(SingleThreadPool::new(per_thread)),
                    #[cfg(unix)]
                    unix_pool: Rc::new(SingleThreadPool::new_unbounded()),
                    last_global_limit: per_thread,
                });
            }
            let pools = opt.as_mut().unwrap();
            if pools.last_global_limit != per_thread {
                pools.tcp_pool.update_capacity(per_thread);
                pools.last_global_limit = per_thread;
            }

            #[cfg(unix)]
            if key.0 .0.proxy_unix.is_some() {
                return pools.unix_pool.pull_with_local_limit(key, limit);
            }
            pools.tcp_pool.pull_with_local_limit(key, limit)
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
    let local_limit_key = local_limit_key.map(UpstreamKey);
    TLS_POOLS.with(|tls| {
        // SAFETY: We are strictly single-threaded per core, and no re-entrant
        // mutable borrows occur during connection pulls.
        let guard = unsafe { &*tls.get() };
        let Some(pools) = guard.as_ref() else {
            return; // Pool not initialized, discard connection
        };

        if is_unix {
            #[cfg(unix)]
            pools.unix_pool.return_connection_with_local_limit(
                key.clone(),
                wrapper,
                local_limit_key,
            );
        } else {
            pools.tcp_pool.return_connection_with_local_limit(
                key.clone(),
                wrapper,
                local_limit_key,
            );
        }
    });
}
