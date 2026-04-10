//! Configuration parsing for `status`, `abort`, `block`, and `allow` directives.

use std::net::IpAddr;

use cidr::IpCidr;
use fancy_regex::Regex;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::ServerConfigurationValue;

/// A rule for returning a custom status code.
pub struct StatusRule {
    /// The HTTP status code to return.
    pub status_code: u16,
    /// Optional exact path match.
    pub url: Option<String>,
    /// Optional regex match against the request path.
    pub regex: Option<Regex>,
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
    pub fn from_config(config: &LayeredConfiguration) -> Self {
        let abort = parse_abort_config(config);
        let ip_access = parse_ip_access_config(config);
        let status_rules = parse_status_rules(config);
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
    let block_entries = config.get_entries("block", true);
    for entry in &block_entries {
        for arg in &entry.args {
            if let Some(cidr) = parse_ip_from_value(arg) {
                ip_access.block_list.push(cidr);
            }
        }
    }

    // Parse `allow` directives
    let allow_entries = config.get_entries("allow", true);
    for entry in &allow_entries {
        for arg in &entry.args {
            if let Some(cidr) = parse_ip_from_value(arg) {
                ip_access.allow_list.push(cidr);
            }
        }
    }

    ip_access
}

fn parse_status_rules(config: &LayeredConfiguration) -> Vec<StatusRule> {
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
            url = children
                .get_value("url")
                .and_then(|v| v.as_str())
                .map(String::from);
            location = children
                .get_value("location")
                .and_then(|v| v.as_str())
                .map(String::from);
            body = children
                .get_value("body")
                .and_then(|v| v.as_str())
                .map(String::from);

            if let Some(regex_str) = children.get_value("regex").and_then(|v| v.as_str()) {
                match Regex::new(regex_str) {
                    Ok(re) => regex = Some(re),
                    Err(_) => {
                        // Skip rules with invalid regex
                        continue;
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
    use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_child_block(
        directives: Vec<(&str, Vec<ServerConfigurationDirectiveEntry>)>,
    ) -> ServerConfigurationBlock {
        let mut d = HashMap::new();
        for (name, entries) in directives {
            d.insert(name.to_string(), entries);
        }
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: HashMap::new(),
            span: None,
        }
    }

    fn make_config_with_status(
        entries: Vec<ServerConfigurationDirectiveEntry>,
    ) -> LayeredConfiguration {
        let mut directives = HashMap::new();
        directives.insert("status".to_string(), entries);

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));
        config
    }

    #[test]
    fn parses_status_code_only() {
        let config = make_config_with_status(vec![ServerConfigurationDirectiveEntry {
            args: vec![make_value_number(403)],
            children: None,
            span: None,
        }]);

        let rules = parse_status_rules(&config);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].status_code, 403);
        assert!(rules[0].url.is_none());
        assert!(rules[0].body.is_none());
    }

    #[test]
    fn parses_status_with_child_block() {
        let mut child_directives = HashMap::new();
        child_directives.insert(
            "url".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("/missing")],
                children: None,
                span: None,
            }],
        );
        child_directives.insert(
            "body".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("Not found")],
                children: None,
                span: None,
            }],
        );

        let config = make_config_with_status(vec![ServerConfigurationDirectiveEntry {
            args: vec![make_value_number(404)],
            children: Some(ServerConfigurationBlock {
                directives: Arc::new(child_directives),
                matchers: HashMap::new(),
                span: None,
            }),
            span: None,
        }]);

        let rules = parse_status_rules(&config);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].status_code, 404);
        assert_eq!(rules[0].url.as_deref(), Some("/missing"));
        assert_eq!(rules[0].body.as_deref(), Some("Not found"));
    }

    #[test]
    fn parses_abort_config() {
        let mut directives = HashMap::new();
        directives.insert(
            "abort".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let abort = parse_abort_config(&config);
        assert!(abort.abort);
    }

    #[test]
    fn parses_block_and_allow_ips() {
        let mut directives = HashMap::new();
        directives.insert(
            "block".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("10.0.0.0/8"),
                    make_value_string("192.168.1.100"),
                ],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "allow".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("192.168.1.0/24")],
                children: None,
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let ip_access = parse_ip_access_config(&config);
        assert_eq!(ip_access.block_list.len(), 2);
        assert_eq!(ip_access.allow_list.len(), 1);
    }

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

    #[test]
    fn parses_early_hints_with_single_link() {
        let link_child = make_child_block(vec![(
            "link",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("</style.css>; rel=preload; as=style")],
                children: None,
                span: None,
            }],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "early_hints".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(link_child),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let early_hints = parse_early_hints_config(&config);
        assert_eq!(early_hints.links.len(), 1);
        assert_eq!(early_hints.links[0], "</style.css>; rel=preload; as=style");
    }

    #[test]
    fn parses_early_hints_with_multiple_links() {
        let link_child = make_child_block(vec![(
            "link",
            vec![
                ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("</style.css>; rel=preload; as=style")],
                    children: None,
                    span: None,
                },
                ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("</script.js>; rel=preload; as=script")],
                    children: None,
                    span: None,
                },
            ],
        )]);

        let mut directives = HashMap::new();
        directives.insert(
            "early_hints".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(link_child),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let early_hints = parse_early_hints_config(&config);
        assert_eq!(early_hints.links.len(), 2);
        assert_eq!(early_hints.links[0], "</style.css>; rel=preload; as=style");
        assert_eq!(early_hints.links[1], "</script.js>; rel=preload; as=script");
    }

    #[test]
    fn parses_empty_early_hints() {
        let empty_child = make_child_block(vec![]);

        let mut directives = HashMap::new();
        directives.insert(
            "early_hints".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(empty_child),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let early_hints = parse_early_hints_config(&config);
        assert!(early_hints.links.is_empty());
    }

    #[test]
    fn parses_no_early_hints() {
        let directives = HashMap::new();

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));

        let early_hints = parse_early_hints_config(&config);
        assert!(early_hints.links.is_empty());
    }
}
