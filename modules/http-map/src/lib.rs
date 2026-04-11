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
use ferron_http::HttpContext;

use crate::config::evaluate_map_directives;
use crate::validator::MapValidator;

/// Module loader for the HTTP map module.
#[derive(Default)]
pub struct HttpMapModuleLoader;

impl ModuleLoader for HttpMapModuleLoader {
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
        for (name, value) in mappings {
            ctx.variables.insert(name, value);
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
    async fn no_map_directives_is_noop() {
        let mut ctx = make_test_context("/any/path", None);
        let stage = MapStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.variables.is_empty());
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
    async fn is_applicable_returns_true_with_map_directive() {
        let config = make_map_config("request.uri.path", "category", None, vec![], vec![]);
        let stage = MapStage;
        assert!(stage.is_applicable(config.layers.first().map(|l| l.as_ref())));
    }

    #[tokio::test]
    async fn is_applicable_returns_false_without_map_directive() {
        let stage = MapStage;
        assert!(!stage.is_applicable(None));
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
