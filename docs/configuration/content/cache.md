---
title: "Configuration: HTTP cache"
description: "In-memory HTTP response caching with RFC 9111 behavior, an optional LSCache mode, and cache observability."
---

`cache` stores complete `GET` responses in memory and serves `HEAD` from the cached `GET` metadata. By default it follows RFC 9111. It also understands a subset of LiteSpeed Cache response headers for LSCache-aware applications.

The cache applies to final HTTP responses from static file serving, reverse proxying, and other response stages.

For static file cache headers such as `file_cache_control` and `etag`, see [Static file serving](/docs/configuration/content/static-files). For response headers and reverse proxying, see [HTTP headers and CORS](/docs/configuration/content/headers) and [Reverse proxying](/docs/configuration/proxy/reverse-proxy).

## `cache`

```ferron
{
    cache {
        max_entries 2048
    }
}

example.com {
    cache {
        max_response_size 1048576
        litespeed_override_cache_control false
        vary Accept-Encoding Accept-Language
        vary_cookies lang ab_bucket
    }

    location /admin {
        cache false
    }
}
```

At HTTP host scope, write `cache` as a block or as a boolean flag. The block enables caching and configures nested directives. The boolean form enables or disables inherited caching without changing nested settings.

### Global cache block

The global `cache { ... }` block configures shared capacity and named zones. It does not enable caching for any host.

| Nested directive   | Arguments  | Description                                                        | Default |
| ------------------ | ---------- | ------------------------------------------------------------------ | ------- |
| `max_entries`      | `<int>`    | Maximum responses in the shared cache. `0` stops new entries.      | `1024`  |
| `zone`             | block      | Named cache zone with its own capacity.                             | none    |
| `persist`          | `<string>` | Directory for disk persistence.                                     | none    |
| `persist_interval` | `<string>` | Interval for writing queued mutations to disk. Minimum is `1s`.     | `30s`   |
| `persist_private`  | `[<bool>]` | Also write private cache entries to disk.                           | `false` |

