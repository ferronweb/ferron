---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, upstream backends, header manipulation, health checks, and observability."
---

`proxy` forwards requests to one or more upstream backends. The reverse proxy supports load balancing, connection pooling with keep-alive reuse, health checks, circuit breaking, retries, session affinity, and TLS upstream connections.

## `proxy`

`proxy` configures the reverse proxy. Use block form with nested directives, or shorthand form with upstreams as arguments. Default: none.

> [!note]
> Host isolation: a named host does not inherit `proxy` from the wildcard `*` host. It inherits from global defaults. A `location` block inherits it from its own host.

| Directive                  | Arguments          | Description                                                                                                                                                                                                                             | Default      |
| -------------------------- | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| `proxy`                    | `<url>` or block   | Reverse proxy with one or more upstreams.                                                                                                                                                                                               | none         |
| `upstream`                 | `<url: string>`    | Backend URL. Accepts `http://` and `https://`. Nested directives: `limit`, `idle_timeout`, `unix`, `logical_dns`, `dns_servers`. A hostname resolves A/AAAA records through Hickory DNS by default, with one backend per resolved IP. | none         |
| `srv`                      | `<name: string>`   | Dynamic upstream resolved from DNS SRV records. Nested directives: `dns_servers`, `limit`, `idle_timeout`.                                                                                                                              | none         |
| `algorithm`                | `<algorithm>`      | Load balancing strategy: `random`, `round_robin`, `least_conn`, `two_random`, `p2c_ewma`.                                                                                                                                               | `two_random` |
| `circuit_breaker`          | `[bool: boolean]`  | Request-time circuit breaking per backend. Transport failures always count. Upstream `5xx` responses count with `record_5xx`. Slow responses count with `latency_threshold`. Nested directives are below.                               | `true`       |
| `retry_connection`         | `[bool: boolean]`  | Retry on a connection or HTTP request failure, if Ferron has not sent the request body yet and another backend is available.                                                                                                            | `true`       |
| `retry_budget`             | `[bool: boolean]`  | Token-bucket retry budget that limits retries to a share of steady-state traffic. Successful requests replenish the pool. When the budget runs out, Ferron refuses the retry and returns `503 Service Unavailable`.                     | `false`      |
| `max_retries_per_upstream` | `<count: integer>` | Retries of the same upstream on a transport or connection failure before failover through `retry_connection`. Only replayable requests are retried: idempotent methods with a buffered body that Ferron has not sent.                   | `1`          |
| `retry_interval`           | `<duration>`       | Delay between same-upstream retries, plus up to 25% jitter. Masks brief backend restarts such as Docker Compose recreates or Apache graceful reloads. `"0s"` retries immediately.                                                      | `"0.2s"`     |
| `metrics_resolved_ip`      | `[bool: boolean]`  | Include `ferron.proxy.backend_resolved_ip` in proxy metrics and access logs. `false` identifies backends by configured URL and optional Unix socket path, which keeps cardinality low. `true` gives each resolved IP its own label.     | `false`      |

Every backend-scoped proxy metric also carries `ferron.proxy.dns_status`: `resolved`, `nxdomain`, `dns_error`, `logical_dns`, or `static`. The value set is fixed, so it does not raise cardinality like the resolved address does. Ferron reports it regardless of `metrics_resolved_ip`.

`max_retries_per_upstream` retries only requests it can replay. Each retry consumes a `retry_budget` token when the budget is enabled. Set it to `0` to disable same-upstream retries and fail over immediately.

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

### Circuit breaker nested directives

| Nested directive       | Arguments               | Description                                                                                                                                                                                                                       | Default  |
| ---------------------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| `max_fails`            | `<count: integer>`      | Transport failures, and `5xx` responses when `record_5xx` is enabled, within the rolling `window` needed to open the circuit.                                                                                                     | 5        |
| `window`               | `<duration: string>`    | Rolling window for counting breaker failures.                                                                                                                                                                                     | `30s`    |
| `open_duration`        | `<duration: string>`    | How long the circuit stays open before Ferron sends a half-open trial request.                                                                                                                                                    | `5s`     |
| `consecutive_passes`   | `<count: integer>`      | Successful half-open trial requests required to close the circuit.                                                                                                                                                                | 1        |
| `record_5xx`           | `[bool: boolean]`       | Count upstream `5xx` responses toward tripping the circuit. Transport failures always count.                                                                                                                                      | `false`  |
| `latency_threshold`    | `<duration>`            | Response time threshold. Slower responses count as failures toward tripping the circuit, alongside transport failures and optional `5xx` responses. Use duration strings such as `"0.1s"` or `"0.5s"`.                              | disabled |
| `flapping_transitions` | `<count: integer>`      | State transitions within `flapping_window` that mark an upstream as flapping. When flapping, Ferron suppresses individual transition logs and emits one warning. `0` disables flapping detection.                                  | 3        |
| `flapping_window`      | `<duration: string>`    | Time window for counting state transitions for flapping detection.                                                                                                                                                                | `10s`    |
| `slow_start`           | `<duration: string>`    | Slow-start window after a backend recovers from half-open to closed. During slow-start, the load balancer applies a decaying virtual connection penalty to prevent a thundering herd. The backend appears busier than it is. `0` disables. | `0`      |

### Retry budget nested directives

