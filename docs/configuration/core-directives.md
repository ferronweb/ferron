---
title: "Configuration: core directives"
description: "Top-level directives for runtime, networking, admin API, observability, and reverse proxy connection limits."
---

This page documents directives that belong in top-level global blocks:

```ferron
{
    # global directives here
}
```

## Directives

### Default ports

- `default_http_port <port: integer | false>`
  - This directive specifies the default HTTP port when no port is specified in a host block. Must be a positive integer ≤ 65535, or `false` to disable the default HTTP listener entirely. Default: `default_http_port 80`
- `default_https_port <port: integer | false>`
  - This directive specifies the default HTTPS port used for HTTP-to-HTTPS redirects and URL generation. Must be a positive integer ≤ 65535, or `false` to disable the default HTTPS listener entirely. Default: `default_https_port 443`

**Configuration example:**

```ferron
{
    default_http_port 8080
    default_https_port 8443
}
```

Notes:

- When no explicit port is specified for a host, Ferron starts both an HTTP listener on `default_http_port` and an HTTPS listener on `default_https_port`.
- The redirect stage constructs `https://` URLs using this port (omitting it when the value is `443`).
- Setting `default_http_port false` disables the automatic HTTP listener for hosts without explicit ports.
- Setting `default_https_port false` disables the automatic HTTPS listener and HTTP-to-HTTPS redirects for hosts without explicit ports.
- If **both** directives are set to `false`, host blocks without explicit ports will not create any listeners and a warning is logged.

**Disable default HTTP listener (HTTPS only):**

```ferron
{
    default_http_port false
}
```

**Disable both default listeners (only explicit ports work):**

```ferron
{
    default_http_port false
    default_https_port false
}
```

### Runtime

- `io_uring <bool>`
  - This directive specifies whether `io_uring` is enabled for the primary runtime when available. If initialization fails, Ferron falls back to epoll and logs a warning. Default: `io_uring true`

**Configuration example:**

```ferron
{
    runtime {
        io_uring true
    }
}
```

### Network and listeners

- `listen <address: string>`
  - This directive specifies the listener bind address for HTTP TCP listeners. Accepts either an IP address or a full socket address. If a socket address is used, its port must match the HTTP port being started. Default: `[::]:<http-port>`
- `send_buf <size: integer>`
  - This directive specifies the TCP send buffer size. Must resolve to a non-negative integer at runtime. Default: OS default
- `recv_buf <size: integer>`
  - This directive specifies the TCP receive buffer size. Must resolve to a non-negative integer at runtime. Default: OS default

**Configuration example:**

```ferron
{
    tcp {
        listen "127.0.0.1"
        send_buf 65536
        recv_buf 131072
    }
}
```

### PROXY protocol

- `protocol_proxy <bool>`
  - This directive specifies whether PROXY protocol v1/v2 parsing is enabled for incoming TCP connections. When enabled, Ferron reads the PROXY protocol header from HAProxy or similar load balancers before processing the HTTP request. The client and server addresses from the PROXY header replace the actual socket addresses for the duration of the connection. Default: `protocol_proxy false`

Notes:

- Supports both PROXY protocol v1 (text-based) and v2 (binary).
- If parsing fails, the connection is rejected with an error logged.
- This is a global directive and applies to all TCP listeners.

### Reverse proxy connection limits

- `concurrent_conns <limit: integer>`
  - This directive specifies the global maximum number of concurrent TCP connections maintained in the reverse proxy keep-alive connection pool. The limit is shared across all hosts that use the `proxy` directive. Unix socket connections are always unbounded. Default: `concurrent_conns 16384`

**Configuration example:**

```ferron
{
    concurrent_conns 10000
}
```

Notes:

- The connection pool is created lazily on the first request that needs it, reading this value at creation time.
- Per-upstream `limit` directives inside `proxy` blocks further restrict connections to individual backends.

### Admin API

The `admin` block configures the built-in administration endpoints. If the `admin` block is absent, the admin API is **disabled** entirely.

- `listen <address: string>` (`admin-api`)
  - This directive specifies the socket address for the admin HTTP listener. Default: `listen 127.0.0.1:8081`
- `health <bool>` (`admin-api`)
  - This directive specifies whether the `GET /health` endpoint is enabled. Returns `200 OK` or `503 Service Unavailable` during shutdown. Default: `health true`
- `status <bool>` (`admin-api`)
  - This directive specifies whether the `GET /status` endpoint is enabled. Returns JSON with uptime, active connections, request count, and reload count. Default: `status true`
- `config <bool>` (`admin-api`)
  - This directive specifies whether the `GET /config` endpoint is enabled. Returns the current effective configuration as sanitized JSON (sensitive fields redacted). Default: `config true`
