//! Configuration parsing and validation for the reverse proxy module.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
#[cfg(feature = "srv-lookup")]
use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    Variables,
};
use ferron_core::util::parse_duration;
use http::header::HeaderName;

#[cfg(feature = "srv-lookup")]
use crate::upstream::SrvUpstreamData;
use crate::upstream::{
    ExpectedStatusCodes, HealthCheckMethod, LoadBalancerAlgorithm, ProxyHeader, Upstream,
    UpstreamConfig, UpstreamHealthCheckConfig,
};

/// Default keep-alive idle timeout in milliseconds.
const DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS: u64 = 60_000;

/// A header action: currently only append is supported for `request_header +Name`.
/// The value is stored as a raw `String` with potential interpolation
/// syntax (`{{...}}`); it is resolved at request time.
#[derive(Clone)]
pub enum HeaderAction {
    /// Append the given value to the header.
    Append(HeaderName, String),
}

/// Parsed reverse proxy configuration.
#[derive(Clone)]
pub struct ProxyConfig {
    pub upstreams: Vec<Upstream>,
    pub lb_algorithm: LoadBalancerAlgorithm,
    pub lb_health_check: bool,
    pub lb_health_check_max_fails: u64,
    pub lb_health_check_window: Duration,
    pub lb_retry_connection: bool,
    pub keepalive: bool,
    pub http2: bool,
    pub http2_only: bool,
    pub intercept_errors: bool,
    pub no_verification: bool,
    pub proxy_header: Option<ProxyHeader>,
    /// Headers to add or append (values may contain `{{...}}` interpolation syntax).
    pub headers_to_add: Vec<HeaderAction>,
    /// Headers to replace (values may contain `{{...}}` interpolation syntax).
    pub headers_to_replace: Vec<(HeaderName, String)>,
    /// Headers to remove.
    pub headers_to_remove: Vec<HeaderName>,
    pub concurrent_conns: Option<usize>,
    /// Pre-built map from upstream URL to idle timeout for O(1) lookup.
    pub idle_timeout_map: HashMap<String, Duration>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstreams: Vec::new(),
            lb_algorithm: LoadBalancerAlgorithm::TwoRandomChoices,
            lb_health_check: false,
            lb_health_check_max_fails: 3,
            lb_health_check_window: Duration::from_millis(5000),
            lb_retry_connection: true,
            keepalive: true,
            http2: false,
            http2_only: false,
            intercept_errors: false,
            no_verification: false,
            proxy_header: None,
            headers_to_add: Vec::new(),
            headers_to_replace: Vec::new(),
            headers_to_remove: Vec::new(),
            concurrent_conns: None,
            idle_timeout_map: HashMap::new(),
        }
    }
}

/// Resolve a config value as a string, interpolating `{{env.*}}` variables only.
///
/// Upstream URLs are resolved at config parse time, so only `env.*` variables
/// are available (no HTTP request data at this stage).
fn resolve_config_value_with_env(value: &ServerConfigurationValue) -> Option<String> {
    struct EnvResolver;
    impl Variables for EnvResolver {
        fn resolve(&self, _name: &str) -> Option<String> {
            None // No request-scoped variables available at parse time
        }
    }
    value.as_string_with_interpolations(&EnvResolver)
}

