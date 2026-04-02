//! Logging stage

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::HttpContext;
use ferron_observability::{AccessEvent, Event};

pub struct LoggingStage;

impl Default for LoggingStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for LoggingStage {
    #[inline]
    fn name(&self) -> &str {
        "logging"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("hello".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if let Some(req) = &ctx.req {
            ctx.events.emit(Event::Access(AccessEvent {
                message: format!("--> {}", req.uri().path()),
            }));
        }
        Ok(true)
    }
}
