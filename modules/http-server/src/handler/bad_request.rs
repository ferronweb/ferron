use std::borrow::Cow;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::Pipeline;
use ferron_http::HttpErrorContext;
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue, TraceAttributeValue, TraceEvent,
};
use http::{HeaderValue, Response};

use super::observability::next_span_key;
use super::request_utils::{
    builtin_error_response, convert_control_plane_span_links, emit_error, execute_error_pipeline,
};
use super::ResponseBody;

pub async fn bad_request_handler(
    is_timeout: bool,
    local_address: SocketAddr,
    remote_address: SocketAddr,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    events: CompositeEventSink,
    control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
    control_plane_span_links: Option<Arc<Vec<ferron_observability::control_plane::SpanLinkConfig>>>,
) -> Result<Response<ResponseBody>, io::Error> {
    let status_code = if is_timeout { 408 } else { 400 };
    ferron_core::admin::ADMIN_METRICS
        .requests_total
        .fetch_add(1, Ordering::Relaxed);
    let request_span_key = if let Some(request_span_key) =
        events.has_trace_sinks().then(|| next_span_key("request"))
    {
        let emitted = events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(request_span_key.clone()),
            name: Cow::Borrowed("ferron.request"),
            parent: None,
            trace_context: None,
            builder_attributes: vec![],
            attributes: vec![(
                "ferron.http.request.stage",
                TraceAttributeValue::StaticStr("pre_handler"),
            )],
            links: convert_control_plane_span_links(&control_plane_span_links),
            control_plane_metadata: control_plane_metadata.clone(),
        }));
        emitted.then_some(request_span_key)
    } else {
        None
    };
    let error_type = if is_timeout { "timeout" } else { "bad_request" };
    emit_error(
        &events,
        format!(
            "{} request error: {}",
            status_code,
            if is_timeout {
                "request timed out"
            } else {
                "bad request"
            }
        ),
        vec![
            (
                "error.type",
                LogAttributeValue::String(error_type.to_string()),
            ),
            (
                "client.address",
                LogAttributeValue::String(remote_address.ip().to_canonical().to_string()),
            ),
            (
                "server.address",
                LogAttributeValue::String(local_address.ip().to_string()),
            ),
        ],
    );
    events.emit(Event::Metric(MetricEvent {
        name: "ferron.http.server.pre_handler_request_count",
        attributes: vec![
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
            (
                "ferron.http.request.stage",
                MetricAttributeValue::StaticStr("pre_handler"),
            ),
            (
                "error.type",
                MetricAttributeValue::String(status_code.to_string()),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some(
            "Number of malformed or timed-out HTTP requests rejected before request handling.",
        ),
        trace_context: None,
    }));
    let mut response = if let Some(response) = execute_error_pipeline(
        error_pipeline.as_ref(),
        status_code,
        None,
        LayeredConfiguration::default(),
        None,
        &events,
        request_span_key.as_deref(),
        control_plane_metadata.clone(),
    )
    .await
    {
        response
    } else {
        builtin_error_response(status_code, None, None, None)
    };
    response
        .headers_mut()
        .insert(http::header::SERVER, HeaderValue::from_static("Ferron"));
    if let Some(request_span_key) = request_span_key {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(request_span_key),
            name: Cow::Borrowed("ferron.request"),
            error: Some(format!("HTTP error {}", status_code)),
            attributes: vec![(
                "http.response.status_code",
                TraceAttributeValue::I64(status_code as i64),
            )],
            control_plane_metadata: control_plane_metadata.clone(),
        }));
    }
    Ok(response)
}
