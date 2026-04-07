//! HTTPS redirect stage
//!
//! Redirects plain HTTP requests to their HTTPS equivalent when the server
//! has TLS enabled (indicated by `ctx.https_port` being set) and the
//! request arrived over an unencrypted connection.

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use http::{HeaderValue, Response};
use http_body_util::{BodyExt, Full};

pub struct HttpsRedirectStage;

impl Default for HttpsRedirectStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

/// Returns true if the hostname is a loopback / development name that should
/// never be redirected to HTTPS (no HTTPS listener is started for it).
fn is_localhost_hostname(hostname: Option<&str>) -> bool {
    matches!(
        hostname,
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

/// Check whether the request has already been served over HTTPS by examining
/// the `X-Forwarded-Proto` header.  This prevents redirect loops when the
/// server sits behind a TLS-terminating reverse proxy.
fn is_already_https_via_header(ctx: &HttpContext) -> bool {
    ctx.req
        .as_ref()
        .and_then(|r| r.headers().get("x-forwarded-proto"))
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Build a full HTTPS URL from the current request context.
fn build_https_url(ctx: &HttpContext) -> Option<String> {
    let req = ctx.req.as_ref()?;
    let https_port = ctx.https_port?;

    // Determine the host: use the original Host header, then the ctx hostname,
    // then fall back to the local address.
    let host_req = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| ctx.hostname.clone())
        .unwrap_or_else(|| ctx.local_address.ip().to_string());
    let host = if let Some((host_split, port)) = host_req.rsplit_once(":") {
        if port.parse::<u16>().is_ok() {
            host_split.to_string()
        } else {
            host_req
        }
    } else {
        host_req
    };

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.uri().path());

    let authority = if https_port == 443 {
        host
    } else {
        format!("{host}:{https_port}")
    };

    Some(format!("https://{authority}{path_and_query}"))
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpsRedirectStage {
    #[inline]
    fn name(&self) -> &str {
        "https_redirect"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("headers".to_string()),
            StageConstraint::After("client_ip".to_string()),
        ]
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // Skip if:
        // - no https_port configured (TLS not enabled for this listener)
        // - already encrypted (came in over HTTPS)
        // - X-Forwarded-Proto: https indicates proxy already handled TLS
        // - localhost hostname (no HTTPS listener exists for it)
        // - listener port equals https_port (no separate HTTPS listener)
        let https_port = match ctx.https_port {
            Some(p) => p,
            None => return Ok(true),
        };
        if ctx.encrypted
            || is_already_https_via_header(ctx)
            || is_localhost_hostname(ctx.hostname.as_deref())
            || ctx.local_address.port() == https_port
        {
            return Ok(true);
        }

        // Check per-host configuration: `https_redirect false` disables the redirect
        let redirect_enabled = ctx
            .configuration
            .get_value("https_redirect", false)
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);

        if !redirect_enabled {
            return Ok(true);
        }

        let Some(https_url) = build_https_url(ctx) else {
            // Cannot determine target URL — let the pipeline continue
            return Ok(true);
        };

        let location = HeaderValue::from_str(&https_url)
            .map_err(|e| PipelineError::custom(format!("Invalid redirect URL: {e}")))?;

        ctx.res = Some(HttpResponse::Custom(
            Response::builder()
                .status(308) // Permanent Redirect — preserves method and body
                .header(http::header::LOCATION, location)
                .body(
                    Full::new(Bytes::from(format!(
                        "<html><body><p>308 Permanent Redirect — <a href=\"{https_url}\">click here</a></p></body></html>"
                    )))
                    .map_err(|e| match e {})
                    .boxed_unsync(),
                )
                .expect("Failed to build 308 redirect response"),
        ));

        // Stop the pipeline — response is ready.
        Ok(false)
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
    use http_body_util::Empty;
    use rustc_hash::FxHashMap;
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;
    use typemap_rev::TypeMap;

    fn make_test_context(
        host_header: Option<&str>,
        encrypted: bool,
        https_port: Option<u16>,
        hostname: Option<String>,
    ) -> HttpContext {
        let mut builder = Request::builder().uri("/path?query=1");
        if let Some(h) = host_header {
            builder = builder.header(http::header::HOST, h);
        }
        let req: HttpRequest = builder
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            encrypted,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port,
            extensions: TypeMap::new(),
        }
    }

    #[tokio::test]
    async fn redirects_http_to_https() {
        let mut ctx = make_test_context(
            Some("example.com"),
            false,
            Some(443),
            Some("example.com".to_string()),
        );
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result, "should stop the pipeline");
        if let Some(HttpResponse::Custom(resp)) = ctx.res {
            assert_eq!(resp.status(), 308);
            assert_eq!(
                resp.headers()[http::header::LOCATION],
                "https://example.com/path?query=1"
            );
        } else {
            panic!("Expected redirect response");
        }
    }

    #[tokio::test]
    async fn skips_when_already_encrypted() {
        let mut ctx = make_test_context(Some("example.com"), true, Some(443), None);
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "should continue pipeline");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn skips_when_no_https_port() {
        let mut ctx = make_test_context(Some("example.com"), false, None, None);
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "should continue pipeline");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn respects_x_forwarded_proto() {
        let mut ctx = make_test_context(Some("example.com"), false, Some(443), None);
        // Inject X-Forwarded-Proto header
        if let Some(ref mut req) = ctx.req {
            let (mut parts, body) = std::mem::replace(
                req,
                Request::builder()
                    .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
                    .unwrap(),
            )
            .into_parts();
            parts
                .headers
                .insert("x-forwarded-proto", HeaderValue::from_static("https"));
            *req = Request::from_parts(parts, body);
        }
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "should continue pipeline due to X-Forwarded-Proto");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn uses_non_standard_port() {
        let mut ctx = make_test_context(
            Some("example.com"),
            false,
            Some(8443),
            Some("example.com".to_string()),
        );
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(!result);
        if let Some(HttpResponse::Custom(resp)) = ctx.res {
            assert_eq!(
                resp.headers()[http::header::LOCATION],
                "https://example.com:8443/path?query=1"
            );
        } else {
            panic!("Expected redirect response");
        }
    }

    #[tokio::test]
    async fn disabled_by_config() {
        let mut ctx = make_test_context(Some("example.com"), false, Some(443), None);
        // Simulate https_redirect false in configuration
        let mut directives = StdHashMap::new();
        directives.insert(
            "https_redirect".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
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

        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "should continue when redirect is disabled");
        assert!(ctx.res.is_none());
    }

    #[tokio::test]
    async fn skips_localhost_hostname() {
        for hostname in &["localhost", "127.0.0.1", "::1"] {
            let mut ctx =
                make_test_context(Some(hostname), false, Some(443), Some(hostname.to_string()));
            let stage = HttpsRedirectStage;
            let result = stage.run(&mut ctx).await.unwrap();
            assert!(
                result,
                "should continue pipeline for localhost hostname: {hostname}"
            );
            assert!(ctx.res.is_none());
        }
    }

    #[tokio::test]
    async fn skips_when_listener_port_equals_https_port() {
        // Simulates explicit port config where no separate HTTPS listener exists
        let mut ctx = make_test_context(Some("example.com"), false, Some(8080), None);
        // local_address port is 8080 (same as https_port)
        ctx.local_address = "0.0.0.0:8080".parse().unwrap();
        let stage = HttpsRedirectStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(
            result,
            "should continue when listener port equals https_port"
        );
        assert!(ctx.res.is_none());
    }
}
