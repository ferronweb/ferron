use std::time::Duration;

use crate::types::lb::LoadBalancerAlgorithm;
use crate::types::upstream::{ProxyHeader, Upstream};

/// A header action: currently only append is supported for `request_header +Name`.
#[derive(Clone)]
pub enum HeaderAction {
    /// Append the given value to the header.
    Append(http::header::HeaderName, String),
}

/// Retry budget configuration for the reverse proxy.
#[derive(Clone)]
pub struct RetryBudgetConfig {
    /// Maximum retry rate as a fraction of steady-state traffic (0.0–1.0).
    pub max_retry_rate: f64,
    /// Maximum number of tokens in the bucket (burst capacity).
    pub max_tokens: u64,
    /// Tokens added per second (steady-state traffic deposit rate).
    pub refill_rate: f64,
}

impl Default for RetryBudgetConfig {
    fn default() -> Self {
        Self {
            max_retry_rate: 0.1,
            max_tokens: 10,
            refill_rate: 2.0,
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
    pub record_5xx: bool,
    pub latency_threshold: Option<Duration>,
    pub flapping_transitions: u64,
    pub flapping_window: Duration,
    pub slow_start_duration: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_fails: 5,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: false,
            latency_threshold: None,
            flapping_transitions: 3,
            flapping_window: Duration::from_secs(10),
            slow_start_duration: Duration::ZERO,
        }
    }
}

/// Parsed reverse proxy configuration.
#[derive(Clone)]
pub struct ProxyConfig {
    pub upstreams: Vec<Upstream>,
    pub algorithm: LoadBalancerAlgorithm,
    pub circuit_breaker: CircuitBreakerConfig,
    pub retry_connection: bool,
    pub retry_budget: Option<RetryBudgetConfig>,
    pub keepalive: bool,
    pub http2: bool,
    pub http2_only: bool,
    pub intercept_errors: bool,
    pub no_verification: bool,
    pub proxy_header: Option<ProxyHeader>,
    pub headers_to_add: Vec<HeaderAction>,
    pub headers_to_replace: Vec<(http::header::HeaderName, String)>,
    pub headers_to_remove: Vec<http::header::HeaderName>,
    pub concurrent_conns: Option<usize>,
    pub affinity: Option<crate::types::affinity::AffinityConfig>,
    pub metrics_resolved_ip: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstreams: Vec::new(),
            algorithm: LoadBalancerAlgorithm::TwoRandomChoices,
            circuit_breaker: CircuitBreakerConfig::default(),
            retry_connection: true,
            retry_budget: None,
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
            affinity: None,
            metrics_resolved_ip: false,
        }
    }
}
