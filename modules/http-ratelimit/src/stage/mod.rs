//! Rate limiting pipeline stage.
//!
//! Evaluates `rate_limit` configuration rules against each request.
//! If any rule's bucket is exhausted, the request is rejected with a 429
//! (or configured) status code and a `Retry-After` header.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::StageConstraint;
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

use crate::backends::{
    BackendError, MemoryBackend, RateLimitBackend, RateLimitBackendKind, RedisBackend,
};
use crate::config::{
    parse_backend_config, parse_rate_limit_config, resolve_zone_id, BackendConfig, BackendType,
    RateLimitConfig, RateLimitZoneId,
};
use crate::key_extractor::KeyExtractor;

/// Shared rate limit engine that manages backends per scope.
///
/// The engine is created once during module loading and shared across all
/// stage invocations. It maintains one backend per unique
/// (zone, rule fingerprint, backend fingerprint) triple so that hosts in
/// different zones get isolated state and different Redis URLs don't share
/// clients.
pub struct RateLimitEngine {
    /// Backends keyed by (zone_id, rule fingerprint, backend fingerprint).
    backends: Mutex<HashMap<(RateLimitZoneId, String, String), Arc<RateLimitBackendKind>>>,
}

impl RateLimitEngine {
    /// Create a new empty rate limit engine.
    pub fn new() -> Self {
        Self {
            backends: Mutex::new(HashMap::new()),
        }
    }

    /// Fingerprint identifying a rule's bucket parameters.
    fn rule_fingerprint(config: &RateLimitConfig) -> String {
        let key_type = match &config.key {
            KeyExtractor::RemoteAddress => "ip",
            KeyExtractor::Uri => "uri",
            KeyExtractor::Header(name) => name.as_str(),
        };
        format!(
            "cap:{}|rate:{}|ttl:{}|max:{}|key:{}",
            config.rate + config.burst,
            config.rate,
            config.bucket_ttl_secs,
            config.max_buckets,
            key_type
        )
    }

    /// Namespaced key for distributed backends.
    ///
    /// Memory backends are already sharded per rule, so they use the raw
    /// user key. Redis keys embed zone + fingerprint to isolate rules sharing
    /// one server.
    fn namespaced_key(zone_id: &RateLimitZoneId, fingerprint: &str, user_key: &str) -> String {
        format!("{}:{}:{}", zone_id.label(), fingerprint, user_key)
    }

    /// Get or create a backend for the given rule, zone and backend config.
    fn get_or_create_backend(
        &self,
        config: &RateLimitConfig,
        zone_id: &RateLimitZoneId,
        backend_config: &BackendConfig,
    ) -> Result<Arc<RateLimitBackendKind>, BackendError> {
        let fingerprint = Self::rule_fingerprint(config);
        let backend_fp = backend_config.fingerprint();
        let key = (zone_id.clone(), fingerprint.clone(), backend_fp);

        // Fast path under lock: clone Arc if present.
        {
            let backends = self.backends.lock();
            if let Some(existing) = backends.get(&key) {
                return Ok(existing.clone());
            }
        }

        let backend: RateLimitBackendKind = match backend_config.backend_type {
            BackendType::Memory => RateLimitBackendKind::Memory(MemoryBackend::new(
                config.rate + config.burst,
                config.rate as f64,
                config.bucket_ttl_secs,
                config.max_buckets,
            )),
            BackendType::Redis => {
                if backend_config.url.is_empty() {
                    return Err(BackendError::Config(
                        "missing `url` for redis backend".into(),
                    ));
                }
                let redis = RedisBackend::new(
                    &backend_config.url,
                    backend_config.key_prefix.clone(),
                    backend_config.timeout,
                )?;
                RateLimitBackendKind::Redis(redis)
            }
        };

        let mut backends = self.backends.lock();
        // Another task may have inserted while we built the client.
        let entry = backends.entry(key).or_insert_with(|| Arc::new(backend));
        Ok(entry.clone())
    }

