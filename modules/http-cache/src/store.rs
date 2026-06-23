use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet, RandomState};
use bytes::Bytes;
use dashmap::DashMap;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode};
use quick_cache::sync::Cache;
use quick_cache::{DefaultHashBuilder, Lifecycle, UnitWeighter};
use rustc_hash::FxBuildHasher;
use tokio::sync::Notify;

use crate::lscache::{PurgeOperation, PurgeSelector, ScopedTag};
use crate::policy::CacheScope;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VaryRule {
    pub header_names: Vec<HeaderName>,
    pub cookie_names: Vec<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StoredVariant {
    pub scope: CacheScope,
    pub vary: VaryRule,
}

#[derive(Clone)]
pub struct StoredEntry {
    pub scope: CacheScope,
    pub base_key: String,
    pub vary: VaryRule,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub lsc_cookies: Vec<HeaderValue>,
    pub created_at: Instant,
    pub ttl: Duration,
    pub access_at: u64,
    pub private_key: Option<String>,
    pub tags: Vec<ScopedTag>,
    pub purge_url: String,
    pub etag: Option<HeaderValue>,
    pub last_modified: Option<HeaderValue>,
}

#[derive(Clone)]
pub struct LookupEntry {
    pub scope: CacheScope,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub lsc_cookies: Vec<HeaderValue>,
    pub age: Duration,
    pub etag: Option<HeaderValue>,
    pub last_modified: Option<HeaderValue>,
}

#[derive(Default, Clone, Copy)]
pub struct StoreStats {
    pub size_evictions: usize,
    pub expired_evictions: usize,
    pub purged: usize,
}

pub struct CacheStore {
    entries: Cache<String, StoredEntry, UnitWeighter, DefaultHashBuilder, StoreLifecycle>,
    variants_by_base: DashMap<String, Vec<StoredVariant>, RandomState>,
    max_entries: AtomicUsize,
    inflight: DashMap<String, InflightEntry, FxBuildHasher>,
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
}

impl Lifecycle<String, StoredEntry> for StoreLifecycle {
    type RequestState = StoreRequestState;

    fn begin_request(&self) -> Self::RequestState {
        StoreRequestState::default()
    }

    fn on_evict(&self, state: &mut Self::RequestState, _key: String, _val: StoredEntry) {
        state.size_evictions += 1;
    }
}

impl CacheStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Cache::with(
                max_entries.max(1),
                max_entries as u64,
                UnitWeighter,
                DefaultHashBuilder::default(),
                StoreLifecycle,
            ),
            variants_by_base: DashMap::with_hasher(RandomState::new()),
            max_entries: AtomicUsize::new(max_entries),
            inflight: DashMap::with_hasher(FxBuildHasher),
        }
    }

    pub fn set_max_entries(&self, max_entries: usize) {
        self.max_entries.store(max_entries, Ordering::Relaxed);
        self.entries.set_capacity(max_entries as u64);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Try to become the in-flight fetch leader for `cache_key`.
    ///
    /// Returns `(is_leader, notify)`:
    /// - `is_leader == true`: This caller won the race. It should proceed with
    ///   the upstream fetch and call `complete_fetch(cache_key)` when done.
    /// - `is_leader == false`: Another request is already fetching. The caller
    ///   should `await notify.notified()` then re-check the cache.
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
        (is_leader, notify)
    }

    /// Complete an in-flight fetch: remove the entry and wake all waiters.
    #[inline]
    pub fn complete_fetch(&self, cache_key: &str) {
        if let Some((_, entry)) = self.inflight.remove(cache_key) {
            entry.notify.notify_waiters();
        }
    }

    pub fn lookup(
        &self,
        base_key: &str,
        headers: &HeaderMap,
        cookies: &AHashMap<String, String>,
        private_key: Option<&str>,
    ) -> (Option<(LookupEntry, String)>, StoreStats, usize, bool) {
        let stats = StoreStats {
            expired_evictions: self.cleanup_expired(),
            ..Default::default()
        };

        let has_variants = self.variants_by_base.contains_key(base_key);

        let variants = self
            .variants_by_base
            .get(base_key)
            .map(|v| v.value().clone())
            .unwrap_or_default();

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

        for key in candidate_keys {
            if let Some(entry) = self.entries.get(&key) {
                let age = entry.created_at.elapsed();
                return (
                    Some((
                        LookupEntry {
                            scope: entry.scope,
                            status: entry.status,
                            headers: entry.headers.clone(),
                            body: entry.body.clone(),
                            lsc_cookies: entry.lsc_cookies.clone(),
                            age,
                            etag: entry.etag.clone(),
                            last_modified: entry.last_modified.clone(),
                        },
                        key,
                    )),
                    stats,
                    self.entries.len(),
                    false,
                );
            }
        }

        (None, stats, self.entries.len(), has_variants)
    }

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
                variants.push(variant);
            }
        }

        let request_state = self.entries.insert_with_lifecycle(key, entry);
        stats.size_evictions = request_state.size_evictions;
        (stats, self.entries.len())
    }

    pub fn purge(
        &self,
        operations: &[PurgeOperation],
        current_private_key: Option<&str>,
    ) -> (StoreStats, usize) {
        let mut stats = StoreStats::default();
        let mut keys_to_remove = AHashSet::default();

        for (key, entry) in self.entries.iter() {
            if operations
                .iter()
                .any(|operation| entry_matches_purge(&entry, operation, current_private_key))
            {
                keys_to_remove.insert(key);
            }
        }

        stats.purged = keys_to_remove.len();
        for key in keys_to_remove {
            self.entries.remove(&key);
        }

        (stats, self.entries.len())
    }

    fn cleanup_expired(&self) -> usize {
        let expired_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.created_at.elapsed() > entry.ttl)
            .map(|(key, _)| key)
            .collect();

        let mut count = 0;
        for key in expired_keys {
            if self.entries.remove(&key).is_some() {
                count += 1;
            }
        }
        count
    }

    /// Update headers on an existing cache entry without replacing the body.
    /// Takes the full cache key and the new headers. Returns `true` if updated.
    pub fn update_entry_headers_by_key(
        &self,
        cache_key: &str,
        new_headers: HeaderMap,
    ) -> bool {
        let Some(mut entry) = self.entries.get(cache_key) else {
            return false;
        };
        entry.headers = new_headers.clone();
        entry.etag = new_headers.get(header::ETAG).cloned();
        entry.last_modified = new_headers.get(header::LAST_MODIFIED).cloned();
        entry.created_at = Instant::now();
        let entry = entry.clone();
        let _ = self.entries.replace(cache_key.to_string(), entry, false);
        true
    }
}

