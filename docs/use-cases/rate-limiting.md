---
title: Rate limiting
description: "Protect login and API endpoints in Ferron with per-IP rate limits and path-specific burst controls."
---

Ferron's `limit` module provides per-IP request throttling using a token-bucket algorithm. This is useful for reducing brute-force login attempts and API abuse.

## Protect login endpoints

Apply stricter limits to authentication paths than to the rest of the site:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    // General traffic limit for the site.
    limit rate=50 burst=100

    // Tighter limits for login endpoints.
    location "/login" {
        limit rate=5 burst=10
    }

    location "/api/auth" {
        limit rate=5 burst=10
    }
}
```

## Protect APIs with tiered limits

Use host-level and path-level limits together:

```kdl
// Replace "api.example.com" with your domain name.
api.example.com {
    proxy "http://localhost:3000/"

    // Default API limit.
    limit rate=100 burst=200

    // Heavier endpoints can have stricter caps.
    location "/v1/search" {
        limit rate=20 burst=40
    }

    location "/v1/login" {
        limit rate=5 burst=10
    }
}
```

## Notes and troubleshooting

- Rate limiting is per client IP. If Ferron is behind another proxy/load balancer, configure `trust_x_forwarded_for` or `protocol_proxy` so Ferron can see real client IPs.
- Start with permissive values and tighten after observing production traffic patterns.
- Keep login and token endpoints on stricter limits than read-only API endpoints.
- For directive details, see [Configuration: traffic control](/docs/configuration/traffic-control).
- For proxy/IP directives, see [Configuration: security & TLS](/docs/configuration/security-tls) and [Configuration: core directives](/docs/configuration/core-directives).
