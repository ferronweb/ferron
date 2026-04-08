//! Client IP from forwarded headers stage
//!
//! Reads the `X-Forwarded-For` or `Forwarded` header (as configured via the
//! `client_ip_from_header` directive) and overwrites `ctx.remote_address` with
//! the extracted client IP. This is disabled by default.

use async_trait::async_trait;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::HttpContext;
use std::net::{IpAddr, SocketAddr};

/// Which header to read the client IP from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIpHeader {
    /// Read from `X-Forwarded-For` — takes the first (leftmost) IP in the comma-separated chain.
    XForwardedFor,
    /// Read from `Forwarded` (RFC 7239) — parses the first `for=` token.
    Forwarded,
}

impl ClientIpHeader {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "x-forwarded-for" => Some(Self::XForwardedFor),
            "forwarded" => Some(Self::Forwarded),
            _ => None,
        }
    }

    fn header_name(self) -> &'static str {
        match self {
            Self::XForwardedFor => "x-forwarded-for",
            Self::Forwarded => "forwarded",
        }
    }
}

pub struct ClientIpFromHeaderStage;

impl Default for ClientIpFromHeaderStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

/// Parse an IP address string, optionally with a port. If no port is present,
/// append port 0 to produce a valid `SocketAddr`.
#[cfg(test)]
fn parse_ip_with_optional_port(s: &str, fallback_port: u16) -> Option<SocketAddr> {
    // Try parsing as a full SocketAddr first (e.g. "192.0.2.1:1234" or "[::1]:8080")
    if let Ok(addr) = s.parse::<SocketAddr>() {
        return Some(addr);
    }

    // Try parsing as bare IP (no port)
    let ip: IpAddr = s.parse().ok()?;
    Some(SocketAddr::new(ip, fallback_port))
}

/// Extract the client IP from an `X-Forwarded-For` header value.
///
/// `X-Forwarded-For` format: `client, proxy1, proxy2`
/// The leftmost IP is the original client address.
fn extract_x_forwarded_for(value: &str) -> Option<IpAddr> {
    let first = value.split(',').next()?.trim();
    first.parse::<IpAddr>().ok()
}

/// Extract the client IP from a `Forwarded` header value (RFC 7239).
///
/// Format: `for=192.0.2.60;proto=https, for="[2001:db8:ca1::1]:8080";proto=http`
/// We take the first `for=` token from the first forwarded element.
fn extract_forwarded_for(value: &str) -> Option<IpAddr> {
    // Take the first forwarded element
    let first_element = split_forwarded_elements(value).first().copied()?;

    // Find `for=` in the first element
    let for_value = find_forwarded_param(first_element, "for")?;

    // Strip quotes if present
    let unquoted = for_value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(for_value);

    // In RFC 7239, IPv6 addresses are enclosed in brackets: [2001:db8::1]
    // Strip brackets if present (IPv6 literal)
    let cleaned = unquoted
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(unquoted);

    // The `for` value can be an obfuscated identifier like "_hidden" or an IP.
    // We only succeed if it parses as an IP.
    cleaned.parse::<IpAddr>().ok()
}

/// Split a `Forwarded` header value into individual forwarded elements,
/// respecting quoted strings.
fn split_forwarded_elements(value: &str) -> Vec<&str> {
    let mut elements = Vec::new();
    let mut current_start = 0;
    let mut in_quotes = false;

    for (i, ch) in value.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                elements.push(value[current_start..i].trim());
                current_start = i + 1;
            }
            _ => {}
        }
    }

    let remainder = value[current_start..].trim();
    if !remainder.is_empty() {
        elements.push(remainder);
    }

    elements
}

/// Find a parameter value in a forwarded element (e.g. `for=...`, `proto=...`).
fn find_forwarded_param<'a>(element: &'a str, param_name: &str) -> Option<&'a str> {
    let prefix = format!("{param_name}=");

    // Split by `;` to get individual parameters
    for part in element.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&prefix) {
            return Some(val.trim());
        }
    }

    None
}

