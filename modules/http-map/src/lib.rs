//! HTTP map module for Ferron.
//!
//! Provides the `map` directive for creating variables whose values depend
//! on values of other variables.

mod config;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::StageConstraint;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::HttpContext;
use ferron_observability::TraceAttributeValue;

use crate::config::evaluate_map_directives;
use crate::validator::MapValidator;

/// Module loader for the HTTP map module.
#[derive(Default)]
pub struct HttpMapModuleLoader;

impl ModuleLoader for HttpMapModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "map",
                    usage: "map <source> <destination> { ... }",
                    description: "This directive maps a source variable to a destination variable using match rules with default value support.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_map")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "default",
                    usage: "default <value>",
                    description: "This directive sets the default value for a map block when no pattern matches.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_map"),
            );
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
            .push(Box::new(MapValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(MapStage))
    }
}

/// Pipeline stage that evaluates `map` directives and populates destination variables.
struct MapStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for MapStage {
    fn name(&self) -> &str {
        "map"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("headers".to_string()),
            StageConstraint::Before("rewrite".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("map"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let mappings = evaluate_map_directives(&ctx.configuration, ctx);
        let edited = !mappings.is_empty();
        for (name, value) in mappings {
            ctx.variables.insert(name.clone(), value);
            ctx.get_span_attributes()
                .insert("ferron.map.variable", TraceAttributeValue::String(name));
        }
        ctx.get_span_attributes()
            .insert("ferron.map.edited", TraceAttributeValue::Bool(edited));
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

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_map_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_map_config(
        source: &str,
        destination: &str,
        default: Option<&str>,
        exact_entries: Vec<ServerConfigurationDirectiveEntry>,
        regex_entries: Vec<ServerConfigurationDirectiveEntry>,
    ) -> LayeredConfiguration {
        let mut directives = StdHashMap::new();

        if let Some(d) = default {
            directives.insert(
                "default".to_string(),
                vec![make_map_entry(vec![make_value_string(d)], None)],
            );
        }

        if !exact_entries.is_empty() {
            directives.insert("exact".to_string(), exact_entries);
        }

        if !regex_entries.is_empty() {
            directives.insert("regex".to_string(), regex_entries);
        }

        let map_block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        };

        let mut top_directives = StdHashMap::new();
        top_directives.insert(
            "map".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string(source), make_value_string(destination)],
                children: Some(map_block),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(top_directives),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    #[tokio::test]
    async fn evaluates_map_with_exact_match() {
        let config = make_map_config(
            "request.uri.path",
            "category",
            Some("default"),
            vec![make_map_entry(
                vec![make_value_string("/api"), make_value_string("api")],
                None,
            )],
            vec![],
        );
        let mut ctx = make_test_context("/api", Some(config));
        let stage = MapStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.variables.get("category"), Some(&"api".to_string()));
    }

    #[tokio::test]
    async fn evaluates_map_with_wildcard() {
        let config = make_map_config(
            "request.uri.path",
            "category",
            Some("default"),
            vec![make_map_entry(
                vec![make_value_string("/api/*"), make_value_string("api")],
                None,
            )],
            vec![],
        );
        let mut ctx = make_test_context("/api/users", Some(config));
        let stage = MapStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.variables.get("category"), Some(&"api".to_string()));
    }

    #[tokio::test]
    async fn evaluates_map_with_regex_captures() {
        let config = make_map_config(
            "request.uri.path",
            "user_id",
            Some(""),
            vec![],
            vec![make_map_entry(
                vec![
                    make_value_string("^/users/([0-9]+)"),
                    make_value_string("$1"),
                ],
                None,
            )],
        );
        let mut ctx = make_test_context("/users/42", Some(config));
        let stage = MapStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.variables.get("user_id"), Some(&"42".to_string()));
    }

    #[tokio::test]
    async fn map_sets_default_when_no_match() {
        let config = make_map_config(
            "request.uri.path",
            "category",
            Some("uncategorized"),
            vec![make_map_entry(
                vec![make_value_string("/api"), make_value_string("api")],
                None,
            )],
            vec![],
        );
        let mut ctx = make_test_context("/blog", Some(config));
        let stage = MapStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get("category"),
            Some(&"uncategorized".to_string())
        );
    }

    #[tokio::test]
    async fn map_sets_empty_string_when_no_default() {
        let config = make_map_config(
            "request.uri.path",
            "category",
            None,
            vec![make_map_entry(
                vec![make_value_string("/api"), make_value_string("api")],
                None,
            )],
            vec![],
        );
        let mut ctx = make_test_context("/blog", Some(config));
        let stage = MapStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("category"), Some(&String::new()));
    }

    #[tokio::test]
    async fn map_case_insensitive_regex() {
        let mut opts = StdHashMap::new();
        opts.insert(
            "case_insensitive".to_string(),
            vec![make_map_entry(vec![make_value_bool(true)], None)],
        );
        let regex_entry = make_map_entry(
            vec![make_value_string("^/api/.*"), make_value_string("api")],
            Some(ServerConfigurationBlock {
                directives: Arc::new(opts),
                matchers: StdHashMap::new(),
                span: None,
            }),
        );

        let mut directives = StdHashMap::new();
        directives.insert("regex".to_string(), vec![regex_entry]);

        let map_block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        };

        let mut top_directives = StdHashMap::new();
        top_directives.insert(
            "map".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("request.uri.path"),
                    make_value_string("category"),
                ],
                children: Some(map_block),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(top_directives),
            matchers: StdHashMap::new(),
            span: None,
        }));

        let mut ctx = make_test_context("/API/USERS", Some(config));
        let stage = MapStage;
        let _ = stage.run(&mut ctx).await.unwrap();
        assert_eq!(ctx.variables.get("category"), Some(&"api".to_string()));
    }
}
