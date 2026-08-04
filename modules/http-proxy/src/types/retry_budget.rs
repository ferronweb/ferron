//! Token-bucket retry budget for preventing cascading retry storms.
//!
//! Allocates a shared pool of retry tokens per backend group. Regular successful
//! requests deposit tokens into the bucket (up to a max capacity), while retries
//! consume tokens. When the budget is exhausted, retries are refused with a fast
//! 503 response to preserve upstream infrastructure.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

/// Thread-safe token-bucket retry budget.
///
/// Shared across all requests for a given proxy configuration. Tokens represent
/// retry capacity: each successful request deposits a token (up to capacity),
/// each retry consumes one. When the bucket is empty, retries are refused.
#[derive(Debug)]
pub struct RetryBudgetState {
    /// Current token count (fractional for sub-token refill).
    tokens: Mutex<f64>,
    /// Maximum tokens (burst capacity).
    capacity: u64,
    /// Tokens added per second by steady-state traffic.
    refill_rate: f64,
    /// Last refill timestamp.
    last_refill: Mutex<Instant>,
    /// Total requests observed (for rate calculation).
    total_requests: AtomicU64,
    /// Total retries attempted.
    total_retries: AtomicU64,
}

impl RetryBudgetState {
    /// Create a new retry budget state.
    ///
    /// The bucket starts full (`tokens == capacity`).
    #[inline]
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        Self {
            tokens: Mutex::new(capacity as f64),
            capacity,
            refill_rate,
            last_refill: Mutex::new(Instant::now()),
            total_requests: AtomicU64::new(0),
            total_retries: AtomicU64::new(0),
        }
    }

    /// Attempt to consume one retry token.
    ///
    /// Returns `true` if a token was consumed (retry is allowed), `false` if
    /// the bucket is empty (retry should be refused).
    #[inline]
    pub fn try_consume_retry_token(&self) -> bool {
        let mut tokens = self.tokens.lock();
        self.refill(&mut tokens);

        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Record a successful request and deposit a token.
    ///
    /// Called after each request completes successfully (regardless of whether
    /// it was a retry). Deposits a token into the bucket to replenish retry
    /// capacity proportional to steady-state traffic.
    #[inline]
    pub fn record_request(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        let mut tokens = self.tokens.lock();
        self.refill(&mut tokens);
        *tokens = (*tokens + 1.0).min(self.capacity as f64);
    }

    /// Record that a retry was attempted.
    #[inline]
    pub fn record_retry(&self) {
        self.total_retries.fetch_add(1, Ordering::Relaxed);
    }

    /// Get the current number of available tokens (after refill).
    #[inline]
    pub fn available_tokens(&self) -> f64 {
        let mut tokens = self.tokens.lock();
        self.refill(&mut tokens);
        *tokens
    }

    /// Get the current retry rate (retries / total requests).
    ///
    /// Returns `0.0` when no requests have been observed.
    #[cfg(test)]
    #[inline]
    pub fn current_retry_rate(&self) -> f64 {
        let requests = self.total_requests.load(Ordering::Relaxed);
        let retries = self.total_retries.load(Ordering::Relaxed);
        if requests == 0 {
            0.0
        } else {
            retries as f64 / requests as f64
        }
    }

    /// Estimate seconds until `n` tokens are available.
    ///
    /// Returns `0.0` if tokens are already available. When the refill rate is
    /// zero (tokens only arrive via `record_request`), returns a conservative
    /// fallback of `5.0` seconds since the arrival rate is unpredictable.
    #[inline]
    pub fn time_until_available(&self, n: u64) -> f64 {
        let current = self.available_tokens();
        let needed = (n as f64) - current;
        if needed <= 0.0 {
            0.0
        } else if self.refill_rate > 0.0 {
            needed / self.refill_rate
        } else {
            5.0
        }
    }

    /// Refill tokens based on elapsed time since last refill.
    #[inline]
    fn refill(&self, tokens: &mut f64) {
        let mut last_refill = self.last_refill.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        if elapsed > 0.0 && self.refill_rate > 0.0 {
            let new_tokens = elapsed * self.refill_rate;
            *tokens = (*tokens + new_tokens).min(self.capacity as f64);
            *last_refill = now;
        }
    }
}

/// Shared retry budget state, clonable via `Arc`.
#[derive(Clone)]
pub struct SharedRetryBudget {
    inner: Arc<RetryBudgetState>,
}