/// Resolve which header to use from the configuration. Returns `None` if the
/// directive is absent (meaning this stage is a no-op).
fn resolve_header_from_config(ctx: &HttpContext) -> Option<ClientIpHeader> {
    let value = ctx
        .configuration
        .get_value("client_ip_from_header", false)?;
    let str_val = value.as_str()?;
    ClientIpHeader::from_str(str_val)
}

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
        let header_type = match resolve_header_from_config(ctx) {
            Some(h) => h,
            None => return Ok(true), // Directive not set — no-op
        };

        let req = match ctx.req.as_ref() {
            Some(r) => r,
            None => return Ok(true), // No request yet — let pipeline continue
        };

        let header_value = match req.headers().get(header_type.header_name()) {
            Some(v) => match v.to_str() {
                Ok(s) => s,
                Err(_) => return Ok(true), // Non-UTF8 header — skip
            },
            None => return Ok(true), // Header not present — skip
        };

        let ip = match header_type {
            ClientIpHeader::XForwardedFor => extract_x_forwarded_for(header_value),
            ClientIpHeader::Forwarded => extract_forwarded_for(header_value),
        };

        let Some(ip) = ip else {
            // Header present but couldn't be parsed — skip silently
            return Ok(true);
        };

        // Preserve the original remote port; only replace the IP.
        let original_port = ctx.remote_address.port();
        ctx.remote_address = SocketAddr::new(ip, original_port);

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
        }

        ctx
    }

    // ── X-Forwarded-For tests ──

    #[tokio::test]
    async fn extracts_single_ip_from_x_forwarded_for() {
        let mut ctx = make_test_context(Some("192.0.2.1"), None, Some("x-forwarded-for"));
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
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.1");
    }

    #[tokio::test]
    async fn handles_ipv6_in_x_forwarded_for() {
        let mut ctx = make_test_context(Some("2001:db8::1"), None, Some("x-forwarded-for"));
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "2001:db8::1");
    }

    #[tokio::test]
    async fn skips_when_x_forwarded_for_header_missing() {
        let mut ctx = make_test_context(None, None, Some("x-forwarded-for"));
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        // remote_address should be unchanged
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_directive_not_set() {
        let mut ctx = make_test_context(Some("192.0.2.1"), None, None);
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_x_forwarded_for_value_is_invalid() {
        let mut ctx = make_test_context(Some("not-an-ip"), None, Some("x-forwarded-for"));
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    // ── Forwarded (RFC 7239) tests ──

    #[tokio::test]
    async fn extracts_ip_from_forwarded_for() {
        let mut ctx =
            make_test_context(None, Some("for=192.0.2.60;proto=https"), Some("forwarded"));
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
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.60");
    }

    #[tokio::test]
    async fn extracts_ipv6_from_forwarded_for() {
        let mut ctx = make_test_context(None, Some("for=\"[2001:db8::1]\""), Some("forwarded"));
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
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "192.0.2.60");
    }

    #[tokio::test]
    async fn skips_when_forwarded_header_missing() {
        let mut ctx = make_test_context(None, None, Some("forwarded"));
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
        );
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    #[tokio::test]
    async fn skips_when_forwarded_for_is_obfuscated() {
        let mut ctx = make_test_context(None, Some("for=_hidden"), Some("forwarded"));
        let stage = ClientIpFromHeaderStage;
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        // "_hidden" is not an IP, so the stage should skip
        assert_eq!(ctx.remote_address.ip().to_string(), "10.0.0.1");
    }

    // ── Helper function tests ──

    #[test]
    fn client_ip_header_from_str_valid() {
        assert_eq!(
            ClientIpHeader::from_str("x-forwarded-for"),
            Some(ClientIpHeader::XForwardedFor)
        );
        assert_eq!(
            ClientIpHeader::from_str("X-Forwarded-For"),
            Some(ClientIpHeader::XForwardedFor)
        );
        assert_eq!(
            ClientIpHeader::from_str("FORWARDED"),
            Some(ClientIpHeader::Forwarded)
        );
        assert_eq!(
            ClientIpHeader::from_str("forwarded"),
            Some(ClientIpHeader::Forwarded)
        );
    }

    #[test]
    fn client_ip_header_from_str_invalid() {
        assert_eq!(ClientIpHeader::from_str("x-real-ip"), None);
        assert_eq!(ClientIpHeader::from_str("cf-connecting-ip"), None);
        assert_eq!(ClientIpHeader::from_str(""), None);
    }

    #[test]
    fn parse_ip_with_optional_port_bare_ipv4() {
        let addr = parse_ip_with_optional_port("192.0.2.1", 0).unwrap();
        assert_eq!(addr.ip().to_string(), "192.0.2.1");
        assert_eq!(addr.port(), 0);
    }

    #[test]
    fn parse_ip_with_optional_port_bare_ipv6() {
        let addr = parse_ip_with_optional_port("2001:db8::1", 0).unwrap();
        assert_eq!(addr.ip().to_string(), "2001:db8::1");
        assert_eq!(addr.port(), 0);
    }

    #[test]
    fn parse_ip_with_optional_port_with_port() {
        let addr = parse_ip_with_optional_port("192.0.2.1:8080", 0).unwrap();
        assert_eq!(addr.ip().to_string(), "192.0.2.1");
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn parse_ip_with_optional_port_ipv6_with_port() {
        let addr = parse_ip_with_optional_port("[2001:db8::1]:8080", 0).unwrap();
        assert_eq!(addr.ip().to_string(), "2001:db8::1");
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn parse_ip_with_optional_port_invalid() {
        assert!(parse_ip_with_optional_port("not-an-ip", 0).is_none());
    }

    #[test]
    fn split_forwarded_elements_single() {
        let elements = split_forwarded_elements("for=192.0.2.60;proto=https");
        assert_eq!(elements, vec!["for=192.0.2.60;proto=https"]);
    }

    #[test]
    fn split_forwarded_elements_multiple() {
        let elements =
            split_forwarded_elements("for=192.0.2.60;proto=https, for=10.0.0.1;proto=http");
        assert_eq!(
            elements,
            vec!["for=192.0.2.60;proto=https", "for=10.0.0.1;proto=http",]
        );
    }

    #[test]
    fn split_forwarded_elements_quoted_comma() {
        let elements =
            split_forwarded_elements("for=\"example.com, inc.\";proto=https, for=10.0.0.1");
        assert_eq!(
            elements,
            vec!["for=\"example.com, inc.\";proto=https", "for=10.0.0.1",]
        );
    }
}
