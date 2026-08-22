---
title: "Configuration: logging"
description: "Log signals, log formatters (JSON and text), log levels, and trace ID display in logs."
---

This page documents logging configuration for Ferron. It covers log signals, log formatters, available fields, and trace ID display in log output.

> [!info]
>
> - For trace context propagation, Baggage, and trace sampling, see [Tracing](/docs/v3/configuration/observability/tracing).
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).

## Log signals

Ferron emits two log signals: **access logs** and **application logs**.

| Signal           | What it captures                                                              |
| ---------------- | ----------------------------------------------------------------------------- |
| Access logs      | Per-request HTTP request/response data (method, path, status, duration, etc.) |
| Application logs | Server-level messages (startup, config reloads, errors, debug output)         |

Configure access logs per-host with the `log` directive. Configure application logs with the `console_log` and `error_log` directives (core-directives) or the `observability` block with `provider console` or `provider file`. There is no separate "error log" signal. The `error_log` directive is the file sink for the application log signal.

> [!tip]
> If log files are not written, verify file paths are accessible and the Ferron process has write permissions. For global observability configuration, see [Core directives](/docs/v3/configuration/server/core-directives#observability).

## Directives

### Access logging

Configure access logs with `log` blocks inside host or global scopes:

```ferron
example.com {
    log "access" {
        format json
        fields "method" "path" "status" "duration_secs"
    }
}
```

| Nested directive | Arguments     | Description                                                                                          | Default    |
| ---------------- | ------------- | ---------------------------------------------------------------------------------------------------- | ---------- |
| `format`         | `<string>`    | Log formatter to use. The available formatters depend on which observability modules load.           | none       |
| `fields`         | `<string>...` | Field names to include in the log output. When you omit this, the server emits all available fields. | all fields |

#### Access log fields

Each access log entry contains the following fields:

| Field                 | Description                                                                |
| --------------------- | -------------------------------------------------------------------------- |
| `path`                | The request URI path (e.g. `/index.html`)                                  |
| `path_and_query`      | The request URI with path and query                                        |
| `method`              | The HTTP request method (e.g. `GET`, `POST`)                               |
| `version`             | The HTTP version (e.g. `HTTP/1.1`, `HTTP/2.0`)                             |
| `scheme`              | The request scheme (`http` or `https`)                                     |
| `client_ip`           | The client IP address                                                      |
| `client_port`         | The client port number                                                     |
| `client_ip_canonical` | The client IP in canonical form                                            |
| `server_ip`           | The server IP address                                                      |
| `server_port`         | The server port number                                                     |
| `server_ip_canonical` | The server IP in canonical form                                            |
| `auth_user`           | The authenticated username, or `-` if not authenticated                    |
| `status`              | The HTTP response status code                                              |
| `content_length`      | The response content length, or `-` if not available                       |
| `duration_secs`       | Request processing duration in seconds                                     |
| `timestamp`           | Request timestamp in CLF format                                            |
| `header_<name>`       | Request header values (one field per header)                               |
| `span_id`             | Optional trace span ID for the request (if W3C trace context is available) |
| `trace_id`            | Optional trace ID for the request (if W3C trace context is available)      |

> [!important]
> Access logs do not contain sensitive fields (such as `header_cookie`, `header_authorization`). This makes sure log output does not expose sensitive data.

> [!info]
> Pipeline modules can contribute additional access log fields when active. These fields are only present when the corresponding module handles the request. For the list of module-contributed access log fields, see the documentation for the respective module.

### Log formatters

#### `json`

The JSON formatter serializes each access log entry as a single-line JSON object. Use the `logformat-json` module for this formatter.

```ferron
example.com {
    log "access" {
        format json
    }
}
```

Example output:

```json
{
  "method": "GET",
  "path": "/index.html",
  "status": 200,
  "duration_secs": 0.012,
  "client_ip": "127.0.0.1",
  "remote_ip": "127.0.0.1"
}
```

Use the `fields` directive to limit which fields appear in the JSON output. If you do not specify `fields`, the server emits all available access log fields.

#### `text`

The text formatter generates each access log entry as a plain text string using a configurable pattern. Use the `logformat-text` module for this formatter.

By default, it uses the **Enhanced Combined Log Format** (ECLF). Ferron extends CLF with `Host` header and trace ID fields.

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
127.0.0.1 - frank [05/Apr/2026:14:32:01 +0200] "GET /index.html HTTP/1.1" 200 1234 "http://www.example.com/start.html" "Mozilla/5.0" "www.example.com" "abc123def456"
```

#### Common format string examples

You can customize the text log format using the `access_pattern` directive. Here are common format strings:

**Enhanced Combined Log Format (Ferron default):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length "%{Referer}i" "%{User-Agent}i" "%{Host}i" "%trace_id"
```

**Combined Log Format (Apache/Nginx standard):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length "%{Referer}i" "%{User-Agent}i"
```

**Common Log Format (CLF):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length
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

You can access request headers via the `%{Header-Name}i` syntax. The header name is case-insensitive, and Ferron converts hyphens to underscores internally.

### Application log formats

The `error_format` directive (in the `observability { provider file ... }` block or the `error_log` shorthand block) controls how the server formats application log messages. It supports the same formatters as access logs: `text` (default) and `json`.

```ferron
example.com {
    error_log /var/log/ferron/error.log {
        error_format json
    }
}
```

The `text` formatter produces human-readable lines:

```text
[2026-04-05 14:32:01.123 INFO] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] Cache miss for key: user:123
[2026-04-05 14:32:01.125 ERROR] [trace=abc123def456] Upstream connection refused
```

The `json` formatter produces structured JSON records:

```json
{"timestamp":1781327817042,"summary":"Request processed successfully","level":"INFO","target":"ferron::http","attributes":{},"trace_context":null}
{"timestamp":1781327818364,"summary":"Upstream connection refused","level":"ERROR","target":"ferron::proxy","attributes":{"upstream":"http://10.0.0.1:3000"},"trace_context":{"trace_id":"abc123def456","span_id":"789012345678","sampled":true}}
```

| Field           | Description                                                     |
| --------------- | --------------------------------------------------------------- |
| `timestamp`     | The Unix timestamp in milliseconds when the log event occurred  |
| `summary`       | The log message summary                                         |
| `level`         | Log severity level (`ERROR`, `WARN`, `INFO`, `DEBUG`)           |
| `target`        | The web server module target that emitted the log               |
| `attributes`    | Typed key-value pairs attached to the log event                 |
| `trace_context` | W3C trace context (`trace_id`, `span_id`, `sampled`), or `null` |

> [!note]
> The `error_format` directive is available for the `file` observability provider and the `error_log` shorthand. Console logs always use their native formatting based on the log level.

## Admin API structured logs

The admin API emits structured log events through the observability pipeline for important operational events:

| Event                         | Level | Condition               |
| ----------------------------- | ----- | ----------------------- |
| Admin config reload completed | INFO  | `POST /reload` succeeds |
| Admin config reload failed    | ERROR | `POST /reload` fails    |
| Admin config queried          | INFO  | `GET /config` requested |

## Reload log events

The `metrics-reload` module emits application log events during configuration reloads:

| Level  | Message                                                                               | Trigger                   |
| ------ | ------------------------------------------------------------------------------------- | ------------------------- |
| `INFO` | `Reloading configuration...`                                                          | A reload starts           |
| `WARN` | `Can't reload the server, continuing to run with the previous configuration: {error}` | The reload attempt failed |

