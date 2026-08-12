//! HTTP canary module for Ferron.
//!
//! Provides weighted, sticky variant selection for canary rollouts and A/B
//! testing. The `canary` directive assigns each request to a named variant
//! through a consistent hash ring and exposes the selected variant as
//! interpolation variables (`canary.variant`, `canary.weight`, `canary.key`).
//! With `set_cookie`, Ferron persists the sticky key in a cookie on the
//! response, so the assignment survives client IP changes.
//!
//! The ring uses consistent hashing, so changing variant weights only moves
//! the requests whose nearest virtual node was added or removed. Existing
//! assignments survive weight changes and configuration reloads.

mod config;
mod ring;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::variables::{resolve_variable, var};
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue, TraceAttributeValue,
};
use parking_lot::RwLock;
use typemap_rev::TypeMapKey;

use crate::config::{parse_canary_config, CanaryAffinity, CanaryConfig};
use crate::ring::VariantRing;
use crate::validator::CanaryValidator;

/// Variable holding the selected variant name.
const CANARY_VARIANT_VAR: &str = "canary.variant";

/// Variable holding the selected variant's configured weight.
const CANARY_WEIGHT_VAR: &str = "canary.weight";

/// Variable holding the sticky key value used for assignment.
const CANARY_KEY_VAR: &str = "canary.key";

/// Pending `Set-Cookie` to apply to the response in the inverse phase.
///
/// The canary stage runs before the response exists, so the cookie is
/// staged here and written by [`CanaryStage::run_inverse`].
struct PendingCanaryCookie {
    name: String,
    value: String,
}

impl TypeMapKey for PendingCanaryCookie {
    type Value = PendingCanaryCookie;
}

/// Resolved sticky key for a request.
struct AffinityKey {
    /// The value hashed into the variant ring.
    value: String,
    /// Where the key came from: `ip`, `cookie`, `header`, `hash`, or `generated`.
    source: &'static str,
    /// Whether Ferron must persist the key in the affinity cookie.
    set_cookie: bool,
}

/// Compiled canary configuration for a specific configuration generation.
struct CanaryCompiled {
    config: CanaryConfig,
    ring: VariantRing,
}

/// Shared cache of compiled canary rings, keyed by configuration layer identity.
#[derive(Default)]
struct CanaryState {
    /// Rings are cached per configuration generation and cleared on reload,
    /// so the expensive virtual node construction never runs per request.
    rings: RwLock<HashMap<Vec<usize>, Arc<CanaryCompiled>>>,
}

impl CanaryState {
    #[inline]
    fn clear(&self) {
        self.rings.write().clear();
    }
}

/// Module loader for the HTTP canary module.
#[derive(Default)]
pub struct HttpCanaryModuleLoader {
    state: Option<Arc<CanaryState>>,
}

impl ModuleLoader for HttpCanaryModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "canary",
                    usage: "canary <name> { ... }",
                    description: "This directive assigns each request to a named variant through a consistent hash ring and exposes it as a variable, for canary rollouts and A/B testing.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_canary")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "affinity",
                    usage: "affinity ip | cookie <name> | header <name> | hash <variable>",
                    description: "This directive selects the stickiness source for canary variant assignment.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_canary"),
            )
            .register(
                Directive {
                    name: "variant",
                    usage: "variant <name> <weight>",
                    description: "This directive defines a canary variant and its relative weight.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_canary"),
            );
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<&'static str, Vec<Box<dyn ConfigurationValidator>>>,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(CanaryValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let state = Arc::new(CanaryState::default());
        self.state = Some(Arc::clone(&state));
        registry.with_stage::<HttpContext, _>(move || {
            Arc::new(CanaryStage {
                state: Arc::clone(&state),
            })
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        _modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Rings are keyed by configuration layer pointer identity; drop them
        // on reload so the next request rebuilds against the new generation.
        if let Some(state) = &self.state {
            state.clear();
        }
        Ok(())
    }
}

