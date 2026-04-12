//! Pipeline stage for HTTP request and response buffering.
//!
//! Provides configurable buffering for incoming request bodies and outgoing
//! response bodies to protect backend servers and control memory usage.

use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use futures_util::stream::{self, StreamExt};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use typemap_rev::TypeMapKey;

/// TypeMap key for passing buffer configuration from run() to run_inverse().
struct BufferStateKey;

impl TypeMapKey for BufferStateKey {
    type Value = BufferState;
}

/// State passed between run() and run_inverse() for response buffering.
struct BufferState {
    response_buffer_size: Option<usize>,
}

/// Pipeline stage for HTTP request and response buffering.
#[derive(Default)]
pub struct HttpBufferStage;

impl HttpBufferStage {
    pub fn new() -> Self {
        Self
    }

    /// Buffer the request body up to max_size bytes.
    ///
    /// Collects frames from the request body until either:
    /// - The buffer limit is reached (remaining stream is preserved)
    /// - The body is fully consumed
    /// - A non-data frame (trailers) is encountered
    async fn buffer_request_body(
        ctx: &mut HttpContext,
        max_size: usize,
    ) -> Result<(), PipelineError> {
        let Some(req) = ctx.req.take() else {
            return Ok(());
        };

        let (parts, mut body) = req.into_parts();
        let mut buffered_frames = Vec::new();
        let mut collected_size = 0usize;

        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| PipelineError::custom(e.to_string()))?;

