//! Pluggable rate limiting backends.
//!
//! The in-memory backend (`memory`) preserves the original single-node
//! token-bucket behavior. The Redis backend (`redis`) provides distributed
//! limiting over Redis or Valkey via an atomic Lua token-bucket script.
//!
//! [`RateLimitBackend`] is the unified async interface used by the pipeline
//! stage. [`RateLimitBackendKind`] dispatches to the configured backend.

pub mod memory;
pub mod redis;
pub mod runtime;

pub use memory::MemoryBackend;
pub use redis::RedisBackend;

/// Outcome of a single token-bucket consume attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitDecision {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// Seconds until a token is available (meaningful when denied).
    pub retry_after_secs: f64,
}

/// Backend failure modes.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendError {
    /// In-memory registry at `max_buckets` capacity (backpressure).
    AtCapacity,
    /// Distributed backend unreachable/timed out.
    Unavailable(String),
    /// Invalid backend configuration.
    Config(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::AtCapacity => write!(f, "rate limit registry at capacity"),
            BackendError::Unavailable(msg) => write!(f, "rate limit backend unavailable: {msg}"),
            BackendError::Config(msg) => write!(f, "invalid rate limit backend config: {msg}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// Unified async backend interface.
///
/// `key` is a fully namespaced key (`{zone}:{fingerprint}:{user_key}` for
/// Redis; plain user key also works for memory since registries are already
/// sharded per rule). `capacity`/`refill_rate`/`ttl_secs` describe the rule;
/// memory backends may ignore per-call values (registry carries them).
#[async_trait::async_trait(?Send)]
pub trait RateLimitBackend: Send + Sync {
    /// Attempt to consume one token.
    async fn try_consume(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
        ttl_secs: u64,
    ) -> Result<RateLimitDecision, BackendError>;

    /// Throttled consume: sleep until a token is available, then allow.
    ///
    /// Returns `true` if the request was delayed. Memory blocks via the
    /// bucket; Redis uses a single sleep + retry (implemented by the stage).
    async fn consume_throttled(&self, key: &str) -> Result<bool, BackendError>;
}

/// Configured backend selected per request scope.
#[derive(Clone)]
pub enum RateLimitBackendKind {
    /// Single-node DashMap backend.
    Memory(MemoryBackend),
    /// Distributed Redis/Valkey backend.
    Redis(RedisBackend),
}

#[async_trait::async_trait(?Send)]
impl RateLimitBackend for RateLimitBackendKind {
    async fn try_consume(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
        ttl_secs: u64,
    ) -> Result<RateLimitDecision, BackendError> {
        match self {
            RateLimitBackendKind::Memory(m) => {
                m.try_consume(key, capacity, refill_rate, ttl_secs).await
            }
            RateLimitBackendKind::Redis(r) => {
                r.try_consume(key, capacity, refill_rate, ttl_secs).await
            }
        }
    }

    async fn consume_throttled(&self, key: &str) -> Result<bool, BackendError> {
        match self {
            RateLimitBackendKind::Memory(m) => m.consume_throttled(key).await,
            RateLimitBackendKind::Redis(r) => r.consume_throttled(key).await,
        }
    }
}
