#![cfg(test)]

use super::*;

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::CACHE_CONTROL;
use http::HeaderMap;
use http::HeaderValue;
use tokio::sync::Notify;

use crate::policy::CacheScope;
use crate::store::persist::record::{decode_next, DecodedRecord};
use crate::store::persist::writer::{
    restore_zone, sanitize_zone_label, PersistManager, RestoreStop, ZonePersistState, JOURNAL_FILE,
    SNAPSHOT_FILE,
};
use crate::store::types::StoredEntry;

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

fn wake_pair() -> Arc<Notify> {
    Arc::new(Notify::new())
}

fn persist_zone(dir: PathBuf) -> Arc<ZonePersistState> {
    Arc::new(ZonePersistState::new(
        "zone".to_string(),
        dir,
        true,
        Duration::from_secs(1),
        1024,
        wake_pair(),
        None,
    ))
}

fn attached_store(max_entries: usize, dir: PathBuf) -> (Arc<CacheStore>, Arc<ZonePersistState>) {
    let store = Arc::new(CacheStore::new(max_entries));
    let persist = persist_zone(dir);
    store.attach_persistence(persist.clone());
    (store, persist)
}

async fn restore_into(store: &Arc<CacheStore>, dir: &Path) {
    let stats = restore_zone(
        dir,
        |key, entry| store.restore_entry(key, entry),
        |key| {
            store.restore_delete(&key);
        },
    )
    .await;
    assert_eq!(stats.stopped, None);
}

#[test]
fn round_trip_restores_inserted_entries() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
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
            restore_into(&fresh, dir.path()).await;

            let LookupOutcome { entry: lookup, .. } =
                fresh.lookup("https://example.com/page", &headers, &cookies, None);
            let (lookup, _, _) = lookup.expect("expected restored cache hit");
            assert_eq!(lookup.scope, CacheScope::Public);
            assert_eq!(lookup.body, Some(Bytes::from_static(b"body-1")));
        });
}

#[test]
fn purge_persists_tombstone() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
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
            restore_into(&fresh, dir.path()).await;

            let LookupOutcome { entry: lookup, .. } =
                fresh.lookup("https://example.com/page", &headers, &cookies, None);
            assert!(
                lookup.is_none(),
                "tombstone must suppress the restored entry"
            );
        })
}

#[test]
fn eviction_at_capacity_records_delete() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
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
            restore_into(&fresh, dir.path()).await;

            let LookupOutcome { entry: a, .. } =
                fresh.lookup("https://example.com/a", &headers, &cookies, None);
            assert!(a.is_none(), "evicted entry must not be restored");
            let LookupOutcome { entry: b, .. } =
                fresh.lookup("https://example.com/b", &headers, &cookies, None);
            assert!(b.is_some(), "live entry must be restored");
        })
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
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let (store, persist) = attached_store(16, dir.path().clone());
            let cookies = AHashMap::default();

            let vary = VaryRule {
                header_names: vec![HeaderName::from_static("accept-language")],
                cookie_names: Vec::new(),
                value: None,
                no_vary: false,
            };
            let headers_en =
                request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
            let headers_fr =
                request_headers(&[(&HeaderName::from_static("accept-language"), "fr-FR")]);

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
            restore_into(&fresh, dir.path()).await;

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
        });
}

#[test]
fn update_entry_headers_records_admitted_replace() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
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
            restore_into(&fresh, dir.path()).await;

            let LookupOutcome { entry: lookup, .. } =
                fresh.lookup("https://example.com/page", &headers, &cookies, None);
            let lookup = lookup.expect("expected restored cache hit").0;
            assert_eq!(
                lookup.headers.get("x-refreshed").unwrap(),
                HeaderValue::from_static("yes")
            );
        });
}

fn public_entry() -> StoredEntry {
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    StoredEntry {
        scope: CacheScope::Public,
        base_key: "https://example.com/page".to_string(),
        vary: Default::default(),
        status: http::StatusCode::OK,
        headers: std::sync::Arc::new(headers),
        body: Some(Bytes::from_static(b"body")),
        lsc_cookies: std::sync::Arc::new(Vec::new()),
        created_at: std::time::Instant::now(),
        ttl: Duration::from_secs(60),
        access_at: 0,
        private_key: None,
        tags: Vec::new(),
        purge_url: "/page".to_string(),
        purge_host: "example.com".to_string(),
        etag: None,
        last_modified: None,
        stale_while_revalidate: None,
        stale_if_error: None,
        must_revalidate: false,
    }
}

