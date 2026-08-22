---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, upstream backends, header manipulation, and connection pooling directives."
---

This page documents directives for forwarding incoming HTTP requests to one or more upstream backend servers. These directives support load balancing, connection pooling with keep-alive reuse, health checking, circuit breaking, and TLS upstream connections.

## Directives

### Reverse proxy and load balancing

- `proxy` (`http-proxy`)
  - This directive configures the reverse proxy with one or more upstream backends. You can use block form with nested directives or shorthand form with upstreams as arguments. Default: none
- `upstream <url: string>` (`http-proxy`)
  - This directive specifies a backend upstream server URL. It accepts `http://` or `https://` URLs. You can nest it inside a `proxy` block with optional `limit`, `idle_timeout`, `unix`, `logical_dns`, and `dns_servers` properties. When the URL contains a hostname, Ferron resolves A/AAAA records via Hickory DNS by default. This strict DNS mode creates a separate backend per resolved IP. Default: none
- `srv <name: string>` (`http-proxy`, requires `srv-lookup` feature)
  - This directive specifies a dynamic upstream that Ferron resolves via DNS SRV records. It supports `dns_servers`, `limit`, and `idle_timeout` nested directives. Default: none
- `algorithm <algorithm: string>` (`http-proxy`)
  - This directive specifies the load balancing strategy. Supported values: `random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`. Default: `algorithm two_random`
- `circuit_breaker [bool: boolean]` (`http-proxy`)
  - This directive enables request-time circuit breaking for backends. Transport failures always count toward tripping the circuit. Upstream `5xx` responses count only when you enable `record_5xx`. Slow responses count when you set `latency_threshold`. It supports nested `max_fails`, `window`, `open_duration`, `consecutive_passes`, `record_5xx`, `latency_threshold`, `flapping_transitions`, `flapping_window`, and `slow_start` directives. Default: `circuit_breaker true`
