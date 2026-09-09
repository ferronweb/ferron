//! Distributed lockfiles for ACME provisioning.
//!
//! When `cache` points at a shared filesystem, only one node may create an
//! ACME order for a given certificate at a time. Coordination uses lockfiles
//! in the same directory with no extension:
//! - `lock_certificate_<hash>` pairs with `certificate_<hash>`
//! - `lock_account_<hash>` pairs with `account_<hash>`
//! - `lock_hostname_<hash>` pairs with `hostname_<hash>` (on-demand)
//!
//! Mechanism (NFS-safe, no `flock` which needs `lockd`):
//! - Acquire with atomic `O_CREAT|O_EXCL` (`create_new`). Exactly one node wins.
//! - Lock content is JSON `{owner, started_at_unix, heartbeat_at_unix, ...}`.
//! - Holder rewrites `heartbeat_at_unix` every [`HEARTBEAT_SECS`].
//! - Peers treat a lock as stale when `now - heartbeat_at > LEASE_SECS` and
//!   break it (remove + retry). Crashes/deadlocks therefore self-heal within
//!   one lease instead of wedging forever.
//! - Release only removes the file when it still holds our `owner` token, so a
//!   node never deletes a peer's fresh lock (prevents stealing/deadlock).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncWriteExt;

use crate::cache::is_valid_key;

/// Lease after which a lock without heartbeat is considered stale/dead.
pub const LOCK_LEASE_SECS: u64 = 300;
/// Interval at which the holder refreshes the heartbeat.
pub const HEARTBEAT_SECS: u64 = 30;
/// How long to wait between acquire retries (plus small deterministic jitter).
pub const RETRY_DELAY_MS: u64 = 500;
/// Files younger than this with unreadable content are treated as contended
/// (a peer is mid-write), not stale.
pub const UNPARSEABLE_GRACE_SECS: u64 = 30;
/// Clock-skew tolerance: headers dated this far in the future are never stale.
pub const FUTURE_SKEW_TOLERANCE_SECS: u64 = 300;

/// Lockfile payload.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct LockHeader {
    /// Unique owner token (`host-pid-nanos`), used to avoid deleting peers' locks.
    pub owner: String,
    pub started_at_unix: u64,
    pub heartbeat_at_unix: u64,
    /// Human label for logs (e.g. domains or `contact;directory`).
    pub label: String,
    /// ACME directory URL, for debugging which provider the holder uses.
    pub directory: String,
}

/// Outcome of a non-blocking acquire attempt.
#[derive(Debug)]
pub enum TryAcquireOutcome {
    /// This node now holds the lock.
    Acquired(DistributedLockGuard),
    /// A live peer holds the lock.
    Contended { owner: String, age_secs: u64 },
    /// Acquisition failed for an I/O reason.
    Error(std::io::Error),
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Builds a unique owner token for this acquisition.
pub fn owner_token() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "ferron".to_string());
    let host: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    format!("{}-{}-{}", host, std::process::id(), now_nanos())
}

/// Returns true when `header` should be considered stale/dead at `now`.
pub fn is_stale(header: &LockHeader, now: u64) -> bool {
    // Peer clock ahead of us: never break, avoid false stale during skew.
    if header.heartbeat_at_unix > now.saturating_add(FUTURE_SKEW_TOLERANCE_SECS) {
        return false;
    }
    now.saturating_sub(header.heartbeat_at_unix) > LOCK_LEASE_SECS
}

async fn file_age_secs(path: &Path) -> Option<u64> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    let modified = meta.modified().ok()?;
    SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs())
        .ok()
}

/// RAII guard for a held distributed lock.
///
/// The heartbeat task aborts on drop. Call [`DistributedLockGuard::release`]
/// to also remove the lockfile (only if it still holds our token).
pub struct DistributedLockGuard {
    dir: PathBuf,
    key: String,
    owner: String,
    started_at: u64,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for DistributedLockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DistributedLockGuard")
            .field("dir", &self.dir)
            .field("key", &self.key)
            .field("owner", &self.owner)
            .field("started_at", &self.started_at)
            .finish_non_exhaustive()
    }
}

