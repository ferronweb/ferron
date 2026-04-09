use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode};
use parking_lot::RwLock;
use quick_cache::{sync::Cache, DefaultHashBuilder, Lifecycle, UnitWeighter};
use rustc_hash::{FxHashMap, FxHashSet};

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
    pub body: Bytes,
    pub lsc_cookies: Vec<HeaderValue>,
    pub created_at: Instant,
    pub ttl: Duration,
    pub access_at: u64,
    pub private_key: Option<String>,
    pub tags: Vec<ScopedTag>,
    pub purge_url: String,
}

#[derive(Clone)]
pub struct LookupEntry {
    pub scope: CacheScope,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub lsc_cookies: Vec<HeaderValue>,
    pub age: Duration,
}

#[derive(Default, Clone, Copy)]
pub struct StoreStats {
    pub size_evictions: usize,
    pub expired_evictions: usize,
    pub purged: usize,
}

pub struct CacheStore {
    entries: Cache<String, StoredEntry, UnitWeighter, DefaultHashBuilder, StoreLifecycle>,
    variants_by_base: RwLock<FxHashMap<String, Vec<StoredVariant>>>,
    max_entries: AtomicUsize,
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
            variants_by_base: RwLock::new(FxHashMap::default()),
            max_entries: AtomicUsize::new(max_entries),
        }
    }

    pub fn set_max_entries(&self, max_entries: usize) {
        self.max_entries.store(max_entries, Ordering::Relaxed);
        self.entries.set_capacity(max_entries as u64);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn lookup(
        &self,
        base_key: &str,
        headers: &HeaderMap,
        cookies: &FxHashMap<String, String>,
        private_key: Option<&str>,
    ) -> (Option<LookupEntry>, StoreStats, usize) {
        let mut stats = StoreStats::default();
        stats.expired_evictions = self.cleanup_expired();

        let variants = self
            .variants_by_base
            .read()
            .get(base_key)
            .cloned()
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
                    Some(LookupEntry {
                        scope: entry.scope,
                        status: entry.status,
                        headers: entry.headers.clone(),
                        body: entry.body.clone(),
                        lsc_cookies: entry.lsc_cookies.clone(),
                        age,
                    }),
                    stats,
                    self.entries.len(),
                );
            }
        }

        (None, stats, self.entries.len())
    }

    pub fn insert_with_request(
        &self,
        mut entry: StoredEntry,
        private_key: Option<&str>,
        request_headers: &HeaderMap,
        request_cookies: &FxHashMap<String, String>,
    ) -> (StoreStats, usize) {
        let mut stats = StoreStats::default();
        stats.expired_evictions = self.cleanup_expired();

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
            let mut variants_by_base = self.variants_by_base.write();
            let variants = variants_by_base.entry(entry.base_key.clone()).or_default();
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
        let mut keys_to_remove = FxHashSet::default();

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
}

pub fn build_entry_key(
    base_key: &str,
    scope: CacheScope,
    private_key: Option<&str>,
    vary: &VaryRule,
    headers: &HeaderMap,
    cookies: &FxHashMap<String, String>,
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

    fn request_cookies(pairs: &[(&str, &str)]) -> FxHashMap<String, String> {
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
            body: Bytes::from(body.to_string()),
            lsc_cookies: Vec::new(),
            created_at: Instant::now(),
            ttl: Duration::from_secs(60),
            access_at: 0,
            private_key: None,
            tags: Vec::new(),
            purge_url: base_key.to_string(),
        }
    }

    #[test]
    fn builds_distinct_public_and_private_keys() {
        let vary = VaryRule::default();
        let headers = HeaderMap::new();
        let cookies = FxHashMap::default();

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

        let (lookup, stats, len) = store.lookup(base_key, &headers, &cookies, None);
        let lookup = lookup.expect("expected cache hit");
        assert_eq!(stats.expired_evictions, 0);
        assert_eq!(len, 1);
        assert_eq!(lookup.scope, CacheScope::Public);
        assert_eq!(lookup.status, StatusCode::OK);
        assert_eq!(lookup.body, Bytes::from_static(b"cached-body"));
        assert!(lookup.age <= Duration::from_secs(1));
    }

    #[test]
    fn lookup_prefers_private_entry_for_matching_private_key() {
        let store = CacheStore::new(4);
        let base_key = "https://example.com/account";
        let headers = HeaderMap::new();
        let cookies = FxHashMap::default();

        let public = stored_entry(base_key, CacheScope::Public, "public", VaryRule::default());
        store.insert_with_request(public, None, &headers, &cookies);

        let private = stored_entry(
            base_key,
            CacheScope::Private,
            "private",
            VaryRule::default(),
        );
        store.insert_with_request(private, Some("user=1"), &headers, &cookies);

        let (lookup, _, _) = store.lookup(base_key, &headers, &cookies, Some("user=1"));
        let lookup = lookup.expect("expected private cache hit");
        assert_eq!(lookup.scope, CacheScope::Private);
        assert_eq!(lookup.body, Bytes::from_static(b"private"));

        let (lookup, _, _) = store.lookup(base_key, &headers, &cookies, None);
        let lookup = lookup.expect("expected public cache hit");
        assert_eq!(lookup.scope, CacheScope::Public);
        assert_eq!(lookup.body, Bytes::from_static(b"public"));
    }

    #[test]
    fn insert_evicts_least_recently_used_entry_at_capacity() {
        let store = CacheStore::new(2);
        let headers = HeaderMap::new();
        let cookies = FxHashMap::default();

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

        let (lookup, _, _) = store.lookup("https://example.com/a", &headers, &cookies, None);
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
        let cookies = FxHashMap::default();

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
        let cookies = FxHashMap::default();

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

        let (lookup, stats, len) =
            store.lookup("https://example.com/fresh", &headers, &cookies, None);
        assert!(lookup.is_some());
        assert_eq!(stats.expired_evictions, 1);
        assert_eq!(len, 1);
        assert!(store
            .lookup("https://example.com/expired", &headers, &cookies, None)
            .0
            .is_none());
    }

    #[test]
    fn purge_respects_scope_selectors_and_private_key() {
        let store = CacheStore::new(8);
        let headers = HeaderMap::new();
        let cookies = FxHashMap::default();

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
        assert_eq!(remaining.body, Bytes::from_static(b"user-2"));
    }

    #[test]
    fn zero_capacity_store_skips_insert() {
        let store = CacheStore::new(0);
        let headers = HeaderMap::new();
        let cookies = FxHashMap::default();

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
}