pub fn build_entry_key(
    base_key: &str,
    scope: CacheScope,
    private_key: Option<&str>,
    vary: &VaryRule,
    headers: &HeaderMap,
    cookies: &AHashMap<String, String>,
) -> String {
    let mut key = String::with_capacity(base_key.len() + 128);
    key.push_str(base_key);
    key.push('\n');
    key.push_str("scope=");
    key.push_str(scope.as_str());

    if scope == CacheScope::Private {
        if let Some(private_key) = private_key {
            key.push('\n');
            key.push_str("private=");
            key.push_str(private_key);
        }
    }

    for name in &vary.header_names {
        key.push('\n');
        key.push_str("h:");
        key.push_str(name.as_str());
        key.push('=');
        key.push_str(&header_values(headers, name));
    }

    for cookie_name in &vary.cookie_names {
        key.push('\n');
        key.push_str("c:");
        key.push_str(cookie_name);
        key.push('=');
        if let Some(value) = cookies.get(cookie_name) {
            key.push_str(value);
        }
    }

    if let Some(value) = &vary.value {
        key.push('\n');
        key.push_str("v:");
        key.push_str(value);
    }

    key
}

fn header_values(headers: &HeaderMap, name: &HeaderName) -> String {
    let mut values = Vec::new();
    for value in headers.get_all(name) {
        if let Ok(value) = value.to_str() {
            values.push(value.to_string());
        }
    }
    values.join(", ")
}

