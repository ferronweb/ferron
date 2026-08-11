//! Mutation journal writer for cache persistence.
//!
//! Request threads feed fully encoded records into a bounded per-zone queue
//! (`record_put` / `record_delete`); a single writer thread is the only
//! consumer and appends batches to the per-zone journal file. Batches go to
//! the page cache without a per-batch fsync; durability is provided by the
//! snapshot compaction (which fsyncs) and by the graceful final flush on
//! shutdown, which syncs the journal.
//!
//! Crash-safety rules:
//!
//! - Records in the queue are not durable until flushed; a crash loses them.
//! - A truncated or checksum-corrupted tail of the journal is treated as
//!   "not written": replay stops there. Corruption before a valid record is
//!   reported on load.
//! - When the queue overflows, the oldest *prefix* is dropped, never a tail.
//!   The newest record for every key therefore always survives, so replay of
//!   the remaining suffix converges to the same state as memory.
//! - A flush failure disables the zone's persistence (memory caching keeps
//!   serving) and is reported once.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::policy::CacheScope;
use crate::store::persist::record::{
    decode_next, encode_delete, encode_put, DecodeError, DecodedRecord,
};
use crate::store::types::StoredEntry;

pub const SNAPSHOT_FILE: &str = "snapshot";
pub const JOURNAL_FILE: &str = "journal";

/// Default upper bound of buffered mutations per zone before the oldest
/// prefix is dropped. The writer drains far faster than the request path
/// fills the queue; this only guards pathological bursts or a wedged disk.
pub const DEFAULT_QUEUE_CAPACITY: usize = 8192;

/// How often the writer compacts a zone's journal into a fresh snapshot.
const COMPACT_INTERVAL: Duration = Duration::from_secs(300);

/// How long the writer sleeps at most when there is nothing to do. Bounds
/// how quickly shutdown is observed and how promptly a freshly registered
/// zone is picked up when no mutation ever wakes the thread.
const WRITER_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Enumerates the live cache entries of a zone. Used by the writer thread to
/// dump a snapshot without knowing anything about the in-memory store.
pub type EntrySource = Box<dyn Fn(&mut dyn FnMut(&str, &StoredEntry)) + Send + Sync>;

/// Per-zone persistence state shared between the request path (producer)
/// and the writer thread (consumer).
pub struct ZonePersistState {
    label: String,
    dir: PathBuf,
    include_private: bool,
    persist_interval: Duration,
    compact_interval: Duration,
    queue_capacity: usize,
    queue: Mutex<VecDeque<Vec<u8>>>,
    last_flush: Mutex<Instant>,
    last_compact: Mutex<Instant>,
    entry_source: Mutex<Option<EntrySource>>,
    dropped: AtomicU64,
    active: AtomicBool,
    /// Only the first flush failure is surfaced to avoid log spam.
    warned: AtomicBool,
    last_error: Mutex<Option<String>>,
    journal: Mutex<Option<File>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
}

