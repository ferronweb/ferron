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

> [!note]
>
> - These directives affect startup and listener construction, not per-request routing.
> - Configuration file parsing is handled by the `config-ferronconf` module (for `.conf` files) or `config-json` module (for `.json` files).

> [!info]
> For observability-specific configuration, see [Observability and logging](/docs/v3/configuration/observability/logging). For per-host HTTP settings, see [HTTP host directives](/docs/v3/configuration/server/host). For admin API security hardening, see [Security considerations](#security-considerations).

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

> [!note]
>
> - When no explicit port is specified for a host, Ferron starts both an HTTP listener on `default_http_port` and an HTTPS listener on `default_https_port`.
> - The redirect stage constructs `https://` URLs using this port (omitting it when the value is `443`).
> - Setting `default_http_port false` disables the automatic HTTP listener for hosts without explicit ports.
> - Setting `default_https_port false` disables the automatic HTTPS listener and HTTP-to-HTTPS redirects for hosts without explicit ports.
> - If **both** directives are set to `false`, host blocks without explicit ports will not create any listeners and a warning is logged.

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
- `backlog <size: integer>`
  - This directive specifies the maximum number of pending connections allowed on the listener socket. Default: `-1` (unlimited)

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

- `protocol_proxy [bool]`
  - This directive specifies whether PROXY protocol v1/v2 parsing is enabled for incoming TCP connections. When enabled, Ferron reads the PROXY protocol header from HAProxy or similar load balancers before processing the HTTP request. The client and server addresses from the PROXY header replace the actual socket addresses for the duration of the connection. Default: `protocol_proxy false`

> [!note]
> Ferron supports both PROXY protocol v1 (text-based) and v2 (binary). If parsing fails, the connection is rejected with an error logged.

### Reverse proxy connection limits

- `concurrent_conns <limit: integer>`
  - This directive specifies the global maximum number of concurrent TCP connections maintained in the reverse proxy keep-alive connection pool. The limit is shared across all hosts that use the `proxy` directive. Unix socket connections are always unbounded. Default: `concurrent_conns 16384`

**Configuration example:**

```ferron
{
    concurrent_conns 10000
}
```

### Admin API

The `admin` block configures the built-in administration endpoints. If the `admin` block is absent, the admin API is **disabled** entirely.

- `listen <address: string>` (`admin-api`)
  - This directive specifies the socket address for the admin HTTP listener. Default: `listen 127.0.0.1:8081`
- `health [bool]` (`admin-api`)
  - This directive specifies whether the `GET /health` endpoint is enabled. Returns `200 OK` or `503 Service Unavailable` during shutdown. Default: `health true`
- `status [bool]` (`admin-api`)
  - This directive specifies whether the `GET /status` endpoint is enabled. Returns JSON with uptime, active connections, request count, and reload count. Default: `status true`
- `config [bool]` (`admin-api`)
  - This directive specifies whether the `GET /config` endpoint is enabled. Returns the current effective configuration as sanitized JSON (sensitive fields redacted). Default: `config true`
- `reload [bool]` (`admin-api`)
  - This directive specifies whether the `POST /reload` endpoint is enabled. Triggers a configuration reload equivalent to SIGHUP. Default: `reload true`
- `reload_get [bool]` (`admin-api`)
  - This directive specifies whether the `GET /reload` endpoint is enabled. Returns the current reload status. Default: `reload_get true`
- `runtime [bool]` (`admin-api`)
  - This directive specifies whether the `GET /runtime` endpoint is enabled. Returns runtime information such as thread count and io_uring status. Default: `runtime true`

**Configuration example:**

```ferron
{
    admin {
        listen "127.0.0.1:8081"

        health true
        status true
        config true
        reload true
        reload_get true
        runtime true
    }
}
```

> [!note]
> The `/config` endpoint redacts sensitive directive names, such as: `key`, `cert`, `private_key`, `password`, `secret`, `token`, `ticket_keys`, `bearer`, `passwd`, `htpasswd`.

### Observability

The `observability` block configures per-host event sinks for logging and metrics. Multiple `observability` directives for the same host accumulate event sinks.

- `provider <name: string>` (`observability-consolelog`, `observability-logfile`)
  - This directive specifies the observability provider name. Required when observability is enabled through the block form. Supported providers: `console` (`observability-consolelog`), `file` (`observability-logfile`). Default: none

**Configuration example:**

```ferron
example.com {
    observability {
        provider console
    }
}
```

#### `provider console`

The bundled `console` provider (`observability-consolelog`) takes no additional subdirectives and writes supported observability events to Ferron's logs.

#### `provider file`

The bundled `file` provider (`observability-logfile`) writes observability events to specified log files.

| Addtional subdirective | Arguments | Description | Default |
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
    observability {
        provider file

        access_log /var/log/ferron/access.log
        error_log /var/log/ferron/error.log
        format text
    }
}
```

> [!note]
>
> - Log files are created if they don't exist and opened in append mode.
> - Writes are buffered and flushed periodically (every 1 second) and on shutdown.
> - If `access_log` is omitted, access events are ignored. Same applies for `error_log`.
> - When rotation is enabled, the current log file is renamed to `<filename>.1`, existing rotated files are shifted up, and a new empty log file is created.
> - If `access_log_rotate_keep` (or `error_log_rotate_keep`) is set to `0`, the log file is deleted on rotation instead of being renamed.

## Observability aliases

Ferron provides shorthand directives for common observability configurations. These are automatically transformed into equivalent `observability` blocks.

### `log`

The `log` directive is shorthand for configuring access logging with the `file` provider.

```ferron
example.com {
    # These are equivalent:

    log /var/log/access.log {
        format text
    }

    observability {
        provider file
        access_log /var/log/access.log
        format text
    }
}
```

Examples:

```ferron
example.com {
    # Enable access logging with default format
    log /var/log/access.log

    # Enable with custom format
    log /var/log/access.log {
        format json
    }

    # Enable with log rotation (100MB max, keep 5 rotated files)
    log /var/log/access.log {
        access_log_rotate_size 104857600
        access_log_rotate_keep 5
    }

    # Disable access logging
    log false
}
```

### `error_log`

The `error_log` directive is shorthand for configuring error logging with the `file` provider.

```ferron
example.com {
    # These are equivalent:

    error_log /var/log/error.log

    observability {
        provider file
        error_log /var/log/error.log
    }
}
```

Examples:

```ferron
example.com {
    # Enable error logging
    error_log /var/log/error.log

    # Enable with log rotation (50MB max, keep 3 rotated files)
    error_log /var/log/error.log {
        error_log_rotate_size 52428800
        error_log_rotate_keep 3
    }

    # Disable error logging
    error_log false
}
```

### `console_log`

The `console_log` directive is shorthand for configuring console-based observability.

```ferron
example.com {
    # These are equivalent:

    console_log {
        format json
    }

    observability {
        provider console
        format json
    }
}
```

## Admin API

The admin API provides a built-in HTTP interface for server health checks, status monitoring, configuration inspection, and reload control. It is designed for local access and debugging purposes.

### Security considerations

The admin API is a **privileged control plane** that provides full server configuration access and reload capability. It is **not encrypted**, has **no authentication**, and **no access control** by default. Treat it with the same security posture as a root shell on your server.

#### Current limitations

| Feature | Status | Notes |
|---------|--------|-------|
| TLS / HTTPS | Not supported | The admin listener accepts plain HTTP only. No TLS configuration options are available. |
| Authentication | Not supported | No username/password, API key, or token mechanism. Any client that can reach the listener has full administrative access. |
| ACL / allowlists | Not supported | No built-in IP address filtering or access restrictions. |

#### Risks of binding to `0.0.0.0`

Setting `listen "0.0.0.0:<port>"` (or omitting the bind address to default to all interfaces) makes the admin API **completely open to any client that can reach the host**. This can happen accidentally in containerized environments (e.g., Docker with bridge networking) or misconfigured networks.

Consequences of an open admin API:

- **Denial of service**: Anyone can send `POST /reload` continuously, causing configuration reload loops that degrade performance.
- **Configuration leak**: Anyone can send `GET /config` to retrieve the full server configuration. While sensitive values (TLS keys, passwords, tokens) are redacted, the structure reveals hostnames, upstream addresses, routing rules, and other operational details.
- **Service disruption**: Any endpoint can be disabled via reload with modified configuration, or misconfigured directives can be injected.

#### Hardening recommendations

1. **Always bind to localhost** unless you have a specific, secure reason not to:

   ```ferron
   {
       admin {
           listen "127.0.0.1:8081"
           health true
           status true
           config true
           reload true
       }
   }
   ```

2. **Disable unnecessary endpoints**. Only enable the endpoints you need:

   ```ferron
   {
       admin {
           listen "127.0.0.1:8081"
           health true
           status false
           config false
           reload true
       }
   }
   ```

3. **Use a reverse proxy for remote access**. If you need to access the admin API from a remote machine, front it with an authenticating reverse proxy rather than binding to `0.0.0.0`:

   ```text
   Remote user → reverse proxy (auth required) → 127.0.0.1:8081 (admin API)
   ```

4. **Restrict network access at the infrastructure level**. Use firewall rules, security groups, or VPC networking to ensure only trusted hosts can reach the admin port.

5. **Monitor admin API access**. Use your observability sinks to track requests to admin endpoints for anomaly detection.

6. **Never expose the admin API to the public internet**. If you need remote administration, use SSH tunneling:

   ```bash
   ssh -L 8081:127.0.0.1:8081 admin@your-server
   # Then access http://127.0.0.1:8081 locally
   ```

### API reference

The admin API provides a RESTful interface for server configuration and control. Below are the available endpoints:

#### `GET /health`

Returns `200 OK` while the server is running, or `503 Service Unavailable` when a shutdown has been initiated. Suitable for load balancer and orchestration health checks.

#### `GET /status`

Returns JSON with server metrics:

```json
{
  "uptime_sec": 12345,
  "connections_active": 42,
  "requests_total": 100000,
  "reloads": 3,
  "observability_events_dropped": 0,
  "observability_event_queue_len": 0
}
```

| Field | Description |
| --- | --- |
| `uptime_sec` | Seconds since the server started. |
| `connections_active` | Currently open TCP connections across all HTTP listeners. |
| `requests_total` | Total HTTP requests served across all listeners. |
| `reloads` | Number of configuration reloads performed. |
| `observability_events_dropped` | Total number of observability events dropped due to backpressure. |
| `observability_event_queue_len` | Approximate current length of the observability event queue. |

#### `GET /config`

Returns the full effective server configuration as sanitized JSON. Sensitive directives (TLS keys, passwords, tokens) are replaced with `"[redacted]"`. Useful for debugging and auditing.

#### `GET /reload`

Returns the current reload status as JSON:

```json
{
  "last_reload_time": "2026-05-29T12:00:00Z",
  "last_reload_error": null,
  "active_generation": 42
}
```

| Field | Description |
| --- | --- |
| `last_reload_time` | ISO 8601 timestamp of the last reload attempt. |
| `last_reload_error` | Error message from the last reload, or `null` if successful. |
| `active_generation` | The configuration generation number currently in effect. |

#### `POST /reload`

Triggers a configuration reload, equivalent to sending `SIGHUP` to the daemon process. Returns `{"status": "reload_initiated"}`.

#### `GET /runtime`

Returns the runtime status as JSON:

```json
{
  "primary_threads": 8,
  "io_uring_supported": true,
  "io_uring_runtime_enabled": true
}
```

| Field | Description |
| --- | --- |
| `primary_threads` | Number of primary threads (typically equal to CPU count). |
| `io_uring_supported` | Whether `io_uring` is supported on the current system. |
| `io_uring_runtime_enabled` | Whether `io_uring` was successfully enabled at runtime. |

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

### Log rotation

- **`log` without rotation** — File-based access logging should include `access_log_rotate_size` (or an external log rotation policy) to prevent unbounded disk growth.
- **`error_log` without rotation** — File-based error logging should include `error_log_rotate_size` (or an external log rotation policy).

### Default ports

- **Both default ports disabled** — Setting `default_http_port false` and `default_https_port false` means host blocks without explicit ports create no listeners. Ensure all host blocks specify explicit ports, or keep at least one default listener enabled.

### PROXY protocol

- **`protocol_proxy` enabled** — PROXY protocol trusts client-provided addresses. Enable it only on listeners reachable exclusively by trusted load balancers.

### Admin API

- **`admin.listen` on non-loopback address** — The admin API is unauthenticated and unencrypted. Bind to a loopback address or restrict access via network controls.

### Location blocks

- **No duplicate `location` block pathnames** — Duplicate pathnames in location blocks will cause the server to return an ambiguous response, so they should be avoided.
