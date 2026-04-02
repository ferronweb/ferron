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

## Notes

- These directives affect startup and listener construction, not per-request routing.
- The built-in blank configuration enables `runtime.io_uring true`.
