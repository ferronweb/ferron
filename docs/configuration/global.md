# Global Directives

These directives belong in top-level global blocks:

```ferron
{
    # global directives here
}
```

## Categories

- Runtime: `runtime`
- Network/listener defaults: `tcp`
- Default ports: `default_http_port`, `default_https_port`
- Admin API: `admin`
- PROXY protocol: `protocol_proxy`
- Observability: `observability`, `log`, `error_log`, `console_log`

## `default_http_port`

Syntax:

```ferron
{
    default_http_port 8080
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<number>` | Default HTTP port when no port is specified in a host block. Must be a positive integer ≤ 65535. | `80` |

## `default_https_port`

Syntax:

```ferron
{
    default_https_port 8443
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<number>` | Default HTTPS port used for HTTP-to-HTTPS redirects and URL generation. Must be a positive integer ≤ 65535. | `443` |

Notes:

- When no explicit port is specified for a host, Titanium starts both an HTTP listener on `default_http_port` and an HTTPS listener on `default_https_port`.
- This port is exposed to the HTTP pipeline via `ctx.https_port` and is used by the built-in HTTPS redirect stage.
- The redirect stage constructs `https://` URLs using this port (omitting it when the value is `443`).

## `runtime`

Syntax:

```ferron
{
    runtime {
        io_uring true
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `io_uring` | `<bool>` | Enables `io_uring` for the primary runtime when available. If initialization fails, Titanium falls back to epoll and logs a warning. | `true` |

## `tcp`

Syntax:

```ferron
{
    tcp {
        listen "127.0.0.1"
        send_buf 65536
        recv_buf 131072
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `listen` | `<string>` | Listener bind address for HTTP TCP listeners. Accepts either an IP address or a full socket address. If a socket address is used, its port must match the HTTP port being started. | `[::]:<http-port>` |
| `send_buf` | `<number>` | TCP send buffer size. Must resolve to a non-negative integer at runtime. | OS default |
| `recv_buf` | `<number>` | TCP receive buffer size. Must resolve to a non-negative integer at runtime. | OS default |

## `admin`

Syntax:

```ferron
{
    admin {
        listen 127.0.0.1:8081

        health true
        status true
        config true
        reload true
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `listen` | `<string>` | Socket address for the admin HTTP listener. | `127.0.0.1:8081` |
| `health` | `<bool>` | Enables the `GET /health` endpoint. Returns `200 OK` or `503 Service Unavailable` during shutdown. | `true` |
| `status` | `<bool>` | Enables the `GET /status` endpoint. Returns JSON with uptime, active connections, request count, and reload count. | `true` |
| `config` | `<bool>` | Enables the `GET /config` endpoint. Returns the current effective configuration as sanitized JSON (sensitive fields redacted). | `true` |
| `reload` | `<bool>` | Enables the `POST /reload` endpoint. Triggers a configuration reload equivalent to SIGHUP. | `true` |

Notes:

- If the `admin` block is absent, the admin API is **disabled** entirely.
- All endpoint flags accept `true` or `false`. A bare directive without a value (e.g. `health`) counts as enabled.
- The admin listener runs on a separate secondary Tokio runtime, isolated from the primary data-plane runtime.
- The `/config` endpoint redacts these sensitive directive names: `key`, `cert`, `private_key`, `password`, `secret`, `token`, `ticket_keys`.
- During configuration reload (SIGHUP), the existing admin listener is gracefully shut down and a new one is started if the `admin` block is still present.

### `GET /health`

Returns `200 OK` while the server is running, or `503 Service Unavailable` when a shutdown has been initiated. Suitable for load balancer and orchestration health checks.

### `GET /status`

Returns JSON with server metrics:

```json
{
  "uptime_sec": 12345,
  "connections_active": 42,
  "requests_total": 100000,
  "reloads": 3
}
```

| Field | Description |
| --- | --- |
| `uptime_sec` | Seconds since the server started. |
| `connections_active` | Currently open TCP connections across all HTTP listeners. |
| `requests_total` | Total HTTP requests served across all listeners. |
| `reloads` | Number of configuration reloads performed. |

### `GET /config`

Returns the full effective server configuration as sanitized JSON. Sensitive directives (TLS keys, passwords, tokens) are replaced with `"[redacted]"`. Useful for debugging and auditing.

### `POST /reload`

Triggers a configuration reload, equivalent to sending `SIGHUP` to the daemon process. Returns `{"status": "reload_initiated"}`.

## `protocol_proxy`

Syntax:

```ferron
{
    protocol_proxy true
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<bool>` | Enables PROXY protocol v1/v2 parsing for incoming TCP connections. When enabled, Titanium reads the PROXY protocol header from HAProxy or similar load balancers before processing the HTTP request. The client and server addresses from the PROXY header replace the actual socket addresses for the duration of the connection. | `false` |

Notes:

- Supports both PROXY protocol v1 (text-based) and v2 (binary).
- If parsing fails, the connection is rejected with an error logged.
- This is a global directive and applies to all TCP listeners.
- Useful when running behind HAProxy, AWS ELB, or other Layer 4 load balancers that forward client IP information via PROXY protocol.

## `observability`

Syntax:

```ferron
example.com {
    observability true {
        provider console
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `<string>` | Observability provider name. Required when observability is enabled through the block form. | none |

Current runtime behavior:

- If `observability` is absent, no host-specific event sink is attached.
- If `observability false { ... }` is used, the block is ignored.
- Multiple `observability` directives for the same host accumulate event sinks.

Bundled provider-specific options:

### `provider console`

The bundled `console` provider takes no additional subdirectives and writes supported observability events to Titanium's logs.

### `provider file`

The bundled `file` provider writes observability events to specified log files.

Syntax:

```ferron
example.com {
    observability true {
        provider file {
        
        access_log "/var/log/ferron/access.log"
        error_log "/var/log/ferron/error.log"
        format "combined"
    }
}
```

| Subdirective | Arguments | Description | Default |
| --- | --- | --- | --- |
| `access_log` | `<string>` | File path for access log output. Access events are written to this file when specified. | none |
| `error_log` | `<string>` | File path for error log output. Log events (error, warn, info, debug) are written to this file with timestamps and severity levels. | none |
| `format` | `<string>` | Optional log formatter name, resolved from the registry. If specified and available, the formatter controls the exact output format of access log entries. | none (falls back to default formatting) |
| `access_log_rotate_size` | `<number>` | Maximum access log file size in bytes before rotation. | disabled |
| `access_log_rotate_keep` | `<number>` | Number of rotated access log files to keep. | none (no limit) |
| `error_log_rotate_size` | `<number>` | Maximum error log file size in bytes before rotation. | disabled |
| `error_log_rotate_keep` | `<number>` | Number of rotated error log files to keep. | none (no limit) |

Notes:

- Log files are created if they don't exist and opened in append mode.
- Writes are buffered and flushed periodically (every 1 second) and on shutdown.
- If `access_log` is omitted, access events are ignored. Same applies for `error_log`.
- If a formatter is specified but not found in the registry, access events are not written (no fallback output).
- When rotation is enabled, the current log file is renamed to `<filename>.1`, existing rotated files are shifted up (`.1` → `.2`, `.2` → `.3`, etc.), and a new empty log file is created.
- If `access_log_rotate_keep` (or `error_log_rotate_keep`) is set to `0`, the log file is deleted on rotation instead of being renamed.

## Observability Aliases

Titanium provides shorthand alias directives for common observability configurations. These aliases are automatically transformed into equivalent `observability` blocks during configuration processing.

### `log`

The `log` directive is a shorthand for configuring access logging with the `file` provider.

Syntax:

```ferron
example.com {
    log "/var/log/ferron/access.log" {
        format "combined"
    }
}
```

| Arguments | Description |
| --- | --- |
| `<string>` | File path for access log output. Required when not using `false`. |
| `false` | Disables access logging. |

| Nested directive | Arguments | Description |
| --- | --- | --- |
| `format` | `<string>` | Optional log formatter name. |
| `access_log_rotate_size` | `<number>` | Maximum access log file size in bytes before rotation. |
| `access_log_rotate_keep` | `<number>` | Number of rotated access log files to keep. |

This is equivalent to:

```ferron
example.com {
    observability {
        provider file
        access_log "/var/log/ferron/access.log"
        format "combined"
    }
}
```

Examples:

```ferron
# Enable access logging with default format
log "/var/log/access.log"

# Enable with custom format
log "/var/log/access.log" {
    format "json"
}

# Enable with log rotation (100MB max, keep 5 rotated files)
log "/var/log/access.log" {
    access_log_rotate_size 104857600
    access_log_rotate_keep 5
}

# Disable access logging
log false
```

### `error_log`

The `error_log` directive is a shorthand for configuring error logging with the `file` provider.

Syntax:

```ferron
example.com {
    error_log "/var/log/ferron/error.log"
}
```

| Arguments | Description |
| --- | --- |
| `<string>` | File path for error log output. Required when not using `false`. |
| `false` | Disables error logging. |

| Nested directive | Arguments | Description |
| --- | --- | --- |
| `error_log_rotate_size` | `<number>` | Maximum error log file size in bytes before rotation. |
| `error_log_rotate_keep` | `<number>` | Number of rotated error log files to keep. |

This is equivalent to:

```ferron
example.com {
    observability {
        provider file
        error_log "/var/log/ferron/error.log"
    }
}
```

Examples:

```ferron
# Enable error logging
error_log "/var/log/error.log"

# Enable with log rotation (50MB max, keep 3 rotated files)
error_log "/var/log/error.log" {
    error_log_rotate_size 52428800
    error_log_rotate_keep 3
}

# Disable error logging
error_log false
```

Notes:

- Error logs include timestamps and severity levels (ERROR, WARN, INFO, DEBUG).

### `console_log`

The `console_log` directive is a shorthand for configuring console-based observability.

Syntax:

```ferron
example.com {
    console_log {
        format "json"
    }
}
```

| Nested directive | Arguments | Description |
| --- | --- | --- |
| `format` | `<string>` | Optional log formatter name. |

This is equivalent to:

```ferron
example.com {
    observability {
        provider console
        format "json"
    }
}
```

Examples:

```ferron
# Enable console logging with default format
console_log

# Enable with custom format
console_log {
    format "json"
}

# Disable console logging
console_log false
```

## Notes

- These directives affect startup and listener construction, not per-request routing.
- The built-in blank configuration enables `runtime.io_uring true`.
