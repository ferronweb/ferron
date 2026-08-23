//! Circuit breaker implementation for protecting upstream backends.

mod tests;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::{
    CircuitBreakerStateMap, CIRCUIT_BREAKER_STATUS_CLOSED, CIRCUIT_BREAKER_STATUS_HALFOPEN,
    CIRCUIT_BREAKER_STATUS_OPEN,
};
use crate::types::flapping::FlappingStateMap;
use crate::types::upstream::UpstreamInner;

/// A view over the circuit-breaker state and configuration for one request.
///
/// Pure reads (`is_available`, `is_open`, `state`) never mutate state;
/// `try_acquire` performs the state transitions and emits events.
pub struct CircuitBreaker<'a> {
    state: Option<&'a CircuitBreakerStateMap>,
    flapping_state: Option<&'a FlappingStateMap>,
    config: &'a CircuitBreakerConfig,
    event_sink: &'a ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
}

impl<'a> CircuitBreaker<'a> {
    #[inline]
    pub fn new(
        state: Option<&'a CircuitBreakerStateMap>,
        flapping_state: Option<&'a FlappingStateMap>,
        config: &'a CircuitBreakerConfig,
        event_sink: &'a ferron_observability::CompositeEventSink,
        event_trace_context: Option<ferron_observability::EventTraceContext>,
        metrics_resolved_ip: bool,
    ) -> Self {
        Self {
            state,
            flapping_state,
            config,
            event_sink,
            event_trace_context,
            metrics_resolved_ip,
        }
    }

