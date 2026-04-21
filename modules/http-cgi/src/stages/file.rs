use std::{collections::HashMap, sync::LazyLock};

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::{HttpFileContext, HttpResponse};
use ferron_observability::{Event, LogEvent};
use http::Response;
use http_body_util::BodyExt;
use tokio::io::AsyncReadExt;
use vibeio_cegla::VibeioCgiRuntime;

use crate::{
    config::CgiConfiguration,
    util::{get_executable, SendWrapBody},
};

static DEFAULT_CGI_INTERPRETERS: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(|| {
    let mut cgi_interpreters = HashMap::new();
    cgi_interpreters.insert(".pl".to_string(), vec!["perl".to_string()]);
    cgi_interpreters.insert(".py".to_string(), vec!["python".to_string()]);
    cgi_interpreters.insert(".sh".to_string(), vec!["bash".to_string()]);
    cgi_interpreters.insert(".ksh".to_string(), vec!["ksh".to_string()]);
    cgi_interpreters.insert(".csh".to_string(), vec!["csh".to_string()]);
    cgi_interpreters.insert(".rb".to_string(), vec!["ruby".to_string()]);
    cgi_interpreters.insert(".php".to_string(), vec!["php-cgi".to_string()]);
    if cfg!(windows) {
        cgi_interpreters.insert(".exe".to_string(), vec![]);
        cgi_interpreters.insert(
            ".bat".to_string(),
            vec!["cmd".to_string(), "/c".to_string()],
        );
        cgi_interpreters.insert(".vbs".to_string(), vec!["cscript".to_string()]);
    }
    cgi_interpreters
});

pub struct CgiStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpFileContext> for CgiStage {
    fn name(&self) -> &str {
        "cgi"
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
        config.is_some_and(|b| b.has_directive("cgi"))
    }

    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        // -- check if CGI is applicable
        let Some(config) = CgiConfiguration::from_http_ctx(&ctx.http) else {
            // CGI not configured
            return Ok(true);
        };

        if !ctx.metadata.is_file() {
            // Not a file, skip
            return Ok(true);
        }

        if !ctx
            .file_path
            .strip_prefix(ctx.file_root.join("cgi-bin"))
            .is_ok_and(|p| {
                p.iter()
                    .next()
                    .is_some_and(|c| c.eq_ignore_ascii_case("cgi-bin"))
            })
            && !ctx
                .file_path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| {
                    config
                        .additional_extensions
                        .contains(&format!(".{}", e.to_lowercase()))
                })
        {
            // CGI not applicable ("cgi-bin" or additional extension)
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
        let mut env_builder = cegla_cgi::client::CgiBuilder::new();

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

        // -- execute CGI --
        let executable_params = match get_executable(&ctx.file_path).await {
            Ok(params) => params,
            Err(err) => {
                let contained_extension = ctx
                    .file_path
                    .extension()
                    .map(|a| format!(".{}", a.to_string_lossy()));
                if let Some(contained_extension) = contained_extension {
                    if let Some(params_init) = config.interpreters.get(&contained_extension) {
                        if let Some(params_init) = params_init {
                            let mut params: Vec<String> =
                                params_init.iter().map(|s| s.to_owned()).collect();
                            params.push(ctx.file_path.to_string_lossy().to_string());
                            params
                        } else {
                            return Err(PipelineError::custom(format!(
                                "Cannot determine the executable {err}"
                            )));
                        }
                    } else if let Some(params_init) =
                        DEFAULT_CGI_INTERPRETERS.get(&contained_extension)
                    {
                        let mut params: Vec<String> =
                            params_init.iter().map(|s| s.to_owned()).collect();
                        params.push(ctx.file_path.to_string_lossy().to_string());
                        params
                    } else {
                        return Err(PipelineError::custom(format!(
                            "Cannot determine the executable {err}"
                        )));
                    }
                } else {
                    return Err(PipelineError::custom(format!(
                        "Cannot determine the executable {err}"
                    )));
                }
            }
        };

        let mut execute_dir_pathbuf = ctx.file_path.clone();
        execute_dir_pathbuf.pop();

        let mut executable_params_iter = executable_params.iter();
        let cmd = std::ffi::OsStr::new(match executable_params_iter.next() {
            Some(executable_name) => executable_name,
            None => return Err(PipelineError::custom("Cannot determine the executable"))?,
        });
        let args: Vec<_> = executable_params_iter.map(std::ffi::OsStr::new).collect();

        let (response, stderr, exit_code_option) = cegla_cgi::client::execute_cgi(
            request,
            VibeioCgiRuntime,
            cmd,
            &args,
            env_builder,
            Some(execute_dir_pathbuf),
        )
        .await
        .map_err(|e| PipelineError::custom(e.to_string()))?;

        let (parts, body) = response.into_parts();
        let response = Response::from_parts(parts, SendWrapBody::new(body).boxed_unsync());

        if let Some(exit_code) = exit_code_option {
            if !exit_code.success() {
                if let Some(mut stderr) = stderr {
                    let mut stderr_string = String::new();
                    stderr
                        .read_to_string(&mut stderr_string)
                        .await
                        .unwrap_or_default();
                    let stderr_string_trimmed = stderr_string.trim();
                    if !stderr_string_trimmed.is_empty() {
                        ctx.http.events.emit(Event::Log(LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!("There were CGI errors: {stderr_string_trimmed}"),
                            target: "ferron-http-cgi",
                        }));
                    }
                    ctx.http.res = Some(HttpResponse::BuiltinError(500, None));
                    return Ok(false);
                }
            }
        }

        let events = ctx.http.events.clone();
        vibeio::spawn(async move {
            if let Some(mut stderr) = stderr {
                let mut stderr_string = String::new();
                stderr
                    .read_to_string(&mut stderr_string)
                    .await
                    .unwrap_or_default();
                let stderr_string_trimmed = stderr_string.trim();
                if !stderr_string_trimmed.is_empty() {
                    events.emit(Event::Log(LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: format!("There were CGI errors: {stderr_string_trimmed}"),
                        target: "ferron-http-cgi",
                    }));
                }
            }
        });

        // CGI response
        ctx.http.res = Some(HttpResponse::Custom(response));
        Ok(false)
    }
}
