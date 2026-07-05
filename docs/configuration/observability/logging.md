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

#### Module-contributed fields

Pipeline modules contribute additional access log fields when active. These fields are only present when the corresponding module handles the request.

| Field | Module | Description |
|---|---|---|
| `ferron.proxy.backend_url` | `http-proxy` | Backend URL that served the proxied request |
| `ferron.proxy.backend_resolved_ip` | `http-proxy` | Resolved IP address of the backend (strict DNS only) |
| `ferron.proxy.backend_unix_path` | `http-proxy` | Unix socket path of the backend (if applicable) |
| `ferron.proxy.connection_reused` | `http-proxy` | Whether a pooled connection was reused |
| `ferron.proxy.retry_count` | `http-proxy` | Number of retry attempts (0 if none) |
| `ferron.proxy.circuit_breaker_state` | `http-proxy` | Circuit breaker state of the backend: `closed`, `open`, or `half_open` |
| `ferron.cache.result` | `http-cache` | Cache outcome: `hit`, `miss`, `stale`, `bypass`, `revalidate`, `purge`, `purge_rejected` |
| `ferron.cache.zone` | `http-cache` | Cache zone identifier |
| `ferron.ratelimit.result` | `http-ratelimit` | Rate limit decision: `allowed` or `rejected` |
| `ferron.ratelimit.zone` | `http-ratelimit` | Rate limit zone identifier |
| `ferron.ratelimit.retry_after_secs` | `http-ratelimit` | Seconds until next request allowed (rejection only) |
| `ferron.abuseban.action` | `http-abuseban` | Action taken: `rejected` or `skip` |
| `ferron.abuseban.reason` | `http-abuseban` | Ban reason (rejection only) |
| `ferron.abuseban.remaining_secs` | `http-abuseban` | Seconds remaining on ban (rejection only) |
| `ferron.basicauth.result` | `http-basicauth` | Auth outcome: `skip`, `failure`, or `success` |
| `ferron.fauth.result` | `http-fauth` | Forwarded auth outcome: `success` or `failure` |
| `ferron.fauth.backend_url` | `http-fauth` | Auth backend URL contacted |
| `ferron.static.file_path` | `http-static` | Absolute file path served |
| `ferron.response.action` | `http-response` | Response action: `abort`, `block`, or `status` |
| `ferron.fproxy.mode` | `http-fproxy` | Forward proxy mode: `tunnel` or `proxy` |
| `ferron.cgi.script_path` | `http-cgi` | Path to CGI script executed |
| `ferron.cgi.exit_code` | `http-cgi` | CGI process exit code |
| `ferron.fcgi.backend_url` | `http-fcgi` | FastCGI backend URL |
| `ferron.fcgi.script_filename` | `http-fcgi` | Script filename (file mode) |
| `ferron.scgi.backend_url` | `http-scgi` | SCGI backend URL |
| `ferron.compression.algorithm` | `http-compression` | Compression algorithm: `gzip`, `br`, `deflate`, `zstd`, or `identity` |
| `ferron.rewrite.applied` | `http-rewrite` | Whether a URL rewrite was applied |

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

By default, it uses the **Enhanced Combined Log Format** (ECLF; Ferron's extended version of CLF), which extends Combined Log Format with `Host` header and trace ID fields.

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

Request headers are available via the `%{Header-Name}i` syntax. The header name is case-insensitive and hyphens are converted to underscores internally.

### Application log formats

The `error_format` directive (in the `observability { provider file ... }` block or the `error_log` shorthand block) controls how application log messages are formatted. It supports the same formatters as access logs: `text` (default) and `json`.

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

| Field | Description |
|-------|-------------|
| `timestamp` | The Unix timestamp in milliseconds when the log event occurred |
| `summary` | The log message summary |
| `level` | Log severity level (`ERROR`, `WARN`, `INFO`, `DEBUG`) |
| `target` | The web server module target that emitted the log |
| `attributes` | Typed key-value pairs attached to the log event |
| `trace_context` | W3C trace context (`trace_id`, `span_id`, `sampled`), or `null` |

> [!note]
> The `error_format` directive is available for the `file` observability provider and the `error_log` shorthand. Console logs always use their native formatting based on the log level.

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
| File (`provider file`) | `format` (access) / `error_format` (application) | `observability { provider file }` |
| Console (`provider console`) | `format` (access only) | `observability { provider console }` |
| OTLP (`provider otlp`) | `log_style modern` or `log_style legacy` (with `format json` or `format text`) | `observability { provider otlp }` |
| Prometheus | N/A (metrics only) | `observability { provider prometheus }` |

> [!note]
> Prometheus is metrics-only — it does not export logs. For log export, configure OTLP or use file/console sinks.

> [!tip]
> If log files are not being written, verify file paths are accessible and the Ferron process has write permissions. For global observability configuration, see [Core directives](/docs/v3/configuration/server/core-directives#observability).

## Admin API structured logs

The admin API emits structured log events through the observability pipeline for important operational events:

| Event | Level | Condition |
|-------|-------|-----------|
| Admin config reload completed | INFO | `POST /reload` succeeds |
| Admin config reload failed | ERROR | `POST /reload` fails |
| Admin config queried | INFO | `GET /config` requested |

## Reload log events

The `metrics-reload` module emits application log events during configuration reloads:

| Level | Message | Trigger |
|-------|---------|---------|
| `INFO` | `Reloading configuration...` | A reload is initiated |
| `WARN` | `Can't reload the server, continuing to run with the previous configuration: {error}` | The reload attempt failed |

These events carry the `ferron-metrics-reload` target and are emitted through the observability event system.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Configuration reload | `INFO` | — |
| Configuration reload error | `WARN` | `error.message` (string) — the reload error message |

### Error log attributes

Structured error logs include contextual attributes to aid troubleshooting:

| Attribute | Description |
|-----------|-------------|
| `error.type` | Error category (e.g. `bad_request`, `timeout`, `tcp_connection_error`, `tcp_tls_handshake_error`) |
| `error.message` | The human-readable error description |
| `client.address` | The client IP address, when available |
| `server.address` | The server IP address, when available |

The `client.address` and `server.address` attributes are included in:

- **Bad request (400) and timeout (408) logs** — emitted when a request is rejected before handler execution.
- **TLS handshake failure logs** — emitted when a TLS connection fails to establish or negotiate a protocol.
- **TCP connection error logs** — emitted when an HTTP/1.x or HTTP/2 connection encounters a transport-level error.
- **Request validation error logs** — emitted for invalid Host headers, malformed URLs, CONNECT path errors, and URL sanitization failures.

> [!note]
> Connection-level errors that occur before the socket address is resolved (e.g. accept failures, PROXY protocol errors) do not include IP attributes.

## Trace ID in console and file logs

Console and file loggers prefix log messages with `[trace=<trace_id>]` when a trace context is available. This enables grep-based filtering by trace ID without requiring an OTLP backend.

**Example log output:**

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

> [!tip]
> Trace context is always enabled, so trace IDs are automatically available in console and file log messages as `[trace=<trace_id>]` prefixes. No additional configuration is needed.
