---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, upstream backends, header manipulation, and connection pooling directives."
---

This page documents directives for forwarding incoming HTTP requests to one or more upstream backend servers. It supports load balancing, connection pooling with keep-alive reuse, health checking, and TLS upstream connections.

## Directives

### Reverse proxy and load balancing

- `proxy` (`http-proxy`)
  - This directive configures the reverse proxy with one or more upstream backends. Supports block form with nested directives or shorthand form with upstreams as arguments. Default: none
- `upstream <url: string>` (`http-proxy`)
  - This directive specifies a backend upstream server URL. Accepts `http://` or `https://` URLs. Can be nested inside a `proxy` block with optional `limit`, `idle_timeout`, and `unix` properties. Default: none
- `srv <name: string>` (`http-proxy`; requires `srv-lookup` feature)
  - This directive specifies a dynamic upstream resolved via DNS SRV records. Supports `dns_servers`, `limit`, and `idle_timeout` nested directives. Default: none
- `lb_algorithm <algorithm: string>` (`http-proxy`)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`. Default: `lb_algorithm two_random`
- `lb_health_check [bool: boolean]` (`http-proxy`)
  - This directive specifies whether passive health checking is enabled. Failed backends are temporarily excluded. Default: `lb_health_check false`
- `lb_health_check_max_fails <count: integer>` (`http-proxy`)
  - This directive specifies the maximum consecutive failures before a backend is marked unhealthy. Default: `lb_health_check_max_fails 3`
- `lb_health_check_window <duration: string>` (`http-proxy`)
  - This directive specifies the time window for the failure counter. After this duration, the failure count resets. Default: `lb_health_check_window 5s`
- `lb_retry_connection [bool: boolean]` (`http-proxy`)
  - This directive specifies whether to retry on connection failure if alternative backends are available. Default: `lb_retry_connection true`

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

- `keepalive [bool: boolean]` (`http-proxy`)
  - This directive specifies whether HTTP keep-alive connection pooling is enabled. Default: `keepalive true`
- `http2 [bool: boolean]` (`http-proxy`)
  - This directive specifies whether HTTP/2 is enabled for upstream connections. Default: `http2 false`
- `http2_only [bool: boolean]` (`http-proxy`)
  - This directive specifies whether only HTTP/2 is used for upstream connections. Default: `http2_only false`
- `intercept_errors [bool: boolean]` (`http-proxy`)
  - This directive specifies whether upstream error responses (4xx/5xx) are passed through to the client unchanged. When `false` (default), Ferron replaces upstream error responses with built-in error pages. When `true`, the full upstream response body and headers are passed through. Default: `intercept_errors false`

### TLS

- `no_verification [bool: boolean]` (`http-proxy`)
  - This directive specifies whether TLS certificate verification is disabled for HTTPS upstreams. Default: `no_verification false`

**Warning:** Only use `no_verification true` in testing or trusted internal networks.

### PROXY protocol

- `proxy_header <version: string>` (`http-proxy`)
  - This directive specifies whether to prepend HAProxy PROXY protocol header to upstream connections. Supported versions: `v1`, `v2`. Default: disabled

### Header manipulation

- `request_header` (`http-proxy`)
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

### Passive health checking

Passive health checking tracks connection failures per backend:

1. Each failed connection increments a counter for that backend.
2. If the counter exceeds `lb_health_check_max_fails` within `lb_health_check_window`, the backend is temporarily excluded from selection.
3. After the window expires, the counter resets and the backend becomes eligible again.
4. When `lb_retry_connection` is enabled and the selected backend fails, Ferron tries the next available backend.

### Active health checking

Active health checks proactively probe backend health on a schedule, independent of incoming traffic. This allows quick detection of backend failures before they affect client requests.

#### Directives

- `health_check [bool: boolean]` (`http-proxy`)
  - This directive specifies whether active health checking is enabled for this upstream. Default: `health_check false`
- `health_check_method <method: string>` (`http-proxy`)
  - This directive specifies the HTTP method for probe requests. Supported values: `GET`, `HEAD`. Default: `health_check_method GET`
- `health_check_uri <path: string>` (`http-proxy`)
  - This directive specifies the endpoint to probe for health checks. Default: `health_check_uri /health`
- `health_check_interval <duration: string>` (`http-proxy`)
  - This directive specifies the interval between health check probes. Default: `health_check_interval 10s`
- `health_check_timeout <duration: string>` (`http-proxy`)
  - This directive specifies the maximum wait time for a probe response. Default: `health_check_timeout 5s`
- `health_check_expect_status <status: string>` (`http-proxy`)
  - This directive specifies the expected HTTP status code(s) for a successful probe. Supports: `2xx`, `3xx`, `2xx,3xx`, specific codes (`200,201`), or ranges (`200-299`). Default: `health_check_expect_status 2xx,3xx`
- `health_check_response_time_threshold <duration: string>` (`http-proxy`)
  - This directive specifies an optional response time threshold; if exceeded, the probe is marked unhealthy. Default: disabled
- `health_check_body_match <substring: string>` (`http-proxy`)
  - This directive specifies an optional substring to match in the response body (GET only). Default: disabled
- `health_check_consecutive_fails <count: integer>` (`http-proxy`)
  - This directive specifies the number of consecutive failures before marking an upstream as unhealthy. Default: `health_check_consecutive_fails 2`
- `health_check_consecutive_passes <count: integer>` (`http-proxy`)
  - This directive specifies the number of consecutive successes before marking an upstream as healthy when recovering. Default: `health_check_consecutive_passes 2`
- `health_check_no_verification <boolean>` (`http-proxy`)
  - This directive specifies whether TLS certificate verification should be skipped for HTTPS health check probes. When set to `true`, the health check will accept any TLS certificate without validation. Default: `health_check_no_verification false`

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:3000 {
            health_check true
            health_check_uri "/health"
            health_check_interval 10s
            health_check_timeout 5s
            health_check_expect_status "200,204"
            health_check_consecutive_fails 2
            health_check_consecutive_passes 2
        }
        upstream https://localhost:3001 {
            health_check true
            health_check_uri "/api/status"
            health_check_method HEAD
            health_check_response_time_threshold 1s
            health_check_no_verification true
        }
        lb_algorithm two_random
    }
}
```

