---
title: "Configuration: logging"
description: "Access logs, application logs, log formats, log fields, and trace IDs in log output."
---

Ferron writes two log signals:

| Signal           | Records                                                            |
| ---------------- | ------------------------------------------------------------------ |
| Access logs      | One entry per HTTP request: method, path, status, duration, and similar fields |
| Application logs | Server messages: startup, configuration reloads, errors, debug output |

Configure access logs per host with the `log` directive. Configure application logs with `console_log` and `error_log`, or with an `observability` block that uses `provider console` or `provider file`.

There is no separate error log signal. The `error_log` directive is the file sink for the application log signal.

## Access logs

```ferron
example.com {
    log access.log {
        format json
        fields method path status duration_secs
    }
}
```

| Directive | Arguments     | Description                                                          | Default    |
| --------- | ------------- | -------------------------------------------------------------------- | ---------- |
| `format`  | `<string>`    | Formatter name. Available formatters depend on the loaded modules.   | none       |
| `fields`  | `<string>...` | Fields to include. When you omit this, the server emits all fields.  | all fields |

> [!tip]
> If Ferron does not write log files, check the file path and the write permissions of the Ferron process. For global settings, see [Core directives](/docs/configuration/server/core-directives#observability).

### Access log fields

| Field                 | Description                                              |
| --------------------- | -------------------------------------------------------- |
| `path`                | Request URI path, for example `/index.html`              |
| `path_and_query`      | Request URI with path and query                          |
| `method`              | HTTP request method, for example `GET` or `POST`         |
| `version`             | HTTP version, for example `HTTP/1.1` or `HTTP/2.0`       |
| `scheme`              | Request scheme: `http` or `https`                        |
| `client_ip`           | Client IP address                                        |
| `client_port`         | Client port number                                       |
| `client_ip_canonical` | Client IP in canonical form                              |
| `server_ip`           | Server IP address                                        |
| `server_port`         | Server port number                                       |
| `server_ip_canonical` | Server IP in canonical form                              |
| `auth_user`           | Authenticated username, or `-` if not authenticated      |
| `status`              | HTTP response status code                                |
| `content_length`      | Response content length, or `-` if not available         |
| `duration_secs`       | Request processing duration in seconds                   |
| `timestamp`           | Request timestamp in CLF format                          |
| `header_<name>`       | Request header values, one field per header              |
| `res_header_<name>`   | Response header values, one field per header             |
| `span_id`             | Trace span ID, when W3C trace context is available       |
| `trace_id`            | Trace ID, when W3C trace context is available            |

> [!important]
> Access logs omit sensitive fields such as `header_cookie`, `header_authorization`, and `res_header_set_cookie`.

Active pipeline modules can add access log fields. A field appears only when its module handles the request.

## Log formats

### JSON

The `json` formatter writes one JSON object per line. It requires the `logformat-json` module.

```ferron
example.com {
    log access.log {
        format json
    }
}
```

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

Use `fields` to limit the output. Without `fields`, the server emits all available fields.

### Text

The `text` formatter writes a plain text line from a pattern. It requires the `logformat-text` module. The default pattern is Enhanced Combined Log Format (ECLF): CLF with the `Host` header and trace ID fields.

```ferron
example.com {
    log access.log {
        format text
    }
}
```

```text
127.0.0.1 - frank [05/Apr/2026:14:32:01 +0200] "GET /index.html HTTP/1.1" 200 1234 "http://www.example.com/start.html" "Mozilla/5.0" "www.example.com" "abc123def456"
```

### Format strings

The `access_pattern` directive sets the text pattern. Common patterns:

**Enhanced Combined Log Format (Ferron default):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length "%{Referer}i" "%{User-Agent}i" "%{Host}i" "%trace_id"
```

**Combined Log Format (Apache and Nginx standard):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length "%{Referer}i" "%{User-Agent}i"
```

**Common Log Format (CLF):**

```text
%client_ip - %auth_user [%t] "%method %path_and_query %version" %status %content_length
```

### Pattern syntax

| Token             | Description                                        | Example                            |
| ----------------- | -------------------------------------------------- | ---------------------------------- |
| `%field_name`     | Access log field                                   | `%client_ip`, `%status`, `%method` |
| `%{Header-Name}i` | Request header                                     | `%{Referer}i`, `%{User-Agent}i`    |
| `%{Header-Name}o` | Response header                                    | `%{Content-Type}o`                 |
| `%{format}t`      | Timestamp with custom format                       | `%{%Y-%m-%d %H:%M:%S}t`            |
| `%t`              | Timestamp with `timestamp_format` or the CLF default | `%t`                             |
| `%%`              | Literal `%` character                              | `%%`                               |
| Other text        | Copied unchanged                                   | `"`, `-`                           |

Header names are case-insensitive. Ferron converts hyphens to underscores internally. Use `%{Header-Name}o` for response headers.

## Application log formats

The `error_format` directive sets the format of application log messages. It works in the `observability { provider file ... }` block and in the `error_log` shorthand block. Supported values are `text` (default) and `json`.

```ferron
example.com {
    error_log /var/log/ferron/error.log {
        error_format json
    }
}
```

Text example:

```text
[2026-04-05 14:32:01.123 INFO] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] Cache miss for key: user:123
[2026-04-05 14:32:01.125 ERROR] [trace=abc123def456] Upstream connection refused
```

JSON example:

```json
{"timestamp":1781327817042,"summary":"Request processed successfully","level":"INFO","target":"ferron::http","attributes":{},"trace_context":null}
{"timestamp":1781327818364,"summary":"Upstream connection refused","level":"ERROR","target":"ferron::proxy","attributes":{"upstream":"http://10.0.0.1:3000"},"trace_context":{"trace_id":"abc123def456","span_id":"789012345678","sampled":true}}
```

| Field           | Description                                                     |
| --------------- | --------------------------------------------------------------- |
| `timestamp`     | Unix timestamp in milliseconds                                  |
| `summary`       | Log message summary                                             |
| `level`         | Severity level: `ERROR`, `WARN`, `INFO`, or `DEBUG`             |
| `target`        | Module target that emitted the log                              |
| `attributes`    | Typed key-value pairs attached to the event                     |
| `trace_context` | W3C trace context (`trace_id`, `span_id`, `sampled`), or `null` |

> [!note]
> `error_format` applies to the `file` provider and the `error_log` shorthand. Console logs use native formatting based on the log level.

## Admin API logs

The admin API emits structured log events for these operations:

| Event                         | Level | Condition               |
| ----------------------------- | ----- | ----------------------- |
| Admin config reload completed | INFO  | `POST /reload` succeeds |
| Admin config reload failed    | ERROR | `POST /reload` fails    |
| Admin config queried          | INFO  | `GET /config` requested |

## Reload logs

The `metrics-reload` module emits application log events during configuration reloads:

| Level  | Message                                                                               | Trigger                   |
| ------ | ------------------------------------------------------------------------------------- | ------------------------- |
| `INFO` | `Reloading configuration...`                                                          | A reload starts           |
| `WARN` | `Can't reload the server, continuing to run with the previous configuration: {error}` | The reload attempt failed |

These events use the `ferron-metrics-reload` target.

### Structured reload events

| Summary                    | Level  | Attributes                                         |
| -------------------------- | ------ | -------------------------------------------------- |
| Configuration reload       | `INFO` | none                                               |
| Configuration reload error | `WARN` | `error.message` (string): the reload error message |

### Error log attributes

The server attaches these attributes to structured error logs:

| Attribute        | Description                                            |
| ---------------- | ------------------------------------------------------ |
| `error.type`     | Error category                                         |
| `error.message`  | Human-readable error description                       |
| `client.address` | Client IP address, when available                      |
| `client.port`    | Client port, when available                            |
| `server.address` | Server IP or Unix socket address, when available       |
| `server.port`    | Server port, when available                            |

`error.type` values include `bad_request`, `timeout`, `tcp_connection_error`, `tcp_tls_handshake_error`, `tcp_stream_error`, and `quic_stream_error`.

Ferron includes `client.address` and `server.address` in:

- Bad request (400) and timeout (408) logs. These requests fail before handler execution.
- TLS handshake failure logs, when a TLS connection fails to establish or negotiate a protocol.
- TCP connection error logs, when an HTTP/1.x or HTTP/2 connection has a transport-level error.
- HTTP/2 stream error logs, with `error.type` `tcp_stream_error` or `unix_stream_error`.
- HTTP/3 stream error logs, with `error.type` `quic_stream_error`.
- Request validation error logs for invalid Host headers, malformed URLs, CONNECT path errors, and URL sanitization failures.

Connection-level errors, such as accept failures and PROXY protocol errors, do not include IP attributes. These errors occur before the server resolves the socket address.

## Log filenames

The `log` and `error_log` directives support `{{variable}}` interpolation in file paths. Use it for per-host access logs or per-target error logs.

```ferron
example.com {
    log "/var/log/ferron/{{accesslog.header_host}}/access.log"
}
```

### Access log variables

Access log fields are available as variables with the `accesslog.` prefix.

| Variable                        | Description                                                 |
| ------------------------------- | ----------------------------------------------------------- |
| `accesslog.path`                | Request URI path                                            |
| `accesslog.path_and_query`      | Request URI with path and query                             |
| `accesslog.method`              | HTTP request method                                         |
| `accesslog.version`             | HTTP version                                                |
| `accesslog.scheme`              | Request scheme: `http` or `https`                           |
| `accesslog.client_ip`           | Client IP address                                           |
| `accesslog.client_port`         | Client port number                                          |
| `accesslog.client_ip_canonical` | Client IP in canonical form                                 |
| `accesslog.server_ip`           | Server IP address                                           |
| `accesslog.server_port`         | Server port number                                          |
| `accesslog.server_ip_canonical` | Server IP in canonical form                                 |
| `accesslog.auth_user`           | Authenticated username, or `-`                              |
| `accesslog.status`              | HTTP response status code                                   |
| `accesslog.content_length`      | Response content length, or `-`                             |
| `accesslog.duration_secs`       | Request processing duration in seconds                      |
| `accesslog.timestamp`           | Request timestamp in CLF format                             |
| `accesslog.header_<name>`       | Request header values. Lowercase, hyphens replaced with underscores. |
| `accesslog.res_header_<name>`   | Response header values. Lowercase, hyphens replaced with underscores. |
| `accesslog.trace_id`            | Trace ID, when W3C trace context is available               |
| `accesslog.span_id`             | Trace span ID, when W3C trace context is available          |

> [!important]
> Filename interpolation omits sensitive fields such as `header_cookie` and `header_authorization`.

Active pipeline modules can add fields. These fields are available as `accesslog.<field_name>` when the module handles the request.

### Application log variables

Application log event fields are available as variables with the `log.` prefix.

| Variable          | Description                                                |
| ----------------- | ---------------------------------------------------------- |
| `log.level`       | Severity level: `ERROR`, `WARN`, `INFO`, or `DEBUG`        |
| `log.target`      | Module target that emitted the log                         |
| `log.message`     | Full-text log message                                      |
| `log.summary`     | Short summary used by OTLP `log_style modern`              |
| `log.trace_id`    | Trace ID, when W3C trace context is available              |
| `log.span_id`     | Trace span ID, when W3C trace context is available         |
| `log.<attribute>` | Structured attribute attached to the log event             |

Common attribute keys include `error.type`, `error.message`, `client.address`, `server.address`, and `upstream.address`. The available keys depend on the log event source.

### Environment variables

Use environment variables in log filenames with the `env.` prefix:

```ferron
example.com {
    log "/var/log/ferron/{{env.CUSTOMER_NAME}}/access.log"
}
```

> [!warning]
> High-cardinality variables such as `{{accesslog.client_ip}}` or `{{accesslog.timestamp}}` create one file per unique value. This can exhaust open file handles. Use variables with bounded value sets, such as `{{accesslog.header_host}}` or `{{log.level}}`.

## Trace IDs in logs

Console and file loggers prefix each message with `[trace=<trace_id>]` when a trace context exists. You can filter logs by trace ID without an OTLP backend.

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

> [!tip]
> Ferron always enables trace context. Trace IDs appear in console and file logs without extra configuration.

## See also

- [Tracing](/docs/configuration/observability/tracing): trace context propagation, baggage, and sampling
- [OTLP observability](/docs/configuration/observability/otlp): export logs over OTLP
- [Core directives](/docs/configuration/server/core-directives#observability): global observability settings
