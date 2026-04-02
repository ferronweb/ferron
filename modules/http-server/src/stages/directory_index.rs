//! Directory index stage

use std::io;

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpFileContext, HttpResponse};

pub struct DirectoryIndexStage;

impl Default for DirectoryIndexStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpFileContext> for DirectoryIndexStage {
    fn name(&self) -> &str {
        "directory_index"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("static_file".to_string())]
    }

    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        if ctx.path_info.is_some() || !ctx.metadata.is_dir() {
            return Ok(true);
        }

        let index_path = ctx.file_path.join("index.html");
        let metadata = match vibeio::fs::metadata(&index_path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(PipelineError::custom(error.to_string())),
        };
        let canonical_path = vibeio::fs::canonicalize(&index_path)
            .await
            .map_err(|error| PipelineError::custom(error.to_string()))?;
        if !canonical_path.starts_with(&ctx.file_path) {
            ctx.http.res = Some(HttpResponse::BuiltinError(403, None));
            return Ok(false);
        }

        ctx.file_path = canonical_path;
        ctx.metadata = metadata;
        Ok(true)
    }
}
