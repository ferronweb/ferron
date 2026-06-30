//! Connection pool using thread-local storage.
//!
//! This is a simple, single-threaded pool stored in thread-local storage.
//! Each thread owns its own pool exclusively, eliminating synchronization
//! overhead entirely.

use std::cell::UnsafeCell;
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use crossbeam_queue::SegQueue;
use dashmap::DashMap;
use rustc_hash::{FxBuildHasher, FxHashMap};
use tokio_util::sync::CancellationToken;

mod pool;

use self::pool::SingleThreadPool;
use crate::send_request::SendRequestWrapper;
use crate::types::upstream::UpstreamInner;

/// Connection pool key type: (upstream via Arc for cheap cloning, optional client IP for PROXY protocol).
pub type PoolKey = (Arc<UpstreamInner>, Option<IpAddr>);

/// Concrete pool item type used throughout the proxy.
pub(crate) type PooledConnection =
    self::pool::PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>;

/// Thread-local pool storage.
///
/// Since we use a thread-per-core runtime, each thread gets its own pool.
/// The pools are stored in `UnsafeCell` for interior mutability within the thread.
struct ThreadLocalPools {
    /// TCP connection pool.
    tcp_pool: Rc<SingleThreadPool<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    /// Unix socket pool (unbounded, separate from TCP pools).
    #[cfg(unix)]
    unix_pool: Rc<SingleThreadPool<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    /// Last per-thread TCP capacity that was synced into this TLS pool.
    last_global_limit: usize,
}

// Thread-local storage for connection pools.
thread_local! {
    static TLS_POOLS: UnsafeCell<Option<ThreadLocalPools>> = const { UnsafeCell::new(None) };
}

#[allow(clippy::type_complexity)]
static PENDING_PULLS: LazyLock<
    parking_lot::RwLock<FxHashMap<(Option<Arc<UpstreamInner>>, bool), SegQueue<CancellationToken>>>,
> = LazyLock::new(|| parking_lot::RwLock::new(FxHashMap::default()));

/// Global pool depth stats collector.
///
/// Tracks idle and outstanding connection counts per (worker thread, upstream) pair.
/// Updated at pull/return boundaries and consumed by a background gauge emission task.
pub static POOL_STATS: LazyLock<PoolStatsCollector> = LazyLock::new(PoolStatsCollector::new);

pub struct PoolStatsCollector {
    inner: DashMap<
        (std::thread::ThreadId, Arc<UpstreamInner>),
        (AtomicUsize, AtomicUsize),
        FxBuildHasher,
    >,
}