| Nested directive | Arguments          | Description                                                                                                                                   | Default     |
| ---------------- | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `max_retry_rate` | `<rate: float>`    | Maximum retry rate as a share of total requests, from 0.0 to 1.0. When exceeded, Ferron refuses retries with `503 Service Unavailable`.       | `0.1` (10%) |
| `max_tokens`     | `<count: integer>` | Maximum tokens in the bucket (burst capacity). Controls how many retries can happen in a short burst before the rate limit applies.           | `10`        |
| `refill_rate`    | `<rate: float>`    | Tokens added per second. Higher values let retries recover faster after sustained traffic.                                                    | `2.0`       |

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        algorithm round_robin
        retry_connection
        retry_budget {
            max_retry_rate 0.1
            max_tokens 10
            refill_rate 2.0
        }
    }
}
```

### SSRF risk with interpolated upstream URLs

The upstream URL supports [interpolation syntax](/docs/configuration/fundamentals/conditionals#built-in-variables) for dynamic values. Do not use user-controlled request headers in upstream URLs, such as `request.header.host`, `request.header.x_forwarded_host`, or `request.header.x_forwarded_proto`. An attacker can craft requests that redirect the proxy to internal services.

```ferron
example.com {
    # DANGEROUS: attacker can set X-Forwarded-Host to 169.254.169.254 or any internal host
    proxy "http://{{request.header.x_forwarded_host}}:8080"
}
```

`request.host` comes from the TLS/SNI matcher, not from the client, so it is safe to interpolate:

```ferron
example.com {
    # Safe: Ferron resolves request.host via its TLS/SNI matcher, not user-controlled
    proxy "http://{{request.host}}:8080"
}
```

Ferron preserves the incoming `Host` header by default. To set it explicitly, for example after other header rules, use the raw header variable:

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header Host "{{request.header.host}}"
    }
}
```

`request.header.host` is the raw client header. It is unsafe in upstream URLs (SSRF), but correct when a shared-hosting backend distinguishes domains by `Host`. Use `request.host`, the Ferron matched server name, when you need the server-controlled value.

### Apache and Host-based virtual hosts

Multiple domains can share one Apache backend IP, with Apache selecting the vhost by `Host`:

- Keep the wire `Host` header intact (default). Do not normalize it to `request.host`, or distinct domains behind a wildcard or catch-all collapse into one vhost.
- Isolate circuit breakers per domain. Use a separate `proxy {}` block or a distinct `upstream` URL per vhost. The breaker key is the upstream URL or IP, not `Host`, so one bare `http://192.0.2.10/` shared by several domains trips as one breaker.
- Set `idle_timeout` at or below Apache `KeepAliveTimeout`, often `5s`.
- Point `active_check.uri` at a real per-vhost path. The default probe can hit the Apache default vhost instead of your domain.
- For `https` backends, TLS SNI uses the upstream URL host, not the client `Host`. Use per-vhost upstream hostnames when Apache uses SNI-based vhosts.
- Tolerate graceful reloads. Apache closes idle keepalives, so the first reused connection can return `502` and then succeed on retry.
- To tell a stale-pool `502` from a real outage, check `ferron.proxy.connection_reused=true` with `retry_count>0` and `ferron.proxy.circuit.state` in metrics or logs.

```ferron
example.com {
    proxy {
        upstream http://192.0.2.10:80 {
            idle_timeout "5s"
            connection_timeout "2s"
            active_check {
                uri "/health"
                expect_status "200"
                interval "10s"
            }
        }
        max_retries_per_upstream 1
        retry_interval "0.2s"
        circuit_breaker {
            max_fails 5
            window "30s"
            open_duration "5s"
        }
    }
}
```

## Upstream settings

### `upstream`

