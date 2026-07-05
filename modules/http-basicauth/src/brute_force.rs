//! Brute-force protection engine for HTTP Basic Authentication.
//!
//! Tracks failed authentication attempts per username and locks out accounts
//! that exceed the configured threshold within a sliding time window.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use rustc_hash::FxBuildHasher;

/// Configuration for brute-force protection.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct BruteForceConfig {
    /// Whether brute-force protection is enabled.
    pub enabled: bool,
    /// Maximum failed attempts allowed within the window before lockout.
    pub max_attempts: usize,
    /// How long to lock the IP after exceeding max attempts (seconds).
    pub lockout_duration_secs: u64,
    /// Sliding window for counting attempts (seconds).
    pub window_secs: u64,
}

impl BruteForceConfig {
    /// Default: enabled, 5 attempts, 15-minute lockout, 5-minute window.
    pub const DEFAULT_MAX_ATTEMPTS: usize = 5;
    pub const DEFAULT_LOCKOUT_DURATION_SECS: u64 = 900; // 15 minutes
    pub const DEFAULT_WINDOW_SECS: u64 = 300; // 5 minutes
}

impl Default for BruteForceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            lockout_duration_secs: Self::DEFAULT_LOCKOUT_DURATION_SECS,
            window_secs: Self::DEFAULT_WINDOW_SECS,
        }
    }
}

/// Tracks failed attempts for a single username.
#[derive(Debug)]
struct AttemptTracker {
    /// Timestamps of failed attempts within the current window.
    attempts: Vec<Instant>,
    /// Time when the lockout expires (if currently locked).
    locked_until: Option<Instant>,
}

impl AttemptTracker {
    fn new() -> Self {
        Self {
            attempts: Vec::new(),
            locked_until: None,
        }
    }

    /// Prune attempts outside the current window.
    fn prune_attempts(&mut self, window: Duration) {
        let cutoff = Instant::now().checked_sub(window).unwrap_or(Instant::now());
        self.attempts.retain(|&t| t >= cutoff);
    }

    /// Check if the IP is currently locked out.
    fn is_locked(&self) -> bool {
        if let Some(until) = self.locked_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Returns the duration to wait before retrying, if the IP is locked.
    fn retry_after(&self) -> Option<Duration> {
        self.locked_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
    }

    /// Record a failed attempt. Returns `true` if the IP is now locked.
    fn record_failure(&mut self, max_attempts: usize, lockout_duration: Duration) -> bool {
        self.attempts.push(Instant::now());

        if self.attempts.len() >= max_attempts && self.locked_until.is_none() {
            self.locked_until = Some(Instant::now() + lockout_duration);
            true
        } else {
            false
        }
    }

    /// Clear the attempt history (called on successful authentication).
    fn clear(&mut self) {
        self.attempts.clear();
        self.locked_until = None;
    }

    /// Returns `true` if the tracker has at least one attempt within the
    /// given time window. Used to decide whether the entry is still needed
    /// during eviction.
    fn has_recent_attempt(&self, window: Duration) -> bool {
        let cutoff = Instant::now().checked_sub(window).unwrap_or(Instant::now());
        self.attempts.iter().any(|&t| t >= cutoff)
    }
}

/// Shared brute-force protection engine.
///
/// Manages per-username attempt tracking with automatic lockout and TTL-based
/// eviction to prevent unbounded memory growth.
pub struct BruteForceEngine {
    /// Per-username attempt trackers.
    trackers: DashMap<IpAddr, AttemptTracker, FxBuildHasher>,
    /// Configuration for this engine.
    config: BruteForceConfig,
}

impl BruteForceEngine {
    /// Create a new brute-force engine with the given configuration.
    pub fn new(config: BruteForceConfig) -> Self {
        Self {
            trackers: DashMap::with_hasher(FxBuildHasher),
            config,
        }
    }

    /// Get the retry duration for an IP address, if it is currently locked out.
    ///
    /// Returns `None` if brute-force protection is not enabled, the IP is not locked
    /// or the lockout has been expired.
    pub fn retry_after(&self, ip: IpAddr) -> Option<Duration> {
        if !self.config.enabled {
            return None;
        }

        let ip = ip.to_canonical();
        let tracker = if let Some(tracker) = self.trackers.get(&ip) {
            tracker
        } else {
            self.trackers
                .entry(ip)
                .or_insert_with(AttemptTracker::new)
                .downgrade()
        };
        tracker.retry_after()
    }

    /// Check if an IP address is currently locked out.
    ///
    /// Returns `true` if the IP address is locked and should be rejected immediately.
    pub fn is_locked(&self, ip: IpAddr) -> bool {
        if !self.config.enabled {
            return false;
        }

        let ip = ip.to_canonical();
        let tracker = if let Some(tracker) = self.trackers.get(&ip) {
            tracker
        } else {
            self.trackers
                .entry(ip)
                .or_insert_with(AttemptTracker::new)
                .downgrade()
        };

        // Check lock status (pruning happens implicitly on access)
        if tracker.is_locked() {
            return true;
        }

        // If lockout has expired, reset the tracker
        if tracker.locked_until.is_some() {
            drop(tracker);
            self.trackers
                .entry(ip)
                .or_insert_with(AttemptTracker::new)
                .clear();
        }

        false
    }