fn entry_matches_purge(
    entry: &StoredEntry,
    operation: &PurgeOperation,
    current_private_key: Option<&str>,
) -> bool {
    if entry.scope != operation.scope {
        return false;
    }

    if operation.scope == CacheScope::Private
        && current_private_key.is_some()
        && entry.private_key.as_deref() != current_private_key
    {
        return false;
    }

    operation.selectors.iter().any(|selector| match selector {
        PurgeSelector::All => true,
        PurgeSelector::Url(url) => entry.purge_url == *url,
        PurgeSelector::UrlPath(path) => {
            // Normalize the pathname (before "?" and "#")
            let normalized_purge_url = entry
                .purge_url
                .split_once(['?', '#'])
                .map_or(entry.purge_url.as_str(), |(url, _)| url);
            normalized_purge_url == *path
        }
        PurgeSelector::Tag(tag) => entry
            .tags
            .iter()
            .any(|entry_tag| entry_tag.scope == operation.scope && entry_tag.name == *tag),
    })
}

pub fn strip_store_headers(headers: &mut HeaderMap) {
    headers.remove(header::AGE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{AGE, CACHE_CONTROL, COOKIE};

    fn request_headers(pairs: &[(&HeaderName, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn request_cookies(pairs: &[(&str, &str)]) -> AHashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    fn stored_entry(base_key: &str, scope: CacheScope, body: &str, vary: VaryRule) -> StoredEntry {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        );

        StoredEntry {
            scope,
            base_key: base_key.to_string(),
            vary,
            status: StatusCode::OK,
            headers,
            body: Some(Bytes::from(body.to_string())),
            lsc_cookies: Vec::new(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(60),
            access_at: 0,
            private_key: None,
            tags: Vec::new(),
            purge_url: base_key.to_string(),
            etag: None,
            last_modified: None,
        }
    }

    #[test]
    fn builds_distinct_public_and_private_keys() {
        let vary = VaryRule::default();
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        let public = build_entry_key(
            "https://example.com/test",
            CacheScope::Public,
            None,
            &vary,
            &headers,
            &cookies,
        );
        let private = build_entry_key(
            "https://example.com/test",
            CacheScope::Private,
            Some("user=1"),
            &vary,
            &headers,
            &cookies,
        );

        assert_ne!(public, private);
    }

    #[test]
    fn lookup_returns_matching_public_entry() {
        let store = CacheStore::new(4);
        let base_key = "https://example.com/page";
        let vary = VaryRule {
            header_names: vec![HeaderName::from_static("accept-language")],
            cookie_names: vec!["currency".to_string()],
            value: Some("mobile".to_string()),
        };
        let headers = request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
        let cookies = request_cookies(&[("currency", "USD")]);

        let entry = stored_entry(base_key, CacheScope::Public, "cached-body", vary);
        let (stats, len) = store.insert_with_request(entry, None, &headers, &cookies);
        assert_eq!(stats.size_evictions, 0);
        assert_eq!(len, 1);

        let (lookup, stats, len, had_expired) = store.lookup(base_key, &headers, &cookies, None);
        let (lookup, _key) = lookup.expect("expected cache hit");
        assert_eq!(stats.expired_evictions, 0);
        assert_eq!(len, 1);
        assert!(!had_expired);
        assert_eq!(lookup.scope, CacheScope::Public);
        assert_eq!(lookup.status, StatusCode::OK);
        assert_eq!(lookup.body, Some(Bytes::from_static(b"cached-body")));
        assert!(lookup.age <= Duration::from_secs(1));
    }

    #[test]
    fn lookup_prefers_private_entry_for_matching_private_key() {
        let store = CacheStore::new(4);
        let base_key = "https://example.com/account";
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        let public = stored_entry(base_key, CacheScope::Public, "public", VaryRule::default());
        store.insert_with_request(public, None, &headers, &cookies);

        let private = stored_entry(
            base_key,
            CacheScope::Private,
            "private",
            VaryRule::default(),
        );
        store.insert_with_request(private, Some("user=1"), &headers, &cookies);

        let (lookup, _, _, _) = store.lookup(base_key, &headers, &cookies, Some("user=1"));
        let (lookup, _) = lookup.expect("expected private cache hit");
        assert_eq!(lookup.scope, CacheScope::Private);
        assert_eq!(lookup.body, Some(Bytes::from_static(b"private")));

        let (lookup, _, _, _) = store.lookup(base_key, &headers, &cookies, None);
        let (lookup, _) = lookup.expect("expected public cache hit");
        assert_eq!(lookup.scope, CacheScope::Public);
        assert_eq!(lookup.body, Some(Bytes::from_static(b"public")));
    }

    #[test]
    fn insert_evicts_least_recently_used_entry_at_capacity() {
        let store = CacheStore::new(2);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        store.insert_with_request(
            stored_entry(
                "https://example.com/a",
                CacheScope::Public,
                "a",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        store.insert_with_request(
            stored_entry(
                "https://example.com/b",
                CacheScope::Public,
                "b",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );

        let (lookup, _, _, _) = store.lookup("https://example.com/a", &headers, &cookies, None);
        assert!(lookup.is_some(), "expected a to become most recently used");

        let (stats, len) = store.insert_with_request(
            stored_entry(
                "https://example.com/c",
                CacheScope::Public,
                "c",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        assert_eq!(stats.size_evictions, 1);
        assert_eq!(len, 2);

        assert!(store
            .lookup("https://example.com/b", &headers, &cookies, None)
            .0
            .is_none());
        assert!(store
            .lookup("https://example.com/a", &headers, &cookies, None)
            .0
            .is_some());
        assert!(store
            .lookup("https://example.com/c", &headers, &cookies, None)
            .0
            .is_some());
    }

    #[test]
    fn set_max_entries_trims_entries_to_capacity() {
        let store = CacheStore::new(3);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        store.insert_with_request(
            stored_entry(
                "https://example.com/a",
                CacheScope::Public,
                "a",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        store.insert_with_request(
            stored_entry(
                "https://example.com/b",
                CacheScope::Public,
                "b",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        store.insert_with_request(
            stored_entry(
                "https://example.com/c",
                CacheScope::Public,
                "c",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );

        store.set_max_entries(1);

        assert_eq!(store.len(), 1);
        let survivors = [
            store
                .lookup("https://example.com/a", &headers, &cookies, None)
                .0
                .is_some(),
            store
                .lookup("https://example.com/b", &headers, &cookies, None)
                .0
                .is_some(),
            store
                .lookup("https://example.com/c", &headers, &cookies, None)
                .0
                .is_some(),
        ];
        assert_eq!(
            survivors.into_iter().filter(|survived| *survived).count(),
            1
        );
    }

    #[test]
    fn lookup_cleans_up_expired_entries() {
        let store = CacheStore::new(4);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        store.insert_with_request(
            stored_entry(
                "https://example.com/expired",
                CacheScope::Public,
                "expired",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        store.insert_with_request(
            stored_entry(
                "https://example.com/fresh",
                CacheScope::Public,
                "fresh",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );

        {
            let mut expired_entry = store
                .entries
                .get("https://example.com/expired\nscope=public")
                .expect("expected inserted expired entry");
            expired_entry.created_at = Instant::now() - Duration::from_secs(5);
            expired_entry.ttl = Duration::from_secs(1);
            assert!(store
                .entries
                .replace(
                    "https://example.com/expired\nscope=public".to_string(),
                    expired_entry,
                    false,
                )
                .is_ok());
        }

        let (lookup, stats, len, had_expired) =
            store.lookup("https://example.com/fresh", &headers, &cookies, None);
        assert!(lookup.is_some());
        assert_eq!(stats.expired_evictions, 1);
        assert_eq!(len, 1);
        assert!(!had_expired);
        assert!(store
            .lookup("https://example.com/expired", &headers, &cookies, None)
            .0
            .is_none());
    }

    #[test]
    fn purge_respects_scope_selectors_and_private_key() {
        let store = CacheStore::new(8);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        let mut public = stored_entry(
            "https://example.com/listing",
            CacheScope::Public,
            "public",
            VaryRule::default(),
        );
        public.tags = vec![ScopedTag {
            scope: CacheScope::Public,
            name: "listing".to_string(),
        }];
        public.purge_url = "/listing".to_string();
        store.insert_with_request(public, None, &headers, &cookies);

        let mut private_user_1 = stored_entry(
            "https://example.com/account",
            CacheScope::Private,
            "user-1",
            VaryRule::default(),
        );
        private_user_1.tags = vec![ScopedTag {
            scope: CacheScope::Private,
            name: "account".to_string(),
        }];
        private_user_1.purge_url = "/account".to_string();
        store.insert_with_request(private_user_1, Some("user=1"), &headers, &cookies);

        let mut private_user_2 = stored_entry(
            "https://example.com/account-2",
            CacheScope::Private,
            "user-2",
            VaryRule::default(),
        );
        private_user_2.tags = vec![ScopedTag {
            scope: CacheScope::Private,
            name: "account".to_string(),
        }];
        private_user_2.purge_url = "/account".to_string();
        store.insert_with_request(private_user_2, Some("user=2"), &headers, &cookies);

        let operations = vec![
            PurgeOperation {
                scope: CacheScope::Public,
                selectors: vec![PurgeSelector::Url("/listing".to_string())],
                stale: false,
            },
            PurgeOperation {
                scope: CacheScope::Private,
                selectors: vec![PurgeSelector::Tag("account".to_string())],
                stale: false,
            },
        ];

        let (stats, len) = store.purge(&operations, Some("user=1"));
        assert_eq!(stats.purged, 2);
        assert_eq!(len, 1);
        assert!(store
            .lookup("https://example.com/listing", &headers, &cookies, None)
            .0
            .is_none());
        assert!(store
            .lookup(
                "https://example.com/account",
                &headers,
                &cookies,
                Some("user=1")
            )
            .0
            .is_none());
        let remaining = store
            .lookup(
                "https://example.com/account-2",
                &headers,
                &cookies,
                Some("user=2"),
            )
            .0
            .expect("expected unmatched private entry to remain");
        assert_eq!(remaining.0.body, Some(Bytes::from_static(b"user-2")));
    }

    #[test]
    fn zero_capacity_store_skips_insert() {
        let store = CacheStore::new(0);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        let (stats, len) = store.insert_with_request(
            stored_entry(
                "https://example.com/a",
                CacheScope::Public,
                "a",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );

        assert_eq!(stats.size_evictions, 0);
        assert_eq!(len, 0);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn strip_store_headers_removes_age_only() {
        let mut headers = HeaderMap::new();
        headers.insert(AGE, HeaderValue::from_static("60"));
        headers.insert(COOKIE, HeaderValue::from_static("a=b"));

        strip_store_headers(&mut headers);

        assert!(!headers.contains_key(AGE));
        assert_eq!(headers.get(COOKIE), Some(&HeaderValue::from_static("a=b")));
    }

    #[test]
    fn had_expired_is_true_when_entry_expired() {
        let store = CacheStore::new(4);
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        store.insert_with_request(
            stored_entry(
                "https://example.com/exp",
                CacheScope::Public,
                "data",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );

        // Expire the entry
        {
            let mut entry = store
                .entries
                .get("https://example.com/exp\nscope=public")
                .expect("expected inserted entry");
            entry.created_at = Instant::now() - Duration::from_secs(10);
            entry.ttl = Duration::from_secs(1);
            assert!(store
                .entries
                .replace(
                    "https://example.com/exp\nscope=public".to_string(),
                    entry,
                    false,
                )
                .is_ok());
        }

        // First-time key returns false
        let (_, _, _, had_expired) =
            store.lookup("https://example.com/new", &headers, &cookies, None);
        assert!(!had_expired);

        // Expired key returns true (variants exist, but no valid entries)
        let (_, _, _, had_expired) =
            store.lookup("https://example.com/exp", &headers, &cookies, None);
        assert!(had_expired);
    }

    #[test]
    fn begin_fetch_returns_leader_and_follower() {
        let store = CacheStore::new(4);
        let key = "https://example.com/concurrent";

        let (is_leader1, _notify1) = store.begin_fetch(key);
        assert!(is_leader1, "first caller should be leader");

        let (is_leader2, _notify2) = store.begin_fetch(key);
        assert!(!is_leader2, "second caller should be follower");

        let (is_leader3, _notify3) = store.begin_fetch(key);
        assert!(!is_leader3, "third caller should be follower");

        // Clean up
        store.complete_fetch(key);
    }

    #[test]
    fn complete_fetch_notifies_waiters() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let store = CacheStore::new(4);
        let key = "https://example.com/notify-test";

        let (_leader, _leader_notify) = store.begin_fetch(key);
        let (follower, follower_notify) = store.begin_fetch(key);

        assert!(!follower);

        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                follower_notify.notified().await;
                fired_clone.store(true, Ordering::SeqCst);
            });
        });

        // Give the thread time to start waiting
        std::thread::sleep(std::time::Duration::from_millis(50));

        store.complete_fetch(key);
        handle.join().unwrap();

        assert!(fired.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn concurrent_misses_coalesce_to_single_upstream_fetch() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let store = Arc::new(CacheStore::new(4));
        let base_key = "https://example.com/popular";

        // Insert an entry and then expire it
        let headers = HeaderMap::new();
        let cookies = AHashMap::default();
        store.insert_with_request(
            stored_entry(base_key, CacheScope::Public, "data", VaryRule::default()),
            None,
            &headers,
            &cookies,
        );
        {
            let mut entry = store
                .entries
                .get(&format!("{base_key}\nscope=public"))
                .expect("expected entry");
            entry.created_at = Instant::now() - Duration::from_secs(10);
            entry.ttl = Duration::from_secs(1);
            store
                .entries
                .replace(format!("{base_key}\nscope=public"), entry, false)
                .ok();
        }

        // Verify lookup returns had_expired
        let (_, _, _, had_expired) = store.lookup(base_key, &headers, &cookies, None);
        assert!(had_expired);

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..10 {
            let store = store.clone();
            let fetch_count = fetch_count.clone();
            let base_key = base_key.to_string();
            let headers = headers.clone();
            let cookies = cookies.clone();

            handles.push(tokio::spawn(async move {
                let (is_leader, notify) = store.begin_fetch(&base_key);

                #[allow(clippy::needless_return)]
                if !is_leader {
                    // Follower: wait for leader to complete
                    notify.notified().await;
                    // Re-check cache
                    let (lookup, _, _, _) = store.lookup(&base_key, &headers, &cookies, None);
                    if lookup.is_some() {
                        return;
                    }
                } else {
                    // Leader: simulate upstream fetch
                    fetch_count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

                    // Store the response
                    store.insert_with_request(
                        stored_entry(
                            &base_key,
                            CacheScope::Public,
                            "fresh-data",
                            VaryRule::default(),
                        ),
                        None,
                        &headers,
                        &cookies,
                    );
                    store.complete_fetch(&base_key);
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // Only one upstream fetch should have occurred
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "only one leader should fetch upstream"
        );

        // Cache should now have the entry
        let (lookup, _, _, _) = store.lookup(base_key, &headers, &cookies, None);
        assert!(
            lookup.is_some(),
            "cache should be populated after coalesced fetch"
        );
    }

    #[tokio::test]
    async fn follower_gets_cached_response_after_leader_stores() {
        let store = Arc::new(CacheStore::new(4));
        let base_key = "https://example.com/leader-follower";

        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        // Insert and expire
        store.insert_with_request(
            stored_entry(base_key, CacheScope::Public, "old", VaryRule::default()),
            None,
            &headers,
            &cookies,
        );
        {
            let mut entry = store
                .entries
                .get(&format!("{base_key}\nscope=public"))
                .expect("expected entry");
            entry.created_at = Instant::now() - Duration::from_secs(10);
            entry.ttl = Duration::from_secs(1);
            store
                .entries
                .replace(format!("{base_key}\nscope=public"), entry, false)
                .ok();
        }

        // Leader begins
        let (is_leader, notify) = store.begin_fetch(base_key);
        assert!(is_leader);

        // Spawn a follower that waits
        let store_clone = store.clone();
        let base_key_clone = base_key.to_string();
        let headers_clone = headers.clone();
        let cookies_clone = cookies.clone();
        let follower_handle = tokio::spawn(async move {
            // Follower waits
            notify.notified().await;
            // After notification, re-check cache
            let (lookup, _, _, _) =
                store_clone.lookup(&base_key_clone, &headers_clone, &cookies_clone, None);
            lookup.and_then(|(entry, _)| entry.body)
        });

        // Give follower time to start waiting
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Leader stores the response
        store.insert_with_request(
            stored_entry(
                base_key,
                CacheScope::Public,
                "new-data",
                VaryRule::default(),
            ),
            None,
            &headers,
            &cookies,
        );
        store.complete_fetch(base_key);

        // Follower should get the new cached response
        let body = follower_handle.await.unwrap();
        assert_eq!(body, Some(Bytes::from_static(b"new-data")));
    }

    #[tokio::test]
    async fn leader_non_cacheable_wakes_followers_without_cached_entry() {
        let store = Arc::new(CacheStore::new(4));
        let base_key = "https://example.com/non-cacheable";

        let headers = HeaderMap::new();
        let cookies = AHashMap::default();

        // Insert and expire
        store.insert_with_request(
            stored_entry(base_key, CacheScope::Public, "old", VaryRule::default()),
            None,
            &headers,
            &cookies,
        );
        {
            let mut entry = store
                .entries
                .get(&format!("{base_key}\nscope=public"))
                .expect("expected entry");
            entry.created_at = Instant::now() - Duration::from_secs(10);
            entry.ttl = Duration::from_secs(1);
            store
                .entries
                .replace(format!("{base_key}\nscope=public"), entry, false)
                .ok();
        }

        // Leader begins
        let (is_leader, notify) = store.begin_fetch(base_key);
        assert!(is_leader);

        let store_clone = store.clone();
        let base_key_clone = base_key.to_string();
        let headers_clone = headers.clone();
        let cookies_clone = cookies.clone();
        let follower_handle = tokio::spawn(async move {
            notify.notified().await;
            let (lookup, _, _, _) =
                store_clone.lookup(&base_key_clone, &headers_clone, &cookies_clone, None);
            lookup.is_none() // Should be None since leader didn't store
        });

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Leader decides NOT to store (non-cacheable response)
        // Just complete the fetch without inserting
        store.complete_fetch(base_key);

        let follower_saw_miss = follower_handle.await.unwrap();
        assert!(
            follower_saw_miss,
            "follower should see miss after non-cacheable leader"
        );
    }

    #[test]
    fn stored_entry_preserves_etag_and_last_modified() {
        let store = CacheStore::new(4);
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        );
        headers.insert(
            header::ETAG,
            HeaderValue::from_static(r#"W/"abc123""#),
        );
        headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_static("Tue, 01 Jan 2024 00:00:00 GMT"),
        );

        let entry = StoredEntry {
            scope: CacheScope::Public,
            base_key: "https://example.com/test".to_string(),
            vary: VaryRule::default(),
            status: StatusCode::OK,
            headers: headers.clone(),
            body: Some(Bytes::from_static(b"body")),
            lsc_cookies: Vec::new(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(60),
            access_at: 0,
            private_key: None,
            tags: Vec::new(),
            purge_url: "/test".to_string(),
            etag: headers.get(header::ETAG).cloned(),
            last_modified: headers.get(header::LAST_MODIFIED).cloned(),
        };

        let request_headers = HeaderMap::new();
        let request_cookies = AHashMap::default();
        store.insert_with_request(entry, None, &request_headers, &request_cookies);

        let (lookup, _, _, _) = store.lookup(
            "https://example.com/test",
            &request_headers,
            &request_cookies,
            None,
        );
        let (entry, _key) = lookup.expect("expected cache hit");
        assert_eq!(
            entry.etag.as_ref().and_then(|v| v.to_str().ok()),
            Some(r#"W/"abc123""#)
        );
        assert_eq!(
            entry.last_modified.as_ref().and_then(|v| v.to_str().ok()),
            Some("Tue, 01 Jan 2024 00:00:00 GMT")
        );
    }

    #[test]
    fn update_entry_headers_by_key_updates_validators() {
        let store = CacheStore::new(4);
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        );
        headers.insert(
            header::ETAG,
            HeaderValue::from_static(r#"W/"old""#),
        );

        let entry = StoredEntry {
            scope: CacheScope::Public,
            base_key: "https://example.com/test".to_string(),
            vary: VaryRule::default(),
            status: StatusCode::OK,
            headers: headers.clone(),
            body: Some(Bytes::from_static(b"body")),
            lsc_cookies: Vec::new(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(60),
            access_at: 0,
            private_key: None,
            tags: Vec::new(),
            purge_url: "/test".to_string(),
            etag: headers.get(header::ETAG).cloned(),
            last_modified: None,
        };

        let request_headers = HeaderMap::new();
        let request_cookies = AHashMap::default();
        store.insert_with_request(entry, None, &request_headers, &request_cookies);

        let (lookup, _, _, _) = store.lookup(
            "https://example.com/test",
            &request_headers,
            &request_cookies,
            None,
        );
        let (_entry, cache_key) = lookup.expect("expected cache hit");

        let mut new_headers = HeaderMap::new();
        new_headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        new_headers.insert(
            header::ETAG,
            HeaderValue::from_static(r#"W/"new""#),
        );
        new_headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_static("Wed, 02 Jan 2024 00:00:00 GMT"),
        );

        let updated = store.update_entry_headers_by_key(&cache_key, new_headers);
        assert!(updated);

        let (lookup, _, _, _) = store.lookup(
            "https://example.com/test",
            &request_headers,
            &request_cookies,
            None,
        );
        let (entry, _) = lookup.expect("expected cache hit after update");
        assert_eq!(
            entry.etag.as_ref().and_then(|v| v.to_str().ok()),
            Some(r#"W/"new""#)
        );
        assert_eq!(
            entry.last_modified.as_ref().and_then(|v| v.to_str().ok()),
            Some("Wed, 02 Jan 2024 00:00:00 GMT")
        );
        // Body should be preserved
        assert_eq!(entry.body, Some(Bytes::from_static(b"body")));
    }

    #[test]
    fn lookup_returns_cache_key_for_revalidation() {
        let store = CacheStore::new(4);
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        );
        headers.insert(
            header::ETAG,
            HeaderValue::from_static(r#"W/"test-etag""#),
        );

        let entry = StoredEntry {
            scope: CacheScope::Public,
            base_key: "https://example.com/test".to_string(),
            vary: VaryRule::default(),
            status: StatusCode::OK,
            headers: headers.clone(),
            body: Some(Bytes::from_static(b"body")),
            lsc_cookies: Vec::new(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(60),
            access_at: 0,
            private_key: None,
            tags: Vec::new(),
            purge_url: "/test".to_string(),
            etag: headers.get(header::ETAG).cloned(),
            last_modified: None,
        };

        let request_headers = HeaderMap::new();
        let request_cookies = AHashMap::default();
        store.insert_with_request(entry, None, &request_headers, &request_cookies);

        let (lookup, _, _, _) = store.lookup(
            "https://example.com/test",
            &request_headers,
            &request_cookies,
            None,
        );
        let (_entry, cache_key) = lookup.expect("expected cache hit");
        assert!(cache_key.starts_with("https://example.com/test"));
        assert!(cache_key.contains("scope=public"));
    }
}
