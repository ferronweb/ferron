//! Configuration parsing for the reverse proxy module.

use std::collections::HashMap;
use std::error::Error;
#[cfg(feature = "srv-lookup")]
use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use http::header::HeaderName;

pub use crate::types::affinity::{AffinityConfig, AffinityType};
use crate::types::affinity::{CookieAffinityConfig, SameSiteMode};
use crate::types::health::{ExpectedStatusCodes, HealthCheckMethod, UpstreamHealthCheckConfig};
use crate::types::lb::LoadBalancerAlgorithm;
#[cfg(feature = "srv-lookup")]
use crate::types::upstream::SrvUpstreamData;
use crate::types::upstream::{ProxyHeader, Upstream, UpstreamConfig};

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

/// Passive health check configuration for the reverse proxy.
#[derive(Clone)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub max_fails: u64,
    pub window: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_fails: 3,
            window: Duration::from_millis(5000),
        }
    }
}

/// Circuit breaker configuration for the reverse proxy.
#[derive(Clone)]
pub struct CircuitBreakerConfig {
    pub enabled: bool,
    pub max_fails: u64,
    pub window: Duration,
    pub open_duration: Duration,
    pub consecutive_passes: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_fails: 5,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
        }
    }
}

/// Parsed reverse proxy configuration.
#[derive(Clone)]
pub struct ProxyConfig {
    pub upstreams: Vec<Upstream>,
    pub algorithm: LoadBalancerAlgorithm,
    pub passive_check: HealthCheckConfig,
    pub circuit_breaker: CircuitBreakerConfig,
    pub retry_connection: bool,
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
    /// Session affinity configuration.
    pub affinity: Option<AffinityConfig>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstreams: Vec::new(),
            algorithm: LoadBalancerAlgorithm::TwoRandomChoices,
            passive_check: HealthCheckConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
            retry_connection: true,
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
            affinity: None,
        }
    }
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
        if let Some(url) = arg.as_string_with_interpolations(ctx) {
            cfg.upstreams.push(Upstream::Static(UpstreamConfig {
                url: url.clone(),
                unix_socket: None,
                limit: None,
                idle_timeout: Some(default_timeout),
                health_check_config: UpstreamHealthCheckConfig::default(),
                weight: 1,
            }));
            cfg.idle_timeout_map.insert(url, default_timeout);
        }
    }

    // Parse block if present
    if let Some(children) = &entry.children {
        parse_proxy_block(children, &mut cfg, ctx)?;
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
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "upstream" => {
                for entry in entries {
                    parse_upstream_entry(entry, cfg, ctx)?;
                }
            }
            #[cfg(feature = "srv-lookup")]
            "srv" => {
                for entry in entries {
                    parse_srv_entry(entry, cfg, ctx)?;
                }
            }
            "algorithm" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.algorithm = match val {
                        "random" => LoadBalancerAlgorithm::Random,
                        "round_robin" => LoadBalancerAlgorithm::RoundRobin,
                        "least_conn" => LoadBalancerAlgorithm::LeastConnections,
                        "two_random" => LoadBalancerAlgorithm::TwoRandomChoices,
                        "p2c_ewma" => LoadBalancerAlgorithm::P2cEwma,
                        _ => {
                            return Err(
                                format!("Unsupported load balancing algorithm: {val}").into()
                            )
                        }
                    };
                }
            }
            "passive_check" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.passive_check.enabled = val;
                    if val {
                        if let Some(children) = entries.first().and_then(|e| e.children.as_ref()) {
                            parse_passive_health_check(children, &mut cfg.passive_check)?;
                        }
                    }
                }
            }
            "circuit_breaker" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.circuit_breaker.enabled = val;
                    if val {
                        if let Some(children) = entries.first().and_then(|e| e.children.as_ref()) {
                            parse_circuit_breaker(children, &mut cfg.circuit_breaker)?;
                        }
                    }
                }
            }
            "retry_connection" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.retry_connection = val;
                }
            }
            "keepalive" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.keepalive = val;
                }
            }
            "http2" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.http2 = val;
                }
            }
            "http2_only" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.http2_only = val;
                }
            }
            "intercept_errors" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.intercept_errors = val;
                }
            }
            "no_verification" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
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
                    parse_request_header_entry(entry, cfg, ctx)?;
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
            "affinity" => {
                if let Some(entry) = entries.first() {
                    if let Some(type_val) = entry.args.first().and_then(|v| v.as_str()) {
                        cfg.affinity = Some(parse_affinity_entry(type_val, entry, ctx)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn parse_passive_health_check(
    entries: &ServerConfigurationBlock,
    health_check_config: &mut HealthCheckConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "max_fails" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    health_check_config.max_fails = val as u64;
                }
            }
            "window" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.window = val;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_active_health_check(
    entries: &ServerConfigurationBlock,
    health_check_config: &mut UpstreamHealthCheckConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "uri" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.uri = val.to_string();
                }
            }
            "method" => {
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
            "interval" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.interval = val;
                }
            }
            "timeout" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.timeout = val;
                }
            }
            "expect_status" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.expect_status = parse_expected_status(val)?;
                }
            }
            "response_time_threshold" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.response_time_threshold = Some(val);
                }
            }
            "body_match" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.body_match = Some(val.to_string());
                }
            }
            "consecutive_fails" => {
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
            "consecutive_passes" => {
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
            "no_verification" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    health_check_config.no_verification = val;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn parse_circuit_breaker(
    entries: &ServerConfigurationBlock,
    circuit_breaker_config: &mut CircuitBreakerConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "max_fails" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        circuit_breaker_config.max_fails = val as u64;
                    }
                }
            }
            "window" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.window = val;
                }
            }
            "open_duration" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.open_duration = val;
                }
            }
            "consecutive_passes" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        circuit_breaker_config.consecutive_passes = val as u64;
                    }
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
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let url = entry
        .args
        .first()
        .and_then(|v| v.as_string_with_interpolations(ctx))
        .ok_or("upstream requires a URL argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut unix_socket: Option<String> = None;
    let mut health_check_config = UpstreamHealthCheckConfig::default();
    let mut weight: u32 = 1;

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
                        .and_then(|v| v.as_duration())
                    {
                        idle_timeout = Some(val);
                    }
                }
                "unix" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        unix_socket = Some(val);
                    }
                }
                "weight" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            weight = val as u32;
                        }
                    }
                }
                "active_check" => {
                    if let Some(val) = entries.first().map(|e| e.get_flag()) {
                        health_check_config.enabled = val;
                        if val {
                            if let Some(children) =
                                entries.first().and_then(|e| e.children.as_ref())
                            {
                                parse_active_health_check(children, &mut health_check_config)?;
                            }
                        }
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
        weight,
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
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let srv_name = entry
        .args
        .first()
        .and_then(|v| v.as_string_with_interpolations(ctx))
        .ok_or("srv requires an SRV record name argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut dns_servers: Vec<IpAddr> = Vec::new();
    let mut weight: u32 = 1;
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
                        .and_then(|v| v.as_duration())
                    {
                        idle_timeout = Some(val);
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
                "weight" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            weight = val as u32;
                        }
                    }
                }
                "active_check" => {
                    if let Some(val) = entries.first().map(|e| e.get_flag()) {
                        health_check_config.enabled = val;
                        if val {
                            if let Some(children) =
                                entries.first().and_then(|e| e.children.as_ref())
                            {
                                parse_active_health_check(children, &mut health_check_config)?;
                            }
                        }
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
        weight,
        health_check_config,
    }));

    Ok(())
}

