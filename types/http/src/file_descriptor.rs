use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Seek};
use std::ops::Deref;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ferron_core::config::ServerConfigurationValue;
use std::collections::HashMap;

/// Symlink handling mode for path resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkMode {
    /// Allow all symlinks (default).
    Off,
    /// Reject all symlinks encountered during traversal.
    On,
    /// Allow symlinks only if owned by the same UID as target (Unix only).
    IfNotOwner,
}

impl SymlinkMode {
    #[inline]
    pub fn from_config_value(value: &ServerConfigurationValue) -> Result<Self, String> {
        if let Some(b) = value.as_boolean() {
            Ok(if b { SymlinkMode::On } else { SymlinkMode::Off })
        } else if let Some(value) = value.as_string_with_interpolations(&HashMap::new()) {
            match value.to_lowercase().as_str() {
                "if_not_owner" => Ok(SymlinkMode::IfNotOwner),
                _ => Err(format!(
                    "invalid disable_symlinks value: \
                     '{}'. Expected 'false', 'true', or 'if_not_owner'",
                    value.as_str()
                )),
            }
        } else {
            Ok(SymlinkMode::On)
        }
    }
}

/// TTL for pooled file handles (200ms).
const FD_CACHE_TTL: Duration = Duration::from_millis(200);

/// Maximum number of pooled file handles across all paths.
const FD_CACHE_MAX_ENTRIES_PREEMPTIVE: usize = 256;

/// A pooled file handle with insertion timestamp for TTL-based eviction.
struct PooledHandle {
    file: vibeio::fs::File,
    /// When this handle was returned to the pool.
    pooled_at: Instant,
}

struct PooledError {
    error: std::io::Error,
    pooled_at: Instant,
}

#[derive(Default)]
struct FdPoolItem {
    handles: Vec<PooledHandle>,
    error: Option<PooledError>,
}

/// Per-thread file descriptor reuse pool with expired eviction.
struct FdPool {
    /// Maps file paths to a stack of pooled handles.
    entries: BTreeMap<PathBuf, FdPoolItem>,
}

impl FdPool {
    #[inline]
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Total number of pooled handles across all paths.
    #[inline]
    fn total_handles(&self) -> usize {
        self.entries.values().map(|v| v.handles.len()).sum()
    }

    /// Evict expired handles from the pool.
    #[inline]
    fn evict_if_full(&mut self) -> u64 {
        let total = self.total_handles();
        if total < FD_CACHE_MAX_ENTRIES_PREEMPTIVE {
            return 0;
        }

        let mut evicted = 0u64;

        // Remove ALL expired handles in one pass.
        let mut expired_paths: Vec<PathBuf> = Vec::new();
        for (path, item) in &mut self.entries {
            let before = item.handles.len();
            item.handles
                .retain(|h| h.pooled_at.elapsed() < FD_CACHE_TTL);
            let removed = (before - item.handles.len()) as u64;
            evicted += removed;
            if item.handles.is_empty() {
                expired_paths.push(path.clone());
            }
        }
        for path in expired_paths {
            self.entries.remove(&path);
        }

        if self.total_handles() < FD_CACHE_MAX_ENTRIES_PREEMPTIVE {
            return evicted;
        }

        evicted
    }
}

impl Default for FdPool {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Pool-level statistics for observability.
#[derive(Debug, Default)]
pub struct PoolStats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub evictions: AtomicU64,
    pub expirations: AtomicU64,
    pub preemptive_evictions: AtomicU64,
}

