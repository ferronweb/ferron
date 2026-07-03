---
title: Reverse proxying
description: "Configure Ferron as a reverse proxy with WebSocket support, optional static/SPA hosting, multiple locations, load balancing, and header manipulation."
---

Configuring Ferron as a reverse proxy is straightforward — you just need to specify the backend server URL using the `proxy` directive. To configure Ferron as a reverse proxy, you can use the configuration below:

```ferron
example.com {
    proxy http://localhost:3000
}
```

The WebSocket protocol is supported out of the box in this configuration — no additional configuration is required.

> [!tip]
> If you get `502 Bad Gateway` or `504 Gateway Timeout`, verify the `upstream` URL is reachable and check `circuit_breaker` and `connection_timeout` upstream settings.

## Reverse proxy with static file serving support

Ferron supports serving static files and reverse proxying at once. You can use separate `location` blocks for this:

```ferron
example.com {
    # The "/api" location is used for reverse proxying
    # For example, "/api/login" is proxied to "http://localhost:3000/api/login"
    location /api {
        proxy http://localhost:3000
    }

    # The "/" location is used for serving static files
    location / {
        root /var/www/html
    }
}
```

> [!tip]
> If only some paths fail, review `location` matching order — more specific locations win over less specific ones.

## Reverse proxy with a single-page application

Ferron supports serving a single-page application and reverse proxying at once. You can use this configuration:

```ferron
example.com {
    # The "/api" location is used for reverse proxying
    location /api {
        proxy http://localhost:3000
    }

    # The "/" location is used for serving static files with SPA fallback
    location / {
        root /var/www/html
        rewrite "^/.*" "/" {
            last
            directory false
            file false
        }
    }
}
```

## Load balancing

Ferron supports load balancing by specifying multiple upstream backends inside a `proxy` block. To configure Ferron as a load balancer, you can use the configuration below:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        algorithm two_random
    }
}
```

### Load balancing algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request. |
| `round_robin` | Distributes requests proportionally to backend weights using smooth weighted round-robin. |
| `least_conn` | Selects the backend with the fewest active tracked connections multiplied by its weight. |
| `two_random` | Picks two random backends and selects the less loaded one. |
| `p2c_ewma` | Selects the backend based on a P2C+EWMA adaptive load balancing algorithm. |

## A/B testing with backends

Ferron supports A/B testing (traffic splitting) between multiple backends using weighted load balancing and session affinity. This is especially useful when migrating between different tech stacks, where application-level routing logic would be difficult or impossible to implement.

### Weighted traffic splitting

You can split traffic between backends using the `weight` directive with the `round_robin` or `least_conn` algorithm. This is useful for gradual rollouts or A/B tests where you want precise control over traffic distribution.

```ferron
example.com {
    proxy {
        # Legacy backend — receives 90% of traffic
        upstream http://legacy.example.com:3000 {
            weight 90
        }

        # New tech stack — receives 10% of traffic
        upstream http://nextjs.example.com:3001 {
            weight 10
        }

        algorithm round_robin
    }
}
```

In this example, approximately 90% of requests go to the legacy backend and 10% to the new tech stack. Adjust the weights to increase the new backend's traffic share as you gain confidence.

### Sticky session A/B testing

For A/B tests where you want each visitor to consistently see the same variant, use cookie affinity. This ensures users always reach the same backend throughout their session.

```ferron
example.com {
    proxy {
        upstream http://variant-a.example.com:3000
        upstream http://variant-b.example.com:3001

        algorithm round_robin
        affinity cookie {
            name "ab_test_variant"
            ttl "7d"
            path "/"
            httponly
            samesite lax
        }
    }
}
```

With cookie affinity, the first request assigns a backend and sets a `ab_test_variant` cookie. Subsequent requests from the same browser are routed to the same backend for the duration of the cookie TTL.

### Header-based variant selection

For controlled testing or developer previews, you can route based on a request header. This is useful for internal testing or when you want to force a specific variant.

```ferron
example.com {
    proxy {
        upstream http://variant-a.example.com:3000
        upstream http://variant-b.example.com:3001

        affinity header {
            name "X-AB-Variant"
        }

        # Fallback to round-robin when header is absent
        algorithm round_robin
    }
}
```

With this configuration, requests containing `X-AB-Variant: b` are routed to the second backend. All other requests fall back to the configured `round_robin` algorithm.

### Migrating tech stacks at the proxy layer

When rewriting a legacy application in a new technology, proxy-level A/B testing lets you gradually shift traffic without modifying either codebase. The proxy intercepts incoming requests and routes them to the appropriate backend.

```ferron
example.com {
    proxy {
        # Legacy PHP/Ruby on Rails backend
        upstream http://legacy-backend:8080 {
            weight 80
        }

        # New Go/Next.js backend
        upstream http://new-backend:3000 {
            weight 20
        }

        algorithm round_robin
        affinity cookie {
            name "_ferron_migration"
            ttl "24h"
            path "/"
            httponly
        }

        # Forward the original host to the backend
        request_header Host "{{request.host}}"

        # Customize passive health check thresholds
        circuit_breaker {
            max_fails 3
            window "10s"
        }
    }
}
```

This configuration gradually shifts 20% of traffic to the new stack while keeping 80% on the legacy backend. Cookie affinity ensures each visitor stays on the same backend during the migration window. Circuit breakers are enabled by default, so passive health checking is active without any configuration — the `circuit_breaker` block above only customizes the thresholds.

### Observing A/B test results

Ferron's proxy metrics make it easy to compare backend performance in Prometheus and Grafana:

- `ferron.proxy.backends.selected` — track which backends receive traffic and at what rate.
- `ferron.proxy.backends.unhealthy` — monitor when backends are marked unhealthy by health checks.
- `ferron.proxy.requests` — compare request counts, status codes, and latency across backends.

You can create Grafana panels to visualize the ratio of requests between backends, compare p99 latency per backend, and alert on increased error rates in the new backend.

## Passive health checking

Ferron provides passive health checking — tracking request-time failures per backend without background probes — through its circuit breaker. The circuit breaker records transport failures (TCP connect errors, TLS errors) and optionally upstream `5xx` responses, then temporarily ejects unstable backends from the load balancer.

Circuit breakers are enabled by default, so passive health checking is active without any configuration:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001
    }
}
```

