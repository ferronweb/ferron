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
use std::thread::ThreadId;
use std::time::Duration;

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
    /// Cached thread ID for this thread, computed once at pool initialization.
    thread_id: ThreadId,
}

// Thread-local storage for connection pools.
thread_local! {
    static TLS_POOLS: UnsafeCell<Option<ThreadLocalPools>> = const { UnsafeCell::new(None) };
}

#[allow(clippy::type_complexity)]
static PENDING_PULLS: LazyLock<
    parking_lot::RwLock<FxHashMap<(Option<Arc<UpstreamInner>>, bool), SegQueue<CancellationToken>>>,
> = LazyLock::new(|| parking_lot::RwLock::new(FxHashMap::default()));

/// Fast-path flag for PENDING_PULLS: when zero, no thread is waiting for a
/// connection and `return_connection_to_pool` can skip the read lock entirely.
static PENDING_PULL_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Global pool depth stats collector.
///
/// Tracks idle and outstanding connection counts per (worker thread, upstream) pair.
/// Updated at pull/return boundaries and consumed by a background gauge emission task.
pub static POOL_STATS: LazyLock<PoolStatsCollector> = LazyLock::new(PoolStatsCollector::new);

pub struct PoolStatsCollector {
    // AtomicUsize #1 - idle connections
    // AtomicUsize #2 - outstanding connections
    inner: DashMap<
        (std::thread::ThreadId, Arc<UpstreamInner>),
        (AtomicUsize, AtomicUsize),
        FxBuildHasher,
    >,
    local_limits: DashMap<Arc<UpstreamInner>, AtomicUsize, FxBuildHasher>,
}

