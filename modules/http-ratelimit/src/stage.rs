//! Rate limiting pipeline stage.
//!
//! Evaluates `rate_limit` configuration rules against each request.
//! If any rule's bucket is exhausted, the request is rejected with a 429
//! (or configured) status code and a `Retry-After` header.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::abuse::{get_global_abuse_recorder, AbuseEvent, AbuseEventType};
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue, TraceAttributeValue,
};
use http::{HeaderMap, HeaderValue};
use parking_lot::Mutex;

use crate::config::{parse_rate_limit_config, resolve_zone_id, RateLimitConfig, RateLimitZoneId};
use crate::key_extractor::KeyExtractor;
use crate::registry::TokenBucketRegistry;

/// Shared rate limit engine that manages per-key token bucket registries.
///
/// The engine is created once during module loading and shared across all
/// stage invocations. It maintains a registry per unique (zone, rule fingerprint)
/// pair so that hosts in different zones get isolated registries.
pub struct RateLimitEngine {
    /// Registries keyed by (zone_id, fingerprint).
    /// The zone_id determines the sharing scope, and the fingerprint
    /// identifies the specific rate limit rule configuration.
    registries: Mutex<HashMap<(RateLimitZoneId, String), TokenBucketRegistry>>,
}

impl RateLimitEngine {
    /// Create a new empty rate limit engine.
    pub fn new() -> Self {
        Self {
            registries: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a registry for the given rate limit config and zone.
    fn get_or_create_registry(
        &self,
        config: &RateLimitConfig,
        zone_id: &RateLimitZoneId,
    ) -> TokenBucketRegistry {
        let key_type = match &config.key {
            KeyExtractor::RemoteAddress => "ip",
            KeyExtractor::Uri => "uri",
            KeyExtractor::Header(name) => name.as_str(),
        };
        let fingerprint = format!(
            "cap:{}|rate:{}|ttl:{}|max:{}|key:{}",
            config.rate + config.burst,
            config.rate,
            config.bucket_ttl_secs,
            config.max_buckets,
            key_type
        );

        let mut registries = self.registries.lock();
        registries
            .entry((zone_id.clone(), fingerprint))
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
    async fn check_rate_limits(&self, ctx: &mut HttpContext) -> Option<HttpResponse> {
        let rules = parse_rate_limit_config(&ctx.configuration);
        if rules.is_empty() {
            return None;
        }

        // Resolve zone ID for this request
        let zone_id = resolve_zone_id(&ctx.configuration, &ctx.hostname);

        for config in &rules {
            let key = match config.key.extract(ctx) {
                Some(k) => k,
                None => continue, // Can't extract key, skip this rule...
            };

            let registry = self.get_or_create_registry(config, &zone_id);

            let Some(bucket) = registry.get_or_create(&key) else {
                // Registry at capacity, apply backpressure
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    target: "ferron-http-ratelimit",
                    message: "Rate limit registry at capacity — applying backpressure".into(),
                    summary: "Rate limit registry at capacity".into(),
                    attributes: vec![],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.ratelimit.rejected",
                    attributes: vec![
                        (
                            "ferron.ratelimit.zone",
                            MetricAttributeValue::String(zone_id.label().to_string()),
                        ),
                        (
                            "ferron.ratelimit.key_type",
                            MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests rejected due to rate limit registry at capacity."),
                    trace_context: current_event_trace_context(ctx),
                }));
                {
                    let sa = ctx.get_span_attributes();
                    sa.insert(
                        "ferron.ratelimit.result",
                        TraceAttributeValue::String("rejected".to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.zone",
                        TraceAttributeValue::String(zone_id.label().to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.key_type",
                        TraceAttributeValue::String(key_type_label(&config.key).to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.limit",
                        TraceAttributeValue::I64(config.rate as i64),
                    );
                    let log_fields = custom_access_log_fields(ctx);
                    log_fields.insert(
                        "ferron.ratelimit.result".into(),
                        CustomAccessLogField::String("rejected".into()),
                    );
                    log_fields.insert(
                        "ferron.ratelimit.zone".into(),
                        CustomAccessLogField::String(zone_id.label().to_string()),
                    );
                }
                return Some(Self::make_response(config.deny_status, 1.0));
            };

            // Attempt to consume one token
            let (allowed, throttled) = if config.throttle {
                let throttled = bucket.consume(1).await;
                (true, throttled) // Always allow, but throttle the bucket
            } else {
                (bucket.try_consume(1).await, false)
            };
            if !allowed {
                let retry_after = bucket.time_until_available(1).await;
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Debug,
                    message: format!(
                        "Rate limit bucket exhausted for key \"{}\" (type: {})",
                        key,
                        key_type_label(&config.key)
                    ),
                    summary: "Rate limit bucket exhausted".into(),
                    target: "ferron-ratelimit",
                    attributes: vec![
                        (
                            "ferron.ratelimit.zone",
                            LogAttributeValue::String(zone_id.label().to_string()),
                        ),
                        ("ferron.ratelimit.key", LogAttributeValue::String(key)),
                        (
                            "ferron.ratelimit.key_type",
                            LogAttributeValue::String(key_type_label(&config.key).to_string()),
                        ),
                    ],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.ratelimit.rejected",
                    attributes: vec![
                        (
                            "ferron.ratelimit.zone",
                            MetricAttributeValue::String(zone_id.label().to_string()),
                        ),
                        (
                            "ferron.ratelimit.key_type",
                            MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests rejected due to exhausted rate limit buckets."),
                    trace_context: current_event_trace_context(ctx),
                }));

                // Emit abuse event so the abuse protection module can track
                // repeated rate limit violations and potentially ban the IP.
                if let (Some(recorder), Some(ip)) = (
                    get_global_abuse_recorder(),
                    ctx.remote_address.map(|a| a.ip()),
                ) {
                    let abuse_event = AbuseEvent::new(
                        AbuseEventType::RateLimitExceeded,
                        ip,
                        format!("Rate limit {} req/s exceeded", config.rate),
                        50,
                    );
                    recorder.record_event(&abuse_event, ctx);
                }

                {
                    let sa = ctx.get_span_attributes();
                    sa.insert(
                        "ferron.ratelimit.result",
                        TraceAttributeValue::String("rejected".to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.zone",
                        TraceAttributeValue::String(zone_id.label().to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.key_type",
                        TraceAttributeValue::String(key_type_label(&config.key).to_string()),
                    );
                    sa.insert(
                        "ferron.ratelimit.limit",
                        TraceAttributeValue::I64(config.rate as i64),
                    );
                    sa.insert(
                        "ferron.ratelimit.retry_after_secs",
                        TraceAttributeValue::I64(retry_after.ceil() as i64),
                    );
                    let log_fields = custom_access_log_fields(ctx);
                    log_fields.insert(
                        "ferron.ratelimit.result".into(),
                        CustomAccessLogField::String("rejected".into()),
                    );
                    log_fields.insert(
                        "ferron.ratelimit.zone".into(),
                        CustomAccessLogField::String(zone_id.label().to_string()),
                    );
                    log_fields.insert(
                        "ferron.ratelimit.retry_after_secs".into(),
                        CustomAccessLogField::U64(retry_after.ceil() as u64),
                    );
                }
                return Some(Self::make_response(config.deny_status, retry_after));
            }

            // Token consumed successfully, emit allowed counter
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.ratelimit.allowed",
                attributes: vec![
                    (
                        "ferron.ratelimit.zone",
                        MetricAttributeValue::String(zone_id.label().to_string()),
                    ),
                    (
                        "ferron.ratelimit.key_type",
                        MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                    ),
                ],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Requests that passed rate limiting."),
                trace_context: current_event_trace_context(ctx),
            }));
            if throttled {
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.ratelimit.throttled",
                    attributes: vec![
                        (
                            "ferron.ratelimit.zone",
                            MetricAttributeValue::String(zone_id.label().to_string()),
                        ),
                        (
                            "ferron.ratelimit.key_type",
                            MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests that were throttled by rate limiting."),
                    trace_context: current_event_trace_context(ctx),
                }));
            }
            {
                let sa = ctx.get_span_attributes();
                sa.insert(
                    "ferron.ratelimit.result",
                    TraceAttributeValue::String(if throttled {
                        "throttled".to_string()
                    } else {
                        "allowed".to_string()
                    }),
                );
                sa.insert(
                    "ferron.ratelimit.zone",
                    TraceAttributeValue::String(zone_id.label().to_string()),
                );
                sa.insert(
                    "ferron.ratelimit.key_type",
                    TraceAttributeValue::String(key_type_label(&config.key).to_string()),
                );
                sa.insert(
                    "ferron.ratelimit.limit",
                    TraceAttributeValue::I64(config.rate as i64),
                );
                let log_fields = custom_access_log_fields(ctx);
                log_fields.insert(
                    "ferron.ratelimit.result".into(),
                    CustomAccessLogField::String("allowed".into()),
                );
                log_fields.insert(
                    "ferron.ratelimit.zone".into(),
                    CustomAccessLogField::String(zone_id.label().to_string()),
                );
            }
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
            StageConstraint::Before("cache".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("rate_limit"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if let Some(response) = self.engine.check_rate_limits(ctx).await {
            ctx.res = Some(response);
            return Ok(false); // response is ready
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
    use std::collections::HashMap as StdHashMap;

    fn make_test_context(
        remote_address: &str,
        config: Option<LayeredConfiguration>,
    ) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri("/path")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        let mut ctx = HttpContext::default();
        ctx.req = Some(req);
        ctx.events = CompositeEventSink::new(Vec::new());
        ctx.configuration = config.unwrap_or_default();
        ctx.encrypted = false;
        ctx.local_address = Some("0.0.0.0:80".parse().unwrap());
        ctx.remote_address = Some(remote_address.parse().unwrap());
        ctx
    }

    fn make_rate_limit_config(rate: u64, burst: u64) -> LayeredConfiguration {
        let mut inner_directives = FxHashMap::default();
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

        let mut directives = FxHashMap::default();
        directives.insert(
            "rate_limit".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(inner_directives),
                    matchers: FxHashMap::default(),
                    span: None,
                }),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.add_layer(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: FxHashMap::default(),
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
}