To customize the passive health checking behavior, add the `circuit_breaker` block:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        circuit_breaker {
            max_fails 3
            window "5s"
        }
    }
}
```

To also count upstream `5xx` responses toward the circuit:

```ferron
circuit_breaker {
    max_fails 3
    window "5s"
    record_5xx true
}
```

## Circuit breaking

Circuit breakers are enabled by default and protect against upstream failures by temporarily ejecting unstable backends from the load balancer. This section covers customizing the default behavior.

To customize the default circuit breaker settings:

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

> [!important]
> Circuit breaking counts transport failures by default. Upstream `5xx` responses count only when `record_5xx true` is set. Circuit breaking does not automatically retry upstream `5xx` responses.

> [!info]
> For circuit breaker configuration details, see [Reverse proxying configuration reference](/docs/v3/configuration/proxy/reverse-proxy#circuit-breaking).

## Active health checks

Ferron also supports active health checks. To enable active health checking:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000 {
            active_check {
                uri "/health"
            }
        }
        upstream http://localhost:3001 {
            active_check {
                uri "/health"
            }
        }
    }
}
```

> [!info]
> For active health check configuration, see [Reverse proxying configuration reference](/docs/v3/configuration/proxy/reverse-proxy).

## Reverse proxy to backends listening on Unix sockets

Ferron supports reverse proxying to backends listening on Unix sockets:

```ferron
example.com {
    proxy {
        upstream http://backend {
            unix /run/backend/web.sock
        }
    }
}
```

## Reverse proxy to gRPC backends

Ferron supports reverse proxying to gRPC backends that accept HTTP/2 requests:

```ferron
grpc.example.com {
    proxy http://localhost:3000 {
        http2_only
    }
}
```

## Reverse proxy to dynamic backends (via SRV records)

Ferron supports reverse proxying to dynamic backends via DNS SRV records (requires `srv-lookup` feature):

```ferron
example.com {
    proxy {
        srv _backend._tcp.example.com
    }
}
```

## Example: Ferron multiplexing to several backend servers

In this example, the `example.com` and `bar.example.com` domains point to a server running Ferron.

Below are assumptions made for this example:

- `https://example.com` is "main site", while `https://example.com/agenda` is hosting a calendar service.
- `https://foo.example.com` is passed to `https://saas.foo.net`
- `https://bar.example.com` is the front for an internal backend.

You can configure Ferron like this:

```ferron
* {
    tls /path/to/certificate.crt /path/to/private.key
}

example.com {
    location /agenda {
        # It would proxy /agenda/example to http://calender.example.net:5000/agenda/example
        proxy http://calender.example.net:5000
    }

    location / {
        # Catch-all path
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

For `http://calender.example.net:5000/agenda/example`, you will probably have to either configure the calendar service to strip `agenda/` or configure URL rewriting in Ferron.

## Trace context propagation

The reverse proxy automatically injects W3C Trace Context headers (`traceparent`, `tracestate`, and `baggage`) into outgoing upstream requests when a trace context exists. This enables end-to-end distributed tracing — your backend services can read these headers to create child spans that connect to the trace initiated by Ferron.

> [!info]
> For details on trace context configuration, sampling, and header behavior, see [Tracing configuration](/docs/v3/configuration/observability/tracing) and [Reverse proxy configuration](/docs/v3/configuration/proxy/reverse-proxy#trace-context-injection).

## Security considerations

### SSRF risk with interpolated upstream URLs

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