            if let Some(data) = frame.data_ref() {
                let frame_size = data.len();
                // Check if adding this frame would exceed the limit
                if collected_size + frame_size > max_size {
                    // Add the frame (we'll go slightly over, which is acceptable)
                    buffered_frames.push(Frame::data(data.clone()));
                    break;
                }
                collected_size += frame_size;
                buffered_frames.push(Frame::data(data.clone()));
            } else {
                // Non-data frame (e.g., trailers), preserve it and stop
                buffered_frames.push(frame);
                break;
            }
        }

        // Chain buffered frames with the remaining body stream
        let prefix_stream = stream::iter(buffered_frames.into_iter().map(Ok));
        let chained = prefix_stream.chain(BodyStream::new(body));
        let new_body = StreamBody::new(chained).boxed_unsync();

        ctx.req = Some(http::Request::from_parts(parts, new_body));
        Ok(())
    }

    /// Buffer the response body up to max_size bytes.
    ///
    /// Similar to request buffering but operates on the response.
    async fn buffer_response_body(
        response: http::Response<UnsyncBoxBody<Bytes, io::Error>>,
        max_size: usize,
    ) -> Result<http::Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
        let (parts, mut body) = response.into_parts();
        let mut buffered_frames = Vec::new();
        let mut collected_size = 0usize;

        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| PipelineError::custom(e.to_string()))?;

            if let Some(data) = frame.data_ref() {
                let frame_size = data.len();
                if collected_size + frame_size > max_size {
                    buffered_frames.push(Frame::data(data.clone()));
                    break;
                }
                collected_size += frame_size;
                buffered_frames.push(Frame::data(data.clone()));
            } else {
                buffered_frames.push(frame);
                break;
            }
        }

        let prefix_stream = stream::iter(buffered_frames.into_iter().map(Ok));
        let chained = prefix_stream.chain(BodyStream::new(body));
        let new_body = StreamBody::new(chained).boxed_unsync();

        Ok(http::Response::from_parts(parts, new_body))
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpBufferStage {
    fn name(&self) -> &str {
        "buffer"
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
        c.has_directive("buffer_request") || c.has_directive("buffer_response")
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // Parse request buffer size
        let request_buffer_size = ctx
            .configuration
            .get_value("buffer_request", true)
            .and_then(|v| v.as_number())
            .map(|n| n as usize);

        // Parse response buffer size and store in state for run_inverse()
        let response_buffer_size = ctx
            .configuration
            .get_value("buffer_response", true)
            .and_then(|v| v.as_number())
            .map(|n| n as usize);

        // Apply request buffering
        if let Some(max_size) = request_buffer_size {
            if max_size > 0 {
                Self::buffer_request_body(ctx, max_size).await?;
            }
        }

        // Store response buffer config for run_inverse()
        ctx.extensions.insert::<BufferStateKey>(BufferState {
            response_buffer_size,
        });

        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        // Retrieve buffer state from extensions
        let Some(state) = ctx.extensions.remove::<BufferStateKey>() else {
            return Ok(());
        };

        let Some(max_size) = state.response_buffer_size else {
            return Ok(());
        };

        if max_size == 0 {
            return Ok(());
        }

        // Only buffer if we have a custom response
        let response = match ctx.res.take() {
            Some(HttpResponse::Custom(resp)) => resp,
            Some(res) => {
                // Put back the response if it's not a Custom variant
                ctx.res = Some(res);
                return Ok(());
            }
            None => return Ok(()),
        };

        // Apply response buffering
        let buffered_response = Self::buffer_response_body(response, max_size).await?;
        ctx.res = Some(HttpResponse::Custom(buffered_response));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_http::HttpContext;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Full};
    use rustc_hash::FxHashMap;
    use std::collections::HashMap;
    use std::sync::Arc;
    use typemap_rev::TypeMap;

    fn make_layered_config(directives: Vec<(&str, i64)>) -> LayeredConfiguration {
        let mut d = HashMap::new();
        for (name, value) in directives {
            d.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::Number(value, None)],
                    children: None,
                    span: None,
                }],
            );
        }
        let block = Arc::new(ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: HashMap::new(),
            span: None,
        });
        let mut config = LayeredConfiguration::new();
        config.add_layer(block);
        config
    }

    fn make_context(
        req: Option<http::Request<UnsyncBoxBody<Bytes, io::Error>>>,
        directives: Vec<(&str, i64)>,
    ) -> HttpContext {
        HttpContext {
            req,
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: make_layered_config(directives),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "127.0.0.1:80".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_request_with_body(body: Bytes) -> http::Request<UnsyncBoxBody<Bytes, io::Error>> {
        Request::builder()
            .uri("/test")
            .body(
                Full::new(body)
                    .map_err(|e: std::convert::Infallible| match e {})
                    .boxed_unsync(),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn test_request_buffering_within_limit() {
        let body_data = Bytes::from("Hello, World!");
        let req = make_request_with_body(body_data.clone());
        let mut ctx = make_context(Some(req), vec![("buffer_request", 1024)]);

        let stage = HttpBufferStage::new();
        let result = stage.run(&mut ctx).await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify the request is still present
        assert!(ctx.req.is_some());

        // Collect the body to verify it's intact
        let req = ctx.req.take().unwrap();
        let (parts, body) = req.into_parts();
        assert_eq!(parts.uri, "/test");

        let collected = body.collect().await.unwrap().to_bytes();
        assert_eq!(collected, body_data);
    }

    #[tokio::test]
    async fn test_request_buffering_exceeds_limit() {
        let body_data = Bytes::from("A".repeat(1000));
        let req = make_request_with_body(body_data.clone());
        let mut ctx = make_context(Some(req), vec![("buffer_request", 500)]);

        let stage = HttpBufferStage::new();
        let result = stage.run(&mut ctx).await;
        assert!(result.is_ok());

        // Verify the request is still present
        assert!(ctx.req.is_some());

        // Collect the entire body to verify it's preserved
        let req = ctx.req.take().unwrap();
        let collected = req.into_body().collect().await.unwrap().to_bytes();
        // The full body should still be there (buffering doesn't truncate)
        assert_eq!(collected.len(), body_data.len());
    }

    #[tokio::test]
    async fn test_response_buffering_within_limit() {
        let body_data = Bytes::from("Response body");
        let response = http::Response::builder()
            .status(200)
            .body(
                Full::new(body_data.clone())
                    .map_err(|e: std::convert::Infallible| match e {})
                    .boxed_unsync(),
            )
            .unwrap();

        let mut ctx = make_context(None, vec![("buffer_response", 1024)]);
        ctx.res = Some(HttpResponse::Custom(response));

        // Simulate run() storing state
        ctx.extensions.insert::<BufferStateKey>(BufferState {
            response_buffer_size: Some(1024),
        });

        let stage = HttpBufferStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        // Verify the response is still present
        assert!(ctx.res.is_some());

        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            let collected = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(collected, body_data);
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_no_buffering_when_not_configured() {
        let body_data = Bytes::from("Test");
        let req = make_request_with_body(body_data.clone());
        let mut ctx = make_context(Some(req), vec![]);

        let stage = HttpBufferStage::new();
        let result = stage.run(&mut ctx).await;
        assert!(result.is_ok());

        // Buffer state should still be inserted (with None values)
        let state = ctx.extensions.remove::<BufferStateKey>();
        assert!(state.is_some());
        assert!(state.unwrap().response_buffer_size.is_none());
    }

    #[tokio::test]
    async fn test_response_buffering_skipped_for_non_custom_response() {
        let mut ctx = make_context(None, vec![("buffer_response", 1024)]);
        ctx.res = Some(HttpResponse::BuiltinError(404, None));

        ctx.extensions.insert::<BufferStateKey>(BufferState {
            response_buffer_size: Some(1024),
        });

        let stage = HttpBufferStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        // Response should still be BuiltinError
        assert!(matches!(ctx.res, Some(HttpResponse::BuiltinError(404, _))));
    }
}