impl PoolStatsCollector {
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_hasher(FxBuildHasher),
        }
    }

    #[inline]
    pub fn record_pull(&self, upstream: &Arc<UpstreamInner>, had_idle: bool) {
        let entry = self
            .inner
            .entry((std::thread::current().id(), upstream.clone()))
            .or_insert_with(|| (AtomicUsize::new(0), AtomicUsize::new(0)));
        entry.value().1.fetch_add(1, Ordering::Relaxed);
        if had_idle {
            entry.value().0.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn record_return(&self, upstream: &Arc<UpstreamInner>, stored: bool) {
        let entry = self
            .inner
            .entry((std::thread::current().id(), upstream.clone()))
            .or_insert_with(|| (AtomicUsize::new(0), AtomicUsize::new(0)));
        entry.value().1.fetch_sub(1, Ordering::Relaxed);
        if stored {
            entry.value().0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[allow(clippy::type_complexity)]
    #[inline]
    pub fn snapshot(&self) -> Vec<((std::thread::ThreadId, Arc<UpstreamInner>), (usize, usize))> {
        self.inner
            .iter()
            .map(|entry| {
                let key = entry.key().clone();
                let idle = entry.value().0.load(Ordering::Relaxed);
                let outstanding = entry.value().1.load(Ordering::Relaxed);
                (key, (idle, outstanding))
            })
            .collect()
    }
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

    /// Pulls a connection from the pool, waiting if necessary for one to become available.
    #[inline]
    pub async fn pull(
        &self,
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
    ) -> PooledConnection {
        loop {
            if let Some(conn) = self.try_pull(upstream.clone(), client_ip) {
                // Pool not under capacity.
                return conn;
            }

            // Pool likely under capacity, wait for a connection to become available
            // Had to wrap in `{ ... }` to prevent subtle `PENDING_PULLS` deadlock,
            // because the lock was held across async boundary.
            let cancel_token = {
                let mut pending_pulls_lock = PENDING_PULLS.upgradable_read();
                let pending_pulls_key = (None, upstream.proxy_unix.is_some());
                let pending_pulls =
                    if let Some(pending_pulls) = pending_pulls_lock.get(&pending_pulls_key) {
                        pending_pulls
                    } else {
                        pending_pulls_lock.with_upgraded(|pp| {
                            pp.insert(pending_pulls_key.clone(), SegQueue::new());
                        });
                        pending_pulls_lock
                            .get(&pending_pulls_key)
                            .expect("pending pulls should have been initialized at this point")
                    };
                let cancel_token = CancellationToken::new();
                pending_pulls.push(cancel_token.clone());
                cancel_token
            };

            // Wait for the connection to be available.
            cancel_token.cancelled().await;
        }
    }

    /// Pull a connection from the pool, returning immediately.
    ///
    /// Returns `None` if the pool is at capacity (caller should establish a new connection).
    #[inline]
    pub async fn pull_with_local_limit(
        &self,
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
        local_limit: Option<usize>,
    ) -> PooledConnection {
        loop {
            if let Some(conn) =
                self.try_pull_with_local_limit(upstream.clone(), client_ip, local_limit)
            {
                // Pool not under capacity.
                return conn;
            }

            // Check if local limit is exceeded
            let at_local_limit = if let Some(ll) = local_limit {
                TLS_POOLS.with(|c| {
                    let ptr = c.get();
                    let opt = unsafe { &mut *ptr };
                    opt.as_ref().is_some_and(|p| {
                        if upstream.proxy_unix.is_some() {
                            #[cfg(unix)]
                            let r = p.unix_pool.is_at_local_limit(&upstream, ll);
                            #[cfg(not(unix))]
                            let r = false;

                            r
                        } else {
                            p.tcp_pool.is_at_local_limit(&upstream, ll)
                        }
                    })
                })
            } else {
                false
            };

            // Pool likely under capacity, wait for a connection to become available
            // Had to wrap in `{ ... }` to prevent subtle `PENDING_PULLS` deadlock,
            // because the lock was held across async boundary.
            let cancel_token = {
                let mut pending_pulls_lock = PENDING_PULLS.upgradable_read();
                let pending_pull_key = (
                    at_local_limit.then_some(upstream.clone()),
                    upstream.proxy_unix.is_some(),
                );
                let pending_pulls =
                    if let Some(pending_pulls) = pending_pulls_lock.get(&pending_pull_key) {
                        pending_pulls
                    } else {
                        pending_pulls_lock.with_upgraded(|pp| {
                            pp.insert(pending_pull_key.clone(), SegQueue::new());
                        });
                        pending_pulls_lock
                            .get(&pending_pull_key)
                            .expect("pending pulls should have been initialized at this point")
                    };
                let cancel_token = CancellationToken::new();
                pending_pulls.push(cancel_token.clone());
                cancel_token
            };

            // Wait for the connection to be available.
            cancel_token.cancelled().await;
        }
    }

    /// Pull a connection from the pool, returning immediately.
    ///
    /// Returns `None` if the pool is at capacity (caller should establish a new connection).
    #[inline]
    pub fn try_pull(
        &self,
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
    ) -> Option<PooledConnection> {
        let upstream_for_stats = upstream.clone();
        let key = (upstream, client_ip);
        let per_thread = self.global_limit_per_thread.load(Ordering::Relaxed);

        let result = TLS_POOLS.with(|c| {
            let ptr = c.get();
            let opt = unsafe { &mut *ptr };

            // Fast path: already initialized and limit matches
            if let Some(pools) = opt.as_ref() {
                if pools.last_global_limit == per_thread {
                    #[cfg(unix)]
                    if key.0.proxy_unix.is_some() {
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
            if key.0.proxy_unix.is_some() {
                return pools.unix_pool.pull(key);
            }
            pools.tcp_pool.pull(key)
        });

        if let Some(result) = &result {
            POOL_STATS.record_pull(&upstream_for_stats, result.inner().is_some());
        }

        result
    }

    /// Pull a connection with a local limit applied, returning immediately.
    ///
    /// Returns `None` if the local or global limit is reached.
    #[inline]
    pub fn try_pull_with_local_limit(
        &self,
        upstream: Arc<UpstreamInner>,
        client_ip: Option<IpAddr>,
        local_limit: Option<usize>,
    ) -> Option<PooledConnection> {
        let upstream_for_stats = upstream.clone();
        let upstream_key = upstream;
        let key = (upstream_key.clone(), client_ip);
        let limit = local_limit.map(|limit| (upstream_key, limit));
        let per_thread = self.global_limit_per_thread.load(Ordering::Relaxed);

        let result = TLS_POOLS.with(|c| {
            let ptr = c.get();
            let opt = unsafe { &mut *ptr };

            // Fast path: already initialized and limit matches
            if let Some(pools) = opt.as_ref() {
                if pools.last_global_limit == per_thread {
                    #[cfg(unix)]
                    if key.0.proxy_unix.is_some() {
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
            if key.0.proxy_unix.is_some() {
                return pools.unix_pool.pull_with_local_limit(key, limit);
            }
            pools.tcp_pool.pull_with_local_limit(key, limit)
        });

        if let Some(result) = &result {
            POOL_STATS.record_pull(&upstream_for_stats, result.inner().is_some());
        }

        result
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
    let stored = TLS_POOLS.with(|tls| {
        // SAFETY: We are strictly single-threaded per core, and no re-entrant
        // mutable borrows occur during connection pulls.
        let guard = unsafe { &*tls.get() };
        let Some(pools) = guard.as_ref() else {
            return false; // Pool not initialized, discard connection
        };

        let stored = if is_unix {
            #[cfg(unix)]
            let stored = pools.unix_pool.return_connection_with_local_limit(
                key.clone(),
                wrapper,
                local_limit_key.clone(),
            );
            #[cfg(not(unix))]
            let stored = false;

            stored
        } else {
            pools.tcp_pool.return_connection_with_local_limit(
                key.clone(),
                wrapper,
                local_limit_key.clone(),
            )
        };

        if let Some(pending_pull) = PENDING_PULLS
            .read()
            .get(&(local_limit_key.clone(), is_unix))
            .and_then(|q| q.pop())
        {
            // Cancel any pending pull for this local limit key, if one exists.
            pending_pull.cancel();
        } else if local_limit_key.is_some() {
            if let Some(pending_pull) = PENDING_PULLS
                .read()
                .get(&(None, is_unix))
                .and_then(|q| q.pop())
            {
                // Cancel any pending pull for the global key, if one exists.
                pending_pull.cancel();
            }
        }

        stored
    });

    POOL_STATS.record_return(&key.0, stored);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_stats_collector() {
        let collector = PoolStatsCollector::new();
        let upstream = Arc::new(UpstreamInner {
            proxy_to: "http://backend".to_string(),
            proxy_unix: None,
            weight: 1,
            mtls: None,
            priority: 0,
        });

        // Record some pulls and returns
        // - record_pull with had_idle = true: +1 outstanding, -1 idle
        // - record_pull with had_idle = false: +1 outstanding, 0 idle
        // - record_return with stored = true: -1 outstanding, +1 idle
        // - record_return with stored = false: -1 outstanding, 0 idle
        collector.record_pull(&upstream, false); // +1 outstanding, 0 idle
        collector.record_return(&upstream, true); // -1 outstanding, +1 idle
        collector.record_pull(&upstream, true); // +1 outstanding, -1 idle
        collector.record_pull(&upstream, false); // +1 outstanding, 0 idle
        collector.record_return(&upstream, false); // -1 outstanding, 0 idle

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.len(), 1);
        let ((_thread_id, recorded_upstream), (idle, outstanding)) = &snapshot[0];
        assert_eq!(recorded_upstream.proxy_to, "http://backend");
        assert_eq!(*idle, 0);
        assert_eq!(*outstanding, 1);
    }
}
