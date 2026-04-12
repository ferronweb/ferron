//! Pipeline stage for HTTP response body string replacement.
//!
//! Provides the `replace`, `replace_last_modified`, and `replace_filter_types`
//! directives for modifying response bodies on the fly.

use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, MetricEvent, MetricType, MetricValue};
use http::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LAST_MODIFIED};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::BodyExt;
use typemap_rev::TypeMapKey;

use crate::body_replacer::BodyReplacer;
use crate::config::{ReplaceConfig, ReplaceRule};

/// TypeMap key for passing replace configuration from run() to run_inverse().
struct ReplaceStateKey;

impl TypeMapKey for ReplaceStateKey {
    type Value = ReplaceState;
}

/// State passed between run() and run_inverse() for replacement.
struct ReplaceState {
    config: ReplaceConfig,
}

/// Pipeline stage for HTTP response body string replacement.
#[derive(Default)]
pub struct HttpReplaceStage;

impl HttpReplaceStage {
    pub fn new() -> Self {
        Self
    }

    /// Apply replacement rules to a response body.
    ///
    /// Wraps the response body with a chain of BodyReplacer instances,
    /// one for each configured replace rule.
    fn apply_replacement(
        response: http::Response<UnsyncBoxBody<Bytes, io::Error>>,
        rules: &[ReplaceRule],
    ) -> http::Response<UnsyncBoxBody<Bytes, io::Error>> {
        let (mut parts, body) = response.into_parts();

        // Remove Content-Length since body size will change
        parts.headers.remove(CONTENT_LENGTH);

        // Apply replacement by chaining BodyReplacer wrappers
        let mut wrapped_body = body;
        for rule in rules.iter().rev() {
            let replacer = BodyReplacer::new(&rule.searched, &rule.replacement, rule.once);
            wrapped_body = replacer.wrap(wrapped_body).boxed_unsync();
        }

        http::Response::from_parts(parts, wrapped_body)
    }

    /// Check if the response MIME type matches any of the filter types.
    fn matches_mime_type(
        content_type: Option<&http::HeaderValue>,
        filter_types: &[String],
    ) -> bool {
        let Some(ct) = content_type else {
            return false;
        };

        // Parse MIME type from Content-Type header (ignore parameters like charset)
        let ct_str = String::from_utf8_lossy(ct.as_bytes());
        let mime_type = ct_str.split(';').next().unwrap_or(&ct_str).trim();

        for filter in filter_types {
            if filter == "*" || filter == mime_type {
                return true;
            }
        }

        false
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpReplaceStage {
    fn name(&self) -> &str {
        "replace"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            // Run after compression to avoid corrupting compressed data
            StageConstraint::After("dynamic_compression".to_string()),
            // Run before caching so cached content is already replaced
            StageConstraint::Before("cache".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        let Some(c) = config else {
            return false;
        };
        c.has_directive("replace")
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // Parse configuration and store for run_inverse()
        let config = ReplaceConfig::from_http_context(ctx);

        // Only continue if there are replacement rules configured
        if config.rules.is_empty() {
            return Ok(true);
        }

        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        Ok(true)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        // Retrieve configuration from extensions
        let Some(state) = ctx.extensions.remove::<ReplaceStateKey>() else {
            return Ok(());
        };

        let config = state.config;
        if config.rules.is_empty() {
            return Ok(());
        }

        // Only process Custom responses
        let response = match ctx.res.take() {
            Some(HttpResponse::Custom(resp)) => resp,
            Some(res) => {
                ctx.res = Some(res);
                return Ok(());
            }
            None => return Ok(()),
        };

        // Skip if response has Content-Encoding (compressed data)
        if response.headers().contains_key(CONTENT_ENCODING) {
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.replace.skipped_compressed",
                attributes: vec![],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{response}"),
                description: Some("Responses skipped due to Content-Encoding header."),
            }));
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        // Check MIME type filter
        let content_type = response.headers().get(CONTENT_TYPE);
        if !Self::matches_mime_type(content_type, &config.filter_types) {
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.replace.skipped_mime",
                attributes: vec![],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{response}"),
                description: Some("Responses skipped due to MIME type mismatch."),
            }));
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        // Apply replacement
        let mut replaced_response = Self::apply_replacement(response, &config.rules);

        // Handle Last-Modified header
        if !config.preserve_last_modified {
            replaced_response.headers_mut().remove(LAST_MODIFIED);
        }

        // Clear the extensions for zerocopy to not interfere with the modified response body
        replaced_response.extensions_mut().clear();