fn decode_file(path: &std::path::Path) -> Vec<DecodedRecord> {
    let mut data = Vec::new();
    std::fs::File::open(path)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    let mut records = Vec::new();
    let mut pos = 0;
    while let Some((record, next)) = decode_next(&data, pos).unwrap() {
        records.push(record);
        pos = next;
    }
    records
}

#[test]
fn journal_roundtrip_in_order() {
    let dir = TempDir::new();
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        4,
        wake_pair(),
        None,
    );
    zone.record_put("k1", &public_entry());
    zone.record_delete("k2");
    zone.record_put("k3", &public_entry());
    zone.flush_all_sync().unwrap();

    let records = decode_file(&zone.journal_path());
    assert_eq!(records.len(), 3);
    match &records[0] {
        DecodedRecord::Put { key, .. } => assert_eq!(key, "k1"),
        _ => panic!("expected Put"),
    }
    assert_eq!(records[1], DecodedRecord::Delete { key: "k2".into() });
    match &records[2] {
        DecodedRecord::Put { key, .. } => assert_eq!(key, "k3"),
        _ => panic!("expected Put"),
    }
}

#[test]
fn append_across_reopen() {
    let dir = TempDir::new();
    {
        let manager = PersistManager::new();
        let zone = manager.register_zone(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_secs(1),
        );
        zone.record_put("k1", &public_entry());
        zone.flush_all_sync().unwrap();
    }
    // A fresh manager on the same directory continues the journal.
    let manager = PersistManager::new();
    let zone = manager.register_zone(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
    );
    zone.record_put("k2", &public_entry());
    zone.flush_all_sync().unwrap();

    let records = decode_file(&zone.journal_path());
    assert_eq!(records.len(), 2);
    match &records[0] {
        DecodedRecord::Put { key, .. } => assert_eq!(key, "k1"),
        _ => panic!("expected Put"),
    }
    match &records[1] {
        DecodedRecord::Put { key, .. } => assert_eq!(key, "k2"),
        _ => panic!("expected Put"),
    }
}

#[test]
fn private_entries_filtered() {
    let dir = TempDir::new();
    let mut entry = public_entry();
    entry.scope = CacheScope::Private;
    entry.private_key = Some("user=alice".to_string());

    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        8,
        wake_pair(),
        None,
    );
    zone.record_put("k1", &entry);
    zone.flush_all_sync().unwrap();
    // Nothing was queued: no journal file was created.
    assert!(!zone.journal_path().exists());

    // Tombsones are never filtered.
    zone.record_delete("k1");
    zone.flush_all_sync().unwrap();
    assert_eq!(
        decode_file(&zone.journal_path()),
        vec![DecodedRecord::Delete { key: "k1".into() }]
    );

    // With include_private, private entries are written.
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        true,
        Duration::from_secs(1),
        8,
        wake_pair(),
        None,
    );
    zone.record_put("k2", &entry);
    zone.flush_all_sync().unwrap();
    let records = decode_file(&zone.journal_path());
    match records.last().unwrap() {
        DecodedRecord::Put {
            key,
            entry: decoded,
        } => {
            assert_eq!(key, "k2");
            assert_eq!(decoded.scope, CacheScope::Private);
        }
        _ => panic!("expected Put"),
    }
}

#[test]
fn overflow_drops_oldest_prefix() {
    let dir = TempDir::new();
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        4,
        wake_pair(),
        None,
    );
    // Fill to capacity, then overflow.
    for i in 0..6 {
        zone.record_put(&format!("k{i}"), &public_entry());
    }
    // Capacity 4: at the 5th push two records are dropped (len 4 -> 2),
    // the 6th push fits. The journal holds only k2..k5.
    assert_eq!(zone.dropped_records(), 2);
    zone.flush_all_sync().unwrap();
    let records = decode_file(&zone.journal_path());
    let keys: Vec<&str> = records
        .iter()
        .map(|r| match r {
            DecodedRecord::Put { key, .. } => key.as_str(),
            DecodedRecord::Delete { key } => key.as_str(),
        })
        .collect();
    assert_eq!(keys, vec!["k2", "k3", "k4", "k5"]);
}

#[test]
fn io_failure_disables_zone() {
    let dir = TempDir::new();
    // Make the journal path unopenable: a directory in its place.
    std::fs::create_dir(dir.path().join(JOURNAL_FILE)).unwrap();

    let manager = PersistManager::new();
    let zone = manager.register_zone(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
    );
    zone.record_put("k1", &public_entry());
    manager.flush_all();
    assert!(!zone.is_active());
    assert!(zone.last_error().is_some());

    // Subsequent records are dropped and further flushes are no-ops.
    zone.record_put("k2", &public_entry());
    manager.flush_all();
    assert!(!zone.is_active());
    // The first (and only) reported error is unchanged.
    assert!(zone.last_error().is_some());
}