`upstream` defines a static backend server.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080 {
            limit 100
            idle_timeout "30s"
            unix /var/run/backend.sock
        }
    }
}
```

| Nested directive     | Arguments           | Description                                                                                                                                                                                                | Default   |
| -------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| `limit`              | `<number>`          | Maximum concurrent connections to this upstream.                                                                                                                                                           | unlimited |
| `idle_timeout`       | `<duration>`        | Keep-alive idle timeout. Ferron evicts connections idle longer than this from the pool.                                                                                                                    | `60s`     |
| `connection_timeout` | `<duration\|false>` | Maximum time to wait for a TCP connection. Set to `false` to disable.                                                                                                                                      | `2s`      |
| `unix`               | `<path>`            | Connect through a Unix domain socket instead of TCP. The URL scheme is still required.                                                                                                                     | TCP       |
| `weight`             | `<number>`          | Weight for weighted load balancing. Higher values receive more requests. Supported by all algorithms and by session affinity.                                                                              | 1         |
| `priority`           | `<number>`          | Priority for tiered failover. Lower values are higher priority. When the highest-priority tier has no available backends, Ferron tries the next tier.                                                      | 0         |
| `cert`               | `<path: string>`    | PEM file with the client certificate chain to present to the upstream for mTLS. Use together with `key`.                                                                                                   | disabled  |
| `key`                | `<path: string>`    | PEM file with the client private key for mTLS. Use together with `cert`.                                                                                                                                   | disabled  |
| `logical_dns`        | `[bool: boolean]`   | Use the system's logical DNS resolution instead of strict A/AAAA resolution. Ferron passes the upstream URL through as-is without per-IP backend splitting.                                                | `false`   |
| `dns_servers`        | `<string>`          | Comma-separated DNS server IPs for strict A/AAAA resolution. Uses Hickory's default resolvers if empty.                                                                                                    | system    |

### `srv`

`srv` defines a dynamic upstream that Ferron resolves from DNS SRV records.

```ferron
example.com {
    proxy {
        srv _http._tcp.example.com {
            dns_servers "8.8.8.8,8.8.4.4"
            limit 100
            idle_timeout "30s"
        }
    }
}
```

| Nested directive     | Arguments           | Description                                                                                                                                                                                                | Default   |
| -------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| `dns_servers`        | `<string>`          | Comma-separated DNS server IPs. Uses the system resolver if empty.                                                                                                                                         | system    |
| `limit`              | `<number>`          | Maximum concurrent connections per resolved backend.                                                                                                                                                      | unlimited |
| `idle_timeout`       | `<duration>`        | Keep-alive idle timeout per resolved backend.                                                                                                                                                              | `60s`     |
| `connection_timeout` | `<duration\|false>` | Maximum time to wait for a TCP connection. Set to `false` to disable.                                                                                                                                      | `2s`      |
| `weight`             | `<number>`          | Multiplier for DNS SRV weights. Effective weight is `dns_weight × config_weight`. `1` uses the DNS weights as-is. Supported by all algorithms and by session affinity.                                      | 1         |
| `priority`           | `<number>`          | Additive offset for DNS SRV priorities. Effective priority is `dns_priority + offset`. Ferron tries lower values first.                                                                                    | 0         |
| `cert`               | `<path: string>`    | PEM file with the client certificate chain to present to resolved backends for mTLS. Use together with `key`.                                                                                             | disabled  |
| `key`                | `<path: string>`    | PEM file with the client private key for mTLS. Use together with `cert`.                                                                                                                                   | disabled  |

## Load balancing

`algorithm` selects the strategy. `weight` on each `upstream` changes how often a backend is selected.

| Algorithm     | Description                                                                                                                                                                                                                     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `random`      | Selects a random backend, weighted by configured weights.                                                                                                                                                                       |
| `round_robin` | Smooth weighted round-robin. Distributes requests in proportion to backend weights.                                                                                                                                             |
| `least_conn`  | Selects the backend with the fewest active tracked connections multiplied by its weight.                                                                                                                                         |
| `two_random`  | Picks two random backends and selects the one with lower weighted load (connections divided by weight).                                                                                                                         |
| `p2c_ewma`    | Power of Two Choices with EWMA latency scoring. Picks two random backends and selects the one with the lower score: EWMA response latency plus active connection penalty, divided by weight. Adapts to backend performance changes. |

### Weighted round robin

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

The first backend receives about 62.5% of requests (5/8), the second 25% (2/8), and the third 12.5% (1/8). Smooth weighted round-robin spreads requests evenly over time. It does not send all requests to one backend before moving to the next.

## Session affinity

Session affinity (sticky sessions) sends the same client to the same backend. Use it for stateful applications, WebSocket-heavy workloads, and cache locality. Configure it with `affinity` inside a `proxy` block. There are four affinity types.

### Cookie affinity

Reads and sets a cookie to track which backend receives a client's requests. With no cookie present, Ferron selects a backend and sets a cookie on the response.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity cookie {
            name ferron_sticky
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
| `secure`         | `[bool]`     | Send the cookie only over HTTPS.                | `false`                  |
| `httponly`       | `[bool]`     | Prevent JavaScript access to the cookie.        | `true`                   |
| `samesite`       | `<mode>`     | SameSite attribute: `strict`, `lax`, or `none`. | `lax`                    |

> [!important]
> With HTTP caching (caching reverse proxy), set the cache `vary_cookies` setting to the same cookie name configured in the `proxy` block. Otherwise cookie-based affinity does not work correctly.
>
> ```ferron
> example.com {
>     cache {
>         vary_cookies ferron_sticky
>     }
> }
> ```

### Header affinity

Routes by a request header value using consistent hashing.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity header {
            name X-Backend-Id
        }
    }
}
```

> [!important]
> With HTTP caching, set the cache `vary` setting to the same header name configured in the `proxy` block. Otherwise header-based affinity does not work correctly.
>
> ```ferron
> example.com {
>     cache {
>         vary X-Backend-Id
>     }
> }
> ```

### IP affinity

Routes by the client IP address using consistent hashing. The same IP routes to the same backend while that backend is healthy.

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
> HTTP caching with IP affinity is not supported. It can cause session affinity correctness issues.

### Hash affinity

