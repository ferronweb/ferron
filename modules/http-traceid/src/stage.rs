use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::TraceAttributeValue;
use http::{HeaderMap, HeaderValue};

use crate::config::TraceIdConfig;

/// Pipeline stage for trace ID headers.
#[derive(Default)]
pub struct HttpTraceIdStage;

impl HttpTraceIdStage {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpTraceIdStage {
    fn name(&self) -> &str {
        "trace_id"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("rewrite".to_string()),
            StageConstraint::Before("rate_limit".to_string()),
            StageConstraint::Before("basicauth".to_string()),
            StageConstraint::Before("cache".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else { return false };
        c.has_directive("trace_id_header")
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let Some(trace_id_config) = TraceIdConfig::from_layered_config(&ctx.configuration) else {
            // No configuration, or it's disabled
            return Ok(true);
        };

        if trace_id_config.reflect_request
            && ctx.req.as_ref().is_none_or(|req| {
                req.headers()
                    .get("x-ferron-trace-reflect")
                    .and_then(|h| str::from_utf8(h.as_bytes()).ok())
                    != Some("1")
            })
        {
            // Trace ID reflection not applicable for this request
            return Ok(true);
        }

        // Save the trace ID header configuration in type map, so inverse stage can use it.
        ctx.extensions.insert::<TraceIdConfig>(trace_id_config);

        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let Some(config) = ctx.extensions.remove::<TraceIdConfig>() else {
            // Trace ID header not applicable for this request
            return Ok(());
        };

        // Get trace ID
        let Some(trace_id) = current_event_trace_context(ctx)
            .and_then(|tctx| HeaderValue::from_bytes(&tctx.trace_id).ok())
        else {
            return Ok(());
        };

        if ctx.res.is_none() {
            // In Ferron's pipeline execution, no response = 404 Not Found built-in error
            ctx.res = Some(HttpResponse::BuiltinError(404, None));
        }

        match &mut ctx.res {
            Some(HttpResponse::Custom(resp)) => {
                resp.headers_mut().insert(config.header_name, trace_id);
            }
            Some(HttpResponse::BuiltinError(_, headers)) => {
                headers
                    .get_or_insert(HeaderMap::default())
                    .insert(config.header_name, trace_id);
            }
            _ => {}
        }

        ctx.get_span_attributes()
            .insert("ferron.traceid.injected", TraceAttributeValue::Bool(true));

        Ok(())
    }
}
