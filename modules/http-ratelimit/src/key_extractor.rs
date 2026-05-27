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
            KeyExtractor::Uri => match &ctx.routing_uri {
                Some(uri) => Some(uri.path().to_string()),
                None => ctx.req.as_ref().map(|r| r.uri().path().to_string()),
            },
            KeyExtractor::Header(name) => ctx.req.as_ref().and_then(|r| {
                r.headers()
                    .get(name.as_str())
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            }),
        }
    }
}