Routes by a hashed variable value. Supports any built-in variable or request header.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        upstream http://localhost:8081

        affinity hash {
            # Variable name you'd use in string interpolations, but with quotes instead of double-curly-braces.
            variable "request.header.x-tenant-id"
        }
    }
}
```

### Affinity behavior

- Ferron uses affinity only when the target backend is healthy. If the target is unhealthy, Ferron falls back to the configured load balancing algorithm.
- With `retry_connection` enabled, a failed affinity-targeted backend triggers a retry on another backend.
- Cookie affinity sets the cookie on the first request when no valid cookie is present.
- Ferron uses the affinity key with a consistent hash ring for deterministic routing.

## Priority-based failover

Backends can carry numeric priorities for tiered failover. Lower values are higher priority. When every backend in the highest-priority tier is unavailable, Ferron uses the next tier. A backend is unavailable if it is unhealthy, has an open circuit, or was already tried.

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

Ferron tries the primary tier first, then the secondary tier, and so on. Within a tier, the load balancing algorithm selects the backend. Priority chooses the tier. Weight distributes requests within a tier.

For `srv` upstreams, `priority` is an additive offset to DNS SRV priorities. Effective priority is `dns_priority + offset`. If DNS returns priorities 10 and 20, `priority 5` shifts them to 15 and 25.

```ferron
example.com {
    proxy {
        srv _http._tcp.example.com {
            priority 5
        }
    }
}
```

## Strict DNS resolution

When an `upstream` URL contains a hostname instead of an IP literal or `localhost`, Ferron resolves A and AAAA records with Hickory DNS. Each resolved IP becomes a separate backend. This enables per-IP load balancing, circuit breaking, and health checking.

For example, if `http://myapp.example.com:8080` resolves to three IPs, Ferron creates three backends, one per IP. Ferron keeps the original hostname for TLS SNI and the HTTP `Host` header. Circuit breaker state and active health check state are tracked per address.

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

Set `logical_dns true` to disable strict A/AAAA resolution. Ferron passes the upstream URL through as-is, and the system resolver handles address selection. Use it when the upstream uses DNS for logical routing, for example geographic steering. You get one logical backend instead of one backend per IP.

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
> For ephemeral network addresses, for example when you use Ferron as a Kubernetes ingress controller, define upstreams with DNS hostnames such as `http://my-service.default.svc.cluster.local:8080`. Do not use individual Pod IPs as upstream URLs. DNS hostnames stay stable across restarts and scaling, and Pod IPs change often. Do not rely on per-IP metric dimensions (`metrics_resolved_ip true`). They can cause cardinality explosion in time-series databases, which degrades observability and increases storage cost.

## Forwarding headers

The reverse proxy manages these headers automatically:

| Header                 | Behavior                                                                                                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `X-Forwarded-For`      | With `client_ip_from_header` enabled, appends the extracted client IP to the existing chain. Otherwise, sets it to the direct peer IP.                                             |
| `X-Forwarded-Proto`    | Always set to the incoming request scheme (`http` or `https`).                                                                                                                      |
| `X-Real-IP`            | Always set to the client IP.                                                                                                                                                        |
| `Forwarded` (RFC 7239) | With `client_ip_from_header` enabled, appends an element (`for=...;proto=...;by=...`). Otherwise, sets a single element. Ferron quotes IPv6 addresses per RFC 7239.                  |

## Trace context injection

When a trace context exists, the reverse proxy injects W3C Trace Context headers into the upstream request. This enables distributed tracing across Ferron and your backend services.

| Header        | Behavior                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| `traceparent` | Always injected when a trace context is present. Format: `00-{trace_id}-{span_id}-{flags}`.             |
| `tracestate`  | Injected only if the incoming request or Ferron's trace context carries a non-empty `tracestate` value. |
| `baggage`     | Injected only if the incoming request or Ferron's trace context carries non-empty `baggage` values.     |

Injection happens after Ferron applies all `request_header` transformations:

- `request_header +traceparent "..."` adds a custom value.
- `request_header -traceparent` removes the injected header to suppress propagation.
- `headers_to_remove` cannot remove injected headers because injection happens last.

> [!info]
> By default, Ferron discards incoming `traceparent` headers. Ferron creates trace context when `http { trace { generate true } }` (the default) is active and trace sinks are configured. To trust incoming trace context, enable `trust_request true` inside the `trace` block. See [Tracing configuration](/docs/configuration/observability/tracing) for details.

## Connection pooling

Ferron keeps a keep-alive connection pool for upstream backends:

- Connection reuse: Ferron reuses pooled connections for later requests to the same upstream.
- Idle eviction: Ferron evicts connections idle longer than `idle_timeout`.
- HTTP/2 multiplexing: HTTP/2 connections share one TCP connection for multiple concurrent requests.

> [!tip]
> If you get 502 errors from backends, verify that the `upstream` URLs are reachable and check the circuit breaker settings (`max_fails`).

### Global connection limit

`proxy_concurrent_conns` sets the global maximum number of concurrent TCP connections in the keep-alive pool across all upstream backends. Unix socket connections are always unbounded.

> [!note]
> Host isolation: a named host does not inherit `proxy_concurrent_conns` from the wildcard `*` host. It inherits from global defaults. Default: `16384`.

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

### Connection directives

| Directive          | Arguments         | Description                                                                                                                                                                          | Default |
| ------------------ | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------- |
| `keepalive`        | `[bool: boolean]` | Enable HTTP keep-alive connection pooling.                                                                                                                                           | `true`  |
| `http2`            | `[bool: boolean]` | Enable HTTP/2 for upstream connections.                                                                                                                                              | `false` |
| `http2_only`       | `[bool: boolean]` | Use only HTTP/2 for upstream connections.                                                                                                                                             | `false` |
| `intercept_errors` | `[bool: boolean]` | Replace upstream 4xx and 5xx responses with Ferron error pages. When `false`, Ferron passes the upstream body and headers through unchanged.                                          | `false` |

