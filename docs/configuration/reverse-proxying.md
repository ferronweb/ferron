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
- `algorithm <algorithm: string>` (`http-proxy`)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`. Default: `algorithm two_random`
- `passive_check [bool: boolean]` (`http-proxy`)
  - This directive enables passive health checking for backends. Supports nested `max_fails` and `window` directives. Default: `passive_check false`
- `retry_connection [bool: boolean]` (`http-proxy`)
  - This directive specifies whether to retry on connection failure if alternative backends are available. Default: `retry_connection true`

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081 {
            limit 100
            idle_timeout "30s"
        }

        algorithm two_random
        passive_check {
            max_fails 3
            window "5s"
        }
    }
}
```

#### Passive health check nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_fails` | `<count: integer>` | Maximum consecutive failures before marking backend unhealthy. | 3 |
| `window` | `<duration: string>` | Time window for the failure counter. After this duration, the counter resets. | `5s` |

#### SSRF risk with interpolated upstream URLs

The upstream URL supports [interpolation syntax](/docs/v3/configuration/conditionals#built-in-variables) for dynamic values. **Never use user-controlled request headers** (e.g., `request.header.host`, `request.header.x_forwarded_host`, `request.header.x_forwarded_proto`) in upstream URLs, as an attacker can craft requests to redirect the proxy to internal services.

**Unsafe — user-controlled header in upstream URL:**

```ferron
example.com {
    # DANGEROUS: attacker can set X-Forwarded-Host to 169.254.169.254 or any internal host
    proxy "http://{{request.header.x_forwarded_host}}:8080"
}
```

**Safe — static upstream URL:**

```ferron
example.com {
    proxy http://localhost:8080
}
```

**Safe — upstream URL derived from trusted, server-controlled variables:**

```ferron
example.com {
    # Safe: request.host is resolved by Ferron's TLS/SNI matcher, not user-controlled
    proxy "http://{{request.host}}:8080"
}
```

If you need to forward the original host to a backend, use the `Host` header manipulation instead:

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header Host "{{request.host}}"
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
        keepalive
    }
}
```

## Upstream nested properties

### `upstream`

Defines a static backend server.

```ferron
example.com {
    upstream http://localhost:8080 {
        limit 100
        idle_timeout "30s"
        unix /var/run/backend.sock
    }
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
example.com {
    srv _http._tcp.example.com {
        dns_servers "8.8.8.8,8.8.4.4"
        limit 100
        idle_timeout "30s"
    }
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

- **Connection reuse** - pooled connections are automatically reused for subsequent requests to the same upstream.
- **Idle eviction** - connections idle longer than `idle_timeout` are evicted from the pool.
- **HTTP/2 multiplexing** - HTTP/2 connections share a single TCP connection for multiple concurrent requests.

## Health checking

### Passive health checking

Passive health checking tracks connection failures per backend:

1. Each failed connection increments a counter for that backend.
2. If the counter exceeds `max_fails` within the `window` duration, the backend is temporarily excluded from selection.
3. After the window expires, the counter resets and the backend becomes eligible again.
4. When `retry_connection` is enabled and the selected backend fails, Ferron tries the next available backend.

### Active health checking

Active health checks proactively probe backend health on a schedule, independent of incoming traffic. This allows quick detection of backend failures before they affect client requests.

Active health checks are configured per-upstream inside an `active_check` block.

#### `active_check` nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `uri` | `<path: string>` | The endpoint to probe for health checks. | `/health` |
| `method` | `<method: string>` | HTTP method for probe requests. Supported values: `GET`, `HEAD`. | `GET` |
| `interval` | `<duration: string>` | Interval between health check probes. | `10s` |
| `timeout` | `<duration: string>` | Maximum wait time for a probe response. | `5s` |
| `expect_status` | `<status: string>` | Expected HTTP status code(s) for a successful probe. Supports: `2xx`, `3xx`, `2xx,3xx`, specific codes (`200,204`), or ranges (`200-299`). | `2xx,3xx` |
| `response_time_threshold` | `<duration: string>` | Optional response time threshold; if exceeded, the probe is marked unhealthy. | disabled |
| `body_match` | `<substring: string>` | Optional substring to match in the response body (GET only). | disabled |
| `consecutive_fails` | `<count: integer>` | Number of consecutive failures before marking an upstream as unhealthy. | 2 |
| `consecutive_passes` | `<count: integer>` | Number of consecutive successes before marking an upstream as healthy when recovering. | 2 |
| `no_verification` | `[bool: boolean]` | Whether to skip TLS certificate verification for HTTPS probes. | `false` |

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:3000 {
            active_check {
                uri "/health"
                interval "10s"
                timeout "5s"
                expect_status "200,204"
                consecutive_fails 2
                consecutive_passes 2
            }
        }
        upstream https://localhost:3001 {
            active_check {
                uri "/api/status"
                method HEAD
                response_time_threshold "1s"
                no_verification
            }
        }
        algorithm two_random
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

- If you get 502 errors from backends, verify the `upstream` URLs are reachable and check passive health check settings (`max_fails`).
- For active health checks:
  - Ensure the probe endpoint is configured and reachable on all backends (e.g., `/health` must return 2xx by default).
  - If upstreams are incorrectly marked unhealthy, check logs for "marked unhealthy" messages and verify the `expect_status` and response times.
  - Probe endpoints should be lightweight and low-latency to avoid impacting performance.
  - Use HEAD requests when the response body is not needed for faster probes.
  - Optional: Use `body_match` to ensure critical responses contain expected content (e.g., `"ok"` or `"healthy"`).
  - For HTTPS probes with self-signed certificates, use `no_verification true` to skip TLS certificate validation.
  - Both passive and active health checks work together: either can mark a backend as unhealthy.
- For the global connection limit (`concurrent_conns`), see [Core directives](/docs/v3/configuration/core-directives#reverse-proxy-connection-limits).
- For forward proxy configuration, see [Forward proxy](/docs/v3/configuration/http-fproxy).
