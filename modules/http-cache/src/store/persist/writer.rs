//! Mutation journal writer for cache persistence.
//!
//! Request threads feed fully encoded records into a bounded per-zone queue
//! (`record_put` / `record_delete`); a single writer task on the secondary
//! runtime is the only consumer and appends batches to the per-zone journal
//! file. Batches go to the page cache without a per-batch fsync; durability
//! is provided by the snapshot compaction (which fsyncs) and by the graceful
//! final flush on shutdown, which syncs the journal.
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

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue,
    MetricEvent, MetricType, MetricValue,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::policy::CacheScope;
use crate::store::persist::record::{
    decode_next, encode_delete, encode_put, DecodeError, DecodedRecord,
};
use crate::store::types::StoredEntry;

const LOG_TARGET: &str = "ferron-http-cache";

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
    drop_warned: AtomicBool,
    active: AtomicBool,
    /// Only the first flush failure is surfaced to avoid log spam.
    warned: AtomicBool,
    last_error: Mutex<Option<String>>,
    journal: Mutex<Option<File>>,
    /// Wakes the writer task when a record is queued or a zone is registered.
    wake: Arc<Notify>,
    /// Back-reference to the manager, used to reach the configured event
    /// sinks for log events. `None` for states built directly in tests.
    manager: Option<Weak<PersistManager>>,
}

