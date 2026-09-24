//! HTTP URL rewrite module for Ferron.
//!
//! Provides the `rewrite` and `rewrite_log` directives for URL rewriting
//! based on regular expression patterns.

mod config;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::registry::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::HttpContext;
use ferron_observability::{
    Event, LogAttributeValue, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    TraceAttributeValue,
};
use rustc_hash::FxBuildHasher;

use crate::config::{
    apply_rewrite_rules, is_rewrite_log_enabled, parse_rewrite_config, RewriteResult,
};
use crate::validator::RewriteValidator;

/// Shared state for the http-response module.
pub struct RewriteEngine {
    pub compiled_regexes: DashMap<String, Arc<regex::Regex>, FxBuildHasher>,
}

impl RewriteEngine {
    pub fn new() -> Self {
        Self {
            compiled_regexes: DashMap::with_hasher(FxBuildHasher),
        }
    }
}

impl Default for RewriteEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Module loader for the HTTP rewrite module.
#[derive(Default)]
pub struct HttpRewriteModuleLoader;

impl ModuleLoader for HttpRewriteModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "rewrite",
                    usage: "rewrite <regex> <replacement> { ... }",
                    description: "This directive rewrites request URIs using a regex pattern with optional last, directory, file, allow_double_slashes, and name options.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_rewrite")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "rewrite_log",
                    usage: "rewrite_log [bool]",
                    description: "This directive enables logging of rewrite rule evaluations.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "last",
                    usage: "last [bool]",
                    description: "This directive stops processing further rewrite rules on match.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_rewrite"),
            )
            .register(
                Directive {
                    name: "directory",
                    usage: "directory [bool]",
                    description: "This directive restricts the rewrite rule to apply only when the path is a directory.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_rewrite"),
            )
            .register(
                Directive {
                    name: "file",
                    usage: "file [bool]",
                    description: "This directive restricts the rewrite rule to apply only when the path is a file.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_rewrite"),
            )
            .register(
                Directive {
                    name: "allow_double_slashes",
                    usage: "allow_double_slashes [bool]",
                    description: "This directive preserves double slashes in the rewritten URL.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_rewrite"),
            )
            .register(
                Directive {
                    name: "name",
                    usage: "name <label>",
                    description: "This directive sets an operator-chosen identifier for a rewrite rule, surfaced in rewrite observability output. Must be a plain string.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_rewrite"),
            );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(RewriteValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(RewriteValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| {
            Arc::new(RewriteStage::new(Arc::new(RewriteEngine::new())))
        })
    }
}

/// Pipeline stage that applies URL rewrite rules from configuration.
struct RewriteStage {
    engine: Arc<RewriteEngine>,
}

