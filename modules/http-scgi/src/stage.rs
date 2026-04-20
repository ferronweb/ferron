use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent};
use http::Response;
use http_body_util::BodyExt;
use vibeio_cegla::VibeioScgiRuntime;

use crate::{
    config::ScgiConfiguration,
    util::{ConnectedSocket, SendWrapBody},
};

pub struct ScgiStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for ScgiStage {
    fn name(&self) -> &str {
        "scgi"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![
            ferron_core::StageConstraint::Before("reverse_proxy".to_string()),
            ferron_core::StageConstraint::After("forward_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|b| b.has_directive("cgi"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // -- check if SCGI is applicable
        let Some(config) = ScgiConfiguration::from_http_ctx(&ctx) else {
            // SCGI not configured
            return Ok(true);
        };

        let Some(mut request) = ctx.req.take() else {
            // Request struct not found
            return Ok(true);
        };

        // -- set environment variables --

        // Remove "Proxy" header from the request to prevent "httpoxy" vulnerability
        request
            .headers_mut()
            .remove(http::header::HeaderName::from_static("proxy"));

        let original_request_uri = ctx.original_uri.as_ref().unwrap_or(request.uri());
        let mut env_builder = cegla_scgi::client::CgiBuilder::new();

        if let Some(auth_user) = ctx.auth_user.as_deref() {
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
            .configuration
            .get_value("admin_email", true)
            .and_then(|v| v.as_string_with_interpolations(ctx))
        {
            env_builder = env_builder.server_admin(server_administrator_email);
        }

        if ctx.encrypted {
            env_builder = env_builder.https();
        }

        env_builder = env_builder
            .server("Ferron".to_string())
            .server_address(ctx.local_address)
            .client_address(ctx.remote_address)
            .request_uri(original_request_uri);

        for (env_var_key, env_var_value) in config.environment {
            env_builder = env_builder.var(env_var_key, env_var_value);
        }

        // -- execute SCGI --
        let scgi_to_fixed = if let Some(stripped) = config.backend_server.strip_prefix("unix:///") {
            // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
            &format!("unix://ignore/{stripped}")
        } else {
            &config.backend_server
        };

        let scgi_to_url = scgi_to_fixed
            .parse::<http::Uri>()
            .map_err(|e| PipelineError::custom(e.to_string()))?;
        let scheme_str = scgi_to_url.scheme_str();

        let connected_socket = match scheme_str {
            Some("tcp") => {
                let host = match scgi_to_url.host() {
                    Some(host) => host,
                    None => {
                        return Err(PipelineError::custom(
                            "The SCGI URL doesn't include the host",
                        ))
                    }
                };

                let port = match scgi_to_url.port_u16() {
                    Some(port) => port,
                    None => {
                        return Err(PipelineError::custom(
                            "The SCGI URL doesn't include the port",
                        ))
                    }
                };

                let addr = format!("{host}:{port}");

                match ConnectedSocket::connect_tcp(&addr).await {
                    Ok(data) => data,
                    Err(err) => match err.kind() {
                        std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::HostUnreachable => {
                            ctx.events.emit(Event::Log(LogEvent {
                                level: ferron_observability::LogLevel::Error,
                                message: format!("Service unavailable: {err}"),
                                target: "ferron-http-scgi",
                            }));
                            ctx.res = Some(HttpResponse::BuiltinError(503, None));
                            return Ok(true);
                        }
                        _ => return Err(PipelineError::custom(err.to_string())),
                    },
                }
            }
            Some("unix") => {
                let path = scgi_to_url.path();
                match ConnectedSocket::connect_unix(path).await {
                    Ok(data) => data,
                    Err(err) => match err.kind() {
                        std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::HostUnreachable => {
                            ctx.events.emit(Event::Log(LogEvent {
                                level: ferron_observability::LogLevel::Error,
                                message: format!("Service unavailable: {err}"),
                                target: "ferron-http-scgi",
                            }));
                            ctx.res = Some(HttpResponse::BuiltinError(503, None));
                            return Ok(true);
                        }
                        _ => return Err(PipelineError::custom(err.to_string())),
                    },
                }
            }
            _ => {
                return Err(PipelineError::custom(
                    "Only TCP and Unix socket URLs are supported.",
                ))
            }
        };

        let response = cegla_scgi::client::client_handle_scgi(
            request,
            VibeioScgiRuntime,
            connected_socket,
            env_builder,
        )
        .await
        .map_err(|e| PipelineError::custom(e.to_string()))?;

        let (parts, body) = response.into_parts();
        let response = Response::from_parts(parts, SendWrapBody::new(body).boxed_unsync());

        // SCGI response
        ctx.res = Some(HttpResponse::Custom(response));
        Ok(false)
    }
}
