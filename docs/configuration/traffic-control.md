---
title: "Configuration: traffic control"
description: "Request rate-limiting directives and examples for global and per-location traffic control."
---

This page covers KDL traffic-control directives used to rate-limit requests globally or within specific locations.

## Directives

### Rate limiting

- `limit [enable_limit: bool] [rate=<rate: integer|float>] [burst=<rate: integer|float>]` (_limit_ module)
  - This directive specifies whether the rate limiting is enabled. The `rate` prop specifies the maximum average amount of requests per second, defaults to 25 requests per second. The `burst` prop specifies the maximum peak amount of requests per second, defaults to 4 times the maximum average amount of requests per second. Default: `limit #false`

**Configuration example:**

```kdl
example.com {
    // Global rate limiting
    limit rate=100 burst=200

    // Different rate limits for different paths
    location "/api" {
        limit rate=10 burst=20
    }

    location "/login" {
        limit rate=5 burst=10
    }
}
```
