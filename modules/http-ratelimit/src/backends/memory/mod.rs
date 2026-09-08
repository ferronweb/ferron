//! In-memory rate limiting backend.
//!
//! This module hosts the original single-node implementation based on
//! per-key token buckets stored in a sharded `DashMap`. It is the default
//! backend and preserves the exact semantics used before distributed
//! backends were introduced.

pub mod registry;
pub mod token_bucket;

// Exports used for fuzzing. For not fuzzing, they are unused
// and marked with `#[allow(unused_imports)]`.
#[allow(unused_imports)]
pub use registry::TokenBucketRegistry;
#[allow(unused_imports)]
pub use token_bucket::{ConcurrentTokenBucket, TokenBucket};

use super::{BackendError, RateLimitBackend, RateLimitDecision};

/// In-memory backend backed by [`TokenBucketRegistry`].
///
/// Each `MemoryBackend` instance owns one registry (i.e. one
/// `(zone, rule)` shard). Cloning shares the underlying buckets.
#[derive(Clone)]
pub struct MemoryBackend {
    registry: TokenBucketRegistry,
}

impl MemoryBackend {
    /// Create a backend with the given bucket parameters.
    pub fn new(capacity: u64, refill_rate: f64, ttl_secs: u64, max_buckets: usize) -> Self {
        Self {
            registry: TokenBucketRegistry::new(capacity, refill_rate, ttl_secs, max_buckets),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl RateLimitBackend for MemoryBackend {
    async fn try_consume(
        &self,
        key: &str,
        _capacity: u64,
        _refill_rate: f64,
        _ttl_secs: u64,
    ) -> Result<RateLimitDecision, BackendError> {
        let Some(bucket) = self.registry.get_or_create(key) else {
            return Err(BackendError::AtCapacity);
        };
        if bucket.try_consume(1).await {
            Ok(RateLimitDecision {
                allowed: true,
                retry_after_secs: 0.0,
            })
        } else {
            let retry_after_secs = bucket.time_until_available(1).await;
            Ok(RateLimitDecision {
                allowed: false,
                retry_after_secs,
            })
        }
    }

    /// Sleep until a token is likely available, then retry once.
    ///
    /// For memory this delegates to the bucket's blocking `consume`, which
    /// sleeps on the primary `zincio` timer.
    async fn consume_throttled(&self, key: &str) -> Result<bool, BackendError> {
        let Some(bucket) = self.registry.get_or_create(key) else {
            return Err(BackendError::AtCapacity);
        };
        let throttled = bucket.consume(1).await;
        Ok(throttled)
    }
}
