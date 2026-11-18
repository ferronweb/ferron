---
title: Reverse proxying
description: "Forward requests to backends with WebSocket support, load balancing, health checks, and header control."
---

Set the backend URL with `proxy`:

```ferron
example.com {
    proxy http://localhost:3000
}
```

Ferron supports WebSocket upgrades without extra configuration.

> [!tip]
> If you get `502 Bad Gateway` or `504 Gateway Timeout`, check that the upstream is reachable. Also check `connection_timeout` and the circuit breaker settings.

## Serve static files and proxy an API

Use separate `location` blocks for static files and API traffic:

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
    }

    location / {
        root /var/www/html
    }
}
```

Ferron strips the `/api` prefix before it forwards the request. A request to `/api/login` reaches the backend as `/login`.

> [!tip]
> If only some paths fail, check the `location` match. The longest prefix wins.

## Serve a single-page application and proxy an API

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
    }

    location / {
        root /var/www/html
        rewrite r"^/.*" "/" {
            last
            directory false
            file false
        }
    }
}
```

## Load balancing

Add several `upstream` entries to a `proxy` block:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        algorithm two_random
    }
}
```

| Algorithm     | Behavior                                                                                 |
| ------------- | ---------------------------------------------------------------------------------------- |
| `random`      | Selects a backend at random.                                                             |
| `round_robin` | Distributes requests by backend weight with smooth weighted round-robin.                 |
| `least_conn`  | Selects the backend with the fewest active connections, multiplied by its weight.        |
| `two_random`  | Picks two random backends and selects the less loaded one.                               |
| `p2c_ewma`    | Selects a backend with the P2C+EWMA adaptive algorithm.                                  |

## Split traffic between backends

Use weights with `round_robin` or `least_conn` for a gradual migration or an A/B test:

```ferron
example.com {
    proxy {
        upstream http://legacy.example.com:3000 {
            weight 90
        }

        upstream http://nextjs.example.com:3001 {
            weight 10
        }

        algorithm round_robin
    }
}
```

About 90% of requests go to the legacy backend and 10% to the new stack. Change the weights as the new stack takes more traffic.

> [!note]
> To split traffic between static content variants on one server, use [canary deployment](/docs/configuration/routing/canary) instead. It needs no backend.

### Keep a visitor on one backend

Cookie affinity assigns a backend on the first request and keeps the visitor there:

```ferron
example.com {
    proxy {
        upstream http://variant-a.example.com:3000
        upstream http://variant-b.example.com:3001

        algorithm round_robin
        affinity cookie {
            name ab_test_variant
            ttl "7d"
            path "/"
            httponly
            samesite lax
        }
    }
}
```

To select a variant from a request header instead:

```ferron
example.com {
    proxy {
        upstream http://variant-a.example.com:3000
        upstream http://variant-b.example.com:3001

        affinity header {
            name X-AB-Variant
        }

        algorithm round_robin
    }
}
```

A request with `X-AB-Variant: b` goes to the second backend. Other requests use `round_robin`.

### Migration example

This configuration moves 20% of traffic to a new backend. Cookie affinity keeps each visitor on one backend, and the circuit breaker uses custom thresholds:

```ferron
example.com {
    proxy {
        upstream http://legacy-backend:8080 {
            weight 80
        }

        upstream http://new-backend:3000 {
            weight 20
        }

        algorithm round_robin
        affinity cookie {
            name _ferron_migration
            ttl "24h"
            path "/"
            httponly
        }

        request_header Host "{{request.host}}"

        circuit_breaker {
            max_fails 3
            window "10s"
        }
    }
}
```

### Compare the backends

These metrics compare backend traffic and health:

- `ferron.proxy.backends.selected` shows which backends receive traffic.
- `ferron.proxy.backends.unhealthy` shows when a health check marks a backend as unhealthy.
- `ferron.proxy.requests` shows request counts, status codes, and latency per backend.

## Passive health checks

Ferron checks backend health from real requests. The circuit breaker records transport failures such as TCP connect errors and TLS errors. It can also record upstream `5xx` responses and slow responses. It then removes an unstable backend from the load balancer for a time.

Health checks are on by default:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001
    }
}
```

