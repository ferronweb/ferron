use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpFileContext, HttpResponse};
use ferron_observability::{Event, LogEvent};
use http::Response;
use http_body_util::BodyExt;
use tokio::io::AsyncReadExt;

use crate::{
    client::{ClientError, FcgiClient},
    config::FcgiConfiguration,
    util::{SendWrapBody, TrackedBody},
};

pub struct FcgiFileStage {
    client: Arc<FcgiClient>,
}

impl FcgiFileStage {
    pub fn new(client: Arc<FcgiClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpFileContext> for FcgiFileStage {
    fn name(&self) -> &str {
        "fcgi_pass"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![ferron_core::StageConstraint::Before(
            "static_file".to_string(),
        )]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|b| b.has_directive("fcgi") || b.has_directive("fcgi_php"))
    }

    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        // -- check if FastCGI is applicable
        let Some(config) = FcgiConfiguration::from_http_ctx(&ctx.http) else {
            // FastCGI not configured
            return Ok(true);
        };

        if config.pass {
            // Pass
            return Ok(true);
        }

        if !ctx
            .file_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                config
                    .extensions
                    .contains(&format!(".{}", e.to_lowercase()))
            })
        {
            // FastCGI not applicable for this file extension
            return Ok(true);
        }

        let Some(mut request) = ctx.http.req.take() else {
            // Request struct not found
            return Ok(true);
        };

        // -- set environment variables --

        // Remove "Proxy" header from the request to prevent "httpoxy" vulnerability
        request
            .headers_mut()
            .remove(http::header::HeaderName::from_static("proxy"));

        let original_request_uri = ctx.http.original_uri.as_ref().unwrap_or(request.uri());
        let mut env_builder = cegla_fcgi::client::CgiBuilder::new();

        if let Some(auth_user) = ctx.http.auth_user.as_deref() {
            let authorization_type =
                if let Some(authorization) = request.headers().get(http::header::AUTHORIZATION) {
                    let authorization_value =
                        String::from_utf8_lossy(authorization.as_bytes()).to_string();
                    let mut authorization_value_split = authorization_value.split(" ");
                    authorization_value_split
                        .next()
                        .map(|authorization_type| authorization_type.to_string())
                } else {
                    None
                };
            env_builder = env_builder.auth(authorization_type, auth_user.to_string());
        }

        if let Some(server_administrator_email) = ctx
            .http
            .configuration
            .get_value("admin_email", true)
            .and_then(|v| v.as_string_with_interpolations(&ctx.http))
        {
            env_builder = env_builder.server_admin(server_administrator_email);
        }

        if ctx.http.encrypted {
            env_builder = env_builder.https();
        }

        env_builder = env_builder
            .server("Ferron".to_string())
            .server_address(ctx.http.local_address)
            .client_address(ctx.http.remote_address)
            .script_path(
                ctx.file_path.clone(),
                ctx.file_root.clone(),
                ctx.path_info.clone(),
            )
            .request_uri(original_request_uri);

        for (env_var_key, env_var_value) in config.environment {
            env_builder = env_builder.var(env_var_key, env_var_value);
        }

        // -- execute FastCGI --
        let scgi_to_fixed = if let Some(stripped) = config.backend_server.strip_prefix("unix:///") {
            // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
            &format!("unix://ignore/{stripped}")
        } else {
            &config.backend_server
        };

        // Set and get local limit for the connection pool
        if let Some(limit) = config.local_limit {
            self.client.set_local_limit(scgi_to_fixed, limit).await;
        }
        let local_limit = self.client.get_local_limit(scgi_to_fixed).await;

        // Get connection from pool
        let mut conn_item = match self
            .client
            .get_connection(scgi_to_fixed, config.keepalive, local_limit)
            .await
        {
            Ok(conn) => conn,
            Err(ClientError::ServiceUnavailable(err)) => {
                ctx.http.events.emit(Event::Log(LogEvent {
                    level: ferron_observability::LogLevel::Error,
                    message: format!("Service unavailable: {err}"),
                    target: "ferron-http-scgi",
                }));
                ctx.http.res = Some(HttpResponse::BuiltinError(503, None));
                return Ok(false);
            }
            Err(ClientError::Other(err)) => {
                return Err(PipelineError::custom(err.to_string()));
            }
        };

        let (response, mut stderr) = conn_item
            .inner()
            .as_ref()
            .unwrap()
            .send_request(request, env_builder)
            .await
            .map_err(|e| PipelineError::custom(e.to_string()))?;

        let events = ctx.http.events.clone();
        vibeio::spawn(async move {
            let mut stderr_string = String::new();
            stderr
                .read_to_string(&mut stderr_string)
                .await
                .unwrap_or_default();
            let stderr_string_trimmed = stderr_string.trim();
            if !stderr_string_trimmed.is_empty() {
                events.emit(Event::Log(LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!("There were FastCGI errors: {stderr_string_trimmed}"),
                    target: "ferron-http-fcgi",
                }));
            }
        });

        if !config.keepalive {
            // Remove connection from pool
            conn_item.inner_mut().take();
        }

        let (parts, body) = response.into_parts();
        let response = Response::from_parts(
            parts,
            TrackedBody::new(SendWrapBody::new(body), conn_item).boxed_unsync(),
        );

        // FastCGI response
        ctx.http.res = Some(HttpResponse::Custom(response));
        Ok(false)
    }
}