    /// Check all rate limit rules against the current request.
    ///
    /// Returns `Some(response)` if any rule is exhausted, or `None` if all rules pass.
    async fn check_rate_limits(&self, ctx: &mut HttpContext) -> Option<HttpResponse> {
        let rules = parse_rate_limit_config(&ctx.configuration);
        if rules.is_empty() {
            return None;
        }

        // Resolve zone and backend once per request. `rate_limit_backend` blocks are
        // inherited like other directives: host/location without its own
        // block inherits the global one; absent means in-memory.
        let zone_id = resolve_zone_id(&ctx.configuration, &ctx.hostname);
        let backend_config = parse_backend_config(&ctx.configuration);
        let backend_label = backend_config.backend_type.label().to_string();

        for config in &rules {
            let key = match config.key.extract(ctx) {
                Some(k) => k,
                None => continue, // Can't extract key, skip this rule...
            };

            let backend = match self.get_or_create_backend(config, &zone_id, &backend_config) {
                Ok(b) => b,
                Err(e) => {
                    // Invalid backend config (e.g. redis without URL).
                    Self::emit_backend_error(ctx, &zone_id, config, &backend_label, &e.to_string());
                    if backend_config.fail_open {
                        Self::emit_allowed(ctx, &zone_id, config, &backend_label, false);
                        continue;
                    }
                    return Some(Self::make_response(config.deny_status, 1.0));
                }
            };

            let fingerprint = Self::rule_fingerprint(config);
            let is_redis = matches!(*backend, RateLimitBackendKind::Redis(_));
            let lookup_key = if is_redis {
                Self::namespaced_key(&zone_id, &fingerprint, &key)
            } else {
                key.clone()
            };
            let capacity = config.rate + config.burst;
            let refill_rate = config.rate as f64;

            // Attempt to consume one token, mapping backend errors.
            let decision = match backend
                .try_consume(&lookup_key, capacity, refill_rate, config.bucket_ttl_secs)
                .await
            {
                Ok(d) => d,
                Err(BackendError::AtCapacity) => {
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
                                MetricAttributeValue::String(
                                    key_type_label(&config.key).to_string(),
                                ),
                            ),
                            (
                                "ferron.ratelimit.backend",
                                MetricAttributeValue::String(backend_label.clone()),
                            ),
                        ],
                        ty: MetricType::Counter,
                        value: MetricValue::U64(1),
                        unit: Some("{request}"),
                        description: Some(
                            "Requests rejected due to rate limit registry at capacity.",
                        ),
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
                            "ferron.ratelimit.backend",
                            TraceAttributeValue::String(backend_label.clone()),
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
                }
                Err(e @ (BackendError::Unavailable(_) | BackendError::Config(_))) => {
                    Self::emit_backend_error(ctx, &zone_id, config, &backend_label, &e.to_string());
                    if backend_config.fail_open {
                        Self::emit_allowed(ctx, &zone_id, config, &backend_label, false);
                        continue;
                    }
                    // Fail-closed: deny. Documented for high-security endpoints.
                    Self::emit_rejected_for_backend_error(ctx, &zone_id, config, &backend_label);
                    return Some(Self::make_response(config.deny_status, 1.0));
                }
            };