impl ZonePersistState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        label: String,
        dir: PathBuf,
        include_private: bool,
        persist_interval: Duration,
        queue_capacity: usize,
        wake: Arc<(Mutex<bool>, Condvar)>,
    ) -> Self {
        Self::with_compact_interval(
            label,
            dir,
            include_private,
            persist_interval,
            COMPACT_INTERVAL,
            queue_capacity,
            wake,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_compact_interval(
        label: String,
        dir: PathBuf,
        include_private: bool,
        persist_interval: Duration,
        compact_interval: Duration,
        queue_capacity: usize,
        wake: Arc<(Mutex<bool>, Condvar)>,
    ) -> Self {
        // First flush is due immediately so the journal is written promptly
        // after the first mutation instead of after a full interval.
        let last_flush = Instant::now() - persist_interval;
        Self {
            label,
            dir,
            include_private,
            persist_interval,
            compact_interval,
            queue_capacity: queue_capacity.max(1),
            queue: Mutex::new(VecDeque::new()),
            last_flush: Mutex::new(last_flush),
            last_compact: Mutex::new(Instant::now() - compact_interval),
            entry_source: Mutex::new(None),
            dropped: AtomicU64::new(0),
            active: AtomicBool::new(true),
            warned: AtomicBool::new(false),
            last_error: Mutex::new(None),
            journal: Mutex::new(None),
            wake,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn journal_path(&self) -> PathBuf {
        self.dir.join(JOURNAL_FILE)
    }

    /// Whether the zone still accepts records for persistence.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Records dropped by queue overflow (oldest prefix).
    pub fn dropped_records(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// First flush error, if any.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }

    /// Queue a `Put` record. Private entries are skipped unless
    /// `include_private` was set at zone creation.
    pub fn record_put(&self, key: &str, entry: &StoredEntry) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        if entry.scope == CacheScope::Private && !self.include_private {
            return;
        }
        self.push(encode_put(key, entry));
        self.wake();
    }

    /// Queue a `Delete` tombstone for a cache key. Never filtered: a purge or
    /// eviction must survive restart even for entries that were never written.
    pub fn record_delete(&self, key: &str) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        self.push(encode_delete(key));
        self.wake();
    }

    fn push(&self, record: Vec<u8>) {
        let mut queue = self.queue.lock().unwrap();
        let capacity = self.queue_capacity;
        if queue.len() >= capacity {
            // Drop the oldest prefix, never a tail: the newest record for
            // every key stays in the suffix, so replay converges.
            let drop = (queue.len().saturating_sub(capacity / 2)).max(1);
            for _ in 0..drop {
                queue.pop_front();
            }
            self.dropped.fetch_add(drop as u64, Ordering::Relaxed);
        }
        queue.push_back(record);
    }

    fn wake(&self) {
        let (lock, condvar) = &*self.wake;
        *lock.lock().unwrap() = true;
        condvar.notify_one();
    }

    /// Whether the interval since the last flush has elapsed.
    pub fn flush_due(&self) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.last_flush.lock().unwrap().elapsed() >= self.persist_interval
    }

    /// Drain the queue and append the batch to the journal. Returns early
    /// when nothing is queued or the zone is inactive. Writer thread only.
    pub(crate) fn flush(&self) -> io::Result<()> {
        if !self.active.load(Ordering::Relaxed) {
            return Ok(());
        }
        let records: Vec<Vec<u8>> = self.queue.lock().unwrap().drain(..).collect();
        if records.is_empty() {
            return Ok(());
        }
        let mut buf = Vec::new();
        for record in &records {
            buf.extend_from_slice(record);
        }

        let mut slot = self.journal.lock().unwrap();
        let file = match slot.as_mut() {
            Some(file) => file,
            None => {
                fs::create_dir_all(&self.dir)?;
                let file = open_append(&self.journal_path())?;
                slot.insert(file)
            }
        };
        file.write_all(&buf)?;
        drop(slot);
        *self.last_flush.lock().unwrap() = Instant::now();
        Ok(())
    }

    /// Flush the queue and sync the journal to disk. Used by the graceful
    /// shutdown path and by tests.
    pub fn flush_all_sync(&self) -> io::Result<()> {
        self.flush()?;
        if let Some(file) = self.journal.lock().unwrap().as_ref() {
            file.sync_all()?;
        }
        Ok(())
    }

    /// Point the compaction source at the live cache entries. The store
    /// calls this once at zone creation; compaction is a no-op without it.
    pub fn register_entry_source(&self, source: EntrySource) {
        *self.entry_source.lock().unwrap() = Some(source);
    }

    /// Whether the interval since the last compaction has elapsed.
    pub fn compact_due(&self) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.last_compact.lock().unwrap().elapsed() >= self.compact_interval
    }

    /// Compact when due: flush the queue, dump a fresh snapshot of the live
    /// entries, then truncate the journal. Returns whether a compaction ran.
    ///
    /// Crash-safety: the snapshot is written to a temp file, fsynced, and
    /// renamed over the old snapshot before the journal is truncated. A crash
    /// at any point replays to the same state, because the old journal still
    /// covers the pre-compaction records and the snapshot is idempotent with
    /// them (re-putting the same keys converges).
    pub(crate) fn maybe_compact(&self) -> io::Result<bool> {
        if !self.compact_due() {
            return Ok(false);
        }
        self.compact()?;
        Ok(true)
    }

    /// Write a full snapshot of the live entries and truncate the journal.
    /// Writer thread only.
    pub fn compact(&self) -> io::Result<CompactionStats> {
        self.flush()?;
        // Take the source out so it can run without a lock held; only the
        // writer thread reads or replaces it.
        let source = self.entry_source.lock().unwrap().take();
        let Some(source) = source else {
            return Ok(CompactionStats::default());
        };

        let result = self.compact_with(&source);
        *self.entry_source.lock().unwrap() = Some(source);
        result
    }

    fn compact_with(&self, source: &EntrySource) -> io::Result<CompactionStats> {
        fs::create_dir_all(&self.dir)?;
        let tmp_path = self.dir.join("snapshot.tmp");
        let mut entries = 0u64;
        {
            let mut file = File::create(&tmp_path)?;
            let mut result = Ok(());
            source(&mut |key, entry| {
                let record = encode_put(key, entry);
                if let Err(error) = file.write_all(&record) {
                    result = Err(error);
                }
                entries += 1;
            });
            result?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, self.dir.join(SNAPSHOT_FILE))?;
        sync_dir(&self.dir)?;

        // Truncate the journal in place, keeping the open handle for appends.
        let mut slot = self.journal.lock().unwrap();
        let file = open_truncate(&self.journal_path())?;
        *slot = Some(file);
        *self.last_compact.lock().unwrap() = Instant::now();
        Ok(CompactionStats { entries })
    }

    fn on_flush_error(&self, error: io::Error) {
        self.active.store(false, Ordering::Relaxed);
        if !self.warned.swap(true, Ordering::Relaxed) {
            *self.last_error.lock().unwrap() = Some(format!("{error}"));
        }
    }
}

