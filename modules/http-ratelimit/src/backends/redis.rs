//! Redis/Valkey distributed token-bucket backend.
//!
//! Implements the same token-bucket semantics as the in-memory backend via
//! an atomic Lua script, so multiple Ferron instances share the same limits.
//! All Redis I/O runs on the secondary Tokio runtime (the `redis` crate
//! requires Tokio); callers on primary `zincio` threads are bridged via the
//! captured handle in `super::runtime`.

use std::time::Duration;

use super::runtime::try_get_secondary_handle;
use super::{BackendError, RateLimitBackend, RateLimitDecision};

/// Lua token-bucket emulation.
///
/// `KEYS[1]` = bucket hash key, `ARGV` = `capacity, refill_per_sec, now_ms, ttl_ms`.
/// Stored hash fields: `tokens` (float), `ts` (ms). Returns `{allowed, retry_after_ms}`.
const TOKEN_BUCKET_LUA: &str = r#"
local key = KEYS[1]
local capacity = tonumber(ARGV[1])
local refill_rate = tonumber(ARGV[2])
local now_ms = tonumber(ARGV[3])
local ttl_ms = tonumber(ARGV[4])

local data = redis.call('HMGET', key, 'tokens', 'ts')
local tokens = tonumber(data[1])
local ts = tonumber(data[2])

if tokens == nil or ts == nil then
  tokens = capacity
  ts = now_ms
else
  local elapsed = math.max(0, now_ms - ts) / 1000.0
  tokens = math.min(capacity, tokens + elapsed * refill_rate)
  ts = now_ms
end

local allowed = 0
local retry_after_ms = 0
if tokens >= 1 then
  tokens = tokens - 1
  allowed = 1
else
  if refill_rate > 0 then
    retry_after_ms = math.ceil((1 - tokens) / refill_rate * 1000)
  else
    retry_after_ms = ttl_ms
  end
end

redis.call('HSET', key, 'tokens', tokens, 'ts', ts)
redis.call('PEXPIRE', key, ttl_ms)
return {allowed, retry_after_ms}
"#;

/// Distributed backend backed by Redis or Valkey (RESP-compatible).
#[derive(Clone)]
pub struct RedisBackend {
    client: redis::Client,
    key_prefix: String,
    timeout: Duration,
    /// Lazily-initialized shared connection manager.
    ///
    /// Clones share the same cell via `Arc`, so all backend instances built
    /// from one config reuse a single auto-reconnecting connection instead of
    /// handshaking per request. Initialization runs on the secondary Tokio
    /// runtime (see `try_consume`), where `ConnectionManager` can spawn its
    /// background task.
    manager:
        std::sync::Arc<tokio::sync::OnceCell<tokio::sync::Mutex<redis::aio::ConnectionManager>>>,
}

impl RedisBackend {
    /// Create a backend from a Redis/Valkey URL.
    ///
    /// The URL is parsed with `redis::Client::open` and supports
    /// `redis://`, `rediss://` and `valkey://` schemes. No connection is
    /// opened until the first request.
    pub fn new(url: &str, key_prefix: String, timeout: Duration) -> Result<Self, BackendError> {
        let client = redis::Client::open(url).map_err(|e| BackendError::Config(e.to_string()))?;
        Ok(Self {
            client,
            key_prefix,
            timeout,
            manager: std::sync::Arc::new(tokio::sync::OnceCell::new()),
        })
    }

    async fn execute(
        backend: RedisBackend,
        namespaced_key: String,
        capacity: u64,
        refill_rate: f64,
        ttl_secs: u64,
    ) -> Result<RateLimitDecision, BackendError> {
        let redis_key = format!("{}{}", backend.key_prefix, namespaced_key);
        let ttl_ms = ttl_secs.saturating_mul(1000).max(1000) as i64;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let op = async move {
            let mutex = backend
                .manager
                .get_or_try_init(|| async {
                    let manager = backend
                        .client
                        .get_connection_manager()
                        .await
                        .map_err(|e| BackendError::Unavailable(e.to_string()))?;
                    Ok::<_, BackendError>(tokio::sync::Mutex::new(manager))
                })
                .await?;
            let mut guard = mutex.lock().await;
            let script = redis::Script::new(TOKEN_BUCKET_LUA);
            let (allowed, retry_after_ms): (i32, i64) = script
                .key(redis_key)
                .arg(capacity as i64)
                .arg(refill_rate)
                .arg(now_ms)
                .arg(ttl_ms)
                .invoke_async(&mut *guard)
                .await
                .map_err(|e| BackendError::Unavailable(e.to_string()))?;
            Ok(RateLimitDecision {
                allowed: allowed == 1,
                retry_after_secs: (retry_after_ms.max(0) as f64) / 1000.0,
            })
        };

        tokio::time::timeout(backend.timeout, op)
            .await
            .map_err(|_| BackendError::Unavailable("redis timeout".to_string()))?
    }
}

