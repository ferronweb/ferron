//! HTTP headers and CORS module for Ferron.
//!
//! Provides pipeline stages for:
//! - Response header manipulation (add/replace/remove with interpolation)
//! - CORS preflight handling (OPTIONS) and response header injection

mod config;
mod cors;
mod validator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::PipelineError;
use ferron_core::registry::RegistryBuilder;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue, TraceAttributeValue,
};
use http_body_util::BodyExt;

pub use validator::HttpHeadersConfigurationValidator;

/// Stage for applying response headers and handling CORS preflight requests.
#[derive(Default)]
pub struct HeadersStage {
    _private: (),
}

impl HeadersStage {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

#[async_trait::async_trait(?Send)]
impl ferron_core::pipeline::Stage<HttpContext> for HeadersStage {
    fn name(&self) -> &str {
        "headers"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![
            ferron_core::StageConstraint::Before("reverse_proxy".to_string()),
            ferron_core::StageConstraint::Before("static_file".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else { return false };
        c.has_directive("header") || c.has_directive("cors")
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match config::parse_headers_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(true),
            Err(e) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("Failed to apply response headers: {e}"),
                    summary: "Failed to apply response headers".into(),
                    target: "ferron-http-headers".into(),
                    attributes: vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    control_plane_metadata: None,
                }));
                return Ok(true);
            }
        };

        if let (Some(cors), Some(req)) = (config.cors.as_ref(), ctx.req.as_ref()) {
            let origin = req
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let request_method = req
                .headers()
                .get("access-control-request-method")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let request_headers = req
                .headers()
                .get("access-control-request-headers")
                .and_then(|v| v.to_str().ok());

            if cors::is_preflight(req.method(), req.headers()) {
                let response =
                    cors::build_preflight_response(cors, origin, request_method, request_headers)
                        .map_err(|e| PipelineError::Custom(e.to_string()))?;
                let response = response.map(|b| b.map_err(|e| match e {}).boxed_unsync());
                let status_code = response.status().as_u16();
                ctx.res = Some(HttpResponse::Custom(response));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.http.server.cors_preflights",
                    attributes: vec![(
                        "http.response.status_code",
                        MetricAttributeValue::I64(status_code as i64),
                    )],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "Number of CORS preflight requests handled before the rest of the HTTP pipeline.",
                    ),
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    control_plane_metadata: None,
                }));
                return Ok(false);
            }
        }

        let headers_ctx = HeadersContext {
            config,
            origin: ctx
                .req
                .as_ref()
                .and_then(|r| r.headers().get("origin").and_then(|v| v.to_str().ok()))
                .unwrap_or("")
                .to_string(),
            method: ctx
                .req
                .as_ref()
                .map(|r| r.method().as_str().to_string())
                .unwrap_or_default(),
            headers: ctx
                .req
                .as_ref()
                .and_then(|r| {
                    r.headers()
                        .get("access-control-request-headers")
                        .and_then(|v| v.to_str().ok())
                })
                .map(String::from),
        };
        ctx.extensions.insert::<HeadersContext>(headers_ctx);

        Ok(true)
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let Some(header_ctx) = ctx.extensions.get::<HeadersContext>() else {
            return Ok(());
        };

        // Pre-resolve all header values before borrowing ctx.res mutably
        let resolved_headers: Vec<(usize, String)> = header_ctx
            .config
            .header_actions
            .iter()
            .enumerate()
            .filter_map(|(i, action)| {
                let value = match action {
                    config::HeaderAction::Append(_, v) | config::HeaderAction::Replace(_, v) => {
                        Some(v.clone())
                    }
                    config::HeaderAction::Remove(_) => None,
                };
                value.map(|v| (i, v))
            })
            .collect();

        // Set fallback response (404 Not Found default error page)
        if ctx.res.is_none() {
            ctx.res = Some(HttpResponse::BuiltinError(404, None))
        }

        // Apply header actions and CORS to the response
        if let Some(HttpResponse::Custom(ref mut response)) = ctx.res {
            let headers = response.headers_mut();

            // Apply custom header actions using pre-resolved values
            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in header_ctx.config.header_actions.iter().enumerate() {
                match action {
                    config::HeaderAction::Remove(name) => {
                        headers.remove(name);
                    }
                    config::HeaderAction::Replace(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.insert(name.clone(), val);
                            }
                        }
                    }
                    config::HeaderAction::Append(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.append(name.clone(), val);
                            }
                        }
                    }
                }
            }

            if let Some(cors) = header_ctx.config.cors.as_ref() {
                cors::apply_cors_headers(
                    headers,
                    cors,
                    &header_ctx.origin,
                    &header_ctx.method,
                    header_ctx.headers.as_deref(),
                );
            }
        } else if let Some(HttpResponse::BuiltinError(_, ref mut maybe_headers)) = ctx.res {
            let headers = maybe_headers.get_or_insert_with(http::HeaderMap::new);

            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in header_ctx.config.header_actions.iter().enumerate() {
                match action {
                    config::HeaderAction::Remove(name) => {
                        headers.remove(name);
                    }
                    config::HeaderAction::Replace(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.insert(name.clone(), val);
                            }
                        }
                    }
                    config::HeaderAction::Append(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.append(name.clone(), val);
                            }
                        }
                    }
                }
            }

            if let Some(cors) = header_ctx.config.cors.as_ref() {
                cors::apply_cors_headers(
                    headers,
                    cors,
                    &header_ctx.origin,
                    &header_ctx.method,
                    None,
                );
            }
        }

        let set_count = header_ctx
            .config
            .header_actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    config::HeaderAction::Append(_, _) | config::HeaderAction::Replace(_, _)
                )
            })
            .count();
        let unset_count = header_ctx
            .config
            .header_actions
            .iter()
            .filter(|a| matches!(a, config::HeaderAction::Remove(_)))
            .count();
        ctx.get_span_attributes().insert(
            "ferron.headers.set",
            TraceAttributeValue::I64(set_count as i64),
        );
        ctx.get_span_attributes().insert(
            "ferron.headers.unset",
            TraceAttributeValue::I64(unset_count as i64),
        );

        Ok(())
    }
}