/// How replay of a zone's on-disk state ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreStop {
    /// The snapshot file could not be read.
    SnapshotIo,
    /// The snapshot contains a corrupted record; replay stopped there.
    SnapshotCorrupt,
    /// The snapshot ends with a truncated record (normal crash artifact).
    SnapshotTruncated,
    /// The journal file could not be read.
    JournalIo,
    /// The journal contains a corrupted record; replay stopped there.
    JournalCorrupt,
    /// The journal ends with a truncated record (normal crash artifact).
    JournalTruncated,
}

/// Outcome of a zone restore.
#[derive(Debug, Default, Clone, Copy)]
pub struct RestoreStats {
    /// Records read from snapshot and journal combined.
    pub records: u64,
    /// Entries accepted into the store.
    pub puts: u64,
    /// Entries skipped by the store (e.g. already expired).
    pub skipped: u64,
    /// Delete tombstones replayed.
    pub deletes: u64,
    /// Where replay stopped, if not a clean end of both files.
    pub stopped: Option<RestoreStop>,
}

/// Replay a zone's snapshot and then its journal into the store.
///
/// `on_put` returns whether the entry was accepted (e.g. not expired); the
/// return value is counted in [`RestoreStats::skipped`]. A truncated tail is
/// reported through [`RestoreStop`] but is not an error: it is the expected
/// artifact of a crash between a flush and a sync.
pub fn restore_zone(
    dir: &Path,
    mut on_put: impl FnMut(String, StoredEntry) -> bool,
    mut on_delete: impl FnMut(String),
) -> RestoreStats {
    fn replay_file(
        path: &Path,
        on_put: &mut impl FnMut(String, StoredEntry) -> bool,
        on_delete: &mut impl FnMut(String),
        trunc_stop: RestoreStop,
        corrupt_stop: RestoreStop,
    ) -> Result<(u64, u64, u64, Option<RestoreStop>), io::Error> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((0, 0, 0, None)),
            Err(error) => return Err(error),
        };
        let mut records = 0u64;
        let mut puts = 0u64;
        let mut deletes = 0u64;
        let mut stop = None;
        let mut pos = 0;
        loop {
            match decode_next(&data, pos) {
                Ok(Some((record, next))) => {
                    match record {
                        DecodedRecord::Put { key, entry } => {
                            records += 1;
                            if on_put(key, *entry) {
                                puts += 1;
                            }
                        }
                        DecodedRecord::Delete { key } => {
                            records += 1;
                            deletes += 1;
                            on_delete(key);
                        }
                    }
                    pos = next;
                }
                Ok(None) => break,
                Err(DecodeError::Eof) => {
                    stop = Some(trunc_stop);
                    break;
                }
                Err(_) => {
                    stop = Some(corrupt_stop);
                    break;
                }
            }
        }
        Ok((records, puts, deletes, stop))
    }

    let mut stats = RestoreStats::default();
    match replay_file(
        &dir.join(SNAPSHOT_FILE),
        &mut on_put,
        &mut on_delete,
        RestoreStop::SnapshotTruncated,
        RestoreStop::SnapshotCorrupt,
    ) {
        Ok((records, puts, deletes, stop)) => {
            stats.records += records;
            stats.puts += puts;
            stats.deletes += deletes;
            stats.stopped = stop;
        }
        Err(_) => stats.stopped = Some(RestoreStop::SnapshotIo),
    }
    match replay_file(
        &dir.join(JOURNAL_FILE),
        &mut on_put,
        &mut on_delete,
        RestoreStop::JournalTruncated,
        RestoreStop::JournalCorrupt,
    ) {
        Ok((records, puts, deletes, stop)) => {
            stats.records += records;
            stats.puts += puts;
            stats.deletes += deletes;
            // Journal replay runs after the snapshot; a snapshot stop is
            // superseded by whatever the journal reported.
            if stop.is_some() {
                stats.stopped = stop;
            }
        }
        Err(_) => stats.stopped = Some(RestoreStop::JournalIo),
    }
    stats.skipped = stats.records - stats.puts - stats.deletes;
    stats
}