- `retry_connection [bool: boolean]` (`http-proxy`)
  - This directive specifies whether to retry on connection or HTTP request (if request body wasn't sent yet) failure if alternative backends are available. Default: `retry_connection true`
- `retry_budget [bool: boolean]` (`http-proxy`)
  - This directive enables a token-bucket retry budget that limits retries to a share of steady-state traffic. When you enable it alongside `retry_connection true`, retries consume tokens from a shared pool. Successful requests replenish the pool. If the retry budget runs out, Ferron refuses further retries and the request immediately returns `503 Service Unavailable`. This prevents cascading retry storms from overwhelming remaining healthy backends. It supports `max_retry_rate`, `max_tokens`, and `refill_rate` nested directives. Default: `retry_budget false`
- `max_retries_per_upstream <count: integer>` (`http-proxy`)
  - This directive sets how many times Ferron retries the **same** upstream on a transport or connection failure before it falls back to another backend via `retry_connection`. Only requests that can be replayed (idempotent methods with a buffered body that has not been sent yet) are retried. Each retry consumes a token from `retry_budget` when it is enabled. Set to `0` to disable same-upstream retries and fall back immediately. Default: `max_retries_per_upstream 1`
- `metrics_resolved_ip [bool: boolean]` (`http-proxy`)
  - This directive controls whether Ferron includes the `ferron.proxy.backend_resolved_ip` and `ferron.proxy.dns_status` attributes in proxy metrics and access logs. When `false` (default), metrics identify backends by their configured URL and optional Unix socket path only. This keeps metric cardinality low. When `true`, each resolved IP address becomes a distinct metric label value. A `ferron.proxy.dns_status` attribute indicates the DNS resolution outcome (`resolved`, `nxdomain`, `dns_error`, `logical_dns`, `static`). Enable this only when you need per-IP metric granularity and the IP set is stable. Default: `metrics_resolved_ip false`

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

In this example, the first backend receives approximately 62.5% of requests (5/8). The second receives 25% (2/8), and the third receives 12.5% (1/8). The smooth weighted round-robin algorithm distributes requests evenly over time. It does not send all requests to one backend before moving to the next.

#### Circuit breaker nested directives

| Nested directive       | Arguments               | Description                                                                                                                                                                                                                                                                                                       | Default  |
| ---------------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| `max_fails`            | `<count: integer>`      | Number of transport failures (and `5xx` responses when you enable `record_5xx`) within the rolling `window` needed to open the circuit.                                                                                                                                                                           | 5        |
| `window`               | `<duration: string>`    | Rolling time window used for counting breaker failures.                                                                                                                                                                                                                                                           | `30s`    |
| `open_duration`        | `<duration: string>`    | How long the circuit stays open before you can send a half-open trial request.                                                                                                                                                                                                                                    | `30s`    |
| `consecutive_passes`   | `<count: integer>`      | Number of successful half-open trial requests required to close the circuit again.                                                                                                                                                                                                                                | 1        |
| `record_5xx`           | `[bool: boolean]`       | Whether upstream `5xx` responses count toward tripping the circuit. Transport failures always count.                                                                                                                                                                                                              | `false`  |
| `latency_threshold`    | `<threshold: duration>` | Upstream response time threshold. Responses exceeding this duration count as failures toward tripping the circuit, alongside transport failures and (optionally) `5xx` responses. Uses duration strings (for example `"0.1s"`, `"0.5s"`).                                                                         | disabled |
| `flapping_transitions` | `<count: integer>`      | Number of circuit breaker state transitions within `flapping_window` required to mark an upstream as flapping. When flapping, Ferron suppresses individual transition logs and emits a single warning. Set to `0` to disable flapping detection.                                                                  | 3        |
| `flapping_window`      | `<duration: string>`    | Time window for counting state transitions for flapping detection.                                                                                                                                                                                                                                                | `10s`    |
| `slow_start`           | `<duration: string>`    | Length of slow-start window after a backend's circuit breaker recovers (half-open → closed). During slow-start, the load balancer applies a decaying virtual connection penalty to prevent thundering herd. The recovered backend appears busier than it is until real traffic catches up. Set to `0` to disable. | `0`      |

#### Retry budget nested directives

| Nested directive | Arguments          | Description                                                                                                                                   | Default     |
| ---------------- | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `max_retry_rate` | `<rate: float>`    | Maximum retry rate as a share of total requests (0.0–1.0). When exceeded, Ferron refuses further retries with `503 Service Unavailable`.      | `0.1` (10%) |
| `max_tokens`     | `<count: integer>` | Maximum number of tokens in the bucket (burst capacity). Controls how many retries can happen in a short burst before the rate limit applies. | `10`        |
| `refill_rate`    | `<rate: float>`    | Tokens added per second to the bucket. Higher values allow retries to recover faster after sustained traffic.                                 | `2.0`       |

**Configuration example:**

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        algorithm round_robin
        retry_connection true
        retry_budget {
            max_retry_rate 0.1
            max_tokens 10
            refill_rate 2.0
        }
    }
}
```

#### SSRF risk with interpolated upstream URLs

The upstream URL supports [interpolation syntax](/docs/v3/configuration/fundamentals/conditionals#built-in-variables) for dynamic values. **Never use user-controlled request headers** in upstream URLs. Examples include `request.header.host`, `request.header.x_forwarded_host`, and `request.header.x_forwarded_proto`. An attacker can craft requests to redirect the proxy to internal services.

**Unsafe: user-controlled header in upstream URL:**

```ferron
example.com {
    # DANGEROUS: attacker can set X-Forwarded-Host to 169.254.169.254 or any internal host
    proxy "http://{{request.header.x_forwarded_host}}:8080"
}
```

**Safe: static upstream URL:**

```ferron
example.com {
    proxy http://localhost:8080
}
```

**Safe: upstream URL derived from trusted, server-controlled variables:**

```ferron
example.com {
    # Safe: Ferron resolves request.host via its TLS/SNI matcher, not user-controlled
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
  - This directive specifies whether you enable HTTP keep-alive connection pooling. Default: `keepalive true`
- `http2 [bool: boolean]` (`http-proxy`)
  - This directive specifies whether you enable HTTP/2 for upstream connections. Default: `http2 false`
- `http2_only [bool: boolean]` (`http-proxy`)
  - This directive specifies whether to use only HTTP/2 for upstream connections. Default: `http2_only false`
- `intercept_errors [bool: boolean]` (`http-proxy`)
  - This directive specifies whether Ferron intercepts upstream error responses (4xx/5xx) and replaces them with built-in error pages. When `true`, Ferron replaces upstream error responses with its own error pages. When `false` (default), Ferron passes the full upstream response body and headers through unchanged. Default: `intercept_errors false`

### TLS

- `no_verification [bool: boolean]` (`http-proxy`)
  - This directive specifies whether to disable TLS certificate checks for HTTPS upstreams. Default: `no_verification false`

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

You must provide both `cert` and `key` for mTLS to activate. The certificate chain and private key must be PEM-encoded. Ferron scopes mTLS credentials per-upstream. Different backends can require different client certificates. Active health check probes also use the configured mTLS credentials. Ferron caches mTLS credentials in memory until configuration reload or server shutdown.

### PROXY protocol

- `proxy_header <version: string>` (`http-proxy`)
  - This directive specifies whether to prepend an HAProxy PROXY protocol header to upstream connections. Supported versions: `v1`, `v2`. Default: disabled

### Header manipulation

- `request_header` (`http-proxy`)
  - This directive manipulates request headers before forwarding to upstream. It supports three forms:
    - `request_header +Name "value"`: **add** header (appends, allows duplicates)
    - `request_header -Name`: **remove** all instances of the header
    - `request_header Name "value"`: **replace** header (removes existing, sets new value)
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
  - This directive specifies the global maximum number of concurrent TCP connections in the keep-alive connection pool across all upstream backends. Unix socket connections are always unbounded. Default: `proxy_concurrent_conns 16384`

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

| Nested directive     | Arguments           | Description                                                                                                                                                                                                 | Default   |
| -------------------- | ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| `limit`              | `<number>`          | Maximum concurrent connections to this specific upstream.                                                                                                                                                   | unlimited |
| `idle_timeout`       | `<duration>`        | Keep-alive idle timeout. Ferron evicts connections idle longer than this from the pool.                                                                                                                     | `60s`     |
| `connection_timeout` | `<duration\|false>` | Maximum time to wait for a TCP connection to establish. Set to `false` to disable.                                                                                                                          | `5s`      |
| `unix`               | `<path>`            | Connect via Unix domain socket instead of TCP. The URL scheme remains required.                                                                                                                             | TCP       |
| `weight`             | `<number>`          | Weight for weighted load balancing. Higher values receive more requests. Supported by all load balancing algorithms (`random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`) and session affinity. | 1         |
| `priority`           | `<number>`          | Priority for tiered failover. Lower values are higher priority. When the highest-priority tier has no available backends, Ferron tries the next tier.                                                       | 0         |
| `cert`               | `<path: string>`    | Path to a PEM file containing the client certificate chain to present to the upstream server for mTLS. Use together with `key`.                                                                             | disabled  |
| `key`                | `<path: string>`    | Path to a PEM file containing the client private key for mTLS. Use together with `cert`.                                                                                                                    | disabled  |
| `logical_dns`        | `[bool: boolean]`   | When `true`, disables strict A/AAAA DNS resolution and uses the system's logical DNS resolution instead. Ferron passes the upstream URL through as-is without per-IP backend splitting.                     | `false`   |
| `dns_servers`        | `<string>`          | Comma-separated DNS server IPs used for strict A/AAAA resolution. Uses Hickory's default resolvers if empty.                                                                                                | system    |

### `srv`

Defines a dynamic upstream that Ferron resolves via DNS SRV records.

```ferron
example.com {
    srv _http._tcp.example.com {
        dns_servers "8.8.8.8,8.8.4.4"
        limit 100
        idle_timeout "30s"
    }
}
```

| Nested directive     | Arguments           | Description                                                                                                                                                                                                                                                                     | Default   |
| -------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| `dns_servers`        | `<string>`          | Comma-separated DNS server IPs. Uses system resolver if empty.                                                                                                                                                                                                                  | system    |
| `limit`              | `<number>`          | Maximum concurrent connections per resolved backend.                                                                                                                                                                                                                            | unlimited |
| `idle_timeout`       | `<duration>`        | Keep-alive idle timeout per resolved backend.                                                                                                                                                                                                                                   | `60s`     |
| `connection_timeout` | `<duration\|false>` | Maximum time to wait for a TCP connection to establish. Set to `false` to disable.                                                                                                                                                                                              | `5s`      |
| `weight`             | `<number>`          | Multiplier applied to DNS SRV weights. Each backend's effective weight is `dns_weight × config_weight`. Set to `1` to use DNS weights as-is. Supported by all load balancing algorithms (`random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`) and session affinity. | 1         |
| `priority`           | `<number>`          | Additive offset applied to DNS SRV priorities. Ferron calculates a backend's effective priority as `dns_priority + offset`. Ferron tries lower effective values first.                                                                                                          | 0         |
| `cert`               | `<path: string>`    | Path to a PEM file containing the client certificate chain to present to resolved backends for mTLS. Use together with `key`.                                                                                                                                                   | disabled  |
| `key`                | `<path: string>`    | Path to a PEM file containing the client private key for mTLS. Use together with `cert`.                                                                                                                                                                                        | disabled  |

## Load balancing algorithms

| Algorithm     | Description                                                                                                                                                                                                                                                                                       |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `random`      | Selects a backend randomly for each request, weighted by configured weights.                                                                                                                                                                                                                      |
| `round_robin` | Distributes requests proportionally to backend weights using smooth weighted round-robin.                                                                                                                                                                                                         |
| `least_conn`  | Selects the backend with the fewest active tracked connections multiplied by its weight.                                                                                                                                                                                                          |
| `two_random`  | Picks two random backends and selects the one with lower weighted load (connections divided by weight).                                                                                                                                                                                           |
| `p2c_ewma`    | Power of Two Choices with EWMA (Exponentially Weighted Moving Average) latency scoring. Picks two random backends. Selects the one with the lower combined score of EWMA response latency plus active connection penalty, divided by weight. Automatically adapts to backend performance changes. |

## Session affinity

Session affinity (sticky sessions) makes sure the same client's requests always go to the same backend server. This is useful for stateful applications, WebSocket-heavy workloads, and improving cache locality.

The `affinity` directive configures session affinity inside a `proxy` block. It supports four affinity types:

### Cookie affinity

Reads and sets a cookie to track which backend receives each client's requests. If no cookie is present, Ferron selects a backend and sets a cookie on the response.

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

| Nested directive | Arguments    | Description                                     | Default                  |
| ---------------- | ------------ | ----------------------------------------------- | ------------------------ |
| `name`           | `<string>`   | Cookie name.                                    | `ferron_sticky`          |
| `ttl`            | `<duration>` | Cookie time-to-live.                            | Session (browser closes) |
| `path`           | `<string>`   | Cookie path.                                    | `/`                      |
| `domain`         | `<string>`   | Cookie domain.                                  | Current domain           |
| `secure`         | `[bool]`     | Only send cookie over HTTPS.                    | `false`                  |
| `httponly`       | `[bool]`     | Prevent JavaScript access to cookie.            | `true`                   |
| `samesite`       | `<mode>`     | SameSite attribute: `strict`, `lax`, or `none`. | `lax`                    |

> [!important]
> If using reverse proxying with cookie affinity along with HTTP caching (caching reverse proxy), configure the cache `vary_cookies` setting to use the same cookie name as configured in the `proxy` block, otherwise cookie-based affinity will not work correctly.
>
> **Example cache configuration:**
>
> ```ferron
> example.com {
>     cache {
>         vary_cookies ferron_sticky
>     }
> }
> ```

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

> [!important]
> If using reverse proxy with header affinity along with HTTP caching (caching reverse proxy), configure the cache `vary` setting to use the same header name as configured in the `proxy` block, otherwise cookie-based affinity will not work correctly.
>
> **Example cache configuration:**
>
> ```ferron
> example.com {
>     cache {
>         vary "X-Backend-Id"
>     }
> }
> ```

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

> [!warning]
> Using HTTP caching alongside reverse proxying with IP affinity is not supported and may lead to correctness issues related to session affinity.

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

- Ferron respects affinity only when the target backend is healthy. If the affinity target is unhealthy, Ferron uses the configured load balancing algorithm as a fallback.
- When you enable `retry_connection` and the affinity-targeted backend fails, Ferron retries with another backend.
- Cookie affinity automatically sets the cookie on the first request if no valid cookie is present.
- Ferron uses the affinity key with a consistent hash ring for deterministic routing.

## Priority-based failover

You can assign numeric priority values to backends to implement tiered failover. Lower values indicate higher priority. When all backends in the highest-priority tier are unavailable, Ferron uses the next tier as a fallback. A backend is unavailable if it is unhealthy, has an open circuit, or was already tried.

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

Ferron routes requests to the primary tier (priority 0) first. If all primary backends are unavailable, Ferron tries the secondary tier (priority 1), and so on. Within each tier, the configured load balancing algorithm selects which backend handles the request. Priority and weight work together. Priority determines which tier Ferron tries. Weight determines how Ferron distributes requests within a tier.

For SRV upstreams, the `priority` subdirective is an additive offset applied to DNS SRV priorities. A backend's effective priority is `dns_priority + offset`. For example, if DNS returns priorities 10 and 20, setting `priority 5` on the SRV block shifts them to 15 and 25.

```ferron
example.com {
    proxy {
        srv _http._tcp.example.com {
            priority 5
        }
    }
}
```

## Strict DNS (A/AAAA) resolution

When an `upstream` URL contains a hostname instead of an IP literal, Ferron resolves A and AAAA records using Hickory DNS. Each resolved IP address becomes a separate backend in the load balancer. This enables per-IP load balancing, circuit breaking, and health checking.

For example, if `http://myapp.example.com:8080` resolves to three IPs (`10.0.0.1`, `10.0.0.2`, `10.0.0.3`), Ferron creates three distinct backends, one per IP. Ferron preserves the original hostname for TLS SNI and the HTTP `Host` header.

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

Set `logical_dns true` to disable strict A/AAAA resolution. Ferron passes the upstream URL through as-is, and the system's DNS resolver handles address selection. This is useful when the upstream uses DNS for logical routing (for example geographic steering). You then get a single logical backend rather than per-IP backends.

```ferron
example.com {
    proxy {
        upstream http://myapp.example.com:8080 {
            logical_dns
        }
    }
}
```

> [!important]
> When using Ferron with ephemeral network addresses (for example as a Kubernetes ingress controller), always define upstreams using DNS hostnames. Use service identifiers such as `http://my-service.default.svc.cluster.local:8080` rather than individual Pod IP addresses. DNS hostnames remain stable across pod restarts and scaling events. Pod IPs change frequently. Do not use individual Pod IPs directly as upstream URLs. Do not rely on per-IP metric dimensions (`metrics_resolved_ip true`). These can cause cardinality explosion in time-series databases. This degrades observability performance and increases storage costs.

## Forwarding headers

The reverse proxy module automatically manages standard forwarding headers:

| Header                 | Behavior                                                                                                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `X-Forwarded-For`      | When you enable `client_ip_from_header`, it appends the extracted client IP to the existing chain. Otherwise, it sets it to the direct connecting peer IP.                          |
| `X-Forwarded-Proto`    | Always set to the incoming request scheme (`http` or `https`).                                                                                                                      |
| `X-Real-IP`            | Always set to the client IP.                                                                                                                                                        |
| `Forwarded` (RFC 7239) | When you enable `client_ip_from_header`, it appends a new element (`for=...;proto=...;by=...`). Otherwise, it sets a single element. The module quotes IPv6 addresses per RFC 7239. |

## Trace context injection

When a trace context exists, the reverse proxy module injects W3C Trace Context headers into the upstream request. This enables distributed tracing across Ferron and your backend services.

| Header        | Behavior                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| `traceparent` | Always injected when a trace context is present. Format: `00-{trace_id}-{span_id}-{flags}`.             |
| `tracestate`  | Injected only if the incoming request or Ferron's trace context carries a non-empty `tracestate` value. |
| `baggage`     | Injected only if the incoming request or Ferron's trace context carries non-empty `baggage` values.     |

Trace context injection happens after Ferron applies all `request_header` transformations. This means:

- You can override the injected headers using `request_header +traceparent "..."` to add a custom value.
- You can remove injected headers using `request_header -traceparent` to suppress propagation.
- `headers_to_remove` cannot remove the injected headers since injection occurs last.

> [!info]
> By default, Ferron discards incoming `traceparent` headers. Ferron creates trace context when `http { trace { generate true } }` (the default) is active and you configure trace sinks. To trust incoming trace context, enable `trust_request true` inside the `trace` block. See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details.

## Connection pooling

Ferron maintains a keep-alive connection pool for upstream backends. Key behaviors:

- **Connection reuse** - Ferron automatically reuses pooled connections for later requests to the same upstream.
- **Idle eviction** - Ferron evicts connections idle longer than `idle_timeout` from the pool.
- **HTTP/2 multiplexing** - HTTP/2 connections share a single TCP connection for multiple concurrent requests.

> [!tip]
> If you get 502 errors from backends, verify the `upstream` URLs are reachable and check circuit breaker settings (`max_fails`).

## Health checking

### Circuit breaking

Circuit breaking provides passive health checking. It tracks request-time failures per backend without background probes. The circuit breaker records transport failures (TCP connect errors, TLS errors) and optionally upstream `5xx` responses. It then temporarily ejects unstable backends from the load balancer.

1. Ferron counts transport failures and (optionally) upstream `5xx` responses per backend in a rolling window.
2. When the backend reaches `max_fails` failures within `window`, Ferron opens the circuit and stops selecting that backend.
3. After `open_duration`, Ferron allows a single half-open trial request to the backend.
4. If the trial request succeeds, Ferron closes the circuit after `consecutive_passes` successful half-open requests.
5. If the half-open trial request fails, Ferron reopens the circuit immediately.

By default, only transport failures count toward the circuit. Set `record_5xx true` to also count upstream `5xx` responses. Set `latency_threshold` to also count slow responses.

Circuit breaking does not automatically retry upstream `5xx` responses. It only changes which backends are eligible for future requests.

> [!note]
>
> - Half-open recovery allows only one trial request at a time. If recovery is too aggressive for your workload, increase `open_duration` or `consecutive_passes`.
> - Circuit breaking and active health checks work together. Either can make a backend temporarily ineligible.

> [!tip]
> If a backend flaps, circuit breaking can protect the rest of the pool. It temporarily ejects the backend after repeated transport failures or upstream `5xx` responses.

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
            latency_threshold "0.5s"
        }
    }
}
```

### Active health checking

Active health checks proactively probe backend health on a schedule, independent of incoming traffic. This lets you detect backend failures quickly before they affect client requests.

You configure active health checks per-upstream inside an `active_check` block.

#### `active_check` nested directives

| Nested directive          | Arguments             | Description                                                                                                                                | Default   |
| ------------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | --------- |
| `uri`                     | `<path: string>`      | The endpoint to probe for health checks.                                                                                                   | `/health` |
| `method`                  | `<method: string>`    | HTTP method for probe requests. Supported values: `GET`, `HEAD`.                                                                           | `GET`     |
| `interval`                | `<duration: string>`  | Interval between health check probes.                                                                                                      | `10s`     |
| `timeout`                 | `<duration: string>`  | Maximum wait time for a probe response.                                                                                                    | `5s`      |
| `expect_status`           | `<status: string>`    | Expected HTTP status code(s) for a successful probe. Supports: `2xx`, `3xx`, `2xx,3xx`, specific codes (`200,204`), or ranges (`200-299`). | `2xx,3xx` |
| `response_time_threshold` | `<duration: string>`  | Optional response time threshold. If exceeded, Ferron marks the probe unhealthy.                                                           | disabled  |
| `body_match`              | `<substring: string>` | Optional substring to match in the response body (GET only).                                                                               | disabled  |
| `consecutive_fails`       | `<count: integer>`    | Number of consecutive failures before marking an upstream as unhealthy.                                                                    | 2         |
| `consecutive_passes`      | `<count: integer>`    | Number of consecutive successes before marking an upstream as healthy when recovering.                                                     | 2         |
| `no_verification`         | `[bool: boolean]`     | Whether to skip TLS certificate checks for HTTPS probes.                                                                                   | `false`   |

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
> For active health checks, make sure the probe endpoint is reachable on all backends. Keep probes lightweight. Use HEAD requests when the response body is not needed. If Ferron marks upstreams unhealthy incorrectly, check logs and verify `expect_status`.

## Retry budgets

The retry budget uses a token-bucket algorithm shared across all requests for a given proxy setup:

1. The bucket starts full with `max_tokens` tokens.
2. Each successful request deposits one token (up to capacity), replenishing retry capacity proportional to steady-state traffic.
3. Each retry consumes one token, this includes same-upstream retries (`max_retries_per_upstream`) and cross-backend failovers (`retry_connection`). If the bucket is empty, Ferron refuses the retry. The request then returns `503 Service Unavailable` with a `Retry-After` header indicating when the client should retry.
4. Ferron lazily refills tokens based on elapsed time and `refill_rate`.

This prevents retry storms. When multiple backends fail simultaneously, the retry budget caps the total retry amplification factor. For example, with `max_retry_rate 0.1` and three backends where two fail, at most ~10% of total traffic will be retries. The remaining healthy backend is not overwhelmed.

> [!note]
> Ferron scopes the retry budget per proxy config block. Different hosts or locations can have independent budgets. The budget does not add delays between retries. It limits the _count_ of retries, not their timing. For delay-based retry control, use circuit breakers with `open_duration`.

> [!tip]
> Start with the defaults (`max_retry_rate 0.1`, `max_tokens 10`, `refill_rate 2.0`) for most workloads. Increase `max_retry_rate` only if you see the retry budget rejecting legitimate transient failures. Increase `max_tokens` if your traffic pattern has bursty spikes that need more retry headroom.

## Observability

### Metrics

| Metric                                         | Type      | Attributes                                                                                                                                                                                                                         | Description                                                                                                                                                                                                                  |
| ---------------------------------------------- | --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ferron.proxy.backends.selected`               | Counter   | backend URL or unix socket path, optionally resolved IP address and `ferron.proxy.dns_status`                                                                                                                                      | Backends selected during load balancing                                                                                                                                                                                      |
| `ferron.proxy.backends.selected_per_request`   | Counter   | backend URL or unix socket path, optionally resolved IP address and `ferron.proxy.dns_status`                                                                                                                                      | Backends selected per request (including retries)                                                                                                                                                                            |
| `ferron.proxy.backends.unhealthy`              | Counter   | backend URL or unix socket path, optionally resolved IP address and `ferron.proxy.dns_status`, `ferron.proxy.health_check_type` (`"active"` for health check probe failures, `"circuit_breaker"` for opened request-time circuits) | Backends marked as unhealthy                                                                                                                                                                                                 |
| `ferron.proxy.requests`                        | Counter   | backend URL or unix socket path, optionally resolved IP address and `ferron.proxy.dns_status`, `ferron.proxy.connection_reused` (`true`/`false`), `http.response.status_code`                                                      | Upstream proxy requests completed                                                                                                                                                                                            |
| `ferron.proxy.tls_handshake_failures`          | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | TLS handshake failures with upstream backends                                                                                                                                                                                |
| `ferron.proxy.pool.waits`                      | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Times the connection pool ran out of connections and a request had to wait                                                                                                                                                   |
| `ferron.proxy.pool.wait_time`                  | Histogram | backend URL or unix socket path                                                                                                                                                                                                    | Duration spent waiting for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s                                                                                                                          |
| `ferron.proxy.lb.active_connections`           | Gauge     | backend URL or unix socket path                                                                                                                                                                                                    | Active tracked connections for the selected backend                                                                                                                                                                          |
| `ferron.proxy.lb.ewma_latency`                 | Gauge     | backend URL or unix socket path                                                                                                                                                                                                    | Current EWMA response latency for the selected backend (`p2c_ewma` algorithm)                                                                                                                                                |
| `ferron.proxy.lb.warmup_state`                 | Gauge     | backend URL or unix socket path                                                                                                                                                                                                    | Whether the selected backend is in EWMA warm-up phase (1) or settled (0)                                                                                                                                                     |
| `ferron.proxy.lb.score`                        | Gauge     | backend URL or unix socket path, resolved IP address                                                                                                                                                                               | Combined load-balancer selection score for the selected backend. Lower = more preferred. Emitted for `two_random` (weighted connection count) and `p2c_ewma` (EWMA latency + connection penalty) algorithms.                 |
| `ferron.proxy.backends.excluded`               | Counter   | backend URL or unix socket path, optionally resolved IP address and `ferron.proxy.dns_status`, `ferron.proxy.reason` (`"circuit_open"`, `"already_tried"`, `"overloaded"`)                                                         | Backend excluded from selection                                                                                                                                                                                              |
| `ferron.proxy.retry.count`                     | Counter   | backend URL or unix socket path, `http.request.method`, `ferron.proxy.method_idempotent`                                                                                                                                           | Number of retry attempts made for a request (includes same-upstream and cross-backend retries)                                                                                                                               |
| `ferron.proxy.same_upstream_retry.count`       | Counter   | backend URL or unix socket path, `http.request.method`, `ferron.proxy.method_idempotent`                                                                                                                                           | Number of times Ferron retried the **same** upstream on a transport failure before it tried another backend. Ferron increments this together with `ferron.proxy.retry.count`. Use it to tell transient blips from failovers. |
| `ferron.proxy.retry.final`                     | Gauge     | backend URL or unix socket path, `http.request.method`, `ferron.proxy.method_idempotent`                                                                                                                                           | Whether the final retry attempt succeeded (`1`) or failed (`0`)                                                                                                                                                              |
| `ferron.proxy.retry.budget_exhausted`          | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Number of requests where Ferron refused a retry because the retry budget ran out                                                                                                                                             |
| `ferron.proxy.retry.budget_tokens_available`   | Gauge     | backend URL or unix socket path                                                                                                                                                                                                    | Current available retry budget tokens                                                                                                                                                                                        |
| `ferron.proxy.pool.hit`                        | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Pooled connection reused successfully                                                                                                                                                                                        |
| `ferron.proxy.pool.miss`                       | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Pooled connection unavailable, new connection established                                                                                                                                                                    |
| `ferron.proxy.pool.idle`                       | Gauge     | backend URL or unix socket path, `worker` (thread identifier)                                                                                                                                                                      | Current number of idle connections in the pool                                                                                                                                                                               |
| `ferron.proxy.pool.outstanding`                | Gauge     | backend URL or unix socket path, `worker` (thread identifier)                                                                                                                                                                      | Current number of outstanding (in-use) connections in the pool                                                                                                                                                               |
| `ferron.proxy.pool.local_limit`                | Gauge     | backend URL or unix socket path                                                                                                                                                                                                    | Current local connection limit for reverse proxy                                                                                                                                                                             |
| `ferron.proxy.pool.global_limit`               | Gauge     | None                                                                                                                                                                                                                               | Current global connection limit for reverse proxy                                                                                                                                                                            |
| `ferron.proxy.connect.latency`                 | Histogram | backend URL or unix socket path                                                                                                                                                                                                    | Time to establish a TCP/TLS connection to the backend                                                                                                                                                                        |
| `ferron.proxy.ttfb`                            | Histogram | backend URL or unix socket path                                                                                                                                                                                                    | Time to first response byte from the backend                                                                                                                                                                                 |
| `ferron.proxy.health.success`                  | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Health check probe succeeded                                                                                                                                                                                                 |
| `ferron.proxy.health.failure`                  | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Health check probe failed                                                                                                                                                                                                    |
| `ferron.proxy.health.duration`                 | Histogram | backend URL or unix socket path                                                                                                                                                                                                    | Health check probe duration                                                                                                                                                                                                  |
| `ferron.proxy.circuit.state`                   | Gauge     | backend URL or unix socket path, optionally resolved IP address (when `metrics_resolved_ip true`) and `ferron.proxy.dns_status`                                                                                                    | Circuit breaker state: `0` Closed, `1` Open, `2` HalfOpen                                                                                                                                                                    |
| `ferron.proxy.circuit.open_total`              | Counter   | backend URL or unix socket path, optionally resolved IP address (when `metrics_resolved_ip true`) and `ferron.proxy.dns_status`                                                                                                    | Number of times the circuit breaker has transitioned to Open state                                                                                                                                                           |
| `ferron.proxy.circuit.flapping`                | Gauge     | backend URL or unix socket path, optionally resolved IP address (when `metrics_resolved_ip true`) and `ferron.proxy.dns_status`                                                                                                    | Whether an upstream backend flaps (`1` = flapping, `0` = stable)                                                                                                                                                             |
| `ferron.proxy.failures`                        | Counter   | `http.response.status_code` (HTTP response status code), `error.type` (error type classification)                                                                                                                                  | Reverse-proxy failures that returned an error before producing a backend response                                                                                                                                            |
| `ferron.proxy.dns.cache_hit`                   | Counter   | None                                                                                                                                                                                                                               | DNS cache hits for strict DNS and SRV lookups                                                                                                                                                                                |
| `ferron.proxy.dns.cache_miss`                  | Counter   | None                                                                                                                                                                                                                               | DNS cache misses (miss or expiry) for strict DNS and SRV lookups                                                                                                                                                             |
| `ferron.proxy.dns.cache_ttl_remaining_seconds` | Gauge     | `aggregation` (`min`, `max`, or `avg`)                                                                                                                                                                                             | Aggregated remaining TTL across all DNS cache entries                                                                                                                                                                        |
| `ferron.proxy.dns.cache_entries`               | Gauge     | None                                                                                                                                                                                                                               | Number of active entries in the DNS cache                                                                                                                                                                                    |
| `ferron.proxy.upstream.response_truncated`     | Counter   | backend URL or unix socket path                                                                                                                                                                                                    | Upstream responses that ended before the declared Content-Length                                                                                                                                                             |

