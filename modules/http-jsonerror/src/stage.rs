//! JSON error response pipeline stage.
//!
//! Generates RFC 9457 Problem Details or simple JSON error bodies
//! for HTTP 4xx/5xx error responses.

use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::StageConstraint;
use ferron_http::HttpErrorContext;
use http::{header, HeaderValue, Response};
use http_body_util::{BodyExt, Full};

use crate::config::{JsonErrorConfig, JsonErrorFormat};

/// Status code descriptions matching the built-in HTML error pages.
fn status_description(status: u16) -> &'static str {
    match status {
        400 => "The request was invalid.",
        401 => "Authentication is required to access the resource.",
        402 => "Payment is required to access the resource.",
        403 => "You're not authorized to access this resource.",
        404 => "The requested resource wasn't found.",
        405 => "The request method is not allowed for this resource.",
        406 => "The server cannot provide a response in an acceptable format.",
        407 => "Proxy authentication is required.",
        408 => "The request took too long and timed out.",
        409 => "There's a conflict with the current state of the server.",
        410 => "The requested resource has been permanently removed.",
        411 => "The request must include a Content-Length header.",
        412 => "The request doesn't meet the server's preconditions.",
        413 => "The request is too large for the server to process.",
        414 => "The requested URL is too long.",
        415 => "The server doesn't support the request's media type.",
        416 => "The requested content range is invalid or unavailable.",
        417 => "The expectation in the Expect header couldn't be met.",
        418 => "This server refuses to make coffee!",
        421 => "The request was directed to the wrong server.",
        422 => "The server couldn't process the provided content.",
        423 => "The requested resource is locked.",
        424 => "The request failed due to a dependency on another failed request.",
        425 => "The server refuses to process a request that might be replayed.",
        426 => "The client must upgrade its protocol to proceed.",
        428 => "A precondition is required for this request.",
        429 => "Too many requests were sent in a short period.",
        431 => "The request headers are too large.",
        451 => "Access to this resource is restricted due to legal reasons.",
        500 => "The server encountered an unexpected error.",
        501 => "The server doesn't support the requested functionality.",
        502 => "The server, acting as a gateway, received an invalid response.",
        503 => "The server is temporarily unavailable. Try again later.",
        504 => "The server, acting as a gateway, timed out waiting for a response.",
        505 => "The HTTP version used in the request isn't supported.",
        506 => "The Variant header caused a content negotiation loop.",
        507 => "The server lacks sufficient storage to complete the request.",
        508 => "The server detected an infinite loop while processing the request.",
        509 => "Bandwidth limit exceeded on the server.",
        510 => "The server requires an extended HTTP request.",
        511 => "Authentication is required to access the network.",
        _ => "No description found for the status code.",
    }
}

/// Pipeline stage that generates JSON error responses.
pub struct JsonErrorStage;

impl Default for JsonErrorStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpErrorContext> for JsonErrorStage {
    #[inline]
    fn name(&self) -> &str {
        "json_error"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("error_page".to_string())]
    }

    #[inline]
    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else { return false };
        c.has_directive("json_errors")
    }

    async fn run(&self, ctx: &mut HttpErrorContext) -> Result<bool, PipelineError> {
        if ctx.res.is_some() {
            return Ok(true);
        }

        let config = JsonErrorConfig::from_config(&ctx.configuration);
        if !config.enabled {
            return Ok(true);
        }

        let status = ctx.error_code;
        let description = status_description(status);
        let trace_id = if config.trace_id {
            ctx.trace_context.as_ref().map(|tc| tc.trace_id.as_str())
        } else {
            None
        };

        let body = match config.format {
            JsonErrorFormat::Problem => build_problem_json(status, description, &config, trace_id)
                .map_err(|e| PipelineError::custom(e.to_string()))?,
            JsonErrorFormat::Simple => build_simple_json(status, description, trace_id)
                .map_err(|e| PipelineError::custom(e.to_string()))?,
        };

        let content_type = match config.format {
            JsonErrorFormat::Problem => "application/problem+json",
            JsonErrorFormat::Simple => "application/json",
        };

        let bytes = Bytes::from(body);
        let mut builder = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
            .header(header::CONTENT_LENGTH, bytes.len());

        if let Some(ref headers) = ctx.headers {
            for (name, value) in headers.iter() {
                builder = builder.header(name.clone(), value.clone());
            }
        }

        let response = builder
            .body(Full::new(bytes).map_err(|e| match e {}).boxed_unsync())
            .map_err(|e| PipelineError::custom(e.to_string()))?;

        ctx.res = Some(response);
        Ok(false)
    }
}

#[inline]
fn build_problem_json(
    status: u16,
    description: &str,
    config: &JsonErrorConfig,
    trace_id: Option<&str>,
) -> Result<String, serde_json::Error> {
    let type_uri = config.type_uri.replace("{status}", &status.to_string());

    let reason = http::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason().map(|r| r.to_string()))
        .unwrap_or_else(|| "Unknown".to_string());

    let mut parts = serde_json::Map::default();
    parts.insert("type".into(), type_uri.into());
    parts.insert("title".into(), reason.into());
    parts.insert("status".into(), status.into());
    parts.insert("detail".into(), description.into());

    if let Some(tid) = trace_id {
        parts.insert("trace_id".into(), tid.into());
    }

    serde_json::to_string(&parts)
}

#[inline]
fn build_simple_json(
    status: u16,
    description: &str,
    trace_id: Option<&str>,
) -> Result<String, serde_json::Error> {
    let reason = http::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason().map(|r| r.to_string()))
        .unwrap_or_else(|| "Unknown".to_string());

    let mut parts = serde_json::Map::default();
    parts.insert("error".into(), reason.into());
    parts.insert("status".into(), status.into());
    parts.insert("detail".into(), description.into());

    if let Some(tid) = trace_id {
        parts.insert("trace_id".into(), tid.into());
    }

    serde_json::to_string(&parts)
}
