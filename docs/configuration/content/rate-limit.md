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

You can define multiple `rate_limit` blocks to apply different rules simultaneously (for example, one per IP and one per API key).

| Nested directive | Arguments  | Description                                                                           | Default          |
| ---------------- | ---------- | ------------------------------------------------------------------------------------- | ---------------- |
| `rate`           | `<int>`    | Sustained requests per second (required).                                             | none             |
| `burst`          | `<int>`    | Extra tokens above `rate` (bucket capacity = `rate + burst`).                         | `0`              |
| `key`            | `<string>` | What to key buckets on. See key types below.                                          | `remote_address` |
| `deny_status`    | `<int>`    | HTTP status code when a client exceeds the rate limit.                                | `429`            |
| `bucket_ttl`     | `<int>`    | Seconds before Ferron removes an unused bucket.                                       | `600`            |
| `max_buckets`    | `<int>`    | Maximum buckets per rule (prevents memory exhaustion).                                | `100000`         |
| `zone`           | `<string>` | Named zone for sharing rate limit buckets across hosts.                               | none             |
| `throttle`       | `<bool>`   | If `true`, Ferron delays requests instead of rejecting them when the bucket is empty. | `false`          |

### Key types

The `key` directive determines which value each bucket uses:

| Value                   | Description                                                                      |
| ----------------------- | -------------------------------------------------------------------------------- |
| `remote_address`        | Client IP address (default).                                                     |
| `uri`                   | Request URI path.                                                                |
| `request.header.<name>` | Value of the specified request header (for example, `request.header.X-Api-Key`). |

## Behavior

> [!important]
> Ferron applies rate limiting per server instance. For distributed rate limiting, use an external service (for example, Redis). The rate limiting module does not support this.

### Token bucket algorithm

Each key gets its own token bucket:

- **Capacity** = `rate + burst` tokens (bucket starts full)
- **Refill rate** = `rate` tokens per second (refilled lazily on each request)
- **Consumption** = 1 token per request

When the bucket is empty, the server rejects the request with the configured `deny_status`. The response includes a `Retry-After` header that shows how many seconds to wait.

### Bucket eviction

To prevent unbounded memory growth from one-shot clients, the module evicts buckets after `bucket_ttl` seconds of inactivity. The `max_buckets` setting enforces a hard upper limit. When the registry reaches this limit, the module rejects new requests until it removes stale buckets.

### Per-location limits

`rate_limit` blocks inside `location` blocks apply only to requests matching that path. Ferron evaluates both host-level and location-level rules. A request must pass all rules before Ferron serves it.

### Rate limit zones

By default, each host gets its own isolated set of rate limit buckets. Rate limit zones allow multiple hostnames to share the same buckets, or to explicitly opt out of a global zone.

**Zone resolution order:**

1. If the host-level `rate_limit` block contains `zone "name"`, the host joins the named zone.
2. If the host has its own `rate_limit` block (without `zone`) and a global zone exists, the host gets a per-host zone. This opts out of the global zone.
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

Both `example.com` and `api.example.com` share the same global zone. Buckets use the client IP as the key, so a client hitting both hosts shares the same token pool.

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

Both `api.example.com` and `api-v2.example.com` share the named zone `"api"` and the same rate limit buckets.

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

Ferron stores rate limit buckets in memory. They do not survive a configuration reload. A reload creates fresh buckets with the new configuration.

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
> When a request has no valid key, for example because a header is absent, the request skips that rule.

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

| Metric                       | Type    | Attributes                                                                            | Description                                                        |
| ---------------------------- | ------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ferron.ratelimit.allowed`   | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`) | Requests that passed rate limiting                                 |
| `ferron.ratelimit.rejected`  | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`) | Requests rejected due to exhausted buckets or registry at capacity |
| `ferron.ratelimit.throttled` | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`) | Requests delayed due to throttling                                 |

The `ferron.ratelimit.zone` attribute identifies which rate limit zone the request belongs to. It has the value `"global"` for the shared global zone. It uses the zone name for named zones and the hostname for per-host zones.

### Logs

- **`DEBUG`**: logged when a rate limit bucket has no tokens left for a key.
- **`WARN`**: logged when the registry reaches `max_buckets` capacity and applies backpressure.

### Structured logs

| Description (summary)       | Level | Attributes                                                                                                                                                                                     |
| --------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rate limit bucket exhausted | DEBUG | `ferron.ratelimit.zone` (string). Zone identifier. `ferron.ratelimit.key` (string). The rate limit key value. `ferron.ratelimit.key_type` (string). Key type (`"ip"`, `"uri"`, or `"header"`). |

### Access log fields

The rate limiting module contributes the following fields to the HTTP access log line:

| Field                               | Type   | Description                                          |
| ----------------------------------- | ------ | ---------------------------------------------------- |
| `ferron.ratelimit.result`           | string | Rate limit decision: `allowed` or `rejected`.        |
| `ferron.ratelimit.zone`             | string | Rate limit zone identifier.                          |
| `ferron.ratelimit.retry_after_secs` | int    | Seconds until next request allowed (rejection only). |

### Trace spans

The rate limit stage sets the following attributes on its `ferron.stage.rate_limit` span:

| Attribute                           | Type   | Description                                                      |
| ----------------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.ratelimit.result`           | string | Rate limit decision: `allowed`, `throttled` or `rejected`.       |
| `ferron.ratelimit.zone`             | string | The rate limit zone name.                                        |
| `ferron.ratelimit.key_type`         | string | Key extractor type: `ip`, `uri`, or `header`.                    |
| `ferron.ratelimit.limit`            | int    | The configured rate limit (requests per second).                 |
| `ferron.ratelimit.retry_after_secs` | int    | Seconds until the bucket is available again (on rejection only). |
