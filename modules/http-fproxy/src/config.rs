//! Configuration parsing for the forward proxy module.

use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;

use globset::{Glob, GlobMatcher};
use ipnet::IpNet;

/// Default denied IP ranges: loopback, RFC 1918, link-local, shared,
/// documentation, IPv6 ULA, and cloud metadata.
fn default_denied_ips() -> Vec<IpNet> {
    [
        "127.0.0.0/8",
        "::1/128",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16",
        "100.64.0.0/10",
        "192.0.2.0/24",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "fd00::/8",
        "169.254.169.254/32",
    ]
    .iter()
    .filter_map(|s| s.parse().ok())
    .collect()
}

/// Default allowed ports for forward proxy connections.
fn default_allowed_ports() -> Vec<u16> {
    vec![80, 443]
}

/// A parsed forward proxy configuration block.
#[derive(Clone)]
pub struct ForwardProxyConfig {
    /// Allowed destination domains (supports `*` wildcards).
    /// If empty, all domains are denied (deny-by-default).
    pub allow_domains: Vec<GlobMatcher>,
    /// Allowed destination ports.
    pub allow_ports: Vec<u16>,
    /// Denied destination IP ranges (applied after DNS resolution).
    pub deny_ips: Vec<IpNet>,
    /// Enable HTTP CONNECT tunneling.
    pub connect_method: bool,
    /// HTTP version for upstream connections: 10 = HTTP/1.0, 11 = HTTP/1.1.
    pub http_version: u8,
}

impl Default for ForwardProxyConfig {
    fn default() -> Self {
        Self {
            allow_domains: Vec::new(),
            allow_ports: Vec::new(),
            deny_ips: default_denied_ips(),
            connect_method: true,
            http_version: 11,
        }
    }
}

/// Parse forward proxy configuration from an `HttpContext`.
///
/// Returns `Some(config)` if `forward_proxy` is enabled, `None` otherwise.
pub fn parse_forward_proxy_config(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<ForwardProxyConfig>, Box<dyn Error + Send + Sync>> {
    let entries = ctx.configuration.get_entries("forward_proxy", true);
    if entries.is_empty() {
        return Ok(None);
    }

    let entry = &entries[0];

    // Check if explicitly disabled
    if let Some(first_arg) = entry.args.first() {
        if let Some(false) = first_arg.as_boolean() {
            return Ok(None);
        }
    }

    let mut cfg = ForwardProxyConfig::default();

    if let Some(block) = &entry.children {
        parse_forward_proxy_block(block, &mut cfg)?;
    }

    Ok(Some(cfg))
}

fn parse_forward_proxy_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    cfg: &mut ForwardProxyConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut has_allow_ports = false;
    let mut has_deny_ips = false;

    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "allow_domains" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(pattern) = arg.as_str() {
                            let glob = Glob::new(&convert_wildcard_to_glob(pattern))?;
                            cfg.allow_domains.push(glob.compile_matcher());
                        }
                    }
                }
            }
            "allow_ports" => {
                has_allow_ports = true;
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(port) = arg.as_number() {
                            if port > 0 && port <= 65535 {
                                cfg.allow_ports.push(port as u16);
                            }
                        }
                    }
                }
            }
            "deny_ips" => {
                if !has_deny_ips {
                    cfg.deny_ips.clear();
                    has_deny_ips = true;
                }
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(cidr) = arg.as_str() {
                            if let Ok(net) = IpNet::from_str(cidr) {
                                cfg.deny_ips.push(net);
                            } else if let Ok(ip) = IpAddr::from_str(cidr) {
                                let prefix = if ip.is_ipv4() { 32 } else { 128 };
                                cfg.deny_ips
                                    .push(IpNet::new(ip, prefix).map_err(|e| e.to_string())?);
                            }
                        }
                    }
                }
            }
            "connect_method" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.connect_method = val;
                }
            }
            "http_version" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.http_version = match val {
                        "1.0" => 10,
                        "1.1" => 11,
                        _ => {
                            return Err(format!(
                                "Invalid `http_version`: {val}. Expected 1.0 or 1.1"
                            )
                            .into())
                        }
                    };
                }
            }
            _ => {}
        }
    }

    if !has_allow_ports {
        cfg.allow_ports = default_allowed_ports();
    }

    Ok(())
}

/// Convert a domain pattern with `*` wildcards to a glob pattern.
///
/// `*.example.com` becomes `*.example.com` (glob-compatible).
/// `example.com` becomes `example.com` (exact match).
fn convert_wildcard_to_glob(pattern: &str) -> String {
    // The `*` in domain patterns maps directly to glob `*`
    pattern.to_string()
}

/// Check if a domain matches any of the allowed domain patterns.
pub fn domain_matches(allow_domains: &[GlobMatcher], domain: &str) -> bool {
    if allow_domains.is_empty() {
        return false; // deny-by-default
    }
    allow_domains.iter().any(|m| m.is_match(domain))
}

/// Check if a port is in the allowed ports list.
pub fn port_allowed(allow_ports: &[u16], port: u16) -> bool {
    if allow_ports.is_empty() {
        return false;
    }
    allow_ports.contains(&port)
}

/// Check if an IP is in the denied IP list.
pub fn ip_denied(deny_ips: &[IpNet], ip: IpAddr) -> bool {
    deny_ips.iter().any(|net| net.contains(&ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_matches() {
        let patterns: Vec<GlobMatcher> = ["example.com", "*.example.com"]
            .iter()
            .map(|p| Glob::new(p).unwrap().compile_matcher())
            .collect();

        assert!(domain_matches(&patterns, "example.com"));
        assert!(domain_matches(&patterns, "api.example.com"));
        assert!(!domain_matches(&patterns, "evil.com"));
        assert!(!domain_matches(&patterns, "notexample.com"));
    }

    #[test]
    fn test_domain_matches_empty() {
        // deny-by-default when no patterns configured
        let patterns: Vec<GlobMatcher> = vec![];
        assert!(!domain_matches(&patterns, "example.com"));
    }

    #[test]
    fn test_port_allowed() {
        assert!(port_allowed(&[80, 443], 80));
        assert!(port_allowed(&[80, 443], 443));
        assert!(!port_allowed(&[80, 443], 8080));
        assert!(!port_allowed(&[], 80));
    }

    #[test]
    fn test_ip_denied() {
        let denied = default_denied_ips();
        let localhost: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(ip_denied(&denied, localhost));
        let private: IpAddr = "192.168.0.1".parse().unwrap();
        assert!(ip_denied(&denied, private));
        let rfc1918: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(ip_denied(&denied, rfc1918));
        let metadata: IpAddr = "169.254.169.254".parse().unwrap();
        assert!(ip_denied(&denied, metadata));
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!ip_denied(&denied, public));
    }

    #[test]
    fn test_convert_wildcard_to_glob() {
        assert_eq!(convert_wildcard_to_glob("example.com"), "example.com");
        assert_eq!(convert_wildcard_to_glob("*.example.com"), "*.example.com");
        assert_eq!(convert_wildcard_to_glob("*.corp.*"), "*.corp.*");
    }
}
