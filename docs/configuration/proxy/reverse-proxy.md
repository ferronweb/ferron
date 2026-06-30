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
  - This directive specifies a backend upstream server URL. Accepts `http://` or `https://` URLs. Can be nested inside a `proxy` block with optional `limit`, `idle_timeout`, `unix`, `logical_dns`, and `dns_servers` properties. When the URL contains a hostname, A/AAAA records are resolved via Hickory DNS by default (strict DNS), creating a separate backend per resolved IP. Default: none
- `srv <name: string>` (`http-proxy`; requires `srv-lookup` feature)
  - This directive specifies a dynamic upstream resolved via DNS SRV records. Supports `dns_servers`, `limit`, and `idle_timeout` nested directives. Default: none
- `algorithm <algorithm: string>` (`http-proxy`)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`. Default: `algorithm two_random`
- `circuit_breaker [bool: boolean]` (`http-proxy`)
  - This directive enables request-time circuit breaking for backends. Transport failures always count toward tripping the circuit. Upstream `5xx` responses count only when `record_5xx` is enabled. Supports nested `max_fails`, `window`, `open_duration`, `consecutive_passes`, and `record_5xx` directives. Default: `circuit_breaker false`
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

#### Circuit breaker nested directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_fails` | `<count: integer>` | Number of transport failures (and `5xx` responses when `record_5xx` is enabled) within the rolling `window` required to open the circuit. | 5 |
| `window` | `<duration: string>` | Rolling time window used for counting breaker failures. | `30s` |
| `open_duration` | `<duration: string>` | How long the circuit stays open before a half-open trial request is allowed. | `30s` |
| `consecutive_passes` | `<count: integer>` | Number of successful half-open trial requests required to close the circuit again. | 1 |
| `record_5xx` | `[bool: boolean]` | Whether upstream `5xx` responses count toward tripping the circuit. Transport failures always count. | `false` |

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
  - This directive specifies whether upstream error responses (4xx/5xx) are intercepted and replaced with built-in error pages. When `true`, Ferron replaces upstream error responses with its own error pages. When `false` (default), the full upstream response body and headers are passed through unchanged. Default: `intercept_errors false`

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
| `weight` | `<number>` | Weight for weighted load balancing. Higher values receive more requests. Supported by all load balancing algorithms (`random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`) and session affinity. | 1 |
| `priority` | `<number>` | Priority for tiered failover. Lower values are higher priority. When the highest-priority tier is exhausted, the next tier is tried. | 0 |
| `cert` | `<path: string>` | Path to a PEM file containing the client certificate chain to present to the upstream server for mTLS. Must be used together with `key`. | disabled |
| `key` | `<path: string>` | Path to a PEM file containing the client private key for mTLS. Must be used together with `cert`. | disabled |
| `logical_dns` | `[bool: boolean]` | When `true`, disables strict A/AAAA DNS resolution and uses the system's logical DNS resolution instead. The upstream URL is passed through as-is without per-IP backend splitting. | `false` |
| `dns_servers` | `<string>` | Comma-separated DNS server IPs used for strict A/AAAA resolution. Uses Hickory's default resolvers if empty. | system |

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
| `weight` | `<number>` | Multiplier applied to DNS SRV weights. Each backend's effective weight is `dns_weight × config_weight`. Set to `1` to use DNS weights as-is. Supported by all load balancing algorithms (`random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`) and session affinity. | 1 |
| `priority` | `<number>` | Additive offset applied to DNS SRV priorities. A backend's effective priority is `dns_priority + offset`. Lower effective values are tried first. | 0 |
| `cert` | `<path: string>` | Path to a PEM file containing the client certificate chain to present to resolved backends for mTLS. Must be used together with `key`. | disabled |
| `key` | `<path: string>` | Path to a PEM file containing the client private key for mTLS. Must be used together with `cert`. | disabled |

## Load balancing algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request, weighted by configured weights. |
| `round_robin` | Distributes requests proportionally to backend weights using smooth weighted round-robin. |
| `least_conn` | Selects the backend with the fewest active tracked connections multiplied by its weight. |
| `two_random` | Picks two random backends and selects the one with lower weighted load (connections divided by weight). |
| `p2c_ewma` | Power of Two Choices with EWMA (Exponentially Weighted Moving Average) latency scoring. Picks two random backends and selects the one with the lower combined score of EWMA response latency + active connection penalty, divided by weight. Automatically adapts to backend performance changes. |

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

## Priority-based failover

Backends can be assigned numeric priority values to implement tiered failover. Lower values indicate higher priority. When all backends in the highest-priority tier are unavailable (unhealthy, circuit-open, or already-tried), the next tier is used as a fallback.

```ferron
example.com {
    proxy {
        upstream http://primary:8080 {
            priority 0
        }
        upstream http://secondary:8080 {
            priority 1
        }
        upstream http://standby:8080 {
            priority 2
        }
    }
}
```

