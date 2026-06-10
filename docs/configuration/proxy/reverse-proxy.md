---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, upstream backends, header manipulation, and connection pooling directives."
---

This page documents directives for forwarding incoming HTTP requests to one or more upstream backend servers. It supports load balancing, connection pooling with keep-alive reuse, health checking, circuit breaking, and TLS upstream connections.

## Directives

### Reverse proxy and load balancing

- `proxy` (`http-proxy`)
  - This directive configures the reverse proxy with one or more upstream backends. Supports block form with nested directives or shorthand form with upstreams as arguments. Default: none
- `upstream <url: string>` (`http-proxy`)
  - This directive specifies a backend upstream server URL. Accepts `http://` or `https://` URLs. Can be nested inside a `proxy` block with optional `limit`, `idle_timeout`, and `unix` properties. Default: none
- `srv <name: string>` (`http-proxy`; requires `srv-lookup` feature)
  - This directive specifies a dynamic upstream resolved via DNS SRV records. Supports `dns_servers`, `limit`, and `idle_timeout` nested directives. Default: none
- `algorithm <algorithm: string>` (`http-proxy`)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`. Default: `algorithm two_random`
- `passive_check [bool: boolean]` (`http-proxy`)
  - This directive enables passive health checking for backends. Supports nested `max_fails` and `window` directives. Default: `passive_check false`
- `circuit_breaker [bool: boolean]` (`http-proxy`)
  - This directive enables request-time circuit breaking for backends. Transport failures and upstream `5xx` responses count toward tripping the circuit. Supports nested `max_fails`, `window`, `open_duration`, and `consecutive_passes` directives. Default: `circuit_breaker false`
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

**Weighted round-robin example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:8080 {
            weight 5
        }
        upstream http://localhost:8081 {
            weight 2
        }
        upstream http://localhost:8082 {
            weight 1
        }

        algorithm round_robin
    }
}
```

In this example, the first backend receives approximately 62.5% of requests (5/8), the second receives 25% (2/8), and the third receives 12.5% (1/8). The smooth weighted round-robin algorithm distributes requests evenly over time rather than sending all requests to one backend before moving to the next.

#### Passive health check nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_fails` | `<count: integer>` | Maximum consecutive failures before marking backend unhealthy. | 3 |
| `window` | `<duration: string>` | Time window for the failure counter. After this duration, the counter resets. | `5s` |

#### Circuit breaker nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_fails` | `<count: integer>` | Number of transport failures or upstream `5xx` responses within the rolling `window` required to open the circuit. | 5 |
| `window` | `<duration: string>` | Rolling time window used for counting breaker failures. | `30s` |
| `open_duration` | `<duration: string>` | How long the circuit stays open before a half-open trial request is allowed. | `30s` |
| `consecutive_passes` | `<count: integer>` | Number of successful half-open trial requests required to close the circuit again. | 1 |

#### SSRF risk with interpolated upstream URLs

The upstream URL supports [interpolation syntax](/docs/v3/configuration/fundamentals/conditionals#built-in-variables) for dynamic values. **Never use user-controlled request headers** (e.g., `request.header.host`, `request.header.x_forwarded_host`, `request.header.x_forwarded_proto`) in upstream URLs, as an attacker can craft requests to redirect the proxy to internal services.

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

> [!warning]
> Only use `no_verification true` in testing or trusted internal networks.

### Client certificate authentication (mTLS)

When connecting to an upstream over HTTPS, Ferron can present a client certificate to authenticate itself. Configure the `cert` and `key` subdirectives on the upstream:

```ferron
example.com {
    proxy {
        upstream https://backend.internal:443 {
            cert "/etc/ferron/client-cert.pem"
            key "/etc/ferron/client-key.pem"
        }
    }
}
```

Both `cert` and `key` must be provided for mTLS to activate. The certificate chain and private key must be PEM-encoded. mTLS credentials are scoped per-upstream, so different backends can require different client certificates. Active health check probes also use the configured mTLS credentials. mTLS credentials are cached in memory until configuration reload or server shutdown.

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
| `weight` | `<number>` | Weight for weighted load balancing algorithms. Higher values receive more requests. Used with `round_robin`, `least_conn`, and affinity-based routing. | 1 |
| `cert` | `<path: string>` | Path to a PEM file containing the client certificate chain to present to the upstream server for mTLS. Must be used together with `key`. | disabled |
| `key` | `<path: string>` | Path to a PEM file containing the client private key for mTLS. Must be used together with `cert`. | disabled |

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
| `weight` | `<number>` | Weight for weighted load balancing algorithms. Applied to all backends resolved from this SRV record. Used with `round_robin`, `least_conn`, and affinity-based routing. | 1 |
| `cert` | `<path: string>` | Path to a PEM file containing the client certificate chain to present to resolved backends for mTLS. Must be used together with `key`. | disabled |
| `key` | `<path: string>` | Path to a PEM file containing the client private key for mTLS. Must be used together with `cert`. | disabled |

## Load balancing algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request. |
| `round_robin` | Distributes requests proportionally to backend weights using smooth weighted round-robin. |
| `least_conn` | Selects the backend with the fewest active tracked connections multiplied by its weight. |
| `two_random` | Picks two random backends and selects the less loaded one. |
| `p2c_ewma` | Power of Two Choices with EWMA (Exponentially Weighted Moving Average) latency scoring. Picks two random backends and selects the one with the lower combined score of EWMA response latency + active connection penalty. Automatically adapts to backend performance changes. |

## Session affinity

Session affinity (sticky sessions) ensures that requests from the same client are consistently routed to the same backend server. This is useful for stateful applications, WebSocket-heavy workloads, and improving cache locality.

The `affinity` directive configures session affinity inside a `proxy` block. Four affinity types are supported:

### Cookie affinity

Reads and sets a cookie to track which backend a client should be routed to. If no cookie is present, a backend is selected and a cookie is set on the response.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity cookie {
            name "ferron_sticky"
            ttl "24h"
            path "/"
            httponly
            samesite lax
        }
    }
}
```

#### Cookie nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `name` | `<string>` | Cookie name. | `ferron_sticky` |
| `ttl` | `<duration>` | Cookie time-to-live. | Session (browser closes) |
| `path` | `<string>` | Cookie path. | `/` |
| `domain` | `<string>` | Cookie domain. | Current domain |
| `secure` | `[bool]` | Only send cookie over HTTPS. | `false` |
| `httponly` | `[bool]` | Prevent JavaScript access to cookie. | `true` |
| `samesite` | `<mode>` | SameSite attribute: `strict`, `lax`, or `none`. | `lax` |

### Header affinity

Routes based on a specific request header value using consistent hashing.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity header {
            name "X-Backend-Id"
        }
    }
}
```

