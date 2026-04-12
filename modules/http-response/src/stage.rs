//! HTTP response control pipeline stage.
//!
//! Evaluates `abort`, `block`/`allow`, and `status` directives
//! to short-circuit requests before content handling.

use std::sync::Arc;

use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use http::header::LOCATION;
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};
#[cfg(test)]
use rustc_hash::FxHashMap;

use crate::config::{ResponseConfig, StatusRule};

const LOG_TARGET: &str = "ferron-http-response";

/// Shared state for the http-response module.
pub struct ResponseEngine {
    // Currently empty — config is read per-request from LayeredConfiguration.
    // This struct exists for future shared-state needs (e.g., compiled regex cache).
}

impl ResponseEngine {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for ResponseEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Pipeline stage that enforces abort, IP access control, and custom status codes.
pub struct HttpResponseStage {
    _engine: Arc<ResponseEngine>,
}

impl HttpResponseStage {
    pub fn new(engine: Arc<ResponseEngine>) -> Self {
        Self { _engine: engine }
    }

    fn evaluate_abort(ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = ResponseConfig::from_config(&ctx.configuration);
        if config.abort.abort {
            ctx.res = Some(HttpResponse::Abort);
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.response.aborted",
                attributes: vec![],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Connections aborted via the abort directive."),
            }));
            return Ok(false);
        }
        Ok(true)
    }

    fn evaluate_ip_access(ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = ResponseConfig::from_config(&ctx.configuration);
        if config.ip_access.is_blocked(ctx.remote_address.ip()) {
            ctx.res = Some(HttpResponse::BuiltinError(403, None));
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.response.ip_blocked",
                attributes: vec![],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some(
                    "Connections blocked via block/allow directives (raw IPs not included).",
                ),
            }));
            return Ok(false);
        }
        Ok(true)
    }

    fn evaluate_status_rules(ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = ResponseConfig::from_http_context(ctx);
        if config.status_rules.is_empty() {
            return Ok(true);
        }

        let request_path = match &ctx.routing_uri {
            Some(uri) => uri.path().to_string(),
            None => ctx
                .req
                .as_ref()
                .map(|req| req.uri().path().to_string())
                .unwrap_or_default(),
        };

        for rule in &config.status_rules {
            if !Self::rule_matches(rule, &request_path) {
                continue;
            }

            // Rule matched — build response
            let response = Self::build_response(rule)?;
            ctx.res = Some(response);

            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.response.status_rule_matched",
                attributes: vec![
                    (
                        "http.response.status_code",
                        MetricAttributeValue::I64(rule.status_code as i64),
                    ),
                    (
                        "ferron.rule_id",
                        MetricAttributeValue::String(rule.status_code.to_string()),
                    ),
                ],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Custom status codes returned via status directives."),
            }));

            return Ok(false);
        }

        Ok(true)
    }

    /// Check whether a status rule matches the given request path.
    fn rule_matches(rule: &StatusRule, path: &str) -> bool {
        match (&rule.url, &rule.regex) {
            // Neither url nor regex: match everything
            (None, None) => true,
            // Only url: exact match
            (Some(url), None) => url == path,
            // Only regex: regex match
            (None, Some(regex)) => regex.is_match(path).unwrap_or(false),
            // Both url and regex: both must match
            (Some(url), Some(regex)) => url == path && regex.is_match(path).unwrap_or(false),
        }
    }

    /// Build an HttpResponse from a matched status rule.
    fn build_response(rule: &StatusRule) -> Result<HttpResponse, PipelineError> {
        let status_code = StatusCode::from_u16(rule.status_code)
            .map_err(|e| PipelineError::custom(e.to_string()))?;

        // Handle redirects (3xx with location)
        if (300..400).contains(&rule.status_code) {
            if let Some(location) = &rule.location {
                let mut headers = HeaderMap::new();
                headers.insert(
                    LOCATION,
                    HeaderValue::from_str(location)
                        .map_err(|e| PipelineError::custom(e.to_string()))?,
                );

                if let Some(body) = &rule.body {
                    let body_bytes = Bytes::from(body.clone());
                    let content_length = body_bytes.len();
                    headers.insert(
                        http::header::CONTENT_LENGTH,
                        HeaderValue::from_str(&content_length.to_string())
                            .expect("content length header value should be valid"),
                    );

                    let response = Response::builder()
                        .status(status_code)
                        .body(
                            Full::new(body_bytes)
                                .map_err(|e: std::convert::Infallible| match e {})
                                .boxed_unsync(),
                        )
                        .map_err(|e| PipelineError::custom(e.to_string()))?;

                    return Ok(HttpResponse::Custom(response));
                } else {
                    let response = Response::builder()
                        .status(status_code)
                        .body(
                            Empty::new()
                                .map_err(|e: std::convert::Infallible| match e {})
                                .boxed_unsync(),
                        )
                        .map_err(|e| PipelineError::custom(e.to_string()))?;

                    return Ok(HttpResponse::Custom(response));
                }
            }
        }

        // Non-redirect or redirect without location: return status + optional body
        if let Some(body) = &rule.body {
            let body_bytes = Bytes::from(body.clone());
            let content_length = body_bytes.len();
            let mut headers = HeaderMap::new();
            headers.insert(
                http::header::CONTENT_LENGTH,
                HeaderValue::from_str(&content_length.to_string())
                    .expect("content length header value should be valid"),
            );

            let response = Response::builder()
                .status(status_code)
                .body(
                    Full::new(body_bytes)
                        .map_err(|e: std::convert::Infallible| match e {})
                        .boxed_unsync(),
                )
                .map_err(|e| PipelineError::custom(e.to_string()))?;

            Ok(HttpResponse::Custom(response))
        } else {
            Ok(HttpResponse::BuiltinError(rule.status_code, None))
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for HttpResponseStage {
    fn name(&self) -> &str {
        "http_response"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            // Run before content-handling stages
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else { return false };
        c.has_directive("abort")
            || c.has_directive("block")
            || c.has_directive("allow")
            || c.has_directive("status")
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // 1. Check abort directive — if true, immediately abort
        if !Self::evaluate_abort(ctx)? {
            return Ok(false);
        }

        // 2. Check IP access control
        if !Self::evaluate_ip_access(ctx)? {
            return Ok(false);
        }

        // 3. Evaluate status rules
        if !Self::evaluate_status_rules(ctx)? {
            return Ok(false);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use std::collections::HashMap;
    use typemap_rev::TypeMap;

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_test_context_with_request(path: &str) -> HttpContext {
        let req: ferron_http::HttpRequest = Request::builder()
            .uri(path)
            .body(
                Empty::<Bytes>::new()
                    .map_err(|e: std::convert::Infallible| match e {})
                    .boxed_unsync(),
            )
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "192.168.1.50:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_config_with_layer(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> LayeredConfiguration {
        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));
        config
    }

    fn make_child_block(
        directives: Vec<(&str, Vec<ServerConfigurationDirectiveEntry>)>,
    ) -> ServerConfigurationBlock {
        let mut d = HashMap::new();
        for (name, entries) in directives {
            d.insert(name.to_string(), entries);
        }
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[tokio::test]
    async fn abort_stops_pipeline() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut directives = HashMap::new();
        directives.insert(
            "abort".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "abort should stop the pipeline");
        assert!(matches!(ctx.res, Some(HttpResponse::Abort)));
    }

    #[tokio::test]
    async fn no_abort_passes_through() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut directives = HashMap::new();
        directives.insert(
            "abort".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(false)],
                children: None,
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "non-abort should continue the pipeline");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn blocked_ip_gets_403() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut directives = HashMap::new();
        directives.insert(
            "block".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("192.168.1.0/24")],
                children: None,
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "blocked IP should stop the pipeline");
        assert!(matches!(
            ctx.res,
            Some(HttpResponse::BuiltinError(403, None))
        ));
    }

    #[tokio::test]
    async fn allowed_ip_passes() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut directives = HashMap::new();
        directives.insert(
            "allow".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("192.168.1.0/24")],
                children: None,
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "allowed IP should continue the pipeline");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn status_without_body_returns_builtin_error() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut directives = HashMap::new();
        directives.insert(
            "status".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(503)],
                children: None,
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "status rule should stop the pipeline");
        assert!(matches!(
            ctx.res,
            Some(HttpResponse::BuiltinError(503, None))
        ));
    }

    #[tokio::test]
    async fn status_with_body_returns_custom_response() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let child = make_child_block(vec![(
            "body",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("Service Unavailable")],
                children: None,
                span: None,
            }],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "status".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(503)],
                children: Some(child),
                span: None,
            }],
        );

        let mut ctx = make_test_context_with_request("/");
        ctx.configuration = make_config_with_layer(directives);

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result);
        if let Some(HttpResponse::Custom(response)) = ctx.res {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        } else {
            panic!("Expected custom response with body");
        }
    }

    #[tokio::test]
    async fn status_url_match_exact() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let child = make_child_block(vec![(
            "url",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("/secret")],
                children: None,
                span: None,
            }],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "status".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(403)],
                children: Some(child),
                span: None,
            }],
        );

        // Should match
        let mut ctx = make_test_context_with_request("/secret");
        ctx.configuration = make_config_with_layer(directives.clone());
        assert!(!stage.run(&mut ctx).await.unwrap());

        // Should not match
        let mut ctx2 = make_test_context_with_request("/other");
        ctx2.configuration = make_config_with_layer(directives);
        assert!(stage.run(&mut ctx2).await.unwrap());
    }

    #[tokio::test]
    async fn status_url_match_uses_routing_uri_not_req_uri() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let child = make_child_block(vec![(
            "url",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("/canonical/secret")],
                children: None,
                span: None,
            }],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "status".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(403)],
                children: Some(child),
                span: None,
            }],
        );

        // req.uri() is "/rewritten/path" but routing_uri is "/canonical/secret"
        // The rule should match based on routing_uri.
        let mut ctx = make_test_context_with_request("/rewritten/path");
        ctx.routing_uri = Some("/canonical/secret".parse().unwrap());
        ctx.configuration = make_config_with_layer(directives);
        assert!(!stage.run(&mut ctx).await.unwrap());
    }

    #[tokio::test]
    async fn status_regex_match() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let child = make_child_block(vec![(
            "regex",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("^/api/v[0-9]+")],
                children: None,
                span: None,
            }],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "status".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(410)],
                children: Some(child),
                span: None,
            }],
        );

        // Should match
        let mut ctx = make_test_context_with_request("/api/v2/users");
        ctx.configuration = make_config_with_layer(directives.clone());
        assert!(!stage.run(&mut ctx).await.unwrap());

        // Should not match
        let mut ctx2 = make_test_context_with_request("/api/users");
        ctx2.configuration = make_config_with_layer(directives);
        assert!(stage.run(&mut ctx2).await.unwrap());
    }

    #[tokio::test]
    async fn no_rules_is_noop() {
        let engine = Arc::new(ResponseEngine::new());
        let stage = HttpResponseStage::new(engine);

        let mut ctx = make_test_context_with_request("/");
        // Empty configuration

        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
    }

    #[test]
    fn rule_matches_without_url_or_regex() {
        let rule = StatusRule {
            status_code: 403,
            url: None,
            regex: None,
            location: None,
            body: None,
        };
        // Should match any path
        assert!(HttpResponseStage::rule_matches(&rule, "/anything"));
    }

    #[test]
    fn rule_matches_url_exact() {
        let rule = StatusRule {
            status_code: 404,
            url: Some("/missing".to_string()),
            regex: None,
            location: None,
            body: None,
        };
        assert!(HttpResponseStage::rule_matches(&rule, "/missing"));
        assert!(!HttpResponseStage::rule_matches(&rule, "/other"));
    }
}

