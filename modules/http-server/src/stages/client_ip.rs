//! Client IP from forwarded headers stage
//!
//! Reads the `X-Forwarded-For` or `Forwarded` header (as configured via the
//! `client_ip_from_header` directive) and overwrites `ctx.remote_address` with
//! the extracted client IP when the connecting peer is in the configured
//! trusted-proxy allowlist. This is disabled by default.

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::client_ip::ClientIpFromHeaderConfig;
use ferron_http::HttpContext;
use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};
use std::net::SocketAddr;

pub struct ClientIpFromHeaderStage;

#[async_trait(?Send)]
impl Stage<HttpContext> for ClientIpFromHeaderStage {
    #[inline]
    fn name(&self) -> &str {
        "client_ip_from_header"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        // Run before https_redirect so downstream stages see the correct remote_address.
        vec![StageConstraint::Before("https_redirect".to_string())]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("client_ip_from_header"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match ClientIpFromHeaderConfig::resolve_from_context(ctx) {
            Some(c) => c,
            None => return Ok(true), // Directive not set — no-op
        };

        if !config.is_trusted_proxy(ctx.remote_address.ip()) {
            return Ok(true);
        }

        let Some(ip) = config.extract_client_ip(ctx) else {
            // Header present but couldn't be parsed — skip silently
            return Ok(true);
        };

        // Preserve the original remote port; only replace the IP.
        let original_port = ctx.remote_address.port();
        ctx.remote_address = SocketAddr::new(ip, original_port);
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.http.server.client_ip_rewrites",
            attributes: vec![(
                "ferron.client_ip.header",
                MetricAttributeValue::StaticStr(config.header_name()),
            )],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{rewrite}"),
            description: Some(
                "Number of times the client IP address was rewritten from a trusted proxy header.",
            ),
        }));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    use std::sync::Arc;
    use typemap_rev::TypeMap;

    fn make_test_context(
        x_forwarded_for: Option<&str>,
        forwarded: Option<&str>,
        config_directive: Option<&str>,
        trusted_proxies: &[&str],
    ) -> HttpContext {
        let mut builder = Request::builder().uri("/path");
        if let Some(h) = x_forwarded_for {
            builder = builder.header("x-forwarded-for", h);
        }
        if let Some(h) = forwarded {
            builder = builder.header("forwarded", h);
        }
        let req: HttpRequest = builder
            .body(
                Empty::<bytes::Bytes>::new()
                    .map_err(|e| match e {})
                    .boxed_unsync(),
            )
            .unwrap();

        let mut ctx = HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "10.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        };

        if let Some(directive) = config_directive {
            let mut directives = StdHashMap::new();
            directives.insert(
                "client_ip_from_header".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(
                        directive.to_string(),
                        None,
                    )],
                    children: if trusted_proxies.is_empty() {
                        None
                    } else {
                        let mut nested_directives = StdHashMap::new();
                        nested_directives.insert(
                            "trusted_proxy".to_string(),
                            vec![ServerConfigurationDirectiveEntry {
                                args: trusted_proxies
                                    .iter()
                                    .map(|cidr| {
                                        ServerConfigurationValue::String(cidr.to_string(), None)
                                    })
                                    .collect(),
                                children: None,
                                span: None,
                            }],
                        );
                        Some(ServerConfigurationBlock {
                            directives: Arc::new(nested_directives),
                            matchers: StdHashMap::new(),
                            span: None,
                        })
                    },
                    span: None,
                }],
            );
            ctx.configuration
                .layers
                .push(Arc::new(ServerConfigurationBlock {
                    directives: Arc::new(directives),
                    matchers: StdHashMap::new(),
                    span: None,
                }));
        }

        ctx
    }

    // ── X-Forwarded-For tests ──

    #[tokio::test]
    async fn extracts_single_ip_from_x_forwarded_for() {
        let mut ctx = make_test_context(
            Some("192.0.2.1"),
            None,
            Some("x-forwarded-for"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.1");
        assert_eq!(ctx.remote_address.port(), 12345); // original port preserved
    }

    #[tokio::test]
    async fn extracts_first_ip_from_x_forwarded_for_chain() {
        let mut ctx = make_test_context(
            Some("192.0.2.1, 10.0.0.1, 172.16.0.1"),
            None,
            Some("x-forwarded-for"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.1");
    }

    #[tokio::test]
    async fn handles_ipv6_in_x_forwarded_for() {
        let mut ctx = make_test_context(
            Some("2001:db8::1"),
            None,
            Some("x-forwarded-for"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "2001:db8::1");
    }

    #[tokio::test]
    async fn skips_when_x_forwarded_for_header_missing() {
        let mut ctx = make_test_context(None, None, Some("x-forwarded-for"), &["10.0.0.1/32"]);
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        // remote_address should be unchanged
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_directive_not_set() {
        let mut ctx = make_test_context(Some("192.0.2.1"), None, None, &[]);
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_x_forwarded_for_value_is_invalid() {
        let mut ctx = make_test_context(
            Some("not-an-ip"),
            None,
            Some("x-forwarded-for"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_remote_ip_is_not_trusted_proxy() {
        let mut ctx = make_test_context(
            Some("192.0.2.1"),
            None,
            Some("x-forwarded-for"),
            &["192.168.0.0/16"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    // ── Forwarded (RFC 7239) tests ──

    #[tokio::test]
    async fn extracts_ip_from_forwarded_for() {
        let mut ctx = make_test_context(
            None,
            Some("for=192.0.2.60;proto=https"),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.60");
    }

    #[tokio::test]
    async fn extracts_quoted_ip_from_forwarded_for() {
        let mut ctx = make_test_context(
            None,
            Some("for=\"192.0.2.60\";proto=https"),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.60");
    }

    #[tokio::test]
    async fn extracts_ipv6_from_forwarded_for() {
        let mut ctx = make_test_context(
            None,
            Some("for=\"[2001:db8::1]\""),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "2001:db8::1");
    }

    #[tokio::test]
    async fn extracts_first_forwarded_element() {
        let mut ctx = make_test_context(
            None,
            Some("for=192.0.2.60;proto=https, for=10.0.0.1;proto=http"),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.60");
    }

    #[tokio::test]
    async fn skips_when_forwarded_header_missing() {
        let mut ctx = make_test_context(None, None, Some("forwarded"), &["10.0.0.1/32"]);
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_forwarded_value_has_no_for() {
        let mut ctx = make_test_context(
            None,
            Some("proto=https;by=proxy.example.com"),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_forwarded_for_is_obfuscated() {
        let mut ctx = make_test_context(
            None,
            Some("for=_hidden"),
            Some("forwarded"),
            &["10.0.0.1/32"],
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        // "_hidden" is not an IP, so the stage should skip
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }
}
