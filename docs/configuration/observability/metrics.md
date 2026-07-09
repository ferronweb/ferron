---
title: "Configuration: metrics"
description: "Process, admin API, and HTTP/proxy metrics emitted by Ferron."
---

This page documents the metrics emitted by Ferron. Ferron emits OpenTelemetry-style metrics through the observability event system. Each module documents its own metrics, such as:

- **Core HTTP server metrics** — active requests, request duration, and request count. See [HTTP host directives](/docs/v3/configuration/server/host#metrics).
- **Cache metrics** — cache hits, misses, entries, stores, evictions, and purges, with zone scoping. See [HTTP caching](/docs/v3/configuration/content/cache#metrics).
- **Rate limiting metrics** — allowed and rejected requests, with zone scoping. See [Rate limiting](/docs/v3/configuration/content/rate-limit#metrics).
- **Response control metrics** — aborted connections, IP blocks, and status rule matches. See [HTTP response control](/docs/v3/configuration/routing/response#metrics).
- **Static file metrics** — files served and bytes sent, with compression and cache hit attributes. See [Static file serving](/docs/v3/configuration/content/static-files#metrics).
- **Rewrite metrics** — applied rewrites and invalid rewrite errors. See [URL rewriting](/docs/v3/configuration/routing/rewrite#metrics).
- **Proxy metrics** — backend selection, health, connection pooling, and TLS failures. See [Reverse proxying](/docs/v3/configuration/proxy/reverse-proxy#metrics).
- **CGI/FastCGI/SCGI metrics** — request counts, failures, upstream duration, and stderr errors. See [CGI](/docs/v3/configuration/content/cgi#metrics), [FastCGI](/docs/v3/configuration/content/fastcgi#metrics), [SCGI](/docs/v3/configuration/content/scgi#metrics).

> [!info]
>
> - This page lists core metrics emitted by Ferron. For per-module metrics, see respective configuration pages.
> - For native Prometheus metrics export, see [Prometheus metrics](/docs/v3/configuration/observability/prometheus).
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).

## Understanding metric types

Ferron uses OpenTelemetry metric types. Understanding the type helps you write correct PromQL or OTLP queries:

| Type | Behavior | Typical use |
|------|----------|-------------|
| `Counter` | Monotonically increases. Resets to zero on process restart. | Request counts, error counts, reload counts |
| `Gauge` | Can go up or down. Represents an absolute value at time of measurement. | Active connections, queue length, uptime |
| `UpDownCounter` | Can go up or down. Represents a delta since the last measurement. | Memory usage, file descriptor count |

## Exponential histograms

When using the OTLP observability backend, all histogram metrics use **Base2 Exponential Histograms** instead of fixed linear buckets. This provides several advantages for latency-sensitive workloads:

- **Automatic bucket allocation** — bucket boundaries are computed automatically using a base-2 exponential formula, eliminating the need to manually tune bucket boundaries.
- **High resolution across orders of magnitude** — the default configuration uses 160 buckets covering the range from sub-millisecond to 100 seconds with less than 5% relative error, preserving tail-latency outliers (p99, p99.9) that would be masked by coarse fixed buckets.
- **No configuration required** — the exponential histogram aggregation is applied automatically at the SDK layer via an OTel View. No configuration changes are needed.

The Prometheus observability backend also supports native exponential histograms when `endpoint_native_histograms true` is configured and the output format is set to `"protobuf"`. In text format, it uses explicit bucket histograms with predefined boundaries.

## Process metrics

The `metrics-process` module collects process-level metrics automatically when an observability backend is configured. On Linux it reads `/proc/self/stat`; on Windows it uses the `GetProcessTimes` and `GetProcessMemoryInfo` APIs. On both platforms the collection interval is 1 second.

**Platform support:** Linux and Windows. On other platforms, the module is a no-op.

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `process.cpu.time` | Counter | `cpu.mode` (`"user"` or `"system"`) | Total CPU seconds broken down by different states |
| `process.cpu.utilization` | Gauge | `cpu.mode` (`"user"` or `"system"`) | CPU utilization since the last measurement |
| `process.memory.usage` | UpDownCounter | — | The change in physical memory (RSS) since the last measurement |
| `process.memory.virtual` | UpDownCounter | — | The change in committed virtual memory (VMS) since the last measurement |
| `process.unix.file_descriptor.count` | UpDownCounter | — | The change in number of unix file descriptors since the last measurement (Linux only) |

## Admin API metrics

The `metrics-admin` module collects metrics exposed via admin API automatically when an observability backend is configured.

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.admin.uptime` | Gauge | — | Time since the server started |
| `ferron.admin.connections_active` | Gauge | — | Currently open TCP connections across all HTTP listeners |
| `ferron.admin.requests_total` | Counter | — | Total HTTP requests served across all listeners |
| `ferron.admin.reloads` | Counter | — | Number of configuration reloads performed |
| `ferron.admin.observability_events_dropped` | Counter | — | Total number of observability events dropped due to backpressure |
| `ferron.admin.observability_event_queue_len` | Gauge | — | Approximate current length of the observability event queue |
| `ferron.admin.config_mtime` | Gauge | — | Last modification time of the configuration source (epoch seconds) |

### Admin API request metrics

The admin API emits per-request metrics for all endpoints except `/health` (which is a high-frequency probe with low signal per-request).

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.admin.request.duration` | Histogram | `http.request.method`, `url.path`, `http.response.status_code` | Duration of admin API requests in seconds |
| `ferron.admin.request.count` | Counter | `http.request.method`, `url.path`, `http.response.status_code` | Total number of admin API requests |
| `ferron.admin.reload.count` | Counter | `http.response.status_code` | Total number of admin config reload attempts (POST /reload only) |

### Admin API runtime metrics

The `metrics-admin` module also collects runtime status metrics when an observability backend is configured.

- `ferron.admin.runtime.primary_threads` (Gauge) — number of primary threads (typically equal to CPU count).
- `ferron.admin.runtime.io_uring_supported` (Gauge) — whether `io_uring` is supported on the current system (`1` = yes, `0` = no).
- `ferron.admin.runtime.io_uring_runtime_enabled` (Gauge) — whether `io_uring` was successfully enabled at runtime (`1` = yes, `0` = no).

These metrics correspond to the same data exposed by the admin API's `GET /status` endpoint, but are available as time-series data for monitoring and alerting.

## Reload metrics

The `metrics-reload` module emits a counter metric around configuration reload lifecycle events when an observability backend is configured.

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.reloads` | Counter | `ferron.reload.successful` (bool), `error.message` (string, on failure) | Number of configuration reloads performed, annotated with success or failure |

## HTTP and proxy metrics

Ferron also emits common request-path metrics:

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.http.server.pre_handler_request_count` | Counter | — | Malformed or timed-out requests rejected before the normal HTTP handler completes |
| `ferron.http.server.redirects` | Counter | — | Redirects emitted by the core server, including trailing-slash and HTTP-to-HTTPS redirects |
| `ferron.http.server.client_ip_rewrites` | Counter | — | Requests whose client IP was rewritten from a trusted proxy header |
| `ferron.http.server.cors_preflights` | Counter | — | CORS preflight requests handled before the main pipeline |
| `ferron.http.server.connection_errors` | Counter | transport, lifecycle stage | Listener and handshake failures |

## Prometheus scrape metrics

The Prometheus endpoint emits self-referential metrics about its own scrape performance. These are registered in the same registry that serves `/metrics`, so they appear in every scrape response.

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron_prometheus_scrape_duration_seconds` | Histogram | — | Duration of Prometheus scrape requests in seconds |
| `ferron_prometheus_scrape_total` | Counter | — | Total number of Prometheus scrape requests |
| `ferron_prometheus_scrape_errors_total` | Counter | — | Total number of failed Prometheus scrape requests |

> [!tip]
> Use `ferron_prometheus_scrape_duration_seconds` to monitor scrape latency. If p99 exceeds your Prometheus `scrape_timeout`, consider reducing the number of registered metrics or increasing the timeout.

## TLS metrics

The core server emits per-host TLS metrics for every HTTPS connection. Each metric carries a `ferron.host` attribute (the SNI hostname, or `"_global"` for connections without SNI) for per-host filtering and multi-tenant monitoring.

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.tls.certificate_not_after` | Gauge | `ferron.host`, `ferron.tls.provider`, `crypto.certificate.serial_number` | Certificate `notAfter` field as Unix epoch seconds |
| `ferron.tls.handshake.duration` | Histogram | `ferron.host`, `tls.protocol.version`, `tls.cipher_suite`, `ferron.tls.handshake.result` | TLS handshake latency |
| `ferron.tls.handshake.total` | Counter | `ferron.host`, `ferron.tls.handshake.result` | Total TLS handshake attempts |
| `ferron.tls.connections.active` | UpDownCounter | `ferron.host` | Currently open TLS connections |

## Common alerting patterns

The table below maps core metrics to practical alerting thresholds. Adjust thresholds based on your deployment size and traffic patterns.

| Alert | Metric | Suggested threshold |
|-------|--------|---------------------|
| High error rate | `ferron.http.server.connection_errors` | Rate > 10/s for 1m |
| Connection saturation | `ferron.admin.connections_active` | Approaching configured limit |
| Config reload failures | `ferron.admin.reloads` | Count > 0 with error attribute |
| Export backpressure | `ferron.admin.observability_events_dropped` | Rate > 0 |
| Queue buildup | `ferron.admin.observability_event_queue_len` | > 1000 |
| Memory growth | `process.memory.usage` | Sustained increase over hours |
| CPU saturation | `process.cpu.utilization` | > 0.9 sustained |
| TLS handshake failures | `ferron.tls.handshake.total{result="error"}` | Rate > 5/s for 1m |
| Slow TLS handshakes | `ferron.tls.handshake.duration` p99 | > 500ms |

## Cardinality guidance

Observability sinks may drop events under high load. Ferron exposes `ferron.admin.observability_events_dropped` and `ferron.admin.observability_event_queue_len` to help detect and tune exporter queues.

When using Prometheus or OTLP with baggage promotion, monitor label cardinality. High-cardinality labels (such as user IDs or request IDs used as metric labels) can cause significant performance degradation and memory consumption in Prometheus. Use `max_distinct` on baggage keys to cap distinct label values and prevent label explosion. Values exceeding the distinct cap are automatically hashed to a deterministic string.
