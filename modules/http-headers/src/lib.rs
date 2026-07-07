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
    Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue, TraceAttributeValue,
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
                ferron_core::log_error!("Failed to parse headers config: {e}");
                return Ok(true);
            }
        };

        // Handle CORS preflight
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
                    cors::build_preflight_response(cors, origin, request_method, request_headers);
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

        Ok(true)
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let config = match config::parse_headers_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(()),
            Err(e) => {
                ferron_core::log_error!("Failed to apply response headers: {e}");
                return Ok(());
            }
        };

        // Pre-resolve all header values before borrowing ctx.res mutably
        let resolved_headers: Vec<(usize, String)> = config
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

        // Collect CORS context
        let origin = ctx
            .req
            .as_ref()
            .and_then(|r| r.headers().get("origin").and_then(|v| v.to_str().ok()))
            .unwrap_or("")
            .to_string();
        let request_method = ctx
            .req
            .as_ref()
            .map(|r| r.method().as_str().to_string())
            .unwrap_or_default();
        let request_headers = ctx
            .req
            .as_ref()
            .and_then(|r| {
                r.headers()
                    .get("access-control-request-headers")
                    .and_then(|v| v.to_str().ok())
            })
            .map(String::from);

        // Set fallback response (404 Not Found default error page)
        if ctx.res.is_none() {
            ctx.res = Some(HttpResponse::BuiltinError(404, None))
        }

        // Apply header actions and CORS to the response
        if let Some(HttpResponse::Custom(ref mut response)) = ctx.res {
            let headers = response.headers_mut();

            // Apply custom header actions using pre-resolved values
            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in config.header_actions.iter().enumerate() {
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

            if let Some(cors) = config.cors.as_ref() {
                cors::apply_cors_headers(
                    headers,
                    cors,
                    &origin,
                    &request_method,
                    request_headers.as_deref(),
                );
            }
        } else if let Some(HttpResponse::BuiltinError(_, ref mut maybe_headers)) = ctx.res {
            let headers = maybe_headers.get_or_insert_with(http::HeaderMap::new);

            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in config.header_actions.iter().enumerate() {
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

            if let Some(cors) = config.cors.as_ref() {
                cors::apply_cors_headers(headers, cors, &origin, &request_method, None);
            }
        }

        let set_count = config
            .header_actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    config::HeaderAction::Append(_, _) | config::HeaderAction::Replace(_, _)
                )
            })
            .count();
        let unset_count = config
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