#[test]
fn flush_due_respects_interval() {
    let dir = TempDir::new();
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(3600),
        8,
        wake_pair(),
        None,
    );
    // Due immediately after creation (no flush yet this "interval").
    assert!(zone.flush_due());
    zone.record_put("k1", &public_entry());
    zone.flush_all_sync().unwrap();
    assert!(!zone.flush_due());

    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_millis(10),
        8,
        wake_pair(),
        None,
    );
    assert!(zone.flush_due());
    zone.flush_all_sync().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(zone.flush_due());
}

#[test]
fn sanitize_zone_label_replaces_unsafe_characters() {
    assert_eq!(sanitize_zone_label("example.com"), "example.com");
    assert_eq!(sanitize_zone_label("zone one!@#"), "zone_one___");
    assert_eq!(sanitize_zone_label("a/b\\c:d*e?\"f"), "a_b_c_d_e__f");
    assert!(sanitize_zone_label("ünïcodé")
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')));
}

#[test]
fn writer_task_flushes_periodically() {
    let dir = TempDir::new();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let manager = PersistManager::new();
    manager.start_on(rt.handle());
    let zone = manager.register_zone(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_millis(20),
    );
    zone.record_put("k1", &public_entry());

    // Poll until the writer task has written the journal.
    let mut found = false;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        if zone.journal_path().exists() {
            let records = decode_file(&zone.journal_path());
            if matches!(&records.first(), Some(DecodedRecord::Put { key, .. }) if key == "k1") {
                found = true;
                break;
            }
        }
    }
    assert!(found, "writer task did not flush the journal");
    manager.stop();
    // Give the task a chance to observe the stop and exit cleanly.
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn compact_writes_snapshot_and_truncates_journal() {
    let dir = TempDir::new();
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        8,
        wake_pair(),
        None,
    );
    let mut live = HashMap::new();
    live.insert("k1".to_string(), public_entry());
    live.insert("k2".to_string(), public_entry());
    zone.register_entry_source(Box::new(move |visit| {
        for (key, entry) in &live {
            visit(key, entry);
        }
    }));

    zone.record_put("k1", &public_entry());
    zone.record_put("k2", &public_entry());
    zone.record_put("k3", &public_entry());
    zone.flush_all_sync().unwrap();
    assert_eq!(decode_file(&zone.journal_path()).len(), 3);

    let stats = zone.compact().unwrap();
    assert_eq!(stats.entries, 2);

    // The snapshot holds only the live entries, the journal is empty.
    let mut snapshot_keys: Vec<String> = decode_file(&dir.path().join(SNAPSHOT_FILE))
        .iter()
        .map(|record| match record {
            DecodedRecord::Put { key, .. } => key.clone(),
            DecodedRecord::Delete { key } => key.clone(),
        })
        .collect();
    snapshot_keys.sort();
    assert_eq!(snapshot_keys, vec!["k1", "k2"]);
    assert!(decode_file(&zone.journal_path()).is_empty());

    // Appends continue in the truncated journal.
    zone.record_put("k4", &public_entry());
    zone.flush_all_sync().unwrap();
    let journal = decode_file(&zone.journal_path());
    assert_eq!(journal.len(), 1);
    match &journal[0] {
        DecodedRecord::Put { key, .. } => assert_eq!(key, "k4"),
        _ => panic!("expected Put"),
    }
}

#[test]
fn compact_without_source_is_noop() {
    let dir = TempDir::new();
    let zone = ZonePersistState::new(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        8,
        wake_pair(),
        None,
    );
    zone.record_put("k1", &public_entry());
    zone.flush_all_sync().unwrap();
    zone.compact().unwrap();
    assert!(!dir.path().join(SNAPSHOT_FILE).exists());
    assert_eq!(decode_file(&zone.journal_path()).len(), 1);
}

#[test]
fn maybe_compact_respects_interval() {
    let dir = TempDir::new();
    let zone = ZonePersistState::with_compact_interval(
        "zone".to_string(),
        dir.path().clone(),
        false,
        Duration::from_secs(1),
        Duration::from_millis(10),
        8,
        wake_pair(),
        None,
    );
    zone.register_entry_source(Box::new(|visit| {
        let _ = visit;
    }));
    assert!(zone.compact_due());
    assert!(zone.maybe_compact().unwrap());
    assert!(!zone.compact_due());
    std::thread::sleep(Duration::from_millis(30));
    assert!(zone.compact_due());
    assert!(zone.maybe_compact().unwrap());
}