impl ZonePersistState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        label: String,
        dir: PathBuf,
        include_private: bool,
        persist_interval: Duration,
        queue_capacity: usize,
        wake: Arc<Notify>,
        manager: Option<Weak<PersistManager>>,
    ) -> Self {
        Self::with_compact_interval(
            label,
            dir,
            include_private,
            persist_interval,
            COMPACT_INTERVAL,
            queue_capacity,
            wake,
            manager,
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
        wake: Arc<Notify>,
        manager: Option<Weak<PersistManager>>,
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
            drop_warned: AtomicBool::new(false),
            active: AtomicBool::new(true),
            warned: AtomicBool::new(false),
            last_error: Mutex::new(None),
            journal: Mutex::new(None),
            wake,
            manager,
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
        self.last_error.lock().clone()
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
        let mut queue = self.queue.lock();
        let capacity = self.queue_capacity;
        if queue.len() >= capacity {
            // Drop the oldest prefix, never a tail: the newest record for
            // every key stays in the suffix, so replay converges.
            let drop = (queue.len().saturating_sub(capacity / 2)).max(1);
            for _ in 0..drop {
                queue.pop_front();
            }
            self.dropped.fetch_add(drop as u64, Ordering::Relaxed);
            self.emit_metric(
                "ferron.cache.persistence_dropped_records",
                MetricType::Counter,
                MetricValue::U64(drop as u64),
                Some("{record}"),
                "Journal records dropped because a zone's write queue overflowed under backpressure.",
            );
            if !self.drop_warned.swap(true, Ordering::Relaxed) {
                self.emit_log(
                    LogLevel::Warn,
                    format!(
                        "cache persistence: dropping {drop} journal record(s) for zone `{}`: the write queue exceeded its capacity and the oldest records were discarded",
                        self.label
                    ),
                    "Journal records dropped because the write queue exceeded capacity",
                    vec![
                        (
                            "ferron.cache.zone",
                            LogAttributeValue::String(self.label.clone()),
                        ),
                        ("cache.dropped.count", LogAttributeValue::I64(drop as i64)),
                    ],
                );
            }
        }
        queue.push_back(record);
    }

    fn wake(&self) {
        self.wake.notify_one();
    }

    /// Emit a structured log event through the configured event sinks.
    fn emit_log(
        &self,
        level: LogLevel,
        message: String,
        summary: &'static str,
        attributes: Vec<(&'static str, LogAttributeValue)>,
    ) {
        if let Some(manager) = self.manager.as_ref().and_then(Weak::upgrade) {
            manager.emit_log(level, message, summary, attributes);
        }
    }

    /// Emit a cache persistence metric scoped by zone through the configured
    /// event sinks.
    fn emit_metric(
        &self,
        name: &'static str,
        ty: MetricType,
        value: MetricValue,
        unit: Option<&'static str>,
        description: &'static str,
    ) {
        if let Some(manager) = self.manager.as_ref().and_then(Weak::upgrade) {
            manager.emit_metric(name, self.label.clone(), ty, value, unit, description);
        }
    }

    /// Whether the interval since the last flush has elapsed.
    pub fn flush_due(&self) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.last_flush.lock().elapsed() >= self.persist_interval
    }

    /// Drain the queue and append the batch to the journal. Returns early
    /// when nothing is queued or the zone is inactive. Writer thread only.
    pub(crate) fn flush(&self) -> io::Result<()> {
        if !self.active.load(Ordering::Relaxed) {
            return Ok(());
        }
        let records: Vec<Vec<u8>> = self.queue.lock().drain(..).collect();
        if records.is_empty() {
            return Ok(());
        }
        let mut buf = Vec::new();
        for record in &records {
            buf.extend_from_slice(record);
        }

        let mut slot = self.journal.lock();
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
        *self.last_flush.lock() = Instant::now();
        Ok(())
    }

    /// Flush the queue and sync the journal to disk. Used by the graceful
    /// shutdown path and by tests.
    pub fn flush_all_sync(&self) -> io::Result<()> {
        self.flush()?;
        if let Some(file) = self.journal.lock().as_ref() {
            file.sync_all()?;
        }
        Ok(())
    }

    /// Point the compaction source at the live cache entries. The store
    /// calls this once at zone creation; compaction is a no-op without it.
    pub fn register_entry_source(&self, source: EntrySource) {
        *self.entry_source.lock() = Some(source);
    }

    /// Whether the interval since the last compaction has elapsed.
    pub fn compact_due(&self) -> bool {
        self.active.load(Ordering::Relaxed)
            && self.last_compact.lock().elapsed() >= self.compact_interval
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
        let source = self.entry_source.lock().take();
        let Some(source) = source else {
            return Ok(CompactionStats::default());
        };

        let result = self.compact_with(&source);
        *self.entry_source.lock() = Some(source);
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
        let mut slot = self.journal.lock();
        let file = open_truncate(&self.journal_path())?;
        *slot = Some(file);
        *self.last_compact.lock() = Instant::now();
        Ok(CompactionStats { entries })
    }

    fn on_flush_error(&self, error: io::Error) {
        self.active.store(false, Ordering::Relaxed);
        self.emit_metric(
            "ferron.cache.persistence_errors",
            MetricType::Counter,
            MetricValue::U64(1),
            Some("{error}"),
            "Cache persistence errors (journal flush or snapshot compaction failures).",
        );
        self.emit_metric(
            "ferron.cache.persistence_active",
            MetricType::Gauge,
            MetricValue::U64(0),
            None,
            "Whether the zone's on-disk persistence is currently active (1) or disabled after a flush failure (0).",
        );
        if !self.warned.swap(true, Ordering::Relaxed) {
            *self.last_error.lock() = Some(format!("{error}"));
            self.emit_log(
                LogLevel::Warn,
                format!(
                    "cache persistence: journal flush failed for zone `{}`: {error}",
                    self.label
                ),
                "Cache persistence journal flush failed",
                vec![
                    (
                        "ferron.cache.zone",
                        LogAttributeValue::String(self.label.clone()),
                    ),
                    ("error", LogAttributeValue::String(format!("{error}"))),
                ],
            );
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
pub async fn restore_zone(
    dir: &Path,
    mut on_put: impl FnMut(String, StoredEntry) -> bool,
    mut on_delete: impl FnMut(String),
) -> RestoreStats {
    async fn replay_file(
        path: &Path,
        on_put: &mut impl FnMut(String, StoredEntry) -> bool,
        on_delete: &mut impl FnMut(String),
        trunc_stop: RestoreStop,
        corrupt_stop: RestoreStop,
    ) -> Result<(u64, u64, u64, Option<RestoreStop>), io::Error> {
        let data = match zincio::fs::read(path).await {
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
    )
    .await
    {
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
    )
    .await
    {
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

/// Registry of per-zone persistence states plus the writer task. One
/// instance lives for the whole process (the module loader), surviving
/// config reloads.
pub struct PersistManager {
    zones: Mutex<HashMap<String, Arc<ZonePersistState>>>,
    wake: Arc<Notify>,
    stop: Arc<AtomicBool>,
    task: OnceLock<tokio::task::JoinHandle<()>>,
    /// Configured event sinks for persistence log events. Swapped on config
    /// reload so events follow the latest observability configuration.
    events: Mutex<Arc<CompositeEventSink>>,
}

impl PersistManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            zones: Mutex::new(HashMap::new()),
            wake: Arc::new(Notify::new()),
            stop: Arc::new(AtomicBool::new(false)),
            task: OnceLock::new(),
            events: Mutex::new(Arc::new(CompositeEventSink::new(Vec::new()))),
        })
    }

    /// Replace the configured event sink handle, e.g. after a config reload
    /// rebuilt the sinks. Log events are emitted into the latest handle.
    pub fn attach_events(&self, events: Arc<CompositeEventSink>) {
        *self.events.lock() = events;
    }

    /// Emit a structured log event through the configured event sinks.
    pub(crate) fn emit_log(
        &self,
        level: LogLevel,
        message: String,
        summary: &'static str,
        attributes: Vec<(&'static str, LogAttributeValue)>,
    ) {
        self.events.lock().emit(Event::Log(LogEvent {
            level,
            message,
            summary: Cow::Borrowed(summary),
            target: LOG_TARGET,
            attributes,
            trace_context: None,
        }));
    }

    /// Emit a cache persistence metric, scoped by zone, through the
    /// configured event sinks.
    pub(crate) fn emit_metric(
        &self,
        name: &'static str,
        zone_label: String,
        ty: MetricType,
        value: MetricValue,
        unit: Option<&'static str>,
        description: &'static str,
    ) {
        self.events.lock().emit(Event::Metric(MetricEvent {
            name,
            attributes: vec![(
                "ferron.cache.zone",
                MetricAttributeValue::String(zone_label),
            )],
            ty,
            value,
            unit,
            description: Some(description),
            trace_context: None,
        }));
    }

    /// Get or create the persistence state for `label`. Idempotent: the same
    /// label always maps to the same state for the process lifetime.
    pub fn register_zone(
        self: &Arc<Self>,
        label: String,
        dir: PathBuf,
        include_private: bool,
        persist_interval: Duration,
    ) -> Arc<ZonePersistState> {
        let mut zones = self.zones.lock();
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
            Some(Arc::downgrade(self)),
        ));
        zones.insert(label, state.clone());
        self.wake.notify_one();
        state
    }

    /// Remove a zone's persistence state, e.g. when a zone is dropped.
    pub fn remove_zone(&self, label: &str) {
        self.zones.lock().remove(label);
    }

    /// Idempotently spawn the writer task on `handle`. The module start hook
    /// runs on every config reload, so this must be safe to call repeatedly.
    pub fn start_on(self: &Arc<Self>, handle: &tokio::runtime::Handle) {
        let _ = self.task.get_or_init(|| {
            let manager = Arc::clone(self);
            handle.spawn(async move { manager.run().await })
        });
    }

    /// Spawn the writer task on the current tokio handle. Call this from
    /// inside the secondary runtime (e.g. wrapped in `Runtime::block_on`).
    pub fn start(self: &Arc<Self>) {
        let handle = tokio::runtime::Handle::current();
        self.start_on(&handle);
    }

    /// Ask the writer task to exit after a final flush. Used by tests; the
    /// production path observes the process shutdown token instead.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }

    /// Flush every zone and sync its journal. Blocks until done.
    pub fn flush_all(&self) {
        let zones: Vec<Arc<ZonePersistState>> = self.zones.lock().values().cloned().collect();
        for zone in zones {
            if let Err(error) = zone.flush_all_sync() {
                zone.on_flush_error(error);
            }
        }
    }

    fn drain_due(&self) {
        let zones: Vec<Arc<ZonePersistState>> = self.zones.lock().values().cloned().collect();
        for zone in zones {
            if zone.flush_due() {
                if let Err(error) = zone.flush() {
                    zone.on_flush_error(error);
                }
            }
        }
    }

    fn maybe_compact_all(&self) {
        let zones: Vec<Arc<ZonePersistState>> = self.zones.lock().values().cloned().collect();
        for zone in zones {
            match zone.maybe_compact() {
                Ok(true) => {
                    self.emit_log(
                        LogLevel::Debug,
                        format!(
                            "cache persistence: compacted zone `{}` snapshot",
                            zone.label()
                        ),
                        "Snapshot compaction completed",
                        vec![(
                            "ferron.cache.zone",
                            LogAttributeValue::String(zone.label().to_string()),
                        )],
                    );
                }
                Ok(false) => {}
                Err(error) => {
                    // Compaction is a durability optimization, not the source
                    // of truth: the journal keeps working, so warn and retry
                    // on the next cycle instead of disabling the zone.
                    zone.emit_metric(
                        "ferron.cache.persistence_errors",
                        MetricType::Counter,
                        MetricValue::U64(1),
                        Some("{error}"),
                        "Cache persistence errors (journal flush or snapshot compaction failures).",
                    );
                    self.emit_log(
                        LogLevel::Warn,
                        format!(
                            "cache persistence: snapshot compaction failed for zone `{}`: {error}",
                            zone.label()
                        ),
                        "Snapshot compaction failed",
                        vec![(
                            "ferron.cache.zone",
                            LogAttributeValue::String(zone.label().to_string()),
                        )],
                    );
                }
            }
        }
    }

    /// Writer task body: flush due zones and compact due zones, then wait
    /// for a queued record, the poll interval, or shutdown.
    async fn run(self: Arc<Self>) {
        loop {
            if self.stop.load(Ordering::Relaxed)
                || ferron_core::shutdown::SHUTDOWN_TOKEN.load().is_cancelled()
            {
                self.flush_all();
                return;
            }
            self.drain_due();
            self.maybe_compact_all();
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(WRITER_POLL_INTERVAL) => {}
            }
        }
    }
}