impl RewriteStage {
    fn new(engine: Arc<RewriteEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for RewriteStage {
    fn name(&self) -> &str {
        "rewrite"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::After("https_redirect".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("rewrite"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let rules = parse_rewrite_config(&ctx.configuration, &self.engine);
        if rules.is_empty() {
            ctx.get_span_attributes()
                .insert("ferron.rewrite.applied", TraceAttributeValue::Bool(false));
            ctx.get_span_attributes()
                .insert("ferron.rewrite.pattern_count", TraceAttributeValue::I64(0));
            custom_access_log_fields(ctx).insert(
                "ferron.rewrite.applied".into(),
                CustomAccessLogField::Bool(false),
            );
            return Ok(true);
        }

        let root = ctx
            .configuration
            .get_value("root", true)
            .and_then(|v| v.as_string_with_interpolations(ctx));

        // We need a mutable request reference to mutate the URI
        let Some(req) = ctx.req.as_mut() else {
            return Ok(true);
        };

        let original_url = format!(
            "{}{}",
            req.uri().path(),
            req.uri().query().map_or(String::new(), |q| format!("?{q}"))
        );

        let result = apply_rewrite_rules(&original_url, &rules, root.as_deref()).await;
        let (rewritten, steps) = match result {
            RewriteResult::NoMatch => {
                ctx.get_span_attributes()
                    .insert("ferron.rewrite.applied", TraceAttributeValue::Bool(false));
                ctx.get_span_attributes().insert(
                    "ferron.rewrite.pattern_count",
                    TraceAttributeValue::I64(rules.len() as i64),
                );
                custom_access_log_fields(ctx).insert(
                    "ferron.rewrite.applied".into(),
                    CustomAccessLogField::Bool(false),
                );
                return Ok(true);
            }
            RewriteResult::InvalidRewrite { rule_index } => {
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
                let mut invalid_attrs = vec![(
                    "ferron.rewrite.rule_index",
                    MetricAttributeValue::I64(rule_index as i64 + 1),
                )];
                if let Some(name) = rules.get(rule_index).and_then(|rule| rule.name.as_deref()) {
                    invalid_attrs.push((
                        "ferron.rewrite.rule_name",
                        MetricAttributeValue::String(name.to_string()),
                    ));
                }
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.rewrite.invalid",
                    attributes: invalid_attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "Rewrite rules that produced an invalid path (400 response).",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
                let sa = ctx.get_span_attributes();
                sa.insert("ferron.rewrite.applied", TraceAttributeValue::Bool(false));
                sa.insert(
                    "ferron.rewrite.pattern_count",
                    TraceAttributeValue::I64(rules.len() as i64),
                );
                sa.insert(
                    "ferron.rewrite.rule_index",
                    TraceAttributeValue::I64(rule_index as i64 + 1),
                );
                return Ok(false);
            }
            RewriteResult::Rewritten { url, steps } => (url, steps),
        };

        let should_log = is_rewrite_log_enabled(&ctx.configuration);

        // Store original URI if not already set
        if ctx.original_uri.is_none() {
            ctx.original_uri = Some(req.uri().clone());
        }

        // Mutate the request URI
        let mut uri_parts = req.uri().clone().into_parts();
        let new_path_and_query = match rewritten.parse::<http::Uri>() {
            Ok(uri) => uri.into_parts().path_and_query,
            Err(_) => {
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
                return Ok(false);
            }
        };
        uri_parts.path_and_query = new_path_and_query;

        match http::Uri::from_parts(uri_parts) {
            Ok(new_uri) => {
                *req.uri_mut() = new_uri;
            }
            Err(_) => {
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
                return Ok(false);
            }
        }

        if should_log {
            // One log line per fired rule so chained rewrites stay debuggable;
            // chains are short (bounded by the configured rule count).
            for step in &steps {
                let mut attributes = vec![
                    (
                        "ferron.rewrite.from",
                        LogAttributeValue::String(step.from.clone()),
                    ),
                    (
                        "ferron.rewrite.to",
                        LogAttributeValue::String(step.to.clone()),
                    ),
                    (
                        "ferron.rewrite.rule_index",
                        LogAttributeValue::I64(step.rule_index as i64 + 1),
                    ),
                ];
                if let Some(name) = rules
                    .get(step.rule_index)
                    .and_then(|rule| rule.name.as_deref())
                {
                    attributes.push((
                        "ferron.rewrite.rule_name",
                        LogAttributeValue::String(name.to_string()),
                    ));
                }
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-rewrite",
                        level: ferron_observability::LogLevel::Info,
                        message: format!("URL rewritten from \"{}\" to \"{}\"", step.from, step.to),
                        summary: "URL rewritten".into(),
                        attributes,
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
            }
        }

        // One counter increment per fired rule so dashboards can break down
        // rewrites by rule. Single-rule rewrites behave exactly as before.
        for step in &steps {
            let mut metric_attrs = vec![(
                "ferron.rewrite.rule_index",
                MetricAttributeValue::I64(step.rule_index as i64 + 1),
            )];
            if let Some(name) = rules
                .get(step.rule_index)
                .and_then(|rule| rule.name.as_deref())
            {
                metric_attrs.push((
                    "ferron.rewrite.rule_name",
                    MetricAttributeValue::String(name.to_string()),
                ));
            }
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.rewrite.rewrites_applied",
                attributes: metric_attrs,
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Rewrite rule firings (one per matched rule)."),
                trace_context: current_event_trace_context(ctx),
            }));
        }

        let first_step = steps
            .first()
            .expect("rewritten URLs have at least one step");
        let first_rule_name = rules
            .get(first_step.rule_index)
            .and_then(|rule| rule.name.as_deref());
        let sa = ctx.get_span_attributes();
        sa.insert("ferron.rewrite.applied", TraceAttributeValue::Bool(true));
        sa.insert(
            "ferron.rewrite.pattern_count",
            TraceAttributeValue::I64(rules.len() as i64),
        );
        sa.insert(
            "ferron.rewrite.matched_rule_count",
            TraceAttributeValue::I64(steps.len() as i64),
        );
        sa.insert(
            "ferron.rewrite.rule_index",
            TraceAttributeValue::I64(first_step.rule_index as i64 + 1),
        );
        if let Some(name) = first_rule_name {
            sa.insert(
                "ferron.rewrite.rule_name",
                TraceAttributeValue::String(name.to_string()),
            );
        }
        let log_fields = custom_access_log_fields(ctx);
        log_fields.insert(
            "ferron.rewrite.applied".into(),
            CustomAccessLogField::Bool(true),
        );
        log_fields.insert(
            "ferron.rewrite.matched_rules".into(),
            CustomAccessLogField::U64(steps.len() as u64),
        );
        log_fields.insert(
            "ferron.rewrite.rule_index".into(),
            CustomAccessLogField::U64(first_step.rule_index as u64 + 1),
        );
        if let Some(name) = first_rule_name {
            log_fields.insert(
                "ferron.rewrite.rule_name".into(),
                CustomAccessLogField::String(name.to_string()),
            );
        }

        Ok(true)
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
    use rustc_hash::FxHashMap as StdHashMap;

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

    fn make_rewrite_config(
        rules: Vec<(&str, &str, Option<ServerConfigurationBlock>)>,
    ) -> LayeredConfiguration {
        let mut directives = StdHashMap::default();
        let mut entries = Vec::new();
        for (regex, replacement, children) in rules {
            entries.push(ServerConfigurationDirectiveEntry {
                args: vec![
                    ServerConfigurationValue::String(regex.to_string(), None),
                    ServerConfigurationValue::String(replacement.to_string(), None),
                ],
                children,
                span: None,
            });
        }
        directives.insert("rewrite".to_string(), entries);

        let mut config = LayeredConfiguration::new();
        config.add_layer(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::default(),
            span: None,
        }));
        config
    }