impl SharedRetryBudget {
    /// Create a new shared retry budget from configuration parameters.
    #[inline]
    pub fn new(capacity: u64, refill_rate: f64, _max_retry_rate: f64) -> Self {
        Self {
            inner: Arc::new(RetryBudgetState::new(capacity, refill_rate)),
        }
    }

    /// Attempt to consume one retry token. Returns `true` if allowed.
    #[inline]
    pub fn try_consume_retry_token(&self) -> bool {
        self.inner.try_consume_retry_token()
    }

    /// Record a successful request and deposit a token.
    #[inline]
    pub fn record_request(&self) {
        self.inner.record_request();
    }

    /// Record that a retry was attempted.
    #[inline]
    pub fn record_retry(&self) {
        self.inner.record_retry();
    }

    /// Get the current number of available tokens (after refill).
    #[inline]
    pub fn available_tokens(&self) -> f64 {
        self.inner.available_tokens()
    }

    /// Estimate seconds until `n` tokens are available.
    #[inline]
    pub fn time_until_available(&self, n: u64) -> f64 {
        self.inner.time_until_available(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn budget_starts_full() {
        let budget = RetryBudgetState::new(10, 1.0);
        assert!(budget.available_tokens() >= 10.0);
    }

    #[test]
    fn consumes_retry_tokens() {
        let budget = RetryBudgetState::new(5, 0.0);
        assert!(budget.try_consume_retry_token());
        assert!(budget.try_consume_retry_token());
        assert!(budget.try_consume_retry_token());
        assert!(budget.try_consume_retry_token());
        assert!(budget.try_consume_retry_token());
        assert!(!budget.try_consume_retry_token());
    }

    #[test]
    fn deposits_tokens_on_success() {
        let budget = RetryBudgetState::new(5, 0.0);
        // Drain the bucket
        for _ in 0..5 {
            assert!(budget.try_consume_retry_token());
        }
        assert!(!budget.try_consume_retry_token());

        // Record successful requests to replenish
        budget.record_request();
        assert!(budget.try_consume_retry_token());
    }

    #[test]
    fn capacity_is_capped() {
        let budget = RetryBudgetState::new(3, 1000.0);
        budget.record_request();
        budget.record_request();
        budget.record_request();
        budget.record_request();
        assert!(budget.available_tokens() <= 3.0);
    }

    #[test]
    fn tracks_retry_rate() {
        let budget = RetryBudgetState::new(10, 0.0);
        for _ in 0..10 {
            budget.record_request();
        }
        budget.record_retry();
        budget.record_retry();
        assert!((budget.current_retry_rate() - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_rate_when_no_requests() {
        let budget = RetryBudgetState::new(10, 0.0);
        assert_eq!(budget.current_retry_rate(), 0.0);
    }

    #[test]
    fn shared_budget_clone_shares_state() {
        let budget = SharedRetryBudget::new(5, 0.0, 0.1);
        let budget2 = budget.clone();
        budget.try_consume_retry_token();
        budget.try_consume_retry_token();
        assert!((budget2.available_tokens() - 3.0).abs() < 0.1);
    }

    #[test]
    fn concurrent_access() {
        let budget = SharedRetryBudget::new(100, 0.0, 0.1);
        let mut handles = Vec::new();

        for _ in 0..10 {
            let b = budget.clone();
            handles.push(thread::spawn(move || {
                let mut count = 0;
                for _ in 0..20 {
                    if b.try_consume_retry_token() {
                        count += 1;
                    }
                }
                count
            }));
        }

        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 100, "exactly 100 of 200 should succeed");
    }

    #[test]
    fn time_until_available_zero_when_full() {
        let budget = RetryBudgetState::new(10, 1.0);
        assert_eq!(budget.time_until_available(1), 0.0);
    }

    #[test]
    fn time_until_available_positive_when_drained() {
        let budget = RetryBudgetState::new(10, 2.0);
        // Drain the bucket
        for _ in 0..10 {
            budget.try_consume_retry_token();
        }
        let wait = budget.time_until_available(1);
        assert!(wait > 0.0, "expected positive wait time, got {wait}");
        // At 2 tokens/sec, 1 token takes 0.5s
        assert!((wait - 0.5).abs() < 0.1);
    }

    #[test]
    fn time_until_available_fallback_when_no_refill() {
        let budget = RetryBudgetState::new(10, 0.0);
        for _ in 0..10 {
            budget.try_consume_retry_token();
        }
        assert_eq!(budget.time_until_available(1), 5.0);
    }
}
