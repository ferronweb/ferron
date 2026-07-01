//! HTTP request stage for abuse ban checks.

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::abuse::{get_global_abuse_recorder, AbuseEvent, AbuseEventType};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{LogAttributeValue, TraceAttributeValue};
use http::HeaderMap;
use std::sync::Arc;

use crate::config::parse_abuse_protection_config;
use crate::registry::{AbuseRegistry, AbuseRegistryConfig};

/// HTTP pipeline stage that checks for IP bans and rejects banned clients.
pub struct AbuseProtectionStage {
    registry: Arc<AbuseRegistry>,
}

impl AbuseProtectionStage {
    pub fn new(registry: Arc<AbuseRegistry>) -> Self {
        Self { registry }
    }

    #[inline]
    async fn run_abuse_protection(&self, context: &mut HttpContext) -> Result<bool, PipelineError> {
        let Some(config) = parse_abuse_protection_config(&context.configuration) else {
            // No abuse protection config, skip this stage
            return Ok(true);
        };

        context
            .extensions
            .insert::<AbuseRegistryConfig>(config.registry_config.clone());

        // Get the client IP address from the context
        let client_ip = context.remote_address.ip();

        // Check if this IP is banned
        if let Some(remaining) = self
            .registry
            .ban_time_remaining(client_ip, &config.registry_config)
        {
            let remaining_secs = remaining.as_secs();
            let reason = self
                .registry
                .ban_reason(client_ip, &config.registry_config)
                .unwrap_or_else(|| "IP address temporarily banned".to_string());

            // Build error response with Retry-After header
            let mut headers = HeaderMap::new();
            headers.insert(
                http::header::RETRY_AFTER,
                http::HeaderValue::from_str(&remaining_secs.to_string())
                    .expect("retry-after value should be valid"),
            );

            context.res = Some(HttpResponse::BuiltinError(403, Some(headers)));

            // Log ban rejection
            context.events.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Debug,
                    message: format!("Ban rejection: IP {} - {}", client_ip, reason),
                    summary: "Ban rejection".into(),
                    target: "ferron-http-abuseban",
                    attributes: vec![
                        (
                            "client.address",
                            LogAttributeValue::String(client_ip.to_string()),
                        ),
                        (
                            "ferron.abuseban.reason",
                            LogAttributeValue::String(reason.clone()),
                        ),
                        (
                            "ferron.abuseban.remaining_secs",
                            LogAttributeValue::I64(remaining_secs as i64),
                        ),
                    ],
                    trace_context: ferron_http::trace_context::current_event_trace_context(context),
                },
            ));

            // Emit metric for ban rejection
            context.events.emit(ferron_observability::Event::Metric(
                ferron_observability::MetricEvent {
                    name: "ferron.abuseban.rejected",
                    attributes: vec![(
                        "ferron.abuseban.reason",
                        ferron_observability::MetricAttributeValue::String(reason.clone()),
                    )],
                    ty: ferron_observability::MetricType::Counter,
                    value: ferron_observability::MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests rejected due to IP ban."),
                    trace_context: ferron_http::trace_context::current_event_trace_context(context),
                },
            ));

            {
                let sa = context.get_span_attributes();
                sa.insert(
                    "ferron.abuseban.action",
                    TraceAttributeValue::String("rejected".to_string()),
                );
                sa.insert(
                    "ferron.abuseban.reason",
                    TraceAttributeValue::String(reason.clone()),
                );
                sa.insert(
                    "ferron.abuseban.remaining_secs",
                    TraceAttributeValue::I64(remaining_secs as i64),
                );
                sa.insert(
                    "error.type",
                    TraceAttributeValue::String("ip_banned".to_string()),
                );
            }
            return Ok(false); // Stop pipeline execution
        }

        {
            let sa = context.get_span_attributes();
            sa.insert(
                "ferron.abuseban.action",
                TraceAttributeValue::String("skip".to_string()),
            );
        }
        Ok(true) // Continue pipeline execution
    }

    #[inline]
    async fn run_abuse_event(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let Some(abuse_event_type) = ctx
            .configuration
            .get_value("abuse_event", true)
            .and_then(|v| v.as_str())
        else {
            return Ok(true);
        };

        if let Some(recorder) = get_global_abuse_recorder() {
            let abuse_event = AbuseEvent::new(
                AbuseEventType::Custom,
                ctx.remote_address.ip(),
                format!("Custom abuse event: {abuse_event_type}"),
                40,
            );
            recorder.record_event(&abuse_event, ctx);
        }

        Ok(true)
    }

    #[inline]
    async fn run_error_rate_check(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let config = match ctx.extensions.get::<AbuseRegistryConfig>() {
            Some(config) => config.clone(),
            None => return Ok(()),
        };

        if config.error_rate_thresholds.is_empty() {
            return Ok(());
        }

        // Extract status code from the response
        let status_code = match &ctx.res {
            Some(HttpResponse::Custom(response)) => response.status().as_u16(),
            Some(HttpResponse::BuiltinError(status, _)) => *status,
            None => 404, // implicit 404 when no response is set
            _ => return Ok(()),
        };

        // Check if this status code matches any configured error rate threshold
        let matches_any = config
            .error_rate_thresholds
            .iter()
            .any(|t| t.status_codes.contains(&status_code));

        if !matches_any {
            return Ok(());
        }

        if let Some(recorder) = get_global_abuse_recorder() {
            let abuse_event = AbuseEvent::with_status_code(
                AbuseEventType::ErrorRate,
                ctx.remote_address.ip(),
                format!("Error rate: {status_code} responses"),
                40,
                status_code,
            );
            recorder.record_event(&abuse_event, ctx);
        }

        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for AbuseProtectionStage {
    fn name(&self) -> &str {
        "abuse_protection"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run early, after client IP is resolved but before rate limiting
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("cache".to_string()),
            StageConstraint::Before("rate_limit".to_string()),
            StageConstraint::Before("basicauth".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config
            .is_some_and(|c| c.has_directive("abuse_protection") || c.has_directive("abuse_event"))
    }

    async fn run(&self, context: &mut HttpContext) -> Result<bool, PipelineError> {
        let to_continue = self.run_abuse_protection(context).await?;
        if to_continue {
            self.run_abuse_event(context).await
        } else {
            Ok(to_continue)
        }
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        self.run_error_rate_check(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;
    use std::net::SocketAddr;

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
    use typemap_rev::TypeMap;

    use crate::registry::{AbuseRegistry, AbuseRegistryConfig, EventThreshold};
    use ferron_http::abuse::{AbuseEvent, AbuseEventType};

    fn make_context(remote_addr: SocketAddr, config: LayeredConfiguration) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri("/test")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: config,
            hostname: Some("example.com".to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: remote_addr,
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_config_with_abuse() -> LayeredConfiguration {
        let mut inner = StdHashMap::new();
        inner.insert(
            "enabled".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(true, None)],
                children: None,
                span: None,
            }],
        );

        let mut outer = StdHashMap::new();
        outer.insert(
            "abuse_protection".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(inner),
                    matchers: StdHashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(outer),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    fn make_config_without_abuse() -> LayeredConfiguration {
        LayeredConfiguration::new()
    }

    #[tokio::test]
    async fn allows_request_when_not_banned() {
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry);

        let config = make_config_with_abuse();
        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let mut ctx = make_context(addr, config);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "non-banned IP should pass through");
        assert!(ctx.res.is_none(), "no response should be set");
    }

    #[tokio::test]
    async fn rejects_banned_ip_with_403() {
        let registry_config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                1,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry.clone());

        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            addr.ip(),
            "Test ban".into(),
            50,
        );
        registry.record_event(&event, &registry_config);

        let config = make_config_with_abuse();
        let mut ctx = make_context(addr, config);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "banned IP should be rejected");
        assert!(ctx.res.is_some(), "response should be set");

        if let Some(ferron_http::HttpResponse::BuiltinError(status, _)) = &ctx.res {
            assert_eq!(*status, 403, "should return 403");
        } else {
            panic!("expected BuiltinError(403, _)");
        }
    }

    #[tokio::test]
    async fn includes_retry_after_header() {
        let registry_config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 3600,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                1,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry.clone());

        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            addr.ip(),
            "Retry-After test".into(),
            50,
        );
        registry.record_event(&event, &registry_config);

        let config = make_config_with_abuse();
        let mut ctx = make_context(addr, config);

        stage.run(&mut ctx).await.unwrap();

        if let Some(ferron_http::HttpResponse::BuiltinError(_, headers)) = &ctx.res {
            assert!(headers.is_some(), "should have headers");
            let headers = headers.as_ref().unwrap();
            let retry_after = headers.get(http::header::RETRY_AFTER);
            assert!(retry_after.is_some(), "should have Retry-After header");
            let value = retry_after.unwrap().to_str().unwrap();
            let secs: u64 = value.parse().unwrap();
            assert!(secs > 0, "Retry-After should be positive");
        } else {
            panic!("expected BuiltinError with headers");
        }
    }

    #[tokio::test]
    async fn not_applicable_without_abuse_protection_block() {
        let registry = Arc::new(AbuseRegistry::default());
        let stage = AbuseProtectionStage::new(registry);

        let config = make_config_without_abuse();
        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let mut ctx = make_context(addr, config);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "should pass through when not banned");
    }

    #[tokio::test]
    async fn multiple_ips_independent() {
        let registry_config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                1,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry.clone());

        let banned_addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let allowed_addr: SocketAddr = "192.0.2.2:12345".parse().unwrap();

        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            banned_addr.ip(),
            "Ban this".into(),
            50,
        );
        registry.record_event(&event, &registry_config);

        let config = make_config_with_abuse();

        // Banned IP rejected
        let mut ctx1 = make_context(banned_addr, config.clone());
        let result1 = stage.run(&mut ctx1).await.unwrap();
        assert!(!result1);

        // Clean IP allowed
        let mut ctx2 = make_context(allowed_addr, config);
        let result2 = stage.run(&mut ctx2).await.unwrap();
        assert!(result2);
    }

    #[test]
    fn is_applicable_checks_directive() {
        let registry = Arc::new(AbuseRegistry::default());
        let stage = AbuseProtectionStage::new(registry);

        // Config with abuse_protection block
        let block_with = Some(ServerConfigurationBlock {
            directives: {
                let mut m = StdHashMap::new();
                m.insert("abuse_protection".to_string(), vec![]);
                Arc::new(m)
            },
            matchers: StdHashMap::new(),
            span: None,
        });
        assert!(stage.is_applicable(block_with.as_ref()));

        // Config without abuse_protection block
        let block_without = Some(ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::new()),
            matchers: StdHashMap::new(),
            span: None,
        });
        assert!(!stage.is_applicable(block_without.as_ref()));

        // No config at all
        assert!(!stage.is_applicable(None));
    }

    #[tokio::test]
    async fn run_inverse_records_error_rate_event() {
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry.clone());

        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![crate::registry::ErrorRateThresholdConfig::new(
                2,
                60,
                vec![404],
            )],
            allowlist: Vec::new(),
        };

        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let mut ctx = make_context(addr, make_config_with_abuse());
        ctx.extensions.insert::<AbuseRegistryConfig>(config.clone());

        // Set a 404 response
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(404, None));

        // Without global recorder, run_inverse should not panic and should not record events
        stage.run_inverse(&mut ctx).await.unwrap();
        assert!(
            !registry.is_banned(addr.ip(), &config),
            "should not be banned without global recorder"
        );

        // But the registry's record_error_rate_event should work directly
        let event = ferron_http::abuse::AbuseEvent::with_status_code(
            ferron_http::abuse::AbuseEventType::ErrorRate,
            addr.ip(),
            "Error rate: 404 responses".into(),
            40,
            404,
        );
        assert_eq!(
            registry.record_event(&event, &config),
            ferron_http::abuse::EventResult::Recorded
        );
        assert_eq!(
            registry.record_event(&event, &config),
            ferron_http::abuse::EventResult::BanTriggered
        );
        assert!(registry.is_banned(addr.ip(), &config));
    }

    #[tokio::test]
    async fn run_inverse_ignores_non_matching_status_code() {
        let registry = Arc::new(AbuseRegistry::new());
        let stage = AbuseProtectionStage::new(registry.clone());

        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![crate::registry::ErrorRateThresholdConfig::new(
                2,
                60,
                vec![404],
            )],
            allowlist: Vec::new(),
        };

        let addr: SocketAddr = "192.0.2.1:12345".parse().unwrap();
        let mut ctx = make_context(addr, make_config_with_abuse());
        ctx.extensions.insert::<AbuseRegistryConfig>(config.clone());

        // Set a 200 response (not matching 404)
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(200, None));

        for _ in 0..5 {
            stage.run_inverse(&mut ctx).await.unwrap();
        }

        assert!(
            !registry.is_banned(addr.ip(), &config),
            "should not be banned for non-matching status codes"
        );
    }
}