    /// The underlying state map, used by the load-balancer selectors.
    #[inline]
    pub fn state(&self) -> Option<&'a CircuitBreakerStateMap> {
        self.state
    }

    /// The slow-start duration from the circuit-breaker configuration.
    #[inline]
    pub fn slow_start_duration(&self) -> Duration {
        self.config.slow_start_duration
    }

    /// Whether a backend is currently available for new circuit-breaker traffic.
    #[inline]
    pub fn is_available(&self, upstream: &Arc<UpstreamInner>) -> bool {
        if !self.config.enabled {
            return true;
        }

        let Some(state) = self.state.and_then(|s| s.get(upstream)) else {
            return true;
        };

        match state.status.load(Ordering::Relaxed) {
            CIRCUIT_BREAKER_STATUS_CLOSED => true,
            CIRCUIT_BREAKER_STATUS_OPEN => state
                .opened_at
                .read()
                .is_some_and(|opened_at| opened_at.elapsed() >= self.config.open_duration),
            CIRCUIT_BREAKER_STATUS_HALFOPEN => !state.half_open_in_flight.load(Ordering::Relaxed),
            _ => false,
        }
    }

    /// Whether a backend's circuit breaker is currently in the open state.
    #[inline]
    pub fn is_open(&self, upstream: &Arc<UpstreamInner>) -> bool {
        self.config
            .enabled
            .then_some(self.state)
            .flatten()
            .and_then(|s| s.get(upstream))
            .is_some_and(|s| s.status.load(Ordering::Relaxed) == CIRCUIT_BREAKER_STATUS_OPEN)
    }

    /// Record a circuit breaker slot acquisition attempt.
    ///
    /// The decision is delegated to [`Self::is_available`]: backends that
    /// are unavailable (closed-circuit cooldown, half-open slot busy) are
    /// rejected without any state transitions. Otherwise the state machine
    /// advances (open -> half-open) and the acquisition is emitted.
    #[inline]
    pub fn try_acquire(&self, upstream: &Arc<UpstreamInner>) -> bool {
        if !self.is_available(upstream) {
            return false;
        }

        if !self.config.enabled {
            return true;
        }

        let Some(circuit_breaker_state) = self.state else {
            return true;
        };

        let state = if let Some(state) = circuit_breaker_state.get(upstream) {
            state
        } else {
            let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();
            state.recent_failures = (self.config.max_fails > 1).then(|| {
                Arc::new(crossbeam_queue::ArrayQueue::new(
                    (self.config.max_fails as usize).saturating_sub(1),
                ))
            });
            state.downgrade()
        };

        match state.status.load(Ordering::Relaxed) {
            CIRCUIT_BREAKER_STATUS_CLOSED => true,
            CIRCUIT_BREAKER_STATUS_OPEN => {
                let opened_at = {
                    let mut opened_at_ref = state.opened_at.upgradable_read();
                    let opened_at = if let Some(opened_at) = &*opened_at_ref {
                        opened_at
                    } else {
                        // This might be a rare edge case, reset the opened_at counter...
                        opened_at_ref.with_upgraded(|r| {
                            if r.is_some() {
                                // Double check, to not overwrite the instant
                                return;
                            }
                            *r = Some(std::time::Instant::now());
                        });
                        let Some(opened_at) = &*opened_at_ref else {
                            // At this point, something else has overwriten the value, so return `false`
                            return false;
                        };
                        opened_at
                    };

                    if opened_at.elapsed() < self.config.open_duration {
                        return false;
                    }

                    *opened_at
                };

                state
                    .status
                    .store(CIRCUIT_BREAKER_STATUS_HALFOPEN, Ordering::Relaxed);
                *state.opened_at.write() = None;
                state.half_open_in_flight.store(true, Ordering::Relaxed);
                state.half_open_pass_count.store(0, Ordering::Relaxed);
                // Clear slow-start recovery timestamp; the circuit is re-entering
                // half-open, so the slow-start penalty should not apply.
                if let Some(ref recovery_at) = state.slow_start_recovery_at {
                    *recovery_at.write() = None;
                }
                crate::upstream::flapping::record_circuit_transition(
                    self.flapping_state,
                    self.config,
                    &upstream.proxy_to,
                    self.event_sink,
                    self.event_trace_context.clone(),
                );
                self.event_sink.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Info,
                        message: format!(
                            "Upstream {} circuit transitioned to half-open after {:?}",
                            upstream.proxy_to,
                            opened_at.elapsed()
                        ),
                        summary: "Upstream circuit transitioned to half-open".into(),
                        target: crate::LOG_TARGET,
                        attributes: vec![
                            (
                                "upstream.address",
                                ferron_observability::LogAttributeValue::String(
                                    upstream.proxy_to.clone(),
                                ),
                            ),
                            (
                                "ferron.proxy.circuit.open_duration_ms",
                                ferron_observability::LogAttributeValue::I64(
                                    opened_at.elapsed().as_millis() as i64,
                                ),
                            ),
                        ],
                        trace_context: self.event_trace_context.clone(),
                    },
                ));
                emit_circuit_metric(
                    self.event_sink,
                    upstream,
                    "ferron.proxy.circuit.state",
                    ferron_observability::MetricType::Gauge,
                    ferron_observability::MetricValue::U64(CIRCUIT_BREAKER_STATUS_HALFOPEN as u64),
                    self.event_trace_context.clone(),
                    self.metrics_resolved_ip,
                );
                emit_circuit_metric(
                    self.event_sink,
                    upstream,
                    "ferron.proxy.circuit.open_total",
                    ferron_observability::MetricType::Counter,
                    ferron_observability::MetricValue::I64(-1),
                    self.event_trace_context.clone(),
                    self.metrics_resolved_ip,
                );
                true
            }
            CIRCUIT_BREAKER_STATUS_HALFOPEN => {
                !state.half_open_in_flight.swap(true, Ordering::Relaxed)
            }
            _ => false, // Possibly corrupted state
        }
    }
}