impl DistributedLockGuard {
    /// Lockfile key (e.g. `lock_certificate_...`).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Owner token recorded in the lockfile.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Releases the lock if it still holds our token. Always aborts the heartbeat.
    pub async fn release(mut self) {
        if let Some(handle) = self.heartbeat.take() {
            handle.abort();
        }
        let path = self.dir.join(&self.key);
        // Only delete our own lock — a peer may have taken over after a stale break.
        let ours = tokio::fs::read(&path)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<LockHeader>(&b).ok())
            .is_some_and(|h| h.owner == self.owner && h.started_at_unix == self.started_at);
        if ours {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

impl Drop for DistributedLockGuard {
    fn drop(&mut self) {
        // Stop heartbeating so a dropped guard can never overwrite a peer's
        // lock taken over after our lease expired. File removal happens in
        // `release()` (async, ownership-checked); the lease is the backstop
        // if the holder crashes without releasing.
        if let Some(handle) = self.heartbeat.take() {
            handle.abort();
        }
    }
}

async fn write_header_atomic(dir: &Path, key: &str, header: &LockHeader) {
    if let Ok(bytes) = serde_json::to_vec(header) {
        crate::cache::AcmeCache::File(dir.to_path_buf())
            .set(key, bytes)
            .await
            .ok();
    }
}

fn spawn_heartbeat(
    dir: PathBuf,
    key: String,
    owner: String,
    started_at: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(HEARTBEAT_SECS)).await;
            // Re-check ownership before every rewrite so a stale-broken former
            // holder stops instead of clobbering the new owner's lock.
            let path = dir.join(&key);
            let current: Option<LockHeader> = tokio::fs::read(&path)
                .await
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            match current {
                Some(h) if h.owner == owner && h.started_at_unix == started_at => {
                    let refreshed = LockHeader {
                        heartbeat_at_unix: now_unix(),
                        ..h
                    };
                    write_header_atomic(&dir, &key, &refreshed).await;
                }
                _ => break,
            }
        }
    })
}

