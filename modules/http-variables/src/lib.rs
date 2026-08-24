//! HTTP variables module for Ferron.
//!
//! Provides the `set_var` directive for setting interpolation variables based
//! on request conditions, and the `log_field` directive for mapping variables
//! to custom access log fields after the response is generated.

mod config;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::Variables;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::HttpContext;
use ferron_observability::TraceAttributeValue;

use crate::config::{parse_log_field_rules, parse_set_var_rules, LogFieldSource};
use crate::validator::VariablesValidator;

/// Module loader for the HTTP variables module.
#[derive(Default)]
pub struct HttpVariablesModuleLoader;

impl ModuleLoader for HttpVariablesModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "set_var",
                    usage: "set_var <source> <regex> <variable> { ... }",
                    description: "This directive extracts values from request fields using regex and assigns them to custom variables with optional value, case_insensitive, and negate options.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_set_var")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "log_field",
                    usage: "log_field <name> <source>",
                    description: "This directive defines a custom log field by extracting data from request or response sources.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "value",
                    usage: "value <string>",
                    description: "This directive sets the variable value returned when set_var matches.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_set_var"),
            )
            .register(
                Directive {
                    name: "case_insensitive",
                    usage: "case_insensitive [bool]",
                    description: "This directive enables case-insensitive regex matching for set_var.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_set_var"),
            )
            .register(
                Directive {
                    name: "negate",
                    usage: "negate [bool]",
                    description: "This directive inverts the set_var match condition.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_set_var"),
            );
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<&'static str, Vec<Box<dyn ConfigurationValidator>>>,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(VariablesValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(VariablesStage))
    }
}

