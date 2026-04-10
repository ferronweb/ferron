//! Configuration parsing and validation for the forward proxy module.

use std::collections::HashSet;
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationDirectiveEntry;
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
            allow_ports: default_allowed_ports(),
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

/// Configuration validator for the forward proxy module.
pub struct ForwardProxyConfigurationValidator;

impl ConfigurationValidator for ForwardProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        if is_global {
            return Ok(());
        }

        if let Some(entries) = config.directives.get("forward_proxy") {
            used_directives.insert("forward_proxy".to_string());
            validate_forward_proxy_entries(entries, used_directives)?;
        }
        Ok(())
    }
}

fn validate_forward_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        // Validate args: at most one boolean (the enable toggle)
        if entry.args.len() > 1 {
            return Err(
                "The `forward_proxy` directive may have at most one boolean argument".into(),
            );
        }
        if let Some(arg) = entry.args.first() {
            if arg.as_boolean().is_none() {
                return Err("Invalid `forward_proxy` — expected a boolean".into());
            }
        }

        // Validate block children
        if let Some(block) = &entry.children {
            validate_forward_proxy_block(block, used_directives)?;
        }
    }
    Ok(())
}

fn validate_forward_proxy_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    // allow_domains — accepts string arguments
    if let Some(entries) = block.directives.get("allow_domains") {
        used_directives.insert("allow_domains".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `allow_domains` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `allow_domains` — expected a string".into());
                }
            }
        }
    }

    // allow_ports — accepts numeric arguments
    if let Some(entries) = block.directives.get("allow_ports") {
        used_directives.insert("allow_ports".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `allow_ports` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if let Some(val) = arg.as_number() {
                    if val <= 0 || val > 65535 {
                        return Err("Invalid `allow_ports` — must be between 1 and 65535".into());
                    }
                } else {
                    return Err("Invalid `allow_ports` — expected a number".into());
                }
            }
        }
    }

    // deny_ips — accepts CIDR string arguments
    if let Some(entries) = block.directives.get("deny_ips") {
        used_directives.insert("deny_ips".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `deny_ips` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `deny_ips` — expected a CIDR string".into());
                }
            }
        }
    }

    // connect_method — boolean
    validate_bool(block, used_directives, "connect_method")?;

    // http_version — enum
    if let Some(entries) = block.directives.get("http_version") {
        used_directives.insert("http_version".to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                if val != "1.0" && val != "1.1" {
                    return Err("Invalid `http_version` — expected 1.0 or 1.1".into());
                }
            } else {
                return Err("Invalid `http_version` — expected a string".into());
            }
        }
    }

    Ok(())
}

fn validate_bool(
    block: &ferron_core::config::ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err(format!("Invalid `{name}` — expected a boolean").into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_denied_ips_parse() {
        let ips = default_denied_ips();
        assert!(!ips.is_empty());
        // Verify localhost is in the list
        let localhost: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(ips.iter().any(|n| n.contains(&localhost)));
        let private: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(ips.iter().any(|n| n.contains(&private)));
        let metadata: IpAddr = "169.254.169.254".parse().unwrap();
        assert!(ips.iter().any(|n| n.contains(&metadata)));
    }

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