### Logs

- **`ERROR`**: Ferron logs this when a proxy setup error occurs during parsing. The message includes the error details.
- **`ERROR`**: Ferron logs this when a proxy execution error occurs (for example connection failure, transport error). The message includes the error type and details.
- **`WARN`**: Ferron logs this when active health checks mark an upstream unhealthy. The message includes the upstream address and failure reason.
- **`INFO`**: Ferron logs this when an upstream recovers after consecutive successful health check probes.
- **`DEBUG`**: Ferron logs this when it initializes a health check loop for a given upstream.

### Structured logs

| Description (summary)                                   | Level | Attributes                                                                                                                                                           |
| ------------------------------------------------------- | ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Reverse proxy config error                              | ERROR | `error.message` (string): setup error details                                                                                                                        |
| Reverse proxy: `<error type>`                           | ERROR | `error.type` (string): error type classification, `error.message` (string): error details                                                                            |
| Upstream marked unhealthy                               | WARN  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream recovered                                      | INFO  | `upstream.address` (string): backend server URL                                                                                                                      |
| Initializing health check                               | DEBUG | `ferron.proxy.health.address` (string): backend identifier, `ferron.proxy.health.method` (string): HTTP method, `ferron.proxy.health.uri` (string): health check URI |
| Upstream circuit opened                                 | WARN  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream circuit closed                                 | INFO  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream circuit reopened after half-open trial failure | WARN  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream flapping                                       | WARN  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream flapping resolved                              | INFO  | `upstream.address` (string): backend server URL                                                                                                                      |
| Upstream circuit transitioned to half-open              | INFO  | `upstream.address` (string): backend server URL, `ferron.proxy.circuit.open_duration_ms`: open duration in milliseconds                                              |
| Upstream response truncated                             | WARN  | `ferron.proxy.backend_url` (string): backend server URL, `upstream.bytes_received` (int): bytes received, `upstream.content_length` (int): expected Content-Length   |

