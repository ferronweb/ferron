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

## Tracing

Each HTTP request generates a trace span:

- **`StartSpan("ferron.request_handler")`** — emitted when the request enters
  the handler.
- **`EndSpan("ferron.request_handler", error)`** — emitted when the request
  completes. The optional error field contains the error description if the
  handler returned an error.

Trace events are consumed by observability backends that support tracing (e.g.
OTLP).
