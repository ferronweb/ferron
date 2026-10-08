use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::ThreadId;

use dashmap::DashMap;
use rustc_hash::FxBuildHasher;

use crate::types::upstream::ResolvedUpstream;

/// Global pool depth stats collector.
///
/// Tracks idle and outstanding connection counts per (worker thread, upstream) pair.
/// Updated at pull/return boundaries and consumed by a background gauge emission task.
pub static POOL_STATS: LazyLock<PoolStatsCollector> = LazyLock::new(PoolStatsCollector::new);

pub struct PoolStatsCollector {
    // AtomicUsize #1 - idle connections
    // AtomicUsize #2 - outstanding connections
    inner: DashMap<
        (std::thread::ThreadId, Arc<ResolvedUpstream>),
        (AtomicUsize, AtomicUsize),
        FxBuildHasher,
    >,
    local_limits: DashMap<Arc<ResolvedUpstream>, AtomicUsize, FxBuildHasher>,
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
    pub fn record_pull(
        &self,
        thread_id: ThreadId,
        upstream: &Arc<ResolvedUpstream>,
        had_idle: bool,
    ) {
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
    pub fn record_return(
        &self,
        thread_id: ThreadId,
        upstream: &Arc<ResolvedUpstream>,
        stored: bool,
    ) {
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
    pub fn record_local_limit(&self, upstream: &Arc<ResolvedUpstream>, local_limit: usize) {
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
    pub fn snapshot(
        &self,
    ) -> Vec<(
        (std::thread::ThreadId, Arc<ResolvedUpstream>),
        (usize, usize),
    )> {
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
    pub fn snapshot_local_limits(&self) -> Vec<(Arc<ResolvedUpstream>, usize)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_stats_collector() {
        let collector = PoolStatsCollector::new();
        let upstream = Arc::new(ResolvedUpstream {
            proxy_to: "http://backend".to_string(),
            connect_to: None,
            proxy_unix: None,
            inner: crate::types::upstream::UpstreamInner {
                weight: 1,
                mtls: None,
                priority: 0,
                connection_timeout: None,
                idle_timeout: std::time::Duration::from_secs(60),
                limit: None,
            },
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
