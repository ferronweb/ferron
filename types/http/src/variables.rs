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
/// - Custom variables stored in `ctx.variables` (e.g., `request.path_info`)
///
/// Unresolved variables return the variable name itself as a fallback string.
#[inline]
pub fn resolve_variable(name: &str, ctx: &HttpContext) -> Option<String> {
    match name {
        var::REQUEST_METHOD => ctx.req.as_ref().map(|r| r.method().to_string()),
        var::REQUEST_URI_PATH => ctx.req.as_ref().map(|r| r.uri().path().to_string()),
        var::REQUEST_URI_QUERY => ctx
            .req
            .as_ref()
            .map(|r| r.uri().query().unwrap_or("").to_string()),
        var::REQUEST_URI => ctx.req.as_ref().map(|r| r.uri().to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpRequest;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_observability::CompositeEventSink;
    use rustc_hash::FxHashMap;
    use std::net::SocketAddr;
    use typemap_rev::TypeMap;

    fn make_test_context() -> HttpContext {
        HttpContext {
            req: Some(HttpRequest::default()),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: Some("example.com".to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: true,
            local_address: "0.0.0.0:443".parse().unwrap(),
            remote_address: "[::ffff:192.0.2.1]:12345".parse().unwrap(),
            auth_user: None,
            https_port: Some(443),
            extensions: TypeMap::new(),
        }
    }

    #[test]
    fn test_resolve_request_host() {
        let ctx = make_test_context();
        assert_eq!(
            resolve_variable(var::REQUEST_HOST, &ctx),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_resolve_request_scheme_https() {
        let ctx = make_test_context();
        assert_eq!(
            resolve_variable(var::REQUEST_SCHEME, &ctx),
            Some("https".to_string())
        );
    }

    #[test]
    fn test_resolve_request_scheme_http() {
        let mut ctx = make_test_context();
        ctx.encrypted = false;
        assert_eq!(
            resolve_variable(var::REQUEST_SCHEME, &ctx),
            Some("http".to_string())
        );
    }

    #[test]
    fn test_resolve_server_ip() {
        let ctx = make_test_context();
        assert_eq!(
            resolve_variable(var::SERVER_IP, &ctx),
            Some("0.0.0.0".to_string())
        );
    }

    #[test]
    fn test_resolve_server_port() {
        let ctx = make_test_context();
        assert_eq!(
            resolve_variable(var::SERVER_PORT, &ctx),
            Some("443".to_string())
        );
    }

    #[test]
    fn test_resolve_remote_ip_canonicalized() {
        let ctx = make_test_context();
        // ::ffff:192.0.2.1 should be canonicalized to 192.0.2.1
        assert_eq!(
            resolve_variable(var::REMOTE_IP, &ctx),
            Some("192.0.2.1".to_string())
        );
    }

    #[test]
    fn test_resolve_remote_port() {
        let ctx = make_test_context();
        let addr: SocketAddr = ctx.remote_address;
        assert_eq!(
            resolve_variable(var::REMOTE_PORT, &ctx),
            Some(addr.port().to_string())
        );
    }

    #[test]
    fn test_resolve_custom_variable_from_hashmap() {
        let mut ctx = make_test_context();
        ctx.variables
            .insert("custom.var".to_string(), "custom_value".to_string());
        assert_eq!(
            resolve_variable("custom.var", &ctx),
            Some("custom_value".to_string())
        );
    }

    #[test]
    fn test_resolve_unresolved_variable_returns_name() {
        let ctx = make_test_context();
        assert_eq!(
            resolve_variable("nonexistent.var", &ctx),
            Some("nonexistent.var".to_string())
        );
    }

    #[test]
    fn test_canonicalize_ip_ipv4() {
        let ip: std::net::IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(canonicalize_ip(ip), "192.0.2.1");
    }

    #[test]
    fn test_canonicalize_ip_ipv4_mapped_ipv6() {
        let ip: std::net::IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(canonicalize_ip(ip), "192.0.2.1");
    }

    #[test]
    fn test_canonicalize_ip_pure_ipv6() {
        let ip: std::net::IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(canonicalize_ip(ip), "2001:db8::1");
    }
}