/// Result of a snapshot compaction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompactionStats {
    /// Entries written into the snapshot.
    pub entries: u64,
}

/// Replace characters that are unsafe in file names with `_` so the zone
/// label can be used as a directory name.
pub fn sanitize_zone_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Open (creating if needed) the journal in append mode with owner-only
/// permissions on Unix.
fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Open (creating or truncating) the journal for a fresh compaction cycle.
fn open_truncate(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Fsync a directory so a rename within it is durable.
fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

/// Registry of per-zone persistence states plus the writer thread. One
/// instance lives for the whole process (the module loader), surviving
/// config reloads.
pub struct PersistManager {
    zones: Mutex<HashMap<String, Arc<ZonePersistState>>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: OnceLock<JoinHandle<()>>,
}

impl PersistManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            zones: Mutex::new(HashMap::new()),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: OnceLock::new(),
        })
    }

    /// Get or create the persistence state for `label`. Idempotent: the same
    /// label always maps to the same state for the process lifetime.
    pub fn register_zone(
        &self,
        label: String,
        dir: PathBuf,
        include_private: bool,
        persist_interval: Duration,
    ) -> Arc<ZonePersistState> {
        let mut zones = self.zones.lock().unwrap();
        if let Some(existing) = zones.get(&label) {
            return existing.clone();
        }
        let state = Arc::new(ZonePersistState::new(
            label.clone(),
            dir,
            include_private,
            persist_interval,
            DEFAULT_QUEUE_CAPACITY,
            self.wake.clone(),
        ));
        zones.insert(label, state.clone());
        let (lock, condvar) = &*self.wake;
        *lock.lock().unwrap() = true;
        condvar.notify_one();
        state
    }

    /// Remove a zone's persistence state, e.g. when a zone is dropped.
    pub fn remove_zone(&self, label: &str) {
        self.zones.lock().unwrap().remove(label);
    }

    /// Idempotently start the writer thread. The module start hook runs on
    /// every config reload, so this must be safe to call repeatedly.
    pub fn start(self: &Arc<Self>) {
        let _ = self.thread.get_or_init(|| {
            let manager = Arc::clone(self);
            thread::Builder::new()
                .name("cache-persist".to_string())
                .spawn(move || writer_main(manager))
                .expect("failed to spawn cache persistence writer thread")
        });
    }

    /// Ask the writer thread to exit after a final flush. Used by tests; the
    /// production path observes the process shutdown token instead.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let (lock, condvar) = &*self.wake;
        *lock.lock().unwrap() = true;
        condvar.notify_one();
    }

    /// Flush every zone and sync its journal. Blocks until done.
    pub fn flush_all(&self) {
        let zones: Vec<Arc<ZonePersistState>> =
            self.zones.lock().unwrap().values().cloned().collect();
        for zone in zones {
            if let Err(error) = zone.flush_all_sync() {
                zone.on_flush_error(error);
            }
        }
    }

    fn drain_due(&self) {
        let zones: Vec<Arc<ZonePersistState>> =
            self.zones.lock().unwrap().values().cloned().collect();
        for zone in zones {
            if zone.flush_due() {
                if let Err(error) = zone.flush() {
                    zone.on_flush_error(error);
                }
            }
        }
    }

    fn maybe_compact_all(&self) {
        let zones: Vec<Arc<ZonePersistState>> =
            self.zones.lock().unwrap().values().cloned().collect();
        for zone in zones {
            match zone.maybe_compact() {
                Ok(true) => {
                    ferron_core::log_debug!(
                        "cache persistence: compacted zone `{}` snapshot",
                        zone.label()
                    );
                }
                Ok(false) => {}
                Err(error) => {
                    // Compaction is a durability optimization, not the source
                    // of truth: the journal keeps working, so warn and retry
                    // on the next cycle instead of disabling the zone.
                    ferron_core::log_warn!(
                        "cache persistence: snapshot compaction failed for zone `{}`: {error}",
                        zone.label()
                    );
                }
            }
        }
    }
}

