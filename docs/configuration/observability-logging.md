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

All metrics include attributes for `http.request.method`, `url.scheme`, `network.protocol.name`, and `network.protocol.version`.

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

Each HTTP request generates a trace span:

- **`StartSpan("ferron.request_handler")`** — emitted when the request enters the handler.
- **`EndSpan("ferron.request_handler", error)`** — emitted when the request completes.

Trace events are consumed by observability backends that support tracing (e.g. OTLP).

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
- For global observability configuration (`console_log`, `log`, `error_log` shorthand directives), see [Core directives](/docs/v3/core-directives#observability).
- For log format details, see the `json` and `text` formatter sections above.