With `max_entries 0`, the module stays loaded but stores no new entries. See [Cache zones](#cache-zones) and [Disk persistence](#disk-persistence).

### Host cache block

The host `cache { ... }` block enables caching for that host or `location` and tunes how Ferron stores responses.

| Nested directive                   | Arguments                 | Description                                             | Default             |
| ---------------------------------- | ------------------------- | ------------------------------------------------------- | ------------------- |
| `max_response_size`                | `<int>`                   | Largest response body to store, in bytes.               | `2097152`           |
| `litespeed_override_cache_control` | `[<bool>]`                | Let `X-LiteSpeed-Cache-Control` override standard headers. | `false`          |
| `emit_litespeed_headers`           | `[<bool>]`                | Emit `X-LiteSpeed-Cache-Control` on cache hits.         | `false`             |
| `purge_method`                     | `[<bool>]`                | Accept the `PURGE` HTTP method.                         | `false`             |
| `purge_allowed_ips`                | `<string> [<string> ...]` | IP addresses or CIDR ranges allowed to send `PURGE`.    | none                |
| `vary`                             | `<string> [<string> ...]` | Extra request headers in the cache key.                 | none                |
| `vary_cookies`                     | `<string> [<string> ...]` | Cookie names in the cache key.                          | none                |
| `ignore`                           | `<string> [<string> ...]` | Response headers removed from the stored copy.          | none                |
| `ignore_request_cache_control`     | `[<bool>]`                | Ignore request cache control in favor of the policy.    | `false`             |
| `enable_stale_while_revalidate`    | `[<bool>]`                | Revalidate expired entries synchronously.               | `true`              |
| `enable_stale_if_error`            | `[<bool>]`                | Serve stale entries when the upstream returns 5xx.      | `true`              |
| `coalesce_timeout`                 | `<int>` (seconds)         | Wait time for a request coalesced onto an in-flight fetch. | `5`              |
| `purge_propagation`                | block                     | Propagate purges to other instances.                    | disabled            |
| `zone`                             | `<string>`                | Named cache zone to join.                               | implicit per-host   |
| `max_entries`                      | `<int>`                   | Entry capacity for the host cache.                      | global or `1024`    |
| `persist`                          | `<string>`                | Directory for disk persistence. Overrides outer scopes. | inherited           |
| `persist_interval`                 | `<string>`                | Interval for writing queued mutations to disk.          | `30s`               |
| `persist_private`                  | `[<bool>]`                | Also write private cache entries to disk.               | `false`             |

Responses larger than `max_response_size` are still served, but Ferron does not store them.

`purge_method` requires either HTTP basic authentication in the same scope or a matching `purge_allowed_ips` entry. Ferron returns `403 Forbidden` for an unauthenticated request from an IP that is not allowed. `purge_allowed_ips` passes matching IPs and also passes requests that already authenticated with basic authentication.

`vary` adds request headers to the cache key, on top of the standard `Vary` response headers from the origin. `vary_cookies` adds named cookies to the key. A cookie listed in `vary_cookies` also counts as client identity for private responses. This keeps high-entropy tracking or session cookies from fragmenting the cache.

`ignore` removes response headers from the stored copy only. The live response still includes them.

`coalesce_timeout` sets how long a request that coalesces onto an in-flight upstream fetch waits for the leader. After the timeout, the request stops coalescing and fetches from the upstream itself. This stops one hung upstream request from stalling every concurrent request for the same entry.

You can specify `purge_allowed_ips`, `vary`, `vary_cookies`, and `ignore` more than once. `enable_stale_while_revalidate` and `enable_stale_if_error` are described in [Stale-while-revalidate](#stale-while-revalidate) and [Stale-if-error](#stale-if-error). Zone and persistence options are described in [Cache zones](#cache-zones) and [Disk persistence](#disk-persistence).

```ferron
example.com {
    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
        vary Accept-Encoding Accept-Language
        vary_cookies lang ab_bucket
        ignore Set-Cookie
    }
}
```

> [!important]
> `litespeed_override_cache_control` makes `X-LiteSpeed-Cache-Control` override standard HTTP caching rules. Ferron then intentionally does not follow RFC 9111. Enable it only for upstreams that expect LiteSpeed-style semantics. Request headers such as `Cache-Control: no-cache` and `Pragma: no-cache` still affect lookup, unless `ignore_request_cache_control` is enabled.

### Boolean form

| Form          | Effect                                                        |
| ------------- | ------------------------------------------------------------- |
| `cache`       | Enables caching for the current host or `location` scope.      |
| `cache true`  | Explicitly enables caching for the current scope.              |
| `cache false` | Disables caching for the current scope, overriding an inherited block. |

Caching is off by default.

### Purge propagation block

`purge_propagation { ... }` goes inside a host `cache { ... }` block. A local purge, from the `PURGE` method or an `X-LiteSpeed-Purge` header, sends a webhook POST to an external control plane. The control plane then broadcasts `PURGE` requests to the other edge instances.

| Nested directive    | Arguments  | Description                                                             | Default     |
| ------------------- | ---------- | ----------------------------------------------------------------------- | ----------- |
| `control_plane_url` | `<string>` | Control-plane endpoint that receives purge events.                      | none        |
| `shared_secret`     | `<string>` | Secret sent as `X-Purge-Secret` and verified on inbound propagated purges. | none     |
| `node_id`           | `<string>` | Identifier for this edge in outbound webhook payloads.                  | `"unknown"` |

```ferron
example.com {
    cache {
        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url http://control-plane:9090/cache/purge
            shared_secret my-secret
            node_id edge-1
        }
    }
}
```

Ferron only sends the outbound webhook and handles inbound `PURGE` broadcasts. The control plane does the broadcasting. See [Cache purge propagation](#cache-purge-propagation) for the protocol and loop prevention.

> [!tip]
> Use HTTPS for `control_plane_url` in production. The shared secret and purge payloads travel in the webhook.

## Behavior

### Cache eligibility

- Only `GET` and `HEAD` requests trigger cache lookups.
- `HEAD` reuses cached `GET` representations and returns headers only.
- Non-`GET` responses are not stored. They may still trigger LSCache-compatible purge headers.
- Responses with `Vary: *` are never stored.
- Responses with `no-store` in `Cache-Control` are never stored.
- Responses with `206 Partial Content` bypass the cache when the request has a `Range` header.
- Built-in error responses generated after the main pipeline are not stored.

### Cache zones

A zone decides which hosts share one physical cache store. There are three zone types:

- Global zone: when a global `cache { max_entries N }` block exists without explicit `zone` blocks, all hosts share that store by default.
- Named zone: defined at global scope with `zone name { max_entries N }`. Several hosts can join the same zone.
- Per-host zone: the hostname gets its own store. Ferron uses this when no global zone exists and the host does not set `zone`.

Global zone example. Both hosts share the 4096-entry store.

```ferron
{
    cache {
        max_entries 4096
    }
}

example.com {
    cache
}

www.example.com {
    cache
}
```

Named zone example. Both hosts use the 8192-entry `shared_assets` store.

```ferron
{
    cache {
        max_entries 4096
        zone shared_assets {
            max_entries 8192
        }
    }
}

example.com {
    cache {
        zone shared_assets
    }
}

www.example.com {
    cache {
        zone shared_assets
    }
}
```

To opt a host out of the global zone, give it a unique `zone` name or set `max_entries` in its host block:

```ferron
{
    cache {
        max_entries 4096
    }
}

example.com {
    cache  # uses global zone
}

admin.example.com {
    cache {
        max_entries 2048  # implicitly creates a per-host zone
    }
}
```

Here `example.com` uses the global 4096-entry store. `admin.example.com` gets its own 2048-entry store.

Cache keys include the full URL, so `https://example.com/page` and `https://www.example.com/page` are separate entries even in one zone. A zone only decides which store holds the entries. The key uses the resolved vhost, not the raw `Host` header, and is lowercased. A `Host` header that differs in case, has a trailing dot, or spoofs another vhost cannot fragment or leak a tenant's entries.

> [!important]
> In a named zone, set `max_entries` in the global `zone` block. Setting `max_entries` in a host block that also uses `zone` triggers a validation warning.

Zone resolution order:

1. Named zone: the host sets `zone name`. Capacity comes from the global `zone name { max_entries N }` block.
2. Host `max_entries`: the host sets `max_entries` without `zone`. Ferron creates an implicit per-host zone with that capacity, which overrides the global zone.
3. Global zone: the host sets no `zone` or `max_entries`, and a global `cache { max_entries N }` block exists without explicit `zone` blocks. The host shares the global store.
4. Per-host zone: none of the above apply. Ferron uses the hostname as the zone ID, with capacity from the host or global `max_entries`.

### Disk persistence

`persist` writes cache mutations to disk as a durability mirror. The in-memory cache remains the only lookup path. A full process restart restores the cache from disk. A SIGHUP config reload does not touch the on-disk cache.

Ferron stores each zone under `<persist dir>/<zone>`. Global zones use `global`, named zones use the zone name, and per-host zones use the hostname. Two files make up the store:

- `journal` holds recent mutations: entries stored and entries deleted.
- `snapshot` holds a compact dump of all entries. Ferron rebuilds it periodically.

Mutations go to the journal in batches, controlled by `persist_interval`. The minimum is `1s`. Each batch lands in the operating system page cache, so it survives a process crash but not a power loss. A clean shutdown flushes pending mutations and syncs the files. On restart, Ferron replays the snapshot and then the journal.

Settings apply per zone in this order:

1. The host `cache` block, when the host has its own zone.
2. The named `zone` block, for a named zone.
3. The global `cache` block, for the global zone.

By default Ferron persists public entries only. Set `persist_private` to persist private entries too. `ferron doctor` warns when you enable it, because private responses can contain personal data. Deletions such as `PURGE` persist regardless of `persist_private`, so a deleted private entry stays deleted after a restart.

> [!important]
> The on-disk format is machine-specific. Do not share a persistence directory between Ferron instances. One process writes to a directory at a time.

Example with a persistent named zone:

```ferron
{
    cache {
        max_entries 4096
        persist /var/cache/ferron
        persist_interval "10s"
        zone shared_assets {
            max_entries 8192
            persist /var/cache/ferron-shared
        }
    }
}

example.com {
    cache {
        zone shared_assets
    }
}
```

`example.com` restores from `/var/cache/ferron-shared` after a restart. Hosts on the global zone restore from `/var/cache/ferron`.

### PURGE method cache invalidation

With `purge_method`, Ferron accepts `PURGE` for cache invalidation. A `PURGE` request to a URL removes all cached entries for that URL, both public and private. Later requests fetch fresh content.

PURGE is scoped to the requesting host. In a shared zone, a `PURGE` from one host only removes that host's entries. When the request carries no host, Ferron uses the zone's default host.

A PURGE request must authenticate in one of two ways:

- HTTP basic authentication, with the `basic_auth` block in the same scope as `cache`.
- A source IP in `purge_allowed_ips`.

Basic auth is scoped. Credentials that authenticate a user on one host do not authorize a `PURGE` on another host without a `basic_auth` block. If neither condition holds, Ferron returns `403 Forbidden`.

```ferron
example.com {
    cache {
        purge_method
        purge_allowed_ips "127.0.0.1" "10.0.0.0/8"
    }
}
```

```ferron
example.com {
    cache {
        purge_method
    }
    basic_auth {
        users {
            user "$argon2id$..."
        }
    }
}
```

```http
PURGE /blog/post-123 HTTP/1.1
Host: example.com
```

### Public and private cache behavior

- Ferron does not store public responses that contain `Set-Cookie`.
- Ferron partitions private responses by client identity. An identity is an authenticated user, a recognized private cookie such as `phpsessid`, a cookie named in `vary_cookies`, or an automatic `_lscache_vary` cookie. Ferron never keys a private response on the client IP address alone.
- A private response with no client identity is served uncached and not stored. Clients behind the same public IP do not share private data.
- The private cache key uses at most 8 cookie components and truncates each cookie value to 256 characters. This bounds the key space against arbitrary session cookies.
- Ferron does not store responses to authorized requests unless the response explicitly allows shared caching. The response must include `public`, `s-maxage`, `must-revalidate`, or `proxy-revalidate`. A bare `max-age` does not allow shared caching.

### Stale-while-revalidate

When an upstream response has the `stale-while-revalidate` directive in `Cache-Control`, Ferron keeps serving the cached entry after `max-age` expires. Concurrent requests behave differently:

- Leader request: the first request that finds the expired entry revalidates synchronously with the upstream. The fresh response replaces the cache entry.
- Follower requests: requests that arrive while the leader revalidates get the stale cached response immediately.

One request still contacts the upstream for fresh content. No background task runs, and followers do not wait.

> [!note]
> Ferron 3 has no internal route invocation mechanism, so background revalidation is not supported. `stale-while-revalidate` always uses a synchronous upstream request for the leader.

The stale window comes from the origin `Cache-Control` header:

```http
Cache-Control: public, max-age=60, stale-while-revalidate=3600
```

This caches the response for 60 seconds and allows stale serving for up to 3600 seconds after expiry.

When Ferron serves a stale response this way, `Cache-Status` includes `detail=stale-while-revalidate`:

```http
Cache-Status: FerronCache; hit; detail=stale-while-revalidate,public; age=120
```

Responses with `must-revalidate` or `proxy-revalidate` never serve stale, even inside a `stale-while-revalidate` window. `s-maxage` implies `proxy-revalidate`. With either directive, Ferron treats the entry as fresh or a miss. It revalidates or returns a miss instead of serving stale content.

### Stale-if-error

When an upstream response has the `stale-if-error` directive in `Cache-Control`, Ferron can serve the stale copy when revalidation fails with a 5xx error:

```http
Cache-Control: public, max-age=300, stale-if-error=3600
```

This caches the response for 300 seconds and allows stale serving after an upstream error for up to 3600 seconds after expiry.

1. A request triggers revalidation. For example, the entry expired, or the client sent `Cache-Control: max-age=0`.
2. Ferron contacts the upstream and gets a 5xx status code.
3. If a valid stale entry with `stale-if-error` exists, Ferron serves it with a `Cache-Status` header containing `detail=stale-if-error`.
4. If no stale entry exists or the window has passed, Ferron returns the 5xx error to the client.

This keeps previously cached content available during transient backend failures.

### LSCache-compatible response headers

Ferron reads these response headers from upstream applications and origin handlers:

| Header                      | Description                                                          | Notes                                                                                                                       |
| --------------------------- | -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `X-LiteSpeed-Cache-Control` | Sets cache scope and TTL with LSCache directives.                    | Standard HTTP caching rules win unless `litespeed_override_cache_control` is enabled.                                        |
| `X-LiteSpeed-Vary`          | Adds LSCache vary dimensions.                                        | Supports `cookie=<name>` and `value=<name>`. `no-vary` disables all cookie vary dimensions for the response.                  |
| `X-LiteSpeed-Tag`           | Tags cached responses for later purge.                               | On private responses, `public:` prefixes stay public tags.                                                                   |
| `X-LiteSpeed-Purge`         | Purges cached responses by tag, URL, or wildcard.                    | A `stale` marker expires entries in place, so they keep serving stale while revalidating. A plain miss applies when no stale window exists. |
| `LSC-Cookie`                | Adds cache-safe cookie replay metadata.                              | Ferron converts it to `Set-Cookie` before sending the response.                                                              |
| `X-LiteSpeed-Cache`         | Shows hit, miss, or bypass status on outgoing responses.             | Ferron sets this header itself when enabled and ignores origin values.                                                       |

`X-LiteSpeed-Cache-Control` accepts LSCache-style directives such as `public`, `private`, `max-age`, `s-maxage`, `no-cache`, `no-store`, and `no-vary`.

`X-LiteSpeed-Vary: value=<name>` varies on the request variable `<name>`, resolved per request and empty when unset. Populate it with `set_var`, which runs before cache lookup:

```ferron
*:80 {
  set_var request.header.user_agent r"Mobile|Android" device_class {
    value mobile
  }
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    litespeed_override_cache_control true
  }
}
```

With the upstream sending `X-LiteSpeed-Vary: value=device_class`, mobile clients share one variant and other clients share the default variant. As in LiteSpeed, only one value dimension is supported and the last `value=` wins. Keep values low-entropy, such as a device class or country. Ferron normalizes them, truncates them past 256 characters, and includes them in the key once declared. `ignore` affects only the stored representation. The live response still includes those headers unless another module removes them.

### Automatic vary cookies

With `litespeed_override_cache_control`, Ferron adds request cookies whose names start with `_lscache_vary` to the cache key. The `_litespeed_vary` prefix is an alias. A request without these cookies uses a blank vary string. A request with `_lscache_vary=Alabama` gets a different entry than one with `_lscache_vary=California`.

> [!note]
> `X-LiteSpeed-Cache-Control: no-vary` disables this behavior. Ferron then ignores all vary cookies for that response, including the automatic ones.

### Cache purge propagation

With `purge_propagation`, Ferron takes part in multi-instance invalidation through an external control plane:

1. A local purge occurs, from a `PURGE` request or an `X-LiteSpeed-Purge` response header from the upstream.
2. Ferron sends an HTTP `POST` to `control_plane_url` with a JSON body that contains the purged path and the originating node ID.
3. The control plane sends `PURGE` requests to all other registered edges, excluding the origin.
4. Each edge receives a `PURGE` with `X-Purge-Source: propagation`, verifies the shared secret, and purges locally without re-propagating.

Webhook from an edge to the control plane:

```http
POST /cache/purge HTTP/1.1
Host: control-plane:9090
Content-Type: application/json
X-Purge-Secret: <shared_secret>

{
  "path": "/blog/post-123",
  "origin": "edge-1"
}
```

Broadcast from the control plane to an edge:

```http
PURGE /blog/post-123 HTTP/1.1
Host: edge-2:80
X-Purge-Source: propagation
X-Purge-Secret: <shared_secret>
```

An edge returns `403 Forbidden` when `X-Purge-Secret` does not match `purge_propagation.shared_secret`. The comparison is constant-time. An edge also rejects a propagation claim when no secret is configured. A client therefore cannot mark its own purge as propagated to skip the normal `PURGE` authorization.

Ferron uses two mechanisms to prevent purge loops:

- The `X-Purge-Source: propagation` header. When an edge sees this header, it purges locally and does not forward the request to the control plane. A matching `X-Purge-Secret` is also required.
- Origin exclusion. The control plane removes the originating node, identified by the `origin` field, from its broadcast list. The origin does not receive its own purge back.

The external control plane must:

1. Accept `POST` requests at the configured URL with a JSON body containing `path` and `origin`.
2. Authenticate requests with the `X-Purge-Secret` header.
3. Keep a list of registered edge instance URLs.
4. Send `PURGE` requests to all registered edges except the origin, with `X-Purge-Source: propagation` and `X-Purge-Secret: <shared_secret>`.

Ferron does not include a control plane. Implement one with any HTTP framework, or use an existing cache coordination service.

## Observability

### Metrics

| Metric                                     | Type    | Attributes                                                                                                  | Description                                                                  |
| ------------------------------------------ | ------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `ferron.cache.requests`                    | Counter | `ferron.cache.zone`, `ferron.cache.result`, `ferron.cache.scope`, `ferron.cache.reason` (miss/bypass only)   | Cache hits, misses, bypasses, and revalidations.                             |
| `ferron.cache.entries`                     | Gauge   | `ferron.cache.zone`                                                                                         | Current number of cached entries.                                            |
| `ferron.cache.stores`                      | Counter | `ferron.cache.zone`, `ferron.cache.scope`, `http.response.status_code`                                      | Responses stored in the cache.                                               |
| `ferron.cache.evictions`                   | Counter | `ferron.cache.zone`, `ferron.cache.reason` (`expired` or `size`)                                            | Entries evicted from the cache.                                              |
| `ferron.cache.purges`                      | Counter | `ferron.cache.zone`, `ferron.cache.scope`                                                                   | Entries purged through LSCache controls.                                     |
| `ferron.cache.zero_purge`                  | Counter | `ferron.cache.zone`, `ferron.cache.scope`                                                                   | Purge requests that matched no entries.                                      |
| `ferron.cache.coalesced_requests`          | Counter | none                                                                                                        | Requests intercepted by singleflight deduplication.                          |
| `ferron.cache.singleflight_active_locks`   | Gauge   | none                                                                                                        | Active in-flight upstream fetches coordinated by singleflight.               |
| `ferron.cache.persistence_errors`          | Counter | `ferron.cache.zone`                                                                                         | Persistence failures: a journal flush failure or a snapshot compaction failure. |
| `ferron.cache.persistence_dropped_records` | Counter | `ferron.cache.zone`                                                                                         | Journal records dropped because the zone write queue exceeded capacity.      |
| `ferron.cache.persistence_active`          | Gauge   | `ferron.cache.zone`                                                                                         | `0` after a journal flush failure. Not emitted while the zone is healthy.    |

`ferron.cache.zone` is `global` for the shared global zone, the zone name for named zones, and the hostname for per-host zones.

Persistence runs on a background task, so its metrics follow the writer schedule rather than per-request events. The structured logs below also show persistence health. `persist_interval` controls how often the journal is written and sets the trade-off between write amplification and data loss on a crash.

### Logs

- `DEBUG`: Ferron skips storing a response because the body exceeds `cache.max_response_size`.
- `DEBUG`: Ferron purges through `X-LiteSpeed-Purge`.
- `DEBUG`: Ferron purges through the `PURGE` method.
- `DEBUG`: Ferron performs an LSCache stale purge. Entries stay stored and serve stale until revalidated.
- `WARN`: outbound purge propagation to the control plane fails.

### Structured logs

| Event                                                              | Level | Attributes                                                                           |
| ------------------------------------------------------------------ | ----- | ------------------------------------------------------------------------------------ |
| Skipping store because the response body exceeded the maximum size | DEBUG | none                                                                                 |
| Cache purged via LSCache controls                                  | DEBUG | `cache.purged.count` (purged entries)                                                |
| Cache purged via the PURGE method                                  | DEBUG | `cache.purged.count` (purged entries)                                                |
| LSCache stale purge                                                | DEBUG | none                                                                                 |
| Cache entries evicted                                              | DEBUG | `eviction.reason` (string), `eviction.count` (integer), `ferron.cache.zone` (string)  |
| Cache entries restored from disk at startup                       | DEBUG | `ferron.cache.zone` (string)                                                         |
| Truncated tail in persistence files, treated as a clean stop       | DEBUG | `ferron.cache.zone` (string)                                                         |
| Snapshot compaction completed                                      | DEBUG | `ferron.cache.zone` (string)                                                         |
| Could not read the persistence files on disk                       | WARN  | `ferron.cache.zone` (string)                                                         |
| Corrupted record in the persistence files, replay stopped          | WARN  | `ferron.cache.zone` (string)                                                         |
| Cache persistence journal flush failed                             | WARN  | `ferron.cache.zone` (string), `error` (string)                                       |
| Snapshot compaction failed                                         | WARN  | `ferron.cache.zone` (string)                                                         |
| Journal records dropped because the queue exceeded capacity        | WARN  | `ferron.cache.zone` (string), `cache.dropped.count` (integer)                        |

### Access log fields

| Field                                    | Type   | Description                                                                                                                  |
| ---------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `ferron.cache.result`                    | string | Lookup outcome: `hit`, `miss`, `bypass`, `stale`, `revalidate`, `purge`, or `purge_rejected`.                                 |
| `ferron.cache.zone`                      | string | Cache zone that served the request.                                                                                          |
| `ferron.cache.key_fingerprint`           | string | Truncated cache key, up to 48 characters, without the query string. Helps diagnose misses. May include `q=<16 hex chars>`, and `h=<16 hex chars>` when the base key was truncated. |
| `ferron.cache.detail`                    | string | Why the response was not served from or stored in the cache. Present on `miss` and `bypass`.                                  |
| `ferron.cache.bypass_reason`             | string | Request-side reason the lookup was bypassed, such as `request-only-if-cached`.                                              |
| `ferron.cache.coalesced`                 | bool   | Whether this request was a follower held behind a singleflight lock for the same key.                                        |
| `ferron.cache.coalesce_wait_duration_ms` | float  | Milliseconds the follower waited for the leader. `0` for leaders and requests that did not coalesce.                          |

### Trace spans

The cache stage sets these attributes on its `ferron.stage.cache` span.

| Attribute                            | Type   | Description                                                                          |
| ------------------------------------ | ------ | ------------------------------------------------------------------------------------ |
| `ferron.cache.result`                | string | Lookup result: `hit`, `miss`, `bypass`, `revalidate`, `stale`, or `purge`.            |
| `ferron.cache.zone`                  | string | Cache zone that served the request.                                                  |
| `ferron.cache.scope`                 | string | Cache scope (`public` or `private`), when available.                                 |
| `ferron.cache.detail`                | string | Extra detail about the cache decision, such as a bypass or skip reason.               |
| `ferron.cache.key.uri`               | string | Request URI path and query. Helps debug hit-rate loss from high-cardinality metadata. |
| `ferron.cache.key.method`            | string | HTTP method of the request.                                                          |
| `ferron.cache.key.evaluated_cookies` | string | Semicolon-separated cookie names used in the vary rule, when available.              |

## Best practices

`ferron doctor` reports these checks for directives on this page.

- `litespeed_override_cache_control`: LiteSpeed cache headers override standard HTTP cache policy. Enable only for applications that need LiteSpeed-compatible semantics.
- `ignore_request_cache_control`: Ferron ignores request cache control and uses the configured policy.
- `purge_method` without access control: cache purging without `purge_allowed_ips` or `basic_auth` in the same scope allows unauthenticated invalidation.
- `purge_allowed_ips` with a wildcard: allowing every source address for purges risks abuse. Restrict it to trusted operators or internal networks.
- `control_plane_url` without `shared_secret`: any source can trigger purges across all edge instances.
- `control_plane_url` over HTTP: the webhook exposes the shared secret and purge payloads. Use HTTPS in production.
- `persist_private`: private entries are written to disk. Enable it only when the directory is protected and your privacy policy requires it.