/// Attempts once to acquire `lock_key` in `dir` (non-blocking).
///
/// On success returns `Acquired`. When a peer holds a fresh lock returns
/// `Contended`. Stale locks are broken (removed) and reported as `Contended`
/// with the previous owner so the caller can retry or skip this cycle; the
/// next attempt will then race fairly via `create_new`.
pub async fn try_acquire(
    dir: &Path,
    lock_key: &str,
    label: &str,
    directory: &str,
) -> TryAcquireOutcome {
    if !is_valid_key(lock_key) || !lock_key.starts_with("lock_") {
        return TryAcquireOutcome::Error(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid lock key: {lock_key:?}"),
        ));
    }
    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        return TryAcquireOutcome::Error(e);
    }
    let path = dir.join(lock_key);
    let owner = owner_token();
    let now = now_unix();

    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
    {
        Ok(mut file) => {
            let header = LockHeader {
                owner: owner.clone(),
                started_at_unix: now,
                heartbeat_at_unix: now,
                label: label.to_string(),
                directory: directory.to_string(),
            };
            let bytes = serde_json::to_vec(&header).unwrap_or_default();
            if let Err(e) = async {
                file.write_all(&bytes).await?;
                file.flush().await.unwrap_or_default();
                file.sync_all().await.unwrap_or_default();
                Ok::<(), std::io::Error>(())
            }
            .await
            {
                let _ = tokio::fs::remove_file(&path).await;
                return TryAcquireOutcome::Error(e);
            }
            drop(file);
            let heartbeat =
                spawn_heartbeat(dir.to_path_buf(), lock_key.to_string(), owner.clone(), now);
            TryAcquireOutcome::Acquired(DistributedLockGuard {
                dir: dir.to_path_buf(),
                key: lock_key.to_string(),
                owner,
                started_at: now,
                heartbeat: Some(heartbeat),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Someone holds (or held) the lock. Inspect it.
            let existing = tokio::fs::read(&path).await.ok();
            let parsed: Option<LockHeader> = existing
                .as_deref()
                .and_then(|b| serde_json::from_slice(b).ok());
            match parsed {
                Some(header) => {
                    if is_stale(&header, now_unix()) {
                        // Break the dead lock; caller retries and races fairly.
                        let _ = tokio::fs::remove_file(&path).await;
                        let age = now_unix().saturating_sub(header.heartbeat_at_unix);
                        TryAcquireOutcome::Contended {
                            owner: format!("stale-broken:{}", header.owner),
                            age_secs: age,
                        }
                    } else {
                        let age = now_unix().saturating_sub(header.heartbeat_at_unix);
                        TryAcquireOutcome::Contended {
                            owner: header.owner,
                            age_secs: age,
                        }
                    }
                }
                None => {
                    // Unparseable (peer mid-write or crashed mid-write).
                    let age = file_age_secs(&path).await.unwrap_or(0);
                    if age > UNPARSEABLE_GRACE_SECS {
                        let _ = tokio::fs::remove_file(&path).await;
                        TryAcquireOutcome::Contended {
                            owner: "stale-broken:unparseable".to_string(),
                            age_secs: age,
                        }
                    } else {
                        TryAcquireOutcome::Contended {
                            owner: "unknown (mid-write)".to_string(),
                            age_secs: age,
                        }
                    }
                }
            }
        }
        Err(e) => TryAcquireOutcome::Error(e),
    }
}

/// Tries to acquire the lock until `deadline`, with backoff + jitter.
///
/// Returns `Some(guard)` on success, `None` on timeout (peer holds a live
/// lock). I/O errors other than contention abort early with the error.
/// The `stale_broken` flag reports whether a dead lock was cleared while waiting.
pub async fn acquire_with_timeout(
    dir: &Path,
    lock_key: &str,
    label: &str,
    directory: &str,
    deadline: std::time::Duration,
) -> Result<(Option<DistributedLockGuard>, bool), std::io::Error> {
    let start = std::time::Instant::now();
    let mut stale_broken = false;
    let mut attempt: u64 = 0;
    loop {
        match try_acquire(dir, lock_key, label, directory).await {
            TryAcquireOutcome::Acquired(guard) => return Ok((Some(guard), stale_broken)),
            TryAcquireOutcome::Error(e) => return Err(e),
            TryAcquireOutcome::Contended { owner, .. } => {
                if owner.starts_with("stale-broken:") {
                    stale_broken = true;
                    // Retry immediately: the path is now free and create_new decides fairly.
                } else if start.elapsed() >= deadline {
                    return Ok((None, stale_broken));
                } else {
                    attempt += 1;
                    // Deterministic jitter from time+pid so restarted peers desynchronize
                    // without needing a `rand` dependency (thundering-herd damping).
                    let jitter = (now_nanos() % 250) as u64 + (std::process::id() as u64 % 100);
                    let backoff = RETRY_DELAY_MS
                        .saturating_mul(attempt.min(4))
                        .saturating_add(jitter)
                        .min(2000);
                    // Don't oversleep past the deadline.
                    let remaining = deadline.saturating_sub(start.elapsed());
                    let sleep_ms =
                        backoff.min(remaining.as_millis().min(u128::from(u64::MAX)) as u64);
                    if sleep_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }
        if start.elapsed() >= deadline {
            return Ok((None, stale_broken));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn unique_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ferron-acme-lock-test-{tag}-{}-{}",
            std::process::id(),
            now_nanos()
        ))
    }

    #[test]
    fn test_is_stale_logic() {
        let fresh = LockHeader {
            owner: "a".into(),
            started_at_unix: 1000,
            heartbeat_at_unix: 1000,
            label: String::new(),
            directory: String::new(),
        };
        assert!(!is_stale(&fresh, 1000 + LOCK_LEASE_SECS));
        assert!(is_stale(&fresh, 1000 + LOCK_LEASE_SECS + 1));
        // Far-future heartbeat (clock skew) is never stale.
        let future = LockHeader {
            heartbeat_at_unix: 10_000,
            ..fresh.clone()
        };
        assert!(!is_stale(&future, 100));
    }

    #[tokio::test]
    async fn test_acquire_contend_release_cycle() {
        let dir = unique_dir("cycle");
        let key = "lock_certificate_test1";
        let (guard, _) =
            acquire_with_timeout(&dir, key, "example.com", "dir", Duration::from_secs(5))
                .await
                .unwrap();
        assert!(guard.is_some());
        // Second acquire while held → contended (timeout → None).
        let (second, _) =
            acquire_with_timeout(&dir, key, "example.com", "dir", Duration::from_millis(600))
                .await
                .unwrap();
        assert!(second.is_none());
        // Release frees the path for the next acquirer.
        guard.unwrap().release().await;
        let (third, _) =
            acquire_with_timeout(&dir, key, "example.com", "dir", Duration::from_secs(5))
                .await
                .unwrap();
        assert!(third.is_some());
        third.unwrap().release().await;
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_stale_lock_is_broken() {
        let dir = unique_dir("stale");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let key = "lock_certificate_stale1";
        let dead = LockHeader {
            owner: "dead-peer".into(),
            started_at_unix: 1,
            heartbeat_at_unix: 1, // ancient → stale
            label: "example.com".into(),
            directory: "dir".into(),
        };
        tokio::fs::write(dir.join(key), serde_json::to_vec(&dead).unwrap())
            .await
            .unwrap();
        let (guard, stale_broken) =
            acquire_with_timeout(&dir, key, "example.com", "dir", Duration::from_secs(5))
                .await
                .unwrap();
        assert!(guard.is_some());
        assert!(stale_broken);
        guard.unwrap().release().await;
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_release_never_deletes_peer_lock() {
        let dir = unique_dir("peer");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let key = "lock_certificate_peer1";
        let (guard, _) =
            acquire_with_timeout(&dir, key, "example.com", "dir", Duration::from_secs(5))
                .await
                .unwrap();
        let guard = guard.unwrap();
        // Simulate a peer taking over (e.g. our lease expired): overwrite file.
        let peer = LockHeader {
            owner: "peer-owner".into(),
            started_at_unix: now_unix(),
            heartbeat_at_unix: now_unix(),
            label: "example.com".into(),
            directory: "dir".into(),
        };
        tokio::fs::write(dir.join(key), serde_json::to_vec(&peer).unwrap())
            .await
            .unwrap();
        guard.release().await;
        // Peer's lock must survive our release.
        assert!(tokio::fs::try_exists(dir.join(key)).await.unwrap());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_invalid_lock_key_rejected() {
        let dir = unique_dir("invalid");
        let err = acquire_with_timeout(&dir, "../evil", "x", "d", Duration::from_millis(100))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
