//! Rate limiting pipeline stage.
//!
//! Evaluates `rate_limit` configuration rules against each request.
//! If any rule's bucket is exhausted, the request is rejected with a 429
//! (or configured) status code and a `Retry-After` header.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use http::{HeaderMap, HeaderValue};
use parking_lot::Mutex;

use crate::config::{parse_rate_limit_config, RateLimitConfig};
use crate::key_extractor::KeyExtractor;
use crate::registry::TokenBucketRegistry;

/// Shared rate limit engine that manages per-key token bucket registries.
///
/// The engine is created once during module loading and shared across all
/// stage invocations. It maintains a registry per unique rate limit rule,
/// identified by a rule fingerprint.
pub struct RateLimitEngine {
    /// Registries keyed by a rule fingerprint string.
    /// The fingerprint is derived from the rule's configuration (rate, burst, key, etc.)
    /// so that identical rules across hosts share the same registry.
    registries: Mutex<HashMap<String, TokenBucketRegistry>>,
}

impl RateLimitEngine {
    /// Create a new empty rate limit engine.
    pub fn new() -> Self {
        Self {
            registries: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a registry for the given rate limit config.
    fn get_or_create_registry(&self, config: &RateLimitConfig) -> TokenBucketRegistry {
        // Create a fingerprint from the config parameters
        let fingerprint = format!(
            "cap:{}|rate:{}|ttl:{}|max:{}",
            config.rate + config.burst,
            config.rate,
            config.bucket_ttl_secs,
            config.max_buckets
        );

        let mut registries = self.registries.lock();
        registries
            .entry(fingerprint)
            .or_insert_with(|| {
                TokenBucketRegistry::new(
                    config.rate + config.burst,
                    config.rate as f64,
                    config.bucket_ttl_secs,
                    config.max_buckets,
                )
            })
            .clone()
    }

    /// Check all rate limit rules against the current request.
    ///
    /// Returns `Some(response)` if any rule is exhausted, or `None` if all rules pass.
    fn check_rate_limits(&self, ctx: &mut HttpContext) -> Option<HttpResponse> {
        let rules = parse_rate_limit_config(&ctx.configuration);
        if rules.is_empty() {
            return None;
        }

        for config in &rules {
            // Extract key from request
            let key = match config.key.extract(ctx) {
                Some(k) => k,
                None => continue, // Can't extract key — skip this rule
            };

            // Get or create registry for this rule
            let registry = self.get_or_create_registry(config);

            // Get or create bucket
            let Some(bucket) = registry.get_or_create(&key) else {
                // Registry at capacity — apply backpressure
                ferron_core::log_warn!("Rate limit registry at capacity — applying backpressure");
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.ratelimit.rejected",
                    attributes: vec![(
                        "ferron.ratelimit.key_type",
                        MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                    )],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests rejected due to rate limit registry at capacity."),
                }));
                return Some(Self::make_response(config.deny_status, 1.0));
            };

            // Attempt to consume one token
            if !bucket.try_consume(1) {
                let retry_after = bucket.time_until_available(1);
                ferron_core::log_debug!(
                    "Rate limit bucket exhausted for key \"{}\" (type: {})",
                    key,
                    key_type_label(&config.key)
                );
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Debug,
                    message: format!(
                        "Rate limit bucket exhausted for key \"{}\" (type: {})",
                        key,
                        key_type_label(&config.key)
                    ),
                    target: "ferron-ratelimit",
                }));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.ratelimit.rejected",
                    attributes: vec![(
                        "ferron.ratelimit.key_type",
                        MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                    )],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests rejected due to exhausted rate limit buckets."),
                }));
                return Some(Self::make_response(config.deny_status, retry_after));
            }

            // Token consumed successfully — emit allowed counter
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.ratelimit.allowed",
                attributes: vec![(
                    "ferron.ratelimit.key_type",
                    MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                )],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Requests that passed rate limiting."),
            }));
        }

        None
    }

    fn make_response(status: u16, retry_after_secs: f64) -> HttpResponse {
        let retry_after_value = retry_after_secs.ceil().max(1.0) as u64;

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::RETRY_AFTER,
            HeaderValue::from_str(&retry_after_value.to_string())
                .expect("retry-after value should be valid"),
        );
        HttpResponse::BuiltinError(status, Some(headers))
    }
}

/// Returns a human-readable label for the key extractor type.
fn key_type_label(key: &KeyExtractor) -> &'static str {
    match key {
        KeyExtractor::RemoteAddress => "ip",
        KeyExtractor::Uri => "uri",
        KeyExtractor::Header(_) => "header",
    }
}