/// Parse expected status codes from a string.
/// Accepts:
/// - "2xx" for 200-299
/// - "3xx" for 300-399
/// - "4xx" for 400-499
/// - "5xx" for 500-599
/// - "2xx,3xx" for multiple ranges
/// - "200,201,204" for specific codes
/// - "200-299" for a range
fn parse_expected_status(s: &str) -> Result<ExpectedStatusCodes, Box<dyn Error + Send + Sync>> {
    let s = s.trim();

    // Check for common shorthands
    if s == "2xx" {
        return Ok(ExpectedStatusCodes::Successful);
    }
    if s == "2xx,3xx" || s == "3xx,2xx" {
        return Ok(ExpectedStatusCodes::SuccessfulOrRedirect);
    }

    // Try to parse as a range (e.g., "200-299")
    if let Some(idx) = s.find('-') {
        let start_str = &s[..idx].trim();
        let end_str = &s[idx + 1..].trim();
        if let (Ok(start), Ok(end)) = (start_str.parse::<u16>(), end_str.parse::<u16>()) {
            if start <= end && start >= 100 && end < 600 {
                return Ok(ExpectedStatusCodes::Range(start, end));
            }
        }
    }

    // Try to parse as comma-separated values
    if s.contains(',') {
        let mut codes = Vec::new();
        for part in s.split(',') {
            let code: u16 = part.trim().parse()?;
            codes.push(code);
        }
        if codes.len() == 1 {
            return Ok(ExpectedStatusCodes::Specific(codes[0]));
        }
        return Ok(ExpectedStatusCodes::Any(codes));
    }

    // Try to parse as a single status code
    let code: u16 = s.parse()?;
    Ok(ExpectedStatusCodes::Specific(code))
}

/// Parse proxy configuration from a server configuration block.
pub fn parse_proxy_config(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<ProxyConfig>, Box<dyn Error + Send + Sync>> {
    let entries = ctx.configuration.get_entries("proxy", true);
    if entries.is_empty() {
        return Ok(None);
    }

    let entry = entries[0];
    let mut cfg = ProxyConfig::default();

    // Check for shorthand upstreams in args (e.g. `proxy http://a http://b { ... }`)
    let default_timeout = Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS);
    for arg in &entry.args {
        if let Some(url) = resolve_config_value_with_env(arg) {
            cfg.upstreams.push(Upstream::Static(UpstreamConfig {
                url: url.clone(),
                unix_socket: None,
                limit: None,
                idle_timeout: Some(default_timeout),
                health_check_config: UpstreamHealthCheckConfig::default(),
            }));
            cfg.idle_timeout_map.insert(url, default_timeout);
        }
    }

    // Parse block if present
    if let Some(children) = &entry.children {
        parse_proxy_block(children, &mut cfg)?;
    }

    if cfg.upstreams.is_empty() {
        return Ok(None);
    }

    // Check for global concurrent_conns
    if let Some(conns_entries) = ctx
        .configuration
        .get_entries("proxy_concurrent_conns", true)
        .first()
    {
        if let Some(val) = conns_entries
            .args
            .first()
            .and_then(|v: &ServerConfigurationValue| v.as_number())
        {
            cfg.concurrent_conns = Some(val as usize);
        }
    }

    Ok(Some(cfg))
}

