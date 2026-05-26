use std::borrow::Cow;
use std::time::Duration;

use ferron_core::pipeline::Pipeline;
use ferron_http::trace_context;
use ferron_http::{HttpContext, HttpFileContext, HttpResponse};
use ferron_observability::{CompositeEventSink, Event, Parent, TraceAttributeValue, TraceEvent};

use super::observability::PerStageSpanHooks;

use super::file_pipeline::{
    execute_http_file_pipeline, strip_matched_path_prefix, FilePipelineExecutionError,
};
use super::request_utils::{emit_error_with_trace, emit_warn_with_trace};

pub async fn execute_pipeline_stages(
    ctx: &mut HttpContext,
    pipeline: &Pipeline<HttpContext>,
    file_pipeline: &Pipeline<HttpFileContext>,
    events: &CompositeEventSink,
    log_prefix: &str,
    path_segments: &[String],
    request_span_key: Option<&str>,
    timeout_duration: Option<Duration>,
) {
    let has_traces = events.has_trace_sinks();
    let pipeline_span_key =
        request_span_key.map(|_| super::observability::next_span_key("pipeline"));
    let log_trace_context = ctx
        .get::<trace_context::TraceContextKey>()
        .map(trace_context::to_event_trace_context);

    // Start pipeline execution span
    if let (true, Some(request_span_key), Some(pipeline_span_key)) =
        (has_traces, request_span_key, pipeline_span_key.as_ref())
    {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(pipeline_span_key.clone()),
            name: Cow::Borrowed("ferron.pipeline.execute"),
            parent: Some(Parent::ByKey(request_span_key.to_string())),
            trace_context: None,
            attributes: vec![(
                "ferron.pipeline.log_prefix",
                TraceAttributeValue::String(log_prefix.to_string()),
            )],
        }));
    }

    // Remove the base URL if path segments were matched
    if !path_segments.is_empty() {
        if let Some(req) = ctx.req.take() {
            let (mut parts, body) = req.into_parts();
            let mut uri_parts = parts.uri.into_parts();
            if let Some(path_and_query) = uri_parts.path_and_query {
                uri_parts.path_and_query =
                    strip_matched_path_prefix(&path_and_query, path_segments.len());
                if uri_parts.path_and_query.is_none() {
                    ctx.res = Some(HttpResponse::BuiltinError(400, None));
                    return;
                }
            }
            let Ok(new_uri) = http::Uri::from_parts(uri_parts) else {
                ctx.res = Some(HttpResponse::BuiltinError(400, None));
                return;
            };
            parts.uri = new_uri;
            ctx.req = Some(http::Request::from_parts(parts, body));
        }
    }

    let instant = std::time::Instant::now();

    // Per-stage span hooks — emit StartSpan/EndSpan around each stage
    let mut stage_hooks = PerStageSpanHooks::new(
        events,
        has_traces && pipeline_span_key.is_some(),
        pipeline_span_key.as_deref().unwrap_or(""),
        "http",
    );

    let executed_stages = match if let Some(timeout_duration) =
        timeout_duration.map(|d| d.saturating_sub(instant.elapsed()))
    {
        vibeio::time::timeout(
            timeout_duration,
            pipeline.execute_without_inverse_with_hooks(ctx, &mut stage_hooks),
        )
        .await
    } else {
        Ok(pipeline
            .execute_without_inverse_with_hooks(ctx, &mut stage_hooks)
            .await)
    } {
        Ok(Ok(executed_stages)) => Some(executed_stages),
        Ok(Err(error)) => {
            emit_error_with_trace(
                events,
                format!("{log_prefix}Pipeline execution error: {error}"),
                log_trace_context.clone(),
            );
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
            None
        }
        Err(_) => {
            emit_error_with_trace(
                events,
                format!("{log_prefix}Pipeline execution timeout"),
                log_trace_context.clone(),
            );
            ctx.res = Some(HttpResponse::BuiltinError(408, None));
            None
        }
    };

    if let Some(executed_stages) = executed_stages {
        if ctx.res.is_none() {
            match execute_http_file_pipeline(
                ctx,
                file_pipeline,
                timeout_duration.map(|d| d.saturating_sub(instant.elapsed())),
                pipeline_span_key.as_deref(),
            )
            .await
            {
                Ok(()) => {}
                Err(FilePipelineExecutionError::Forbidden) => {
                    ctx.res = Some(HttpResponse::BuiltinError(403, None));
                }
                Err(FilePipelineExecutionError::BadRequest) => {
                    ctx.res = Some(HttpResponse::BuiltinError(400, None));
                }
                Err(FilePipelineExecutionError::Timeout) => {
                    ctx.res = Some(HttpResponse::BuiltinError(408, None));
                }
                Err(FilePipelineExecutionError::Io(error)) => {
                    emit_error_with_trace(
                        events,
                        format!("{log_prefix}HTTP file resolution error: {error}"),
                        log_trace_context.clone(),
                    );
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
                Err(FilePipelineExecutionError::Pipeline(error)) => {
                    emit_error_with_trace(
                        events,
                        format!("{log_prefix}Pipeline execution error: {error}"),
                        log_trace_context.clone(),
                    );
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
                Err(FilePipelineExecutionError::WebrootNotFound) => {
                    if let Some(webroot) = ctx
                        .configuration
                        .get_value("root", true)
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        emit_warn_with_trace(
                            events,
                            format!("{log_prefix}Webroot not found: {webroot}"),
                            log_trace_context.clone(),
                        );
                    }
                    ctx.res = Some(HttpResponse::BuiltinError(404, None));
                }
            }
        }

        if let Err(error) = pipeline
            .execute_inverse_with_hooks(ctx, executed_stages, &mut stage_hooks)
            .await
        {
            emit_error_with_trace(
                events,
                format!("{log_prefix}Pipeline inverse execution error: {error}"),
                log_trace_context,
            );
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
        }
    }

    // End pipeline execution span
    if let Some(pipeline_span_key) = pipeline_span_key {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(pipeline_span_key),
            name: Cow::Borrowed("ferron.pipeline.execute"),
            error: ctx.res.as_ref().and_then(|r| match r {
                HttpResponse::BuiltinError(s, _) if *s >= 400 => {
                    Some(format!("builtin error {}", s))
                }
                _ => None,
            }),
            attributes: vec![],
        }));
    }
}
