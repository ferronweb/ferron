//! 404 Not Found stage

use async_trait::async_trait;
use ferron_common::pipeline::{PipelineError, Stage};
use http::Response;

use crate::context::HttpContext;

pub struct NotFoundStage;

impl Default for NotFoundStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Stage<HttpContext> for NotFoundStage {
    fn name(&self) -> &str {
        "not_found"
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if ctx.res.body().is_empty() {
            ctx.res = Response::builder()
                .status(404)
                .body(b"Not Found".to_vec())
                .unwrap();
        }
        Ok(true)
    }
}
