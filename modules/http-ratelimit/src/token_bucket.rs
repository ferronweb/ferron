//! Token bucket algorithm for rate limiting.
//!
//! Each bucket has a capacity (max tokens) and a refill rate (tokens per second).
//! Tokens are lazily refilled on access — no background timer is needed.

use std::sync::Arc;
use std::time::Instant;

/// A single token bucket (not thread-safe).
///
/// Uses floating-point tokens to support fractional refill rates.
/// Tokens are refilled lazily on each `try_consume` call.
#[derive(Debug)]
pub struct TokenBucket {
    /// Maximum number of tokens the bucket can hold.
    capacity: u64,
    /// Current number of tokens (fractional for sub-token refill).
    tokens: f64,
    /// Tokens added per second.
    refill_rate: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket.
    ///
    /// The bucket starts full (`tokens == capacity`).
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity as f64,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Attempt to consume `n` tokens.
    ///
    /// Refills the bucket first based on elapsed time, then attempts consumption.
    /// Returns `true` if tokens were consumed, `false` if the bucket is empty.
    pub fn try_consume(&mut self, n: u64) -> bool {
        self.refill();
        let needed = n as f64;
        if self.tokens >= needed {
            self.tokens -= needed;
            true
        } else {
            false
        }
    }

    /// Refill tokens based on elapsed time since last refill.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            let new_tokens = elapsed * self.refill_rate;
            self.tokens = (self.tokens + new_tokens).min(self.capacity as f64);
            self.last_refill = now;
        }
    }

    /// Get the current number of available tokens (after refill).
    pub fn available_tokens(&self) -> f64 {
        let elapsed = Instant::now()
            .duration_since(self.last_refill)
            .as_secs_f64();
        (self.tokens + elapsed * self.refill_rate).min(self.capacity as f64)
    }

    /// Estimate seconds until `n` tokens are available.
    pub fn time_until_available(&self, n: u64) -> f64 {
        let current = self.available_tokens();
        let needed = (n as f64) - current;
        if needed <= 0.0 {
            0.0
        } else if self.refill_rate > 0.0 {
            needed / self.refill_rate
        } else {
            f64::INFINITY
        }
    }
}

/// Thread-safe token bucket using `parking_lot::Mutex`.
///
/// Wraps a `TokenBucket` in an `Arc<Mutex>` for cheap cloning and concurrent access.
/// Per-key rate limiting naturally shards contention across buckets,
/// so mutex overhead is minimal in practice.
#[derive(Clone)]
pub struct ConcurrentTokenBucket {
    inner: Arc<parking_lot::Mutex<TokenBucket>>,
}

impl ConcurrentTokenBucket {
    /// Create a new thread-safe token bucket.
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        Self {
            inner: Arc::new(parking_lot::Mutex::new(TokenBucket::new(
                capacity,
                refill_rate,
            ))),
        }
    }

    /// Attempt to consume `n` tokens. Returns `true` on success.
    pub fn try_consume(&self, n: u64) -> bool {
        self.inner.lock().try_consume(n)
    }

    /// Estimate seconds until `n` tokens are available.
    pub fn time_until_available(&self, n: u64) -> f64 {
        self.inner.lock().time_until_available(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn bucket_starts_full() {
        let bucket = TokenBucket::new(10, 1.0);
        assert!(bucket.available_tokens() >= 10.0);
    }

    #[test]
    fn consumes_tokens() {
        let mut bucket = TokenBucket::new(10, 1.0);
        assert!(bucket.try_consume(5));
        assert!(bucket.available_tokens() < 6.0);
    }

    #[test]
    fn rejects_when_empty() {
        let mut bucket = TokenBucket::new(2, 0.0); // zero refill rate
        assert!(bucket.try_consume(2));
        assert!(!bucket.try_consume(1));
    }

    #[test]
    fn refills_over_time() {
        let mut bucket = TokenBucket::new(10, 100.0); // 100 tokens/sec
        assert!(bucket.try_consume(10)); // drain the bucket

        // Wait for refill
        thread::sleep(Duration::from_millis(50));

        // Should have ~5 tokens refilled
        assert!(bucket.available_tokens() >= 3.0);
        assert!(bucket.try_consume(3));
    }

    #[test]
    fn capacity_is_capped() {
        let mut bucket = TokenBucket::new(5, 1000.0); // very fast refill
        thread::sleep(Duration::from_millis(10));
        bucket.refill();
        assert!(bucket.tokens <= 5.0);
    }

    #[test]
    fn time_until_available_zero_when_full() {
        let bucket = TokenBucket::new(10, 1.0);
        assert_eq!(bucket.time_until_available(5), 0.0);
    }

    #[test]
    fn time_until_available_positive_when_low() {
        let mut bucket = TokenBucket::new(10, 2.0);
        bucket.try_consume(10); // drain
        let wait = bucket.time_until_available(4);
        assert!(wait > 0.0, "expected positive wait time, got {wait}");
    }

    #[test]
    fn concurrent_bucket_consumption() {
        let bucket = ConcurrentTokenBucket::new(100, 0.0);
        let mut handles = Vec::new();

        for _ in 0..10 {
            let b = bucket.clone();
            handles.push(thread::spawn(move || b.try_consume(10)));
        }

        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&s| s)
            .count();

        // Exactly 10 threads each consuming 1 token from 100 → all should succeed
        assert_eq!(successes, 10);
    }

    #[test]
    fn concurrent_bucket_exhaustion() {
        let bucket = ConcurrentTokenBucket::new(5, 0.0);
        let mut handles = Vec::new();

        for _ in 0..10 {
            let b = bucket.clone();
            handles.push(thread::spawn(move || b.try_consume(1)));
        }

        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|&s| s)
            .count();

        assert_eq!(successes, 5, "exactly 5 of 10 should succeed");
    }
}
