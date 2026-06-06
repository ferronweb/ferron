use std::error::Error as StdError;
use std::fmt;

/// Typed errors for the HTTP reverse proxy.
#[derive(Debug)]
pub enum ProxyError {
    InvalidUpstreamUrl(String),
    Io(std::io::Error),
    Hyper(hyper::Error),
    Http(http::Error),
    TlsHandshakeFailed(String),
    ProxyProtocolWriteFailed(String),
    RequestConstructError(String),
    SendRequestError(String),
    HttpUpgradeFailed(String),
    Timeout(String),
    PoolError(String),
    Other(String),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::InvalidUpstreamUrl(u) => write!(f, "Invalid upstream URL: {u}"),
            ProxyError::Io(e) => write!(f, "IO error: {e}"),
            ProxyError::Hyper(e) => write!(f, "Hyper error: {e}"),
            ProxyError::Http(e) => write!(f, "HTTP error: {e}"),
            ProxyError::TlsHandshakeFailed(s) => write!(f, "TLS handshake failed: {s}"),
            ProxyError::ProxyProtocolWriteFailed(s) => write!(f, "PROXY protocol write failed: {s}"),
            ProxyError::RequestConstructError(s) => write!(f, "Request construct failed: {s}"),
            ProxyError::SendRequestError(s) => write!(f, "Send request failed: {s}"),
            ProxyError::HttpUpgradeFailed(s) => write!(f, "HTTP upgrade failed: {s}"),
            ProxyError::Timeout(s) => write!(f, "Timeout: {s}"),
            ProxyError::PoolError(s) => write!(f, "Pool error: {s}"),
            ProxyError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl StdError for ProxyError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ProxyError::Io(e) => Some(e),
            ProxyError::Hyper(e) => Some(e),
            ProxyError::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ProxyError {
    fn from(e: std::io::Error) -> Self {
        ProxyError::Io(e)
    }
}

impl From<hyper::Error> for ProxyError {
    fn from(e: hyper::Error) -> Self {
        ProxyError::Hyper(e)
    }
}

impl From<http::Error> for ProxyError {
    fn from(e: http::Error) -> Self {
        ProxyError::Http(e)
    }
}

impl From<http::uri::InvalidUri> for ProxyError {
    fn from(e: http::uri::InvalidUri) -> Self {
        ProxyError::Other(e.to_string())
    }
}

impl From<http::header::InvalidHeaderValue> for ProxyError {
    fn from(e: http::header::InvalidHeaderValue) -> Self {
        ProxyError::Other(e.to_string())
    }
}

impl From<String> for ProxyError {
    fn from(s: String) -> Self {
        ProxyError::Other(s)
    }
}

impl From<&str> for ProxyError {
    fn from(s: &str) -> Self {
        ProxyError::Other(s.to_string())
    }
}

impl From<Box<dyn StdError + Send + Sync>> for ProxyError {
    fn from(e: Box<dyn StdError + Send + Sync>) -> Self {
        ProxyError::Other(e.to_string())
    }
}

impl ProxyError {
    /// Machine-friendly error type identifier.
    pub fn error_type(&self) -> &'static str {
        match self {
            ProxyError::InvalidUpstreamUrl(_) => "invalid_upstream_url",
            ProxyError::Io(_) => "io_error",
            ProxyError::Hyper(_) => "hyper_error",
            ProxyError::Http(_) => "http_error",
            ProxyError::TlsHandshakeFailed(_) => "tls_handshake_failed",
            ProxyError::ProxyProtocolWriteFailed(_) => "proxy_protocol_write_failed",
            ProxyError::RequestConstructError(_) => "request_construct_error",
            ProxyError::SendRequestError(_) => "send_request_error",
            ProxyError::HttpUpgradeFailed(_) => "http_upgrade_failed",
            ProxyError::Timeout(_) => "timeout",
            ProxyError::PoolError(_) => "pool_error",
            ProxyError::Other(_) => "other",
        }
    }

    /// Short human-readable summary suitable for log.summary.
    pub fn summary(&self) -> String {
        match self {
            ProxyError::InvalidUpstreamUrl(u) => format!("Reverse proxy: invalid upstream URL: {u}"),
            ProxyError::Io(_) => "Reverse proxy: backend IO error".to_string(),
            ProxyError::Hyper(_) => "Reverse proxy: hyper client error".to_string(),
            ProxyError::Http(_) => "Reverse proxy: HTTP error".to_string(),
            ProxyError::TlsHandshakeFailed(_) => "Reverse proxy: TLS handshake failed".to_string(),
            ProxyError::ProxyProtocolWriteFailed(_) =>
                "Reverse proxy: PROXY protocol write failed".to_string(),
            ProxyError::RequestConstructError(_) => "Reverse proxy: request construction failed".to_string(),
            ProxyError::SendRequestError(_) => "Reverse proxy: sending request failed".to_string(),
            ProxyError::HttpUpgradeFailed(_) => "Reverse proxy: HTTP upgrade failed".to_string(),
            ProxyError::Timeout(_) => "Reverse proxy: timeout".to_string(),
            ProxyError::PoolError(_) => "Reverse proxy: pool error".to_string(),
            ProxyError::Other(s) => format!("Reverse proxy: {s}"),
        }
    }
}
