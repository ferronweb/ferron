//! Configuration parsing for `status`, `abort`, `block`, and `allow` directives.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use cidr::IpCidr;
use fancy_regex::Regex;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::ServerConfigurationValue;
use ferron_http::HttpContext;

use crate::ResponseEngine;

/// A rule for returning a custom status code.
pub struct StatusRule {
    /// The HTTP status code to return.
    pub status_code: u16,
    /// Optional exact path match.
    pub url: Option<String>,
    /// Optional regex match against the request path.
    pub regex: Option<Arc<Regex>>,
    /// Optional redirect destination for 3xx responses.
    pub location: Option<String>,
    /// Optional response body.
    pub body: Option<String>,
}

/// Configuration for the `abort` directive.
#[derive(Default)]
pub struct AbortConfig {
    /// Whether to abort the connection without sending a response.
    pub abort: bool,
}

/// Configuration for IP-based access control (`block` and `allow`).
pub struct IpAccessConfig {
    /// IPs/CIDRs that are always denied.
    pub block_list: Vec<IpCidr>,
    /// IPs/CIDRs that are always allowed (bypasses block list).
    pub allow_list: Vec<IpCidr>,
}

impl Default for IpAccessConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl IpAccessConfig {
    pub fn new() -> Self {
        Self {
            block_list: Vec::new(),
            allow_list: Vec::new(),
        }
    }

    /// Check if an IP is blocked. Returns `true` if the IP should be denied.
    ///
    /// Logic:
    /// - If `allow_list` is non-empty and the IP is not in it, block.
    /// - If `block_list` is non-empty and the IP is in it, block.
    /// - Otherwise, allow.
    pub fn is_blocked(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();

        // If an allowlist exists and the IP isn't in it, block
        if !self.allow_list.is_empty() && !self.allow_list.iter().any(|cidr| cidr.contains(&ip)) {
            return true;
        }

        // If the IP is explicitly blocked, deny
        if self.block_list.iter().any(|cidr| cidr.contains(&ip)) {
            return true;
        }

        false
    }
}

/// Parsed configuration for the `early_hints` directive.
pub struct EarlyHintsConfig {
    /// Raw `Link` header values to send in a 103 Early Hints response.
    pub links: Vec<String>,
}

impl Default for EarlyHintsConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl EarlyHintsConfig {
    pub fn new() -> Self {
        Self { links: Vec::new() }
    }
}

/// Parsed configuration for the http-response module.
pub struct ResponseConfig {
    pub abort: AbortConfig,
    pub ip_access: IpAccessConfig,
    pub status_rules: Vec<StatusRule>,
    pub early_hints: EarlyHintsConfig,
}

impl ResponseConfig {
    /// Parse all http-response directives from the layered configuration.
    pub fn from_config(config: &LayeredConfiguration, engine: &ResponseEngine) -> Self {
        let abort = parse_abort_config(config);
        let ip_access = parse_ip_access_config(config);
        let status_rules = parse_status_rules(config, None, engine);
        let early_hints = parse_early_hints_config(config);

        Self {
            abort,
            ip_access,
            status_rules,
            early_hints,
        }
    }

    pub fn from_http_context(ctx: &HttpContext, engine: &ResponseEngine) -> Self {
        let config = &ctx.configuration;
        let abort = parse_abort_config(config);
        let ip_access = parse_ip_access_config(config);
        let status_rules = parse_status_rules(config, Some(ctx), engine);
        let early_hints = parse_early_hints_config(config);

        Self {
            abort,
            ip_access,
            status_rules,
            early_hints,
        }
    }
}

fn parse_abort_config(config: &LayeredConfiguration) -> AbortConfig {
    let abort_directive = config.get_entries("abort", true);
    for entry in &abort_directive {
        // `abort true` — bare boolean value
        if entry.get_flag() {
            return AbortConfig { abort: true };
        }
        // Also check inside children blocks
        if let Some(children) = &entry.children {
            if children.get_flag("abort") {
                return AbortConfig { abort: true };
            }
        }
    }
    AbortConfig::default()
}

fn parse_ip_from_value(value: &ServerConfigurationValue) -> Option<IpCidr> {
    if let Some(s) = value.as_str() {
        if let Ok(cidr) = s.parse::<IpCidr>() {
            return Some(cidr);
        }
    }
    None
}

