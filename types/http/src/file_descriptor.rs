use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Seek};
use std::ops::Deref;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
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
    fn evict_if_full(&mut self) {
        let total = self.total_handles();
        if total < FD_CACHE_MAX_ENTRIES_PREEMPTIVE {
            return;
        }

        let mut expired_paths: Vec<PathBuf> = Vec::new();
        for (path, item) in &mut self.entries {
            item.handles
                .retain(|h| h.pooled_at.elapsed() < FD_CACHE_TTL);
            if item.handles.is_empty() {
                expired_paths.push(path.clone());
            }
        }
        for path in expired_paths {
            self.entries.remove(&path);
        }
    }
}

impl Default for FdPool {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static FD_REUSE_CACHE: RefCell<FdPool> = RefCell::new(FdPool::new());
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
                }
            }
            if let Some(item) = cache.entries.get(&path_key) {
                if item.handles.is_empty() {
                    cache.entries.remove(&path_key);
                }
            }
            result
        });

        if let Some(pooled) = pooled {
            let file = pooled.file;
            let metadata = file.metadata().await;
            return Ok(Self {
                inner: Some(file),
                metadata,
                path: path.as_ref().to_path_buf(),
            });
        }

        // Pool miss — open fresh
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
            cache.evict_if_full();
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
            //
            // `vibeio` doesn't currently expose `rewind` for `vibeio::fs::File`,
            // but we can work around that by borrowing an fd, wrapping it in a
            // `std::fs::File`, and then rewinding that, and discarding the file
            // without closing the underlying fd.
            #[cfg(unix)]
            {
                let fd = inner.as_raw_fd();
                let mut std_inner = unsafe { std::fs::File::from_raw_fd(fd) };
                let _ = std_inner.rewind();
                let _ = std_inner.into_raw_fd();
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
                let handle = inner.as_raw_handle();
                let mut std_inner = unsafe { std::fs::File::from_raw_handle(handle) };
                let _ = std_inner.rewind();
                let _ = std_inner.into_raw_handle();
            }

            let path_buf = self.path.clone();
            Self::return_handle_to_pool(inner, path_buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_eviction_removes_expired() {
        let mut pool = FdPool::new();

        let dir = std::env::temp_dir().join("ferron-pool-evict-test");
        std::fs::create_dir_all(&dir).unwrap();

        // Fill pool to capacity with non-expired handles
        for i in 0..FD_CACHE_MAX_ENTRIES_PREEMPTIVE {
            let file_path = dir.join(format!("file_{i}.txt"));
            std::fs::write(&file_path, format!("content {i}")).unwrap();
            let file = std::fs::File::open(&file_path).unwrap();
            // `vibeio::fs::File`, but it isn't inside a `vibeio` runtime?
            // How is that possible!? Why does this test somehow pass?
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
        pool.evict_if_full();
        assert!(pool.total_handles() <= FD_CACHE_MAX_ENTRIES_PREEMPTIVE);

        // The expired handle should be removed
        assert!(
            !pool.entries.contains_key(&expired_path)
                || pool.entries.get(&expired_path).unwrap().handles.is_empty()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