/// Pipeline stage that evaluates `set_var` and `log_field` directives.
struct VariablesStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for VariablesStage {
    fn name(&self) -> &str {
        "variables"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("map".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else {
            return false;
        };
        c.has_directive("set_var") || c.has_directive("log_field")
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let rules = parse_set_var_rules(&ctx.configuration);
        let mappings = config::evaluate_set_var_rules(&rules, ctx);
        let set_count = mappings.len();
        for (name, value) in mappings {
            ctx.variables.insert(name, value);
        }
        ctx.get_span_attributes().insert(
            "ferron.variables.set",
            TraceAttributeValue::I64(set_count as i64),
        );
        Ok(true)
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let rules = parse_log_field_rules(&ctx.configuration);
        if rules.is_empty() {
            return Ok(());
        }

        // Resolve all values first to avoid borrow conflicts
        let resolved: Vec<(String, String)> = rules
            .iter()
            .filter_map(|rule| {
                let value = match &rule.source {
                    LogFieldSource::Variable(name) => Variables::resolve(ctx, name),
                    LogFieldSource::Interpolated(parts) => {
                        let istr =
                            ferron_core::config::ServerConfigurationValue::InterpolatedString(
                                parts.clone(),
                                None,
                            );
                        istr.as_string_with_interpolations(ctx)
                    }
                };
                value.map(|v| (rule.field_name.clone(), v))
            })
            .collect();

        let log_fields = custom_access_log_fields(ctx);
        for (field_name, value) in resolved {
            log_fields.insert(field_name, CustomAccessLogField::String(value));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Empty};
    use std::collections::HashMap as StdHashMap;

    fn make_test_context(path: &str, config: Option<LayeredConfiguration>) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri(path)
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        let mut ctx = HttpContext::default();
        ctx.req = Some(req);
        ctx.events = CompositeEventSink::new(Vec::new());
        ctx.configuration = config.unwrap_or_default();
        ctx.encrypted = false;
        ctx.local_address = Some("0.0.0.0:80".parse().unwrap());
        ctx.remote_address = Some("192.0.2.1:12345".parse().unwrap());
        ctx
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_set_var_config(
        source: &str,
        regex: &str,
        variable: &str,
        block: Option<ServerConfigurationBlock>,
    ) -> LayeredConfiguration {
        let mut top_directives = StdHashMap::new();
        top_directives.insert(
            "set_var".to_string(),
            vec![make_entry(
                vec![
                    make_value_string(source),
                    make_value_string(regex),
                    make_value_string(variable),
                ],
                block,
            )],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(top_directives),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    fn make_set_var_block(
        value: Option<&str>,
        ci: Option<bool>,
        negate: Option<bool>,
    ) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();

        if let Some(v) = value {
            directives.insert(
                "value".to_string(),
                vec![make_entry(vec![make_value_string(v)], None)],
            );
        }
        if let Some(ci) = ci {
            directives.insert(
                "case_insensitive".to_string(),
                vec![make_entry(
                    vec![ServerConfigurationValue::Boolean(ci, None)],
                    None,
                )],
            );
        }
        if let Some(n) = negate {
            directives.insert(
                "negate".to_string(),
                vec![make_entry(
                    vec![ServerConfigurationValue::Boolean(n, None)],
                    None,
                )],
            );
        }

        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[tokio::test]
    async fn sets_variable_on_match() {
        let config = make_set_var_config("request.uri.path", r"\.pdf$", "is_pdf", None);
        let mut ctx = make_test_context("/doc.pdf", Some(config));
        let stage = VariablesStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.variables.get("is_pdf"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn does_not_set_variable_on_no_match() {
        let config = make_set_var_config("request.uri.path", r"\.pdf$", "is_pdf", None);
        let mut ctx = make_test_context("/doc.txt", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert!(!ctx.variables.contains_key("is_pdf"));
    }

    #[tokio::test]
    async fn sets_custom_value() {
        let block = make_set_var_block(Some("document"), None, None);
        let config = make_set_var_config("request.uri.path", r"\.pdf$", "file_type", Some(block));
        let mut ctx = make_test_context("/file.pdf", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get("file_type"),
            Some(&"document".to_string())
        );
    }

    #[tokio::test]
    async fn case_insensitive_match() {
        let block = make_set_var_block(None, Some(true), None);
        let config = make_set_var_config("request.uri.path", r"\.PDF$", "is_pdf", Some(block));
        let mut ctx = make_test_context("/doc.pdf", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("is_pdf"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn case_sensitive_no_match() {
        let config = make_set_var_config("request.uri.path", r"\.PDF$", "is_pdf", None);
        let mut ctx = make_test_context("/doc.pdf", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert!(!ctx.variables.contains_key("is_pdf"));
    }

    #[tokio::test]
    async fn negate_sets_on_no_match() {
        let block = make_set_var_block(None, None, Some(true));
        let config = make_set_var_config("request.uri.path", r"\.pdf$", "not_pdf", Some(block));
        let mut ctx = make_test_context("/doc.txt", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("not_pdf"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn negate_does_not_set_on_match() {
        let block = make_set_var_block(None, None, Some(true));
        let config = make_set_var_config("request.uri.path", r"\.pdf$", "not_pdf", Some(block));
        let mut ctx = make_test_context("/doc.pdf", Some(config));
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert!(!ctx.variables.contains_key("not_pdf"));
    }

    #[tokio::test]
    async fn remote_ip_match() {
        let config = make_set_var_config("remote.ip", r"^192\.168\.", "is_local", None);
        let mut ctx = make_test_context("/any", Some(config));
        ctx.remote_address = Some("192.168.1.100:12345".parse().unwrap());
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("is_local"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn header_match() {
        let config =
            make_set_var_config("request.header.user_agent", r"Mozilla", "is_browser", None);
        let mut ctx = make_test_context("/any", Some(config));
        if let Some(req) = &mut ctx.req {
            req.headers_mut()
                .insert("user-agent", "Mozilla/5.0".parse().unwrap());
        }
        let stage = VariablesStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("is_browser"), Some(&"1".to_string()));
    }
}
