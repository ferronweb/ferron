//! Key extraction strategies for rate limiting.
//!
//! Supported key types:
//! - `remote_address` — the client's IP address
//! - `uri` — the request URI path
//! - `request.header.<name>` — value of a specific request header

use ferron_http::HttpContext;

/// Strategy for extracting a rate limit key from an HTTP request.
#[derive(Debug, Clone)]
pub enum KeyExtractor {
    /// Use the client's remote IP address as the key.
    RemoteAddress,
    /// Use the request URI path as the key.
    Uri,
    /// Use the value of a specific request header as the key.
    Header(String),
}

impl KeyExtractor {
    /// Parse a key extractor from a configuration string.
    ///
    /// Supported formats:
    /// - `"remote_address"` → `RemoteAddress`
    /// - `"uri"` → `Uri`
    /// - `"request.header.X-Api-Key"` → `Header("X-Api-Key")`
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "remote_address" => Some(Self::RemoteAddress),
            "uri" => Some(Self::Uri),
            _ => {
                if let Some(header_name) = s.strip_prefix("request.header.") {
                    if header_name.is_empty() {
                        None
                    } else {
                        Some(Self::Header(header_name.to_string()))
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Extract a rate limit key from the given HTTP context.
    ///
    /// Returns `None` if the key cannot be extracted (e.g., header not present).
    pub fn extract(&self, ctx: &HttpContext) -> Option<String> {
        match self {
            KeyExtractor::RemoteAddress => Some(ctx.remote_address.ip().to_string()),
            KeyExtractor::Uri => ctx.req.as_ref().map(|r| r.uri().path().to_string()),
            KeyExtractor::Header(name) => ctx.req.as_ref().and_then(|r| {
                r.headers()
                    .get(name.as_str())
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Empty};
    use std::collections::HashMap as StdHashMap;
    use typemap_rev::TypeMap;

    fn make_test_context(headers: Vec<(&str, &str)>) -> HttpContext {
        let mut builder = Request::builder().uri("/api/v1/users");
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let req: HttpRequest = builder
            .body(
                Empty::<bytes::Bytes>::new()
                    .map_err(|e| match e {})
                    .boxed_unsync(),
            )
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: None,
            variables: StdHashMap::new(),
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

    #[test]
    fn from_str_remote_address() {
        assert!(matches!(
            KeyExtractor::from_str("remote_address"),
            Some(KeyExtractor::RemoteAddress)
        ));
    }

    #[test]
    fn from_str_uri() {
        assert!(matches!(
            KeyExtractor::from_str("uri"),
            Some(KeyExtractor::Uri)
        ));
    }

    #[test]
    fn from_str_header() {
        assert!(matches!(
            KeyExtractor::from_str("request.header.X-Api-Key"),
            Some(KeyExtractor::Header(name)) if name == "X-Api-Key"
        ));
    }

    #[test]
    fn from_str_invalid() {
        assert!(KeyExtractor::from_str("cookie").is_none());
        assert!(KeyExtractor::from_str("request.header.").is_none());
        assert!(KeyExtractor::from_str("").is_none());
    }

    #[test]
    fn extract_remote_address() {
        let ctx = make_test_context(vec![]);
        let extractor = KeyExtractor::RemoteAddress;
        assert_eq!(extractor.extract(&ctx), Some("192.0.2.1".to_string()));
    }

    #[test]
    fn extract_uri() {
        let ctx = make_test_context(vec![]);
        let extractor = KeyExtractor::Uri;
        assert_eq!(extractor.extract(&ctx), Some("/api/v1/users".to_string()));
    }

    #[test]
    fn extract_header_present() {
        let ctx = make_test_context(vec![("X-Api-Key", "secret123")]);
        let extractor = KeyExtractor::Header("X-Api-Key".to_string());
        assert_eq!(extractor.extract(&ctx), Some("secret123".to_string()));
    }

    #[test]
    fn extract_header_missing() {
        let ctx = make_test_context(vec![]);
        let extractor = KeyExtractor::Header("X-Api-Key".to_string());
        assert!(extractor.extract(&ctx).is_none());
    }
}
