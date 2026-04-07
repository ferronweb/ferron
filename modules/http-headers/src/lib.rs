//! HTTP headers and CORS module for Ferron.
//!
//! Provides pipeline stages for:
//! - Response header manipulation (add/replace/remove with interpolation)
//! - CORS preflight handling (OPTIONS) and response header injection

mod config;
mod cors;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::PipelineError;
use ferron_core::registry::RegistryBuilder;
use ferron_http::{HttpContext, HttpResponse};
use http_body_util::BodyExt;

pub use config::HttpHeadersConfigurationValidator;

/// Stage for applying response headers and handling CORS preflight requests.
#[derive(Default)]
pub struct HeadersStage {
    _private: (),
}

impl HeadersStage {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

#[async_trait::async_trait(?Send)]
impl ferron_core::pipeline::Stage<HttpContext> for HeadersStage {
    fn name(&self) -> &str {
        "headers"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![
            ferron_core::StageConstraint::Before("reverse_proxy".to_string()),
            ferron_core::StageConstraint::Before("static_file".to_string()),
        ]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match config::parse_headers_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(true),
            Err(e) => {
                ferron_core::log_error!("Failed to parse headers config: {e}");
                return Ok(true);
            }
        };

        // Handle CORS preflight
        if let (Some(cors), Some(req)) = (config.cors.as_ref(), ctx.req.as_ref()) {
            let origin = req
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let request_method = req
                .headers()
                .get("access-control-request-method")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let request_headers = req
                .headers()
                .get("access-control-request-headers")
                .and_then(|v| v.to_str().ok());

            if cors::is_preflight(req.method(), req.headers()) {
                let response =
                    cors::build_preflight_response(cors, origin, request_method, request_headers);
                let response = response.map(|b| b.map_err(|e| match e {}).boxed_unsync());
                ctx.res = Some(HttpResponse::Custom(response));
                return Ok(false);
            }
        }

        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let config = match config::parse_headers_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(()),
            Err(e) => {
                ferron_core::log_error!("Failed to apply response headers: {e}");
                return Ok(());
            }
        };

        // Pre-resolve all header values before borrowing ctx.res mutably
        let resolved_headers: Vec<(usize, String)> = config
            .header_actions
            .iter()
            .enumerate()
            .filter_map(|(i, action)| {
                let value = match action {
                    config::HeaderAction::Append(_, v) | config::HeaderAction::Replace(_, v) => {
                        Some(interpolate_header_value(v, ctx))
                    }
                    config::HeaderAction::Remove(_) => None,
                };
                value.map(|v| (i, v))
            })
            .collect();

        // Collect CORS context
        let origin = ctx
            .req
            .as_ref()
            .and_then(|r| r.headers().get("origin").and_then(|v| v.to_str().ok()))
            .unwrap_or("")
            .to_string();
        let request_method = ctx
            .req
            .as_ref()
            .map(|r| r.method().as_str().to_string())
            .unwrap_or_default();
        let request_headers = ctx
            .req
            .as_ref()
            .and_then(|r| {
                r.headers()
                    .get("access-control-request-headers")
                    .and_then(|v| v.to_str().ok())
            })
            .map(String::from);

        // Apply header actions and CORS to the response
        if let Some(HttpResponse::Custom(ref mut response)) = ctx.res {
            let headers = response.headers_mut();

            // Apply custom header actions using pre-resolved values
            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in config.header_actions.iter().enumerate() {
                match action {
                    config::HeaderAction::Remove(name) => {
                        headers.remove(name);
                    }
                    config::HeaderAction::Replace(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.insert(name.clone(), val);
                            }
                        }
                    }
                    config::HeaderAction::Append(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.append(name.clone(), val);
                            }
                        }
                    }
                }
            }

            if let Some(cors) = config.cors.as_ref() {
                cors::apply_cors_headers(
                    headers,
                    cors,
                    &origin,
                    &request_method,
                    request_headers.as_deref(),
                );
            }
        } else if let Some(HttpResponse::BuiltinError(_, ref mut maybe_headers)) = ctx.res {
            let headers = maybe_headers.get_or_insert_with(http::HeaderMap::new);

            let mut resolved_iter = resolved_headers.iter().peekable();
            for (i, action) in config.header_actions.iter().enumerate() {
                match action {
                    config::HeaderAction::Remove(name) => {
                        headers.remove(name);
                    }
                    config::HeaderAction::Replace(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.insert(name.clone(), val);
                            }
                        }
                    }
                    config::HeaderAction::Append(name, _) => {
                        if let Some((_, resolved)) = resolved_iter.next_if(|(idx, _)| *idx == i) {
                            if let Ok(val) = http::HeaderValue::from_str(resolved) {
                                headers.append(name.clone(), val);
                            }
                        }
                    }
                }
            }

            if let Some(cors) = config.cors.as_ref() {
                cors::apply_cors_headers(headers, cors, &origin, &request_method, None);
            }
        }

        Ok(())
    }
}

/// Interpolate header value with HTTP request variables, matching the proxy
/// module's implementation.
///
/// Scans for `{{...}}` syntax and resolves variables using the context's
/// `Variables` implementation. Unresolved variables are left as `{{...}}` in
/// the output.
fn interpolate_header_value(value: &str, ctx: &HttpContext) -> String {
    if !value.contains("{{") {
        return value.to_string();
    }

    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next(); // consume second '{'
            let mut var_name = String::new();
            loop {
                match chars.next() {
                    Some('}') if chars.peek() == Some(&'}') => {
                        chars.next(); // consume second '}'
                        break;
                    }
                    Some(c) => var_name.push(c),
                    None => {
                        // Unterminated {{ — emit literally
                        result.push_str("{{");
                        result.push_str(&var_name);
                        return result;
                    }
                }
            }
            // Resolve the variable
            if let Some(env_var) = var_name.strip_prefix("env.") {
                if let Ok(env_value) = std::env::var(env_var) {
                    result.push_str(&env_value);
                } else {
                    result.push_str(&format!("{{{{{}}}}}", var_name));
                }
            } else if let Some(resolved) =
                <dyn ferron_core::config::Variables>::resolve(ctx, &var_name)
            {
                result.push_str(&resolved);
            } else {
                result.push_str(&format!("{{{{{}}}}}", var_name));
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Module loader for the HTTP headers module.
///
/// Registers:
/// - Global configuration validator for headers/CORS directives
/// - Pipeline stage: HeadersStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct HttpHeadersModuleLoader;

impl ModuleLoader for HttpHeadersModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpHeadersConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(HttpHeadersConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HeadersStage::new()))
    }
}
