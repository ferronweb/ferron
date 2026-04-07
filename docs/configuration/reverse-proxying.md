---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, upstream backends, header manipulation, and connection pooling directives."
---

This page documents directives for forwarding incoming HTTP requests to one or more upstream backend servers. It supports load balancing, connection pooling with keep-alive reuse, health checking, and TLS upstream connections.

## Directives

### Reverse proxy and load balancing

- `proxy` (_http_proxy_ module)
  - This directive configures the reverse proxy with one or more upstream backends. Supports block form with nested directives or shorthand form with upstreams as arguments. Default: none
- `upstream <url: string>` (_http_proxy_ module)
  - This directive specifies a backend upstream server URL. Accepts `http://` or `https://` URLs. Can be nested inside a `proxy` block with optional `limit`, `idle_timeout`, and `unix` properties. Default: none
- `srv <name: string>` (_http_proxy_ module; requires `srv-lookup` feature)
  - This directive specifies a dynamic upstream resolved via DNS SRV records. Supports `dns_servers`, `limit`, and `idle_timeout` nested directives. Default: none
- `lb_algorithm <algorithm: string>` (_http_proxy_ module)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`. Default: `lb_algorithm two_random`
- `lb_health_check [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether passive health checking is enabled. Failed backends are temporarily excluded. When omitted, defaults to `true`. Default: `lb_health_check false`
- `lb_health_check_max_fails <count: integer>` (_http_proxy_ module)
  - This directive specifies the maximum consecutive failures before a backend is marked unhealthy. Default: `lb_health_check_max_fails 3`
- `lb_health_check_window <duration: string>` (_http_proxy_ module)
  - This directive specifies the time window for the failure counter. After this duration, the failure count resets. Default: `lb_health_check_window 5s`
- `lb_retry_connection [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether to retry on connection failure if alternative backends are available. When omitted, defaults to `true`. Default: `lb_retry_connection true`

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081 {
            limit 100
            idle_timeout 30s
        }

        lb_algorithm two_random
        lb_health_check
        lb_health_check_max_fails 3
        lb_health_check_window 5s
    }
}
```

### Connection behavior

- `keepalive [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether HTTP keep-alive connection pooling is enabled. When omitted, defaults to `true`. Default: `keepalive true`
- `http2 [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether HTTP/2 is enabled for upstream connections. When omitted, defaults to `true`. Default: `http2 false`
- `http2_only [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether only HTTP/2 is used for upstream connections. When omitted, defaults to `true`. Default: `http2_only false`
- `intercept_errors [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether upstream error responses (4xx/5xx) are passed through to the client as-is. When omitted, defaults to `true`. Default: `intercept_errors false`

### TLS

- `no_verification [bool: boolean]` (_http_proxy_ module)
  - This directive specifies whether TLS certificate verification is disabled for HTTPS upstreams. When omitted, defaults to `true`. Default: `no_verification false`

**Warning:** Only use `no_verification true` in testing or trusted internal networks.

### PROXY protocol

- `proxy_header <version: string>` (_http_proxy_ module)
  - This directive specifies whether to prepend HAProxy PROXY protocol header to upstream connections. Supported versions: `v1`, `v2`. Default: disabled

### Header manipulation

- `request_header` (_http_proxy_ module)
  - This directive manipulates request headers before forwarding to upstream. Three forms are supported:
    - `request_header +Name "value"` — **add** header (appends, allows duplicates)
    - `request_header -Name` — **remove** all instances of the header
    - `request_header Name "value"` — **replace** header (removes existing, sets new value)
  - Default: none

**Configuration example:**

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header +X-Custom-Header "value"
        request_header -X-Sensitive-Header
        request_header Host "new-host.example.com"
    }
}
```

### Global connection limit

- `proxy_concurrent_conns <limit: integer>` (global scope)
  - This directive specifies the global maximum number of concurrent TCP connections maintained in the keep-alive connection pool across all upstream backends. Unix socket connections are always unbounded. Default: `proxy_concurrent_conns 16384`

**Configuration example:**

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

## Upstream nested properties

### `upstream`

Defines a static backend server.

```ferron
upstream http://localhost:8080 {
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

Defines a dynamic upstream resolved via DNS SRV records.

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

## Load balancing algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request. |
| `round_robin` | Cycles through backends in order. |
| `least_conn` | Selects the backend with the fewest active tracked connections. |
| `two_random` | Picks two random backends and selects the less loaded one. |

## Forwarding headers

The reverse proxy module automatically manages standard forwarding headers:

| Header | Behavior |
| --- | --- |
| `X-Forwarded-For` | When `client_ip_from_header` is enabled, appends the extracted client IP to the existing chain. Otherwise, sets it to the direct connecting peer IP. |
| `X-Forwarded-Proto` | Always set to the incoming request scheme (`http` or `https`). |
| `X-Real-IP` | Always set to the client IP. |
| `Forwarded` (RFC 7239) | When `client_ip_from_header` is enabled, appends a new element (`for=...;proto=...;by=...`). Otherwise, sets a single element. IPv6 addresses are quoted per RFC 7239. |

## Connection pooling

Ferron maintains a keep-alive connection pool for upstream backends. Key behaviors:

- **Connection reuse**: Pooled connections are automatically reused for subsequent requests to the same upstream.
- **Idle eviction**: Connections idle longer than `idle_timeout` are evicted from the pool.
- **HTTP/2 multiplexing**: HTTP/2 connections share a single TCP connection for multiple concurrent requests.

## Health checking

Passive health checking tracks connection failures per backend:

1. Each failed connection increments a counter for that backend.
2. If the counter exceeds `lb_health_check_max_fails` within `lb_health_check_window`, the backend is temporarily excluded from selection.
3. After the window expires, the counter resets and the backend becomes eligible again.
4. When `lb_retry_connection` is enabled and the selected backend fails, Ferron tries the next available backend.

## Notes and troubleshooting

- If you get 502 errors from backends, verify the `upstream` URLs are reachable and check `lb_health_check_max_fails` settings.
- For the global connection limit (`concurrent_conns`), see [Core directives](/docs/v3/configuration/core-directives#reverse-proxy-connection-limits).
- For forward proxy configuration, see [Forward proxy](/docs/v3/configuration/http-fproxy).
