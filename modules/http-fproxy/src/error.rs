//! Typed errors for the HTTP forward proxy.

use std::fmt;

use http::header::InvalidHeaderValue;
use http::uri::InvalidUri;

/// Typed errors for the forward proxy module.
#[derive(Debug)]
pub enum ForwardProxyError {
    /// CONNECT method is disabled in configuration.
    ConnectDisabled,
    /// CONNECT request is malformed (missing authority).
    BadConnectRequest,
    /// Destination port denied by ACL.
    PortDenied { port: u16 },
    /// Destination domain denied by ACL.
    DomainDenied { domain: String },
    /// Unsupported URI scheme (e.g., `https://` in a plain forward request).
    UnsupportedScheme(String),
    /// Missing host in request URI.
    MissingHost,
    /// DNS resolution returned no addresses.
    DnsUnresolved(String),
    /// Secondary runtime not available for DNS resolution.
    DnsUnavailable(String),
    /// Resolved IP is in the denied IP list.
    DnsDeniedIp { host: String, ip: std::net::IpAddr },
    /// Any transport/connection failure in the CONNECT tunnel path.
    ConnectError {
        target: String,
        kind: ConnectErrorKind,
    },
    /// Any transport/connection failure in the HTTP forward path.
    ForwardError {
        address: String,
        kind: ForwardErrorKind,
    },
}

/// Kinds of errors that can occur during CONNECT tunnel setup.
#[derive(Debug)]
pub enum ConnectErrorKind {
    /// HTTP CONNECT upgrade future returned `None` or was absent.
    UpgradeFailed { detail: Option<String> },
    /// TCP connection to the target failed.
    ConnectFailed { error: String },
    /// Bidirectional copy between client and target failed.
    CopyFailed { error: String },
}

/// Kinds of errors that can occur during HTTP forwarding.
#[derive(Debug)]
pub enum ForwardErrorKind {
    /// TCP connection to the upstream failed.
    ConnectFailed { error: String },
    /// Hyper HTTP/1 handshake failed.
    HandshakeFailed { error: String },
    /// Sending the request to the upstream failed.
    SendRequestFailed { error: String },
}

impl fmt::Display for ForwardProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForwardProxyError::ConnectDisabled => {
                write!(f, "CONNECT method is disabled for forward proxy")
            }
            ForwardProxyError::BadConnectRequest => {
                write!(f, "CONNECT request missing authority")
            }
            ForwardProxyError::PortDenied { port } => {
                write!(f, "port {port} denied by ACL")
            }
            ForwardProxyError::DomainDenied { domain } => {
                write!(f, "domain '{domain}' denied by domain ACL")
            }
            ForwardProxyError::UnsupportedScheme(scheme) => {
                write!(f, "unsupported scheme '{scheme}'")
            }
            ForwardProxyError::MissingHost => write!(f, "missing host in request URI"),
            ForwardProxyError::DnsUnresolved(host) => {
                write!(f, "can't resolve {host}")
            }
            ForwardProxyError::DnsUnavailable(host) => {
                write!(
                    f,
                    "secondary runtime not available for DNS resolution of {host}"
                )
            }
            ForwardProxyError::DnsDeniedIp { host, ip } => {
                write!(f, "host '{host}' resolved to denied IP {ip}")
            }
            ForwardProxyError::ConnectError { target, kind } => match kind {
                ConnectErrorKind::UpgradeFailed { detail } => match detail {
                    Some(d) => write!(f, "CONNECT upgrade failed for {target}: {d}"),
                    None => write!(f, "no upgrade future for CONNECT {target}"),
                },
                ConnectErrorKind::ConnectFailed { error } => {
                    write!(f, "cannot connect to {target}: {error}")
                }
                ConnectErrorKind::CopyFailed { error } => {
                    write!(f, "CONNECT tunnel error for {target}: {error}")
                }
            },
            ForwardProxyError::ForwardError { address, kind } => match kind {
                ForwardErrorKind::ConnectFailed { error } => {
                    write!(f, "cannot connect to {address}: {error}")
                }
                ForwardErrorKind::HandshakeFailed { error } => {
                    write!(f, "HTTP/1 handshake failed: {error}")
                }
                ForwardErrorKind::SendRequestFailed { error } => {
                    write!(f, "request to backend failed: {error}")
                }
            },
        }
    }
}

impl std::error::Error for ForwardProxyError {}