fn writer_main(manager: Arc<PersistManager>) {
    loop {
        if manager.stop.load(Ordering::Relaxed)
            || ferron_core::shutdown::SHUTDOWN_TOKEN.load().is_cancelled()
        {
            manager.flush_all();
            return;
        }
        manager.drain_due();
        manager.maybe_compact_all();
        let (lock, condvar) = &*manager.wake;
        let guard = lock.lock().unwrap();
        let _ = condvar.wait_timeout(guard, WRITER_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::OpenOptions;
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use bytes::Bytes;
    use http::header::CACHE_CONTROL;
    use http::HeaderMap;
    use http::HeaderValue;

    use super::{
        restore_zone, sanitize_zone_label, PersistManager, RestoreStop, ZonePersistState,
        JOURNAL_FILE, SNAPSHOT_FILE,
    };
    use crate::policy::CacheScope;
    use crate::store::persist::record::{decode_next, DecodedRecord};
    use crate::store::types::StoredEntry;

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ferron-cache-persist-{}-{}",
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
            headers,
            body: Some(Bytes::from_static(b"body")),
            lsc_cookies: Vec::new(),
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
    fn writer_thread_flushes_periodically() {
        let dir = TempDir::new();
        let manager = PersistManager::new();
        manager.start();
        let zone = manager.register_zone(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_millis(20),
        );
        zone.record_put("k1", &public_entry());

        // Poll until the writer thread has written the journal.
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
        assert!(found, "writer thread did not flush the journal");
        manager.stop();
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
        let dir = TempDir::new();
        let zone = ZonePersistState::new(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_secs(1),
            8,
            wake_pair(),
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
        );
        assert_eq!(stats.records, 4);
        assert_eq!(stats.puts, 3);
        assert_eq!(stats.deletes, 1);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.stopped, None);
        puts.sort();
        deletes.sort();
        assert_eq!(puts, vec!["k1", "k2", "k3"]);
        assert_eq!(deletes, vec!["k1"]);
    }

    #[test]
    fn restore_absent_files_is_clean() {
        let dir = TempDir::new();
        let stats = restore_zone(dir.path(), |_, _| true, |_| {});
        assert_eq!(stats.records, 0);
        assert_eq!(stats.puts, 0);
        assert_eq!(stats.stopped, None);
    }

    #[test]
    fn restore_reports_corrupt_journal() {
        let dir = TempDir::new();
        let zone = ZonePersistState::new(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_secs(1),
            8,
            wake_pair(),
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
        );
        assert_eq!(puts, 1);
        assert_eq!(stats.stopped, Some(RestoreStop::JournalCorrupt));
    }

    #[test]
    fn restore_reports_truncated_tail() {
        let dir = TempDir::new();
        let zone = ZonePersistState::new(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_secs(1),
            8,
            wake_pair(),
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
        );
        assert_eq!(puts, 1);
        assert_eq!(stats.stopped, Some(RestoreStop::JournalTruncated));
    }

    #[test]
    fn restore_counts_skipped_entries() {
        let dir = TempDir::new();
        let zone = ZonePersistState::new(
            "zone".to_string(),
            dir.path().clone(),
            false,
            Duration::from_secs(1),
            8,
            wake_pair(),
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

        let stats = restore_zone(dir.path(), |key, _entry| key != "k1", |_| {});
        assert_eq!(stats.records, 2);
        assert_eq!(stats.puts, 1);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.stopped, None);
    }
}
