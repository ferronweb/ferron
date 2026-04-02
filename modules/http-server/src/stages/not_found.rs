//! 404 Not Found stage

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpContext, HttpResponse};

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

    async fn run(&self, _ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        if ctx.res.is_none() {
            ctx.res = Some(HttpResponse::BuiltinError(404, None));
        }
        Ok(())
    }
}
