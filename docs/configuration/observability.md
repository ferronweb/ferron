# Observability And Logging

Titanium provides modular observability backends that can collect access logs,
emit metrics, and record traces. This page describes the logging and log
formatter configuration surface.

## Log Configuration

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

## Access Log Fields

Each access log entry contains the following fields, matching the Ferron 2
placeholder names:

| Field | Placeholder | Description |
| --- | --- | --- |
| `path` | `{path}` | The request URI path (e.g. `/index.html`) |
| `path_and_query` | `{path_and_query}` | The request URI with path and query (e.g. `/index.html?param=value`) |
| `method` | `{method}` | The HTTP request method (e.g. `GET`, `POST`) |
| `version` | `{version}` | The HTTP version (e.g. `HTTP/1.1`, `HTTP/2.0`) |
| `scheme` | `{scheme}` | The request scheme (`http` or `https`) |
| `client_ip` | `{client_ip}` | The client IP address |
| `client_port` | `{client_port}` | The client port number |
| `client_ip_canonical` | `{client_ip_canonical}` | The client IP in canonical form (IPv4-mapped IPv6 addresses like `::ffff:127.0.0.1` are converted to `127.0.0.1`) |
| `server_ip` | `{server_ip}` | The server IP address |
| `server_port` | `{server_port}` | The server port number |
| `server_ip_canonical` | `{server_ip_canonical}` | The server IP in canonical form |
| `auth_user` | `{auth_user}` | The authenticated username, or `-` if not authenticated |
| `status` | `{status_code}` | The HTTP response status code |
| `content_length` | `{content_length}` | The response content length, or `-` if not available |
| `duration_secs` | _(none)_ | Request processing duration in seconds |
| `timestamp` | _(none)_ | Request timestamp in CLF format (e.g. `05/Apr/2026:14:32:01 +0200`) |
| `header_<name>` | `{header:<name>}` | Request header values (one field per header, e.g. `header_user_agent`) |

## Log Formatters

### `json`

The JSON formatter serializes each access log entry as a single-line JSON
object. It is provided by the `ferron-observability-format-json` module.

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

#### Field Filtering

Use the `fields` directive to limit which fields appear in the JSON output:

```ferron
log "access" {
    format "json"
    fields "method" "path" "status" "duration_secs" "client_ip"
}
```

This produces output containing only the specified fields:

```json
{"method":"GET","path":"/index.html","status":200,"duration_secs":0.012,"client_ip":"127.0.0.1"}
```

If `fields` is not specified, all available access log fields are emitted.

### `text`

The text formatter generates each access log entry as a plain text string using a
configurable pattern. It is provided by the `ferron-observability-format-text`
module.

By default, it uses the **Combined Log Format (CLF)**, the same format used by
Apache and Nginx:

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

#### Custom Patterns

Use the `access_pattern` directive to define a custom log format:

```ferron
log "access" {
    format "text"
    access_pattern "%client_ip %method %path %status %content_length %{duration_secs}s"
}
```

Example output:

```
127.0.0.1 GET /index.html 200 1234 0.012s
```

#### Timestamp Formatting

