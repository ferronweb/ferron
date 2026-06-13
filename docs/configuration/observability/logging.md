---
title: "Configuration: logging"
description: "Log signals, log formatters (JSON and text), log levels, and trace ID display in logs."
---

This page documents logging configuration for Ferron, including log signals, log formatters, available fields, and how trace IDs appear in log output.

> [!info]
>
> - For trace context propagation, Baggage, and trace sampling, see [Tracing](/docs/v3/configuration/observability/tracing).
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).

## Log signals

Ferron emits two log signals: **access logs** and **application logs**.

| Signal | What it captures |
|---|---|
| Access logs | Per-request HTTP request/response data (method, path, status, duration, etc.) |
| Application logs | Server-level messages (startup, config reloads, errors, debug output) |

Access logs are configured per-host via the `log` directive. Application logs are configured via the `console_log` and `error_log` directives (core-directives) or the `observability` block with `provider console` or `provider file`. There is no separate "error log" signal — the `error_log` directive is simply the file sink for the application log signal.

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

> [!important]
> Access logs don't contain sensitive fields (such as `header_cookie`, `header_authorization`). This is to ensure sensitive data is not exposed in log output.

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

### Log levels

The `log_level` directive (in the `observability` block or via `console_log`/`error_log` aliases) controls the minimum severity level for application logs:

| Level | When to use |
|-------|-------------|
| `error` | Production default. Only errors are logged. |
| `warn` | Debugging performance issues. |
| `info` | Request-level detail. Use for troubleshooting. |
| `debug` | Deep debugging. High volume. |

**Configuration example:**

```ferron
example.com {
    observability {
        provider console
        log_level debug
    }
}
```

### Console vs file vs OTLP

The `format` directive (json/text) applies to **file and console** sinks. OTLP also a different mechanism:

| Sink | Formatting directive | Configuration |
|------|---------------------|---------------|
| File (`provider file`) | `format json` or `format text` | `observability { provider file }` |
| Console (`provider console`) | `format json` or `format text` | `observability { provider console }` |
| OTLP (`provider otlp`) | `log_style modern` or `log_style legacy` (with `format json` or `format text`) | `observability { provider otlp }` |
| Prometheus | N/A (metrics only) | `observability { provider prometheus }` |

> [!note]
> Prometheus is metrics-only — it does not export logs. For log export, configure OTLP or use file/console sinks.

> [!tip]
> If log files are not being written, verify file paths are accessible and the Ferron process has write permissions. For global observability configuration, see [Core directives](/docs/v3/configuration/server/core-directives#observability).

## Trace ID in console and file logs

Console and file loggers prefix log messages with `[trace=<trace_id>]` when a trace context is available. This enables grep-based filtering by trace ID without requiring an OTLP backend.

**Example log output:**

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

> [!tip]
> To enable trace IDs in logs without full OTLP tracing, set `force_trace true` in the HTTP configuration. See [HTTP host directives](/docs/v3/configuration/server/host).
