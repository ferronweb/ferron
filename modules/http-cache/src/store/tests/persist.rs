#![cfg(test)]

use super::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::store::persist::writer::{restore_zone, ZonePersistState};

static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ferron-cache-store-persist-{}-{}",
            std::process::id(),
            DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wake_pair() -> Arc<(Mutex<bool>, Condvar)> {
    Arc::new((Mutex::new(false), Condvar::new()))
}

fn persist_zone(dir: PathBuf) -> Arc<ZonePersistState> {
    Arc::new(ZonePersistState::new(
        "zone".to_string(),
        dir,
        true,
        Duration::from_secs(1),
        1024,
        wake_pair(),
    ))
}

fn attached_store(max_entries: usize, dir: PathBuf) -> (Arc<CacheStore>, Arc<ZonePersistState>) {
    let store = Arc::new(CacheStore::new(max_entries));
    let persist = persist_zone(dir);
    store.attach_persistence(persist.clone());
    (store, persist)
}

fn restore_into(store: &Arc<CacheStore>, dir: &Path) {
    let stats = restore_zone(
        dir,
        |key, entry| store.restore_entry(key, entry),
        |key| {
            store.restore_delete(&key);
        },
    );
    assert_eq!(stats.stopped, None);
}

#[test]
fn round_trip_restores_inserted_entries() {
    let dir = TempDir::new();
    let (store, persist) = attached_store(16, dir.path().clone());
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body-1",
        VaryRule::default(),
    );
    entry.purge_url = "/page".to_string();
    entry.purge_host = "example.com".to_string();
    store.insert_with_request(entry, None, &headers, &cookies);
    persist.flush_all_sync().unwrap();

    let fresh = Arc::new(CacheStore::new(16));
    restore_into(&fresh, dir.path());

    let LookupOutcome { entry: lookup, .. } =
        fresh.lookup("https://example.com/page", &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected restored cache hit");
    assert_eq!(lookup.scope, CacheScope::Public);
    assert_eq!(lookup.body, Some(Bytes::from_static(b"body-1")));
}

#[test]
fn purge_persists_tombstone() {
    let dir = TempDir::new();
    let (store, persist) = attached_store(16, dir.path().clone());
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "body",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    let (stats, len) = store.purge(
        &[PurgeOperation {
            scope: CacheScope::Public,
            selectors: vec![PurgeSelector::Url("https://example.com/page".to_string())],
            stale: false,
        }],
        None,
        None,
    );
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 0);
    persist.flush_all_sync().unwrap();

    let fresh = Arc::new(CacheStore::new(16));
    restore_into(&fresh, dir.path());

    let LookupOutcome { entry: lookup, .. } =
        fresh.lookup("https://example.com/page", &headers, &cookies, None);
    assert!(
        lookup.is_none(),
        "tombstone must suppress the restored entry"
    );
}

#[test]
fn eviction_at_capacity_records_delete() {
    let dir = TempDir::new();
    let (store, persist) = attached_store(1, dir.path().clone());
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut first = stored_entry(
        "https://example.com/a",
        CacheScope::Public,
        "a",
        VaryRule::default(),
    );
    first.purge_url = "/a".to_string();
    first.purge_host = "example.com".to_string();
    store.insert_with_request(first, None, &headers, &cookies);
    let mut second = stored_entry(
        "https://example.com/b",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    second.purge_url = "/b".to_string();
    second.purge_host = "example.com".to_string();
    store.insert_with_request(second, None, &headers, &cookies);
    persist.flush_all_sync().unwrap();

    let fresh = Arc::new(CacheStore::new(1));
    restore_into(&fresh, dir.path());

    let LookupOutcome { entry: a, .. } =
        fresh.lookup("https://example.com/a", &headers, &cookies, None);
    assert!(a.is_none(), "evicted entry must not be restored");
    let LookupOutcome { entry: b, .. } =
        fresh.lookup("https://example.com/b", &headers, &cookies, None);
    assert!(b.is_some(), "live entry must be restored");
}

#[test]
fn restore_entry_drops_expired() {
    let store = Arc::new(CacheStore::new(16));
    let mut entry = stored_entry(
        "https://example.com/old",
        CacheScope::Public,
        "stale",
        VaryRule::default(),
    );
    entry.created_at = Instant::now() - Duration::from_secs(120);
    entry.ttl = Duration::from_secs(60);
    entry.stale_while_revalidate = None;

    assert!(!store.restore_entry("https://example.com/old".to_string(), entry));

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/old",
        &HeaderMap::new(),
        &AHashMap::default(),
        None,
    );
    assert!(lookup.is_none());
}

#[test]
fn restore_entry_skips_when_capacity_zero() {
    let store = Arc::new(CacheStore::new(0));
    let entry = stored_entry(
        "https://example.com/x",
        CacheScope::Public,
        "x",
        VaryRule::default(),
    );
    assert!(!store.restore_entry("https://example.com/x".to_string(), entry));
}

#[test]
fn restore_entry_rebuilds_variants() {
    let dir = TempDir::new();
    let (store, persist) = attached_store(16, dir.path().clone());
    let cookies = AHashMap::default();

    let vary = VaryRule {
        header_names: vec![HeaderName::from_static("accept-language")],
        cookie_names: Vec::new(),
        value: None,
    };
    let headers_en = request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
    let headers_fr = request_headers(&[(&HeaderName::from_static("accept-language"), "fr-FR")]);

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "en",
            vary.clone(),
        ),
        None,
        &headers_en,
        &cookies,
    );
    store.insert_with_request(
        stored_entry("https://example.com/page", CacheScope::Public, "fr", vary),
        None,
        &headers_fr,
        &cookies,
    );
    persist.flush_all_sync().unwrap();

    let fresh = Arc::new(CacheStore::new(16));
    restore_into(&fresh, dir.path());

    let LookupOutcome { entry: en, .. } =
        fresh.lookup("https://example.com/page", &headers_en, &cookies, None);
    assert_eq!(
        en.expect("expected en variant").0.body,
        Some(Bytes::from_static(b"en"))
    );
    let LookupOutcome { entry: fr, .. } =
        fresh.lookup("https://example.com/page", &headers_fr, &cookies, None);
    assert_eq!(
        fr.expect("expected fr variant").0.body,
        Some(Bytes::from_static(b"fr"))
    );
}

#[test]
fn update_entry_headers_records_admitted_replace() {
    let dir = TempDir::new();
    let (store, persist) = attached_store(16, dir.path().clone());
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "v1",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    let mut refreshed = HeaderMap::new();
    refreshed.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    refreshed.insert(
        HeaderName::from_static("x-refreshed"),
        HeaderValue::from_static("yes"),
    );
    let stored = store.update_entry_headers_by_key(
        "https://example.com/page\nscope=public",
        refreshed.clone(),
        false,
    );
    assert!(stored.is_some());
    persist.flush_all_sync().unwrap();

    let fresh = Arc::new(CacheStore::new(16));
    restore_into(&fresh, dir.path());

    let LookupOutcome { entry: lookup, .. } =
        fresh.lookup("https://example.com/page", &headers, &cookies, None);
    let lookup = lookup.expect("expected restored cache hit").0;
    assert_eq!(
        lookup.headers.get("x-refreshed").unwrap(),
        HeaderValue::from_static("yes")
    );
}
