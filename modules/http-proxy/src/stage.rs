use std::borrow::Cow;
use std::sync::Arc;

use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::HttpContext;
use ferron_observability::TraceAttributeValue;
use parking_lot::RwLock;

use crate::types::circuit::circuit_breaker_state_label;
use crate::types::retry_budget::SharedRetryBudget;
use crate::upstream::lb::p2c_ewma::{self, P2cEwmaParams};
use crate::upstream::lb::{ConsistentHashRing, LoadBalancerAlgorithmInner};
use crate::ProxyState;

pub struct ReverseProxyStage {
    pub state: Arc<ProxyState>,
}

#[async_trait::async_trait(?Send)]
impl ferron_core::pipeline::Stage<HttpContext> for ReverseProxyStage {
    #[inline]
    fn name(&self) -> &str {
        "reverse_proxy"
    }

    #[inline]
    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("proxy"))
    }

    #[inline]
    async fn run(
        &self,
        ctx: &mut HttpContext,
    ) -> Result<bool, ferron_core::pipeline::PipelineError> {
        let entries = ctx.configuration.get_entries("proxy", true);
        if entries.is_empty() {
            return Ok(true);
        }

        // Use the layer Arc pointer identities as a cache key.
        // When config is reloaded, new Arc pointers are created.
        let config_key = ctx
            .configuration
            .layers
            .iter()
            .filter_map(|arc| {
                if arc.has_directive("proxy") {
                    Some(Arc::as_ptr(arc) as usize)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let config = match crate::config::parse_proxy_config(ctx) {
            Ok(Some(cfg)) => Arc::new(cfg),
            Ok(None) => return Ok(true),
            Err(e) => {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-proxy",
                        level: ferron_observability::LogLevel::Error,
                        message: format!("Proxy config error: {e}"),
                        summary: "Reverse proxy config error".into(),
                        attributes: vec![(
                            "error.message",
                            ferron_observability::LogAttributeValue::String(e.to_string()),
                        )],
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                return Ok(true);
            }
        };

        // Spawn health check task for this config if needed
        self.state
            .ensure_health_check_task(&config_key, &config.upstreams);

        self.state.metrics_resolved_ip.store(
            config.metrics_resolved_ip,
            std::sync::atomic::Ordering::Relaxed,
        );

        let (algorithm, ring) = if let Some(algo) = self.state.algorithms.load().get(&config_key) {
            algo.clone()
        } else {
            self.state
                .algorithms
                .load()
                .entry(config_key.clone())
                .or_insert_with(|| {
                    (
                        Arc::new(config.algorithm.into()),
                        // Blank upstream list for now
                        Arc::new(RwLock::new(ConsistentHashRing::new(&[]))),
                    )
                })
                .clone()
        };

        let active_unhealthy_counter = self.state.active_unhealthy_counters.get(&config_key);

        // Capture HTTP method before execute_proxy() consumes ctx.req
        let captured_method = ctx
            .req
            .as_ref()
            .map(|r| crate::proxy::categorize_http_method(r.method()));
        let captured_idempotent = ctx
            .req
            .as_ref()
            .map(|r| crate::proxy::is_method_idempotent(r.method()));

        let retry_budget = config.retry_budget.as_ref().map(|budget_config| {
            self.state
                .retry_budget_states
                .get_or_insert_with(&config_key, || {
                    SharedRetryBudget::new(
                        budget_config.max_tokens,
                        budget_config.refill_rate,
                        budget_config.max_retry_rate,
                    )
                })
        });

        let upstreams = crate::upstream::resolve_upstreams(&config.upstreams).await;

        // Set or update per-upstream local limits.
        let conn_manager = self.state.get_conn_manager();
        for upstream in &upstreams {
            if let Some(limit) = upstream.limit {
                conn_manager.set_local_limit(upstream.clone(), limit);
            }
        }

        let result = crate::proxy::execute_proxy(
            ctx,
            &config,
            &conn_manager,
            Arc::clone(&self.state.circuit_breaker_state),
            Arc::clone(&self.state.flapping_state),
            &algorithm,
            &ring,
            Some(&self.state.conn_state),
            Some(&self.state.ewma_state),
            Some(&self.state.active_health_check_state),
            active_unhealthy_counter.as_deref(),
            upstreams,
            retry_budget.as_ref(),
        )
        .await;

        let (response, mut metrics) = match result {
            Ok((resp, m)) => (resp, m),
            Err(e) => {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-proxy",
                        level: ferron_observability::LogLevel::Error,
                        message: format!("Proxy error: {}", e),
                        summary: e.summary().into(),
                        attributes: vec![
                            (
                                "error.type",
                                ferron_observability::LogAttributeValue::String(
                                    e.error_type().to_string(),
                                ),
                            ),
                            (
                                "error.message",
                                ferron_observability::LogAttributeValue::String(e.to_string()),
                            ),
                        ],
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                let status_code = e.http_status_hint().map_or(502, |sh| sh.as_u16());
                crate::metrics::emit_proxy_failure_metric(
                    ctx,
                    status_code,
                    e.error_type(),
                    current_event_trace_context(ctx),
                );
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(status_code, None));
                ctx.get_span_attributes().insert(
                    "http.response.status_code",
                    TraceAttributeValue::I64(status_code as i64),
                );
                ctx.get_span_attributes().insert(
                    "error.type",
                    TraceAttributeValue::String(e.error_type().to_string()),
                );
                return Ok(false);
            }
        };

        // Attach captured method metadata to metrics
        metrics.request_method = captured_method;
        metrics.method_idempotent = captured_idempotent;

        // Capture retry budget tokens after request completion
        if let Some(ref budget) = retry_budget {
            metrics.retry_budget_tokens = Some(budget.available_tokens());
        }

        ctx.res = Some(response);

        // Inject backend identity into access log fields
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            let log_fields = custom_access_log_fields(ctx);
            log_fields.insert(
                "ferron.proxy.backend_url".into(),
                CustomAccessLogField::String(backend.proxy_to.clone()),
            );
            if let Some(ref resolved_ip) = backend.connect_to {
                log_fields.insert(
                    "ferron.proxy.backend_resolved_ip".into(),
                    CustomAccessLogField::String(resolved_ip.to_string()),
                );
            }
            log_fields.insert(
                "ferron.proxy.dns_status".into(),
                CustomAccessLogField::String(backend.dns_status.as_label().to_string()),
            );
            if let Some(ref unix_path) = backend.proxy_unix {
                log_fields.insert(
                    "ferron.proxy.backend_unix_path".into(),
                    CustomAccessLogField::String(unix_path.clone()),
                );
            }
            log_fields.insert(
                "ferron.proxy.connection_reused".into(),
                CustomAccessLogField::Bool(metrics.connection_reused),
            );
            log_fields.insert(
                "ferron.proxy.retry_count".into(),
                CustomAccessLogField::U64(metrics.retry_count),
            );
            log_fields.insert(
                "ferron.proxy.same_upstream_retry_count".into(),
                CustomAccessLogField::U64(metrics.same_upstream_retry_count),
            );
            log_fields.insert(
                "ferron.proxy.retry_budget_exhausted".into(),
                CustomAccessLogField::Bool(metrics.retry_budget_exhausted),
            );
            // Inject circuit breaker state for the selected backend
            if let Some(cb_state) = self.state.circuit_breaker_state.get(backend) {
                let status = cb_state.status.load(std::sync::atomic::Ordering::Relaxed);
                log_fields.insert(
                    "ferron.proxy.circuit_breaker_state".into(),
                    CustomAccessLogField::String(circuit_breaker_state_label(status).to_string()),
                );
            }
        }

        let metrics_resolved_ip = self
            .state
            .metrics_resolved_ip
            .load(std::sync::atomic::Ordering::Relaxed);

        use ferron_observability::{MetricAttributeValue, MetricEvent, MetricType, MetricValue};

        for backend in &metrics.selected_backends {
            let mut attrs = Vec::with_capacity(4);
            attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            attrs.extend(crate::metrics::resolved_ip_attrs(
                metrics_resolved_ip,
                backend,
            ));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.selected",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of times a backend server was selected."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        for backend in &metrics.circuit_breaker_unhealthy_backends {
            let mut attrs = Vec::with_capacity(5);
            attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            attrs.extend(crate::metrics::resolved_ip_attrs(
                metrics_resolved_ip,
                backend,
            ));
            attrs.push((
                "ferron.proxy.health_check_type",
                MetricAttributeValue::String("circuit_breaker".to_string()),
            ));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        for (backend_url, count) in &metrics.active_unhealthy_backends {
            let attrs = vec![
                (
                    "ferron.proxy.backend_url",
                    MetricAttributeValue::String(backend_url.clone()),
                ),
                (
                    "ferron.proxy.health_check_type",
                    MetricAttributeValue::String("active".to_string()),
                ),
            ];
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(*count),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        let mut upstream_attrs = vec![];
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            upstream_attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                upstream_attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            upstream_attrs.extend(crate::metrics::resolved_ip_attrs(
                metrics_resolved_ip,
                backend,
            ));
        }

        // Emit per-request circuit breaker state gauge for the selected backend
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            if let Some(cb_state) = self.state.circuit_breaker_state.get(backend) {
                let status = cb_state.status.load(std::sync::atomic::Ordering::Relaxed);
                ctx.events
                    .emit(ferron_observability::Event::Metric(MetricEvent {
                        name: "ferron.proxy.circuit.state",
                        attributes: upstream_attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::U64(status as u64),
                        unit: Some("{circuit}"),
                        description: Some("Current circuit breaker state per backend (0=closed, 1=open, 2=half_open)."),
                        trace_context: current_event_trace_context(ctx),

                    }));
            }
            if let Some(flapping) = self.state.flapping_state.get(&backend.proxy_to) {
                let is_flapping = flapping.is_flapping();
                ctx.events
                    .emit(ferron_observability::Event::Metric(MetricEvent {
                        name: "ferron.proxy.circuit.flapping",
                        attributes: upstream_attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::U64(is_flapping as u64),
                        unit: Some("{circuit}"),
                        description: Some(
                            "Whether an upstream backend is flapping (1 = flapping, 0 = stable).",
                        ),
                        trace_context: current_event_trace_context(ctx),
                    }));
            }
        }

        // Emit request counter with connection reuse flag and status code
        let mut request_attrs = Vec::with_capacity(4);
        request_attrs.extend(upstream_attrs.clone());
        request_attrs.push((
            "ferron.proxy.connection_reused",
            MetricAttributeValue::Bool(metrics.connection_reused),
        ));
        if let Some(status) = metrics.status_code {
            request_attrs.push((
                "http.response.status_code",
                MetricAttributeValue::I64(status as i64),
            ));
        }
        ctx.events
            .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.requests",
                attributes: request_attrs,
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Number of reverse proxy requests."),
                trace_context: current_event_trace_context(ctx),
            }));

        let selected_backends_len = metrics.selected_backends.len();
        if selected_backends_len > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.selected_per_request",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(selected_backends_len as u64),
                    unit: Some("{backend}"),
                    description: Some(
                        "Number of backends selected for a request (including retries).",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.tls_handshake_failures > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.tls_handshake_failures",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.tls_handshake_failures),
                    unit: Some("{handshake}"),
                    description: Some("TLS handshake failures with upstream backends."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.pool_waits > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.waits",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.pool_waits),
                    unit: Some("{wait}"),
                    description: Some(
                        "Times the connection pool was exhausted and a request had to wait.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.pool_wait_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.wait_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(
                        crate::metrics::PROXY_POOL_BUCKETS,
                    ))),
                    value: MetricValue::F64(metrics.pool_wait_time_secs),
                    unit: Some("s"),
                    description: Some("Duration spent waiting for a pooled connection."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.upstream_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.upstream.duration",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(
                        crate::metrics::PROXY_POOL_BUCKETS,
                    ))),
                    value: MetricValue::F64(metrics.upstream_time_secs),
                    unit: Some("s"),
                    description: Some("Duration of upstream request-response."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.tls_handshake_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.tls.handshake_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(
                        crate::metrics::PROXY_TLS_BUCKETS,
                    ))),
                    value: MetricValue::F64(metrics.tls_handshake_time_secs),
                    unit: Some("s"),
                    description: Some("TLS handshake duration for upstream connection."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        for backend in &metrics.excluded_circuit_open {
            crate::metrics::emit_backend_excluded(
                &ctx.events,
                backend,
                "circuit_open",
                current_event_trace_context(ctx),
                metrics_resolved_ip,
            );
        }
        for backend in &metrics.excluded_already_tried {
            crate::metrics::emit_backend_excluded(
                &ctx.events,
                backend,
                "already_tried",
                current_event_trace_context(ctx),
                metrics_resolved_ip,
            );
        }
        for backend in &metrics.excluded_overloaded {
            crate::metrics::emit_backend_excluded(
                &ctx.events,
                backend,
                "overloaded",
                current_event_trace_context(ctx),
                metrics_resolved_ip,
            );
        }

        if metrics.retry_count > 0 {
            let mut retry_attrs = upstream_attrs.clone();
            if let Some(method) = metrics.request_method {
                retry_attrs.push((
                    "http.request.method",
                    MetricAttributeValue::StaticStr(method),
                ));
            }
            if let Some(idempotent) = metrics.method_idempotent {
                retry_attrs.push((
                    "ferron.proxy.method_idempotent",
                    MetricAttributeValue::Bool(idempotent),
                ));
            }
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.retry.count",
                    attributes: retry_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.retry_count),
                    unit: Some("{attempt}"),
                    description: Some("Number of retry attempts during backend selection."),

                    trace_context: current_event_trace_context(ctx),
                }));
            retry_attrs.push(("ferron.proxy.retry.final", MetricAttributeValue::Bool(true)));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.retry.final",
                attributes: retry_attrs,
                ty: MetricType::Gauge,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some(
                    "Indicates the request succeeded after a retry (1) or required no retries (0).",
                ),
                trace_context: current_event_trace_context(ctx),
            }));
        }

        if metrics.same_upstream_retry_count > 0 {
            let mut same_attrs = upstream_attrs.clone();
            if let Some(method) = metrics.request_method {
                same_attrs.push((
                    "http.request.method",
                    MetricAttributeValue::StaticStr(method),
                ));
            }
            if let Some(idempotent) = metrics.method_idempotent {
                same_attrs.push((
                    "ferron.proxy.method_idempotent",
                    MetricAttributeValue::Bool(idempotent),
                ));
            }
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.same_upstream_retry.count",
                    attributes: same_attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.same_upstream_retry_count),
                    unit: Some("{attempt}"),
                    description: Some("Number of same-upstream retry attempts for a request."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.retry_budget_exhausted {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.retry.budget_exhausted",
                attributes: upstream_attrs.clone(),
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some(
                    "Number of requests where retry was refused due to retry budget exhaustion.",
                ),
                trace_context: current_event_trace_context(ctx),
            }));
        }
        if let Some(tokens) = metrics.retry_budget_tokens {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.retry.budget_tokens_available",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::F64(tokens),
                    unit: Some("{token}"),
                    description: Some("Current available retry budget tokens."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.pool_hit {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.hit",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("A pooled connection was available immediately."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }
        if metrics.pool_miss {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.miss",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "No pooled connection was available; a new connection was established.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.upstream_response_truncated {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.upstream.response_truncated",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{response}"),
                    description: Some(
                        "Upstream responses that ended before the declared Content-Length.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if metrics.connect_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.connect.latency",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(
                        crate::metrics::PROXY_POOL_BUCKETS,
                    ))),
                    value: MetricValue::F64(metrics.connect_time_secs),
                    unit: Some("s"),
                    description: Some(
                        "Duration of TCP/TLS connection establishment to the upstream.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }
        if metrics.ttfb_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.ttfb",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(
                        crate::metrics::PROXY_POOL_BUCKETS,
                    ))),
                    value: MetricValue::F64(metrics.ttfb_secs),
                    unit: Some("s"),
                    description: Some(
                        "Time from request send to first byte of response headers received.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            // Backend active connections gauge
            let active_conns = self
                .state
                .conn_state
                .get(backend)
                .map_or(0, |e| std::sync::Arc::strong_count(e.value()) - 1);
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.active_connections",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(active_conns as u64),
                    unit: Some("{connection}"),
                    description: Some("Active tracked connections for the selected backend."),
                    trace_context: current_event_trace_context(ctx),
                }));

            // Emit P2C+EWMA adaptive load balancing diagnostics for the selected backend
            if matches!(&*algorithm, LoadBalancerAlgorithmInner::P2cEwma) {
                let params = P2cEwmaParams::default();

                // Backend EWMA latency gauge
                let ewma_latency =
                    p2c_ewma::get_decayed_ewma(&self.state.ewma_state, backend, &params);
                ctx.events
                    .emit(ferron_observability::Event::Metric(MetricEvent {
                        name: "ferron.proxy.lb.ewma_latency",
                        attributes: upstream_attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::F64(ewma_latency),
                        unit: Some("s"),
                        description: Some(
                            "Current EWMA response latency for the selected backend.",
                        ),
                        trace_context: current_event_trace_context(ctx),
                    }));

                // Backend warm-up state gauge
                let warming_up = p2c_ewma::is_warming_up(&self.state.ewma_state, backend);
                ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.warmup_state",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(if warming_up { 1 } else { 0 }),
                    unit: Some("{state}"),
                    description: Some("Whether the selected backend is still in EWMA warm-up phase (1 = warming up, 0 = settled)."),
                    trace_context: current_event_trace_context(ctx),

                }));
            }

            if !metrics.candidate_scores.is_empty() {
                ctx.events
                    .emit(ferron_observability::Event::Metric(MetricEvent {
                        name: "ferron.proxy.lb.score",
                        attributes: upstream_attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::F64(metrics.candidate_scores[0]),
                        unit: Some("{score}"),
                        description: Some(
                            "Combined load-balancer selection score for the selected backend. Lower is more preferred.",
                        ),
                        trace_context: current_event_trace_context(ctx),

                    }));
            }
        }

        let sa = ctx.get_span_attributes();
        if let Some(status) = metrics.status_code {
            sa.insert(
                "http.response.status_code",
                TraceAttributeValue::I64(status as i64),
            );
        }
        sa.insert(
            "ferron.proxy.connection_reused",
            TraceAttributeValue::Bool(metrics.connection_reused),
        );
        sa.insert(
            "ferron.proxy.retry_count",
            TraceAttributeValue::I64(metrics.retry_count as i64),
        );
        sa.insert(
            "ferron.proxy.same_upstream_retry_count",
            TraceAttributeValue::I64(metrics.same_upstream_retry_count as i64),
        );
        if metrics.retry_budget_exhausted {
            sa.insert(
                "ferron.proxy.retry_budget_exhausted",
                TraceAttributeValue::Bool(true),
            );
        }
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            sa.insert(
                "ferron.proxy.backend_url",
                TraceAttributeValue::String(backend.proxy_to.clone()),
            );
            if let Some(ref unix_path) = backend.proxy_unix {
                sa.insert(
                    "ferron.proxy.backend_unix_path",
                    TraceAttributeValue::String(unix_path.clone()),
                );
            }
        }

        // Inject upstream runtime state into the request span for OTLP traces
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            crate::metrics::inject_upstream_state_span_attributes(
                ctx,
                backend,
                &self.state.circuit_breaker_state,
                &self.state.flapping_state,
                &self.state.active_health_check_state,
                &self.state.conn_state,
                config.circuit_breaker.slow_start_duration,
            );
        }

        Ok(false)
    }

    #[inline]
    async fn run_inverse(
        &self,
        _ctx: &mut HttpContext,
    ) -> Result<(), ferron_core::pipeline::PipelineError> {
        Ok(())
    }
}
