//! Directory index file resolution stage

use std::io;

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};

pub struct DirectoryIndexStage;

impl Default for DirectoryIndexStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<ferron_http::HttpFileContext> for DirectoryIndexStage {
    #[inline]
    fn name(&self) -> &str {
        "directory_index"
    }

    async fn run(&self, ctx: &mut ferron_http::HttpFileContext) -> Result<bool, PipelineError> {
        // Skip if root is not configured
        if ctx.http.configuration.get_value("root", true).is_none() {
            return Ok(true);
        }

        // Only handle directories
        if ctx.path_info.is_some() || !ctx.metadata.is_dir() {
            return Ok(true);
        }

        // Get configured index files (default: index.html, index.htm, index.xhtml)
        let index_files: Vec<String> = {
            let entries = ctx.http.configuration.get_entries("index", true);
            if entries.is_empty() {
                vec![
                    "index.html".into(),
                    "index.htm".into(),
                    "index.xhtml".into(),
                ]
            } else {
                entries
                    .iter()
                    .flat_map(|entry| {
                        entry
                            .args
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                    })
                    .collect()
            }
        };

        // Try each index file
        for index in &index_files {
            let index_path = ctx.file_path.join(index);
            let metadata = match vibeio::fs::metadata(&index_path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    ctx.http.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
                    return Ok(false);
                }
                Err(error) => {
                    return Err(PipelineError::custom(error.to_string()));
                }
            };

            if metadata.is_file() {
                // Verify no path traversal
                let canonical = vibeio::fs::canonicalize(&index_path)
                    .await
                    .map_err(|e| PipelineError::custom(e.to_string()))?;

                // Get canonical webroot
                if let Some(root_val) = ctx.http.configuration.get_value("root", true) {
                    if let Some(root_str) = root_val.as_str() {
                        let root_path = std::path::Path::new(root_str);
                        if let Ok(canonical_root) = vibeio::fs::canonicalize(root_path).await {
                            if !canonical.starts_with(&canonical_root) {
                                ctx.http.res =
                                    Some(ferron_http::HttpResponse::BuiltinError(403, None));
                                return Ok(false);
                            }
                        }
                    }
                }

                ctx.file_path = canonical;
                ctx.metadata = metadata;
                return Ok(true);
            }
        }

        // No index file found — let directory_listing or static_file handle it
        Ok(true)
    }
}