fn parse_request_header_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
    ctx: &ferron_http::HttpContext,
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
                .and_then(|v| v.as_string_with_interpolations(ctx))
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
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("request_header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_replace.push((header_name, value));
        }
    }

    Ok(())
}

fn parse_affinity_entry(
    type_val: &str,
    entry: &ServerConfigurationDirectiveEntry,
    _ctx: &ferron_http::HttpContext,
) -> Result<AffinityConfig, Box<dyn Error + Send + Sync>> {
    let affinity_type = match type_val {
        "cookie" => {
            let mut cookie_cfg = CookieAffinityConfig::default();
            if let Some(block) = &entry.children {
                for (name, entries) in block.directives.iter() {
                    match name.as_str() {
                        "name" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.name = val.to_string();
                            }
                        }
                        "ttl" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_duration())
                            {
                                cookie_cfg.ttl = Some(val);
                            }
                        }
                        "path" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.path = val.to_string();
                            }
                        }
                        "domain" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.domain = Some(val.to_string());
                            }
                        }
                        "secure" => {
                            cookie_cfg.secure =
                                entries.first().map(|e| e.get_flag()).unwrap_or(true);
                        }
                        "httponly" => {
                            cookie_cfg.httponly =
                                entries.first().map(|e| e.get_flag()).unwrap_or(true);
                        }
                        "samesite" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.samesite = match val.to_lowercase().as_str() {
                                    "strict" => SameSiteMode::Strict,
                                    "lax" => SameSiteMode::Lax,
                                    "none" => SameSiteMode::None,
                                    _ => {
                                        return Err(format!(
                                            "Invalid samesite mode: {val}, must be strict, lax, or none"
                                        )
                                        .into())
                                    }
                                };
                            }
                        }
                        _ => {}
                    }
                }
            }
            AffinityType::Cookie(cookie_cfg)
        }
        "header" => {
            let header_name = entry
                .children
                .as_ref()
                .and_then(|block| block.directives.get("name"))
                .and_then(|entries| entries.first())
                .and_then(|e| e.args.first())
                .and_then(|v| v.as_str())
                .ok_or("header affinity requires a 'name' subdirective")?;
            let header_name = HeaderName::from_str(header_name)
                .map_err(|e| format!("Invalid header name '{header_name}': {e}"))?;
            AffinityType::Header(header_name)
        }
        "ip" => AffinityType::Ip,
        "hash" => {
            let mut variable: Option<String> = None;
            if let Some(block) = &entry.children {
                for (name, entries) in block.directives.iter() {
                    if name.as_str() == "variable" {
                        if let Some(val) = entries
                            .first()
                            .and_then(|e| e.args.first())
                            .and_then(|v| v.as_str())
                        {
                            variable = Some(val.to_string());
                        }
                    }
                }
            }
            let variable = variable.ok_or("hash affinity requires a 'variable' subdirective")?;
            AffinityType::Hash { variable }
        }
        _ => {
            return Err(format!(
                "Invalid affinity type: {type_val}, must be cookie, header, ip, or hash"
            )
            .into())
        }
    };

    Ok(AffinityConfig { affinity_type })
}
