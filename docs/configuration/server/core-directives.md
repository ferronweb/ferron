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
> - The `config-ferronconf` module (for `.conf` files) or the `config-json` module (for `.json` files) handles configuration file parsing.

> [!info]
> For observability-specific configuration, see [Observability and logging](/docs/v3/configuration/observability/logging). For per-host HTTP settings, see [HTTP host directives](/docs/v3/configuration/server/host). For admin API security hardening, see [Security considerations](#security-considerations).

## Directives

### Default ports

- `default_http_port <port: integer | false>`
  - This directive sets the default HTTP port when a host block does not specify a port. The value must be a positive integer ≤ 65535, or `false` to disable the default HTTP listener. Default: `default_http_port 80`

- `default_https_port <port: integer | false>`
  - This directive sets the default HTTPS port used for HTTP-to-HTTPS redirects and URL generation. The value must be a positive integer ≤ 65535, or `false` to disable the default HTTPS listener. Default: `default_https_port 443`

**Configuration example:**

```ferron
{
    default_http_port 8080
    default_https_port 8443
}
```

> [!note]
>
> - When a host does not specify an explicit port, Ferron starts an HTTP listener on `default_http_port` and an HTTPS listener on `default_https_port`.
> - The redirect stage constructs `https://` URLs using this port (omitting it when the value is `443`).
> - Setting `default_http_port false` disables the automatic HTTP listener, and `default_https_port false` disables the automatic HTTPS listener and HTTP-to-HTTPS redirects.
> - If you set **both** directives to `false`, host blocks without explicit ports create no listeners, and Ferron logs a warning.

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
  - This directive turns on `io_uring` for the server when it is available. If initialization fails, Ferron falls back to epoll and logs a warning. Default: enabled

**Configuration example:**

```ferron
{
    runtime {
        io_uring
    }
}
```

### Network and listeners

- `listen <address: string>`
  - This directive sets the bind address for HTTP TCP listeners. It accepts either an IP address or a full socket address. If you use a socket address, its port must match the HTTP port that Ferron starts. Default: `[::]:<http-port>`

- `send_buf <size: integer>`
  - This directive sets the TCP send buffer size. It must resolve to a non-negative integer at runtime. Default: OS default

- `recv_buf <size: integer>`
  - This directive sets the TCP receive buffer size. It must resolve to a non-negative integer at runtime. Default: OS default

- `backlog <size: integer>`
  - This directive sets the maximum number of pending connections allowed on the listener socket. Default: `-1` (unlimited)

- `multipath <bool>`
  - This directive turns on Multipath TCP (MPTCP) for the listener. MPTCP allows a single TCP connection to use multiple network interfaces at the same time, improving throughput and resilience. When enabled, Ferron attempts to create an MPTCP socket. If the kernel lacks MPTCP support or MPTCP is off, Ferron logs a warning and falls back to standard TCP. Default: disabled

**Configuration example:**

```ferron
{
    tcp {
        listen "127.0.0.1"
        send_buf 65536
        recv_buf 131072
        multipath
    }
}
```

### PROXY protocol

- `protocol_proxy [bool]`
  - This directive turns on PROXY protocol v1/v2 parsing for incoming TCP connections. When enabled, Ferron reads the PROXY protocol header from HAProxy or similar load balancers before processing the HTTP request. The client and server addresses from the PROXY header replace the actual socket addresses while the connection is open. Default: `protocol_proxy false`

> [!note]
> Ferron supports both PROXY protocol v1 (text-based) and v2 (binary). If parsing fails, Ferron rejects the connection and logs an error.

### Reverse proxy connection limits

- `concurrent_conns <limit: integer>`
  - This directive sets the global maximum number of concurrent TCP connections maintained in the reverse proxy keep-alive connection pool. All hosts that use the `proxy` directive share the limit. Unix socket connections have no limit. Default: `concurrent_conns 16384`

**Configuration example:**

```ferron
{
    concurrent_conns 10000
}
```

### Admin API

The `admin` block configures the built-in administration endpoints. If the `admin` block is absent, Ferron disables the admin API entirely.

- `listen <address: string>` (`admin-api`)
  - This directive sets the socket address for the admin HTTP listener. Default: `listen 127.0.0.1:8081`

- `auth_token <token: string>` (`admin-api`)
  - This directive sets a bearer token for authenticating admin API requests. When set, clients must send `Authorization: Bearer <token>` header. The `/health` endpoint is always exempt from authentication (required by load balancers and orchestrators). Default: none (authentication disabled)

- `health [bool]` (`admin-api`)
  - This directive enables the `GET /health` endpoint. It returns `200 OK` or `503 Service Unavailable` during shutdown. Default: `health true`

- `status [bool]` (`admin-api`)
  - This directive enables the `GET /status` endpoint. It returns JSON with uptime, active connections, request count, and reload count. Default: `status true`

- `config [bool]` (`admin-api`)
  - This directive enables the `GET /config` endpoint. It returns the current effective configuration as sanitized JSON (sensitive fields redacted). Default: `config true`

- `reload [bool]` (`admin-api`)
  - This directive enables the `POST /reload` endpoint. It triggers a configuration reload equivalent to SIGHUP. Default: `reload true`

- `reload_get [bool]` (`admin-api`)
  - This directive enables the `GET /reload` endpoint. It returns the current reload status. Default: `reload_get true`

- `runtime [bool]` (`admin-api`)
  - This directive enables the `GET /runtime` endpoint. It returns runtime information such as thread count and io_uring status. Default: `runtime true`

**Configuration example:**

```ferron
{
    admin {
        listen "127.0.0.1:8081"
        auth_token "my-secret-token"

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
  - This directive sets the observability provider name. Required when observability is enabled through the block form. Supported providers: `console` (`observability-consolelog`), `file` (`observability-logfile`). Default: none

**Configuration example:**

```ferron
example.com {
    observability {
        provider console
    }
}
```

#### `provider console`

The bundled `console` provider (`observability-consolelog`) takes no additional subdirectives and writes supported observability events to Ferron logs.

#### `provider file`

The bundled `file` provider (`observability-logfile`) writes observability events to specified log files.

| Additional subdirective  | Arguments  | Description                                            | Default         |
| ------------------------ | ---------- | ------------------------------------------------------ | --------------- |
| `access_log`             | `<string>` | File path for access log output.                       | none            |
| `error_log`              | `<string>` | File path for error log output.                        | none            |
| `format`                 | `<string>` | Access log formatter name (`text` or `json`).          | `text`          |
| `error_format`           | `<string>` | Application log formatter name (`text` or `json`).     | `text`          |
| `access_log_rotate_size` | `<number>` | Maximum access log file size in bytes before rotation. | disabled        |
| `access_log_rotate_keep` | `<number>` | Number of rotated access log files to keep.            | none (no limit) |
| `error_log_rotate_size`  | `<number>` | Maximum error log file size in bytes before rotation.  | disabled        |
| `error_log_rotate_keep`  | `<number>` | Number of rotated error log files to keep.             | none (no limit) |

**Configuration example:**

```ferron
example.com {
    observability {
        provider file

        access_log /var/log/ferron/access.log
        error_log /var/log/ferron/error.log
        format text
        error_format json
    }
}
```

> [!note]
>
> - Ferron creates log files in append mode if they do not exist.
> - Ferron buffers writes and flushes them every 1 second and on shutdown.
> - If you omit `access_log`, Ferron ignores access events, and the same applies to `error_log`.
> - With rotation on, Ferron renames the current log file to `<filename>.1`, shifts rotated files up, and creates a new log file.
> - If you set `access_log_rotate_keep` (or `error_log_rotate_keep`) to `0`, Ferron deletes the log file on rotation instead of renaming it.

## Observability aliases

Ferron has shorthand directives for common observability configurations. Ferron transforms these automatically into equivalent `observability` blocks.

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

    # Enable with JSON application log formatting
    error_log /var/log/error.log {
        error_format json
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

The admin API is a built-in HTTP interface for server health checks, status monitoring, configuration inspection, and reload control. It is meant for local access and debugging purposes.

### Security considerations

The admin API is a **privileged control plane** with full server configuration access and reload capability. It has no encryption and no authentication by default. You can enable bearer token authentication with the `auth_token` directive. Treat it with the same security posture as a root shell on your server.

#### Current limitations

| Feature          | Status        | Notes                                                                                   |
| ---------------- | ------------- | --------------------------------------------------------------------------------------- |
| TLS / HTTPS      | Not supported | The admin listener accepts plain HTTP only. No TLS configuration options are available. |
| Authentication   | Supported     | Use `auth_token` to require a bearer token on all endpoints except `/health`.           |
| ACL / allowlists | Not supported | No built-in IP address filtering or access restrictions.                                |

#### Risks of binding to `0.0.0.0`

Setting `listen "0.0.0.0:<port>"` makes the admin API **completely open to any client that can reach the host**. Omitting the bind address defaults to all interfaces and has the same effect. This can happen accidentally in containerized environments (for example, Docker with bridge networking) or misconfigured networks.

Consequences of an open admin API:

- **Denial of service**: Anyone can send `POST /reload` continuously, causing reload loops that degrade performance.
- **Configuration leak**: `GET /config` reveals the full server configuration, including hostnames, upstream addresses, and routing rules, with sensitive values redacted.
- **Service disruption**: Anyone can use reload with modified configuration to disable any endpoint or inject misconfigured directives.

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

3. **Use a reverse proxy for remote access**. If you need to access the admin API from a remote machine, front it with an authenticating reverse proxy. Do not bind to `0.0.0.0`:

   ```text
   Remote user → reverse proxy (auth required) → 127.0.0.1:8081 (admin API)
   ```

4. **Restrict network access at the infrastructure level**. Use firewall rules, security groups, or VPC networking to make sure only trusted hosts can reach the admin port.

5. **Monitor admin API access**. Use your observability sinks to track requests to admin endpoints for anomaly detection.

6. **Never expose the admin API to the public internet**. If you need remote administration, use SSH tunneling:

   ```bash
   ssh -L 8081:127.0.0.1:8081 admin@your-server
   # Then access http://127.0.0.1:8081 locally
   ```

### API reference

The admin API is a RESTful interface for server configuration and control. Below are the available endpoints:

#### `GET /health`

It returns `200 OK` while the server runs, or `503 Service Unavailable` when shutdown starts. Suitable for load balancer and orchestration health checks.

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

| Field                           | Description                                                       |
| ------------------------------- | ----------------------------------------------------------------- |
| `uptime_sec`                    | Seconds since the server started.                                 |
| `connections_active`            | Currently open TCP connections across all HTTP listeners.         |
| `requests_total`                | Total HTTP requests served across all listeners.                  |
| `reloads`                       | Number of configuration reloads.                                  |
| `observability_events_dropped`  | Total number of observability events dropped due to backpressure. |
| `observability_event_queue_len` | Approximate current length of the observability event queue.      |

#### `GET /config`

It returns the full effective server configuration as sanitized JSON. Ferron replaces sensitive directives (TLS keys, passwords, tokens) with `"[redacted]"`. Useful for debugging and auditing.

#### `GET /reload`

Returns the current reload status as JSON:

```json
{
  "last_reload_time": "2026-05-29T12:00:00Z",
  "last_reload_error": null,
  "active_generation": 42
}
```

| Field               | Description                                                  |
| ------------------- | ------------------------------------------------------------ |
| `last_reload_time`  | ISO 8601 timestamp of the last reload attempt.               |
| `last_reload_error` | Error message from the last reload, or `null` if successful. |
| `active_generation` | The configuration generation number currently in effect.     |

#### `POST /reload`

It triggers a configuration reload, equivalent to sending `SIGHUP` to the daemon process.

Returns the reload status as JSON:

```json
{
  "status": "reload_initiated",
  "error": null
}
```

| Field    | Description                                                                           |
| -------- | ------------------------------------------------------------------------------------- |
| `status` | `"reload_initiated"` if the reload is in progress, or `"reload_failed"` if it failed. |
| `error`  | Error message from the last reload attempt, or `null` if successful.                  |

#### `GET /runtime`

Returns the runtime status as JSON:

```json
{
  "primary_threads": 8,
  "io_uring_supported": true,
  "io_uring_runtime_enabled": true
}
```

| Field                      | Description                                               |
| -------------------------- | --------------------------------------------------------- |
| `primary_threads`          | Number of primary threads (typically equal to CPU count). |
| `io_uring_supported`       | Whether the current system supports `io_uring`.           |
| `io_uring_runtime_enabled` | Whether `io_uring` was successfully enabled at runtime.   |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

### Log rotation

- **`log` without rotation**: File-based access logging should include `access_log_rotate_size` (or an external log rotation policy) to prevent unbounded disk growth.
- **`error_log` without rotation**: File-based error logging should include `error_log_rotate_size` (or an external log rotation policy).

### Default ports

- **Both default ports disabled**: Setting `default_http_port false` and `default_https_port false` means host blocks without explicit ports create no listeners. Make sure all host blocks specify explicit ports, or keep at least one default listener enabled.

### PROXY protocol

- **`protocol_proxy` enabled**: PROXY protocol trusts addresses from clients. Enable it only on listeners reachable exclusively by trusted load balancers.

### Admin API

- **`admin.listen` on non-loopback address**: The admin API has no encryption and no authentication by default. Use `auth_token` to enable bearer token auth. Bind to a loopback address or restrict access via network controls.
- **`admin` without `auth_token`**: The admin API has no authentication by default. Use `auth_token` to require a bearer token on all endpoints except `/health` when the listener is reachable from untrusted networks.

### Location blocks

- **No duplicate `location` block pathnames**: Duplicate pathnames in location blocks cause the server to return an ambiguous response, so avoid them.