### IP affinity

Routes based on the client's IP address using consistent hashing. The same IP always routes to the same backend (while it remains healthy).

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity ip
    }
}
```

### Hash affinity

Routes based on a hashed variable value. Supports any built-in variable or request header.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity hash {
            variable "request.header.x-tenant-id"
        }
    }
}
```

### Affinity behavior

- Affinity is respected only when the target backend is healthy. If the affinity target is unhealthy, the configured load balancing algorithm is used as a fallback.
- When `retry_connection` is enabled and the affinity-targeted backend fails, Ferron retries with another backend.
- Cookie affinity automatically sets the cookie on the first request if no valid cookie is present.
- The affinity key is used with a consistent hash ring for deterministic routing.

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

> [!tip]
> If you get 502 errors from backends, verify the `upstream` URLs are reachable and check passive health check settings (`max_fails`).

## Health checking

### Passive health checking

Passive health checking tracks connection failures per backend:

1. Each failed connection increments a counter for that backend.
2. If the counter exceeds `max_fails` within the `window` duration, the backend is temporarily excluded from selection.
3. After the window expires, the counter resets and the backend becomes eligible again.
4. When `retry_connection` is enabled and the selected backend fails, Ferron tries the next available backend.

### Circuit breaking

Circuit breaking tracks request-time backend failures and temporarily ejects unstable backends:

1. Transport failures and upstream `5xx` responses are counted per backend in a rolling window.
2. When the backend reaches `max_fails` failures within `window`, Ferron opens the circuit and stops selecting that backend.
3. After `open_duration`, Ferron allows a single half-open trial request to the backend.
4. If the trial request succeeds, Ferron closes the circuit after `consecutive_passes` successful half-open requests.
5. If the half-open trial request fails, Ferron reopens the circuit immediately.

Circuit breaking does not automatically retry upstream `5xx` responses. It only changes which backends are eligible for future requests.

> [!note]
>
> - Half-open recovery allows only one trial request at a time. If recovery is too aggressive for your workload, increase `open_duration` or `consecutive_passes`.
> - Passive health checks, circuit breaking, and active health checks work together — any of them can make a backend temporarily ineligible.