### TLS

| Directive         | Arguments         | Description                                                                                      | Default                              |
| ----------------- | ----------------- | ------------------------------------------------------------------------------------------------ | ------------------------------------ |
| `no_verification` | `[bool: boolean]` | Disable TLS certificate checks for HTTPS upstreams.                                              | `false`                              |
| `ca`              | `<path: string>`  | CA bundle path for TLS certificate verification of upstream connections.                         | none (system or Mozilla CA roots)    |

> [!warning]
> Use `no_verification true` only in testing or trusted internal networks. For trusted internal networks, prefer `ca` with a CA bundle path.

### Client certificate authentication (mTLS)

To authenticate to an HTTPS upstream, present a client certificate with `cert` and `key` on the upstream:

```ferron
example.com {
    proxy {
        upstream https://backend.internal:443 {
            cert /etc/ferron/client-cert.pem
            key /etc/ferron/client-key.pem
        }
    }
}
```

Ferron requires both `cert` and `key`. Both files must be PEM-encoded. Credentials are scoped per upstream, so different backends can use different client certificates. Active health checks also use the configured mTLS credentials. Ferron caches the credentials in memory until a configuration reload or server shutdown.

### PROXY protocol

`proxy_header` prepends an HAProxy PROXY protocol header to upstream connections. Supported versions are `v1` and `v2`. Default: disabled.

```ferron
example.com {
    proxy {
        upstream http://localhost:8080
        proxy_header v2
    }
}
```

### Request headers

`request_header` changes request headers before Ferron forwards the request. It supports three forms:

| Form                          | Effect                                                  |
| ----------------------------- | ------------------------------------------------------- |
| `request_header +Name "value"` | Add the header. Appends and allows duplicates.          |
| `request_header -Name`         | Remove all instances of the header.                     |
| `request_header Name "value"`  | Replace the header. Removes existing values and sets one. |

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header +X-Custom-Header value
        request_header -X-Sensitive-Header
        request_header Host "new-host.example.com"
    }
}
```

## Health checking

### Circuit breaking

Circuit breaking is passive health checking. Ferron records request-time failures per backend without background probes, then temporarily ejects unstable backends from the load balancer. Transport failures include TCP connect errors and TLS errors.

1. Ferron counts transport failures and, optionally, upstream `5xx` responses per backend in a rolling window.
2. When the backend reaches `max_fails` failures within `window`, Ferron opens the circuit and stops selecting that backend.
3. After `open_duration`, Ferron allows one half-open trial request.
4. When the trial succeeds, Ferron closes the circuit after `consecutive_passes` successful half-open requests.
5. When the half-open trial fails, Ferron reopens the circuit immediately.

By default, only transport failures count. Set `record_5xx true` to count upstream `5xx` responses. Set `latency_threshold` to count slow responses.

Circuit breaking does not retry upstream `5xx` responses. It only changes which backends are eligible for future requests.

> [!note]
> Half-open recovery allows one trial request at a time. If recovery is too aggressive for your workload, increase `open_duration` or `consecutive_passes`. Circuit breaking and active health checks work together, and either can make a backend temporarily ineligible.

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
            record_5xx
            latency_threshold "0.5s"
        }
    }
}
```

### Active health checking

Active health checks probe backend health on a schedule, independent of incoming traffic. They detect backend failures before client requests reach the failed backend.

Ferron probes each resolved address separately and tracks health per address, so one unreachable address does not remove the other addresses behind the same hostname. A probe connects to the address it reports on, but keeps the hostname in the `Host` header and the TLS server name. An upstream that routes by `Host` or requires a matching certificate still answers correctly.

Configure active health checks per upstream with an `active_check` block.

#### `active_check` nested directives

| Nested directive          | Arguments             | Description                                                                                                                                                    | Default   |
| ------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| `uri`                     | `<path: string>`      | Endpoint to probe.                                                                                                                                             | `/health` |
| `method`                  | `<method: string>`    | HTTP method for probes: `GET` or `HEAD`.                                                                                                                       | `GET`     |
| `interval`                | `<duration: string>`  | Interval between probes.                                                                                                                                       | `10s`     |
| `timeout`                 | `<duration: string>`  | Maximum wait for a probe response.                                                                                                                             | `5s`      |
| `expect_status`           | `<status: string>`    | Expected status codes for a successful probe: `2xx`, `3xx`, `2xx,3xx`, specific codes such as `200,204`, or ranges such as `200-299`.                          | `2xx,3xx` |
| `response_time_threshold` | `<duration: string>`  | Optional response time threshold. A slower probe marks the upstream unhealthy.                                                                                 | disabled  |
| `body_match`              | `<substring: string>` | Optional substring to match in the response body. GET only.                                                                                                    | disabled  |
| `consecutive_fails`       | `<count: integer>`    | Consecutive failures before Ferron marks the upstream unhealthy.                                                                                               | 2         |
| `consecutive_passes`      | `<count: integer>`    | Consecutive successes before Ferron marks a recovering upstream healthy.                                                                                        | 2         |
| `no_verification`         | `[bool: boolean]`     | Skip TLS certificate checks for HTTPS probes.                                                                                                                  | `false`   |

