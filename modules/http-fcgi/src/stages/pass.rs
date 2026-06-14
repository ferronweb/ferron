use std::sync::Arc;
use std::time::Instant;

use ferron_core::config::ServerConfigurationBlockBuilder;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use http::Response;
use http_body_util::BodyExt;
use tokio::io::AsyncReadExt;

use crate::client::{ClientError, FcgiClient};
use crate::config::FcgiConfiguration;
use crate::util::{SendWrapBody, TrackedBody};

pub struct FcgiPassStage {
    client: Arc<FcgiClient>,
}

impl FcgiPassStage {
    pub fn new(client: Arc<FcgiClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for FcgiPassStage {
    fn name(&self) -> &str {
        "fcgi_pass"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![
            ferron_core::StageConstraint::Before("reverse_proxy".to_string()),
            ferron_core::StageConstraint::After("forward_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|b| b.has_directive("fcgi") || b.has_directive("fcgi_php"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // -- check if FastCGI is applicable
        let Some(config) = FcgiConfiguration::from_http_ctx(ctx) else {
            // FastCGI not configured
            return Ok(true);
        };

        if !config.pass {
            // Not pass

            if ctx.configuration.get_entry("index", false).is_none() {
                // Inject default index extensions
                let mut index_inject_ext = vec![
                    "index.html".to_string(),
                    "index.htm".to_string(),
                    "index.xhtml".to_string(),
                ];
                if config.extensions.contains(".cgi") {
                    index_inject_ext.insert(0, "index.cgi".to_string());
                }
                if config.extensions.contains(".php") {
                    index_inject_ext.insert(0, "index.php".to_string());
                }
                if index_inject_ext.len() > 3 {
                    let block = ServerConfigurationBlockBuilder::new()
                        .directive_str("index", index_inject_ext)
                        .build();
                    ctx.configuration.add_layer(Arc::new(block));
                }
            }

            return Ok(true);
        }

        let Some(mut request) = ctx.req.take() else {
            // Request struct not found
            return Ok(true);
        };

        // -- set environment variables --

        // Remove "Proxy" header from the request to prevent "httpoxy" vulnerability
        request
            .headers_mut()
            .remove(http::header::HeaderName::from_static("proxy"));

        // Inject trace context into the request environment
        if let Some(tc) = ctx.get::<ferron_http::trace_context::TraceContextKey>() {
            ferron_http::trace_context::inject_trace_headers(request.headers_mut(), tc);
        }

        let original_request_uri = ctx.original_uri.as_ref().unwrap_or(request.uri());
        let mut env_builder = cegla_fcgi::client::CgiBuilder::new();

        if let Some(auth_user) = ctx.auth_user.as_deref() {
            let authorization_type = if let Some(authorization_value) = request
                .headers()
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
            {
                let mut authorization_value_split = authorization_value.split_whitespace();
                authorization_value_split
                    .next()
                    .map(|authorization_type| authorization_type.to_string())
            } else {
                None
            };
            env_builder = env_builder.auth(authorization_type, auth_user.to_string());
        }

        if let Some(server_administrator_email) = ctx
            .configuration
            .get_value("admin_email", true)
            .and_then(|v| v.as_string_with_interpolations(ctx))
        {
            env_builder = env_builder.server_admin(server_administrator_email);
        }

        if ctx.encrypted {
            env_builder = env_builder.https();
        }

        env_builder = env_builder
            .server("Ferron".to_string())
            .server_address(ctx.local_address)
            .client_address(ctx.remote_address)
            .request_uri(original_request_uri);

        if let Some(hostname) = ctx.hostname.clone() {
            env_builder = env_builder.server(hostname);
        }

        for (env_var_key, env_var_value) in config.environment {
            env_builder = env_builder.var(env_var_key, env_var_value);
        }

        // -- execute FastCGI --
        let scgi_to_fixed = if let Some(stripped) = config.backend_server.strip_prefix("unix:///") {
            // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
            &format!("unix://ignore/{stripped}")
        } else {
            &config.backend_server
        };

        // Set and get local limit for the connection pool
        if let Some(limit) = config.local_limit {
            self.client.set_local_limit(scgi_to_fixed, limit).await;
        }
        let local_limit = self.client.get_local_limit(scgi_to_fixed).await;

        // -- execute FastCGI --
        let request_start = Instant::now();

        // Get connection from pool
        let mut conn_item = match self
            .client
            .get_connection(scgi_to_fixed, config.keepalive, local_limit)
            .await
        {
            Ok(conn) => conn,
            Err(ClientError::ServiceUnavailable(err)) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: ferron_observability::LogLevel::Error,
                    message: format!("Service unavailable: {err}"),
                    summary: "FastCGI service unavailable".into(),
                    target: "ferron-http-scgi",
                    attributes: vec![(
                        "upstream.address",
                        LogAttributeValue::String(scgi_to_fixed.to_string()),
                    )],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.fcgi.failures",
                    attributes: vec![
                        (
                            "error.type",
                            MetricAttributeValue::StaticStr("service_unavailable"),
                        ),
                        (
                            "ferron.fcgi.backend_url",
                            MetricAttributeValue::String(scgi_to_fixed.to_string()),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "Number of FastCGI requests that failed before a backend response was returned.",
                    ),
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                ctx.res = Some(HttpResponse::BuiltinError(503, None));
                return Ok(false);
            }
            Err(ClientError::Other(err)) => {
                return Err(PipelineError::custom(err.to_string()));
            }
        };

        let conn = conn_item.inner().as_ref().ok_or_else(|| {
            // This would be a rare edge case (see `client.rs`)...
            PipelineError::custom("FastCGI connection is not available")
        })?;
        let (response, mut stderr) = conn
            .send_request(request, env_builder)
            .await
            .map_err(|e| PipelineError::custom(e.to_string()))?;

        let events = ctx.events.clone();
        vibeio::spawn(async move {
            let mut stderr_string = String::new();
            stderr
                .read_to_string(&mut stderr_string)
                .await
                .unwrap_or_default();
            let stderr_string_trimmed = stderr_string.trim();
            if !stderr_string_trimmed.is_empty() {
                events.emit(Event::Log(LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!("There were FastCGI errors: {stderr_string_trimmed}"),
                    summary: "FastCGI errors on stderr".into(),
                    target: "ferron-http-fcgi",
                    attributes: vec![(
                        "error.message",
                        LogAttributeValue::String(stderr_string_trimmed.to_string()),
                    )],
                    trace_context: None,
                }));
                events.emit(Event::Metric(MetricEvent {
                    name: "ferron.fcgi.stderr_errors",
                    attributes: vec![],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{error}"),
                    description: Some(
                        "Number of FastCGI requests that produced non-empty stderr output.",
                    ),
                    trace_context: None,
                }));
            }
        });

        if !config.keepalive {
            // Remove connection from pool
            conn_item.inner_mut().take();
        }

        let (parts, body) = response.into_parts();
        let response = Response::from_parts(
            parts,
            TrackedBody::new(SendWrapBody::new(body), conn_item).boxed_unsync(),
        );

        // FastCGI response
        ctx.res = Some(HttpResponse::Custom(response));

        let upstream_duration = request_start.elapsed().as_secs_f64();
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.fcgi.upstream.duration",
            attributes: vec![(
                "ferron.fcgi.backend_url",
                MetricAttributeValue::String(scgi_to_fixed.to_string()),
            )],
            ty: MetricType::Histogram(None),
            value: MetricValue::F64(upstream_duration),
            unit: Some("s"),
            description: Some("Duration of FastCGI upstream request processing."),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        }));
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.fcgi.requests",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Number of FastCGI requests processed."),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        }));

        Ok(false)
    }
}
