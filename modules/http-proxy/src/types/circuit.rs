//! Circuit breaker types.

#[cfg(not(target_has_atomic = "64"))]
use std::sync::atomic::AtomicU32;
#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;

pub const CIRCUIT_BREAKER_STATUS_CLOSED: u8 = 0;
pub const CIRCUIT_BREAKER_STATUS_OPEN: u8 = 1;
pub const CIRCUIT_BREAKER_STATUS_HALFOPEN: u8 = 2;

/// Returns a human-readable label for the circuit breaker status value.
pub fn circuit_breaker_state_label(status: u8) -> &'static str {
    match status {
        CIRCUIT_BREAKER_STATUS_CLOSED => "closed",
        CIRCUIT_BREAKER_STATUS_OPEN => "open",
        CIRCUIT_BREAKER_STATUS_HALFOPEN => "half_open",
        _ => "unknown",
    }
}

/// Circuit breaker state for tracking failures per upstream.
///
/// This state is stored in a [`DashMap`] keyed by [`UpstreamInner`].
#[derive(Clone, Debug)]
pub struct CircuitBreakerState {
    /// Queue of failure timestamps within the configured window.
    pub recent_failures: Option<Arc<crossbeam_queue::ArrayQueue<std::time::Instant>>>,
    /// Current state machine status.
    pub status: Arc<AtomicU8>,
    /// When the circuit opened.
    pub opened_at: Arc<parking_lot::RwLock<Option<std::time::Instant>>>,
    /// Whether a request is currently in-flight in the half-open state.
    pub half_open_in_flight: Arc<AtomicBool>,
    /// Number of successful requests in half-open state.
    #[cfg(target_has_atomic = "64")]
    pub half_open_pass_count: Arc<AtomicU64>,
    /// Number of successful requests in half-open state.
    #[cfg(not(target_has_atomic = "64"))]
    pub half_open_pass_count: Arc<AtomicU32>,
    /// When the circuit breaker transitioned from HALFOPEN to CLOSED (slow-start recovery timestamp).
    /// `Some` means the upstream is in slow-start; the LB applies a decaying virtual connection penalty.
    /// Cleared on OPEN → HALFOPEN transition.
    pub slow_start_recovery_at: Option<Arc<parking_lot::RwLock<Option<std::time::Instant>>>>,
}

impl Default for CircuitBreakerState {
    fn default() -> Self {
        Self {
            recent_failures: None,
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_CLOSED)),
            opened_at: Arc::new(parking_lot::RwLock::new(None)),
            half_open_in_flight: Arc::new(AtomicBool::new(false)),
            #[cfg(target_has_atomic = "64")]
            half_open_pass_count: Arc::new(AtomicU64::new(0)),
            #[cfg(not(target_has_atomic = "64"))]
            half_open_pass_count: Arc::new(AtomicU32::new(0)),
            slow_start_recovery_at: None,
        }
    }
}

pub type CircuitBreakerStateMap = Arc<
    dashmap::DashMap<
        Arc<super::upstream::UpstreamInner>,
        CircuitBreakerState,
        rustc_hash::FxBuildHasher,
    >,
>;