- `reload <bool>` (`admin-api`)
  - This directive specifies whether the `POST /reload` endpoint is enabled. Triggers a configuration reload equivalent to SIGHUP. Default: `reload true`

**Configuration example:**

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

Notes:

- All endpoint flags accept `true` or `false`. A bare directive without a value (e.g. `health`) counts as enabled.
- The admin listener runs on a separate secondary Tokio runtime, isolated from the primary data-plane runtime.
- The `/config` endpoint redacts these sensitive directive names: `key`, `cert`, `private_key`, `password`, `secret`, `token`, `ticket_keys`.

#### `GET /health`

Returns `200 OK` while the server is running, or `503 Service Unavailable` when a shutdown has been initiated. Suitable for load balancer and orchestration health checks.

#### `GET /status`

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

#### `GET /config`

Returns the full effective server configuration as sanitized JSON. Sensitive directives (TLS keys, passwords, tokens) are replaced with `"[redacted]"`. Useful for debugging and auditing.

#### `POST /reload`

Triggers a configuration reload, equivalent to sending `SIGHUP` to the daemon process. Returns `{"status": "reload_initiated"}`.

### Observability

The `observability` block configures per-host event sinks for logging and metrics. Multiple `observability` directives for the same host accumulate event sinks.

- `provider <name: string>` (`observability-consolelog`, `observability-logfile`)
  - This directive specifies the observability provider name. Required when observability is enabled through the block form. Supported providers: `console` (`observability-consolelog`), `file` (`observability-logfile`). Default: none

**Configuration example:**

```ferron
example.com {
    observability true {
        provider console
    }
}
```

#### `provider console`

The bundled `console` provider (`observability-consolelog`) takes no additional subdirectives and writes supported observability events to Ferron's logs.

#### `provider file`

The bundled `file` provider (`observability-logfile`) writes observability events to specified log files.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `access_log` | `<string>` | File path for access log output. | none |
| `error_log` | `<string>` | File path for error log output. | none |
| `format` | `<string>` | Optional log formatter name. | none (default formatting) |
| `access_log_rotate_size` | `<number>` | Maximum access log file size in bytes before rotation. | disabled |
| `access_log_rotate_keep` | `<number>` | Number of rotated access log files to keep. | none (no limit) |
| `error_log_rotate_size` | `<number>` | Maximum error log file size in bytes before rotation. | disabled |
| `error_log_rotate_keep` | `<number>` | Number of rotated error log files to keep. | none (no limit) |

**Configuration example:**

```ferron
example.com {
    observability true {
        provider file {
            access_log "/var/log/ferron/access.log"
            error_log "/var/log/ferron/error.log"
            format "combined"
        }
    }
}
```

Notes:

- Log files are created if they don't exist and opened in append mode.
- Writes are buffered and flushed periodically (every 1 second) and on shutdown.
- If `access_log` is omitted, access events are ignored. Same applies for `error_log`.
- When rotation is enabled, the current log file is renamed to `<filename>.1`, existing rotated files are shifted up, and a new empty log file is created.
- If `access_log_rotate_keep` (or `error_log_rotate_keep`) is set to `0`, the log file is deleted on rotation instead of being renamed.

## Observability aliases

Ferron provides shorthand directives for common observability configurations. These are automatically transformed into equivalent `observability` blocks.

### `log`

The `log` directive is shorthand for configuring access logging with the `file` provider.

```ferron
# These are equivalent:

log "/var/log/access.log" {
    format "combined"
}

observability {
    provider file
    access_log "/var/log/access.log"
    format "combined"
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

The `error_log` directive is shorthand for configuring error logging with the `file` provider.

```ferron
# These are equivalent:

error_log "/var/log/error.log"

observability {
    provider file
    error_log "/var/log/error.log"
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

### `console_log`

The `console_log` directive is shorthand for configuring console-based observability.

```ferron
# These are equivalent:

console_log {
    format "json"
}

observability {
    provider console
    format "json"
}
```

## Notes and troubleshooting

- These directives affect startup and listener construction, not per-request routing.
- The built-in blank configuration enables `runtime.io_uring true`.
- During configuration reload (SIGHUP), the existing admin listener is gracefully shut down and a new one is started if the `admin` block is still present.
- Configuration file parsing is handled by the `config-ferronconf` module (for `.conf` files) or `config-json` module (for `.json` files).
- For observability-specific configuration (log formatters, OTLP export), see [Observability and logging](/docs/v3/configuration/observability-logging).
- For per-host HTTP settings, see [HTTP host directives](/docs/v3/configuration/http-host).
