//! Health check types and state tracking.

use std::time::Duration;

/// HTTP method for active health checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HealthCheckMethod {
    /// HTTP GET request.
    Get,
    /// HTTP HEAD request.
    Head,
}

impl HealthCheckMethod {
    /// Return the string representation for the HTTP method.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthCheckMethod::Get => "GET",
            HealthCheckMethod::Head => "HEAD",
        }
    }
}

/// Expected HTTP status codes for health check success.
#[derive(Clone, Debug)]
pub enum ExpectedStatusCodes {
    /// Match 2xx responses.
    Successful,
    /// Match 2xx and 3xx responses.
    SuccessfulOrRedirect,
    /// Match specific status code.
    Specific(u16),
    /// Match any status code in the list.
    Any(Vec<u16>),
    /// Match status codes in the range [start, end] inclusive.
    Range(u16, u16),
}

impl ExpectedStatusCodes {
    /// Check if a given status code matches the expected set.
    #[inline]
    pub fn matches(&self, status: u16) -> bool {
        match self {
            ExpectedStatusCodes::Successful => (200..300).contains(&status),
            ExpectedStatusCodes::SuccessfulOrRedirect => (200..400).contains(&status),
            ExpectedStatusCodes::Specific(code) => status == *code,
            ExpectedStatusCodes::Any(codes) => codes.contains(&status),
            ExpectedStatusCodes::Range(start, end) => (*start..=*end).contains(&status),
        }
    }
}

/// Active health check configuration for an upstream.
///
/// This struct defines how the proxy actively probes upstream backends
/// to determine their health status. When enabled, the proxy sends
/// periodic HTTP requests to a configured endpoint.
#[derive(Clone, Debug)]
pub struct UpstreamHealthCheckConfig {
    /// Enable active health checks for this upstream.
    pub enabled: bool,
    /// HTTP method for probe requests (GET or HEAD).
    pub method: HealthCheckMethod,
    /// Endpoint to probe (e.g., `/health`).
    pub uri: String,
    /// Interval between probes.
    pub interval: Duration,
    /// Max wait time for probe response.
    pub timeout: Duration,
    /// Expected HTTP status codes for success.
    pub expect_status: ExpectedStatusCodes,
    /// Max response time threshold. If set, mark unhealthy if response takes longer.
    pub response_time_threshold: Option<Duration>,
    /// Optional substring to match in response body (only for GET).
    pub body_match: Option<String>,
    /// Mark unhealthy after N consecutive failures.
    pub consecutive_fails: u64,
    /// Mark healthy after N consecutive successes when recovering.
    pub consecutive_passes: u64,
    /// Skip TLS certificate verification for HTTPS probes.
    pub no_verification: bool,
}

impl Default for UpstreamHealthCheckConfig {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            method: HealthCheckMethod::Get,
            uri: "/health".to_string(),
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(5),
            expect_status: ExpectedStatusCodes::SuccessfulOrRedirect,
            response_time_threshold: None,
            body_match: None,
            consecutive_fails: 2,
            consecutive_passes: 2,
            no_verification: false,
        }
    }
}

/// Health check state for tracking probe results per upstream.
///
/// This struct is updated by the health check task and consumed by
/// the backend selection logic to determine upstream availability.
#[derive(Clone, Debug)]
pub struct HealthCheckState {
    /// Current health status: true = healthy, false = unhealthy.
    pub is_healthy: bool,
    /// Consecutive failure counter when unhealthy.
    pub consecutive_fail_count: u64,
    /// Consecutive success counter when recovering.
    pub consecutive_pass_count: u64,
    /// Last probe result status code (if available).
    pub last_probe_status: Option<u16>,
    /// Last probe error message (if any).
    pub last_probe_error: Option<String>,
    /// Timestamp of last successful probe.
    pub last_success_time: Option<std::time::SystemTime>,
    /// Timestamp of last failed probe.
    pub last_failure_time: Option<std::time::SystemTime>,
}

impl Default for HealthCheckState {
    #[inline]
    fn default() -> Self {
        Self {
            is_healthy: true,
            consecutive_fail_count: 0,
            consecutive_pass_count: 0,
            last_probe_status: None,
            last_probe_error: None,
            last_success_time: None,
            last_failure_time: None,
        }
    }
}

/// Health check state map keyed by upstream URL string.
pub type HealthCheckStateMap =
    std::sync::Arc<dashmap::DashMap<String, HealthCheckState, rustc_hash::FxBuildHasher>>;