These events carry the `ferron-metrics-reload` target, and the observability event system emits them.

### Structured logs

| Description (summary)      | Level  | Attributes                                         |
| -------------------------- | ------ | -------------------------------------------------- |
| Configuration reload       | `INFO` | none                                               |
| Configuration reload error | `WARN` | `error.message` (string): the reload error message |

### Error log attributes

Structured error logs include contextual attributes to aid troubleshooting:

| Attribute        | Description                                                                                       |
| ---------------- | ------------------------------------------------------------------------------------------------- |
| `error.type`     | Error category (e.g. `bad_request`, `timeout`, `tcp_connection_error`, `tcp_tls_handshake_error`) |
| `error.message`  | The human-readable error description                                                              |
| `client.address` | The client IP address, when available                                                             |
| `server.address` | The server IP address, when available                                                             |

Ferron includes the `client.address` and `server.address` attributes in:

- **Bad request (400) and timeout (408) logs**: Ferron emits these logs when a request fails before handler execution.
- **TLS handshake failure logs**: emitted when a TLS connection fails to establish or negotiate a protocol.
- **TCP connection error logs**: emitted when an HTTP/1.x or HTTP/2 connection encounters a transport-level error.
- **Request validation error logs**: emitted for invalid Host headers, malformed URLs, CONNECT path errors, and URL sanitization failures.

