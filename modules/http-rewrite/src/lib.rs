//! HTTP URL rewrite module for Ferron.
//!
//! Provides the `rewrite` and `rewrite_log` directives for URL rewriting
//! based on regular expression patterns.

mod config;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::ServerConfigurationValue;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::StageConstraint;
use ferron_http::HttpContext;
use ferron_observability::{Event, MetricEvent, MetricType, MetricValue};

use crate::config::{
    apply_rewrite_rules, is_rewrite_log_enabled, parse_rewrite_config, RewriteResult,
};
use crate::validator::RewriteValidator;

/// Module loader for the HTTP rewrite module.
#[derive(Default)]
pub struct HttpRewriteModuleLoader;

impl ModuleLoader for HttpRewriteModuleLoader {
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
        registry.with_stage::<HttpContext, _>(|| Arc::new(RewriteStage))
    }
}

/// Pipeline stage that applies URL rewrite rules from configuration.
struct RewriteStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for RewriteStage {
    fn name(&self) -> &str {
        "rewrite"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
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
        let rules = parse_rewrite_config(&ctx.configuration);
        if rules.is_empty() {
            return Ok(true);
        }

        // We need a mutable request reference to mutate the URI
        let Some(req) = ctx.req.as_mut() else {
            return Ok(true);
        };

        let original_url = format!(
            "{}{}",
            req.uri().path(),
            req.uri().query().map_or(String::new(), |q| format!("?{q}"))
        );

        // Extract root directory for file/directory constraint checks
        let root = ctx
            .configuration
            .get_value("root", true)
            .and_then(|v| match v {
                ServerConfigurationValue::String(s, _) => Some(s.clone()),
                _ => None,
            });

        let result = apply_rewrite_rules(&original_url, &rules, root.as_deref());

        let rewritten = match result {
            RewriteResult::NoMatch => return Ok(true),
            RewriteResult::InvalidRewrite => {
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.rewrite.invalid",
                    attributes: vec![],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "Rewrite rules that produced an invalid path (400 response).",
                    ),
                }));
                return Ok(false);
            }
            RewriteResult::Rewritten(url) => url,
        };

        // Check rewrite_log
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
            ctx.events.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    target: "ferron-rewrite",
                    level: ferron_observability::LogLevel::Info,
                    message: format!("URL rewritten from \"{original_url}\" to \"{rewritten}\""),
                },
            ));
        }

        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.rewrite.rewrites_applied",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("URLs successfully rewritten."),
        }));

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
    use rustc_hash::FxHashMap;
    use std::collections::HashMap as StdHashMap;
    use typemap_rev::TypeMap;

    fn make_test_context(path: &str, config: Option<LayeredConfiguration>) -> HttpContext {
        let req: HttpRequest = Request::builder()
            .uri(path)
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: config.unwrap_or_default(),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "192.0.2.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_rewrite_config(
        rules: Vec<(&str, &str, Option<ServerConfigurationBlock>)>,
    ) -> LayeredConfiguration {
        let mut directives = StdHashMap::new();
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
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    fn make_options_block(options: &[(&str, bool)]) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();
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
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[tokio::test]
    async fn rewrites_url_with_simple_rule() {
        let config = make_rewrite_config(vec![("^/old/(.*)", "/new/$1", None)]);
        let mut ctx = make_test_context("/old/path", Some(config));
        let stage = RewriteStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/new/path");
    }

    #[tokio::test]
    async fn no_rules_is_noop() {
        let mut ctx = make_test_context("/any/path", None);
        let stage = RewriteStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/any/path");
    }

    #[tokio::test]
    async fn preserves_query_string() {
        let config = make_rewrite_config(vec![("^/api/(.*)", "/v2/$1", None)]);
        let mut ctx = make_test_context("/api/users?page=2", Some(config));
        let stage = RewriteStage;
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
        let stage = RewriteStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.req.as_ref().unwrap().uri().path(), "/b/test");
    }

    #[tokio::test]
    async fn sets_original_uri() {
        let config = make_rewrite_config(vec![("^/x/(.*)", "/y/$1", None)]);
        let mut ctx = make_test_context("/x/foo", Some(config));
        let stage = RewriteStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert!(ctx.original_uri.is_some());
        assert_eq!(ctx.original_uri.as_ref().unwrap().path(), "/x/foo");
    }
}
