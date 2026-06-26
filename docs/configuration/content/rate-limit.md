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
| `deny_status` | `<int>` | HTTP status code when rate is exceeded. | `429` |
| `bucket_ttl` | `<int>` | Seconds before an unused bucket is evicted. | `600` |
| `max_buckets` | `<int>` | Maximum buckets per rule (prevents memory exhaustion). | `100000` |
| `zone` | `<string>` | Named zone for sharing rate limit buckets across hosts. | — |

### Key types

The `key` directive determines what each bucket is keyed on:

| Value | Description |
| --- | --- |
| `remote_address` | Client IP address (default). |
| `uri` | Request URI path. |
| `request.header.<name>` | Value of the specified request header (e.g. `request.header.X-Api-Key`). |

## Behavior

> [!important]
> Rate limiting is applied per-server-instance. For distributed rate limiting, use an external service (e.g. Redis) — this is not supported by the rate limiting module.

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

### Rate limit zones

By default, each host gets its own isolated set of rate limit buckets. Rate limit zones allow multiple hostnames to share the same buckets, or to explicitly opt out of a global zone.

**Zone resolution order:**

1. If the host-level `rate_limit` block contains `zone "name"`, the host joins the named zone.
2. If the host has its own `rate_limit` block (without `zone`) and a global zone exists, the host gets its own per-host zone (opting out of the global zone).
3. If a global `rate_limit` block exists without `zone` blocks, all hosts without explicit zones share the global zone.
4. Otherwise, each host gets its own per-host zone.

**Global zone:**

```ferron
{
    rate_limit {
        rate 10
        burst 5
        key remote_address
    }
}

example.com {
    rate_limit {
        rate 10
        burst 5
        key remote_address
    }
}

api.example.com {
    rate_limit {
        rate 50
        burst 10
        key remote_address
    }
}
```

Both `example.com` and `api.example.com` share the same global zone. Their rate limit buckets are keyed by IP, so a client hitting both hosts shares the same token pool.

**Named zones:**

```ferron
{
    rate_limit {
        zone "api"
    }
}

api.example.com {
    rate_limit {
        zone "api"
        rate 50
        burst 10
        key remote_address
    }
}

api-v2.example.com {
    rate_limit {
        zone "api"
        rate 50
        burst 10
        key remote_address
    }
}
```

Both `api.example.com` and `api-v2.example.com` share the named zone `"api"`. Their rate limit buckets are shared.

**Opting out of the global zone:**

```ferron
{
    rate_limit {
        rate 10
        burst 5
        key remote_address
    }
}

example.com {
    # Inherits global zone
}

internal.example.com {
    rate_limit {
        rate 100
        burst 20
        key remote_address
    }
    # Has its own rate_limit block → per-host zone (opts out of global)
}
```

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

> [!note]
> Requests where the key cannot be extracted (e.g. missing header) skip that rule.

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

## Observability

### Metrics

The rate limiting module emits the following metrics:

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.ratelimit.allowed` | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`) | Requests that passed rate limiting |
| `ferron.ratelimit.rejected` | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`) | Requests rejected due to exhausted buckets or registry at capacity |

The `ferron.ratelimit.zone` attribute identifies which rate limit zone the request belongs to. It is set to `"global"` for the shared global zone, the zone name for named zones, or the hostname for per-host zones.

### Logs

- **`DEBUG`**: logged when a rate limit bucket is exhausted for a key.
- **`WARN`**: logged when the registry reaches `max_buckets` capacity and backpressure is applied.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Rate limit bucket exhausted | DEBUG | `ferron.ratelimit.zone` (string) — zone identifier, `ferron.ratelimit.key` (string) — the rate limit key value, `ferron.ratelimit.key_type` (string) — key type (`"ip"`, `"uri"`, or `"header"`) |
