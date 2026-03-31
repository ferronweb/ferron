//! Hello handler stage

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use http::Response;
use http_body_util::{BodyExt, Full};

use crate::context::{HttpContext, HttpResponse};

pub struct HelloStage;

impl Default for HelloStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HelloStage {
    fn name(&self) -> &str {
        "hello"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("not_found".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        if ctx.req.as_ref().is_some_and(|r| r.uri().path() == "/") {
            ctx.res = Some(HttpResponse::Custom(
                Response::builder()
                    .status(200)
                    .body(
                        Full::new(Bytes::from_static(b"Hello from Ferron 3"))
                            .map_err(|e| match e {})
                            .boxed_unsync(),
                    )
                    .expect("Failed to build 200 response"),
            ));
            return Ok(false);
        }
        Ok(true)
    }
}