```ferron
example.com {
    proxy {
        upstream http://localhost:3000 {
            active_check {
                uri /health
                interval "10s"
                timeout "5s"
                expect_status "200,204"
                consecutive_fails 2
                consecutive_passes 2
            }
        }
        upstream https://localhost:3001 {
            active_check {
                uri /api/status
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
> Make sure the probe endpoint is reachable on all backends and keep probes lightweight. Use HEAD requests when you do not need the response body. If Ferron marks upstreams unhealthy incorrectly, check the logs and verify `expect_status`.

## Retry budgets

The retry budget is a token bucket shared across all requests for one proxy configuration block:

1. The bucket starts full with `max_tokens` tokens.
2. Each successful request deposits one token, up to capacity. Retry capacity therefore tracks steady-state traffic.
3. Each retry consumes one token. This includes same-upstream retries (`max_retries_per_upstream`) and cross-backend failovers (`retry_connection`). If the bucket is empty, Ferron refuses the retry and returns `503 Service Unavailable` with a `Retry-After` header.
4. Ferron refills tokens lazily based on elapsed time and `refill_rate`.

The budget caps retry amplification when several backends fail at once. With `max_retry_rate 0.1` and three backends where two fail, at most about 10% of total traffic becomes retries. The remaining healthy backend is not overwhelmed.

> [!note]
> Ferron scopes the retry budget per proxy config block, so different hosts or locations can have independent budgets. The budget limits the count of retries, not their timing. It adds no delay. For delay-based retry control, use `retry_interval` for same-upstream waits and circuit breakers with `open_duration`.

> [!tip]
> Start with the defaults (`max_retry_rate 0.1`, `max_tokens 10`, `refill_rate 2.0`). Increase `max_retry_rate` only if the budget rejects legitimate transient failures. Increase `max_tokens` for bursty traffic that needs more retry headroom.

## Observability

### Metrics

In the table, `backend` is the backend URL or Unix socket path. `ip` is the resolved IP address, present when `metrics_resolved_ip` is `true`. `dns` is `ferron.proxy.dns_status`. Metrics marked `worker` also carry a thread identifier.

| Metric                                         | Type      | Attributes                                                                                          | Description                                                                                                                                                                    |
| ---------------------------------------------- | --------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ferron.proxy.backends.selected`               | Counter   | `backend`, `ip`, `dns`                                                                              | Backends selected during load balancing                                                                                                                                        |
| `ferron.proxy.backends.selected_per_request`   | Counter   | `backend`, `ip`, `dns`                                                                              | Backends selected per request, including retries                                                                                                                               |
| `ferron.proxy.backends.unhealthy`              | Counter   | `backend`, `ip`, `dns`, `ferron.proxy.health_check_type` (`active` or `circuit_breaker`)            | Backends marked unhealthy                                                                                                                                                      |
| `ferron.proxy.requests`                        | Counter   | `backend`, `ip`, `dns`, `ferron.proxy.connection_reused`, `http.response.status_code`               | Completed upstream proxy requests                                                                                                                                              |
| `ferron.proxy.tls_handshake_failures`          | Counter   | `backend`                                                                                           | TLS handshake failures with upstream backends                                                                                                                                  |
| `ferron.proxy.pool.waits`                      | Counter   | `backend`                                                                                           | Times the pool ran out of connections and a request had to wait                                                                                                                |
| `ferron.proxy.pool.wait_time`                  | Histogram | `backend`                                                                                           | Wait time for a pooled connection. Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s                                                                                          |
| `ferron.proxy.lb.active_connections`           | Gauge     | `backend`                                                                                           | Active tracked connections for the selected backend                                                                                                                            |
| `ferron.proxy.lb.ewma_latency`                 | Gauge     | `backend`                                                                                           | Current EWMA response latency (`p2c_ewma` algorithm)                                                                                                                           |
| `ferron.proxy.lb.warmup_state`                 | Gauge     | `backend`                                                                                           | Whether the selected backend is in EWMA warm-up (1) or settled (0)                                                                                                             |
| `ferron.proxy.lb.score`                        | Gauge     | `backend`, `ip`                                                                                     | Combined load-balancer score for the selected backend. Lower is more preferred. Emitted for `two_random` and `p2c_ewma`.                                                       |
| `ferron.proxy.backends.excluded`               | Counter   | `backend`, `ip`, `dns`, `ferron.proxy.reason` (`circuit_open`, `already_tried`, `overloaded`)       | Backend excluded from selection                                                                                                                                                |
| `ferron.proxy.retry.count`                     | Counter   | `backend`, `http.request.method`, `ferron.proxy.method_idempotent`                                  | Retry attempts for a request, including same-upstream and cross-backend retries                                                                                                |
| `ferron.proxy.same_upstream_retry.count`       | Counter   | `backend`, `http.request.method`, `ferron.proxy.method_idempotent`                                  | Same-upstream retries on a transport failure before another backend. Incremented together with `ferron.proxy.retry.count`. Use it to tell transient blips from failovers.      |
| `ferron.proxy.retry.final`                     | Gauge     | `backend`, `http.request.method`, `ferron.proxy.method_idempotent`                                  | Whether the final retry attempt succeeded (`1`) or failed (`0`)                                                                                                                |
| `ferron.proxy.retry.budget_exhausted`          | Counter   | `backend`                                                                                           | Requests where Ferron refused a retry because the retry budget ran out                                                                                                         |
| `ferron.proxy.retry.budget_tokens_available`   | Gauge     | `backend`                                                                                           | Current available retry budget tokens                                                                                                                                          |
| `ferron.proxy.pool.hit`                        | Counter   | `backend`                                                                                           | Pooled connection reused successfully                                                                                                                                          |
| `ferron.proxy.pool.miss`                       | Counter   | `backend`                                                                                           | No pooled connection available, so Ferron established a new connection                                                                                                         |
| `ferron.proxy.pool.idle`                       | Gauge     | `backend`, `worker`                                                                                 | Current idle connections in the pool                                                                                                                                           |
| `ferron.proxy.pool.outstanding`                | Gauge     | `backend`, `worker`                                                                                 | Current in-use connections in the pool                                                                                                                                         |
| `ferron.proxy.pool.local_limit`                | Gauge     | `backend`                                                                                           | Current local connection limit for the reverse proxy                                                                                                                           |
| `ferron.proxy.pool.global_limit`               | Gauge     | none                                                                                                | Current global connection limit for the reverse proxy                                                                                                                          |
| `ferron.proxy.connect.latency`                 | Histogram | `backend`                                                                                           | Time to establish a TCP or TLS connection to the backend                                                                                                                       |
| `ferron.proxy.ttfb`                            | Histogram | `backend`                                                                                           | Time to first response byte from the backend                                                                                                                                   |
| `ferron.proxy.upstream.duration`               | Histogram | `backend`                                                                                           | Upstream request-response time                                                                                                                                                 |
| `ferron.proxy.health.success`                  | Counter   | `backend`, `ip`, `dns`                                                                              | Health check probe succeeded                                                                                                                                                   |
| `ferron.proxy.health.failure`                  | Counter   | `backend`, `ip`, `dns`                                                                              | Health check probe failed                                                                                                                                                      |
| `ferron.proxy.health.partial_success`          | Gauge     | `backend`, `ip`, `dns`                                                                              | Partial health probe success. When one upstream has several socket addresses and `metrics_resolved_ip` is disabled, the value is the success percentage from 0.0 to 1.0.       |
| `ferron.proxy.health.duration`                 | Histogram | `backend`, `ip`, `dns`                                                                              | Health check probe duration                                                                                                                                                    |
| `ferron.proxy.circuit.state`                   | Gauge     | `backend`, `ip`, `dns`                                                                              | Circuit breaker state: `0` closed, `1` open, `2` half-open. With multiple addresses, Ferron uses the best state (closed, then half-open, then open).                            |
| `ferron.proxy.circuit.partial_state`           | Gauge     | `backend`, `ip`, `dns`, `ferron.proxy.upstream.circuit_state` (`closed`, `open`, `half_open`)       | Number of circuit breakers in each state for each backend address                                                                                                             |
| `ferron.proxy.circuit.open_total`              | Gauge     | `backend`, `ip`, `dns`                                                                              | `1` when the backend circuit is open, `0` otherwise. Sum it across backends to count open circuits.                                                                            |
| `ferron.proxy.circuit.flapping`                | Gauge     | `backend`, `ip`, `dns`                                                                              | `1` when the backend is flapping, `0` when stable. With multiple addresses, Ferron uses the worst state.                                                                       |
| `ferron.proxy.circuit.half_open_timeouts`      | Counter   | `backend`, `ip`, `dns`                                                                              | Half-open trial requests abandoned before the upstream answered, usually because the pipeline `timeout` elapsed                                                                |
| `ferron.proxy.failures`                        | Counter   | `http.response.status_code`, `error.type`                                                           | Proxy failures that returned an error before a backend response                                                                                                                |
| `ferron.proxy.dns.cache_hit`                   | Counter   | none                                                                                                | DNS cache hits for strict DNS and SRV lookups                                                                                                                                  |
| `ferron.proxy.dns.cache_miss`                  | Counter   | none                                                                                                | DNS cache misses or expiries for strict DNS and SRV lookups                                                                                                                    |
| `ferron.proxy.dns.cache_ttl_remaining_seconds` | Gauge     | `aggregation` (`min`, `max`, `avg`)                                                                 | Remaining TTL across all DNS cache entries, aggregated                                                                                                                         |
| `ferron.proxy.dns.cache_entries`               | Gauge     | none                                                                                                | Active DNS cache entries                                                                                                                                                       |
| `ferron.proxy.upstream.response_truncated`     | Counter   | `backend`                                                                                           | Upstream responses that ended before the declared Content-Length                                                                                                               |