fn parse_proxy_block(
    block: &ServerConfigurationBlock,
    cfg: &mut ProxyConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "upstream" => {
                for entry in entries {
                    parse_upstream_entry(entry, cfg)?;
                }
            }
            #[cfg(feature = "srv-lookup")]
            "srv" => {
                for entry in entries {
                    parse_srv_entry(entry, cfg)?;
                }
            }
            "lb_algorithm" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.lb_algorithm = match val {
                        "random" => LoadBalancerAlgorithm::Random,
                        "round_robin" => LoadBalancerAlgorithm::RoundRobin,
                        "least_conn" => LoadBalancerAlgorithm::LeastConnections,
                        "two_random" => LoadBalancerAlgorithm::TwoRandomChoices,
                        _ => {
                            return Err(
                                format!("Unsupported load balancing algorithm: {val}").into()
                            )
                        }
                    };
                }
            }
            "lb_health_check" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.lb_health_check = val;
                }
            }
            "lb_health_check_max_fails" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    cfg.lb_health_check_max_fails = val as u64;
                }
            }
            "lb_health_check_window" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.lb_health_check_window = parse_duration(val)
                        .map_err(|e| format!("Invalid lb_health_check_window: {e}"))?;
                }
            }
            "lb_retry_connection" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.lb_retry_connection = val;
                }
            }
            "keepalive" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.keepalive = val;
                }
            }
            "http2" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.http2 = val;
                }
            }
            "http2_only" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.http2_only = val;
                }
            }
            "intercept_errors" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.intercept_errors = val;
                }
            }
            "no_verification" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cfg.no_verification = val;
                }
            }
            "proxy_header" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.proxy_header = match val {
                        "v1" => Some(ProxyHeader::V1),
                        "v2" => Some(ProxyHeader::V2),
                        _ => return Err(format!("Invalid PROXY header version: {val}").into()),
                    };
                }
            }
            "request_header" => {
                for entry in entries {
                    parse_request_header_entry(entry, cfg)?;
                }
            }
            "proxy_concurrent_conns" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    cfg.concurrent_conns = Some(val as usize);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_upstream_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let url = entry
        .args
        .first()
        .and_then(resolve_config_value_with_env)
        .ok_or("upstream requires a URL argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut unix_socket: Option<String> = None;
    let mut health_check_config = UpstreamHealthCheckConfig::default();

    if let Some(block) = &entry.children {
        for (name, entries) in block.directives.iter() {
            match name.as_str() {
                "limit" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            limit = Some(val as usize);
                        }
                    }
                }
                "idle_timeout" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        idle_timeout = Some(
                            parse_duration(val)
                                .map_err(|e| format!("Invalid idle_timeout: {e}"))?,
                        );
                    }
                }
                "unix" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(resolve_config_value_with_env)
                    {
                        unix_socket = Some(val);
                    }
                }
                "health_check" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_boolean())
                    {
                        health_check_config.enabled = val;
                    }
                }
                "health_check_uri" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.uri = val.to_string();
                    }
                }
                "health_check_method" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.method = match val.to_uppercase().as_str() {
                            "GET" => HealthCheckMethod::Get,
                            "HEAD" => HealthCheckMethod::Head,
                            _ => {
                                return Err(format!(
                                    "Invalid health_check_method: {val}, must be GET or HEAD"
                                )
                                .into())
                            }
                        };
                    }
                }
                "health_check_interval" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.interval = parse_duration(val)
                            .map_err(|e| format!("Invalid health_check_interval: {e}"))?;
                    }
                }
                "health_check_timeout" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.timeout = parse_duration(val)
                            .map_err(|e| format!("Invalid health_check_timeout: {e}"))?;
                    }
                }
                "health_check_expect_status" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.expect_status = parse_expected_status(val)?;
                    }
                }
                "health_check_response_time_threshold" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.response_time_threshold =
                            Some(parse_duration(val).map_err(|e| {
                                format!("Invalid health_check_response_time_threshold: {e}")
                            })?);
                    }
                }
                "health_check_body_match" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        health_check_config.body_match = Some(val.to_string());
                    }
                }
                "health_check_consecutive_fails" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            health_check_config.consecutive_fails = val as u64;
                        }
                    }
                }
                "health_check_consecutive_passes" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            health_check_config.consecutive_passes = val as u64;
                        }
                    }
                }
                "health_check_no_verification" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_boolean())
                    {
                        health_check_config.no_verification = val;
                    }
                }
                _ => {}
            }
        }
    }

    if idle_timeout.is_none() {
        idle_timeout = Some(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS));
    }

    cfg.upstreams.push(Upstream::Static(UpstreamConfig {
        url: url.clone(),
        unix_socket,
        limit,
        idle_timeout,
        health_check_config,
    }));

    // Populate the O(1) lookup map
    cfg.idle_timeout_map.insert(
        url.clone(),
        idle_timeout.unwrap_or(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS)),
    );

    Ok(())
}

