---
title: "Configuration: metrics"
description: "Process, admin API, HTTP, and TLS metrics emitted by Ferron."
---

Ferron emits OpenTelemetry-style metrics through the observability event system. Each module documents its own metrics:

| Area              | Metrics                                                              | Page                                                                                                              |
| ----------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Core HTTP server  | Active requests, request duration, request count                     | [HTTP host directives](/docs/configuration/server/host#metrics)                                                   |
| Cache             | Hits, misses, entries, stores, evictions, and purges, by zone        | [HTTP caching](/docs/configuration/content/cache#metrics)                                                         |
| Rate limiting     | Allowed and rejected requests, by zone                               | [Rate limiting](/docs/configuration/content/rate-limit#metrics)                                                   |
| Response control  | Aborted connections, IP blocks, status rule matches                  | [HTTP response control](/docs/configuration/routing/response#metrics)                                             |
| Static files      | Files served and bytes sent, with compression and cache attributes   | [Static file serving](/docs/configuration/content/static-files#metrics)                                           |
| URL rewriting     | Applied rewrites and invalid rewrite errors                          | [URL rewriting](/docs/configuration/routing/rewrite#metrics)                                                      |
| Reverse proxy     | Backend selection, health, connection pooling, TLS failures          | [Reverse proxying](/docs/configuration/proxy/reverse-proxy#metrics)                                               |
| CGI, FastCGI, SCGI | Request counts, failures, upstream duration, stderr errors          | [CGI](/docs/configuration/content/cgi#metrics), [FastCGI](/docs/configuration/content/fastcgi#metrics), [SCGI](/docs/configuration/content/scgi#metrics) |

For export setup, see [Prometheus metrics](/docs/configuration/observability/prometheus), [OTLP observability](/docs/configuration/observability/otlp), and [StatsD metrics](/docs/configuration/observability/statsd).

## Metric types

Ferron uses OpenTelemetry metric types. The type determines the correct PromQL or OTLP query:

| Type            | Behavior                                                   | Typical use                                 |
| --------------- | ---------------------------------------------------------- | ------------------------------------------- |
| `Counter`       | Monotonically increases. Resets to zero on process restart. | Request counts, error counts, reload counts |
| `Gauge`         | Goes up or down. Absolute value at measurement time.        | Active connections, queue length, uptime    |
| `UpDownCounter` | Goes up or down. Delta since the last measurement.          | Memory usage, file descriptor count         |

## Exponential histograms

The OTLP backend uses Base2 exponential histograms for all histogram metrics instead of fixed linear buckets.

- Bucket boundaries come from a base-2 exponential formula. You do not tune them.
- The default layout uses 160 buckets from sub-millisecond to 100 seconds, with less than 5% relative error. Tail percentiles (p99, p99.9) keep their resolution.
- The SDK applies the aggregation through an OpenTelemetry View. No configuration is required.

> [!tip]
> If histogram metrics are missing in your backend, disable native exponential histograms with `native_histograms false` in the `metrics` sub-block. See [OTLP observability](/docs/configuration/observability/otlp#metrics-collection-tuning).

The Prometheus backend also supports native exponential histograms with `endpoint_native_histograms` and `endpoint_format "protobuf"`. In text format, it uses explicit bucket histograms with predefined boundaries.

## Metric exemplars

An exemplar connects a metric observation to the trace that produced it. Ferron adds exemplars in the Prometheus and OTLP backends when the request carries a W3C trace ID and span ID.

Each exemplar contains:

- `trace_id`: the W3C trace ID of the request
- `span_id`: the W3C span ID of the request

Counters carry exemplars by default. Prometheus histograms carry exemplars only when `endpoint_native_histograms` is `false` (the default). Native histograms and exemplars are mutually exclusive in this module, so enabling native histograms removes exemplars from histogram metrics.

> [!note]
> Exemplars require a tracing backend, for example OTLP, in the server configuration.

For configuration details, see [Prometheus metrics](/docs/configuration/observability/prometheus#metric-exemplars) and [OTLP observability](/docs/configuration/observability/otlp#metrics-collection-tuning).

## Process metrics

The `metrics-process` module collects process metrics when you configure an observability backend. On Linux it reads `/proc/self/stat`. On Windows it uses the `GetProcessTimes` and `GetProcessMemoryInfo` APIs. The collection interval is 1 second on both platforms.

The module supports Linux and Windows. On other platforms it does nothing.

| Metric                              | Type          | Attributes                          | Description                                                       |
| ----------------------------------- | ------------- | ----------------------------------- | ----------------------------------------------------------------- |
| `process.cpu.time`                  | Counter       | `cpu.mode` (`"user"` or `"system"`) | Total CPU seconds by mode                                         |
| `process.cpu.utilization`           | Gauge         | `cpu.mode` (`"user"` or `"system"`) | CPU use since the last measurement                                |
| `process.memory.usage`              | UpDownCounter | None                                | Change in physical memory (RSS) since the last measurement        |
| `process.memory.virtual`            | UpDownCounter | None                                | Change in committed virtual memory (VMS) since the last measurement |
| `process.unix.file_descriptor.count` | UpDownCounter | None                               | Change in open Unix file descriptors since the last measurement (Linux only) |

## Admin API metrics

The `metrics-admin` module collects these metrics when you configure an observability backend.

| Metric                                       | Type    | Attributes | Description                                                    |
| -------------------------------------------- | ------- | ---------- | -------------------------------------------------------------- |
| `ferron.admin.uptime`                        | Gauge   | None       | Time since the server started                                  |
| `ferron.admin.connections_active`            | Gauge   | None       | Open TCP connections across all HTTP listeners                 |
| `ferron.admin.requests_total`                | Counter | None       | HTTP requests served across all listeners                      |
| `ferron.admin.reloads`                       | Counter | None       | Configuration reloads performed                                |
| `ferron.admin.observability_events_dropped`  | Counter | None       | Observability events dropped due to backpressure               |
| `ferron.admin.observability_event_queue_len` | Gauge   | None       | Approximate current length of the observability event queue    |
| `ferron.admin.config_mtime`                  | Gauge   | None       | Last modification time of the configuration source, in epoch seconds |
| `ferron.admin.config_drift`                  | Gauge   | None       | Configuration drift detected: `1` is drift, `0` is no drift    |

### Admin API request metrics

The admin API emits per-request metrics for all endpoints except `/health`. `/health` is a high-frequency probe with low signal.

| Metric                          | Type      | Attributes                                                     | Description                                     |
| ------------------------------- | --------- | -------------------------------------------------------------- | ----------------------------------------------- |
| `ferron.admin.request.duration` | Histogram | `http.request.method`, `url.path`, `http.response.status_code` | Admin API request duration in seconds           |
| `ferron.admin.request.count`    | Counter   | `http.request.method`, `url.path`, `http.response.status_code` | Total admin API requests                        |
| `ferron.admin.reload.count`     | Counter   | `http.response.status_code`                                    | Config reload attempts (`POST /reload` only)    |

### Admin API runtime metrics

The module also collects runtime status metrics:

- `ferron.admin.runtime.primary_threads` (Gauge): number of primary threads, usually the CPU count.
- `ferron.admin.runtime.io_uring_supported` (Gauge): whether the system supports `io_uring` (`1` is yes, `0` is no).
- `ferron.admin.runtime.io_uring_runtime_enabled` (Gauge): whether `io_uring` is enabled at runtime (`1` is yes, `0` is no).

These metrics expose the same data as the admin API `GET /status` endpoint.

## Reload metrics

The `metrics-reload` module emits a counter around configuration reload lifecycle events.

| Metric           | Type    | Attributes                                                              | Description                                                   |
| ---------------- | ------- | ----------------------------------------------------------------------- | ------------------------------------------------------------- |
| `ferron.reloads` | Counter | `ferron.reload.successful` (bool), `error.message` (string, on failure) | Configuration reloads, annotated with success or failure      |

## HTTP metrics

Ferron also emits these request-path metrics:

| Metric                                         | Type    | Attributes                                        | Description                                                                   |
| ---------------------------------------------- | ------- | ------------------------------------------------- | ----------------------------------------------------------------------------- |
| `ferron.http.server.pre_handler_request_count` | Counter | None                                              | Malformed or timed-out requests rejected before the HTTP handler completes    |
| `ferron.http.server.redirects`                 | Counter | None                                              | Core server redirects, including trailing-slash and HTTP-to-HTTPS redirects   |
| `ferron.http.server.client_ip_rewrites`        | Counter | None                                              | Requests whose client IP was rewritten from a trusted proxy header            |
| `ferron.http.server.cors_preflights`           | Counter | None                                              | CORS preflight requests handled before the main pipeline                      |
| `ferron.http.server.connection_errors`         | Counter | `network.transport`, `ferron.connection.stage`    | Listener and handshake failures                                               |

## Prometheus scrape metrics

The Prometheus endpoint registers self-referential metrics in the same registry that serves `/metrics`. They appear in every scrape response.

| Metric                                      | Type      | Attributes | Description                        |
| ------------------------------------------- | --------- | ---------- | ---------------------------------- |
| `ferron_prometheus_scrape_duration_seconds` | Histogram | None       | Prometheus scrape request duration |
| `ferron_prometheus_scrape_total`            | Counter   | None       | Total Prometheus scrape requests   |
| `ferron_prometheus_scrape_errors_total`     | Counter   | None       | Failed Prometheus scrape requests  |

> [!tip]
> Use `ferron_prometheus_scrape_duration_seconds` to monitor scrape latency. If p99 exceeds your Prometheus `scrape_timeout`, reduce the number of registered metrics or increase the timeout.

## TLS metrics

The core server emits per-host TLS metrics for every HTTPS connection. Each metric carries `ferron.host`: the SNI hostname, or `"_global"` when the connection has no SNI.

| Metric                             | Type          | Attributes                                                                               | Description                       |
| ---------------------------------- | ------------- | ---------------------------------------------------------------------------------------- | --------------------------------- |
| `ferron.tls.certificate_not_after` | Gauge         | `ferron.host`, `ferron.tls.provider`, `crypto.certificate.serial_number`                 | Certificate `notAfter` in epoch seconds |
| `ferron.tls.handshake.duration`    | Histogram     | `ferron.host`, `tls.protocol.version`, `tls.cipher_suite`, `ferron.tls.handshake.result` | TLS handshake latency             |
| `ferron.tls.handshake.total`       | Counter       | `ferron.host`, `ferron.tls.handshake.result`                                             | TLS handshake attempts            |
| `ferron.tls.connections.active`    | UpDownCounter | `ferron.host`                                                                            | Open TLS connections              |

## Alerting patterns

Suggested thresholds for core metrics. Adjust them for your deployment size and traffic.

| Alert                  | Metric                                                 | Suggested threshold            |
| ---------------------- | ------------------------------------------------------ | ------------------------------ |
| High error rate        | `ferron.http.server.connection_errors`                 | Rate > 10/s for 1m             |
| Connection saturation  | `ferron.admin.connections_active`                      | Approaching configured limit   |
| Config reload failures | `ferron.admin.reloads`                                 | Count > 0 with error attribute |
| Config drift detected  | `ferron.admin.config_drift`                            | Value = 1 for > 1m             |
| Export backpressure    | `ferron.admin.observability_events_dropped`            | Rate > 0                       |
| Queue buildup          | `ferron.admin.observability_event_queue_len`           | > 1000                         |
| Memory growth          | `process.memory.usage`                                 | Sustained increase over hours  |
| CPU saturation         | `process.cpu.utilization`                              | > 0.9 sustained                |
| TLS handshake failures | `ferron.tls.handshake.total{ferron.tls.handshake.result="error"}` | Rate > 5/s for 1m   |
| Slow TLS handshakes    | `ferron.tls.handshake.duration` p99                    | > 500ms                        |

## Cardinality guidance

Observability sinks can drop events under load. Use `ferron.admin.observability_events_dropped` and `ferron.admin.observability_event_queue_len` to detect drops and tune exporter queues.

With Prometheus or OTLP baggage promotion, monitor label cardinality. High-cardinality labels, such as user IDs or request IDs, degrade Prometheus performance and increase memory use. Set `max_distinct` on baggage keys to cap distinct label values. Values above the cap are dropped.