        ctx.res = Some(HttpResponse::Custom(replaced_response));

        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.replace.replacements_applied",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{response}"),
            description: Some("Responses successfully modified."),
        }));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Full};
    use rustc_hash::FxHashMap;
    use std::collections::HashMap;
    use std::sync::Arc;
    use typemap_rev::TypeMap;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_layered_config(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> LayeredConfiguration {
        let block = Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        });
        let mut config = LayeredConfiguration::new();
        config.add_layer(block);
        config
    }

    fn make_context(
        res: Option<HttpResponse>,
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> HttpContext {
        let req: ferron_http::HttpRequest = Request::builder()
            .uri("/test")
            .body(
                Full::new(Bytes::from(""))
                    .map_err(|e: std::convert::Infallible| match e {})
                    .boxed_unsync(),
            )
            .unwrap();

        HttpContext {
            req: Some(req),
            res,
            events: CompositeEventSink::new(Vec::new()),
            configuration: make_layered_config(directives),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "127.0.0.1:80".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_response_with_body(
        body: &str,
        content_type: Option<&str>,
        content_encoding: Option<&str>,
    ) -> http::Response<UnsyncBoxBody<Bytes, io::Error>> {
        let mut builder = http::Response::builder().status(200);
        if let Some(ct) = content_type {
            builder = builder.header(CONTENT_TYPE, ct);
        }
        if let Some(ce) = content_encoding {
            builder = builder.header(CONTENT_ENCODING, ce);
        }
        let body_bytes = Bytes::from(body.to_string());
        builder
            .body(
                Full::new(body_bytes)
                    .map_err(|e: std::convert::Infallible| match e {})
                    .boxed_unsync(),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn test_simple_replacement() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );

        let response = make_response_with_body("hello old world", Some("text/html"), None);
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives);

        // Simulate run() storing state
        let config = ReplaceConfig::from_http_context(&ctx);
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        // Collect response body
        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body_str, "hello new world");
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_skips_compressed_responses() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );

        let response = make_response_with_body("compressed data", Some("text/html"), Some("gzip"));
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives);

        let config = ReplaceConfig::from_http_context(&ctx);
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        // Response should be unchanged
        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body_str, "compressed data");
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_skips_non_matching_mime_type() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );

        let response = make_response_with_body("old data", Some("application/json"), None);
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives.clone());

        let mut config = ReplaceConfig::from_config(&make_layered_config(directives));
        config.filter_types = vec!["text/html".to_string()];
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body_str, "old data");
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_removes_last_modified_by_default() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );
        // No replace_last_modified directive

        let mut response = make_response_with_body("old content", Some("text/html"), None);
        response.headers_mut().insert(
            LAST_MODIFIED,
            http::HeaderValue::from_static("Wed, 21 Oct 2024 07:28:00 GMT"),
        );
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives);

        let config = ReplaceConfig::from_http_context(&ctx);
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            assert!(!response.headers().contains_key(LAST_MODIFIED));
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_preserves_last_modified_when_configured() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );
        let mut child = HashMap::new();
        child.insert(
            "once".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(false)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let mut response = make_response_with_body("old content", Some("text/html"), None);
        response.headers_mut().insert(
            LAST_MODIFIED,
            http::HeaderValue::from_static("Wed, 21 Oct 2024 07:28:00 GMT"),
        );
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives);

        let config = ReplaceConfig::from_http_context(&ctx);
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            assert!(response.headers().contains_key(LAST_MODIFIED));
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body_str, "new content");
        } else {
            panic!("Expected Custom response");
        }
    }

    #[tokio::test]
    async fn test_multiple_replacements() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![
                ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("foo"), make_value_string("bar")],
                    children: None,
                    span: None,
                },
                ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("baz"), make_value_string("qux")],
                    children: None,
                    span: None,
                },
            ],
        );

        let response = make_response_with_body("foo and baz", Some("text/html"), None);
        let mut ctx = make_context(Some(HttpResponse::Custom(response)), directives);

        let config = ReplaceConfig::from_http_context(&ctx);
        ctx.extensions
            .insert::<ReplaceStateKey>(ReplaceState { config });

        let stage = HttpReplaceStage::new();
        let result = stage.run_inverse(&mut ctx).await;
        assert!(result.is_ok());

        if let Some(HttpResponse::Custom(response)) = ctx.res.take() {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body_str, "bar and qux");
        } else {
            panic!("Expected Custom response");
        }
    }
}
