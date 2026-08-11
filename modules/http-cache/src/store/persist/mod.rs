//! Persistent to-disk mirror of the HTTP cache.
//!
//! The in-memory store remains the only lookup path; this module keeps the
//! cache durable across process restarts by appending mutation records to a
//! per-zone journal and periodically compacting it into a full snapshot.
//!
//! Layout per cache zone inside the configured directory:
//!
//! ```text
//! <dir>/<sanitized-zone-label>/
//!     snapshot        complete image produced by compaction
//!     journal         append-only mutation log since the last compaction
//! ```

pub(crate) mod record;
pub(crate) mod writer;
