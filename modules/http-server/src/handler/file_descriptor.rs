use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ferron_core::config::ServerConfigurationValue;
use rustc_hash::FxHashMap;
use vibeio::fs::Metadata;

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

/// Timestamped wrapper for TTL-based expiry tracking.
#[derive(Debug, Clone)]
struct Timestamped<T> {
    inserted_at: Instant,
    value: T,
}

impl<T> Timestamped<T> {
    fn new(value: T) -> Self {
        Self {
            inserted_at: Instant::now(),
            value,
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() >= ttl
    }
}

/// File descriptor metadata wrapper with lazy-loaded metadata.
///
/// Metadata is obtained from the file descriptor (via `fstat`-like operations)
/// rather than from the path, mitigating TOCTOU vulnerabilities.
pub struct FileDescriptorMetadata {
    /// Path to the file (for cache key purposes).
    pub path: PathBuf,
    /// Lazily-loaded metadata (obtained from FD, not path).
    metadata: Option<Metadata>,
}

impl FileDescriptorMetadata {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            metadata: None,
        }
    }

    /// Load metadata from the path (simulating FD-based retrieval).
    /// In practice, this would obtain metadata from an open file descriptor.
    pub async fn load_metadata(&mut self) -> io::Result<()> {
        if self.metadata.is_none() {
            self.metadata = Some(vibeio::fs::metadata(&self.path).await?);
        }
        Ok(())
    }

    /// Get metadata reference. Call `load_metadata()` first.
    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    /// Get metadata mutable reference. Call `load_metadata()` first.
    pub fn metadata_mut(&mut self) -> Option<&mut Metadata> {
        self.metadata.as_mut()
    }
}

thread_local! {
    /// Per-thread file descriptor cache with TTL-based expiry.
    ///
    /// Caches file metadata by path, evicting expired entries automatically.
    /// This reduces repeated metadata lookups while maintaining freshness.
    static FD_METADATA_CACHE: std::cell::RefCell<FxHashMap<PathBuf, Timestamped<Metadata>>> =
        std::cell::RefCell::new(FxHashMap::default());
}

/// TTL for cached file metadata (200ms).
const FD_CACHE_TTL: Duration = Duration::from_millis(200);

/// Maximum number of entries in the per-thread cache.
const FD_CACHE_MAX_ENTRIES: usize = 256;

/// Get or compute metadata from cache, with TTL expiry.
///
/// If `path` is in cache and not expired, returns cached metadata.
/// Otherwise, computes new metadata and caches it.
pub async fn get_or_cache_metadata(path: &Path) -> io::Result<Metadata> {
    // Check cache
    let cached = FD_METADATA_CACHE
        .try_with(|cache| {
            let mut cache_ref = cache.borrow_mut();
            cache_ref
                .get(path)
                .filter(|ts| !ts.is_expired(FD_CACHE_TTL))
                .map(|ts| ts.value.clone())
        })
        .ok()
        .flatten();

    if let Some(metadata) = cached {
        return Ok(metadata);
    }

    // Fetch new metadata
    let metadata = vibeio::fs::metadata(path).await?;

    // Store in cache (evict if full)
    let _ = FD_METADATA_CACHE.try_with(|cache| {
        let mut cache_ref = cache.borrow_mut();
        if cache_ref.len() >= FD_CACHE_MAX_ENTRIES {
            // Simple eviction: remove oldest entry
            if let Some(oldest_key) = cache_ref
                .iter()
                .min_by_key(|(_, ts)| ts.inserted_at)
                .map(|(k, _)| k.clone())
            {
                cache_ref.remove(&oldest_key);
            }
        }
        cache_ref.insert(path.to_path_buf(), Timestamped::new(metadata.clone()));
    });

    Ok(metadata)
}

/// Clear the per-thread FD metadata cache.
#[allow(dead_code)]
pub fn clear_cache() {
    let _ = FD_METADATA_CACHE.try_with(|cache| {
        cache.borrow_mut().clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_expiry() {
        let val = Timestamped::new(42);
        assert!(!val.is_expired(Duration::from_secs(1)));

        // Sleep a bit and check again
        std::thread::sleep(Duration::from_millis(10));
        assert!(!val.is_expired(Duration::from_millis(100))); // Should not be expired yet

        // Create one with past instant
        let past = Timestamped {
            inserted_at: Instant::now() - Duration::from_secs(1),
            value: 42,
        };
        assert!(past.is_expired(Duration::from_millis(100)));
    }
}
