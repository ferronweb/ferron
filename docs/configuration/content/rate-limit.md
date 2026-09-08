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

Ferron applies rate limiting with the in-memory backend per server instance by default. For distributed rate limiting across instances, configure a `rate_limit_backend` block with `type redis` (Redis or Valkey, same RESP protocol).

### Token bucket algorithm

Each key gets its own token bucket:

- **Capacity** = `rate + burst` tokens (bucket starts full)
- **Refill rate** = `rate` tokens per second (refilled lazily on each request)
- **Consumption** = 1 token per request

When the bucket is empty, the server rejects the request with the configured `deny_status`. The response includes a `Retry-After` header that shows how many seconds to wait.

### Bucket eviction

To prevent unbounded memory growth from one-shot clients, the module evicts buckets after `bucket_ttl` seconds of inactivity. The `max_buckets` setting enforces a hard upper limit. When the registry reaches this limit, the module rejects new requests until it removes stale buckets. `max_buckets` applies to the in-memory backend only; Redis keys expire via `bucket_ttl`.

### Distributed backend with Redis or Valkey

Rate limit state is selected by a `rate_limit_backend` block, a sibling of `rate_limit` (not nested inside it). A host or location without its own `rate_limit_backend` block inherits the global one; absent entirely means in-memory.

```ferron
{
    rate_limit_backend {
        type redis
        url "redis://127.0.0.1:6379/0"
        key_prefix "ferron:rl:"
        timeout 200
        fail_open true
    }
}

example.com {
    rate_limit {
        rate 100
        burst 50
        key remote_address
    }
}
```

| Nested directive | Arguments    | Description                                                                                                           | Default      |
| ---------------- | ------------ | --------------------------------------------------------------------------------------------------------------------- | ------------ |
| `type`           | `<string>`   | Backend type: `memory` or `redis` (`valkey` is accepted as an alias for `redis`).                                     | `memory`     |
| `url`            | `<string>`   | Redis/Valkey URL, for example `redis://127.0.0.1:6379/0`. Required when `type redis`.                                 | none         |
| `key_prefix`     | `<string>`   | Prefix prepended to every Redis key. Zone, rule fingerprint and user key are appended automatically.                  | `ferron:rl:` |
| `timeout`        | `<duration>` | Per-request Redis timeout in milliseconds as a number (for example `200`), or a duration string (for example `"2s"`). | `200`        |
| `fail_open`      | `<bool>`     | On Redis errors, allow (`true`, default) or deny (`false`) the request.                                               | `true`       |

Redis uses the same token-bucket semantics as memory via an atomic Lua script (capacity `rate + burst`, refill `rate`/sec, key TTL `bucket_ttl`). All Redis I/O runs on the secondary Tokio runtime, never on primary `zincio` threads.

> [!note]
> `fail_open true` (default) favors availability: traffic is allowed during a Redis outage. For high-security endpoints (for example login), set `fail_open false` to deny instead. The module emits `ferron.ratelimit.backend_errors` and a `WARN` log on backend failures either way.

> [!note]
> With `throttle true` over Redis, Ferron sleeps once for `Retry-After` (capped at 30s) and retries once, instead of looping. Memory throttling may wait longer via the bucket.

Override per location by defining another `rate_limit_backend` block:

```ferron
example.com {
    rate_limit_backend {
        type redis
        url "redis://127.0.0.1:6379/0"
    }

    rate_limit {
        rate 100
        burst 50
    }

    location /internal {
        rate_limit_backend {
            type memory
        }

        rate_limit {
            rate 1000
            burst 100
        }
    }
}
```

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

Ferron stores in-memory rate limit buckets in memory. They do not survive a configuration reload. A reload creates fresh buckets with the new configuration. Redis-backed buckets survive reloads (they live in Redis and expire via `bucket_ttl`).

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

| Metric                            | Type    | Attributes                                                                                                        | Description                                                        |
| --------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ferron.ratelimit.allowed`        | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`), `ferron.ratelimit.backend` | Requests that passed rate limiting                                 |
| `ferron.ratelimit.rejected`       | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`), `ferron.ratelimit.backend` | Requests rejected due to exhausted buckets or registry at capacity |
| `ferron.ratelimit.throttled`      | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`"ip"`, `"header"`, or `"uri"`), `ferron.ratelimit.backend` | Requests delayed due to throttling                                 |
| `ferron.ratelimit.backend_errors` | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type`, `ferron.ratelimit.backend`                                  | Backend errors (for example Redis unavailable or timeout)          |

The `ferron.ratelimit.zone` attribute identifies which rate limit zone the request belongs to. It has the value `"global"` for the shared global zone. It uses the zone name for named zones and the hostname for per-host zones. The `ferron.ratelimit.backend` attribute is `"memory"` or `"redis"`.

### Logs

- **`DEBUG`**: logged when a rate limit bucket has no tokens left for a key.
- **`WARN`**: logged when the registry reaches `max_buckets` capacity and applies backpressure.
- **`WARN`**: logged when the Redis backend errors (`Rate limit backend error`; fail-open allows, fail-closed denies).

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
| `ferron.ratelimit.backend`          | string | Backend type: `memory` or `redis`.                               |
| `ferron.ratelimit.limit`            | int    | The configured rate limit (requests per second).                 |
| `ferron.ratelimit.retry_after_secs` | int    | Seconds until the bucket is available again (on rejection only). |