## Observability

### Metrics

The proxy module emits the following metrics:

- `ferron.proxy.backends.selected` (Counter) — backends selected during load balancing.
  - Attributes: backend URL or unix socket path
- `ferron.proxy.backends.unhealthy` (Counter) — backends marked as unhealthy.
  - Attributes: backend URL or unix socket path; `ferron.proxy.health_check_type` (`"passive"` for request-time failures, `"active"` for health check probe failures)
- `ferron.proxy.requests` (Counter) — upstream proxy requests completed.
  - Attributes: `ferron.proxy.connection_reused` (`true`/`false`), `http.response.status_code`, `ferron.proxy.status_code`
- `ferron.proxy.tls_handshake_failures` (Counter) — TLS handshake failures with upstream backends.
- `ferron.proxy.pool.waits` (Counter) — times the connection pool was exhausted and a request had to wait.
- `ferron.proxy.pool.wait_time` (Histogram) — duration spent waiting for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s.

## Notes and troubleshooting

- If you get 502 errors from backends, verify the `upstream` URLs are reachable and check passive health check settings (`lb_health_check_max_fails`).
- For active health checks:
  - Ensure the probe endpoint is configured and reachable on all backends (e.g., `/health` must return 2xx by default).
  - If upstreams are incorrectly marked unhealthy, check logs for "marked unhealthy" messages and verify the `health_check_expect_status` and response times.
  - Probe endpoints should be lightweight and low-latency to avoid impacting performance.
  - Use HEAD requests when the response body is not needed for faster probes.
  - Optional: Use `health_check_body_match` to ensure critical responses contain expected content (e.g., `"ok"` or `"healthy"`).
  - For HTTPS probes with self-signed certificates, use `health_check_no_verification true` to skip TLS certificate validation.
  - Both passive and active health checks work together: either can mark a backend as unhealthy.
- For the global connection limit (`concurrent_conns`), see [Core directives](/docs/v3/configuration/core-directives#reverse-proxy-connection-limits).
- For forward proxy configuration, see [Forward proxy](/docs/v3/configuration/http-fproxy).
