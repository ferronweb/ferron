---
title: "Configuration: rate limiting"
description: "Token bucket rate limits per IP address, URI, or request header, with an optional Redis backend."
---

`rate_limit` limits requests with token buckets. A client that exceeds the limit gets `429 Too Many Requests` with a `Retry-After` header.

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
            log_rejections
        }
    }
}
```

You can define several `rate_limit` blocks to apply several rules at once, for example one per IP and one per API key.

| Nested directive | Arguments  | Description                                                       | Default          |
| ---------------- | ---------- | ----------------------------------------------------------------- | ---------------- |
| `rate`           | `<int>`    | Sustained requests per second. Required.                          | none             |
| `burst`          | `<int>`    | Extra tokens above `rate`. Capacity is `rate + burst`.            | `0`              |
| `key`            | `<string>` | Bucket key. See key types below.                                  | `remote_address` |
| `deny_status`    | `<int>`    | Status code when the limit is exceeded.                           | `429`            |
| `bucket_ttl`     | `<int>`    | Seconds before Ferron removes an unused bucket.                   | `600`            |
| `max_buckets`    | `<int>`    | Maximum buckets per rule. Prevents memory exhaustion.             | `100000`         |
| `zone`           | `<string>` | Named zone shared across hosts.                                   | none             |
| `throttle`       | `<bool>`   | Delay requests instead of rejecting them when the bucket is empty. | `false`          |
| `log_rejections` | `[bool]`   | Per-request rejection debug logs.                                 | `false`          |

Without `log_rejections`, Ferron logs only the first rejection for a key, then at most one per minute.

## Key types

| Value                   | Bucket key                                                          |
| ----------------------- | ------------------------------------------------------------------- |
| `remote_address`        | Client IP address. This is the default.                             |
| `uri`                   | Request URI path.                                                   |
| `request.header.<name>` | Value of a request header, for example `request.header.X-Api-Key`.  |

A request with no valid key, for example because the header is absent, skips that rule.

## Behavior

By default Ferron keeps rate limit state in memory, per server instance. For shared state across instances, configure a `rate_limit_backend` block with `type redis` (Redis or Valkey, which use the same RESP protocol).

### Token bucket algorithm

Each key gets its own bucket:

- Capacity is `rate + burst` tokens, and the bucket starts full.
- Ferron refills `rate` tokens per second, lazily on each request.
- Each request consumes one token.

When the bucket is empty, Ferron rejects the request with `deny_status`. The response carries `Retry-After` with the number of seconds to wait.

### Bucket eviction

Ferron removes a bucket after `bucket_ttl` seconds without activity. This stops one-shot clients from growing memory without limit.

`max_buckets` is a hard upper limit per rule. At the limit, Ferron rejects new requests until stale buckets are removed. `max_buckets` applies only to the in-memory backend. Redis keys expire through `bucket_ttl`.

### Distributed backend with Redis or Valkey

`rate_limit_backend` is a sibling of `rate_limit`, not a nested directive. A host or location without its own block inherits the global block. Without any block, Ferron uses the in-memory backend.

A named host does not inherit `rate_limit_backend` from the wildcard `*` host. It still inherits the global block.

```ferron
{
    rate_limit_backend {
        type redis
        url redis://127.0.0.1:6379/0
        key_prefix "ferron:rl:"
        timeout 200
        fail_open
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

| Nested directive | Arguments    | Description                                                                 | Default      |
| ---------------- | ------------ | --------------------------------------------------------------------------- | ------------ |
| `type`           | `<string>`   | `memory` or `redis`. `valkey` is an alias for `redis`.                      | `memory`     |
| `url`            | `<string>`   | Redis or Valkey URL. Required for `type redis`.                             | none         |
| `key_prefix`     | `<string>`   | Prefix for every Redis key. Zone, rule fingerprint, and user key are appended. | `ferron:rl:` |
| `timeout`        | `<duration>` | Per-request Redis timeout in milliseconds, or a duration string such as `"2s"`. | `200`        |
| `fail_open`      | `<bool>`     | Allow (`true`) or deny (`false`) requests on Redis errors.                  | `true`       |

Redis uses the same token bucket rules as memory, through an atomic Lua script: capacity `rate + burst`, refill `rate` per second, and key TTL `bucket_ttl`.

`fail_open true`, the default, favors availability. Traffic is allowed during a Redis outage. For high-security endpoints such as login, set `fail_open false`. Either way, Ferron emits `ferron.ratelimit.backend_errors` and a `WARN` log on backend failures.

With `throttle true` over Redis, Ferron sleeps once for `Retry-After`, capped at 30 seconds, and retries once. Memory throttling can wait longer through the bucket.

Override the backend for one location:

```ferron
example.com {
    rate_limit_backend {
        type redis
        url redis://127.0.0.1:6379/0
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

A `rate_limit` block inside a `location` applies only to requests that match that path. Ferron evaluates host rules and location rules. A request must pass all of them.

### Rate limit zones

By default each host has its own buckets. A zone lets several hostnames share buckets, or lets a host opt out of a global zone.

Zone resolution order:

1. The host `rate_limit` block has `zone name`. The host joins the named zone.
2. The host has its own `rate_limit` block without `zone`, and a global zone exists. The host gets a per-host zone and opts out of the global zone.
3. A global `rate_limit` block exists without `zone` blocks. All hosts without an explicit zone share the global zone.
4. None of the above apply. Each host gets a per-host zone.

Global zone. Both hosts share one pool, keyed by client IP.

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

Named zone. Both hosts join `api` and share buckets.

```ferron
{
    rate_limit {
        zone api
    }
}

api.example.com {
    rate_limit {
        zone api
        rate 50
        burst 10
        key remote_address
    }
}

api-v2.example.com {
    rate_limit {
        zone api
        rate 50
        burst 10
        key remote_address
    }
}
```

Opt out of a global zone. `internal.example.com` has its own `rate_limit` block, so it gets a per-host zone. `example.com` keeps using the global zone.

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
    # Has its own rate_limit block, which uses per-host zone (opting out of global)
}
```

### Configuration reload

In-memory buckets do not survive a configuration reload. A reload starts with fresh buckets and the new configuration. Redis-backed buckets survive reloads because they live in Redis and expire through `bucket_ttl`.

## Examples

Basic IP-based limit: 15 requests burst, then 10 per second for each IP.

```ferron
example.com {
    rate_limit {
        rate 10
        burst 5
        key remote_address
    }
}
```

API key limit: 150 requests burst, then 50 per second for each key.

```ferron
api.example.com {
    rate_limit {
        rate 50
        burst 100
        key request.header.X-Api-Key
    }
}
```

## Observability

### Metrics

| Metric                            | Type    | Attributes                                                                                                        | Description                                                          |
| --------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `ferron.ratelimit.allowed`        | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`ip`, `header`, `uri`), `ferron.ratelimit.backend`          | Requests that passed rate limiting.                                  |
| `ferron.ratelimit.rejected`       | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`ip`, `header`, `uri`), `ferron.ratelimit.backend`          | Requests rejected by empty buckets or a registry at capacity.        |
| `ferron.ratelimit.throttled`      | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type` (`ip`, `header`, `uri`), `ferron.ratelimit.backend`          | Requests delayed by throttling.                                      |
| `ferron.ratelimit.backend_errors` | Counter | `ferron.ratelimit.zone`, `ferron.ratelimit.key_type`, `ferron.ratelimit.backend`                                  | Backend errors, such as Redis unavailable or a timeout.              |

`ferron.ratelimit.zone` is `global` for the shared zone, the zone name for named zones, and the hostname for per-host zones. `ferron.ratelimit.backend` is `memory` or `redis`.

### Logs

- `INFO`: the first rejection for a key, then at most one per minute while the key stays over limit. The raw key is omitted because it can identify a client. The `ferron.ratelimit.rejected` counter shows rejection volume.
- `DEBUG`: per-request `Rate limit bucket exhausted` logs, only with `log_rejections`.
- `WARN`: the registry reaches `max_buckets` and applies backpressure.
- `WARN`: the Redis backend errors (`Rate limit backend error`). Fail-open allows the request, fail-closed denies it.

| Event                       | Level | Attributes                                                                                                                                         |
| --------------------------- | ----- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rate limit rejecting key    | INFO  | `ferron.ratelimit.zone` (string), `ferron.ratelimit.key_type` (`ip`, `uri`, `header`), `ferron.ratelimit.backend` (string), `ferron.ratelimit.limit` (int). |
| Rate limit bucket exhausted | DEBUG | `ferron.ratelimit.zone` (string), `ferron.ratelimit.key` (string), `ferron.ratelimit.key_type` (`ip`, `uri`, `header`). Only with `log_rejections`.     |

### Access log fields

| Field                               | Type   | Description                                          |
| ----------------------------------- | ------ | ---------------------------------------------------- |
| `ferron.ratelimit.result`           | string | `allowed` or `rejected`.                             |
| `ferron.ratelimit.zone`             | string | Rate limit zone identifier.                          |
| `ferron.ratelimit.retry_after_secs` | int    | Seconds until the next allowed request. Rejection only. |

### Trace spans

The rate limit stage sets these attributes on its `ferron.stage.rate_limit` span.

| Attribute                           | Type   | Description                                                       |
| ----------------------------------- | ------ | ----------------------------------------------------------------- |
| `ferron.ratelimit.result`           | string | `allowed`, `throttled`, or `rejected`.                            |
| `ferron.ratelimit.zone`             | string | Rate limit zone name.                                             |
| `ferron.ratelimit.key_type`         | string | Key extractor type: `ip`, `uri`, or `header`.                     |
| `ferron.ratelimit.backend`          | string | Backend type: `memory` or `redis`.                                |
| `ferron.ratelimit.limit`            | int    | Configured rate limit in requests per second.                     |
| `ferron.ratelimit.retry_after_secs` | int    | Seconds until the bucket is available. Rejection only.            |
