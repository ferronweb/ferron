---
title: "Configuration: rate limiting"
description: "Token bucket-based rate limiting per IP, URI, or request header."
---

This page documents the `rate_limit` directive for configuring token bucket-based rate limiting for HTTP requests. When a client exceeds the configured rate, the server returns a 429 Too Many Requests response with a `Retry-After` header.

## `rate_limit`

```ferron
example.com {
    rate_limit {
        rate 100
        burst 50
        key remote_address
        window 60
        deny_status 429
        bucket_ttl 600
        max_buckets 100000
    }

    location /api {
        rate_limit {
            rate 10
            burst 5
            key remote_address
        }
    }
}
```

Multiple `rate_limit` blocks can be defined to apply different rules simultaneously (e.g. one per IP and one per API key).

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `rate` | `<int>` | Sustained requests per second (required). | — |
| `burst` | `<int>` | Extra tokens above `rate` (bucket capacity = `rate + burst`). | `0` |
| `key` | `<string>` | What to key buckets on. See key types below. | `remote_address` |
| `window` | `<int>` | Time window in seconds for rate calculation (used for `Retry-After`). | `60` |
| `deny_status` | `<int>` | HTTP status code when rate is exceeded. | `429` |
| `bucket_ttl` | `<int>` | Seconds before an unused bucket is evicted. | `600` |
| `max_buckets` | `<int>` | Maximum buckets per rule (prevents memory exhaustion). | `100000` |

### Key types

The `key` directive determines what each bucket is keyed on:

| Value | Description |
| --- | --- |
| `remote_address` | Client IP address (default). |
| `uri` | Request URI path. |
| `request.header.<name>` | Value of the specified request header (e.g. `request.header.X-Api-Key`). |

## Behavior

### Token bucket algorithm

Each key gets its own token bucket:

- **Capacity** = `rate + burst` tokens (bucket starts full)
- **Refill rate** = `rate` tokens per second (refilled lazily on each request)
- **Consumption** = 1 token per request

When the bucket is empty, the request is rejected with the configured `deny_status` and a `Retry-After` header indicating how many seconds to wait.

### Bucket eviction

To prevent unbounded memory growth from one-shot clients, buckets are evicted after `bucket_ttl` seconds of inactivity. The `max_buckets` setting provides a hard upper limit — when reached, new requests are rejected until stale buckets are evicted.

### Per-location limits

`rate_limit` blocks inside `location` blocks apply only to requests matching that path. Both host-level and location-level rules are evaluated — a request must pass all rules to be served.

### Configuration reload

Rate limit buckets are stored in memory and are **not** preserved across configuration reloads. A reload creates fresh buckets with the new configuration.

## Examples

### Basic IP-based rate limiting

```ferron
example.com {
    rate_limit {
        rate 10
        burst 5
        key remote_address
    }
}
```

Allows 15 requests burst, then 10/second sustained per IP.

### API key rate limiting

```ferron
api.example.com {
    rate_limit {
        rate 50
        burst 100
        key request.header.X-Api-Key
    }
}
```

Each unique API key gets 150 requests burst, then 50/second.

### Strict endpoint with custom status

```ferron
example.com {
    location /login {
        rate_limit {
            rate 2
            burst 1
            deny_status 429
        }
    }
}
```

Limits login to 3 requests burst, then 2/second. Returns 429 when exceeded.

## Notes and troubleshooting

- Requests where the key cannot be extracted (e.g. missing header) skip that rule.
- The `Retry-After` header value is rounded up to the nearest whole second.
- Bucket memory usage is approximately 128 bytes per unique key.
- Rate limiting is applied per-server-instance. For distributed rate limiting, use an external service (e.g. Redis) — this is not currently supported.
- For `location` syntax, see [Routing and URL processing](/docs/v3/routing-url-processing).
- For HTTP host directives, see [HTTP host directives](/docs/v3/http-host).