impl ForwardProxyError {
    /// Machine-friendly error type identifier.
    #[inline]
    pub fn error_type(&self) -> &'static str {
        match self {
            ForwardProxyError::ConnectDisabled => "connect_disabled",
            ForwardProxyError::BadConnectRequest => "bad_request",
            ForwardProxyError::PortDenied { .. } => "acl_denied",
            ForwardProxyError::DomainDenied { .. } => "acl_denied",
            ForwardProxyError::UnsupportedScheme(_) => "unsupported_scheme",
            ForwardProxyError::MissingHost => "bad_request",
            ForwardProxyError::DnsUnresolved(_) => "dns_unresolved",
            ForwardProxyError::DnsUnavailable(_) => "dns_unavailable",
            ForwardProxyError::DnsDeniedIp { .. } => "dns_denied_ip",
            ForwardProxyError::ConnectError { kind, .. } => match kind {
                ConnectErrorKind::UpgradeFailed { .. } => "upgrade_failed",
                ConnectErrorKind::ConnectFailed { .. } => "backend_connect_error",
                ConnectErrorKind::CopyFailed { .. } => "tunnel_error",
            },
            ForwardProxyError::ForwardError { kind, .. } => match kind {
                ForwardErrorKind::ConnectFailed { .. } => "backend_connect_error",
                ForwardErrorKind::HandshakeFailed { .. } => "handshake_failed",
                ForwardErrorKind::SendRequestFailed { .. } => "send_request_failed",
            },
        }
    }

    /// Short human-readable summary suitable for `LogEvent::summary`.
    #[inline]
    pub fn summary(&self) -> &'static str {
        match self {
            ForwardProxyError::ConnectDisabled => "Forward proxy: CONNECT disabled",
            ForwardProxyError::BadConnectRequest => "Forward proxy: bad CONNECT request",
            ForwardProxyError::PortDenied { .. } => "Forward proxy: port denied by ACL",
            ForwardProxyError::DomainDenied { .. } => "Forward proxy: domain denied by ACL",
            ForwardProxyError::UnsupportedScheme(_) => "Forward proxy: unsupported scheme",
            ForwardProxyError::MissingHost => "Forward proxy: missing host",
            ForwardProxyError::DnsUnresolved(_) => "Forward proxy: DNS resolution failed",
            ForwardProxyError::DnsUnavailable(_) => "Forward proxy: DNS runtime unavailable",
            ForwardProxyError::DnsDeniedIp { .. } => "Forward proxy: resolved IP denied",
            ForwardProxyError::ConnectError { kind, .. } => match kind {
                ConnectErrorKind::UpgradeFailed { .. } => "Forward proxy: CONNECT upgrade failed",
                ConnectErrorKind::ConnectFailed { .. } => {
                    "Forward proxy: connection to target failed"
                }
                ConnectErrorKind::CopyFailed { .. } => "Forward proxy: CONNECT tunnel error",
            },
            ForwardProxyError::ForwardError { kind, .. } => match kind {
                ForwardErrorKind::ConnectFailed { .. } => "Forward proxy: upstream connect failed",
                ForwardErrorKind::HandshakeFailed { .. } => {
                    "Forward proxy: HTTP/1 handshake failed"
                }
                ForwardErrorKind::SendRequestFailed { .. } => {
                    "Forward proxy: request to backend failed"
                }
            },
        }
    }

    /// HTTP status code hint for mapping errors to builtin responses.
    #[inline]
    pub fn http_status_hint(&self) -> Option<u16> {
        match self {
            ForwardProxyError::ConnectDisabled
            | ForwardProxyError::DomainDenied { .. }
            | ForwardProxyError::PortDenied { .. }
            | ForwardProxyError::DnsUnresolved(_)
            | ForwardProxyError::DnsDeniedIp { .. } => Some(403),
            ForwardProxyError::BadConnectRequest
            | ForwardProxyError::UnsupportedScheme(_)
            | ForwardProxyError::MissingHost => Some(400),
            ForwardProxyError::DnsUnavailable(_) => Some(503),
            ForwardProxyError::ConnectError { kind, .. } => match kind {
                ConnectErrorKind::UpgradeFailed { .. } => Some(502),
                ConnectErrorKind::ConnectFailed { .. } => Some(502),
                ConnectErrorKind::CopyFailed { .. } => Some(502),
            },
            ForwardProxyError::ForwardError { kind, .. } => match kind {
                ForwardErrorKind::ConnectFailed { .. } => None, // determined by io error kind
                ForwardErrorKind::HandshakeFailed { .. } => Some(502),
                ForwardErrorKind::SendRequestFailed { .. } => Some(502),
            },
        }
    }
}

impl From<InvalidUri> for ForwardProxyError {
    fn from(e: InvalidUri) -> Self {
        ForwardProxyError::ForwardError {
            address: String::new(),
            kind: ForwardErrorKind::HandshakeFailed {
                error: e.to_string(),
            },
        }
    }
}

impl From<InvalidHeaderValue> for ForwardProxyError {
    fn from(e: InvalidHeaderValue) -> Self {
        ForwardProxyError::ForwardError {
            address: String::new(),
            kind: ForwardErrorKind::HandshakeFailed {
                error: e.to_string(),
            },
        }
    }
}