fn parse_ip_access_config(config: &LayeredConfiguration) -> IpAccessConfig {
    let mut ip_access = IpAccessConfig::new();

    // Parse `block` directives — can have multiple values on a single directive
    // e.g. `block "10.0.0.0/8" "192.168.1.100"`
    let block_entries = config.get_entries("block", false);
    for entry in &block_entries {
        for arg in &entry.args {
            if let Some(cidr) = parse_ip_from_value(arg) {
                ip_access.block_list.push(cidr);
            }
        }
    }

    // Parse `allow` directives
    let allow_entries = config.get_entries("allow", false);
    for entry in &allow_entries {
        for arg in &entry.args {
            if let Some(cidr) = parse_ip_from_value(arg) {
                ip_access.allow_list.push(cidr);
            }
        }
    }

    ip_access
}

fn parse_status_rules(
    config: &LayeredConfiguration,
    ctx: Option<&HttpContext>,
    engine: &ResponseEngine,
) -> Vec<StatusRule> {
    let mut rules = Vec::new();
    let status_entries = config.get_entries("status", true);

    for entry in &status_entries {
        // The status code is the first argument
        let status_code = match entry.args.first() {
            Some(val) => {
                if let Some(n) = val.as_number() {
                    match n.try_into() {
                        Ok(code) => code,
                        Err(_) => continue,
                    }
                } else {
                    continue;
                }
            }
            None => continue,
        };

        let mut url = None;
        let mut regex = None;
        let mut location = None;
        let mut body = None;

        // Check for child block with additional props
        if let Some(children) = &entry.children {
            url = children.get_value("url").and_then(|v| {
                if let Some(ctx) = ctx {
                    v.as_string_with_interpolations(ctx)
                } else {
                    v.as_string_with_interpolations(&HashMap::new())
                }
            });
            location = children.get_value("location").and_then(|v| {
                if let Some(ctx) = ctx {
                    v.as_string_with_interpolations(ctx)
                } else {
                    v.as_string_with_interpolations(&HashMap::new())
                }
            });
            body = children.get_value("body").and_then(|v| {
                if let Some(ctx) = ctx {
                    v.as_string_with_interpolations(ctx)
                } else {
                    v.as_string_with_interpolations(&HashMap::new())
                }
            });

            if let Some(regex_str) = children.get_value("regex").and_then(|v| v.as_str()) {
                // Cached regex
                if let Some(re) = engine.compiled_regexes.get(regex_str) {
                    regex = Some(re.clone());
                } else {
                    match Regex::new(regex_str) {
                        Ok(re) => {
                            let re = Arc::new(re);
                            engine
                                .compiled_regexes
                                .insert(regex_str.to_string(), re.clone());
                            regex = Some(re);
                        }
                        Err(_) => {
                            // Skip rules with invalid regex
                            continue;
                        }
                    }
                }
            }
        }

        rules.push(StatusRule {
            status_code,
            url,
            regex,
            location,
            body,
        });
    }

    rules
}

fn parse_early_hints_config(config: &LayeredConfiguration) -> EarlyHintsConfig {
    let mut links = Vec::new();
    let early_hints_entries = config.get_entries("early_hints", true);

    for entry in &early_hints_entries {
        // Collect `link` directives from child block
        if let Some(children) = &entry.children {
            if let Some(link_entries) = children.directives.get("link") {
                for link_entry in link_entries {
                    for arg in &link_entry.args {
                        if let Some(link_value) = arg.as_str() {
                            links.push(link_value.to_string());
                        }
                    }
                }
            }
        }
    }

    EarlyHintsConfig { links }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_access_blocklist_deny() {
        let mut ip_access = IpAccessConfig::new();
        ip_access.block_list.push("10.0.0.0/8".parse().unwrap());

        assert!(ip_access.is_blocked("10.0.0.1".parse().unwrap()));
        assert!(!ip_access.is_blocked("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn ip_access_allowlist_deny_non_allowed() {
        let mut ip_access = IpAccessConfig::new();
        ip_access.allow_list.push("192.168.1.0/24".parse().unwrap());

        assert!(!ip_access.is_blocked("192.168.1.50".parse().unwrap()));
        assert!(ip_access.is_blocked("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn ip_access_block_takes_precedence_over_allow() {
        let mut ip_access = IpAccessConfig::new();
        ip_access.allow_list.push("192.168.1.0/24".parse().unwrap());
        ip_access
            .block_list
            .push("192.168.1.100/32".parse().unwrap());

        // This IP is in the allow list range but explicitly blocked
        assert!(ip_access.is_blocked("192.168.1.100".parse().unwrap()));
        // Other IPs in the allow list should pass
        assert!(!ip_access.is_blocked("192.168.1.50".parse().unwrap()));
    }
}
