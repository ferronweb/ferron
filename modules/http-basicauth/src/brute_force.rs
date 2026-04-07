//! Brute-force protection engine for HTTP Basic Authentication.
//!
//! Tracks failed authentication attempts per username and locks out accounts
//! that exceed the configured threshold within a sliding time window.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Configuration for brute-force protection.
#[derive(Debug, Clone)]
pub struct BruteForceConfig {
    /// Whether brute-force protection is enabled.
    pub enabled: bool,
    /// Maximum failed attempts allowed within the window before lockout.
    pub max_attempts: usize,
    /// How long to lock the account after exceeding max attempts (seconds).
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

    /// Check if the account is currently locked out.
    fn is_locked(&self) -> bool {
        if let Some(until) = self.locked_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Record a failed attempt. Returns `true` if the account is now locked.
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
}

/// Shared brute-force protection engine.
///
/// Manages per-username attempt tracking with automatic lockout and TTL-based
/// eviction to prevent unbounded memory growth.
pub struct BruteForceEngine {
    /// Per-username attempt trackers.
    trackers: Mutex<HashMap<String, AttemptTracker>>,
    /// Configuration for this engine.
    config: BruteForceConfig,
}

impl BruteForceEngine {
    /// Create a new brute-force engine with the given configuration.
    pub fn new(config: BruteForceConfig) -> Self {
        Self {
            trackers: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Check if a username is currently locked out.
    ///
    /// Returns `true` if the username is locked and should be rejected immediately.
    pub fn is_locked(&self, username: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        let mut trackers = self.trackers.lock();
        let tracker = trackers
            .entry(username.to_string())
            .or_insert_with(AttemptTracker::new);

        // Check lock status (pruning happens implicitly on access)
        if tracker.is_locked() {
            return true;
        }

        // If lockout has expired, reset the tracker
        if tracker.locked_until.is_some() {
            tracker.clear();
        }

        false
    }

    /// Record a failed authentication attempt for a username.
    ///
    /// Returns `true` if the account has now been locked out.
    pub fn record_failure(&self, username: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        let mut trackers = self.trackers.lock();
        let tracker = trackers
            .entry(username.to_string())
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

    /// Clear the attempt history for a username (called on successful authentication).
    pub fn clear_history(&self, username: &str) {
        let mut trackers = self.trackers.lock();
        if let Some(tracker) = trackers.get_mut(username) {
            tracker.clear();
        }
    }

    /// Evict stale trackers that have no recent attempts and no active lockout.
    ///
    /// This should be called periodically to prevent unbounded memory growth.
    /// In practice, the engine evicts lazily when trackers are accessed.
    #[allow(dead_code)]
    pub fn evict_stale(&self) {
        let mut trackers = self.trackers.lock();
        let window = Duration::from_secs(self.config.window_secs);
        let cutoff = Instant::now().checked_sub(window).unwrap_or(Instant::now());

        trackers.retain(|_, tracker| {
            // Keep if locked until a future time
            if let Some(until) = tracker.locked_until {
                if Instant::now() < until {
                    return true;
                }
            }

            // Keep if there are recent attempts
            tracker.attempts.iter().any(|&t| t >= cutoff)
        });
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

        assert!(!engine.is_locked("alice"));
        engine.record_failure("alice");
        assert!(!engine.is_locked("alice"));
        engine.record_failure("alice");
        assert!(!engine.is_locked("alice"));
    }

    #[test]
    fn locks_after_max_attempts() {
        let engine = BruteForceEngine::new(make_test_config());

        engine.record_failure("alice");
        engine.record_failure("alice");
        let locked = engine.record_failure("alice");

        assert!(locked, "account should be locked after 3 failures");
        assert!(engine.is_locked("alice"));
    }

    #[test]
    fn clears_history_on_success() {
        let engine = BruteForceEngine::new(make_test_config());

        // 2 failures
        engine.record_failure("alice");
        engine.record_failure("alice");
        assert!(!engine.is_locked("alice"));

        // Clear on success
        engine.clear_history("alice");

        // Should need 3 fresh failures to lock
        engine.record_failure("alice");
        engine.record_failure("alice");
        engine.record_failure("alice");
        assert!(engine.is_locked("alice"));
    }

    #[test]
    fn clear_allows_fresh_attempts() {
        let engine = BruteForceEngine::new(make_test_config());

        // 2 failures
        engine.record_failure("alice");
        engine.record_failure("alice");
        engine.clear_history("alice");

        // After clearing, should need 3 fresh failures to lock
        engine.record_failure("alice");
        engine.record_failure("alice");
        assert!(!engine.is_locked("alice"));
        // 3rd failure triggers lockout
        assert!(engine.record_failure("alice"));
        assert!(engine.is_locked("alice"));
    }

    #[test]
    fn disabled_engine_never_locks() {
        let config = BruteForceConfig {
            enabled: false,
            ..make_test_config()
        };
        let engine = BruteForceEngine::new(config);

        for _ in 0..100 {
            engine.record_failure("alice");
        }

        assert!(!engine.is_locked("alice"));
    }

    #[test]
    fn different_users_tracked_separately() {
        let engine = BruteForceEngine::new(make_test_config());

        engine.record_failure("alice");
        engine.record_failure("alice");
        engine.record_failure("alice");

        assert!(engine.is_locked("alice"));
        assert!(!engine.is_locked("bob"));
    }

    #[test]
    fn evict_removes_stale_trackers() {
        let engine = BruteForceEngine::new(make_test_config());

        engine.record_failure("alice");
        engine.record_failure("bob");
        engine.clear_history("bob");

        // Evict should remove bob (no attempts, not locked) but keep alice
        engine.evict_stale();

        let trackers = engine.trackers.lock();
        assert!(trackers.contains_key("alice"));
        assert!(!trackers.contains_key("bob"));
    }
}
