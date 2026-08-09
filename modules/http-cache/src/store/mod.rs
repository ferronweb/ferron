mod key;
mod purge;
mod tests;
pub mod types;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ahash::{AHashMap, AHashSet, RandomState};
use http::header::{self, HeaderMap};
use quick_cache::sync::Cache;
use quick_cache::{DefaultHashBuilder, Lifecycle, UnitWeighter};
use rustc_hash::FxBuildHasher;
use tokio::sync::Notify;

use crate::lscache::PurgeOperation;
use crate::policy::{recalculate_freshness, CacheScope};

pub use self::key::build_entry_key;
pub use self::purge::{remove_hop_by_hop_headers, strip_store_headers};
pub use self::types::{
    LookupEntry, LookupHit, LookupOutcome, StoreStats, StoredEntry, StoredVariant, VaryRule,
};

/// Maximum number of variants tracked per base key. Bounds the variant map so
/// arbitrary `Vary` combinations cannot grow it without limit.
const MAX_VARIANTS_PER_BASE: usize = 64;

/// Minimum interval between full expired-entry scans. `cleanup_expired`
/// iterates the whole entry cache, so it runs at most once per second.
/// Entries that fall outside their TTL plus stale-while-revalidate window are
/// skipped lazily at lookup time even while the scan is throttled.
const CLEANUP_INTERVAL_SECS: u64 = 1;

/// Build the candidate entry keys for a request under a set of registered
/// variants, in lookup order: private variants (when a private key is present)
/// first, then public ones, each in insertion order.
fn build_candidate_keys(
    base_key: &str,
    private_key: Option<&str>,
    headers: &HeaderMap,
    cookies: &AHashMap<String, String>,
    variants: &[StoredVariant],
) -> Vec<String> {
    let mut candidate_keys = Vec::with_capacity(variants.len());
    if let Some(private_key) = private_key {
        for variant in variants
            .iter()
            .filter(|variant| variant.scope == CacheScope::Private)
        {
            candidate_keys.push(build_entry_key(
                base_key,
                variant.scope,
                Some(private_key),
                &variant.vary,
                headers,
                cookies,
            ));
        }
    }
    for variant in variants
        .iter()
        .filter(|variant| variant.scope == CacheScope::Public)
    {
        candidate_keys.push(build_entry_key(
            base_key,
            variant.scope,
            None,
            &variant.vary,
            headers,
            cookies,
        ));
    }
    candidate_keys
}

pub struct CacheStore {
    entries: Cache<String, StoredEntry, UnitWeighter, DefaultHashBuilder, StoreLifecycle>,
    variants_by_base: dashmap::DashMap<String, Vec<StoredVariant>, RandomState>,
    max_entries: AtomicUsize,
    last_cleanup: AtomicU64,
    inflight: dashmap::DashMap<String, InflightEntry, FxBuildHasher>,
    active_locks: AtomicUsize,
}

/// Tracks an in-flight upstream fetch for a specific cache key.
struct InflightEntry {
    notify: Arc<Notify>,
}

#[derive(Clone, Default)]
struct StoreLifecycle;

#[derive(Default)]
struct StoreRequestState {
    size_evictions: usize,
    evicted_base_keys: Vec<String>,
}

impl Lifecycle<String, StoredEntry> for StoreLifecycle {
    type RequestState = StoreRequestState;

    #[inline]
    fn on_evict(&self, state: &mut Self::RequestState, _key: String, val: StoredEntry) {
        state.size_evictions += 1;
        state.evicted_base_keys.push(val.base_key);
    }
}