impl PoolStatsCollector {
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: DashMap::with_hasher(FxBuildHasher),
            local_limits: DashMap::with_hasher(FxBuildHasher),
        }
    }

    #[inline]
    pub fn record_pull(&self, thread_id: ThreadId, upstream: &Arc<UpstreamInner>, had_idle: bool) {
        let key = (thread_id, upstream.clone());
        let entry = if let Some(entry) = self.inner.get(&key) {
            entry
        } else {
            self.inner
                .entry(key)
                .or_insert_with(|| (AtomicUsize::new(0), AtomicUsize::new(0)))
                .downgrade()
        };
        entry.value().1.fetch_add(1, Ordering::Relaxed);
        if had_idle {
            entry.value().0.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn record_return(&self, thread_id: ThreadId, upstream: &Arc<UpstreamInner>, stored: bool) {
        let key = (thread_id, upstream.clone());
        let entry = if let Some(entry) = self.inner.get(&key) {
            entry
        } else {
            self.inner
                .entry(key)
                .or_insert_with(|| (AtomicUsize::new(0), AtomicUsize::new(0)))
                .downgrade()
        };
        entry.value().1.fetch_sub(1, Ordering::Relaxed);
        if stored {
            entry.value().0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn record_local_limit(&self, upstream: &Arc<UpstreamInner>, local_limit: usize) {
        let entry = if let Some(entry) = self.local_limits.get(upstream) {
            entry
        } else {
            self.local_limits
                .entry(upstream.clone())
                .or_insert_with(|| AtomicUsize::new(usize::MAX))
                .downgrade()
        };
        entry.value().store(local_limit, Ordering::Relaxed);
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

    #[allow(clippy::type_complexity)]
    #[inline]
    pub fn snapshot_local_limits(&self) -> Vec<(Arc<UpstreamInner>, usize)> {
        self.local_limits
            .iter()
            .filter_map(|entry| {
                let key = entry.key().clone();
                let local_limit = entry.value().load(Ordering::Relaxed);
                (local_limit != usize::MAX).then_some((key, local_limit))
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

        POOL_STATS.record_local_limit(&upstream, limit);
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
        idle_timeout: Duration,
    ) -> PooledConnection {
        loop {
            if let Some(conn) = self.try_pull(upstream.clone(), client_ip, idle_timeout) {
                // Pool not under capacity.
                return conn;
            }

            // Pool likely under capacity, wait for a connection to become available
            // Had to wrap in `{ ... }` to prevent subtle `PENDING_PULLS` deadlock,
            // because the lock was held across async boundary.
            PENDING_PULL_COUNT.fetch_add(1, Ordering::Relaxed);
            let cancel_token = {
                // Fast path: concurrent read lock (multiple threads can access simultaneously)
                let pending_pulls_key = (None, upstream.proxy_unix.is_some());
                let cancel_token = CancellationToken::new();

                let pending_pulls_read = PENDING_PULLS.read();
                let queue_opt = pending_pulls_read.get(&pending_pulls_key);
                if let Some(queue) = queue_opt {
                    queue.push(cancel_token.clone());
                } else {
                    drop(pending_pulls_read);
                    // Slow path: upgrade to write lock only if the queue doesn't exist yet
                    let mut write_lock = PENDING_PULLS.write();
                    let queue = write_lock.entry(pending_pulls_key).or_default();
                    queue.push(cancel_token.clone());
                }

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
        idle_timeout: Duration,
    ) -> PooledConnection {
        loop {
            if let Some(conn) = self.try_pull_with_local_limit(
                upstream.clone(),
                client_ip,
                local_limit,
                idle_timeout,
            ) {
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
            PENDING_PULL_COUNT.fetch_add(1, Ordering::Relaxed);
            let cancel_token = {
                // Fast path: concurrent read lock (multiple threads can access simultaneously)
                let pending_pulls_key = (
                    at_local_limit.then_some(upstream.clone()),
                    upstream.proxy_unix.is_some(),
                );
                let cancel_token = CancellationToken::new();

                let pending_pulls_read = PENDING_PULLS.read();
                let queue_opt = pending_pulls_read.get(&pending_pulls_key);
                if let Some(queue) = queue_opt {
                    queue.push(cancel_token.clone());
                } else {
                    drop(pending_pulls_read);
                    // Slow path: upgrade to write lock only if the queue doesn't exist yet
                    let mut write_lock = PENDING_PULLS.write();
                    let queue = write_lock.entry(pending_pulls_key).or_default();
                    queue.push(cancel_token.clone());
                }

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
        idle_timeout: Duration,
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
                        return pools
                            .unix_pool
                            .pull(key, |c| c.check_ready(Some(idle_timeout)));
                    }
                    return pools
                        .tcp_pool
                        .pull(key, |c| c.check_ready(Some(idle_timeout)));
                }
            }

            // Slow path: initialize or update capacity
            if opt.is_none() {
                *opt = Some(ThreadLocalPools {
                    tcp_pool: Rc::new(SingleThreadPool::new(per_thread)),
                    #[cfg(unix)]
                    unix_pool: Rc::new(SingleThreadPool::new_unbounded()),
                    last_global_limit: per_thread,
                    thread_id: std::thread::current().id(),
                });
            }
            let pools = opt.as_mut().unwrap();
            if pools.last_global_limit != per_thread {
                pools.tcp_pool.update_capacity(per_thread);
                pools.last_global_limit = per_thread;
            }

            #[cfg(unix)]
            if key.0.proxy_unix.is_some() {
                return pools
                    .unix_pool
                    .pull(key, |c| c.check_ready(Some(idle_timeout)));
            }
            pools
                .tcp_pool
                .pull(key, |c| c.check_ready(Some(idle_timeout)))
        });

        if let Some(result) = &result {
            let thread_id = get_tls_thread_id();
            POOL_STATS.record_pull(thread_id, &upstream_for_stats, result.inner().is_some());
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
        idle_timeout: Duration,
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
                        return pools.unix_pool.pull_with_local_limit(key, limit, |c| {
                            c.check_ready(Some(idle_timeout))
                        });
                    }
                    return pools
                        .tcp_pool
                        .pull_with_local_limit(key, limit, |c| c.check_ready(Some(idle_timeout)));
                }
            }

            // Slow path: initialize or update capacity
            if opt.is_none() {
                *opt = Some(ThreadLocalPools {
                    tcp_pool: Rc::new(SingleThreadPool::new(per_thread)),
                    #[cfg(unix)]
                    unix_pool: Rc::new(SingleThreadPool::new_unbounded()),
                    last_global_limit: per_thread,
                    thread_id: std::thread::current().id(),
                });
            }
            let pools = opt.as_mut().unwrap();
            if pools.last_global_limit != per_thread {
                pools.tcp_pool.update_capacity(per_thread);
                pools.last_global_limit = per_thread;
            }

            #[cfg(unix)]
            if key.0.proxy_unix.is_some() {
                return pools
                    .unix_pool
                    .pull_with_local_limit(key, limit, |c| c.check_ready(Some(idle_timeout)));
            }
            pools
                .tcp_pool
                .pull_with_local_limit(key, limit, |c| c.check_ready(Some(idle_timeout)))
        });

        if let Some(result) = &result {
            let thread_id = get_tls_thread_id();
            POOL_STATS.record_pull(thread_id, &upstream_for_stats, result.inner().is_some());
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

        // Fast path: if no thread is waiting for a connection, skip the
        // PENDING_PULLS lock entirely. This avoids RwLock contention on the
        // hot path when the pool is not exhausted.
        if PENDING_PULL_COUNT.load(Ordering::Relaxed) > 0 {
            if let Some(pending_pull) = PENDING_PULLS
                .read()
                .get(&(local_limit_key.clone(), is_unix))
                .and_then(|q| q.pop())
            {
                // Cancel any pending pull for this local limit key, if one exists.
                PENDING_PULL_COUNT.fetch_sub(1, Ordering::Relaxed);
                pending_pull.cancel();
            } else if local_limit_key.is_some() {
                if let Some(pending_pull) = PENDING_PULLS
                    .read()
                    .get(&(None, is_unix))
                    .and_then(|q| q.pop())
                {
                    // Cancel any pending pull for the global key, if one exists.
                    PENDING_PULL_COUNT.fetch_sub(1, Ordering::Relaxed);
                    pending_pull.cancel();
                }
            }
        }

        stored
    });

    let thread_id = get_tls_thread_id();
    POOL_STATS.record_return(thread_id, &key.0, stored);
}

/// Get the cached thread ID from thread-local pool storage.
///
/// Returns the thread ID that was captured when the pool was first initialized,
/// avoiding repeated `std::thread::current().id()` calls.
#[inline]
fn get_tls_thread_id() -> ThreadId {
    TLS_POOLS.with(|c| {
        let ptr = c.get();
        let opt = unsafe { &*ptr };
        opt.as_ref()
            .map(|p| p.thread_id)
            .unwrap_or_else(|| std::thread::current().id())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_stats_collector() {
        let collector = PoolStatsCollector::new();
        let upstream = Arc::new(UpstreamInner {
            proxy_to: "http://backend".to_string(),
            connect_to: None,
            proxy_unix: None,
            weight: 1,
            mtls: None,
            priority: 0,
            connection_timeout: None,
            idle_timeout: std::time::Duration::from_secs(60),
            dns_status: Default::default(),
        });

        // Record some pulls and returns
        // - record_pull with had_idle = true: +1 outstanding, -1 idle
        // - record_pull with had_idle = false: +1 outstanding, 0 idle
        // - record_return with stored = true: -1 outstanding, +1 idle
        // - record_return with stored = false: -1 outstanding, 0 idle
        let thread_id = std::thread::current().id();
        collector.record_pull(thread_id, &upstream, false); // +1 outstanding, 0 idle
        collector.record_return(thread_id, &upstream, true); // -1 outstanding, +1 idle
        collector.record_pull(thread_id, &upstream, true); // +1 outstanding, -1 idle
        collector.record_pull(thread_id, &upstream, false); // +1 outstanding, 0 idle
        collector.record_return(thread_id, &upstream, false); // -1 outstanding, 0 idle

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.len(), 1);
        let ((_thread_id, recorded_upstream), (idle, outstanding)) = &snapshot[0];
        assert_eq!(recorded_upstream.proxy_to, "http://backend");
        assert_eq!(*idle, 0);
        assert_eq!(*outstanding, 1);
    }
}
