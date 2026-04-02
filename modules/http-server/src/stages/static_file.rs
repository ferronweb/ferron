//! Static file stage

use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpFileContext, HttpResponse};
use http::{Method, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};

pub struct StaticFileStage;

impl Default for StaticFileStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpFileContext> for StaticFileStage {
    #[inline]
    fn name(&self) -> &str {
        "static_file"
    }

    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        let Some(request) = ctx.http.req.as_ref() else {
            return Ok(true);
        };
        if ctx.path_info.is_some() || !ctx.metadata.is_file() {
            return Ok(true);
        }

        let method = request.method().clone();
        match method {
            Method::GET => {
                let body = match vibeio::fs::read(&ctx.file_path).await {
                    Ok(body) => body,
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        ctx.http.res = Some(HttpResponse::BuiltinError(403, None));
                        return Ok(false);
                    }
                    Err(error) => return Err(PipelineError::custom(error.to_string())),
                };
                ctx.http.res = Some(HttpResponse::Custom(build_file_response(
                    ctx.metadata.len(),
                    Full::new(Bytes::from(body))
                        .map_err(|e| match e {})
                        .boxed_unsync(),
                )));
                Ok(false)
            }
            Method::HEAD => {
                ctx.http.res = Some(HttpResponse::Custom(build_file_response(
                    ctx.metadata.len(),
                    Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync(),
                )));
                Ok(false)
            }
            _ => Ok(true),
        }
    }
}

fn build_file_response(
    content_length: u64,
    body: http_body_util::combinators::UnsyncBoxBody<Bytes, io::Error>,
) -> Response<http_body_util::combinators::UnsyncBoxBody<Bytes, io::Error>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_LENGTH, content_length.to_string())
        .header(http::header::CONTENT_TYPE, "application/octet-stream")
        .body(body)
        .expect("failed to build file response")
}
