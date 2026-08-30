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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum SymlinkMode {
    /// Allow all symlinks.
    Off,
    /// Reject all symlinks encountered during traversal (default).
    #[default]
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
    file: zincio::fs::File,
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
    entries: BTreeMap<(PathBuf, SymlinkMode), FdPoolItem>,
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

        let mut expired_paths: Vec<_> = Vec::new();
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
    inner: Option<zincio::fs::File>,
    symlink_mode: SymlinkMode,
    metadata: Result<zincio::fs::Metadata, std::io::Error>,
    path: PathBuf,
    dont_rewind: bool,
}

impl ReusedFile {
    /// Open a file, reusing a pooled handle if available.
    #[inline]
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_symlink_mode(path, Path::new(""), SymlinkMode::Off).await
    }

    /// Open a file, reusing a pooled handle if available, with symlink mode.
    #[inline]
    pub async fn open_with_symlink_mode(
        path: impl AsRef<Path>,
        root_path: impl AsRef<Path>,
        symlink_mode: SymlinkMode,
    ) -> io::Result<Self> {
        let cached_error = FD_REUSE_CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            let path_key = (path.as_ref().to_path_buf(), symlink_mode);
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
            let path_key = (path.as_ref().to_path_buf(), symlink_mode);
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
                symlink_mode,
                dont_rewind: false,
            });
        }

        // Pool miss...
        let file = match Self::open_with_symlink_mode_nocache(
            path.as_ref(),
            root_path.as_ref(),
            symlink_mode,
        )
        .await
        {
            Ok(file) => file,
            Err(e) => {
                let e2 = if let Some(e) = e.raw_os_error() {
                    std::io::Error::from_raw_os_error(e)
                } else {
                    std::io::Error::new(e.kind(), e.to_string())
                };
                FD_REUSE_CACHE.with(|c| {
                    let mut cache = c.borrow_mut();
                    let path_key = (path.as_ref().to_path_buf(), symlink_mode);
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
            symlink_mode,
            dont_rewind: false,
        })
    }

    #[inline]
    async fn open_with_symlink_mode_nocache(
        path: &Path,
        root: &Path,
        mode: SymlinkMode,
    ) -> io::Result<zincio::fs::File> {
        check_symlinks_in_path(path, root, mode).await?;
        zincio::fs::File::open(path).await
    }

    #[inline]
    fn return_handle_to_pool(
        inner: zincio::fs::File,
        path_buf: PathBuf,
        symlink_mode: SymlinkMode,
    ) {
        FD_REUSE_CACHE.with(move |c| {
            let mut cache = c.borrow_mut();
            cache.evict_if_full();
            cache
                .entries
                .entry((path_buf, symlink_mode))
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
    pub fn metadata(&self) -> io::Result<zincio::fs::Metadata> {
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

    /// Get the symlink mode for this reused file.
    #[inline]
    pub fn symlink_mode(&self) -> SymlinkMode {
        self.symlink_mode
    }

    /// Marks the file as not needing to be rewound after reading.
    ///
    /// # Safety
    ///
    /// The user needs to make sure that the file operations don't move the file pointer.
    /// For example, pread will not move the file pointer, but read will move it.
    #[inline]
    pub unsafe fn dont_rewind(&mut self) {
        self.dont_rewind = true;
    }
}

impl Deref for ReusedFile {
    type Target = zincio::fs::File;

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
            if !self.dont_rewind {
                // Rewind the file cursor to the beginning so the next user
                // of this pooled handle starts at offset 0.
                //
                // `zincio` doesn't currently expose `rewind` for `zincio::fs::File`,
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
            }

            let path_buf = self.path.clone();
            let symlink_mode = self.symlink_mode;
            Self::return_handle_to_pool(inner, path_buf, symlink_mode);
        }
    }
}

/// Check for symlinks in the path traversal chain (if enabled).
/// Returns an error if a forbidden symlink is found.
async fn check_symlinks_in_path(path: &Path, root: &Path, mode: SymlinkMode) -> io::Result<()> {
    if mode == SymlinkMode::Off {
        return Ok(());
    }

    // Walk the path components and check each for symlinks
    let mut current = root.to_path_buf();

    for component in path
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(""))
        .components()
    {
        current.push(component);

        // Check if this component is a symlink
        match zincio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.is_symlink() => {
                match mode {
                    SymlinkMode::On => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "symlinks not allowed",
                        ));
                    }
                    SymlinkMode::IfNotOwner => {
                        // The UID check is Unix-specific, skip it on other platforms
                        // Based on NGINX disable_symlinks if_not_owner (UID comparison basically)
                        #[cfg(unix)]
                        {
                            let mut same_owner = false;
                            if let Ok(canonical_metadata) = zincio::fs::metadata(&current).await {
                                same_owner = metadata.uid() == canonical_metadata.uid();
                            }
                            if !same_owner {
                                return Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "different-owner symlinks not allowed",
                                ));
                            }
                        }
                    }
                    SymlinkMode::Off => {} // Already checked above
                }
            }
            _ => {} // Not a symlink or doesn't exist, continue
        }
    }

    Ok(())
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
            // `zincio::fs::File`, but it isn't inside a `zincio` runtime?
            // How is that possible!? Why does this test somehow pass?
            let std_file = zincio::fs::File::from_std(file).unwrap();
            pool.entries
                .entry((file_path, SymlinkMode::default()))
                .or_default()
                .handles
                .push(PooledHandle {
                    file: std_file,
                    pooled_at: Instant::now(),
                });
        }

        // Now add an expired handle; eviction should remove it
        let expired_path = dir.join("expired.txt");
        std::fs::write(&expired_path, b"expired").unwrap();
        let file = std::fs::File::open(&expired_path).unwrap();
        let std_file = zincio::fs::File::from_std(file).unwrap();
        pool.entries
            .entry((expired_path.clone(), SymlinkMode::default()))
            .or_default()
            .handles
            .push(PooledHandle {
                file: std_file,
                pooled_at: Instant::now() - Duration::from_secs(1), // expired
            });

        // Pool is now over capacity; eviction should remove the expired handle
        pool.evict_if_full();
        assert!(pool.total_handles() <= FD_CACHE_MAX_ENTRIES_PREEMPTIVE);

        // The expired handle should be removed
        assert!(
            !pool
                .entries
                .contains_key(&(expired_path.clone(), SymlinkMode::default()))
                || pool
                    .entries
                    .get(&(expired_path.clone(), SymlinkMode::default()))
                    .unwrap()
                    .handles
                    .is_empty()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