/// Parse an SRV upstream entry.
#[cfg(feature = "srv-lookup")]
fn parse_srv_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let srv_name = entry
        .args
        .first()
        .and_then(|v| v.as_str())
        .ok_or("srv requires an SRV record name argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut dns_servers: Vec<IpAddr> = Vec::new();

    if let Some(block) = &entry.children {
        for (name, entries) in block.directives.iter() {
            match name.as_str() {
                "limit" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            limit = Some(val as usize);
                        }
                    }
                }
                "idle_timeout" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        idle_timeout = Some(
                            parse_duration(val)
                                .map_err(|e| format!("Invalid idle_timeout: {e}"))?,
                        );
                    }
                }
                "dns_servers" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        dns_servers = val
                            .split(',')
                            .filter_map(|s| s.trim().parse::<IpAddr>().ok())
                            .collect();
                    }
                }
                _ => {}
            }
        }
    }

    if idle_timeout.is_none() {
        idle_timeout = Some(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS));
    }

    cfg.upstreams.push(Upstream::Srv(SrvUpstreamData {
        srv_name: srv_name.to_string(),
        dns_servers,
        limit,
        idle_timeout,
    }));

    Ok(())
}

fn parse_request_header_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if entry.args.is_empty() {
        return Err("request_header requires at least one argument".into());
    }

    let first_arg = entry.args[0]
        .as_str()
        .ok_or("request_header name must be a string")?;

    match first_arg.chars().next() {
        Some('+') => {
            // Append header — value may contain interpolation syntax
            let name = &first_arg[1..];
            let value = entry
                .args
                .get(1)
                .and_then(resolve_config_value_with_env)
                .ok_or("request_header +Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_add
                .push(HeaderAction::Append(header_name, value));
        }
        Some('-') => {
            // Remove header
            let name = &first_arg[1..];
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_remove.push(header_name);
        }
        _ => {
            // Replace header — value may contain interpolation syntax
            let name = first_arg;
            let value = entry
                .args
                .get(1)
                .and_then(resolve_config_value_with_env)
                .ok_or("request_header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_replace.push((header_name, value));
        }
    }

    Ok(())
}

/// Configuration validator for the reverse proxy module.
pub struct ProxyConfigurationValidator;

impl ConfigurationValidator for ProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        if is_global {
            // Validate global concurrent_conns directive
            if let Some(entries) = config.directives.get("concurrent_conns") {
                used_directives.insert("concurrent_conns".to_string());
                for e in entries {
                    if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                        if val < 0 {
                            return Err("Invalid `concurrent_conns` — must be non-negative".into());
                        }
                    } else {
                        return Err("Invalid `concurrent_conns` — expected a number".into());
                    }
                }
            }
        }
        if let Some(entries) = config.directives.get("proxy") {
            used_directives.insert("proxy".to_string());
            validate_proxy_entries(entries, used_directives)?;
        }
        Ok(())
    }
}

fn validate_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        if entry.args.len() > 1 {
            return Err(
                "The `proxy` directive may have at most one shorthand upstream argument".into(),
            );
        }
        for arg in &entry.args {
            if arg.as_str().is_none() {
                return Err("Invalid proxy upstream URL — expected a string".into());
            }
        }
        if let Some(block) = &entry.children {
            validate_proxy_block(block, used_directives)?;
        }
    }
    Ok(())
}

fn validate_proxy_block(
    block: &ServerConfigurationBlock,
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_str(block, used_directives, "lb_algorithm")?;
    validate_bool(block, used_directives, "lb_health_check")?;
    validate_number(block, used_directives, "lb_health_check_max_fails", 0)?;
    validate_duration(block, used_directives, "lb_health_check_window")?;
    validate_bool(block, used_directives, "lb_retry_connection")?;
    validate_bool(block, used_directives, "keepalive")?;
    validate_bool(block, used_directives, "http2")?;
    validate_bool(block, used_directives, "http2_only")?;
    validate_bool(block, used_directives, "intercept_errors")?;
    validate_bool(block, used_directives, "no_verification")?;
    validate_enum(block, used_directives, "proxy_header", &["v1", "v2"])?;
    validate_request_header(block, used_directives)?;
    validate_number(block, used_directives, "proxy_concurrent_conns", 0)?;
    validate_upstream_directives(block, used_directives)?;
    #[cfg(feature = "srv-lookup")]
    validate_srv_directives(block, used_directives)?;
    Ok(())
}

fn validate_str(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err(format!("Invalid `{name}` — expected a string").into());
            }
        }
    }
    Ok(())
}

