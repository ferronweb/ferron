use std::error::Error as StdError;
use std::fmt;

use http::StatusCode;

/// Typed errors for the HTTP reverse proxy.
#[derive(Debug)]
pub enum ProxyError {
    InvalidUpstreamUrl(String),
    ConnectFailed(String),
    ConnectFailedUnavailable(String),
    Io(std::io::Error),
    Hyper(hyper::Error),
    Http(http::Error),
    TlsHandshakeFailed(String),
    ProxyProtocolWriteFailed(String),
    RequestConstructError(String),
    SendRequestError(String),
    Timeout(String),
    Other(String),
}

impl fmt::Display for ProxyError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::InvalidUpstreamUrl(u) => write!(f, "Invalid upstream URL: {u}"),
            ProxyError::ConnectFailed(s) => write!(f, "Connect failed: {s}"),
            ProxyError::ConnectFailedUnavailable(s) => write!(f, "Connect failed: {s}"),
            ProxyError::Io(e) => write!(f, "IO error: {e}"),
            ProxyError::Hyper(e) => write!(f, "Hyper error: {e}"),
            ProxyError::Http(e) => write!(f, "HTTP error: {e}"),
            ProxyError::TlsHandshakeFailed(s) => write!(f, "TLS handshake failed: {s}"),
            ProxyError::ProxyProtocolWriteFailed(s) => {
                write!(f, "PROXY protocol write failed: {s}")
            }
            ProxyError::RequestConstructError(s) => write!(f, "Request construct failed: {s}"),
            ProxyError::SendRequestError(s) => write!(f, "Send request failed: {s}"),
            ProxyError::Timeout(s) => write!(f, "Timeout: {s}"),
            ProxyError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl StdError for ProxyError {
    #[inline]
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
    #[inline]
    fn from(e: std::io::Error) -> Self {
        ProxyError::Io(e)
    }
}

impl From<hyper::Error> for ProxyError {
    #[inline]
    fn from(e: hyper::Error) -> Self {
        ProxyError::Hyper(e)
    }
}

impl From<http::Error> for ProxyError {
    #[inline]
    fn from(e: http::Error) -> Self {
        ProxyError::Http(e)
    }
}

impl From<http::uri::InvalidUri> for ProxyError {
    #[inline]
    fn from(e: http::uri::InvalidUri) -> Self {
        ProxyError::RequestConstructError(e.to_string())
    }
}

impl From<http::header::InvalidHeaderValue> for ProxyError {
    #[inline]
    fn from(e: http::header::InvalidHeaderValue) -> Self {
        ProxyError::RequestConstructError(e.to_string())
    }
}

impl From<String> for ProxyError {
    #[inline]
    fn from(s: String) -> Self {
        ProxyError::Other(s)
    }
}

impl From<&str> for ProxyError {
    #[inline]
    fn from(s: &str) -> Self {
        ProxyError::Other(s.to_string())
    }
}

impl From<Box<dyn StdError + Send + Sync>> for ProxyError {
    #[inline]
    fn from(e: Box<dyn StdError + Send + Sync>) -> Self {
        ProxyError::Other(e.to_string())
    }
}

impl ProxyError {
    /// Machine-friendly error type identifier.
    #[inline]
    pub fn error_type(&self) -> &'static str {
        match self {
            ProxyError::InvalidUpstreamUrl(_) => "invalid_upstream_url",
            ProxyError::ConnectFailed(_) => "connect_failed",
            ProxyError::ConnectFailedUnavailable(_) => "connect_failed",
            ProxyError::Io(_) => "io_error",
            ProxyError::Hyper(_) => "hyper_error",
            ProxyError::Http(_) => "http_error",
            ProxyError::TlsHandshakeFailed(_) => "tls_handshake_failed",
            ProxyError::ProxyProtocolWriteFailed(_) => "proxy_protocol_write_failed",
            ProxyError::RequestConstructError(_) => "request_construct_error",
            ProxyError::SendRequestError(_) => "send_request_error",
            ProxyError::Timeout(_) => "timeout",
            ProxyError::Other(_) => "other",
        }
    }

    /// Short human-readable summary suitable for log.summary.
    #[inline]
    pub fn summary(&self) -> &'static str {
        match self {
            ProxyError::InvalidUpstreamUrl(_) => "Reverse proxy: invalid upstream URL",
            ProxyError::ConnectFailed(_) => "Reverse proxy: connect failed",
            ProxyError::ConnectFailedUnavailable(_) => {
                "Reverse proxy: connect failed (unavailable)"
            }
            ProxyError::Io(_) => "Reverse proxy: backend I/O error",
            ProxyError::Hyper(_) => "Reverse proxy: hyper client error",
            ProxyError::Http(_) => "Reverse proxy: HTTP error",
            ProxyError::TlsHandshakeFailed(_) => "Reverse proxy: TLS handshake failed",
            ProxyError::ProxyProtocolWriteFailed(_) => "Reverse proxy: PROXY protocol write failed",
            ProxyError::RequestConstructError(_) => "Reverse proxy: request construction failed",
            ProxyError::SendRequestError(_) => "Reverse proxy: sending request failed",
            ProxyError::Timeout(_) => "Reverse proxy: timeout",
            ProxyError::Other(_) => "Reverse proxy: other error",
        }
    }

    /// Optional HTTP status hint for mapping errors to builtin responses.
    #[inline]
    pub fn http_status_hint(&self) -> Option<StatusCode> {
        match self {
            ProxyError::ConnectFailedUnavailable(_) => Some(StatusCode::SERVICE_UNAVAILABLE),
            ProxyError::Timeout(_) => Some(StatusCode::GATEWAY_TIMEOUT),
            // For Io errors, prefer the existing io_error_status() helper in proxy::tls
            ProxyError::Io(_) | ProxyError::Other(_) => None,
            // 502 Bad Gateway by default
            _ => Some(StatusCode::BAD_GATEWAY),
        }
    }
}