/// Record a transport-level backend failure.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn record_backend_transport_failure(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    flapping_state: Option<&crate::types::flapping::FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    if record_circuit_breaker_failure(
        circuit_breaker_state,
        flapping_state,
        circuit_breaker,
        upstream,
        event_sink,
        event_trace_context,
        metrics_resolved_ip,
    ) {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

/// Record an upstream response for the circuit breaker state machine.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn record_backend_response(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    flapping_state: Option<&crate::types::flapping::FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    status: u16,
    upstream_time_secs: Option<f64>,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
    trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    let is_5xx_failure = circuit_breaker.record_5xx && (500..600).contains(&status);
    let is_latency_failure = circuit_breaker.latency_threshold.is_some_and(|threshold| {
        upstream_time_secs.is_some_and(|t| std::time::Duration::from_secs_f64(t) > threshold)
    });

    let should_open = if is_5xx_failure || is_latency_failure {
        record_circuit_breaker_failure(
            circuit_breaker_state,
            flapping_state,
            circuit_breaker,
            upstream,
            event_sink,
            trace_context,
            metrics_resolved_ip,
        )
    } else {
        record_circuit_breaker_success(
            circuit_breaker_state,
            flapping_state,
            circuit_breaker,
            upstream,
            event_sink,
            trace_context,
            metrics_resolved_ip,
        );
        false
    };

    if should_open {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

#[inline]
fn emit_circuit_metric(
    event_sink: &ferron_observability::CompositeEventSink,
    upstream: &Arc<UpstreamInner>,
    name: &'static str,
    metric_type: ferron_observability::MetricType,
    value: ferron_observability::MetricValue,
    trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent};

    let mut attributes = Vec::with_capacity(4);
    attributes.push((
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream.proxy_to.clone()),
    ));
    if let Some(ref unix_path) = upstream.proxy_unix {
        attributes.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(unix_path.clone()),
        ));
    }
    if metrics_resolved_ip {
        if let Some(ref resolved_ip) = upstream.connect_to {
            attributes.push((
                "ferron.proxy.backend_resolved_ip",
                MetricAttributeValue::String(resolved_ip.to_string()),
            ));
        }
        attributes.push((
            "ferron.proxy.dns_status",
            MetricAttributeValue::String(upstream.dns_status.as_label().to_string()),
        ));
    }
    event_sink.emit(Event::Metric(MetricEvent {
        name,
        attributes,
        ty: metric_type,
        value,
        unit: Some("{circuit}"),
        description: Some("Circuit breaker state and transitions for upstream backends."),
        trace_context,
    }));
}

#[inline]
fn record_circuit_breaker_failure(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    flapping_state: Option<&crate::types::flapping::FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) -> bool {
    if !circuit_breaker.enabled {
        return false;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return false;
    };

    let now = std::time::Instant::now();
    let state = if let Some(state) = circuit_breaker_state.get(upstream) {
        let recent_failures_would_some = circuit_breaker.max_fails > 1;
        if (!recent_failures_would_some && state.recent_failures.is_some())
            || (recent_failures_would_some
                && state.recent_failures.as_ref().is_none_or(|q| {
                    q.capacity() != (circuit_breaker.max_fails as usize).saturating_sub(1)
                }))
        {
            drop(state);
            let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();
            state.recent_failures = (circuit_breaker.max_fails > 1).then(|| {
                Arc::new(crossbeam_queue::ArrayQueue::new(
                    (circuit_breaker.max_fails as usize).saturating_sub(1),
                ))
            });
            state.downgrade()
        } else {
            state
        }
    } else {
        let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();
        state.recent_failures = (circuit_breaker.max_fails > 1).then(|| {
            Arc::new(crossbeam_queue::ArrayQueue::new(
                (circuit_breaker.max_fails as usize).saturating_sub(1),
            ))
        });
        state.downgrade()
    };

    match state.status.load(Ordering::Relaxed) {
        CIRCUIT_BREAKER_STATUS_HALFOPEN => {
            state
                .status
                .store(CIRCUIT_BREAKER_STATUS_OPEN, Ordering::Relaxed);
            state.half_open_in_flight.store(false, Ordering::Relaxed);
            state.half_open_pass_count.store(0, Ordering::Relaxed);
            if let Some(rf) = &state.recent_failures {
                while rf.pop().is_some() {}
            }
            *state.opened_at.write() = Some(now);
            crate::upstream::flapping::record_circuit_transition(
                flapping_state,
                circuit_breaker,
                &upstream.proxy_to,
                event_sink,
                event_trace_context.clone(),
            );
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Upstream {} circuit reopened after a half-open trial failure",
                        upstream.proxy_to
                    ),
                    summary: "Upstream circuit reopened after half-open trial failure".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                    )],
                    trace_context: event_trace_context.clone(),
                },
            ));
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.state",
                ferron_observability::MetricType::Gauge,
                ferron_observability::MetricValue::U64(CIRCUIT_BREAKER_STATUS_OPEN as u64),
                event_trace_context.clone(),
                metrics_resolved_ip,
            );
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.open_total",
                ferron_observability::MetricType::Counter,
                ferron_observability::MetricValue::I64(1),
                event_trace_context,
                metrics_resolved_ip,
            );
            true
        }
        CIRCUIT_BREAKER_STATUS_OPEN => {
            *state.opened_at.write() = Some(now);
            false
        }
        CIRCUIT_BREAKER_STATUS_CLOSED
            if state.recent_failures.as_ref().is_none_or(|rf| {
                rf.force_push(now)
                    .is_some_and(|timestamp| now.duration_since(timestamp) < circuit_breaker.window)
            }) =>
        {
            state
                .status
                .store(CIRCUIT_BREAKER_STATUS_OPEN, Ordering::Relaxed);

            if let Some(rf) = &state.recent_failures {
                while rf.pop().is_some() {}
            }
            *state.opened_at.write() = Some(now);
            state.half_open_pass_count.store(0, Ordering::Relaxed);
            state.half_open_in_flight.store(false, Ordering::Relaxed);
            crate::upstream::flapping::record_circuit_transition(
                flapping_state,
                circuit_breaker,
                &upstream.proxy_to,
                event_sink,
                event_trace_context.clone(),
            );
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Upstream {} circuit opened after {} failures within {:?}",
                        upstream.proxy_to, circuit_breaker.max_fails, circuit_breaker.window
                    ),
                    summary: "Upstream circuit opened".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                    )],
                    trace_context: event_trace_context.clone(),
                },
            ));
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.state",
                ferron_observability::MetricType::Gauge,
                ferron_observability::MetricValue::U64(CIRCUIT_BREAKER_STATUS_OPEN as u64),
                event_trace_context.clone(),
                metrics_resolved_ip,
            );
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.open_total",
                ferron_observability::MetricType::Counter,
                ferron_observability::MetricValue::I64(1),
                event_trace_context,
                metrics_resolved_ip,
            );
            true
        }
        _ => false,
    }
}