Requests are routed to the primary tier (priority 0) first. If all primary backends are unavailable, the secondary tier (priority 1) is tried, and so on. Within each tier, the configured load balancing algorithm selects which backend handles the request. Priority and weight work together: priority determines which tier is tried, and weight determines how requests are distributed within a tier.

For SRV upstreams, the `priority` subdirective is an additive offset applied to DNS SRV priorities. A backend's effective priority is `dns_priority + offset`. For example, if DNS returns priorities 10 and 20, setting `priority 5` on the SRV block shifts them to 15 and 25.

```ferron
example.com {
    srv _http._tcp.example.com {
        priority 5
    }
}
```

## Strict DNS (A/AAAA) resolution

When an `upstream` URL contains a hostname (not an IP literal; except `localhost`), Ferron resolves A and AAAA records using Hickory DNS by default. Each resolved IP address becomes a separate backend in the load balancer, enabling per-IP load balancing, circuit breaking, and health checking.

For example, if `http://myapp.example.com:8080` resolves to three IPs (`10.0.0.1`, `10.0.0.2`, `10.0.0.3`), Ferron creates three distinct backends — one per IP. The original hostname is preserved for TLS SNI and the HTTP `Host` header.

```ferron
example.com {
    proxy {
        upstream http://myapp.example.com:8080 {
            dns_servers "8.8.8.8,8.8.4.4"
        }
    }
}
```

### Opting out with `logical_dns`

Set `logical_dns true` to disable strict A/AAAA resolution. The upstream URL is passed through as-is, and the system's DNS resolver handles address selection. This is useful when the upstream uses DNS for logical routing (e.g., geographic steering) and you want a single logical backend rather than per-IP backends.

```ferron
example.com {
    proxy {
        upstream http://myapp.example.com:8080 {
            logical_dns
        }
    }
}
```

## Forwarding headers

The reverse proxy module automatically manages standard forwarding headers:

| Header | Behavior |
| --- | --- |
| `X-Forwarded-For` | When `client_ip_from_header` is enabled, appends the extracted client IP to the existing chain. Otherwise, sets it to the direct connecting peer IP. |
| `X-Forwarded-Proto` | Always set to the incoming request scheme (`http` or `https`). |
| `X-Real-IP` | Always set to the client IP. |
| `Forwarded` (RFC 7239) | When `client_ip_from_header` is enabled, appends a new element (`for=...;proto=...;by=...`). Otherwise, sets a single element. IPv6 addresses are quoted per RFC 7239. |

## Trace context injection

When a trace context exists for the request, the reverse proxy module automatically injects W3C Trace Context headers into the outgoing upstream request. This enables end-to-end distributed tracing across Ferron and your backend services.

| Header | Behavior |
| --- | --- |
| `traceparent` | Always injected when a trace context is present. Format: `00-{trace_id}-{span_id}-{flags}`. |
| `tracestate` | Injected only if the incoming request or Ferron's trace context carries a non-empty `tracestate` value. |
| `baggage` | Injected only if the incoming request or Ferron's trace context carries non-empty `baggage` values. |

Trace context injection happens after all `request_header` transformations are applied. This means:

- You can override the injected headers using `request_header +traceparent "..."` to add a custom value.
- You can remove injected headers using `request_header -traceparent` to suppress propagation.
- The injected headers cannot be removed by `headers_to_remove` since injection occurs last.

> [!info]
> By default, incoming `traceparent` headers are discarded. Trace context is created when `http { trace { generate true } }` (the default) is active and trace sinks are configured. To trust incoming trace context, enable `trust_request true` inside the `trace` block. See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details.

## Connection pooling

Ferron maintains a keep-alive connection pool for upstream backends. Key behaviors:

- **Connection reuse** - pooled connections are automatically reused for subsequent requests to the same upstream.
- **Idle eviction** - connections idle longer than `idle_timeout` are evicted from the pool.
- **HTTP/2 multiplexing** - HTTP/2 connections share a single TCP connection for multiple concurrent requests.

> [!tip]
> If you get 502 errors from backends, verify the `upstream` URLs are reachable and check circuit breaker settings (`max_fails`).

## Health checking

### Circuit breaking

Circuit breaking provides passive health checking — tracking request-time failures per backend without background probes. The circuit breaker records transport failures (TCP connect errors, TLS errors) and optionally upstream `5xx` responses, then temporarily ejects unstable backends from the load balancer.

1. Transport failures and (optionally) upstream `5xx` responses are counted per backend in a rolling window.
2. When the backend reaches `max_fails` failures within `window`, Ferron opens the circuit and stops selecting that backend.
3. After `open_duration`, Ferron allows a single half-open trial request to the backend.
4. If the trial request succeeds, Ferron closes the circuit after `consecutive_passes` successful half-open requests.
5. If the half-open trial request fails, Ferron reopens the circuit immediately.

By default, only transport failures count toward the circuit. Set `record_5xx true` to also count upstream `5xx` responses.