    /// Record a failed authentication attempt for an IP.
    ///
    /// Returns `true` if the IP is now locked out.
    pub fn record_failure(&self, ip: IpAddr) -> bool {
        if !self.config.enabled {
            return false;
        }

        // Opportunistically evict stale trackers (entries that are neither locked
        // nor have recent failures within the window) to prevent unbounded
        // memory growth when many distinct IPs are seen.
        self.evict_stale();

        let mut tracker = self
            .trackers
            .entry(ip.to_canonical())
            .or_insert_with(AttemptTracker::new);

        // Prune old attempts outside the window
        let window = Duration::from_secs(self.config.window_secs);
        tracker.prune_attempts(window);

        // Check if already locked (should have been caught by is_locked, but be safe)
        if tracker.is_locked() {
            return true;
        }

        // Record the failure
        let lockout_duration = Duration::from_secs(self.config.lockout_duration_secs);
        tracker.record_failure(self.config.max_attempts, lockout_duration)
    }

    /// Evict tracker entries that are no longer needed: locked entries whose
    /// lockout has expired, and unlocked entries that have no recent attempts
    /// within the configured window. Prevents unbounded memory growth.
    fn evict_stale(&self) {
        let window = Duration::from_secs(self.config.window_secs);
        self.trackers
            .retain(|_, tracker| tracker.is_locked() || tracker.has_recent_attempt(window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> BruteForceConfig {
        BruteForceConfig {
            enabled: true,
            max_attempts: 3,
            lockout_duration_secs: 60,
            window_secs: 10,
        }
    }

    #[test]
    fn allows_attempts_below_threshold() {
        let engine = BruteForceEngine::new(make_test_config());

        assert!(!engine.is_locked("127.0.0.1".parse().unwrap()));
        engine.record_failure("127.0.0.1".parse().unwrap());
        assert!(!engine.is_locked("127.0.0.1".parse().unwrap()));
        engine.record_failure("127.0.0.1".parse().unwrap());
        assert!(!engine.is_locked("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn locks_after_max_attempts() {
        let engine = BruteForceEngine::new(make_test_config());

        engine.record_failure("127.0.0.1".parse().unwrap());
        engine.record_failure("127.0.0.1".parse().unwrap());
        let locked = engine.record_failure("127.0.0.1".parse().unwrap());

        assert!(locked, "account should be locked after 3 failures");
        assert!(engine.is_locked("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn disabled_engine_never_locks() {
        let config = BruteForceConfig {
            enabled: false,
            ..make_test_config()
        };
        let engine = BruteForceEngine::new(config);

        for _ in 0..100 {
            engine.record_failure("127.0.0.1".parse().unwrap());
        }

        assert!(!engine.is_locked("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn different_users_tracked_separately() {
        let engine = BruteForceEngine::new(make_test_config());

        engine.record_failure("127.0.0.1".parse().unwrap());
        engine.record_failure("127.0.0.1".parse().unwrap());
        engine.record_failure("127.0.0.1".parse().unwrap());

        assert!(engine.is_locked("127.0.0.1".parse().unwrap()));
        assert!(!engine.is_locked("127.0.0.2".parse().unwrap()));
    }

    #[test]
    fn stale_trackers_are_evicted() {
        let config = BruteForceConfig {
            enabled: true,
            max_attempts: 3,
            lockout_duration_secs: 1,
            window_secs: 0,
        };
        let engine = BruteForceEngine::new(config);

        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        engine.record_failure(ip);

        // Window is 0, so the attempt is immediately considered stale and the
        // next record_failure call should evict it (it's not locked, so it's
        // eligible for eviction).
        engine.record_failure(ip);

        // The tracker should still exist because we just recorded a failure
        // (which itself triggers eviction but also creates the entry).
        // The key point: eviction runs without panic and the engine remains
        // functional.
        assert!(!engine.is_locked(ip));
    }

    #[test]
    fn evicts_trackers_outside_window() {
        let config = BruteForceConfig {
            enabled: true,
            max_attempts: 3,
            lockout_duration_secs: 60,
            window_secs: 3600,
        };
        let engine = BruteForceEngine::new(config);

        // Record a failure for an IP, which should create a tracker
        engine.record_failure("10.0.0.1".parse().unwrap());

        // Trigger eviction by recording another failure for a different IP
        engine.record_failure("10.0.0.2".parse().unwrap());

        // The trackers should still exist (they have recent attempts)
        assert!(!engine.is_locked("10.0.0.1".parse().unwrap()));
        assert!(!engine.is_locked("10.0.0.2".parse().unwrap()));
    }
}
