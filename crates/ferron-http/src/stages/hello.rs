//! Hello handler stage

use async_trait::async_trait;
use ferron_common::pipeline::{PipelineError, Stage};
use ferron_common::StageConstraint;
use http::Response;

use crate::context::HttpContext;

pub struct HelloStage;

impl Default for HelloStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Stage<HttpContext> for HelloStage {
    fn name(&self) -> &str {
        "hello"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("not_found".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if ctx.req.uri().path() == "/" {
            ctx.res = Response::builder()
                .status(200)
                .body(b"Hello from Ferron 3".to_vec())
                .expect("Failed to build 200 response");
        }
        Ok(true)
    }
}