To change the thresholds:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        circuit_breaker {
            max_fails 5
            window "30s"
            open_duration "10s"
            consecutive_passes 1
            record_5xx
        }
    }
}
```

| Option                | Effect                                                           |
| --------------------- | ---------------------------------------------------------------- |
| `max_fails`           | Failures needed to open the circuit.                             |
| `window`              | Time window for counting failures.                               |
| `open_duration`       | Time before Ferron tries the backend again.                      |
| `consecutive_passes`  | Successful checks needed to close the circuit.                   |
| `record_5xx`          | Counts upstream 5xx responses as failures.                       |
| `latency_threshold`   | Counts responses slower than this value as failures.             |

> [!important]
> The circuit breaker counts transport failures by default. It counts upstream `5xx` only with `record_5xx`. It does not retry upstream `5xx` responses.

> [!info]
> For all circuit breaker options, see [Reverse proxy configuration](/docs/configuration/proxy/reverse-proxy#circuit-breaking).

## Active health checks

Active checks send requests to the backend on a schedule:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000 {
            active_check {
                uri /health
            }
        }
        upstream http://localhost:3001 {
            active_check {
                uri /health
            }
        }
    }
}
```

> [!info]
> For all active health check options, see [Reverse proxy configuration](/docs/configuration/proxy/reverse-proxy).

## Unix socket backends

```ferron
example.com {
    proxy {
        upstream http://backend {
            unix /run/backend/web.sock
        }
    }
}
```

The hostname in `upstream` is arbitrary when you set `unix`.

## gRPC backends

A gRPC backend accepts HTTP/2 requests. Set `http2_only`:

```ferron
grpc.example.com {
    proxy http://localhost:3000 {
        http2_only
    }
}
```

## Dynamic backends with SRV records

```ferron
example.com {
    proxy {
        srv _backend._tcp.example.com
    }
}
```

## HTTPS backend with a private CA

```ferron
example.com {
    proxy {
        upstream https://somehost.example:8443/
        ca /etc/ferron/ca-bundle.pem
    }
}
```

## Multiplex several domains and backends

In this example, one Ferron instance serves several hostnames:

- `https://example.com` serves the main site.
- `https://example.com/agenda` proxies a calendar service.
- `https://foo.example.com` proxies `https://saas.foo.net`.
- `https://bar.example.com` proxies an internal backend.

```ferron
* {
    tls /path/to/certificate.crt /path/to/private.key
}

example.com {
    location /agenda {
        proxy http://calendar.example.net:5000
    }

    location / {
        proxy http://localhost:3000
    }
}

foo.example.com {
    proxy https://saas.foo.net
}

bar.example.com {
    proxy http://backend.example.net:4000
}
```

Ferron strips `/agenda` before it forwards the request. The calendar service must accept paths without that prefix, or you must add a rewrite rule.

## Trace context

When a trace context exists, Ferron adds W3C Trace Context headers to the upstream request:

- `traceparent`
- `tracestate`
- `baggage`

The backend can read these headers and create child spans.

> [!info]
> See [Tracing configuration](/docs/configuration/observability/tracing) and [Reverse proxy configuration](/docs/configuration/proxy/reverse-proxy#trace-context-injection).

## SSRF risk with interpolated upstream URLs

An upstream URL supports [interpolation](/docs/configuration/fundamentals/conditionals#built-in-variables). Do not put user-controlled request headers in an upstream URL. An attacker can send a crafted header and make Ferron proxy a request to an internal service.

Unsafe: the client controls `X-Forwarded-Host`:

```ferron
example.com {
    proxy "http://{{request.header.x_forwarded_host}}:8080"
}
```

Safe: a static upstream URL:

```ferron
example.com {
    proxy http://localhost:8080
}
```

Safe: `request.host` from the Ferron host matcher:

```ferron
example.com {
    proxy "http://{{request.host}}:8080"
}
```

To send the original host to the backend, set the `Host` header instead:

```ferron
example.com {
    proxy http://localhost:8080 {
        request_header Host "{{request.host}}"
    }
}
```