/// Pipeline stage that sends 103 Early Hints responses.
///
/// This stage evaluates the `early_hints` directive and sends a 103 Early Hints
/// response with the configured `Link` headers before the final response is ready.
/// The stage never short-circuits the pipeline — it always returns `Ok(true)`.
pub struct EarlyHintsStage;

impl Default for EarlyHintsStage {
    fn default() -> Self {
        Self::new()
    }
}

impl EarlyHintsStage {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for EarlyHintsStage {
    fn name(&self) -> &str {
        "early_hints"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("http_response".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else { return false };
        c.has_directive("early_hints")
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = crate::config::ResponseConfig::from_config(&ctx.configuration);
        if config.early_hints.links.is_empty() {
            return Ok(true);
        }

        // Build Link headers
        let mut headers = HeaderMap::new();
        for link in &config.early_hints.links {
            if let Ok(value) = HeaderValue::from_str(link) {
                headers.insert(http::header::LINK, value);
            }
        }

        if headers.is_empty() {
            return Ok(true);
        }

        // Attempt to send 103 Early Hints. If it fails (e.g., not supported on
        // this connection), log a warning and continue the pipeline normally.
        if let Some(req) = ctx.req.as_mut() {
            if let Err(e) = vibeio_http::send_early_hints(req, headers).await {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    target: LOG_TARGET,
                    message: format!("Failed to send 103 Early Hints: {e}"),
                }));
            }
        }

        Ok(true)
    }
}
