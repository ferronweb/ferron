//! Logging stage

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::{log_info, StageConstraint};

use crate::context::HttpContext;

pub struct LoggingStage;

impl Default for LoggingStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for LoggingStage {
    fn name(&self) -> &str {
        "logging"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("hello".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if let Some(req) = &ctx.req {
            log_info!("--> {}", req.uri().path());
        }
        Ok(true)
    }
}