    fn make_options_block(options: &[(&str, bool)]) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::default();
        for (name, value) in options {
            directives.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::Boolean(*value, None)],
                    children: None,
                    span: None,
                }],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::default(),
            span: None,
        }
    }

    fn make_named_options_block(name: &str) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::default();
        directives.insert(
            "name".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(name.to_string(), None)],
                children: None,
                span: None,
            }],
        );
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::default(),
            span: None,
        }
    }

    fn span_attr(ctx: &mut HttpContext, key: &str) -> Option<TraceAttributeValue> {
        use ferron_http::span::HttpContextSpanExt;
        ctx.get_span_attributes().get(key).cloned()
    }

    #[tokio::test]
    async fn rewrites_url_with_simple_rule() {
        let config = make_rewrite_config(vec![("^/old/(.*)", "/new/$1", None)]);
        let mut ctx = make_test_context("/old/path", Some(config));
        let stage = RewriteStage::new(Default::default());
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/new/path");
    }

    #[tokio::test]
    async fn preserves_query_string() {
        let config = make_rewrite_config(vec![("^/api/(.*)", "/v2/$1", None)]);
        let mut ctx = make_test_context("/api/users?page=2", Some(config));
        let stage = RewriteStage::new(Default::default());
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(
            ctx.req
                .as_ref()
                .unwrap()
                .uri()
                .path_and_query()
                .unwrap()
                .as_str(),
            "/v2/users?page=2"
        );
    }

    #[tokio::test]
    async fn last_flag_stops_chaining() {
        let config = make_rewrite_config(vec![
            (
                "^/a/(.*)",
                "/b/$1",
                Some(make_options_block(&[("last", true)])),
            ),
            ("^/b/(.*)", "/c/$1", None),
        ]);
        let mut ctx = make_test_context("/a/test", Some(config));
        let stage = RewriteStage::new(Default::default());
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/b/test");
    }

    #[tokio::test]
    async fn sets_original_uri() {
        let config = make_rewrite_config(vec![("^/x/(.*)", "/y/$1", None)]);
        let mut ctx = make_test_context("/x/foo", Some(config));
        let stage = RewriteStage::new(Default::default());
        let _ = stage.run(&mut ctx).await.unwrap();
        assert!(ctx.original_uri.is_some());
        assert_eq!(ctx.original_uri.as_ref().unwrap().path(), "/x/foo");
    }

    #[tokio::test]
    async fn chained_rewrite_reports_first_rule_and_step_count() {
        let config = make_rewrite_config(vec![
            ("^/legacy/(.*)", "/modern/$1", None),
            ("^/modern/(.*)", "/current/$1", None),
        ]);
        let mut ctx = make_test_context("/legacy/foo", Some(config));
        let stage = RewriteStage::new(Default::default());
        assert!(stage.run(&mut ctx).await.unwrap());
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/current/foo");
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.rule_index"),
            Some(TraceAttributeValue::I64(1))
        );
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.matched_rule_count"),
            Some(TraceAttributeValue::I64(2))
        );
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.rule_name"),
            None,
            "unnamed rules must not emit a rule name"
        );
    }

    #[tokio::test]
    async fn invalid_rewrite_reports_offending_rule_index() {
        let config = make_rewrite_config(vec![
            ("^/ok/(.*)", "/fine/$1", None),
            ("^/bad/(.*)", "$1", None),
        ]);
        let mut ctx = make_test_context("/bad/path", Some(config));
        let stage = RewriteStage::new(Default::default());
        assert!(!stage.run(&mut ctx).await.unwrap());
        assert!(matches!(
            ctx.res,
            Some(ferron_http::HttpResponse::BuiltinError(400, None))
        ));
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.rule_index"),
            Some(TraceAttributeValue::I64(2))
        );
    }

    #[tokio::test]
    async fn named_rule_surfaces_name_in_span() {
        let config = make_rewrite_config(vec![(
            "^/old/(.*)",
            "/new/$1",
            Some(make_named_options_block("legacy-redirect")),
        )]);
        let mut ctx = make_test_context("/old/path", Some(config));
        let stage = RewriteStage::new(Default::default());
        assert!(stage.run(&mut ctx).await.unwrap());
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.rule_name"),
            Some(TraceAttributeValue::String("legacy-redirect".to_string()))
        );
        assert_eq!(
            span_attr(&mut ctx, "ferron.rewrite.rule_index"),
            Some(TraceAttributeValue::I64(1))
        );
    }

    #[test]
    fn rewrite_rule_name_is_parsed() {
        let engine = RewriteEngine::new();
        let mut directives = StdHashMap::default();
        directives.insert(
            "rewrite".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    ServerConfigurationValue::String("^/old/(.*)".to_string(), None),
                    ServerConfigurationValue::String("/new/$1".to_string(), None),
                ],
                children: Some(make_named_options_block("legacy-redirect")),
                span: None,
            }],
        );
        let mut config = LayeredConfiguration::new();
        config.add_layer(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::default(),
            span: None,
        }));
        let rules = crate::config::parse_rewrite_config(&config, &engine);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name.as_deref(), Some("legacy-redirect"));
    }
}
