use crate::HttpContext;

/// Variable name constants to avoid magic strings throughout the codebase.
pub mod var {
    pub const REQUEST_METHOD: &str = "request.method";
    pub const REQUEST_URI_PATH: &str = "request.uri.path";
    pub const REQUEST_URI_QUERY: &str = "request.uri.query";
    pub const REQUEST_URI: &str = "request.uri";
    pub const REQUEST_VERSION: &str = "request.version";
    pub const REQUEST_HOST: &str = "request.host";
    pub const REQUEST_SCHEME: &str = "request.scheme";
    pub const REQUEST_PATH_INFO: &str = "request.path_info";
    pub const REQUEST_HEADER_PREFIX: &str = "request.header.";
    pub const SERVER_IP: &str = "server.ip";
    pub const SERVER_PORT: &str = "server.port";
    pub const REMOTE_IP: &str = "remote.ip";
    pub const REMOTE_PORT: &str = "remote.port";
    pub const AUTH_USER: &str = "auth.user";
    pub const TRACE_ID: &str = "trace.id";
    pub const TRACE_SPANID: &str = "trace.spanid";
}

/// Canonicalize an IP address: convert IPv4-mapped IPv6 (`::ffff:x.x.x.x`) to IPv4.
#[inline]
pub fn canonicalize_ip(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(_) => ip.to_string(),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                v4.to_string()
            } else {
                ip.to_string()
            }
        }
    }
}

/// Resolve a variable by name from the HTTP context.
///
/// Supports:
/// - `request.method`, `request.uri.path`, `request.uri.query`, `request.uri`, `request.version`
/// - `request.header.<name>` — header values (names lowercased, `_` → `-`)
/// - `request.host`, `request.scheme`, `request.path_info`
/// - `server.ip`, `server.port`, `remote.ip`, `remote.port`
/// - `auth.user`, `trace.id`, `trace.spanid`
/// - Custom variables stored in `ctx.variables` (e.g., `request.path_info`)
///
/// Unresolved variables return the variable name itself as a fallback string.
#[inline]
pub fn resolve_variable(name: &str, ctx: &HttpContext) -> Option<String> {
    match name {
        var::REQUEST_METHOD => ctx.req.as_ref().map(|r| r.method().to_string()),
        var::REQUEST_URI_PATH => ctx
            .original_uri
            .as_ref()
            .map(|u| u.path().to_string())
            .or_else(|| ctx.req.as_ref().map(|r| r.uri().path().to_string())),
        var::REQUEST_URI_QUERY => ctx
            .original_uri
            .as_ref()
            .map(|u| u.query().unwrap_or("").to_string())
            .or_else(|| {
                ctx.req
                    .as_ref()
                    .map(|r| r.uri().query().unwrap_or("").to_string())
            }),
        var::REQUEST_URI => ctx
            .original_uri
            .as_ref()
            .map(|u| u.to_string())
            .or_else(|| ctx.req.as_ref().map(|r| r.uri().to_string())),
        var::REQUEST_VERSION => ctx.req.as_ref().map(|r| match r.version() {
            http::Version::HTTP_09 => "HTTP/0.9".to_string(),
            http::Version::HTTP_10 => "HTTP/1.0".to_string(),
            http::Version::HTTP_11 => "HTTP/1.1".to_string(),
            http::Version::HTTP_2 => "HTTP/2.0".to_string(),
            http::Version::HTTP_3 => "HTTP/3.0".to_string(),
            _ => "unknown".to_string(),
        }),
        var::REQUEST_HOST => Some(ctx.hostname.clone().unwrap_or_default()),
        var::REQUEST_SCHEME => Some(if ctx.encrypted { "https" } else { "http" }.to_string()),
        var::SERVER_IP => Some(canonicalize_ip(ctx.local_address.ip())),
        var::SERVER_PORT => Some(ctx.local_address.port().to_string()),
        var::REMOTE_IP => Some(canonicalize_ip(ctx.remote_address.ip())),
        var::REMOTE_PORT => Some(ctx.remote_address.port().to_string()),
        var::AUTH_USER => Some(ctx.auth_user.clone().unwrap_or_default()),
        var::TRACE_ID => Some(
            crate::trace_context::current_event_trace_context(ctx)
                .map_or(Default::default(), |ctx| hex::encode(ctx.trace_id)),
        ),
        var::TRACE_SPANID => Some(
            crate::trace_context::current_event_trace_context(ctx)
                .map_or(Default::default(), |ctx| hex::encode(ctx.span_id)),
        ),
        n if n.starts_with(var::REQUEST_HEADER_PREFIX) => {
            let header_name = n
                .trim_start_matches(var::REQUEST_HEADER_PREFIX)
                .to_ascii_lowercase()
                .replace("_", "-");
            ctx.req
                .as_ref()
                .and_then(|r| r.headers().get(&header_name))
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        }
        // Fallback to custom variables in the HashMap (e.g., request.path_info)
        n => ctx
            .variables
            .get(n)
            .cloned()
            .or_else(|| Some(name.to_string())),
    }
}
