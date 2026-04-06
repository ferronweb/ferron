# HTTP Reverse Proxy Directives

The `proxy` directive configures Titanium to forward incoming HTTP requests to one or more upstream backend servers. It supports load balancing, connection pooling with keep-alive reuse, health checking, and TLS upstream connections.

## Categories

- Main directive: `proxy`
- Upstream backends: `upstream`, `srv`
- Load balancing: `lb_algorithm`, `lb_health_check`, `lb_health_check_max_fails`, `lb_health_check_window`, `lb_retry_connection`
- Connection behavior: `keepalive`, `http2`, `http2_only`, `intercept_errors`
- TLS: `no_verification`
- PROXY protocol: `proxy_header`
- Header manipulation: `request_header`
- Global connection limit: `proxy_concurrent_conns` (global scope)

## `proxy`

Syntax — block form:

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081 {
            limit 100
            idle_timeout 30s
        }

        lb_algorithm two_random
        keepalive true
        http2 false
    }
}
```

Syntax — shorthand form (upstreams as arguments):

```ferron
example.com {
    proxy http://localhost:8080 http://localhost:8081 {
        lb_algorithm two_random
        keepalive
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `upstream` | `<url>` | Backend upstream server URL. Accepts `http://` or `https://` URLs. | none |
| `srv` | `<name>` | SRV-based upstream (requires `srv-lookup` feature). Resolves DNS SRV records dynamically. | none |
| `lb_algorithm` | `<string>` | Load balancing strategy. See below. | `two_random` |
| `lb_health_check` | *(optional)* `<boolean>` | Enable passive health checking. Failed backends are temporarily excluded. When omitted, defaults to `true`. | `false` |
| `lb_health_check_max_fails` | `<number>` | Maximum consecutive failures before a backend is marked unhealthy. | `3` |
| `lb_health_check_window` | `<duration>` | Time window for the failure counter. After this duration, the failure count resets. | `5s` |
| `lb_retry_connection` | *(optional)* `<boolean>` | Retry on connection failure if alternative backends are available. When omitted, defaults to `true`. | `true` |
| `keepalive` | *(optional)* `<boolean>` | Enable HTTP keep-alive connection pooling. When omitted, defaults to `true`. | `true` |
| `http2` | *(optional)* `<boolean>` | Enable HTTP/2 for upstream connections. When omitted, defaults to `true`. | `false` |
| `http2_only` | *(optional)* `<boolean>` | Only use HTTP/2 for upstream connections. When omitted, defaults to `true`. | `false` |
| `intercept_errors` | *(optional)* `<boolean>` | Pass upstream error responses (4xx/5xx) through to the client as-is. When omitted, defaults to `true`. | `false` |
| `no_verification` | *(optional)* `<boolean>` | Disable TLS certificate verification for HTTPS upstreams. When omitted, defaults to `true`. | `false` |
| `proxy_header` | `v1 \| v2` | Prepend HAProxy PROXY protocol header to upstream connections. | disabled |
| `request_header` | see below | Add, remove, or replace request headers before forwarding. | none |

### `upstream`

Defines a static backend server.

```ferron
upstream http://localhost:8080
```

With optional nested properties:

```ferron
upstream http://localhost:8081 {
    limit 100
    idle_timeout 30s
    unix /var/run/backend.sock
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `limit` | `<number>` | Maximum concurrent connections to this specific upstream. | unlimited |
| `idle_timeout` | `<duration>` | Keep-alive idle timeout. Connections idle longer than this are evicted from the pool. | `60s` |
| `unix` | `<path>` | Connect via Unix domain socket instead of TCP. The URL scheme is still required. | TCP |

### `srv` (feature-gated)

Defines a dynamic upstream resolved via DNS SRV records. Requires the `srv-lookup` feature and a secondary Tokio runtime (captured during module startup).

```ferron
srv _http._tcp.example.com {
    dns_servers 8.8.8.8,8.8.4.4
    limit 100
    idle_timeout 30s
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `dns_servers` | `<string>` | Comma-separated DNS server IPs. Uses system resolver if empty. | system |
| `limit` | `<number>` | Maximum concurrent connections per resolved backend. | unlimited |
| `idle_timeout` | `<duration>` | Keep-alive idle timeout per resolved backend. | `60s` |

SRV resolution happens at each request. The resolver:
1. Looks up the SRV record on the secondary Tokio runtime.
2. Filters out backends that have exceeded `lb_health_check_max_fails`.
3. Keeps only the highest-priority group (lowest numeric priority).
4. Performs weighted random selection within the group.

### Load Balancing Algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request. |
| `round_robin` | Cycles through backends in order. |
| `least_conn` | Selects the backend with the fewest active tracked connections. |
| `two_random` | Picks two random backends and selects the less loaded one. |

## `request_header`

Manipulates request headers before forwarding to upstream. Three forms are supported:

| Syntax | Effect |
| --- | --- |
| `request_header +Name "value"` | **Add** header (appends, allows duplicates) |
| `request_header -Name` | **Remove** all instances of the header |
| `request_header Name "value"` | **Replace** header (removes existing, sets new value) |

Examples:

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header +X-Custom-Header "value"
        request_header -X-Sensitive-Header
        request_header Host "new-host.example.com"
    }
}
```

## Forwarding Headers

The reverse proxy module also manages standard forwarding headers:

| Header | Behavior |
| --- | --- |
| `X-Forwarded-For` | When `client_ip_from_header` is enabled, appends the extracted client IP to the existing chain. Otherwise, sets it to the direct connecting peer IP. |
| `X-Forwarded-Proto` | Always set to the incoming request scheme (`http` or `https`). |
| `X-Real-IP` | Always set to the client IP. |
| `Forwarded` (RFC 7239) | When `client_ip_from_header` is enabled, appends a new element (`for=...;proto=...;by=...`). Otherwise, sets a single element. IPv6 addresses are quoted per RFC 7239. |

## `proxy_concurrent_conns` (global scope)

Sets the global maximum number of concurrent TCP connections maintained in the keep-alive connection pool across all upstream backends. Unix socket connections are always unbounded.

Syntax:

```ferron
{
    proxy_concurrent_conns 10000
}

example.com {
    proxy http://localhost:8080 {
        keepalive true
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<number>` | Maximum concurrent TCP connections in the pool. Must be non-negative. | `16384` |

Notes:

- This limit applies to the entire reverse proxy module, shared across all hosts that use `proxy`.
- The pool is created lazily on the first request that needs it, reading this global value at creation time.
- Per-upstream `limit` directives further restrict connections to individual backends.

## Connection Pooling

Titanium maintains a keep-alive connection pool for upstream backends using the `connpool` crate. Key behaviors:

- **Connection reuse**: Pooled connections are automatically reused for subsequent requests to the same upstream.
- **Idle eviction**: Connections idle longer than `idle_timeout` are evicted from the pool.
- **Racing non-ready connections**: When a pooled connection is not yet ready (e.g., mid-handshake), it is collected and raced against establishing a new connection, avoiding unnecessary duplicate connection establishments.
- **HTTP/2 multiplexing**: HTTP/2 connections share a single TCP connection for multiple concurrent requests.

## Health Checking

Passive health checking tracks connection failures per backend:

1. Each failed connection increments a counter for that backend.
2. If the counter exceeds `lb_health_check_max_fails` within `lb_health_check_window`, the backend is temporarily excluded from selection.
3. After the window expires, the counter resets and the backend becomes eligible again.
4. When `lb_retry_connection` is enabled and the selected backend fails, Titanium tries the next available backend.