fn validate_bool(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.is_empty() {
                continue;
            }
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err(format!("Invalid `{name}` — expected a boolean").into());
            }
        }
    }
    Ok(())
}

fn validate_number(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
    min: i64,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                if val < min {
                    return Err(format!("Invalid `{name}` — must be >= {min}").into());
                }
            } else {
                return Err(format!("Invalid `{name}` — expected a number").into());
            }
        }
    }
    Ok(())
}

fn validate_duration(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                parse_duration(val).map_err(|e| format!("Invalid `{name}` duration: {e}"))?;
            } else {
                return Err(format!("Invalid `{name}` — expected a duration string").into());
            }
        }
    }
    Ok(())
}

fn validate_enum(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
    variants: &[&str],
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                if !variants.contains(&val) {
                    return Err(format!(
                        "Invalid `{name}` — expected one of: {}",
                        variants.join(", ")
                    )
                    .into());
                }
            } else {
                return Err(format!("Invalid `{name}` — expected a string").into());
            }
        }
    }
    Ok(())
}

fn validate_request_header(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("request_header") {
        used.insert("request_header".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("request_header requires at least one argument".into());
            }
            let first = e.args[0]
                .as_str()
                .ok_or("The header name must be a string")?;
            let (name, needs_value) = match first.chars().next() {
                Some('+') => (&first[1..], true),
                Some('-') => (&first[1..], false),
                _ => (first, true),
            };
            HeaderName::from_str(name).map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            if needs_value
                && e.args
                    .get(1)
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .is_none()
            {
                return Err("request_header requires a value for add/replace operations".into());
            }
        }
    }
    Ok(())
}

fn validate_upstream_directives(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("upstream") {
        used.insert("upstream".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("The `upstream` directive requires a URL argument".into());
            }
            if let Some(up_block) = &e.children {
                validate_upstream_block(up_block, used)?;
            }
        }
    }
    Ok(())
}

fn validate_upstream_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_number(block, used, "limit", 1)?;
    validate_duration(block, used, "idle_timeout")?;
    validate_str(block, used, "unix")?;
    #[cfg(not(unix))]
    if block.directives.contains_key("unix") {
        return Err("Unix sockets are not supported on this platform".into());
    }
    Ok(())
}

/// Validate SRV upstream directives.
#[cfg(feature = "srv-lookup")]
fn validate_srv_directives(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("srv") {
        used.insert("srv".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("The `srv` directive requires an SRV record name argument".into());
            }
            if let Some(srv_block) = &e.children {
                validate_srv_block(srv_block, used)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "srv-lookup")]
fn validate_srv_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_number(block, used, "limit", 1)?;
    validate_duration(block, used, "idle_timeout")?;
    validate_str(block, used, "dns_servers")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_expected_status_2xx() {
        let status = parse_expected_status("2xx").unwrap();
        assert!(status.matches(200));
        assert!(status.matches(299));
        assert!(!status.matches(300));
    }

    #[test]
    fn test_parse_expected_status_2xx_3xx() {
        let status = parse_expected_status("2xx,3xx").unwrap();
        assert!(status.matches(200));
        assert!(status.matches(399));
        assert!(!status.matches(400));
    }

    #[test]
    fn test_parse_expected_status_specific() {
        let status = parse_expected_status("200").unwrap();
        assert!(status.matches(200));
        assert!(!status.matches(201));
    }

    #[test]
    fn test_parse_expected_status_list() {
        let status = parse_expected_status("200,201,204").unwrap();
        assert!(status.matches(200));
        assert!(status.matches(201));
        assert!(status.matches(204));
        assert!(!status.matches(202));
    }

    #[test]
    fn test_parse_expected_status_range() {
        let status = parse_expected_status("200-299").unwrap();
        assert!(status.matches(200));
        assert!(status.matches(250));
        assert!(status.matches(299));
        assert!(!status.matches(199));
        assert!(!status.matches(300));
    }

    #[test]
    fn test_parse_expected_status_invalid_code() {
        let result = parse_expected_status("invalid");
        assert!(result.is_err());
    }
}