/// Pipeline stage that assigns each request to a canary variant.
struct CanaryStage {
    state: Arc<CanaryState>,
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for CanaryStage {
    fn name(&self) -> &str {
        "canary"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run after the client IP is finalized and before `set_var`/`map`, so
        // downstream stages can branch on the `canary.*` variables.
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("variables".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("canary"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let entries = parse_canary_config(&ctx.configuration);
        let Some(config) = entries.into_iter().next_back() else {
            return Ok(true);
        };

        // Use the layer Arc pointer identities as a cache key. When the
        // configuration is reloaded, new Arc pointers are created, so the
        // ring cache entries are naturally invalidated (and cleared via
        // register_modules).
        let config_key = ctx
            .configuration
            .layers
            .iter()
            .filter_map(|arc| {
                if arc.has_directive("canary") {
                    Some(Arc::as_ptr(arc) as usize)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let compiled = self.state.get_or_insert(&config_key, &config);

        let resolution = affinity_key(&compiled.config.affinity, ctx, compiled.config.set_cookie);
        let variant_index = compiled.ring.get(resolution.value.as_bytes());
        let Some(variant) = variant_index.and_then(|i| compiled.config.variants.get(i)) else {
            // No variants configured — nothing to branch on.
            return Ok(true);
        };

        let variant_name = variant.name.clone();
        if resolution.set_cookie {
            if let CanaryAffinity::Cookie(name) = &compiled.config.affinity {
                ctx.extensions
                    .insert::<PendingCanaryCookie>(PendingCanaryCookie {
                        name: name.clone(),
                        value: resolution.value.clone(),
                    });
            }
        }
        ctx.variables
            .insert(CANARY_VARIANT_VAR.to_string(), variant_name.clone());
        ctx.variables
            .insert(CANARY_WEIGHT_VAR.to_string(), variant.weight.to_string());
        ctx.variables
            .insert(CANARY_KEY_VAR.to_string(), resolution.value);

        custom_access_log_fields(ctx).insert(
            "ferron.canary.variant".into(),
            CustomAccessLogField::String(variant_name.clone()),
        );

        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.canary.requests",
            attributes: vec![
                (
                    "ferron.canary.name",
                    MetricAttributeValue::String(compiled.config.name.clone()),
                ),
                (
                    "ferron.canary.variant",
                    MetricAttributeValue::String(variant_name.clone()),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Number of requests assigned to a canary variant."),
            trace_context: current_event_trace_context(ctx),
        }));

        ctx.get_span_attributes().insert(
            "ferron.canary.variant",
            TraceAttributeValue::String(variant_name),
        );
        ctx.get_span_attributes().insert(
            "ferron.canary.name",
            TraceAttributeValue::String(compiled.config.name.clone()),
        );
        ctx.get_span_attributes().insert(
            "ferron.canary.key_source",
            TraceAttributeValue::String(resolution.source.to_string()),
        );
        ctx.get_span_attributes().insert(
            "ferron.canary.weight",
            TraceAttributeValue::I64(variant.weight as i64),
        );

        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let Some(pending) = ctx.extensions.remove::<PendingCanaryCookie>() else {
            return Ok(());
        };

        let cookie_value = format!("{}={}; Path=/", pending.name, pending.value);
        let Ok(header_value) = http::HeaderValue::from_str(&cookie_value) else {
            return Ok(());
        };

        if ctx.res.is_none() {
            ctx.res = Some(HttpResponse::BuiltinError(404, None));
        }

        match &mut ctx.res {
            Some(HttpResponse::Custom(resp)) => {
                resp.headers_mut()
                    .insert(http::header::SET_COOKIE, header_value);
            }
            Some(HttpResponse::BuiltinError(_, headers)) => {
                headers
                    .get_or_insert(http::HeaderMap::default())
                    .insert(http::header::SET_COOKIE, header_value);
            }
            _ => {}
        }

        Ok(())
    }
}

impl CanaryState {
    /// Get the compiled canary configuration for the given config key,
    /// building (and caching) the consistent hash ring on a miss.
    #[inline]
    fn get_or_insert(&self, config_key: &Vec<usize>, config: &CanaryConfig) -> Arc<CanaryCompiled> {
        {
            let cache = self.rings.read();
            if let Some(compiled) = cache.get(config_key) {
                return Arc::clone(compiled);
            }
        }

        let compiled = Arc::new(CanaryCompiled {
            config: config.clone(),
            ring: VariantRing::new(&config.variants),
        });
        self.rings
            .write()
            .insert(config_key.clone(), Arc::clone(&compiled));
        compiled
    }
}

/// Resolve the sticky key for the configured affinity source.
///
/// When the source is missing (no cookie, no header, unresolvable variable),
/// the client IP is hashed instead, so every client still receives a stable
/// variant. With `set_cookie` enabled and a missing cookie, a random key is
/// generated instead and flagged for persistence in the response cookie.
#[inline]
fn affinity_key(affinity: &CanaryAffinity, ctx: &HttpContext, set_cookie: bool) -> AffinityKey {
    let candidate = match affinity {
        CanaryAffinity::Ip => None,
        CanaryAffinity::Cookie(name) => Some(resolve_variable(
            &format!("{}{}", var::REQUEST_COOKIE_PREFIX, name),
            ctx,
        )),
        CanaryAffinity::Header(name) => Some(resolve_variable(
            &format!("{}{}", var::REQUEST_HEADER_PREFIX, name),
            ctx,
        )),
        CanaryAffinity::Hash(variable) => Some(resolve_variable(variable, ctx)),
    };

    match candidate {
        Some(Some(value)) if !value.is_empty() && !is_unresolved(&value, affinity, ctx) => {
            AffinityKey {
                value,
                source: affinity_source(affinity),
                set_cookie: false,
            }
        }
        Some(None) if set_cookie && matches!(affinity, CanaryAffinity::Cookie(_)) => {
            // No cookie in the request: generate a fresh sticky key and
            // persist it in a Set-Cookie response header.
            let random_bytes: [u8; 16] = rand::random();
            AffinityKey {
                value: random_bytes
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                source: "generated",
                set_cookie: true,
            }
        }
        _ => AffinityKey {
            value: resolve_variable(var::REMOTE_IP, ctx).unwrap_or_default(),
            source: "ip",
            set_cookie: false,
        },
    }
}

/// Plain-text source label for the span attribute.
#[inline]
fn affinity_source(affinity: &CanaryAffinity) -> &'static str {
    match affinity {
        CanaryAffinity::Ip => "ip",
        CanaryAffinity::Cookie(_) => "cookie",
        CanaryAffinity::Header(_) => "header",
        CanaryAffinity::Hash(_) => "hash",
    }
}

/// Detect variable names that `resolve_variable` left unresolved (it echoes
/// the name itself as a fallback for custom variables).
#[inline]
fn is_unresolved(value: &str, affinity: &CanaryAffinity, ctx: &HttpContext) -> bool {
    match affinity {
        CanaryAffinity::Hash(variable) => {
            value == variable && !ctx.variables.contains_key(variable)
        }
        _ => false,
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
            routing_uri: None,
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

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
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

    fn make_canary_config(affinity: &[&str], variants: &[(&str, i64)]) -> LayeredConfiguration {
        make_canary_config_with_set_cookie(affinity, variants, false)
    }

    fn make_canary_config_with_set_cookie(
        affinity: &[&str],
        variants: &[(&str, i64)],
        set_cookie: bool,
    ) -> LayeredConfiguration {
        let mut block_directives = StdHashMap::new();
        if !affinity.is_empty() {
            block_directives.insert(
                "affinity".to_string(),
                vec![make_entry(
                    affinity.iter().map(|a| make_value_string(a)).collect(),
                    None,
                )],
            );
        }
        block_directives.insert(
            "variant".to_string(),
            variants
                .iter()
                .map(|(name, weight)| {
                    make_entry(
                        vec![make_value_string(name), make_value_number(*weight)],
                        None,
                    )
                })
                .collect(),
        );
        if set_cookie {
            block_directives.insert("set_cookie".to_string(), vec![make_entry(vec![], None)]);
        }

        let mut top_directives = StdHashMap::new();
        top_directives.insert(
            "canary".to_string(),
            vec![make_entry(
                vec![make_value_string("ab_test")],
                Some(ServerConfigurationBlock {
                    directives: Arc::new(block_directives),
                    matchers: StdHashMap::new(),
                    span: None,
                }),
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

    #[tokio::test]
    async fn no_canary_directive_is_noop() {
        let mut ctx = make_test_context("/any", None);
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(!ctx.variables.contains_key(CANARY_VARIANT_VAR));
    }

    #[tokio::test]
    async fn ip_affinity_sets_variant_variables() {
        let config = make_canary_config(&["ip"], &[("stable", 90), ("new", 10)]);
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);

        let variant = ctx.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();
        assert!(variant == "stable" || variant == "new");
        assert!(ctx
            .variables
            .get(CANARY_WEIGHT_VAR)
            .is_some_and(|w| w == "90" || w == "10"));
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"192.0.2.1".to_string())
        );
    }

    #[tokio::test]
    async fn ip_affinity_is_sticky() {
        let config = make_canary_config(&["ip"], &[("stable", 50), ("new", 50)]);

        let mut ctx = make_test_context("/any", Some(config.clone()));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        let first = ctx.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();

        // Same client IP must keep the same variant on the next request.
        let mut ctx2 = make_test_context("/other", Some(config));
        stage.run(&mut ctx2).await.unwrap();
        let second = ctx2.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn cookie_affinity_uses_cookie_value() {
        let config = make_canary_config(&["cookie", "ab_variant"], &[("stable", 1), ("new", 1)]);
        let mut ctx = make_test_context("/any", Some(config.clone()));
        if let Some(req) = &mut ctx.req {
            req.headers_mut()
                .insert("cookie", "ab_variant=experiment-42".parse().unwrap());
        }
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"experiment-42".to_string())
        );
        let variant = ctx.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();

        // The same cookie value must map to the same variant.
        let mut ctx2 = make_test_context("/other", Some(config));
        if let Some(req) = &mut ctx2.req {
            req.headers_mut()
                .insert("cookie", "ab_variant=experiment-42".parse().unwrap());
        }
        stage.run(&mut ctx2).await.unwrap();
        assert_eq!(ctx2.variables.get(CANARY_VARIANT_VAR).unwrap(), &variant);
    }

    #[tokio::test]
    async fn cookie_affinity_falls_back_to_ip_when_cookie_missing() {
        let config = make_canary_config(&["cookie", "ab_variant"], &[("stable", 1), ("new", 1)]);
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"192.0.2.1".to_string())
        );
    }

    #[tokio::test]
    async fn header_affinity_uses_header_value() {
        let config = make_canary_config(&["header", "X-Experiment"], &[("stable", 1), ("new", 1)]);
        let mut ctx = make_test_context("/any", Some(config));
        if let Some(req) = &mut ctx.req {
            req.headers_mut()
                .insert("x-experiment", "group-b".parse().unwrap());
        }
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"group-b".to_string())
        );
    }

    #[tokio::test]
    async fn hash_affinity_hashes_query_parameter() {
        let config = make_canary_config(
            &["hash", "request.uri.query.bucket"],
            &[("stable", 1), ("new", 1)],
        );
        let mut ctx = make_test_context("/any?bucket=alpha", Some(config.clone()));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        let variant = ctx.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();

        let mut ctx2 = make_test_context("/any?bucket=alpha", Some(config));
        stage.run(&mut ctx2).await.unwrap();
        assert_eq!(ctx2.variables.get(CANARY_VARIANT_VAR).unwrap(), &variant);
    }

    #[tokio::test]
    async fn hash_affinity_falls_back_to_ip_for_unresolvable_variable() {
        let config = make_canary_config(
            &["hash", "some.missing.variable"],
            &[("stable", 1), ("new", 1)],
        );
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"192.0.2.1".to_string())
        );
    }

    #[tokio::test]
    async fn is_applicable_with_canary_directive() {
        let config = make_canary_config(&["ip"], &[("stable", 1)]);
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        assert!(stage.is_applicable(config.layers.first().map(|l| l.as_ref())));
    }

    #[tokio::test]
    async fn is_not_applicable_without_canary_directive() {
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        assert!(!stage.is_applicable(None));
    }

    #[tokio::test]
    async fn weight_stickiness_survives_config_reload() {
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };

        // First generation: 90/10 split.
        let gen1 = make_canary_config(&["ip"], &[("stable", 90), ("new", 10)]);
        let mut ctx = make_test_context("/any", Some(gen1));
        stage.run(&mut ctx).await.unwrap();
        let first_variant = ctx.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();

        // Second generation: identical weights, fresh layer pointers
        // (simulating a reload where the configuration did not change).
        let gen2 = make_canary_config(&["ip"], &[("stable", 90), ("new", 10)]);
        let mut ctx2 = make_test_context("/any", Some(gen2));
        stage.run(&mut ctx2).await.unwrap();
        let second_variant = ctx2.variables.get(CANARY_VARIANT_VAR).cloned().unwrap();

        // Identical configuration must keep identical assignments.
        assert_eq!(first_variant, second_variant);
    }

    #[tokio::test]
    async fn set_cookie_generates_key_and_persists_cookie() {
        let config = make_canary_config_with_set_cookie(
            &["cookie", "ab_variant"],
            &[("stable", 1), ("new", 1)],
            true,
        );
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        stage.run_inverse(&mut ctx).await.unwrap();

        // A random key was generated and used for the assignment.
        let key = ctx.variables.get(CANARY_KEY_VAR).cloned().unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));

        // The response carries the affinity cookie.
        let cookie = ctx
            .res
            .as_ref()
            .and_then(|res| match res {
                HttpResponse::Custom(resp) => resp
                    .headers()
                    .get(http::header::SET_COOKIE)
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_string),
                HttpResponse::BuiltinError(_, headers) => headers
                    .as_ref()
                    .and_then(|map| map.get(http::header::SET_COOKIE))
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_string),
                _ => None,
            })
            .expect("Set-Cookie header expected");
        assert_eq!(cookie, format!("ab_variant={key}; Path=/"));

        // The generated key must map to the same variant on the next request
        // when the client sends the cookie back.
        let mut ctx2 = make_test_context(
            "/other",
            Some(make_canary_config_with_set_cookie(
                &["cookie", "ab_variant"],
                &[("stable", 1), ("new", 1)],
                true,
            )),
        );
        if let Some(req) = &mut ctx2.req {
            req.headers_mut()
                .insert("cookie", format!("ab_variant={key}").parse().unwrap());
        }
        stage.run(&mut ctx2).await.unwrap();
        assert_eq!(
            ctx2.variables.get(CANARY_VARIANT_VAR),
            ctx.variables.get(CANARY_VARIANT_VAR)
        );
        assert_eq!(
            ctx2.variables.get(CANARY_KEY_VAR).unwrap(),
            &ctx.variables.get(CANARY_KEY_VAR).unwrap().clone()
        );
    }

    #[tokio::test]
    async fn set_cookie_generates_fresh_keys_per_client() {
        let config = make_canary_config_with_set_cookie(
            &["cookie", "ab_variant"],
            &[("stable", 1), ("new", 1)],
            true,
        );
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };

        let mut ctx = make_test_context("/any", Some(config.clone()));
        stage.run(&mut ctx).await.unwrap();
        let first = ctx.variables.get(CANARY_KEY_VAR).cloned().unwrap();

        let mut ctx2 = make_test_context("/any", Some(config));
        stage.run(&mut ctx2).await.unwrap();
        let second = ctx2.variables.get(CANARY_KEY_VAR).cloned().unwrap();

        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn set_cookie_skips_existing_cookie() {
        let config = make_canary_config_with_set_cookie(
            &["cookie", "ab_variant"],
            &[("stable", 1), ("new", 1)],
            true,
        );
        let mut ctx = make_test_context("/any", Some(config));
        if let Some(req) = &mut ctx.req {
            req.headers_mut()
                .insert("cookie", "ab_variant=experiment-42".parse().unwrap());
        }
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        stage.run_inverse(&mut ctx).await.unwrap();

        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"experiment-42".to_string())
        );
        assert!(ctx
            .res
            .as_ref()
            .and_then(|res| match res {
                HttpResponse::Custom(resp) => resp.headers().get(http::header::SET_COOKIE),
                _ => None,
            })
            .is_none());
    }

    #[tokio::test]
    async fn set_cookie_disabled_keeps_ip_fallback() {
        let config = make_canary_config(&["cookie", "ab_variant"], &[("stable", 1), ("new", 1)]);
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        stage.run_inverse(&mut ctx).await.unwrap();

        // Without set_cookie the fallback key is the client IP and no
        // Set-Cookie header is emitted.
        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"192.0.2.1".to_string())
        );
        assert!(ctx.res.as_ref().is_none());
    }

    #[tokio::test]
    async fn set_cookie_without_cookie_affinity_never_generates() {
        let config =
            make_canary_config_with_set_cookie(&["ip"], &[("stable", 1), ("new", 1)], true);
        let mut ctx = make_test_context("/any", Some(config));
        let stage = CanaryStage {
            state: Arc::new(CanaryState::default()),
        };
        stage.run(&mut ctx).await.unwrap();
        stage.run_inverse(&mut ctx).await.unwrap();

        assert_eq!(
            ctx.variables.get(CANARY_KEY_VAR),
            Some(&"192.0.2.1".to_string())
        );
        assert!(ctx.res.as_ref().is_none());
    }
}