> [!tip]
> If a backend is flapping, circuit breaking can protect the rest of the pool by temporarily ejecting it after repeated transport failures or upstream `5xx` responses.

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        algorithm round_robin
        retry_connection false

        circuit_breaker {
            max_fails 5
            window "30s"
            open_duration "10s"
            consecutive_passes 1
        }
    }
}
```

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

> [!tip]
> For active health checks: ensure the probe endpoint is reachable on all backends, keep probes lightweight, and use HEAD requests when the response body is not needed. If upstreams are incorrectly marked unhealthy, check logs and verify `expect_status`.

## Observability

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.proxy.backends.selected` | Counter | backend URL or unix socket path | Backends selected during load balancing |
| `ferron.proxy.backends.unhealthy` | Counter | backend URL or unix socket path; `ferron.proxy.health_check_type` (`"passive"` for request-time failures, `"active"` for health check probe failures, `"circuit_breaker"` for opened request-time circuits) | Backends marked as unhealthy |
| `ferron.proxy.requests` | Counter | `ferron.proxy.connection_reused` (`true`/`false`), `http.response.status_code`, `ferron.proxy.status_code` | Upstream proxy requests completed |
| `ferron.proxy.tls_handshake_failures` | Counter | backend URL or unix socket path | TLS handshake failures with upstream backends |
| `ferron.proxy.pool.waits` | Counter | backend URL or unix socket path | Times the connection pool was exhausted and a request had to wait |
| `ferron.proxy.pool.wait_time` | Histogram | backend URL or unix socket path | Duration spent waiting for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s |
| `ferron.proxy.lb.active_connections` | Gauge | backend URL or unix socket path | Active tracked connections for the selected backend |
| `ferron.proxy.lb.ewma_latency` | Gauge | backend URL or unix socket path | Current EWMA response latency for the selected backend (`p2c_ewma` algorithm) |
| `ferron.proxy.lb.warmup_state` | Gauge | backend URL or unix socket path | Whether the selected backend is in EWMA warm-up phase (1) or settled (0) |
| `ferron.proxy.lb.selections` | Counter | backend URL or unix socket path; `ferron.proxy.lb.reason` (`"p2c_ewma"`); `ferron.proxy.lb.score` (combined adaptive score) | P2C+EWMA backend selection with combined score |
| `ferron.proxy.backend.excluded` | Counter | backend URL or unix socket path; `ferron.proxy.reason` (`"passive"`, `"circuit_open"`, `"already_tried"`, `"overloaded"`) | Backend excluded from selection |
| `ferron.proxy.retry.count` | Counter | backend URL or unix socket path | Number of retry attempts made for a request |
| `ferron.proxy.retry.final` | Gauge | backend URL or unix socket path | Whether the final retry attempt succeeded (`1`) or failed (`0`) |
| `ferron.proxy.pool.hit` | Counter | backend URL or unix socket path | Pooled connection reused successfully |
| `ferron.proxy.pool.miss` | Counter | backend URL or unix socket path | Pooled connection unavailable, new connection established |
| `ferron.proxy.pool.idle` | Gauge | backend URL or unix socket path; `worker` (thread identifier) | Current number of idle connections in the pool |
| `ferron.proxy.pool.outstanding` | Gauge | backend URL or unix socket path; `worker` (thread identifier) | Current number of outstanding (in-use) connections in the pool |
| `ferron.proxy.connect.latency` | Histogram | backend URL or unix socket path | Time to establish a TCP/TLS connection to the backend |
| `ferron.proxy.ttfb` | Histogram | backend URL or unix socket path | Time to first response byte from the backend |
| `ferron.proxy.health.success` | Counter | backend URL or unix socket path | Health check probe succeeded |
| `ferron.proxy.health.failure` | Counter | backend URL or unix socket path | Health check probe failed |
| `ferron.proxy.health.duration` | Histogram | backend URL or unix socket path | Duration of health check probes |
| `ferron.proxy.circuit.state` | Gauge | backend URL or unix socket path | Circuit breaker state: `0` Closed, `1` HalfOpen, `2` Open |
| `ferron.proxy.circuit.open_total` | Counter | backend URL or unix socket path | Number of times the circuit breaker has transitioned to Open state |
| `ferron.proxy.failures` | Counter | `http.response.status_code` (HTTP response status code), `error.type` (error type classification) | Reverse-proxy failures that returned an error before a backend response was produced |

### Logs

- **`ERROR`**: logged when a proxy configuration error occurs during parsing. The message includes the error details.
- **`ERROR`**: logged when a proxy execution error occurs (e.g., connection failure, transport error). The message includes the error type and details.
- **`WARN`**: logged when an upstream is marked unhealthy by active health checks. The message includes the upstream address and failure reason.
- **`INFO`**: logged when an upstream recovers after consecutive successful health check probes.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Reverse proxy config error | ERROR | `error.message` (string) — configuration error details |
| Proxy error             | ERROR | `error.type` (string) — error type classification, `error.message` (string) — error details |
| Upstream marked unhealthy | WARN  | `upstream.address` (string) — backend server URL |
| Upstream recovered      | INFO  | `upstream.address` (string) — backend server URL |

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

### TLS verification

- **`proxy { no_verification }`** — Disabling TLS certificate verification for HTTPS upstreams should only be used for testing or tightly controlled internal networks.
- **`active_check { no_verification }`** — Disabling TLS verification for health check probes should only be used for strictly internal endpoints.

### Upstream SSRF risk

- **Upstream URL with request header interpolation** — Upstream URLs containing `{{request.header.*}}` are vulnerable to SSRF. Derive upstream targets from static configuration or trusted server-controlled variables.