impl Default for RateLimitEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Pipeline stage that enforces rate limit rules from configuration.
pub struct RateLimitStage {
    engine: Arc<RateLimitEngine>,
}

impl RateLimitStage {
    /// Create a new rate limit stage with the shared engine.
    pub fn new(engine: Arc<RateLimitEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for RateLimitStage {
    fn name(&self) -> &str {
        "rate_limit"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run after client_ip is resolved (so remote_address is accurate),
        // and before the main request handler.
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
        ]
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if let Some(response) = self.engine.check_rate_limits(ctx) {
            ctx.res = Some(response);
            return Ok(false); // Stop pipeline — response is ready
        }

        Ok(true) // Continue to next stage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Empty};
    use rustc_hash::FxHashMap;
    use std::collections::HashMap as StdHashMap;
    use typemap_rev::TypeMap;

    fn make_test_context(
        remote_address: &str,
        config: Option<LayeredConfiguration>,
    ) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri("/path")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: config.unwrap_or_default(),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: remote_address.parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_rate_limit_config(rate: u64, burst: u64) -> LayeredConfiguration {
        let mut inner_directives = StdHashMap::new();
        inner_directives.insert(
            "rate".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(rate as i64, None)],
                children: None,
                span: None,
            }],
        );
        inner_directives.insert(
            "burst".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(burst as i64, None)],
                children: None,
                span: None,
            }],
        );

        let mut directives = StdHashMap::new();
        directives.insert(
            "rate_limit".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(inner_directives),
                    matchers: StdHashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let engine = Arc::new(RateLimitEngine::new());
        let stage = RateLimitStage::new(engine);
        let config = make_rate_limit_config(10, 5);

        for i in 0..15 {
            let mut ctx =
                make_test_context(&format!("192.0.2.1:{}", 20000 + i), Some(config.clone()));
            let result = stage.run(&mut ctx).await.unwrap();
            assert!(result, "request should be allowed");
            assert!(ctx.res.is_none());
        }
    }

    #[tokio::test]
    async fn rejects_when_bucket_exhausted() {
        let engine = Arc::new(RateLimitEngine::new());
        let stage = RateLimitStage::new(engine);
        let config = make_rate_limit_config(5, 0);

        // First 5 requests should pass
        for i in 0..5 {
            let mut ctx = make_test_context(&format!("192.0.2.1:1234{}", i), Some(config.clone()));
            let result = stage.run(&mut ctx).await.unwrap();
            assert!(result);
            assert!(ctx.res.is_none());
        }

        // 6th should be rejected
        let mut ctx = make_test_context("192.0.2.1:12345", Some(config));
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "should stop pipeline when rate limited");
        assert!(ctx.res.is_some());
    }

    #[tokio::test]
    async fn different_ips_get_separate_buckets() {
        let engine = Arc::new(RateLimitEngine::new());
        let stage = RateLimitStage::new(engine);
        let config = make_rate_limit_config(1, 0);

        // IP1 uses its token
        let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config.clone()));
        assert!(stage.run(&mut ctx1).await.unwrap());

        // IP2 should still have its own token
        let mut ctx2 = make_test_context("192.0.2.2:12345", Some(config.clone()));
        assert!(stage.run(&mut ctx2).await.unwrap());

        // IP1 should be exhausted
        let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config));
        assert!(!stage.run(&mut ctx1).await.unwrap());
    }

    #[tokio::test]
    async fn sets_retry_after_header() {
        let engine = Arc::new(RateLimitEngine::new());
        let stage = RateLimitStage::new(engine);
        let config = make_rate_limit_config(1, 0);

        // Use the token
        let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config.clone()));
        stage.run(&mut ctx1).await.unwrap();

        // Next request should be rejected with Retry-After
        let mut ctx2 = make_test_context("192.0.2.1:12345", Some(config));
        stage.run(&mut ctx2).await.unwrap();

        if let Some(HttpResponse::BuiltinError(status, headers)) = ctx2.res {
            assert!(headers.unwrap().contains_key(http::header::RETRY_AFTER));
            assert_eq!(status, 429);
        } else {
            panic!("Expected rate limit response");
        }
    }

    #[tokio::test]
    async fn no_rules_is_noop() {
        let engine = Arc::new(RateLimitEngine::new());
        let stage = RateLimitStage::new(engine);
        let mut ctx = make_test_context("192.0.2.1:12345", None);
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
    }
}
