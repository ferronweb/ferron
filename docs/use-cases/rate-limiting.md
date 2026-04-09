---
title: Rate limiting
description: "Protect login and API endpoints in Ferron with token bucket-based rate limiting per IP, URI, or request header."
---

Ferron's `rate_limit` directive provides token bucket-based rate limiting. This is useful for reducing brute-force login attempts and API abuse. When a client exceeds the configured rate, the server returns a `429 Too Many Requests` response with a `Retry-After` header.

## Protect login endpoints

Apply stricter limits to authentication paths than to the rest of the site:

```ferron
example.com {
    root /var/www/html

    # General traffic limit for the site.
    rate_limit {
        rate 50
        burst 100
        key remote_address
    }

    # Tighter limits for login endpoints.
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

## Protect APIs with tiered limits

Use host-level and location-level limits together:

```ferron
api.example.com {
    location / {
        proxy http://localhost:3000

        # Default API limit.
        rate_limit {
            rate 100
            burst 200
            key remote_address
        }
    }

    # Heavier endpoints can have stricter caps.
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

## API key rate limiting

You can also key rate limits off request headers (for example, API keys):

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

Each unique API key gets its own token bucket, independent of the client IP.

## Notes and troubleshooting

- Rate limiting uses a token bucket algorithm: capacity = `rate + burst` tokens, refilled at `rate` tokens per second.
- If Ferron is behind another proxy/load balancer, ensure the client IP is correctly resolved. See [HTTP host directives](/docs/v3/configuration/http-host) for `client_ip_from_header` configuration.
- Start with permissive values and tighten after observing production traffic patterns.
- Keep login and token endpoints on stricter limits than read-only API endpoints.
- Rate limit buckets are stored in memory and are not preserved across configuration reloads.
- For directive details, see [Configuration: rate limiting](/docs/v3/configuration/http-ratelimit).