> [!note]
> Connection-level errors (for example, accept failures, PROXY protocol errors) do not include IP attributes. These errors occur before the server resolves the socket address.

## Variable interpolation in log filenames

The `log` and `error_log` directives support variable interpolation in file paths using the `{{variable}}` syntax. This enables use cases like per-host access logs or per-target error logs.

```ferron
example.com {
    log "/var/log/ferron/{{accesslog.header_host}}/access.log"
}
```

### Access log filename variables

When the server resolves an `log` filename, it uses the access log event fields as variables. All variable names are prefixed with `accesslog.`.

| Variable                        | Description                                                                                |
| ------------------------------- | ------------------------------------------------------------------------------------------ |
| `accesslog.path`                | The request URI path (e.g. `/index.html`)                                                  |
| `accesslog.path_and_query`      | The request URI with path and query                                                        |
| `accesslog.method`              | The HTTP request method (e.g. `GET`, `POST`)                                               |
| `accesslog.version`             | The HTTP version (e.g. `HTTP/1.1`, `HTTP/2.0`)                                             |
| `accesslog.scheme`              | The request scheme (`http` or `https`)                                                     |
| `accesslog.client_ip`           | The client IP address                                                                      |
| `accesslog.client_port`         | The client port number                                                                     |
| `accesslog.client_ip_canonical` | The client IP in canonical form                                                            |
| `accesslog.server_ip`           | The server IP address                                                                      |
| `accesslog.server_port`         | The server port number                                                                     |
| `accesslog.server_ip_canonical` | The server IP in canonical form                                                            |
| `accesslog.auth_user`           | The authenticated username, or `-` if not authenticated                                    |
| `accesslog.status`              | The HTTP response status code                                                              |
| `accesslog.content_length`      | The response content length, or `-` if not available                                       |
| `accesslog.duration_secs`       | Request processing duration in seconds                                                     |
| `accesslog.timestamp`           | Request timestamp in CLF format                                                            |
| `accesslog.header_<name>`       | Request header values (one field per header, lowercase, hyphens replaced with underscores) |
| `accesslog.trace_id`            | Optional trace ID (if W3C trace context is available)                                      |
| `accesslog.span_id`             | Optional trace span ID (if W3C trace context is available)                                 |

> [!important]
> Access log filename interpolation does not include sensitive fields (such as `header_cookie`, `header_authorization`). This makes sure log output does not expose sensitive data.

> [!info]
> Pipeline modules can contribute additional access log fields when active. These fields are available as `accesslog.<field_name>` variables when the corresponding module handles the request.

### Application log filename variables

When the server resolves an `error_log` filename, it uses the application log event fields as variables. All variable names are prefixed with `log.`.

| Variable          | Description                                                |
| ----------------- | ---------------------------------------------------------- |
| `log.level`       | Log severity level (`ERROR`, `WARN`, `INFO`, `DEBUG`)      |
| `log.target`      | The web server module target that emitted the log          |
| `log.message`     | The full-text log message                                  |
| `log.summary`     | The short summary used by OTLP `log_style modern`          |
| `log.trace_id`    | Optional trace ID (if W3C trace context is available)      |
| `log.span_id`     | Optional trace span ID (if W3C trace context is available) |
| `log.<attribute>` | Any structured attribute attached to the log event         |

Common attribute keys used across the server include `error.type`, `error.message`, `client.address`, `server.address`, and `upstream.address`. The available attributes depend on the log event source.

### Environment variables

You can also use environment variables in log filenames with the `env.` prefix:

```ferron
example.com {
    log "/var/log/ferron/{{env.CUSTOMER_NAME}}/access.log"
}
```

> [!warning]
> Using high-cardinality variables (such as `{{accesslog.client_ip}}` or `{{accesslog.timestamp}}`) in log filenames creates a separate file for each unique value. This can lead to a large number of open file handles. Use variables with bounded value sets (such as `{{accesslog.header_host}}` or `{{log.level}}`).

## Trace ID in console and file logs

Console and file loggers prefix log messages with `[trace=<trace_id>]` when a trace context exists. This enables grep-based filtering by trace ID without requiring an OTLP backend.

**Example log output:**

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

> [!tip]
> Ferron always enables trace context, so trace IDs appear automatically in console and file log messages as `[trace=<trace_id>]` prefixes. You do not need additional configuration.
