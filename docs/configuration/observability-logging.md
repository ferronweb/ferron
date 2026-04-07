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
        format "json"
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

| Field | Placeholder | Description |
| --- | --- | --- |
| `path` | `{path}` | The request URI path (e.g. `/index.html`) |
| `path_and_query` | `{path_and_query}` | The request URI with path and query |
| `method` | `{method}` | The HTTP request method (e.g. `GET`, `POST`) |
| `version` | `{version}` | The HTTP version (e.g. `HTTP/1.1`, `HTTP/2.0`) |
| `scheme` | `{scheme}` | The request scheme (`http` or `https`) |
| `client_ip` | `{client_ip}` | The client IP address |
| `client_port` | `{client_port}` | The client port number |
| `client_ip_canonical` | `{client_ip_canonical}` | The client IP in canonical form |
| `server_ip` | `{server_ip}` | The server IP address |
| `server_port` | `{server_port}` | The server port number |
| `server_ip_canonical` | `{server_ip_canonical}` | The server IP in canonical form |
| `auth_user` | `{auth_user}` | The authenticated username, or `-` if not authenticated |
| `status` | `{status_code}` | The HTTP response status code |
| `content_length` | `{content_length}` | The response content length, or `-` if not available |
| `duration_secs` | _(none)_ | Request processing duration in seconds |
| `timestamp` | _(none)_ | Request timestamp in CLF format |
| `header_<name>` | `{header:<name>}` | Request header values (one field per header) |

### Log formatters

#### `json`

The JSON formatter serializes each access log entry as a single-line JSON object. Provided by the `ferron-observability-format-json` module.

```ferron
example.com {
    log "access" {
        format "json"
    }
}
```

Example output:

```json
{"method":"GET","path":"/index.html","status":200,"duration_secs":0.012,"client_ip":"127.0.0.1","remote_ip":"127.0.0.1"}
```

Use the `fields` directive to limit which fields appear in the JSON output. If `fields` is not specified, all available access log fields are emitted.

#### `text`

The text formatter generates each access log entry as a plain text string using a configurable pattern. Provided by the `ferron-observability-format-text` module.

By default, it uses the **Combined Log Format (CLF)**, the same format used by Apache and Nginx.

**Configuration example:**

```ferron
example.com {
    log "access" {
        format "text"
    }
}
```

Example output:

```
127.0.0.1 - frank [05/Apr/2026:14:32:01 +0200] "GET /index.html HTTP/1.1" 200 1234 "http://www.example.com/start.html" "Mozilla/5.0"
```

#### Pattern syntax

The `access_pattern` directive supports the following tokens:

| Token | Description | Example |
| --- | --- | --- |
| `%field_name` | Access log field | `%client_ip`, `%status`, `%method` |
| `%{Header-Name}i` | Request header | `%{Referer}i`, `%{User-Agent}i` |
| `%{format}t` | Timestamp with custom format | `%{%Y-%m-%d %H:%M:%S}t` |
| `%t` | Timestamp (uses `timestamp_format` or CLF default) | `%t` |
| `%%` | Literal `%` character | `%%` |
| Other text | Passed through literally | `"`, ` `, `-` |

Request headers are available via the `%{Header-Name}i` syntax. The header name is case-insensitive and hyphens are converted to underscores internally.

### Metrics

The HTTP server emits the following OpenTelemetry-style metrics via the observability event system:

| Metric | Type | Unit | Description |
| --- | --- | --- | --- |
| `http.server.active_requests` | UpDownCounter | `{request}` | Number of active HTTP requests. |
| `http.server.request.duration` | Histogram | `s` | Duration of HTTP requests. |
| `ferron.http.server.request_count` | Counter | `{request}` | Total number of HTTP requests completed. |

All metrics include attributes for `http.request.method`, `url.scheme`, `network.protocol.name`, and `network.protocol.version`. The `http.server.request.duration` and `ferron.http.server.request_count` metrics also include `http.response.status_code` and `error.type` (for 4xx/5xx responses).

#### Rate limiting metrics

The `http-ratelimit` module emits counters for allowed and rejected requests:

| Metric | Type | Description | Attributes |
| --- | --- | --- | --- |
| `ferron.ratelimit.allowed` | Counter | Requests that passed rate limiting. | `ferron.ratelimit.key_type`: `"ip"`, `"header"`, or `"uri"` |
| `ferron.ratelimit.rejected` | Counter | Requests rejected due to exhausted rate limit buckets or registry at capacity. | `ferron.ratelimit.key_type`: `"ip"`, `"header"`, or `"uri"` |

#### Response control metrics

The `http-response` module emits metrics for security and policy enforcement:

| Metric | Type | Description | Attributes |
| --- | --- | --- | --- |
| `ferron.response.aborted` | Counter | Connections aborted via the `abort` directive. | _none_ |
| `ferron.response.ip_blocked` | Counter | Connections blocked via `block`/`allow` directives. | _none_ (raw IPs are not included in metrics) |
| `ferron.response.status_rule_matched` | Counter | Custom status codes returned via the `status` directive. | `http.response.status_code`, `ferron.rule_id` |

#### Static file metrics

The `http-static` module emits metrics for file serving:

| Metric | Type | Unit | Description | Attributes |
| --- | --- | --- | --- | --- |
| `ferron.static.files_served` | Counter | `{file}` | Number of static files served. | `ferron.compression`: `"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`; `ferron.cache_hit`: `"true"` or `"false"` |
| `ferron.static.bytes_sent` | Histogram | `By` | Bytes sent for static file responses. Buckets: 1KB, 10KB, 100KB, 1MB, 10MB, 100MB. | Same as above |

#### Rewrite metrics

The `http-rewrite` module emits counters for URL rewrites:

| Metric | Type | Description | Attributes |
| --- | --- | --- | --- |
| `ferron.rewrite.rewrites_applied` | Counter | URLs successfully rewritten. | _none_ |
| `ferron.rewrite.invalid` | Counter | Rewrite rules that resulted in an invalid path (400 response). | _none_ |

#### Proxy metrics

The `http-proxy` module emits the following metrics:

| Metric | Type | Unit | Description | Attributes |
| --- | --- | --- | --- | --- |
| `ferron.proxy.backends.selected` | Counter | `{backend}` | Backends selected during load balancing. | Backend URL/unix path |
| `ferron.proxy.backends.unhealthy` | Counter | `{backend}` | Backends marked as unhealthy. | Backend URL/unix path |
| `ferron.proxy.requests` | Counter | `{request}` | Upstream proxy requests completed. | `ferron.proxy.connection_reused`: `true`/`false`; `http.response.status_code`; `ferron.proxy.status_code` |
| `ferron.proxy.tls_handshake_failures` | Counter | `{handshake}` | TLS handshake failures with upstream backends. | _none_ |
| `ferron.proxy.pool.waits` | Counter | `{wait}` | Times the connection pool was exhausted and a request had to wait. | _none_ |
| `ferron.proxy.pool.wait_time` | Histogram | `s` | Duration spent waiting for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s. | _none_ |

#### Process metrics

The `observability-process-metrics` module collects process-level metrics automatically when an observability backend is configured. It reads `/proc/self/stat` every 1 second.

**Platform support:** Linux only. On other platforms, the module is a no-op.

| Metric | Type | Unit | Description | Attributes |
| --- | --- | --- | --- | --- |
| `process.cpu.time` | Counter | `s` | Total CPU seconds broken down by different states. | `cpu.mode`: `"user"` or `"system"` |
| `process.cpu.utilization` | Gauge | `1` | CPU utilization since the last measurement. | `cpu.mode`: `"user"` or `"system"` |
| `process.memory.usage` | UpDownCounter | `By` | The change in physical memory (RSS) since the last measurement. | _none_ |
| `process.memory.virtual` | UpDownCounter | `By` | The change in committed virtual memory (VMS) since the last measurement. | _none_ |

### Tracing

Each HTTP request generates a root trace span and multiple nested spans for pipeline execution:

#### Root request span

- **`StartSpan("ferron.request_handler")`** — emitted when the request enters the handler.
  - Attributes: `http.request.method`, `url.path`, `url.scheme`, `server.address`, `server.port`, `client.address`
- **`EndSpan("ferron.request_handler", error)`** — emitted when the request completes.
  - Attributes: `http.response.status_code`, `http.route` (if applicable), `error.type` (if status >= 400)

#### Pipeline execution span

- **`ferron.pipeline.execute`** — wraps the entire pipeline execution, including all forward and inverse stages.

#### Per-stage spans

Each pipeline stage generates its own forward and inverse span, enabling flame graph analysis:

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

Trace events are consumed by observability backends that support tracing (e.g. OTLP). All spans from the same request share the same `trace_id`, enabling correlated queries.

### OTLP export

The `observability-otlp` module exports logs, metrics, and traces to an OTLP collector. Configured via `observability` blocks with `provider "otlp"`.

**Configuration example:**

```ferron
example.com {
    observability {
        provider "otlp"

        logs "https://collector:4318/v1/Logs" {
            protocol "http/protobuf"
        }

        metrics "https://collector:4318/v1/Metrics" {
            protocol "http/protobuf"
        }

        traces "https://collector:4317" {
            protocol "grpc"
        }

        service_name "my-service"
    }
}
```

#### Signal sub-blocks

Each signal type (`logs`, `metrics`, `traces`) is configured independently. Omitting a signal disables it for that host.

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `logs` | `<endpoint>` | OTLP logs endpoint. | disabled |
| `metrics` | `<endpoint>` | OTLP metrics endpoint. | disabled |
| `traces` | `<endpoint>` | OTLP traces endpoint. | disabled |

Each signal sub-block supports these nested directives:

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `protocol` | `<string>` | Transport protocol. One of `grpc`, `http/protobuf`, `http/json`. | `grpc` |
| `authorization` | `<string>` | HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC). | none |

#### Global options

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `service_name` | `<string>` | OTLP resource service name. | `"ferron"` |
| `no_verify` | `<bool>` | Disable TLS certificate verification. Use with caution. | `false` |

#### Signal correlation

All three signals from the same HTTP request share the same `trace_id`. This enables correlated queries like "show me all logs and metrics for trace `abc123`".

## Notes and troubleshooting

- If log files are not being written, verify the file paths are accessible and the Ferron process has write permissions.
- For global observability configuration (`console_log`, `log`, `error_log` shorthand directives), see [Core directives](/docs/v3/configuration/core-directives#observability).
- For log format details, see the `json` and `text` formatter sections above.
