---
title: "Configuration: observability and logging"
description: "Access logging, log formatters, metrics, tracing, and OTLP export configuration."
---

This page documents the observability configuration surface for Ferron, including access logs, log formatters, metrics, tracing, and OTLP export.

## Directives

### Access logging

Access logs are configured via `log` blocks inside host or global scopes:

```ferron
example.com {
    log "access" {
        format json
        fields "method" "path" "status" "duration_secs"
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `format` | `<string>` | Log formatter to use. Available formatters depend on which observability modules are loaded. | none |
| `fields` | `<string>...` | Field names to include in the log output. When omitted, all available fields are emitted. | all fields |

#### Access log fields

Each access log entry contains the following fields:

| Field | Description |
| --- | --- |
| `path` | The request URI path (e.g. `/index.html`) |
| `path_and_query` | The request URI with path and query |
| `method` | The HTTP request method (e.g. `GET`, `POST`) |
| `version` | The HTTP version (e.g. `HTTP/1.1`, `HTTP/2.0`) |
| `scheme` | The request scheme (`http` or `https`) |
| `client_ip` | The client IP address |
| `client_port` | The client port number |
| `client_ip_canonical` | The client IP in canonical form |
| `server_ip` | The server IP address |
| `server_port` | The server port number |
| `server_ip_canonical` | The server IP in canonical form |
| `auth_user` | The authenticated username, or `-` if not authenticated |
| `status` | The HTTP response status code |
| `content_length` | The response content length, or `-` if not available |
| `duration_secs` | Request processing duration in seconds |
| `timestamp` | Request timestamp in CLF format |
| `header_<name>` | Request header values (one field per header) |
| `span_id` | Optional trace span ID for the request (if W3C trace context is available) |
| `trace_id` | Optional trace ID for the request (if W3C trace context is available) |

### Log formatters

#### `json`

The JSON formatter serializes each access log entry as a single-line JSON object. Provided by the `logformat-json` module.

```ferron
example.com {
    log "access" {
        format json
    }
}
```

Example output:

```json
{"method":"GET","path":"/index.html","status":200,"duration_secs":0.012,"client_ip":"127.0.0.1","remote_ip":"127.0.0.1"}
```

Use the `fields` directive to limit which fields appear in the JSON output. If `fields` is not specified, all available access log fields are emitted.

#### `text`

The text formatter generates each access log entry as a plain text string using a configurable pattern. Provided by the `logformat-text` module.

By default, it uses the **Combined Log Format (CLF)**, the same format used by Apache and Nginx.

**Configuration example:**

```ferron
example.com {
    log "access" {
        format text
    }
}
```

Example output:

```text
127.0.0.1 - frank [05/Apr/2026:14:32:01 +0200] "GET /index.html HTTP/1.1" 200 1234 "http://www.example.com/start.html" "Mozilla/5.0"
```

#### Pattern syntax

The `access_pattern` directive supports the following tokens:

| Token             | Description                                        | Example                            |
| ----------------- | -------------------------------------------------- | ---------------------------------- |
| `%field_name`     | Access log field                                   | `%client_ip`, `%status`, `%method` |
| `%{Header-Name}i` | Request header                                     | `%{Referer}i`, `%{User-Agent}i`    |
| `%{format}t`      | Timestamp with custom format                       | `%{%Y-%m-%d %H:%M:%S}t`            |
| `%t`              | Timestamp (uses `timestamp_format` or CLF default) | `%t`                               |
| `%%`              | Literal `%` character                              | `%%`                               |
| Other text        | Passed through literally                           | `"`, ``, `-`                       |

Request headers are available via the `%{Header-Name}i` syntax. The header name is case-insensitive and hyphens are converted to underscores internally.

### Metrics

Ferron emits OpenTelemetry-style metrics through the observability event system. Each module documents its own metrics:

- **Core HTTP server metrics** — active requests, request duration, and request count. See [HTTP host directives](/docs/v3/configuration/http-host#metrics).
- **Rate limiting metrics** — allowed and rejected requests. See [Rate limiting](/docs/v3/configuration/http-ratelimit#metrics).
- **Response control metrics** — aborted connections, IP blocks, and status rule matches. See [HTTP response control](/docs/v3/configuration/http-response#metrics).
- **Static file metrics** — files served and bytes sent, with compression and cache hit attributes. See [Static file serving](/docs/v3/configuration/static-content#metrics).
- **Rewrite metrics** — applied rewrites and invalid rewrite errors. See [URL rewriting](/docs/v3/configuration/http-rewrite#metrics).
- **Proxy metrics** — backend selection, health, connection pooling, and TLS failures. See [Reverse proxying](/docs/v3/configuration/reverse-proxying#metrics).
- **Process metrics** — CPU time, CPU utilization, and memory usage from `/proc/self/stat`. See [Process metrics](#process-metrics) below.
- **Admin API metrics** — uptime, active connections, request count, configuration reloads, and observability event backpressure. See [Admin API metrics](#admin-api-metrics) below.

#### Process metrics

The `metrics-process` module collects process-level metrics automatically when an observability backend is configured. It reads `/proc/self/stat` every 1 second.

**Platform support:** Linux only. On other platforms, the module is a no-op.

- `process.cpu.time` (Counter) — total CPU seconds broken down by different states.
  - Attributes: `cpu.mode` (`"user"` or `"system"`)
- `process.cpu.utilization` (Gauge) — CPU utilization since the last measurement.
  - Attributes: `cpu.mode` (`"user"` or `"system"`)
- `process.memory.usage` (UpDownCounter) — the change in physical memory (RSS) since the last measurement.
- `process.memory.virtual` (UpDownCounter) — the change in committed virtual memory (VMS) since the last measurement.

#### Admin API metrics

The `metrics-admin` module collects metrics exposed via admin API automatically when an observability backend is configured.

- `ferron.admin.uptime` (Gauge) — time since the server started.
- `ferron.admin.connections_active` (Gauge) — currently open TCP connections across all HTTP listeners.
- `ferron.admin.requests_total` (Counter) — total HTTP requests served across all listeners.
- `ferron.admin.reloads` (Counter) — number of configuration reloads performed.
- `ferron.admin.observability_events_dropped` (Counter) — total number of observability events dropped due to backpressure.
- `ferron.admin.observability_event_queue_len` (Gauge) — approximate current length of the observability event queue.

These metrics correspond to the same data exposed by the admin API's `GET /status` endpoint, but are available as time-series data for monitoring and alerting.

#### HTTP and proxy metrics

Ferron also emits request-path metrics for common observability gaps:

- `ferron.http.server.pre_handler_request_count` (Counter) — malformed or timed-out requests rejected before the normal HTTP handler completes.
- `ferron.http.server.redirects` (Counter) — redirects emitted by the core server, including trailing-slash and HTTP-to-HTTPS redirects.
- `ferron.http.server.client_ip_rewrites` (Counter) — requests whose client IP was rewritten from a trusted proxy header.
- `ferron.http.server.cors_preflights` (Counter) — CORS preflight requests handled before the main pipeline.
- `ferron.http.server.connection_errors` (Counter) — listener and handshake failures, labeled by transport and lifecycle stage.
- `ferron.proxy.failures` (Counter) — reverse-proxy failures that returned an error before a backend response was produced.
- `ferron.forward_proxy.requests` (Counter) — forward-proxy requests, labeled by mode (`connect` or `request`) and outcome.
- `ferron.static.responses` (Counter) — static-file responses across normal, conditional, range, and error paths.

### Tracing

Each HTTP request generates a root trace span and multiple nested spans for pipeline execution:

#### Root request span

- **`StartSpan("ferron.request")`** — emitted when the request enters the handler.
  - Attributes: `http.request.method`, `url.full`, `url.scheme`, `server.address`, `server.port`, `client.address`
- **`EndSpan("ferron.request", error)`** — emitted when the request completes.
  - Attributes: `http.response.status_code`, `http.route` (if applicable), `error.type` (if status >= 400)

#### Pipeline execution span

- **`ferron.pipeline.execute`** — wraps the entire pipeline execution, including all forward and inverse stages. This span is a child of `ferron.request`.

#### Per-stage spans

Each pipeline and file-serving stage generates its own forward and inverse span as a child of `ferron.pipeline.execute`, enabling flame graph analysis:

| Span name | Module | Description |
| --- | --- | --- |
| `ferron.stage.rewrite` | `http-rewrite` | URL rewrite stage |
| `ferron.stage.rate_limit` | `http-ratelimit` | Rate limiting stage |
| `ferron.stage.headers` | `http-headers` | Response header manipulation stage |
| `ferron.stage.reverse_proxy` | `http-proxy` | Reverse proxy stage |
| `ferron.stage.static_file` | `http-static` | Static file serving stage |
| `ferron.stage.http_response` | `http-response` | Response control stage |
| `ferron.stage.<name>.inverse` | (any) | Inverse (cleanup) operation for a stage |

#### Error pipeline span

- **`ferron.pipeline.execute_error`** — wraps error pipeline execution when generating error responses.
  - Attributes: `http.response.status_code`

Trace events are consumed by observability backends that support tracing (e.g. OTLP). All spans from the same request share the same `trace_id`, and access logs carry the matching request span context when available.

### Prometheus export

For Prometheus-specific metrics export, see [Prometheus metrics](/docs/v3/configuration/observability-prometheus).

### OTLP export

For OpenTelemetry Protocol (OTLP) export configuration, see [OTLP observability](/docs/v3/configuration/observability-otlp).

## Notes and troubleshooting

- If log files are not being written, verify the file paths are accessible and the Ferron process has write permissions.
- For global observability configuration (`console_log`, `log`, `error_log` shorthand directives), see [Core directives](/docs/v3/configuration/core-directives#observability).
- For log format details, see the `json` and `text` formatter sections above.
- Observability sinks may drop events when under high load; Ferron exposes an admin metric `observability_events_dropped` and a status field to help detect and tune exporter queues.