The `%t` token outputs the request timestamp. By default, it uses the CLF
timestamp format (`%d/%b/%Y:%H:%M:%S %z`). Use the `timestamp_format` directive
to customize it with [chrono format specifiers](https://docs.rs/chrono/latest/chrono/format/strftime/index.html):

```ferron
log "access" {
    format "text"
    timestamp_format "%Y-%m-%d %H:%M:%S"
}
```

Example output:

```
127.0.0.1 - frank [2026-04-05 14:32:01] "GET /index.html HTTP/1.1" 200 1234
```

#### Pattern Syntax

The `access_pattern` directive supports the following tokens:

| Token | Description | Example |
| --- | --- | --- |
| `%field_name` | Access log field | `%client_ip`, `%status`, `%method` |
| `%{Header-Name}i` | Request header | `%{Referer}i`, `%{User-Agent}i` |
| `%{format}t` | Timestamp with custom format | `%{%Y-%m-%d %H:%M:%S}t` |
| `%t` | Timestamp (uses `timestamp_format` or CLF default) | `%t` |
| `%%` | Literal `%` character | `%%` |
| Other text | Passed through literally | `"`, ` `, `-` |

#### Request Header Access

Request headers are available via the `%{Header-Name}i` syntax. The header name
is case-insensitive and hyphens are converted to underscores internally:

```ferron
access_pattern "%client_ip \"%{User-Agent}i\" %{Referer}i"
```

#### Field Filtering

Like the JSON formatter, the text formatter supports the `fields` directive to
limit which fields are collected:

```ferron
log "access" {
    format "text"
    access_pattern "%client_ip %method %path %status"
    fields "client_ip" "method" "path" "status"
}
```

## Metrics

The HTTP server emits the following OpenTelemetry-style metrics via the
observability event system:

| Metric | Type | Unit | Description |
| --- | --- | --- | --- |
| `http.server.active_requests` | UpDownCounter | `{request}` | Number of active HTTP requests. Incremented at request start, decremented at completion. |
| `http.server.request.duration` | Histogram | `s` | Duration of HTTP requests. Includes buckets for common latency percentiles. |
| `ferron.http.server.request_count` | Counter | `{request}` | Total number of HTTP requests completed. |

All metrics include attributes for `http.request.method`, `url.scheme`,
`network.protocol.name`, and `network.protocol.version`. The
`ferron.http.server.request_count` metric also includes `http.response.status_code`
and `error.type` (for 4xx/5xx responses). When an error occurred before the
request handler executed, `ferron.http.request.error_status_code` is included.

### Process Metrics

The `observability-process-metrics` module collects process-level metrics
automatically when an observability backend is configured (OTLP, console,
file, etc.). It reads `/proc/self/stat` every 1 second and emits metrics
through the same observability event pipeline as HTTP metrics.

**Platform support:** Linux only. On other platforms, the module is a no-op.

| Metric | Type | Unit | Description | Attributes |
| --- | --- | --- | --- | --- |
| `process.cpu.time` | Counter | `s` | Total CPU seconds broken down by different states. | `cpu.mode`: `"user"` or `"system"` |
| `process.cpu.utilization` | Gauge | `1` | Difference in `process.cpu.time` since the last measurement, divided by the elapsed time and number of CPUs available to the process. | `cpu.mode`: `"user"` or `"system"` |
| `process.memory.usage` | UpDownCounter | `By` | The change in physical memory (RSS) since the last measurement. | _none_ |
| `process.memory.virtual` | UpDownCounter | `By` | The change in committed virtual memory (VMS) since the last measurement. | _none_ |

#### CPU Time

`process.cpu.time` is a **counter** that reports the delta in CPU seconds
(user or system mode) since the previous collection interval. To get the
cumulative CPU time, sum all received values per `cpu.mode`.

```
process.cpu.time{cpu.mode="user"}    0.023
process.cpu.time{cpu.mode="system"}  0.005
```

#### CPU Utilization

`process.cpu.utilization` is a **gauge** representing the instantaneous
utilization, calculated as:

```
utilization = delta_cpu_time / (elapsed_seconds * num_cpus)
```

Values range from `0.0` (no CPU usage) to `1.0` (all CPUs fully utilized),
and can exceed `1.0` under certain conditions (e.g., CPU time accounting).

#### Memory Usage and Virtual Memory

`process.memory.usage` and `process.memory.virtual` are **up-down counters**
that report the delta in RSS and VMS bytes since the last collection interval.
A positive value means memory grew; a negative value means it shrank. To get
the current memory usage, sum all received values from startup.

#### Configuration

Process metrics are automatically enabled whenever an observability backend
(OTLP, console, file) is configured in the global scope or on any host. The
metrics flow through the same backend configuration. For example, with OTLP:

```ferron
observability {
    provider "otlp"

    metrics "http://localhost:4318/v1/metrics" {
        protocol "http/protobuf"
    }
}
```

Process metrics will be exported alongside HTTP server metrics to the same
collector, with the same `service_name` and signal configuration.

## Tracing

Each HTTP request generates a trace span:

- **`StartSpan("ferron.request_handler")`** — emitted when the request enters
  the handler.
- **`EndSpan("ferron.request_handler", error)`** — emitted when the request
  completes. The optional error field contains the error description if the
  handler returned an error.

Trace events are consumed by observability backends that support tracing (e.g.
OTLP).

## OTLP (OpenTelemetry Protocol)

The `observability-otlp` module exports logs, metrics, and traces to an OTLP
collector. It is configured via `observability` blocks with `provider "otlp"`.

### Basic Configuration

```ferron
example.com {
    observability {
        provider "otlp"

        logs "https://collector:4318/v1/logs" {
            protocol "http/protobuf"
        }

        metrics "https://collector:4318/v1/metrics" {
            protocol "http/protobuf"
        }

        traces "https://collector:4317" {
            protocol "grpc"
        }

        service_name "my-service"
    }
}
```

### Signal Sub-Blocks

Each signal type (`logs`, `metrics`, `traces`) is configured independently.
Omitting a signal disables it for that host.

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

### Global Options

These directives sit at the `observability` block level (not inside signal sub-blocks):

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `service_name` | `<string>` | OTLP resource service name. | `"ferron"` |
| `no_verify` | `<bool>` | Disable TLS certificate verification. Use with caution — only for development or trusted internal networks. | `false` |

### Signal Correlation

All three signals from the same HTTP request share the same `trace_id`. When a
request enters the handler, a trace span is started. Any log events or metric
records generated during that request are automatically tagged with the span's
`trace_id` and `span_id`. This enables correlated queries like "show me all logs
and metrics for trace `abc123`".

### Per-Host Configuration

Different hosts can send to different collectors:

```ferron
api.example.com {
    observability {
        provider "otlp"
        service_name "api"

        logs "https://prod-collector:4318/v1/logs" {}
        metrics "https://prod-collector:4318/v1/metrics" {}
        traces "https://prod-collector:4317" {}
    }
}

admin.example.com {
    observability {
        provider "otlp"
        service_name "admin"

        logs "https://dev-collector:4318/v1/logs" {
            protocol "http/json"
        }
    }
}
```

### Mixing Observability Backends

You can combine OTLP with other backends (console, file) on the same host:

```ferron
example.com {
    log "access.log" {
        format "json"
    }

    observability {
        provider "otlp"

        metrics "https://collector:4318/v1/metrics" {}
    }
}
```
