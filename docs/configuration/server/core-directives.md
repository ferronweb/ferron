---
title: "Configuration: core directives"
description: "Top-level directives for runtime, networking, admin API, observability, and reverse proxy connection limits."
---

These directives belong in top-level global blocks:

```ferron
{
    # global directives here, for example:
    console_log
}
```

> [!note]
> These directives affect startup and listener construction, not per-request routing.
>
> The `config-ferronconf` module parses `.conf` files and the `config-json` module parses `.json` files.

> [!info]
> For observability settings, see [Observability and logging](/docs/configuration/observability/logging). For per-host HTTP settings, see [HTTP host directives](/docs/configuration/server/host). For admin API hardening, see [Admin API security](#admin-api-security).

## Default ports

`default_http_port <port: integer | false>` sets the HTTP port for a host block without an explicit port. The value must be a positive integer no larger than `65535`, or `false` to disable the default HTTP listener. Default: `80`.

`default_https_port <port: integer | false>` sets the HTTPS port for HTTP-to-HTTPS redirects and URL generation. The value must be a positive integer no larger than `65535`, or `false` to disable the default HTTPS listener. Default: `443`.

```ferron
{
    default_http_port 8080
    default_https_port 8443
}
```

> [!note]
> A host without an explicit port starts an HTTP listener on `default_http_port` and an HTTPS listener on `default_https_port`.
>
> The redirect stage builds `https://` URLs with this port. It omits the port when the value is `443`.
>
> `default_http_port false` disables the automatic HTTP listener. `default_https_port false` disables the automatic HTTPS listener and the HTTP-to-HTTPS redirects.
>
> With both directives set to `false`, host blocks without explicit ports create no listeners, and Ferron logs a warning.

HTTPS only:

```ferron
{
    default_http_port false
}
```

Explicit ports only:

```ferron
{
    default_http_port false
    default_https_port false
}
```

## Host definitions

`strict_hosts <bool>` rejects a host that host blocks define in more than one configuration file.

By default, Ferron merges all host blocks for the same host, meaning the same protocol, port, IP address, and hostname, even across files. With `strict_hosts` on, a second file that defines an existing host makes validation, startup, and reload fail. The error names both files. Blocks for the same host within one file still merge.

Default: disabled.

```ferron
{
    strict_hosts
}

include "sites/*.ferron"
```

> [!note]
> Use `strict_hosts` when each site has its own file, for example on a server that hosts sites for several teams.
>
> Without it, a second file for an existing domain merges into the first. First-value directives such as `root` keep the value from the file loaded first. Directives such as `header` add to the existing value. `basic_auth` accepts users from both files.
>
> Ferron loads files matched by an `include` glob in alphabetical order.

## Runtime

`io_uring <bool>` enables `io_uring` when the system supports it. It can improve static file serving for large files and reverse proxying under very high concurrency. If initialization fails, Ferron falls back to `epoll` and logs a warning.

Default: disabled.

`cpu_affinity <core_id> [<core_id> ...]` pins Ferron's primary (data plane) threads to the listed CPU cores, using 0-based IDs. This helps with asymmetric CPU cores (ASMP) and NUMA, where some cores are symmetric with each other or share a NUMA node. The thread count becomes the number of listed cores.

Default: all available CPU cores.

```ferron
{
    runtime {
        io_uring
        cpu_affinity 0 1 2 3 # first 4 CPU cores
    }
}
```

## Network and listeners

`listen <address: string>` sets the bind addresses for TCP listeners. It accepts IP addresses or full socket addresses. A socket address must use the same port that Ferron starts. To change the default HTTP ports, use `default_http_port` and `default_https_port` instead.

Default: `[::]:<http-port>`.

`send_buf <size: integer>` sets the TCP send buffer size. The value must resolve to a non-negative integer at runtime. Default: OS default.

`recv_buf <size: integer>` sets the TCP receive buffer size. The value must resolve to a non-negative integer at runtime. Default: OS default.

`backlog <size: integer>` sets the maximum number of pending connections on the listener socket. Default: `-1` unlimited on Windows, BSD, and macOS, and `4096` elsewhere.

`multipath <bool>` enables Multipath TCP (MPTCP) on the listener. MPTCP lets one TCP connection use several network interfaces at once, which improves throughput and resilience. If the kernel does not support MPTCP or has it disabled, Ferron logs a warning and falls back to standard TCP.

Default: disabled.

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

## Unix domain sockets (Unix only)

`unix <path: string> { ... }` creates a Unix domain socket listener. Repeat the directive to listen on several sockets. Each directive takes a filesystem path and an optional block with `backlog`, `mode`, `owner`, and `group`.

The path can be absolute (`/run/ferron.sock`) or relative (`ferron.sock`, `tmp/ferron.sock`). A relative path resolves against the server working directory and is canonicalized when the target exists. Otherwise Ferron keeps the path as written.

Default: none.

| Directive | Arguments                  | Description                     | Default                       |
| --------- | -------------------------- | ------------------------------- | ----------------------------- |
| `backlog` | `<size: integer>`          | Maximum pending connections     | `-1` (unlimited)              |
| `mode`    | `<mode: string\|integer>`  | File permissions for the socket | OS default respecting `umask` |
| `owner`   | `<user: string\|integer>`  | Owner of the socket             | Process user                  |
| `group`   | `<group: string\|integer>` | Group of the socket             | Process group                 |

```ferron
{
    unix /run/ferron.sock {
        backlog 128
        mode "0660"
        owner ferron
        group ferron
    }

    # Relative path, resolved against the server working directory.
    # The default Docker image uses /var/log/ferron.
    unix "ferron.sock" {
        mode "0660"
    }

    unix /run/ferron-api.sock {
        mode "0660"
    }
}
```

> [!note]
> With at least one `unix` directive, Ferron disables all TCP and QUIC (HTTP/3) listeners and serves HTTP only over Unix sockets. Host blocks then use the Unix sockets regardless of their port assignments, except for IP blocks because Unix sockets have no IP address. The `Host` header and TLS SNI still select the virtual host.
>
> To serve TCP and Unix at the same time, do not use `unix`. Run a separate TCP terminator or reverse proxy in front of the Unix socket instead.

> [!important]
> `unix` works only on Unix-like systems. The validator rejects it on Windows.
>
> A socket path must be shorter than 108 bytes (`sun_path` limit), must not contain `NUL`, and must not already exist as a non-socket file.
>
> Ferron creates parent directories automatically. It removes stale socket files from a previous run when they are sockets. Otherwise startup fails.

## Reverse proxy connection limits

`concurrent_conns <limit: integer>` sets the global maximum number of concurrent TCP connections in the reverse proxy keep-alive pool. Every host that uses `proxy` shares the limit. Unix socket connections have no limit.

Default: `16384`.

```ferron
{
    concurrent_conns 10000
}
```

## Admin API

The `admin` block configures the built-in administration endpoints. Without an `admin` block, Ferron disables the admin API.

| Directive     | Arguments            | Default            | Description                                                             |
| ------------- | -------------------- | ------------------ | ----------------------------------------------------------------------- |
| `listen`      | `<address: string>`  | `127.0.0.1:8081`   | Socket address for the admin HTTP listener.                             |
| `auth_token`  | `<token: string>`    | none               | Bearer token. Clients send `Authorization: Bearer <token>`.             |
| `health`      | `[bool]`             | `true`             | Enables `GET /health`.                                                  |
| `status`      | `[bool]`             | `true`             | Enables `GET /status`.                                                  |
| `config`      | `[bool]`             | `true`             | Enables `GET /config`.                                                  |
| `reload`      | `[bool]`             | `true`             | Enables `POST /reload`.                                                 |
| `reload_get`  | `[bool]`             | `true`             | Enables `GET /reload`.                                                  |
| `runtime`     | `[bool]`             | `true`             | Enables `GET /runtime`.                                                 |

`/health` returns `200 OK`, or `503 Service Unavailable` during shutdown. It is always exempt from authentication, because load balancers and orchestrators require it.

`/status` returns JSON with uptime, active connections, request count, and reload count.

`/config` returns the current effective configuration as JSON with sensitive values redacted.

`POST /reload` triggers a configuration reload, equivalent to `SIGHUP`. `GET /reload` returns the current reload status.

`/runtime` returns runtime information such as thread count and `io_uring` status.

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
> `/config` redacts sensitive directive names, including `key`, `cert`, `private_key`, `password`, `secret`, `token`, `ticket_keys`, `bearer`, `passwd`, and `htpasswd`.

## Admin API security

The admin API is a privileged control plane with full access to the server configuration and the reload capability. It has no encryption and no authentication by default. `auth_token` enables bearer token authentication. Treat the admin API with the same care as a root shell on the server.

### Risks of binding to `0.0.0.0`

`listen "0.0.0.0:<port>"` opens the admin API to any client that can reach the host. Omitting the bind address defaults to all interfaces and has the same effect. This can happen by accident in a containerized environment, for example Docker with bridge networking, or on a misconfigured network.

Consequences of an open admin API:

- Denial of service: anyone can send `POST /reload` in a loop, which degrades performance.
- Configuration leak: `GET /config` reveals hostnames, upstream addresses, and routing rules. Sensitive values stay redacted.
- Service disruption: anyone can reload a modified configuration to disable an endpoint or inject a misconfigured directive.

### Hardening

1. Bind to localhost unless you have a specific, secure reason not to:

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

2. Disable endpoints that you do not need:

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

3. For remote access, front the admin API with an authenticating reverse proxy. Do not bind to `0.0.0.0`:

   ```text
   Remote user -> reverse proxy (auth required) -> 127.0.0.1:8081 (admin API)
   ```

4. Restrict network access at the infrastructure level. Use firewall rules, security groups, or VPC networking so that only trusted hosts reach the admin port.

5. Monitor admin API access with your observability sinks and watch for anomalies.

6. Never expose the admin API to the public internet. For remote administration, use an SSH tunnel:

   ```bash
   ssh -L 8081:127.0.0.1:8081 admin@your-server
   # Then access http://127.0.0.1:8081 locally
   ```

## Admin API endpoints

### `GET /health`

Returns `200 OK` while the server runs, and `503 Service Unavailable` when shutdown starts. Use it for load balancer and orchestration health checks.

### `GET /status`

Returns server metrics as JSON:

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

### `GET /config`

Returns the full effective configuration as JSON with sensitive directives redacted. Ferron replaces TLS keys, passwords, and tokens with `"[redacted]"`. Use it for debugging and auditing.

### `GET /reload`

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

### `POST /reload`

Triggers a configuration reload, equivalent to sending `SIGHUP` to the daemon.

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

### `GET /runtime`

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

## Observability

The `observability` block configures per-host event sinks for logging and metrics. Several `observability` directives for one host accumulate sinks.

`provider <name: string>` sets the observability provider. It is required when observability is enabled through the block form.

| Value     | Module                     |
| --------- | -------------------------- |
| `console` | `observability-consolelog` |
| `file`    | `observability-logfile`    |

Default: none.

### provider console

```ferron
example.com {
    observability {
        provider console
    }
}
```

The `console` provider takes no subdirectives. It writes supported observability events to Ferron logs.

### provider file

The `file` provider writes observability events to log files:

| Subdirective             | Arguments  | Description                                            | Default         |
| ------------------------ | ---------- | ------------------------------------------------------ | --------------- |
| `access_log`             | `<string>` | File path for access log output.                       | none            |
| `error_log`              | `<string>` | File path for error log output.                        | none            |
| `format`                 | `<string>` | Access log formatter name (`text` or `json`).          | `text`          |
| `error_format`           | `<string>` | Application log formatter name (`text` or `json`).     | `text`          |
| `access_log_rotate_size` | `<number>` | Maximum access log file size in bytes before rotation. | disabled        |
| `access_log_rotate_keep` | `<number>` | Number of rotated access log files to keep.            | none (no limit) |
| `error_log_rotate_size`  | `<number>` | Maximum error log file size in bytes before rotation.  | disabled        |
| `error_log_rotate_keep`  | `<number>` | Number of rotated error log files to keep.             | none (no limit) |

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
> Ferron creates log files in append mode when they do not exist.
>
> Ferron buffers writes and flushes them every second and on shutdown.
>
> Without `access_log`, Ferron ignores access events. Without `error_log`, it ignores error events.
>
> With rotation on, Ferron renames the current log file to `<filename>.1`, shifts the rotated files up, and creates a new log file.
>
> With `access_log_rotate_keep` or `error_log_rotate_keep` set to `0`, Ferron deletes the log file on rotation instead of renaming it.

## Observability aliases

Ferron has shorthand directives for common observability configurations. It converts them into equivalent `observability` blocks.

### `log`

`log` configures access logging with the `file` provider.

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

```ferron
example.com {
    # Access logging with the default format
    log /var/log/access.log

    # Access logging with JSON output
    log /var/log/access.log {
        format json
    }

    # Rotation: 100 MB maximum, keep 5 rotated files
    log /var/log/access.log {
        access_log_rotate_size 104857600
        access_log_rotate_keep 5
    }

    # Disable access logging
    log false
}
```

### `error_log`

`error_log` configures error logging with the `file` provider.

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

```ferron
example.com {
    error_log /var/log/error.log

    # Rotation: 50 MB maximum, keep 3 rotated files
    error_log /var/log/error.log {
        error_log_rotate_size 52428800
        error_log_rotate_keep 3
    }

    # JSON application log formatting
    error_log /var/log/error.log {
        error_format json
    }

    # Disable error logging
    error_log false
}
```

### `console_log`

`console_log` configures console-based observability.

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

## Best practices

`ferron doctor` reports these checks for directives on this page.

### Log rotation

- `log` without rotation: file-based access logging should set `access_log_rotate_size` or use an external rotation policy, so log files do not grow without bound.
- `error_log` without rotation: file-based error logging should set `error_log_rotate_size` or use an external rotation policy.

### Default ports

- Both default ports disabled: with `default_http_port false` and `default_https_port false`, host blocks without explicit ports create no listeners. Set explicit ports on every host block, or keep at least one default listener enabled.

### PROXY protocol

- `protocol_proxy` enabled: the PROXY protocol trusts addresses from clients. Enable it only on listeners that only trusted load balancers can reach.

### Admin API

- `admin.listen` on a non-loopback address: the admin API has no encryption and no authentication by default. Enable `auth_token`, bind to a loopback address, or restrict access with network controls.
- `admin` without `auth_token`: add `auth_token` to require a bearer token on every endpoint except `/health` when the listener is reachable from untrusted networks.

### Location blocks

- Duplicate `location` pathnames: a duplicate pathname makes the server response ambiguous. Give each `location` a distinct path.