> [!note]
> The connection pool and DNS result cache are process wide. Ferron reports `ferron.proxy.pool.*` and `ferron.proxy.dns.cache_*` once for the whole server, and only to an observability backend configured in the global block. A provider configured inside a host block receives that host's per-request proxy metrics, active health check probe metrics, and circuit breaker metrics, but not the process-wide pool and DNS cache metrics.

### Logs

- `ERROR`: Ferron logs a proxy setup error during parsing, or a proxy execution error such as a connection or transport failure.
- `WARN`: active health checks marked an upstream unhealthy.
- `INFO`: an upstream recovered after consecutive successful probes.
- `DEBUG`: Ferron initialized a health check loop for an upstream.

### Structured logs

| Description (summary)                                   | Level | Attributes                                                                                                                                                                                                                                           |
| ------------------------------------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Reverse proxy config error                              | ERROR | `error.message` (string): setup error details                                                                                                                                                                                                        |
| Reverse proxy: `<error type>`                           | ERROR | `error.type` (string): error type classification, `error.message` (string): error details                                                                                                                                                            |
| Upstream marked unhealthy                               | WARN  | `upstream.address` (string): backend server URL, `ferron.proxy.backend_url` (string): backend server URL, `ferron.proxy.backend_unix_path` (string): backend server Unix path, `ferron.proxy.backend_resolved_ip` (string): resolved backend address |
| Upstream recovered                                      | INFO  | `upstream.address` (string): backend server URL, `ferron.proxy.backend_url` (string): backend server URL, `ferron.proxy.backend_unix_path` (string): backend server Unix path, `ferron.proxy.backend_resolved_ip` (string): resolved backend address |
| Initializing health check                               | DEBUG | `ferron.proxy.health.address` (string): backend identifier, `ferron.proxy.health.method` (string): HTTP method, `ferron.proxy.health.uri` (string): health check URI                                                                                 |
| Upstream circuit opened                                 | WARN  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream circuit closed                                 | INFO  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream circuit reopened after half-open trial failure | WARN  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream half-open trial timed out                      | WARN  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream flapping                                       | WARN  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream flapping resolved                              | INFO  | `upstream.address` (string): backend server URL                                                                                                                                                                                                      |
| Upstream circuit transitioned to half-open              | INFO  | `upstream.address` (string): backend server URL, `ferron.proxy.circuit.open_duration_ms`: open duration in milliseconds                                                                                                                              |
| Upstream response truncated                             | WARN  | `ferron.proxy.backend_url` (string): backend server URL, `upstream.bytes_received` (int): bytes received, `upstream.content_length` (int): expected Content-Length                                                                                   |