impl CacheStore {
    #[inline]
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Cache::with(
                max_entries.max(1),
                max_entries as u64,
                UnitWeighter,
                DefaultHashBuilder::default(),
                StoreLifecycle,
            ),
            variants_by_base: dashmap::DashMap::with_hasher(RandomState::new()),
            max_entries: AtomicUsize::new(max_entries),
            last_cleanup: AtomicU64::new(0),
            inflight: dashmap::DashMap::with_hasher(FxBuildHasher),
            active_locks: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn set_max_entries(&self, max_entries: usize) {
        self.max_entries.store(max_entries, Ordering::Relaxed);
        self.entries.set_capacity(max_entries as u64);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return the number of active in-flight upstream fetches currently
    /// being coordinated by the singleflight mechanism.
    #[inline]
    pub fn active_locks(&self) -> usize {
        self.active_locks.load(Ordering::Relaxed)
    }

    /// Try to become the in-flight fetch leader for `cache_key`.
    #[inline]
    pub fn begin_fetch(&self, cache_key: &str) -> (bool, Arc<Notify>) {
        let mut is_leader = false;
        let entry = self
            .inflight
            .entry(cache_key.to_string())
            .or_insert_with(|| {
                is_leader = true;
                InflightEntry {
                    notify: Arc::new(Notify::new()),
                }
            });
        let notify = entry.notify.clone();
        if is_leader {
            self.active_locks.fetch_add(1, Ordering::Relaxed);
        }
        (is_leader, notify)
    }

    /// Complete an in-flight fetch: remove the entry and wake all waiters.
    #[inline]
    pub fn complete_fetch(&self, cache_key: &str) {
        if let Some((_, entry)) = self.inflight.remove(cache_key) {
            entry.notify.notify_waiters();
            self.active_locks.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn lookup(
        &self,
        base_key: &str,
        headers: &HeaderMap,
        cookies: &AHashMap<String, String>,
        private_key: Option<&str>,
    ) -> LookupOutcome {
        let stats = StoreStats {
            expired_evictions: self.cleanup_expired(),
            ..Default::default()
        };

        let Some(variants) = self.variants_by_base.get(base_key) else {
            return LookupOutcome {
                entry: None,
                stats,
                items: self.entries.len(),
                had_expired: false,
            };
        };
        let variants = variants.value().clone();
        let has_variants = true;

        let candidate_keys =
            build_candidate_keys(base_key, private_key, headers, cookies, &variants);

        // First pass: look for fresh entries
        for key in &candidate_keys {
            if let Some(entry) = self.entries.get(key) {
                let age = entry.created_at.elapsed();
                if age <= entry.ttl {
                    return LookupOutcome {
                        entry: Some((
                            LookupEntry {
                                scope: entry.scope,
                                status: entry.status,
                                headers: entry.headers.clone(),
                                body: entry.body.clone(),
                                lsc_cookies: entry.lsc_cookies.clone(),
                                age,
                                etag: entry.etag.clone(),
                                last_modified: entry.last_modified.clone(),
                                stale_if_error: entry.stale_if_error,
                                must_revalidate: entry.must_revalidate,
                                ttl: entry.ttl,
                            },
                            key.clone(),
                            LookupHit::Fresh,
                        )),
                        stats,
                        items: self.entries.len(),
                        had_expired: false,
                    };
                }
            }
        }

        // Second pass: look for stale entries within the SWR window
        for key in &candidate_keys {
            if let Some(entry) = self.entries.get(key) {
                let age = entry.created_at.elapsed();
                let swr_window = entry.stale_while_revalidate.unwrap_or_default();
                if age <= entry.ttl + swr_window && !entry.must_revalidate {
                    return LookupOutcome {
                        entry: Some((
                            LookupEntry {
                                scope: entry.scope,
                                status: entry.status,
                                headers: entry.headers.clone(),
                                body: entry.body.clone(),
                                lsc_cookies: entry.lsc_cookies.clone(),
                                age,
                                etag: entry.etag.clone(),
                                last_modified: entry.last_modified.clone(),
                                stale_if_error: entry.stale_if_error,
                                must_revalidate: entry.must_revalidate,
                                ttl: entry.ttl,
                            },
                            key.clone(),
                            LookupHit::StaleWhileRevalidate,
                        )),
                        stats,
                        items: self.entries.len(),
                        had_expired: false,
                    };
                }
            }
        }

        LookupOutcome {
            entry: None,
            stats,
            items: self.entries.len(),
            had_expired: has_variants,
        }
    }

    /// Return the first candidate entry key for `base_key` under this
    /// request's scope/vary/private context, matching `lookup`'s candidate
    /// ordering (private variants before public ones). Singleflight coalescing
    /// keys on this so that distinct vary variants of the same URL do not
    /// share an in-flight upstream fetch.
    #[inline]
    pub fn primary_candidate_key(
        &self,
        base_key: &str,
        headers: &HeaderMap,
        cookies: &AHashMap<String, String>,
        private_key: Option<&str>,
    ) -> Option<String> {
        let variants = self.variants_by_base.get(base_key)?;
        build_candidate_keys(base_key, private_key, headers, cookies, variants.value())
            .into_iter()
            .next()
    }

    #[inline]
    pub fn insert_with_request(
        &self,
        mut entry: StoredEntry,
        private_key: Option<&str>,
        request_headers: &HeaderMap,
        request_cookies: &AHashMap<String, String>,
    ) -> (StoreStats, usize) {
        let mut stats = StoreStats {
            expired_evictions: self.cleanup_expired(),
            ..Default::default()
        };

        let max_entries = self.max_entries.load(Ordering::Relaxed);
        if max_entries == 0 {
            return (stats, self.entries.len());
        }

        let key = build_entry_key(
            &entry.base_key,
            entry.scope,
            private_key,
            &entry.vary,
            request_headers,
            request_cookies,
        );

        entry.access_at = 0;
        if entry.scope == CacheScope::Private {
            entry.private_key = private_key.map(str::to_string);
        }

        {
            let variant = StoredVariant {
                scope: entry.scope,
                vary: entry.vary.clone(),
            };
            let mut variants = self
                .variants_by_base
                .entry(entry.base_key.clone())
                .or_default();
            if !variants.contains(&variant) {
                if variants.len() >= MAX_VARIANTS_PER_BASE {
                    // Bound variant cardinality per base: drop the oldest
                    // variant (front of the insertion-ordered Vec) before
                    // admitting a new one.
                    variants.remove(0);
                }
                variants.push(variant);
            }
        }

        let mut request_state = StoreRequestState::default();
        self.entries
            .insert_with_lifecycle(key, entry, &mut request_state);
        stats.size_evictions = request_state.size_evictions;
        for base_key in request_state.evicted_base_keys {
            self.remove_orphaned_base_key(&base_key);
        }
        (stats, self.entries.len())
    }

    #[inline]
    pub fn purge(
        &self,
        operations: &[PurgeOperation],
        current_private_key: Option<&str>,
        requesting_host: Option<&str>,
    ) -> (StoreStats, usize) {
        let mut stats = StoreStats::default();
        let mut keys_to_remove = AHashSet::default();

        for (key, entry) in self.entries.iter() {
            if operations.iter().any(|operation| {
                purge::entry_matches_purge(&entry, operation, current_private_key, requesting_host)
            }) {
                keys_to_remove.insert(key);
            }
        }

        let mut affected_base_keys = AHashSet::default();
        stats.purged = keys_to_remove.len();
        for key in keys_to_remove {
            if let Some((_, entry)) = self.entries.remove(&key) {
                affected_base_keys.insert(entry.base_key);
            }
        }

        for base_key in &affected_base_keys {
            self.remove_orphaned_base_key(base_key);
        }

        (stats, self.entries.len())
    }

    /// Drop a base key's variant registry once no entry references it, so the
    /// map does not leak orphaned base keys after expiry, size eviction, or
    /// purge.
    #[inline]
    fn remove_orphaned_base_key(&self, base_key: &str) {
        let has_remaining = self
            .entries
            .iter()
            .any(|(_, entry)| entry.base_key == base_key);
        if !has_remaining {
            self.variants_by_base.remove(base_key);
        }
    }

    /// Run the full expired-entry scan only if at least `CLEANUP_INTERVAL_SECS`
    /// elapsed since the last scan. Returns true when the caller's thread won
    /// the right to scan this interval.
    #[inline]
    fn should_cleanup(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = self.last_cleanup.load(Ordering::Relaxed);
        if now.saturating_sub(last) < CLEANUP_INTERVAL_SECS {
            return false;
        }
        self.last_cleanup
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    #[inline]
    fn cleanup_expired(&self) -> usize {
        if !self.should_cleanup() {
            return 0;
        }
        let expired_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                let elapsed = entry.created_at.elapsed();
                let swr_window = entry.stale_while_revalidate.unwrap_or_default();
                elapsed > entry.ttl + swr_window
            })
            .map(|(key, _)| key)
            .collect();

        let mut count = 0;
        let mut orphaned_base_keys = AHashSet::default();
        for key in expired_keys {
            if let Some((_, entry)) = self.entries.remove(&key) {
                count += 1;
                orphaned_base_keys.insert(entry.base_key);
            }
        }
        for base_key in orphaned_base_keys {
            self.remove_orphaned_base_key(&base_key);
        }
        count
    }

    /// Update headers on an existing cache entry without replacing the body.
    #[inline]
    pub fn update_entry_headers_by_key(
        &self,
        cache_key: &str,
        new_headers: HeaderMap,
        litespeed_override_cache_control: bool,
    ) -> Option<HeaderMap> {
        let mut entry = self.entries.get(cache_key)?;
        let mut new_headers = new_headers;
        strip_store_headers(&mut new_headers);
        entry.headers.remove(header::SET_COOKIE);
        merge_revalidation_headers(&mut entry.headers, new_headers);
        entry.etag = entry.headers.get(header::ETAG).cloned();
        entry.last_modified = entry.headers.get(header::LAST_MODIFIED).cloned();
        entry.created_at = Instant::now();

        let ls_control = crate::lscache::parse_litespeed_cache_control(&entry.headers);
        let (ttl, stale_while_revalidate, stale_if_error, must_revalidate) = recalculate_freshness(
            entry.scope,
            &entry.headers,
            ls_control.as_ref(),
            litespeed_override_cache_control,
        );
        entry.ttl = ttl;
        entry.stale_while_revalidate = stale_while_revalidate;
        entry.stale_if_error = stale_if_error;
        entry.must_revalidate = must_revalidate;

        let entry2 = entry.clone();
        let _ = self.entries.replace(cache_key.to_string(), entry2, false);
        Some(entry.headers.clone())
    }
}

/// Merge 304 revalidation headers into the stored headers.
///
/// Per RFC 9111 §4.3.4, field values received in a 304 response replace the
/// stored values for the same field names. Field names absent from the 304
/// keep their stored values. `HeaderMap::extend` appends instead, which would
/// accumulate duplicate `Cache-Control` (and other) values on every revalidation.
pub(crate) fn merge_revalidation_headers(stored: &mut HeaderMap, update: HeaderMap) {
    let names: Vec<http::header::HeaderName> = update.keys().cloned().collect();
    for name in names {
        stored.remove(name);
    }
    stored.extend(update);
}