#### Access log fields

The reverse proxy module contributes the following fields to the HTTP access log line:

| Field                                    | Type   | Description                                                             |
| ---------------------------------------- | ------ | ----------------------------------------------------------------------- |
| `ferron.proxy.backend_url`               | string | Backend URL that served the proxied request.                            |
| `ferron.proxy.backend_resolved_ip`       | string | Resolved IP address of the backend (strict DNS only).                   |
| `ferron.proxy.backend_unix_path`         | string | Unix socket path of the backend (if applicable).                        |
| `ferron.proxy.connection_reused`         | bool   | Whether the module reused a pooled connection.                          |
| `ferron.proxy.retry_count`               | int    | Number of retry attempts (0 if none). Includes same-upstream retries.   |
| `ferron.proxy.same_upstream_retry_count` | int    | Number of times Ferron retried the same upstream (0 if none).           |
| `ferron.proxy.circuit_breaker_state`     | string | Circuit breaker state of the backend: `closed`, `open`, or `half_open`. |

### Trace spans

The reverse proxy stage sets the following attributes on its `ferron.stage.reverse_proxy` span:

| Attribute                                    | Type   | Description                                                                                                                                   |
| -------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `http.response.status_code`                  | int    | HTTP status code returned by the upstream backend.                                                                                            |
| `error.type`                                 | string | Error type string on failure (for example `connection_refused`, `timeout`), enabling trace UI highlighting.                                   |
| `ferron.proxy.backend_url`                   | string | URL of the upstream backend selected for the request.                                                                                         |
| `ferron.proxy.backend_unix_path`             | string | Unix socket path of the backend, when using Unix sockets.                                                                                     |
| `ferron.proxy.connection_reused`             | bool   | Whether the module reused the connection to the backend from the pool.                                                                        |
| `ferron.proxy.retry_count`                   | int    | Number of retry attempts made during the request (includes same-upstream retries).                                                            |
| `ferron.proxy.same_upstream_retry_count`     | int    | Number of times Ferron retried the same upstream during the request.                                                                          |
| `ferron.proxy.upstream.circuit_state`        | string | Circuit breaker state of the selected backend: `closed`, `open`, or `half_open`.                                                              |
| `ferron.proxy.upstream.is_flapping`          | bool   | Whether the selected backend is currently flapping (rapidly oscillating circuit breaker states).                                              |
| `ferron.proxy.upstream.slow_start`           | bool   | Whether the selected backend is in slow-start (circuit breaker recently recovered). Only present when you configure `slow_start`.             |
| `ferron.proxy.upstream.health_status`        | string | Active health check status of the selected backend: `healthy` or `unhealthy`. Only present when you configure health checks for the upstream. |
| `ferron.proxy.upstream.consecutive_failures` | int    | Number of consecutive health check failures for the selected backend. Only present when you configure health checks.                          |
| `ferron.proxy.upstream.active_connections`   | int    | Approximate number of active connections to the selected backend at the time of routing.                                                      |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

### TLS verification

- **`proxy { no_verification }`**: Only disable TLS certificate checks for HTTPS upstreams in testing or tightly controlled internal networks.
- **`active_check { no_verification }`**: Only disable TLS checks on health check probes for strictly internal endpoints.

### Upstream SSRF risk

- **Upstream URL with request header interpolation**: Upstream URLs containing `{{request.header.*}}` are vulnerable to SSRF. Derive upstream targets from static setup or trusted server-controlled variables.
