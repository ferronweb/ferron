//! Abuse protection types shared across HTTP modules.
//!
//! This module defines the common types for abuse event recording and a global
//! recorder holder. The actual abuse protection implementation lives in the
//! `ferron-http-abuseban` crate, while rate limiting and basic auth modules
//! emit events through the shared trait defined here.

use std::net::IpAddr;
use std::sync::{Arc, OnceLock};

use crate::HttpContext;

/// Event types that can trigger an abuse ban.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbuseEventType {
    /// Rate limit threshold exceeded.
    RateLimitExceeded,
    /// Brute-force auth failure threshold exceeded.
    BruteForceFailure,
    /// Custom threshold (for extensibility).
    Custom,
    /// Error rate threshold exceeded (e.g., too many 404/403 responses).
    ErrorRate,
}

impl AbuseEventType {
    /// String representation for observability.
    pub fn as_str(&self) -> &'static str {
        match self {
            AbuseEventType::RateLimitExceeded => "rate_limit_exceeded",
            AbuseEventType::BruteForceFailure => "brute_force_failure",
            AbuseEventType::Custom => "custom",
            AbuseEventType::ErrorRate => "error_rate",
        }
    }
}

/// An abuse event reported by the system (e.g., rate limit breach).
#[derive(Debug, Clone)]
pub struct AbuseEvent {
    /// Type of abuse event.
    pub event_type: AbuseEventType,
    /// IP address involved.
    pub ip: IpAddr,
    /// Human-readable reason (e.g., "rate limit 100 req/s exceeded").
    pub reason: String,
    /// Severity level (0-100, higher = more severe).
    pub severity: u8,
    /// HTTP status code, if this event is related to a response (e.g., for ErrorRate events).
    pub status_code: Option<u16>,
}

impl AbuseEvent {
    /// Create a new abuse event.
    pub fn new(event_type: AbuseEventType, ip: IpAddr, reason: String, severity: u8) -> Self {
        Self {
            event_type,
            ip,
            reason,
            severity,
            status_code: None,
        }
    }

    /// Create a new abuse event with an associated HTTP status code.
    pub fn with_status_code(
        event_type: AbuseEventType,
        ip: IpAddr,
        reason: String,
        severity: u8,
        status_code: u16,
    ) -> Self {
        Self {
            event_type,
            ip,
            reason,
            severity,
            status_code: Some(status_code),
        }
    }
}

/// Result of recording an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventResult {
    /// Event was recorded but threshold not met.
    Recorded,
    /// Threshold was reached and IP is now banned.
    BanTriggered,
}

/// Trait for recording abuse events and checking ban status.
///
/// Implemented by the abuse protection registry and accessible via the
/// global recorder for cross-module event emission.
pub trait AbuseRecorder: Send + Sync {
    /// Record an abuse event. Returns whether a ban was triggered.
    fn record_event(&self, event: &AbuseEvent, ctx: &HttpContext) -> EventResult;
    /// Check if an IP address is currently banned.
    fn is_banned(&self, ip: IpAddr, ctx: &HttpContext) -> bool;
}

// Blanket impl so that Arc<T: AbuseRecorder>, Box<T: AbuseRecorder>, etc. all work
impl<T: AbuseRecorder + ?Sized> AbuseRecorder for std::sync::Arc<T> {
    fn record_event(&self, event: &AbuseEvent, ctx: &HttpContext) -> EventResult {
        (**self).record_event(event, ctx)
    }

    fn is_banned(&self, ip: IpAddr, ctx: &HttpContext) -> bool {
        (**self).is_banned(ip, ctx)
    }
}

// Global holder for the shared abuse recorder
static GLOBAL_ABUSE_RECORDER: OnceLock<Arc<dyn AbuseRecorder>> = OnceLock::new();

/// Set the global abuse recorder. Returns an error if already set.
pub fn set_global_abuse_recorder(
    recorder: Arc<dyn AbuseRecorder>,
) -> Result<(), Arc<dyn AbuseRecorder>> {
    GLOBAL_ABUSE_RECORDER.set(recorder)
}

/// Get the global abuse recorder, if one has been set.
pub fn get_global_abuse_recorder() -> Option<&'static dyn AbuseRecorder> {
    GLOBAL_ABUSE_RECORDER.get().map(|r| &**r)
}