#[test]
fn restore_replays_snapshot_then_journal() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let zone = ZonePersistState::new(
                "zone".to_string(),
                dir.path().clone(),
                false,
                Duration::from_secs(1),
                8,
                wake_pair(),
                None,
            );
            let mut live = HashMap::new();
            live.insert("k1".to_string(), public_entry());
            live.insert("k2".to_string(), public_entry());
            zone.register_entry_source(Box::new(move |visit| {
                for (key, entry) in &live {
                    visit(key, entry);
                }
            }));
            zone.compact().unwrap();
            zone.record_put("k3", &public_entry());
            zone.record_delete("k1");
            zone.flush_all_sync().unwrap();

            let mut puts = Vec::new();
            let mut deletes = Vec::new();
            let stats = restore_zone(
                dir.path(),
                |key, _entry| {
                    puts.push(key);
                    true
                },
                |key| deletes.push(key),
            )
            .await;
            assert_eq!(stats.records, 4);
            assert_eq!(stats.puts, 3);
            assert_eq!(stats.deletes, 1);
            assert_eq!(stats.skipped, 0);
            assert_eq!(stats.stopped, None);
            puts.sort();
            deletes.sort();
            assert_eq!(puts, vec!["k1", "k2", "k3"]);
            assert_eq!(deletes, vec!["k1"]);
        });
}

#[test]
fn restore_absent_files_is_clean() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let stats = restore_zone(dir.path(), |_, _| true, |_| {}).await;
            assert_eq!(stats.records, 0);
            assert_eq!(stats.puts, 0);
            assert_eq!(stats.stopped, None);
        });
}

#[test]
fn restore_reports_corrupt_journal() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let zone = ZonePersistState::new(
                "zone".to_string(),
                dir.path().clone(),
                false,
                Duration::from_secs(1),
                8,
                wake_pair(),
                None,
            );
            zone.record_put("k1", &public_entry());
            zone.flush_all_sync().unwrap();
            {
                use std::io::Write;
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(zone.journal_path())
                    .unwrap();
                // 12 garbage bytes: the length field is implausible.
                file.write_all(&[0xffu8; 12]).unwrap();
            }
            let mut puts = 0;
            let stats = restore_zone(
                dir.path(),
                |_, _| {
                    puts += 1;
                    true
                },
                |_| {},
            )
            .await;
            assert_eq!(puts, 1);
            assert_eq!(stats.stopped, Some(RestoreStop::JournalCorrupt));
        });
}

#[test]
fn restore_reports_truncated_tail() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let zone = ZonePersistState::new(
                "zone".to_string(),
                dir.path().clone(),
                false,
                Duration::from_secs(1),
                8,
                wake_pair(),
                None,
            );
            zone.record_put("k1", &public_entry());
            zone.record_put("k2", &public_entry());
            zone.flush_all_sync().unwrap();
            // Cut into the middle of the second record.
            let path = zone.journal_path();
            let data = std::fs::read(&path).unwrap();
            std::fs::write(&path, &data[..data.len() - 5]).unwrap();

            let mut puts = 0;
            let stats = restore_zone(
                dir.path(),
                |_, _| {
                    puts += 1;
                    true
                },
                |_| {},
            )
            .await;
            assert_eq!(puts, 1);
            assert_eq!(stats.stopped, Some(RestoreStop::JournalTruncated));
        });
}

#[test]
fn restore_counts_skipped_entries() {
    zincio::RuntimeBuilder::new()
        .build()
        .unwrap()
        .block_on(async move {
            let dir = TempDir::new();
            let zone = ZonePersistState::new(
                "zone".to_string(),
                dir.path().clone(),
                false,
                Duration::from_secs(1),
                8,
                wake_pair(),
                None,
            );
            let mut live = HashMap::new();
            live.insert("k1".to_string(), public_entry());
            live.insert("k2".to_string(), public_entry());
            zone.register_entry_source(Box::new(move |visit| {
                for (key, entry) in &live {
                    visit(key, entry);
                }
            }));
            zone.compact().unwrap();

            let stats = restore_zone(dir.path(), |key, _entry| key != "k1", |_| {}).await;
            assert_eq!(stats.records, 2);
            assert_eq!(stats.puts, 1);
            assert_eq!(stats.skipped, 1);
            assert_eq!(stats.stopped, None);
        });
}