/// Module loader for the HTTP headers module.
///
/// Registers:
/// - Global configuration validator for headers/CORS directives
/// - Pipeline stage: HeadersStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct HttpHeadersModuleLoader;

impl ModuleLoader for HttpHeadersModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "header",
                    usage: "header <name> <value> | header +<name> <value> | header -<name>",
                    description: "This directive sets, appends to, or removes HTTP response headers with support for variable interpolation.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "cors",
                    usage: "cors { ... }",
                    description: "This directive configures CORS with origins, methods, headers, credentials, max_age, and expose_headers settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_cors")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "origins",
                    usage: "origins <origin>...",
                    description: "This directive sets allowed CORS origins.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            )
            .register(
                Directive {
                    name: "methods",
                    usage: "methods <method>...",
                    description: "This directive sets allowed CORS HTTP methods.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            )
            .register(
                Directive {
                    name: "headers",
                    usage: "headers <header>...",
                    description: "This directive sets allowed CORS request headers.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            )
            .register(
                Directive {
                    name: "credentials",
                    usage: "credentials [bool]",
                    description: "This directive enables CORS credentials (Access-Control-Allow-Credentials).",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            )
            .register(
                Directive {
                    name: "max_age",
                    usage: "max_age <duration>",
                    description: "This directive sets the CORS preflight cache duration.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            )
            .register(
                Directive {
                    name: "expose_headers",
                    usage: "expose_headers <header>...",
                    description: "This directive sets CORS exposed response headers (Access-Control-Expose-Headers).",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cors"),
            );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpHeadersConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(HttpHeadersConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HeadersStage::new()))
    }
}

struct HeadersContext {
    pub config: crate::config::HeadersConfig,
    pub origin: String,
    pub method: String,
    pub headers: Option<String>, // access-control-request-headers
}

impl typemap_rev::TypeMapKey for HeadersContext {
    type Value = HeadersContext;
}
