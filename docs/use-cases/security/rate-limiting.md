---
title: Rate limiting
description: "Limit login and API traffic per IP, URI, or request header with token buckets."
---

`rate_limit` limits requests with token buckets. A client that exceeds the limit gets `429 Too Many Requests` with a `Retry-After` header.

Each key has a bucket with `rate + burst` tokens. Ferron refills the bucket at `rate` tokens per second and consumes one token per request.

> [!important]
> If Ferron is behind another proxy or load balancer, resolve the client IP correctly. See [HTTP host directives](/docs/configuration/server/host) for `client_ip_from_header`.

## Limit login paths

Set a general host limit, then tighten the limit for login paths:

```ferron
example.com {
    root /var/www/html

    rate_limit {
        rate 50
        burst 100
        key remote_address
    }

    location /login {
        rate_limit {
            rate 5
            burst 10
            key remote_address
        }
    }

    location /api/auth {
        rate_limit {
            rate 5
            burst 10
            key remote_address
        }
    }
}
```

## Tiered API limits

Apply a default API limit and stricter caps on expensive endpoints:

```ferron
api.example.com {
    location / {
        proxy http://localhost:3000

        rate_limit {
            rate 100
            burst 200
            key remote_address
        }
    }

    location /v1/search {
        rate_limit {
            rate 20
            burst 40
            key remote_address
        }
    }

    location /v1/login {
        rate_limit {
            rate 5
            burst 10
            key remote_address
        }
    }
}
```

Start with permissive values. Tighten them after you observe production traffic.

## Limit by API key

Key the bucket on a request header to limit each API key independently:

```ferron
api.example.com {
    proxy http://localhost:3000

    rate_limit {
        rate 50
        burst 100
        key request.header.X-Api-Key
    }
}
```

A request without the header skips that rule.

## Throttle instead of reject

`throttle` delays a request when the bucket is empty. Ferron does not reject the request:

```ferron
api.example.com {
    proxy http://localhost:3000
    rate_limit {
        rate 50
        burst 100
        key request.header.X-Api-Key
        throttle
    }
}
```

## Return JSON errors

By default, Ferron returns an HTML body for `429 Too Many Requests`. Add `json_errors` to return a JSON object instead:

```ferron
api.example.com {
    proxy http://localhost:3000
    rate_limit {
        rate 50
        burst 100
        key remote_address
    }
    json_errors
}
```

## Distributed limits

In-memory buckets apply to one Ferron instance and do not survive a configuration reload. To share limits across instances, configure a `rate_limit_backend` block with `type redis`:

```ferron
{
    rate_limit_backend {
        type redis
        url redis://127.0.0.1:6379/0
    }
}
```

Redis-backed buckets survive reloads. By default, `fail_open true` allows traffic when Redis is unavailable. Set `fail_open false` for high-security endpoints such as login.

> [!info]
> For all options, see [Configuration: rate limiting](/docs/configuration/content/rate-limit).
