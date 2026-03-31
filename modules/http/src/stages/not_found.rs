//! 404 Not Found stage

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use http::Response;
use http_body_util::{BodyExt, Full};

use crate::context::{HttpContext, HttpResponse};

pub struct NotFoundStage;

impl Default for NotFoundStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for NotFoundStage {
    fn name(&self) -> &str {
        "not_found"
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        ctx.res = Some(HttpResponse::Custom(
            Response::builder()
                .status(404)
                .body(
                    Full::new(Bytes::from_static(b"Not Found"))
                        .map_err(|e| match e {})
                        .boxed_unsync(),
                )
                .expect("Failed to build 404 response"),
        ));
        Ok(true)
    }
}