#[inline]
fn record_circuit_breaker_success(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    flapping_state: Option<&crate::types::flapping::FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    if !circuit_breaker.enabled {
        return;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return;
    };

    // Fast path: bail if not in HALFOPEN state.
    if circuit_breaker_state
        .get(upstream)
        .is_none_or(|s| s.status.load(Ordering::Relaxed) != CIRCUIT_BREAKER_STATUS_HALFOPEN)
    {
        return;
    }

    let Some(state) = circuit_breaker_state.get(upstream) else {
        return;
    };

    state.half_open_in_flight.store(false, Ordering::Relaxed);
    let half_open_pass_count = state.half_open_pass_count.fetch_add(1, Ordering::Relaxed) + 1;

    if half_open_pass_count >= circuit_breaker.consecutive_passes {
        state
            .status
            .store(CIRCUIT_BREAKER_STATUS_CLOSED, Ordering::Relaxed);
        *state.opened_at.write() = None;
        state.half_open_pass_count.store(0, Ordering::Relaxed);
        if let Some(rf) = &state.recent_failures {
            while rf.pop().is_some() {}
        }
        // Record slow-start recovery timestamp so the LB applies a decaying
        // virtual connection penalty, preventing thundering herd.
        if circuit_breaker.slow_start_duration > Duration::ZERO {
            if let Some(ssra) = &state.slow_start_recovery_at {
                *ssra.write() = Some(Instant::now());
            } else {
                drop(state);
                if let Some(mut state) = circuit_breaker_state.get_mut(upstream) {
                    let recovery_at = state
                        .slow_start_recovery_at
                        .get_or_insert_with(|| Arc::new(parking_lot::RwLock::new(None)));
                    *recovery_at.write() = Some(Instant::now());
                }
            }
        }
        crate::upstream::flapping::record_circuit_transition(
            flapping_state,
            circuit_breaker,
            &upstream.proxy_to,
            event_sink,
            event_trace_context.clone(),
        );
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: format!(
                    "Upstream {} circuit closed after {} successful half-open request(s)",
                    upstream.proxy_to, circuit_breaker.consecutive_passes
                ),
                summary: "Upstream circuit closed".into(),
                target: crate::LOG_TARGET,
                attributes: vec![(
                    "upstream.address",
                    ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                )],
                trace_context: event_trace_context.clone(),
            },
        ));
        emit_circuit_metric(
            event_sink,
            upstream,
            "ferron.proxy.circuit.state",
            ferron_observability::MetricType::Gauge,
            ferron_observability::MetricValue::U64(CIRCUIT_BREAKER_STATUS_CLOSED as u64),
            event_trace_context.clone(),
            metrics_resolved_ip,
        );
        emit_circuit_metric(
            event_sink,
            upstream,
            "ferron.proxy.circuit.open_total",
            ferron_observability::MetricType::Counter,
            ferron_observability::MetricValue::I64(-1),
            event_trace_context,
            metrics_resolved_ip,
        );
    }
}
