//! Error page stage — serves static HTML files for HTTP error responses.

use std::io;
use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::config::ServerConfigurationValue;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::HttpErrorContext;
use futures_util::TryStreamExt;
use http::header::{self, HeaderValue};
use http::Response;
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};

use crate::util::file_stream::FileStream;

pub struct ErrorPageStage;

impl Default for ErrorPageStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpErrorContext> for ErrorPageStage {
    #[inline]
    fn name(&self) -> &str {
        "error_page"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![]
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpErrorContext) -> Result<bool, PipelineError> {
        // Skip if a response has already been set
        if ctx.res.is_some() {
            return Ok(true);
        }

        let error_code = ctx.error_code;
        let config = &ctx.configuration;

        // Collect all error_page entries across layers
        let entries = config.get_entries("error_page", true);

        for entry in entries {
            // Need at least 2 args: one or more status codes + file path
            if entry.args.len() < 2 {
                continue;
            }

            // The last argument is the file path
            let file_path = match entry.args.last() {
                Some(ServerConfigurationValue::String(path, _)) => path.as_str(),
                _ => continue,
            };

            // All preceding arguments are status codes
            let mut matches_error_code = false;
            for arg in &entry.args[..entry.args.len() - 1] {
                let code = match arg {
                    ServerConfigurationValue::Number(n, _) => *n as u16,
                    ServerConfigurationValue::String(s, _) => match s.parse::<u16>() {
                        Ok(n) => n,
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                if code == error_code {
                    matches_error_code = true;
                    break;
                }
            }

            if !matches_error_code {
                continue;
            }

            // Try to open the error page file
            let path = Path::new(file_path);
            let meta = match vibeio::fs::metadata(path).await {
                Ok(m) => m,
                Err(_) => {
                    ferron_core::log_warn!("Error page file cannot be opened: {}", file_path);
                    continue;
                }
            };

            if !meta.is_file() {
                continue;
            }

            let file_length = meta.len();

            // Open file for reading
            let file = vibeio::fs::File::open(path)
                .await
                .map_err(|e| PipelineError::custom(format!("failed to open error page: {e}")))?;

            // Extract raw fd for zerocopy (unix) or handle (windows)
            #[cfg(unix)]
            let raw_fd = {
                use std::os::fd::AsRawFd;
                Some(file.as_raw_fd())
            };
            #[cfg(not(unix))]
            let raw_fd: Option<i64> = None;

            // Build response
            let mut builder = Response::builder()
                .status(error_code)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))
                .header(header::CONTENT_LENGTH, file_length);

            // Copy over any headers from the error context (e.g., Allow for 405)
            if let Some(ref headers) = ctx.headers {
                for (name, value) in headers.iter() {
                    builder = builder.header(name.clone(), value.clone());
                }
            }

            // For HEAD-like scenarios or zero-length files, return empty body
            if file_length == 0 {
                let response = builder
                    .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                    .map_err(|e| PipelineError::custom(e.to_string()))?;
                ctx.res = Some(response);
                return Ok(false);
            }

            // Stream the file content
            let body: UnsyncBoxBody<Bytes, io::Error> =
                StreamBody::new(FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data))
                    .boxed_unsync();

            let mut response = builder
                .body(body)
                .map_err(|e| PipelineError::custom(e.to_string()))?;

            // Enable zerocopy for error page responses on unix
            #[cfg(unix)]
            {
                if let Some(fd) = raw_fd {
                    use std::os::fd::RawFd;
                    unsafe { vibeio_http::install_zerocopy(&mut response, fd as RawFd) };
                }
            }

            ctx.res = Some(response);
            return Ok(false);
        }

        // No matching error page found — pass through
        Ok(true)
    }
}
