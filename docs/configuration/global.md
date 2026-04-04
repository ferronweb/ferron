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
- PROXY protocol: `protocol_proxy`
- Observability: `observability`

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

Notes:

- Log files are created if they don't exist and opened in append mode.
- Writes are buffered and flushed periodically (every 1 second) and on shutdown.
- If `access_log` is omitted, access events are ignored. Same applies for `error_log`.
- If a formatter is specified but not found in the registry, access events are not written (no fallback output).

## Notes

- These directives affect startup and listener construction, not per-request routing.
- The built-in blank configuration enables `runtime.io_uring true`.