### Access log fields

The reverse proxy module adds these fields to the HTTP access log line:

| Field                                    | Type   | Description                                                             |
| ---------------------------------------- | ------ | ----------------------------------------------------------------------- |
| `ferron.proxy.backend_url`               | string | Backend URL that served the proxied request.                            |
| `ferron.proxy.backend_resolved_ip`       | string | Resolved IP address of the backend (strict DNS only).                   |
| `ferron.proxy.backend_unix_path`         | string | Unix socket path of the backend, if applicable.                         |
| `ferron.proxy.connection_reused`         | bool   | Whether the module reused a pooled connection.                          |
| `ferron.proxy.retry_count`               | int    | Number of retry attempts (0 if none). Includes same-upstream retries.   |
| `ferron.proxy.same_upstream_retry_count` | int    | Number of same-upstream retries (0 if none).                            |
| `ferron.proxy.circuit_breaker_state`     | string | Circuit breaker state of the backend: `closed`, `open`, or `half_open`. |

### Trace spans

The reverse proxy stage sets these attributes on its `ferron.stage.reverse_proxy` span:

| Attribute                                    | Type   | Description                                                                                                                                   |
| -------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `http.response.status_code`                  | int    | HTTP status code returned by the upstream backend.                                                                                            |
| `error.type`                                 | string | Error type on failure, for example `connection_refused` or `timeout`, for trace UI highlighting.                                               |
| `ferron.proxy.backend_url`                   | string | URL of the upstream backend selected for the request.                                                                                         |
| `ferron.proxy.backend_unix_path`             | string | Unix socket path of the backend, when using Unix sockets.                                                                                     |
| `ferron.proxy.connection_reused`             | bool   | Whether the module reused the connection from the pool.                                                                                       |
| `ferron.proxy.retry_count`                   | int    | Retry attempts during the request, including same-upstream retries.                                                                           |
| `ferron.proxy.same_upstream_retry_count`     | int    | Same-upstream retries during the request.                                                                                                     |
| `ferron.proxy.upstream.circuit_state`        | string | Circuit breaker state of the selected backend: `closed`, `open`, or `half_open`.                                                              |
| `ferron.proxy.upstream.is_flapping`          | bool   | Whether the selected backend is flapping (its circuit breaker states oscillate quickly).                                                      |
| `ferron.proxy.upstream.slow_start`           | bool   | Whether the selected backend is in slow-start after a circuit breaker recovery. Present only when `slow_start` is configured.                 |
| `ferron.proxy.upstream.health_status`        | string | Active health check status of the selected backend: `healthy` or `unhealthy`. Present only when health checks are configured.                 |
| `ferron.proxy.upstream.consecutive_failures` | int    | Consecutive health check failures for the selected backend. Present only when health checks are configured.                                   |
| `ferron.proxy.upstream.active_connections`   | int    | Approximate active connections to the selected backend at routing time.                                                                       |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

### TLS verification

- `proxy { no_verification }`: disable TLS certificate checks for HTTPS upstreams only in testing or tightly controlled internal networks.
- `active_check { no_verification }`: disable TLS checks on health check probes only for strictly internal endpoints.

### Upstream SSRF risk

- Upstream URL with request header interpolation: URLs that contain `{{request.header.*}}` are vulnerable to SSRF. Derive upstream targets from static configuration or trusted server-controlled variables.