Circuit breaking does not automatically retry upstream `5xx` responses. It only changes which backends are eligible for future requests.

> [!note]
>
> - Half-open recovery allows only one trial request at a time. If recovery is too aggressive for your workload, increase `open_duration` or `consecutive_passes`.
> - Circuit breaking and active health checks work together — either can make a backend temporarily ineligible.

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
            record_5xx true
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

## DNS result caching

When strict DNS or SRV resolution is enabled, Ferron caches resolved DNS results in memory with TTL-based expiry. The cache key includes the hostname (strict DNS) or SRV name, port, and DNS server list, ensuring that different resolver configurations are not served stale results.

The cached TTL is derived from the minimum TTL across all DNS response records, with a 30-second fallback when no TTL is available. This prevents stale backends from remaining in the pool while avoiding unnecessary DNS resolution for high-traffic hostnames.

Cache entries are evicted lazily — expired entries are treated as cache misses and re-resolved on demand. A periodic background task removes expired entries every 60 seconds to prevent unbounded memory growth for hostnames no longer queried.

> [!info]
> Cache metrics are available as `ferron.proxy.dns.cache_hit` and `ferron.proxy.dns.cache_miss` counters.

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://myapp.example.com:8080 {
            dns_servers "8.8.8.8,8.8.4.4"
        }
    }
}
```

In this example, the strict DNS resolution for `myapp.example.com` is cached. Subsequent requests use the cached result until the DNS TTL expires, reducing DNS resolution overhead.



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
            record_5xx true
        }
    }
}
```

## Observability

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.proxy.backends.selected` | Counter | backend URL or unix socket path | Backends selected during load balancing |
| `ferron.proxy.backends.unhealthy` | Counter | backend URL or unix socket path; `ferron.proxy.health_check_type` (`"active"` for health check probe failures, `"circuit_breaker"` for opened request-time circuits) | Backends marked as unhealthy |
| `ferron.proxy.requests` | Counter | `ferron.proxy.connection_reused` (`true`/`false`), `http.response.status_code`, `ferron.proxy.status_code` | Upstream proxy requests completed |
| `ferron.proxy.tls_handshake_failures` | Counter | backend URL or unix socket path | TLS handshake failures with upstream backends |
| `ferron.proxy.pool.waits` | Counter | backend URL or unix socket path | Times the connection pool was exhausted and a request had to wait |
| `ferron.proxy.pool.wait_time` | Histogram | backend URL or unix socket path | Duration spent waiting for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s |
| `ferron.proxy.lb.active_connections` | Gauge | backend URL or unix socket path | Active tracked connections for the selected backend |
| `ferron.proxy.lb.ewma_latency` | Gauge | backend URL or unix socket path | Current EWMA response latency for the selected backend (`p2c_ewma` algorithm) |
| `ferron.proxy.lb.warmup_state` | Gauge | backend URL or unix socket path | Whether the selected backend is in EWMA warm-up phase (1) or settled (0) |
| `ferron.proxy.lb.selections` | Counter | backend URL or unix socket path; `ferron.proxy.lb.reason` (`"p2c_ewma"`); `ferron.proxy.lb.score` (combined adaptive score) | P2C+EWMA backend selection with combined score |
| `ferron.proxy.backend.excluded` | Counter | backend URL or unix socket path; `ferron.proxy.reason` (`"circuit_open"`, `"already_tried"`, `"overloaded"`) | Backend excluded from selection |
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
| Reverse proxy: `<error type>` | ERROR | `error.type` (string) — error type classification, `error.message` (string) — error details |
| Upstream marked unhealthy | WARN  | `upstream.address` (string) — backend server URL |
| Upstream recovered      | INFO  | `upstream.address` (string) — backend server URL |

### Trace spans

The reverse proxy stage sets the following attributes on its `ferron.stage.reverse_proxy` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `http.response.status_code` | int | HTTP status code returned by the upstream backend. |
| `error.type` | string | Error type string on failure (e.g., `connection_refused`, `timeout`), enabling trace UI highlighting. |
| `ferron.proxy.backend_url` | string | URL of the upstream backend selected for the request. |
| `ferron.proxy.backend_unix_path` | string | Unix socket path of the backend, when using Unix sockets. |
| `ferron.proxy.connection_reused` | bool | Whether the connection to the backend was reused from the pool. |
| `ferron.proxy.retry_count` | int | Number of retry attempts made during the request. |

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

### TLS verification

- **`proxy { no_verification }`** — Disabling TLS certificate verification for HTTPS upstreams should only be used for testing or tightly controlled internal networks.
- **`active_check { no_verification }`** — Disabling TLS verification for health check probes should only be used for strictly internal endpoints.

### Upstream SSRF risk

- **Upstream URL with request header interpolation** — Upstream URLs containing `{{request.header.*}}` are vulnerable to SSRF. Derive upstream targets from static configuration or trusted server-controlled variables.