            // Attempt to consume one token, with throttle handling.
            // For Redis, throttle is a single sleep + single retry (never a
            // busy loop), per agreed semantics.
            let (allowed, throttled, retry_after) = if config.throttle {
                if decision.allowed {
                    (true, false, 0.0)
                } else if is_redis {
                    let wait = decision.retry_after_secs.clamp(0.0, 30.0);
                    if wait > 0.0 {
                        sleep_for_throttle(std::time::Duration::from_secs_f64(wait)).await;
                    }
                    match backend
                        .try_consume(&lookup_key, capacity, refill_rate, config.bucket_ttl_secs)
                        .await
                    {
                        Ok(second) if second.allowed => (true, true, 0.0),
                        Ok(second) => (false, true, second.retry_after_secs),
                        Err(BackendError::AtCapacity) => {
                            return Some(Self::make_response(config.deny_status, 1.0));
                        }
                        Err(e) => {
                            Self::emit_backend_error(
                                ctx,
                                &zone_id,
                                config,
                                &backend_label,
                                &e.to_string(),
                            );
                            if backend_config.fail_open {
                                Self::emit_allowed(ctx, &zone_id, config, &backend_label, true);
                                continue;
                            }
                            return Some(Self::make_response(config.deny_status, 1.0));
                        }
                    }
                } else {
                    // In-memory: delegate to the bucket's blocking consume.
                    match backend.consume_throttled(&lookup_key).await {
                        Ok(was_throttled) => (true, was_throttled, 0.0),
                        Err(BackendError::AtCapacity) => {
                            return Some(Self::make_response(config.deny_status, 1.0));
                        }
                        Err(e) => {
                            Self::emit_backend_error(
                                ctx,
                                &zone_id,
                                config,
                                &backend_label,
                                &e.to_string(),
                            );
                            if backend_config.fail_open {
                                Self::emit_allowed(ctx, &zone_id, config, &backend_label, true);
                                continue;
                            }
                            return Some(Self::make_response(config.deny_status, 1.0));
                        }
                    }
                }
            } else {
                (decision.allowed, false, decision.retry_after_secs)
            };
            if !allowed {
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
                        (
                            "ferron.ratelimit.backend",
                            MetricAttributeValue::String(backend_label.clone()),
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
                        "ferron.ratelimit.backend",
                        TraceAttributeValue::String(backend_label.clone()),
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
                    (
                        "ferron.ratelimit.backend",
                        MetricAttributeValue::String(backend_label.clone()),
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
                        (
                            "ferron.ratelimit.backend",
                            MetricAttributeValue::String(backend_label.clone()),
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
                    "ferron.ratelimit.backend",
                    TraceAttributeValue::String(backend_label.clone()),
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

    /// Emit `ferron.ratelimit.backend_errors` and a WARN log for Redis failures.
    fn emit_backend_error(
        ctx: &mut HttpContext,
        zone_id: &RateLimitZoneId,
        config: &RateLimitConfig,
        backend_label: &str,
        error: &str,
    ) {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            target: "ferron-http-ratelimit",
            message: format!("Rate limit backend error: {error}"),
            summary: "Rate limit backend error".into(),
            attributes: vec![
                (
                    "ferron.ratelimit.zone",
                    LogAttributeValue::String(zone_id.label().to_string()),
                ),
                (
                    "ferron.ratelimit.backend",
                    LogAttributeValue::String(backend_label.to_string()),
                ),
                (
                    "error.message",
                    LogAttributeValue::String(error.to_string()),
                ),
            ],
            trace_context: current_event_trace_context(ctx),
        }));
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.ratelimit.backend_errors",
            attributes: vec![
                (
                    "ferron.ratelimit.zone",
                    MetricAttributeValue::String(zone_id.label().to_string()),
                ),
                (
                    "ferron.ratelimit.key_type",
                    MetricAttributeValue::String(key_type_label(&config.key).to_string()),
                ),
                (
                    "ferron.ratelimit.backend",
                    MetricAttributeValue::String(backend_label.to_string()),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{error}"),
            description: Some("Rate limit backend errors (e.g. Redis unavailable)."),
            trace_context: current_event_trace_context(ctx),
        }));
        let sa = ctx.get_span_attributes();
        sa.insert(
            "ferron.ratelimit.backend",
            TraceAttributeValue::String(backend_label.to_string()),
        );
        sa.insert(
            "ferron.ratelimit.backend_error",
            TraceAttributeValue::String(error.to_string()),
        );
    }

    /// Emit an `allowed` decision after a fail-open backend error.
    fn emit_allowed(
        ctx: &mut HttpContext,
        zone_id: &RateLimitZoneId,
        config: &RateLimitConfig,
        backend_label: &str,
        throttled: bool,
    ) {
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
                (
                    "ferron.ratelimit.backend",
                    MetricAttributeValue::String(backend_label.to_string()),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Requests that passed rate limiting."),
            trace_context: current_event_trace_context(ctx),
        }));
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
            "ferron.ratelimit.backend",
            TraceAttributeValue::String(backend_label.to_string()),
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

    /// Emit a `rejected` span/metrics for fail-closed backend errors.
    fn emit_rejected_for_backend_error(
        ctx: &mut HttpContext,
        zone_id: &RateLimitZoneId,
        config: &RateLimitConfig,
        backend_label: &str,
    ) {
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
                (
                    "ferron.ratelimit.backend",
                    MetricAttributeValue::String(backend_label.to_string()),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Requests rejected due to rate limit backend errors."),
            trace_context: current_event_trace_context(ctx),
        }));
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
            "ferron.ratelimit.backend",
            TraceAttributeValue::String(backend_label.to_string()),
        );
    }

    fn make_response(status: u16, retry_after_secs: f64) -> HttpResponse {
        let retry_after_value = retry_after_secs.ceil().max(1.0) as u64;

        let mut headers = HeaderMap::new();
        // The value is digits only, so parsing cannot fail in practice;
        // fall back to 1 rather than panicking on the request path.
        if let Ok(value) = HeaderValue::from_str(&retry_after_value.to_string()) {
            headers.insert(http::header::RETRY_AFTER, value);
        }
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

/// Sleep that works on both primary (`zincio`) and secondary/tests (`tokio`).
async fn sleep_for_throttle(duration: std::time::Duration) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::time::sleep(duration).await;
    } else {
        zincio::time::sleep(duration).await;
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
mod tests;