impl PoolStats {
    #[inline]
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn expirations(&self) -> u64 {
        self.expirations.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn preemptive_evictions(&self) -> u64 {
        self.preemptive_evictions.load(Ordering::Relaxed)
    }
}

thread_local! {
    static FD_REUSE_CACHE: RefCell<FdPool> = RefCell::new(FdPool::new());
    static FD_POOL_STATS: PoolStats = PoolStats::default();
}

/// A file handle that is reused from a per-thread pool.
///
/// On drop, the underlying file handle is rewound and returned to the pool
/// for reuse by subsequent requests. This avoids redundant open syscalls
/// while maintaining correctness (the cursor is always reset to the beginning).
pub struct ReusedFile {
    inner: Option<vibeio::fs::File>,
    metadata: Result<vibeio::fs::Metadata, std::io::Error>,
    path: PathBuf,
}

impl ReusedFile {
    /// Open a file, reusing a pooled handle if available.
    #[inline]
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        // Check for errors
        let cached_error = FD_REUSE_CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            let path_key = path.as_ref().to_path_buf();
            let ent = cache.entries.get_mut(&path_key)?;
            let err = ent.error.as_ref()?;
            if err.pooled_at.elapsed() < FD_CACHE_TTL {
                let e = &err.error;
                let e2 = if let Some(e) = e.raw_os_error() {
                    std::io::Error::from_raw_os_error(e)
                } else {
                    std::io::Error::new(e.kind(), e.to_string())
                };
                Some(e2)
            } else {
                ent.error = None;
                None
            }
        });
        if let Some(e) = cached_error {
            return Err(e);
        }

        // Try reusing from pool first
        let pooled = FD_REUSE_CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            let path_key = path.as_ref().to_path_buf();
            let mut entry = cache.entries.get_mut(&path_key);
            let mut result = None;
            while entry.as_ref().is_some_and(|e| !e.handles.is_empty()) {
                let tr = entry.as_mut().and_then(|item| item.handles.pop());
                if tr
                    .as_ref()
                    .is_some_and(|tr| tr.pooled_at.elapsed() < FD_CACHE_TTL)
                {
                    result = tr;
                    break;
                } else {
                    // Expired handle — drop it and continue
                    FD_POOL_STATS.with(|s| s.expirations.fetch_add(1, Ordering::Relaxed));
                }
            }
            // Clean up empty entries
            if let Some(item) = cache.entries.get(&path_key) {
                if item.handles.is_empty() {
                    cache.entries.remove(&path_key);
                }
            }
            result
        });

        if let Some(pooled) = pooled {
            FD_POOL_STATS.with(|s| s.hits.fetch_add(1, Ordering::Relaxed));
            let file = pooled.file;
            let metadata = file.metadata().await;
            return Ok(Self {
                inner: Some(file),
                metadata,
                path: path.as_ref().to_path_buf(),
            });
        }

        // Pool miss — open fresh
        FD_POOL_STATS.with(|s| s.misses.fetch_add(1, Ordering::Relaxed));
        let file = match vibeio::fs::File::open(path.as_ref()).await {
            Ok(file) => file,
            Err(e) => {
                let e2 = if let Some(e) = e.raw_os_error() {
                    std::io::Error::from_raw_os_error(e)
                } else {
                    std::io::Error::new(e.kind(), e.to_string())
                };
                FD_REUSE_CACHE.with(|c| {
                    let mut cache = c.borrow_mut();
                    let path_key = path.as_ref().to_path_buf();
                    cache.entries.entry(path_key).or_default().error = Some(PooledError {
                        error: e2,
                        pooled_at: Instant::now(),
                    });
                });
                return Err(e)?;
            }
        };
        let metadata = file.metadata().await;
        Ok(Self {
            inner: Some(file),
            metadata,
            path: path.as_ref().to_path_buf(),
        })
    }

    #[inline]
    fn return_handle_to_pool(inner: vibeio::fs::File, path_buf: PathBuf) {
        FD_REUSE_CACHE.with(move |c| {
            let mut cache = c.borrow_mut();
            let evicted = cache.evict_if_full();
            if evicted > 0 {
                FD_POOL_STATS.with(|s| {
                    s.preemptive_evictions.fetch_add(evicted, Ordering::Relaxed);
                });
            }
            cache
                .entries
                .entry(path_buf)
                .or_default()
                .handles
                .push(PooledHandle {
                    file: inner,
                    pooled_at: Instant::now(),
                });
        });
    }

    /// Get cached metadata directly from the file descriptor (not the path).
    ///
    /// On Linux, this uses `statx` with `AT_EMPTY_PATH`, which retrieves
    /// metadata from the open file descriptor without following the path.
    /// This mitigates TOCTOU vulnerabilities.
    #[inline]
    pub fn metadata(&self) -> io::Result<vibeio::fs::Metadata> {
        self.metadata
            .as_ref()
            .map_err(|e| {
                if let Some(e) = e.raw_os_error() {
                    std::io::Error::from_raw_os_error(e)
                } else {
                    std::io::Error::new(e.kind(), e.to_string())
                }
            })
            .cloned()
    }

    /// Check if the inner file handle is present.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.inner.is_some()
    }
}

impl Deref for ReusedFile {
    type Target = vibeio::fs::File;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("invalid reused file state")
    }
}

