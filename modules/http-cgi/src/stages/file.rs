use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{HttpFileContext, HttpResponse};
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    TraceAttributeValue,
};
use http::Response;
use http_body_util::BodyExt;
use tokio::io::AsyncReadExt;
use vibeio_cegla::VibeioCgiRuntime;

use crate::config::CgiConfiguration;
use crate::util::{get_executable, SendWrapBody};

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

        // Get metadata from file handle (CGI doesn't need the handle for streaming)
        let is_file = if let Some(ref file) = ctx.file {
            file.metadata().map(|m| m.is_file()).unwrap_or(false)
        } else {
            false
        };
        if !is_file {
            // Not a file, skip
            return Ok(true);
        }

        if !ctx.file_path.starts_with(ctx.file_root.join("cgi-bin"))
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

        // Inject trace context into the request environment
        if let Some(tc) = ctx
            .http
            .get::<ferron_http::trace_context::TraceContextKey>()
        {
            ferron_http::trace_context::inject_trace_headers(request.headers_mut(), tc);
        }

        let original_request_uri = ctx.http.original_uri.as_ref().unwrap_or(request.uri());
        let mut env_builder = cegla_cgi::client::CgiBuilder::new();

        if let Some(auth_user) = ctx.http.auth_user.as_deref() {
            let authorization_type = if let Some(authorization_value) = request
                .headers()
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
            {
                let mut authorization_value_split = authorization_value.split_whitespace();
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

        // Canonicalize the file path and root if they are relative paths
        let file_path = if ctx.file_path.has_root()
            && !ctx.file_path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            }) {
            ctx.file_path.clone()
        } else {
            vibeio::fs::canonicalize(&ctx.file_path)
                .await
                .unwrap_or(ctx.file_path.clone())
        };
        let file_root = if ctx.file_path.has_root()
            && !ctx.file_root.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            }) {
            ctx.file_root.clone()
        } else {
            vibeio::fs::canonicalize(&ctx.file_root)
                .await
                .unwrap_or(ctx.file_root.clone())
        };

        env_builder = env_builder
            .server("Ferron".to_string())
            .server_address(ctx.http.local_address)
            .client_address(ctx.http.remote_address)
            .script_path(file_path, file_root, ctx.path_info.clone())
            .request_uri(original_request_uri);

        if let Some(hostname) = ctx.http.hostname.clone() {
            env_builder = env_builder.server(hostname);
        }

        for (env_var_key, env_var_value) in config.environment {
            env_builder = env_builder.var(env_var_key, env_var_value);
        }

        // -- execute CGI --
        let process_start = Instant::now();
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

        let process_duration = process_start.elapsed().as_secs_f64();

        let (parts, body) = response.into_parts();
        let response = Response::from_parts(parts, SendWrapBody::new(body).boxed_unsync());

        if let Some(exit_code) = exit_code_option {
            if !exit_code.success() {
                let exit_code_str = exit_code
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let exit_code_val = exit_code_str.clone();
                ctx.http.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.cgi.failures",
                    attributes: vec![
                        (
                            "error.type",
                            MetricAttributeValue::StaticStr("non_zero_exit_code"),
                        ),
                        (
                            "ferron.cgi.exit_code",
                            MetricAttributeValue::String(exit_code_str),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "Number of CGI requests that failed with a non-zero exit code.",
                    ),
                    trace_context: ferron_http::trace_context::current_event_trace_context(
                        &ctx.http,
                    ),
                }));
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
                            summary: "CGI errors on stderr".into(),
                            target: "ferron-http-cgi",
                            attributes: vec![(
                                "error.message",
                                LogAttributeValue::String(stderr_string_trimmed.to_string()),
                            )],
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                &ctx.http,
                            ),
                        }));
                        ctx.http.events.emit(Event::Metric(MetricEvent {
                            name: "ferron.cgi.stderr_errors",
                            attributes: vec![],
                            ty: MetricType::Counter,
                            value: MetricValue::U64(1),
                            unit: Some("{error}"),
                            description: Some(
                                "Number of CGI requests that produced non-empty stderr output.",
                            ),
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                &ctx.http,
                            ),
                        }));
                    }
                    let script_path = ctx.file_path.to_string_lossy().to_string();
                    ctx.http.res = Some(HttpResponse::BuiltinError(500, None));
                    ctx.get_span_attributes().insert(
                        "error.type",
                        TraceAttributeValue::StaticStr("non_zero_exit_code"),
                    );
                    ctx.get_span_attributes()
                        .insert("http.response.status_code", TraceAttributeValue::I64(500));
                    ctx.get_span_attributes().insert(
                        "ferron.cgi.script_path",
                        TraceAttributeValue::String(script_path.clone()),
                    );
                    ctx.get_span_attributes().insert(
                        "ferron.cgi.exit_code",
                        TraceAttributeValue::String(exit_code_val.clone()),
                    );
                    let log_fields = custom_access_log_fields(&mut ctx.http);
                    log_fields.insert(
                        "ferron.cgi.script_path".into(),
                        CustomAccessLogField::String(script_path),
                    );
                    log_fields.insert(
                        "ferron.cgi.exit_code".into(),
                        CustomAccessLogField::String(exit_code_val),
                    );
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
                        summary: "CGI errors on stderr".into(),
                        target: "ferron-http-cgi",
                        attributes: vec![(
                            "error.message",
                            LogAttributeValue::String(stderr_string_trimmed.to_string()),
                        )],
                        trace_context: None,
                    }));
                    events.emit(Event::Metric(MetricEvent {
                        name: "ferron.cgi.stderr_errors",
                        attributes: vec![],
                        ty: MetricType::Counter,
                        value: MetricValue::U64(1),
                        unit: Some("{error}"),
                        description: Some(
                            "Number of CGI requests that produced non-empty stderr output.",
                        ),
                        trace_context: None,
                    }));
                }
            }
        });

        // CGI response
        let status_code = response.status().as_u16();
        ctx.http.res = Some(HttpResponse::Custom(response));

        ctx.http.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cgi.process.duration",
            attributes: vec![],
            ty: MetricType::Histogram(None),
            value: MetricValue::F64(process_duration),
            unit: Some("s"),
            description: Some("Duration of CGI process execution."),
            trace_context: ferron_http::trace_context::current_event_trace_context(&ctx.http),
        }));
        ctx.http.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cgi.requests",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Number of CGI requests processed."),
            trace_context: ferron_http::trace_context::current_event_trace_context(&ctx.http),
        }));

        let script_path = ctx.file_path.to_string_lossy().to_string();
        ctx.get_span_attributes().insert(
            "http.response.status_code",
            TraceAttributeValue::I64(status_code as i64),
        );
        ctx.get_span_attributes().insert(
            "ferron.cgi.script_path",
            TraceAttributeValue::String(script_path.clone()),
        );
        if let Some(exit_code) = exit_code_option {
            let exit_code_str = exit_code
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            ctx.get_span_attributes().insert(
                "ferron.cgi.exit_code",
                TraceAttributeValue::String(exit_code_str.clone()),
            );
            let log_fields = custom_access_log_fields(&mut ctx.http);
            log_fields.insert(
                "ferron.cgi.exit_code".into(),
                CustomAccessLogField::String(exit_code_str),
            );
        }
        custom_access_log_fields(&mut ctx.http).insert(
            "ferron.cgi.script_path".into(),
            CustomAccessLogField::String(script_path),
        );

        Ok(false)
    }
}
