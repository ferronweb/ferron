//! 404 Not Found stage

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::HttpErrorContext;
use http_body_util::BodyExt;

pub struct NotFoundStage;

impl Default for NotFoundStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpErrorContext> for NotFoundStage {
    #[inline]
    fn name(&self) -> &str {
        "not_found"
    }

    async fn run(&self, ctx: &mut HttpErrorContext) -> Result<bool, PipelineError> {
        if ctx.error_code == 404 && ctx.res.is_none() {
            ctx.res = Some(
                http::Response::builder()
                    .status(404)
                    .body(
                        http_body_util::Full::<bytes::Bytes>::new(bytes::Bytes::from_static(
                            b"404 Not Found - custom handler",
                        ))
                        .map_err(|e| match e {})
                        .boxed_unsync(),
                    )
                    .map_err(|e| PipelineError::custom(e.to_string()))?,
            );
            return Ok(false);
        }
        Ok(true)
    }
}
