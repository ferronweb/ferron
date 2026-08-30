//! Error page stage: serves static HTML files for HTTP error responses.

use std::io;
use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::config::ServerConfigurationValue;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::StageConstraint;
use ferron_http::file_descriptor::ReusedFile;
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

        // Check if placeholder substitution is enabled
        let placeholders_enabled = config.get_flag("error_page_placeholders", true);

        // Collect all error_page entries across layers
        let entries = config.get_entries("error_page", true);

        for entry in entries {
            // Need at least 2 args: one or more status codes + file path
            if entry.args.len() < 2 {
                continue;
            }

            let Some(file_path) = entry
                .args
                .last()
                .and_then(|v| v.as_string_with_interpolations(ctx))
            else {
                continue;
            };

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

            let path = Path::new(&file_path);

            let Ok(file) = ReusedFile::open(path).await else {
                continue;
            };

            let Ok(meta) = file.metadata() else {
                continue;
            };

            if !meta.is_file() {
                continue;
            }

            let file_length = meta.len();

            if placeholders_enabled {
                if let Some(ref trace_context) = ctx.trace_context {
                    if let Ok(content) = zincio::fs::read_to_string(path).await {
                        let content = content
                            .replace("{{trace.id}}", &trace_context.trace_id)
                            .replace("{{trace.spanid}}", &trace_context.span_id);
                        let bytes = Bytes::from(content);

                        let mut builder = Response::builder()
                            .status(error_code)
                            .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))
                            .header(header::CONTENT_LENGTH, bytes.len());

                        if let Some(ref headers) = ctx.headers {
                            for (name, value) in headers.iter() {
                                builder = builder.header(name.clone(), value.clone());
                            }
                        }

                        let body: UnsyncBoxBody<Bytes, io::Error> =
                            http_body_util::Full::new(bytes)
                                .map_err(|_| unreachable!())
                                .boxed_unsync();

                        let response = builder
                            .body(body)
                            .map_err(|e| PipelineError::custom(e.to_string()))?;

                        ctx.res = Some(response);
                        return Ok(false);
                    }
                }
            }

            // Extract raw fd for zerocopy (unix) or handle (windows)
            #[cfg(unix)]
            let raw_fd = {
                use std::os::fd::AsRawFd;
                Some(file.as_raw_fd())
            };
            #[cfg(not(unix))]
            let raw_fd: Option<i64> = None;

            let mut builder = Response::builder()
                .status(error_code)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))
                .header(header::CONTENT_LENGTH, file_length);

            if let Some(ref headers) = ctx.headers {
                for (name, value) in headers.iter() {
                    builder = builder.header(name.clone(), value.clone());
                }
            }

            if file_length == 0 {
                let response = builder
                    .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                    .map_err(|e| PipelineError::custom(e.to_string()))?;
                ctx.res = Some(response);
                return Ok(false);
            }

            let body: UnsyncBoxBody<Bytes, io::Error> =
                StreamBody::new(FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data))
                    .boxed_unsync();

            let mut response = builder
                .body(body)
                .map_err(|e| PipelineError::custom(e.to_string()))?;

            #[cfg(unix)]
            {
                if let Some(fd) = raw_fd {
                    use std::os::fd::RawFd;
                    unsafe { zincio_http::install_zerocopy(&mut response, fd as RawFd) };
                }
            }

            ctx.res = Some(response);
            return Ok(false);
        }

        Ok(true)
    }
}