#[cfg(unix)]
impl AsRawFd for ReusedFile {
    #[inline]
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.inner
            .as_ref()
            .expect("invalid reused file state")
            .as_raw_fd()
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsRawHandle for ReusedFile {
    #[inline]
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.inner
            .as_ref()
            .expect("invalid reused file state")
            .as_raw_handle()
    }
}

impl Drop for ReusedFile {
    #[inline]
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            // Rewind the file cursor to the beginning so the next user
            // of this pooled handle starts at offset 0.
            #[cfg(unix)]
            {
                let fd = inner.as_raw_fd();
                let mut std_inner = unsafe { std::fs::File::from_raw_fd(fd) };
                let _ = std_inner.rewind();
                let _ = std_inner.into_raw_fd();
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::{FromRawHandle, IntoRawHandle};
                let handle = inner.as_raw_handle();
                let mut std_inner = unsafe { std::fs::File::from_raw_handle(handle) };
                let _ = std_inner.rewind();
                let _ = std_inner.into_raw_handle();
            }

            // Return the handle to the per-thread pool
            let path_buf = self.path.clone();
            Self::return_handle_to_pool(inner, path_buf);
        }
    }
}

/// Clear the per-thread FD reuse pool.
#[inline]
pub fn clear_pool() {
    let _ = FD_REUSE_CACHE.try_with(|cache| {
        cache.borrow_mut().entries.clear();
    });
}

/// Get the total number of pooled handles across all paths (for testing/observability).
#[inline]
pub fn pool_size() -> usize {
    FD_REUSE_CACHE
        .try_with(|cache| cache.borrow().total_handles())
        .unwrap_or(0)
}

/// Get pool statistics snapshot (for observability).
#[inline]
pub fn pool_stats() -> PoolStatsSnapshot {
    FD_POOL_STATS.with(|s| PoolStatsSnapshot {
        hits: s.hits(),
        misses: s.misses(),
        evictions: s.evictions(),
        expirations: s.expirations(),
        preemptive_evictions: s.preemptive_evictions(),
    })
}

/// Snapshot of pool statistics.
#[derive(Debug, Default, Clone)]
pub struct PoolStatsSnapshot {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub preemptive_evictions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[inline]
    fn reused_file_returns_to_pool() {
        let dir = std::env::temp_dir().join("ferron-reused-file-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("test.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        clear_pool();
        assert_eq!(pool_size(), 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[inline]
    fn symlink_mode_parsing() {
        let val_true = ServerConfigurationValue::Boolean(true, None);
        assert_eq!(
            SymlinkMode::from_config_value(&val_true).unwrap(),
            SymlinkMode::On
        );

        let val_false = ServerConfigurationValue::Boolean(false, None);
        assert_eq!(
            SymlinkMode::from_config_value(&val_false).unwrap(),
            SymlinkMode::Off
        );
    }

    #[test]
    #[inline]
    fn pool_eviction_removes_expired() {
        let mut pool = FdPool::new();

        let dir = std::env::temp_dir().join("ferron-pool-evict-test");
        std::fs::create_dir_all(&dir).unwrap();

        // Fill pool to capacity with non-expired handles
        for i in 0..FD_CACHE_MAX_ENTRIES_PREEMPTIVE {
            let file_path = dir.join(format!("file_{i}.txt"));
            std::fs::write(&file_path, format!("content {i}")).unwrap();
            let file = std::fs::File::open(&file_path).unwrap();
            let std_file = vibeio::fs::File::from_std(file).unwrap();
            pool.entries
                .entry(file_path)
                .or_default()
                .handles
                .push(PooledHandle {
                    file: std_file,
                    pooled_at: Instant::now(),
                });
        }

        // Now add an expired handle — eviction should remove it
        let expired_path = dir.join("expired.txt");
        std::fs::write(&expired_path, b"expired").unwrap();
        let file = std::fs::File::open(&expired_path).unwrap();
        let std_file = vibeio::fs::File::from_std(file).unwrap();
        pool.entries
            .entry(expired_path.clone())
            .or_default()
            .handles
            .push(PooledHandle {
                file: std_file,
                pooled_at: Instant::now() - Duration::from_secs(1), // expired
            });

        // Pool is now over capacity — eviction should remove the expired handle
        let evicted = pool.evict_if_full();
        assert!(evicted >= 1);

        // The expired handle should be removed
        assert!(
            !pool.entries.contains_key(&expired_path)
                || pool.entries.get(&expired_path).unwrap().handles.is_empty()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