#[async_trait::async_trait(?Send)]
impl RateLimitBackend for RedisBackend {
    async fn try_consume(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
        ttl_secs: u64,
    ) -> Result<RateLimitDecision, BackendError> {
        let fut = Self::execute(
            self.clone(),
            key.to_string(),
            capacity,
            refill_rate,
            ttl_secs,
        );

        if let Some(handle) = try_get_secondary_handle() {
            // We are likely on a primary `zincio` thread; hop to Tokio.
            // `handle.spawn` requires `Send`, and the redis future is `Send`.
            let join = handle
                .spawn(fut)
                .await
                .map_err(|e| BackendError::Unavailable(format!("redis task join failed: {e}")))?;
            join
        } else if tokio::runtime::Handle::try_current().is_ok() {
            // Unit tests on Tokio...
            fut.await
        } else {
            // Backend not yet available, bailing out...
            Err(BackendError::Unavailable(
                "module not yet initialized".into(),
            ))
        }
    }

    async fn consume_throttled(&self, key: &str) -> Result<bool, BackendError> {
        // Throttle over Redis is a single sleep, per agreed semantics:
        // try once, sleep `retry_after`, retry once. Never busy-loop.
        // The caller supplies capacity/rate/ttl via `try_consume`; this
        // helper is unused for Redis (stage implements the sleep). Return
        // `false` to indicate "not throttled" if somehow called.
        let _ = key;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::RateLimitBackend;

    fn test_url() -> String {
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/0".to_string())
    }

    async fn reachable(url: &str) -> bool {
        let Ok(client) = redis::Client::open(url) else {
            return false;
        };
        let Ok(mut con) = client.get_multiplexed_async_connection().await else {
            return false;
        };
        redis::cmd("PING")
            .query_async::<String>(&mut con)
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn redis_rejects_invalid_url_at_construction() {
        let err = RedisBackend::new("not a url %%", "p:".into(), Duration::from_millis(200));
        assert!(matches!(err, Err(BackendError::Config(_))));
    }

    #[tokio::test]
    async fn redis_token_bucket_burst_then_denies() {
        let url = test_url();
        if !reachable(&url).await {
            eprintln!("skipping redis test: {url} unreachable");
            return;
        }
        let backend =
            RedisBackend::new(&url, "test:rl:burst:".into(), Duration::from_secs(2)).unwrap();
        let key = format!("k1:{}", uuid_like());
        // capacity 3, refill 10/sec
        for _ in 0..3 {
            let d = backend.try_consume(&key, 3, 10.0, 60).await.unwrap();
            assert!(d.allowed);
        }
        let denied = backend.try_consume(&key, 3, 10.0, 60).await.unwrap();
        assert!(!denied.allowed);
        assert!(denied.retry_after_secs > 0.0);
    }

    #[tokio::test]
    async fn redis_refills_over_time() {
        let url = test_url();
        if !reachable(&url).await {
            eprintln!("skipping redis test: {url} unreachable");
            return;
        }
        let backend =
            RedisBackend::new(&url, "test:rl:refill:".into(), Duration::from_secs(2)).unwrap();
        let key = format!("k2:{}", uuid_like());
        // capacity 2, refill 20/sec => ~1 token per 50ms
        assert!(
            backend
                .try_consume(&key, 2, 20.0, 60)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            backend
                .try_consume(&key, 2, 20.0, 60)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !backend
                .try_consume(&key, 2, 20.0, 60)
                .await
                .unwrap()
                .allowed
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            backend
                .try_consume(&key, 2, 20.0, 60)
                .await
                .unwrap()
                .allowed
        );
    }

    #[tokio::test]
    async fn redis_unavailable_returns_error() {
        // Unroutable port: expect Unavailable, stage maps it to fail-open/closed.
        let backend = RedisBackend::new(
            "redis://127.0.0.1:6399/0",
            "test:rl:down:".into(),
            Duration::from_millis(200),
        )
        .unwrap();
        let err = backend
            .try_consume("any", 10, 10.0, 60)
            .await
            .expect_err("expected unavailable");
        assert!(matches!(err, BackendError::Unavailable(_)));
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}-{}", std::process::id(), nanos)
    }
}
