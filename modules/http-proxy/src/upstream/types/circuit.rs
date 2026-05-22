//! Circuit breaker types.

use std::sync::Arc;

/// Circuit breaker status in the state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CircuitBreakerStatus {
    Closed,
    Open,
    HalfOpen,
}

/// Circuit breaker state for tracking failures per upstream.
///
/// This state is stored in a [`DashMap`] keyed by [`UpstreamInner`].
#[derive(Clone, Debug)]
pub(crate) struct CircuitBreakerState {
    /// Queue of failure timestamps within the configured window.
    pub recent_failures: std::collections::VecDeque<std::time::Instant>,
    /// Current state machine status.
    pub status: CircuitBreakerStatus,
    /// When the circuit opened.
    pub opened_at: Option<std::time::Instant>,
    /// Whether a request is currently in-flight in the half-open state.
    pub half_open_in_flight: bool,
    /// Number of successful requests in half-open state.
    pub half_open_pass_count: u64,
}

impl Default for CircuitBreakerState {
    fn default() -> Self {
        Self {
            recent_failures: std::collections::VecDeque::new(),
            status: CircuitBreakerStatus::Closed,
            opened_at: None,
            half_open_in_flight: false,
            half_open_pass_count: 0,
        }
    }
}

pub type CircuitBreakerStateMap =
    Arc<dashmap::DashMap<crate::upstream::UpstreamInner, CircuitBreakerState>>;
