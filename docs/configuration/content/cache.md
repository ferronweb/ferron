---
title: "Configuration: HTTP cache"
description: "In-memory HTTP response caching with RFC 9111 behavior, an optional LSCache override mode, and cache observability."
---

This page documents the `cache` directive to configure the Ferron in-memory HTTP response cache. The cache stores complete `GET` response representations and serves `HEAD` from cached `GET` metadata. It follows standard HTTP caching semantics by default. It also understands a subset of LiteSpeed Cache response headers for LSCache-aware applications.

The cache applies to final HTTP responses produced by static file serving, reverse proxying, and other response stages.

> [!info]
>
> - For static file cache headers such as `file_cache_control` and `etag`, see [Static file serving](/docs/configuration/content/static-files.md).
> - For response headers and reverse proxy configuration, see [HTTP headers and CORS](/docs/configuration/content/headers.md) and [Reverse proxying](/docs/configuration/proxy/reverse-proxy.md).

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
        ignore Set-Cookie
    }

    location /admin {
        cache false
    }
}
```

At HTTP host scope, you can write `cache` either as a block or as a boolean flag. Block form enables caching for that scope and configures nested directives. Boolean form is useful when you want to enable or disable inherited caching without changing any nested settings.

### Global `cache` block

Use the global `cache { ... }` block to configure shared cache capacity and named cache zones.

| Nested directive   | Arguments  | Description                                                                                                                                                                                                    | Default |
| ------------------ | ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `max_entries`      | `<int>`    | This directive specifies the maximum number of response entries stored in the shared in-memory HTTP cache. Setting this directive to `0` keeps the module loaded but prevents Ferron from storing new entries. | `1024`  |
| `zone`             | block      | Defines a named cache zone with custom capacity. See [Cache zones](#cache-zones) below.                                                                                                                        | (none)  |
| `persist`          | `<string>` | Directory for to-disk cache persistence. See [Disk persistence](#disk-persistence) below.                                                                                                                      | (none)  |
| `persist_interval` | `<string>` | How often Ferron writes queued cache mutations to disk. Minimum is `1s`.                                                                                                                                       | `30s`   |
| `persist_private`  | `[<bool>]` | Whether Ferron also writes private-scoped cache entries to disk.                                                                                                                                               | `false` |

**Configuration example:**

```ferron
{
    cache {
        max_entries 4096
        zone "shared_assets" {
            max_entries 8192
        }
    }
}
```

> [!tip]
> Global `cache { ... }` blocks are only for shared cache sizing. They do not enable caching for HTTP hosts by themselves.

### HTTP host `cache` block

Use the HTTP host `cache { ... }` block to enable caching and tune how the system stores responses for that host or matching `location`.

| Nested directive                   | Arguments                 | Description                                                                                                                                                                                                                                                                                                                                                                                                          | Default             |
| ---------------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| `max_response_size`                | `<int>`                   | The maximum response body size, in bytes, that Ferron can buffer and store in the cache. Ferron still serves responses larger than this limit, but it does not store them.                                                                                                                                                                                                                                           | `2097152`           |
| `litespeed_override_cache_control` | `[<bool>]`                | Whether `X-LiteSpeed-Cache-Control` overrides standard response caching headers such as `Cache-Control` and `Expires`. This applies when Ferron decides whether to store a response and what TTL to use. This mode is intentionally non-standard. Use it only for applications that expect LiteSpeed-style cache semantics.                                                                                          | `false`             |
| `emit_litespeed_headers`           | `[<bool>]`                | Whether Ferron emits the `X-LiteSpeed-Cache-Control` response header when serving a cached response.                                                                                                                                                                                                                                                                                                                 | `false`             |
| `purge_method`                     | `[<bool>]`                | Whether Ferron accepts the `PURGE` HTTP method for cache invalidation. When enabled, requests with method `PURGE` to a given URL remove all cached entries matching that URL. This directive requires either HTTP basic authentication or the `purge_allowed_ips` directive. Ferron rejects unauthenticated requests from non-allowed IPs with a 403 Forbidden response.                                             | `false`             |
| `purge_allowed_ips`                | `<string> [<string> ...]` | One or more IP addresses or CIDR ranges that may send `PURGE` requests. When non-empty, only requests from these IPs pass (unless the request is already authenticated via HTTP basic authentication). You can specify this directive multiple times.                                                                                                                                                                | none                |
| `vary`                             | `<string> [<string> ...]` | Additional request headers that Ferron adds to the cache key, alongside any standard `Vary` response headers returned by the origin. You can specify this directive multiple times.                                                                                                                                                                                                                                  | none                |
| `vary_cookies`                     | `<string> [<string> ...]` | Specific cookie names to include in the cache key. When set, Ferron uses only the listed cookies (along with any cookies added by the LSCache `X-LiteSpeed-Vary` header) for cache key differentiation. This prevents high-entropy tracking or session cookies from fragmenting the cache. A cookie listed here also counts as client identity for private responses. You can specify this directive multiple times. | none                |
| `ignore`                           | `<string> [<string> ...]` | Response headers that Ferron removes from the stored cache representation while leaving the live response unchanged. You can specify this directive multiple times.                                                                                                                                                                                                                                                  | none                |
| `ignore_request_cache_control`     | `[<bool>]`                | When enabled, Ferron ignores request-based cache control (for example, `Cache-Control`) in favor of the configured cache policy.                                                                                                                                                                                                                                                                                     | `false`             |
| `enable_stale_while_revalidate`    | `[<bool>]`                | When enabled, Ferron revalidates cached responses with a `stale-while-revalidate` directive synchronously after their `max-age` expires, instead of returning them immediately as a cache hit. See [Stale-while-revalidate](#stale-while-revalidate) below.                                                                                                                                                          | `true`              |
| `enable_stale_if_error`            | `[<bool>]`                | When enabled, Ferron serves cached responses with a `stale-if-error` directive from cache when the upstream returns a 5xx error during revalidation. See [Stale-if-error](#stale-if-error) below.                                                                                                                                                                                                                    | `true`              |
| `coalesce_timeout`                 | `<int>` (seconds)         | How long a request that coalesces onto an in-flight upstream fetch waits for the leader before it stops coalescing and fetches from the upstream itself. Guards against one hung upstream request stalling every concurrent request for the same entry.                                                                                                                                                              | `5`                 |
| `purge_propagation`                | block                     | Configures multi-instance cache purge propagation via an external control-plane service. See [Cache purge propagation](#cache-purge-propagation) below.                                                                                                                                                                                                                                                              | (disabled)          |
| `zone`                             | `<string>`                | Assign this host to a named cache zone. Hosts sharing the same zone name share a single cache store. If omitted, the host uses an implicit per-host zone. See [Cache zones](#cache-zones) below.                                                                                                                                                                                                                     | (implicit per-host) |
| `max_entries`                      | `<int>`                   | Maximum number of response entries for the host cache. When specified without `zone`, this implicitly creates a per-host zone with the given capacity, even if a global zone exists. See [Cache zones](#cache-zones) below.                                                                                                                                                                                          | (global or `1024`)  |
| `persist`                          | `<string>`                | Directory for to-disk cache persistence. Overrides the global and named-zone settings. See [Disk persistence](#disk-persistence) below.                                                                                                                                                                                                                                                                              | (inherited)         |
| `persist_interval`                 | `<string>`                | How often Ferron writes queued cache mutations to disk. Minimum is `1s`. Overrides the global and named-zone settings.                                                                                                                                                                                                                                                                                               | `30s`               |
| `persist_private`                  | `[<bool>]`                | Whether Ferron also writes private-scoped cache entries to disk. Overrides the global and named-zone settings.                                                                                                                                                                                                                                                                                                       | `false`             |

**Configuration example:**

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
> `litespeed_override_cache_control` makes Ferron treat `X-LiteSpeed-Cache-Control` as overriding standard HTTP caching rules. Ferron intentionally does not comply with RFC 9111. Enable it only when the upstream targets LiteSpeed-style cache semantics. Request-side directives such as `Cache-Control: no-cache` and `Pragma: no-cache` still affect cache lookup behavior normally (unless `ignore_request_cache_control` overrides them).

### Boolean `cache` form

| Form          | Description                                                                                                | Default |
| ------------- | ---------------------------------------------------------------------------------------------------------- | ------- |
| `cache`       | Enables caching for the current HTTP host or `location` scope.                                             | `false` |
| `cache true`  | Explicitly enables caching for the current scope.                                                          | `false` |
| `cache false` | Disables caching for the current scope, which is useful for overriding an inherited `cache { ... }` block. | `false` |

### `purge_propagation` block

Use the `purge_propagation { ... }` block inside a host `cache { ... }` block to propagate cache purges to other instances. Propagation uses an external control-plane service. When enabled, Ferron sends a webhook POST to the control-plane whenever a local purge occurs (via `PURGE` method or `X-LiteSpeed-Purge` header). The control-plane then broadcasts `PURGE` requests to all other registered edge instances.

| Nested directive    | Arguments  | Description                                                                                                                                  | Default     |
| ------------------- | ---------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `control_plane_url` | `<string>` | URL of the external control-plane endpoint to POST purge events to.                                                                          | (none)      |
| `shared_secret`     | `<string>` | Shared secret sent as the `X-Purge-Secret` header when pushing purge events to the control-plane, and verified on inbound propagated purges. | (none)      |
| `node_id`           | `<string>` | Identifier for this edge instance, included in outbound webhook payloads so the control-plane can avoid broadcasting back to the origin.     | `"unknown"` |

**Configuration example:**

```ferron
example.com {
    cache {
        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url "http://control-plane:9090/cache/purge"
            shared_secret "my-secret"
            node_id "edge-1"
        }
    }
}
```

> [!tip]
> Use HTTPS for `control_plane_url` in production environments to protect the shared secret and purge payloads in transit.

> [!important]
> The external control-plane broadcasts incoming purge webhooks to all other registered edge instances. Ferron only handles the outbound webhook and the inbound `PURGE` method for receiving broadcast purges. See [Cache purge propagation](#cache-purge-propagation) below for details on the webhook protocol and loop prevention.

## Behavior

### Cache eligibility

- Only `GET` and `HEAD` requests trigger cache lookups.
- `HEAD` requests reuse cached `GET` representations and return only headers.
- Non-`GET` responses are not stored, but they may still trigger LSCache-compatible purge headers.
- Responses with `Vary: *` are never stored.
- Responses with a `no-store` directive in `Cache-Control` header are never stored.
- Responses with `206 Partial Content` bypass cache if `Range` request header is present.
- Built-in error responses generated after the main HTTP pipeline are not currently stored.

### Cache zones

Cache zones determine which hosts share a physical cache store. There are three zone types:

- **Global zone**: When a global `cache { max_entries = N }` block exists (without explicit `zone` blocks), all hosts share a single cache store by default.
- **Named zone**: Explicitly defined at global scope via `zone "name" { max_entries = N }`. Multiple hostnames can reference the same named zone.
- **Per-host zone**: Each hostname gets its own independent cache store. Used when no global zone exists and the host does not specify an explicit `zone` directive.

**Global zone (default when a global cache block exists):**

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

In this configuration, both `example.com` and `www.example.com` share the same 4096-entry cache store. No explicit `zone` directive matters. The global `cache` block establishes a global zone.

**Named zones:**

```ferron
{
    cache {
        max_entries 4096
        zone "shared_assets" {
            max_entries 8192
        }
    }
}

example.com {
    cache {
        zone "shared_assets"
    }
}

www.example.com {
    cache {
        zone "shared_assets"
    }
}
```

Both hosts share the 8192-entry `shared_assets` zone.

**Opting out of the global zone:**

If a global zone exists but a host should have its own isolated cache, use `zone` with a unique name. Alternatively, specify `max_entries` in the host block:

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

Here `admin.example.com` gets its own 2048-entry cache, while `example.com` shares the global 4096-entry store.

> [!note]
> Cache keys still include the full URL (including hostname), so `https://example.com/page` and `https://www.example.com/page` are distinct cache entries even within the same zone. The zone only determines which physical cache store holds the entries. The key uses the resolved vhost (the host Ferron matched in configuration), not the raw `Host` header, and is lowercased. A `Host` header that differs in case, carries a trailing dot, or spoofs another vhost cannot fragment or leak a tenant's entries.

> [!important]
> When using named zones, define the `max_entries` capacity in the global `zone` block, not in the host-level `cache` block. Specifying `max_entries` in a host block that also uses `zone` triggers a validation warning.

Zone resolution follows this order:

1. **Named zone**: If the host specifies `zone "name"`, it uses the named zone `CacheStore`. Capacity comes from the global `zone "name" { max_entries = N }` definition.
2. **Host-level `max_entries`**: If the host specifies `max_entries` in its cache block (without `zone`), Ferron creates an implicit per-host zone with that capacity. This overrides the global zone.
3. **Global zone**: If the host specifies no `zone` or host-level `max_entries`, a global `cache { max_entries = N }` block must exist (without explicit `zone` blocks). The host then uses the global `CacheStore`. All hosts without an explicit `zone` share this store.
4. **Per-host zone**: If none of the above apply, Ferron uses the hostname as the zone ID. Capacity comes from the host-level or global `max_entries`.

### Disk persistence

When you set `persist` to a directory, Ferron writes cache mutations to disk as a durability mirror. The in-memory cache remains the only lookup path. When the process restarts, Ferron restores the cache contents from disk. A restart means a full process stop and start. A SIGHUP config reload does not touch the on-disk cache.

Ferron stores each zone under `<persist dir>/<zone>`, where `<zone>` is the zone label (global zones use `global`). Named zones use their zone name, and per-host zones use the hostname. Two files make up the store:

- `journal` holds recent cache mutations (entries stored and deletions).
- `snapshot` holds a compact dump of all entries. Ferron rebuilds it periodically.

Ferron writes mutations to the journal in batches. The `persist_interval` directive controls how often. The minimum interval is `1s`. Each batch lands in the operating system page cache, so it survives a process crash but not a power loss. During a clean shutdown, Ferron flushes all pending mutations and syncs the files. When the server restarts, Ferron replays the snapshot and then the journal.

Settings apply per zone in this order:

1. The host `cache` block, when the host has its own zone.
2. The named `zone` block, for named zones.
3. The global `cache` block, for the global zone.

**Private entries:**

By default Ferron persists only public entries. Set `persist_private` to persist private-scoped entries too. Enabling it triggers a best-practice warning from `ferron doctor`, because private responses can contain personal data. Deletions (for example `PURGE` requests) persist regardless of `persist_private`, so a private entry removed before a restart stays removed.

> [!important]
> The on-disk format is machine-specific. Do not share a persistence directory between Ferron instances, and do not point two servers at the same directory. One process writes to the directory at a time.

Example with a persistent named zone:

```ferron
{
    cache {
        max_entries 4096
        persist /var/cache/ferron
        persist_interval "10s"
        zone "shared_assets" {
            max_entries 8192
            persist /var/cache/ferron-shared
        }
    }
}

example.com {
    cache {
        zone "shared_assets"
    }
}
```

`example.com` restores its entries from `/var/cache/ferron-shared` after a restart. Hosts that use the global zone restore from `/var/cache/ferron`.

### PURGE method cache invalidation

When you enable the `purge_method` subdirective, Ferron accepts the `PURGE` HTTP method for cache invalidation. A `PURGE` request to a specific URL removes all cached entries (both public and private) matching that URL. This causes later requests to fetch fresh content.

PURGE is scoped to the requesting host. In a shared zone (named or global), a `PURGE` from one host only invalidates entries that were cached for that same host. It does not touch other hosts' entries. When the request carries no host, Ferron falls back to the zone's default host.

**Security:**

PURGE requests must be either:

- Authenticated via HTTP basic authentication, where the `basic_auth` block is configured in the same scope as the `cache` block, or
- Originating from an IP address matching the `purge_allowed_ips` list.

The basic-auth requirement is scoped: credentials that authenticate a user on one host do not authorize a `PURGE` on another host that has no `basic_auth` block in scope. If neither condition is met, Ferron returns a "403 Forbidden" response. This makes sure that Ferron never accidentally leaves cache purging unsecured.

**Example using trusted IP list:**

```ferron
example.com {
    cache {
        purge_method
        purge_allowed_ips "127.0.0.1" "10.0.0.0/8"
    }
}
```

**Example using basic authentication:**

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

**Example request:**

```http
PURGE /blog/post-123 HTTP/1.1
Host: example.com
```

### Public and private cache behavior

- Ferron does not store public responses containing `Set-Cookie`.
- Ferron partitions private responses by client identity. An identity is an authenticated user, a recognized private cookie (for example `phpsessid`), or a cookie named in `vary_cookies`. Ferron never keys a private response on the client IP address alone.
- When a private response arrives with no client identity, Ferron does not store it. It serves the response uncached instead, so clients behind the same public IP do not share private data.
- The private cache key uses at most 8 cookie components and truncates each cookie value to 256 characters. This bounds the key space against arbitrary session cookies.
- Ferron does not store responses to authorized requests unless the response explicitly authorizes shared caching. The response must include any of: `public`, `s-maxage`, `must-revalidate`, `proxy-revalidate`. A bare `max-age` without one of these directives does not authorize shared caching.

### Stale-while-revalidate

When an upstream response includes the `stale-while-revalidate` directive in its `Cache-Control` header, Ferron extends the cached entry lifetime beyond `max-age`. The behavior differs depending on concurrent request patterns:

- **Leader request**: The first request to encounter the expired entry becomes the leader and revalidates synchronously with the upstream. It receives a fresh response that replaces the cache entry.
- **Follower requests**: Ferron serves the stale cached response immediately to concurrent requests that arrive while the leader revalidates.

This makes sure that one request still contacts the upstream for fresh content. No background tasks run. Other concurrent requests avoid waiting for revalidation.

> [!note]
> Ferron 3 does not have an internal route invocation mechanism, so background revalidation is not supported. `stale-while-revalidate` always involves a synchronous upstream request for the leader, and followers receive the stale response.

The `stale-while-revalidate` duration comes from the origin `Cache-Control` header. For example:

```http
Cache-Control: public, max-age=60, stale-while-revalidate=3600
```

This caches the response for 60 seconds, then allows stale serving for up to 3600 seconds after expiry.

When Ferron serves a stale response via this mechanism, the `Cache-Status` response header includes `detail=stale-while-revalidate`:

```http
Cache-Status: FerronCache; hit; detail=stale-while-revalidate,public; age=120
```

#### Interaction with `must-revalidate` and `proxy-revalidate`

Per RFC 9111, responses with `must-revalidate` or `proxy-revalidate` directives (or `s-maxage`, which implies `proxy-revalidate`) never serve stale. This applies even within a `stale-while-revalidate` window. When either directive is present, Ferron treats the entry as strictly fresh-or-miss. It either revalidates or returns a miss rather than serving stale content.

### Stale-if-error

When an upstream response includes the `stale-if-error` directive in its `Cache-Control` header, Ferron can serve the stale cached response. It does this when revalidation fails with a 5xx server error:

```http
Cache-Control: public, max-age=300, stale-if-error=3600
```

This caches the response for 300 seconds. It allows stale serving on upstream errors for up to 3600 seconds after expiry.

How it works:

1. A request triggers revalidation (for example, the cached entry has expired, or the client sent `Cache-Control: max-age=0`).
2. Ferron contacts the upstream, which returns a 5xx status code.
3. If a valid stale entry with `stale-if-error` exists, Ferron serves the stale response with a `Cache-Status` header containing `detail=stale-if-error`.
4. If no stale entry exists or the `stale-if-error` window has elapsed, Ferron returns the 5xx error to the client.

This gives resilience against transient backend failures by falling back to previously cached content.

### LSCache-compatible response headers

When the cache module runs, Ferron understands the following response headers from upstream applications and origin handlers:

| Header                      | Description                                                                                                                                 | Notes                                                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `X-LiteSpeed-Cache-Control` | Controls cache scope and TTL using LSCache-style directives such as `public`, `private`, `max-age`, `s-maxage`, `no-cache`, and `no-store`. | By default, standard HTTP caching rules still take precedence. Enable `litespeed_override_cache_control` to prefer this header instead. |
| `X-LiteSpeed-Vary`          | Adds LSCache-style vary dimensions.                                                                                                         | Ferron supports `cookie=<name>`. Ferron does not support `value=<name>` yet and skips cache storage for that response.                  |
| `X-LiteSpeed-Tag`           | Assigns tags to cached responses so you can purge them later.                                                                               | On private responses, `public:` prefixes remain public tags.                                                                            |
| `X-LiteSpeed-Purge`         | Purges cached responses by tag, URL, or wildcard.                                                                                           | The `stale` marker currently falls back to an immediate hard purge.                                                                     |
| `LSC-Cookie`                | Adds cache-safe cookie replay metadata.                                                                                                     | Ferron converts this header to `Set-Cookie` before sending the response.                                                                |
| `X-LiteSpeed-Cache`         | Exposes cache hit, miss, or bypass status on outgoing responses.                                                                            | Ferron sets this header itself (if enabled). It ignores origin-provided values.                                                         |

> [!note]
> `X-LiteSpeed-Vary: value=...` is not supported yet because Ferron does not currently have a request-time equivalent of LiteSpeed rewrite-rule vary environment values. The `ignore` directive affects only the stored representation. The live response sent to the client still includes those headers unless another module removes them.

### Cache purge propagation

When you configure `purge_propagation`, Ferron participates in multi-instance cache invalidation through an external control-plane service. The propagation flow works as follows:

1. **Local purge occurs**: Either via a `PURGE` HTTP method request or an `X-LiteSpeed-Purge` response header from the upstream.
2. **Webhook sent**: Ferron sends an HTTP `POST` to the configured `control_plane_url` with a JSON body containing the purged path and the originating node ID.
3. **Control-plane broadcasts**: The external control-plane sends `PURGE` requests to all other registered edge instances, excluding the origin.
4. **Edges receive purges**: Other edges receive `PURGE` requests with an `X-Purge-Source: propagation` header, verify the shared secret, and execute the purge locally without re-propagating.

**Webhook protocol (edge to control-plane):**

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

**Broadcast protocol (control-plane to edge):**

```http
PURGE /blog/post-123 HTTP/1.1
Host: edge-2:80
X-Purge-Source: propagation
X-Purge-Secret: <shared_secret>
```

An edge rejects a propagation claim (HTTP 403) when the `X-Purge-Secret` value does not match the configured `purge_propagation.shared_secret`. The comparison is constant-time. A claim with no secret configured is also rejected. This makes sure that a client cannot tag its own purge as propagated to skip the normal `PURGE` authorization.

**Loop prevention:**

Ferron uses two mechanisms to prevent infinite purge loops:

- **`X-Purge-Source: propagation` header**: When an edge receives a `PURGE` request with this header, it executes the purge locally. It does not forward the request to the control-plane. This prevents re-propagation loops. A propagation claim must also carry a matching `X-Purge-Secret` value.
- **Origin exclusion**: The control-plane removes the originating node (identified by the `origin` field in the webhook payload) from its broadcast list. This prevents the origin from receiving its own purge back.

**Control-plane requirements:**

The external control-plane service must:

1. Accept `POST` requests at the configured URL with a JSON body containing `path` and `origin` fields.
2. Authenticate requests using the `X-Purge-Secret` header.
3. Maintain a list of registered edge instance URLs.
4. Send `PURGE` requests to all registered edges except the origin, including `X-Purge-Source: propagation` and `X-Purge-Secret: <shared_secret>` headers.

Ferron does not include a built-in control-plane. Operators can implement one using any HTTP framework or use an existing cache coordination service.

## Observability

### Metrics

The cache module emits the following metrics:

| Metric                                     | Type    | Attributes                                                                                                 | Description                                                                                         |
| ------------------------------------------ | ------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `ferron.cache.requests`                    | Counter | `ferron.cache.zone`, `ferron.cache.result`, `ferron.cache.scope`, `ferron.cache.reason` (miss/bypass only) | Cache hits, misses, bypasses, and revalidations.                                                    |
| `ferron.cache.entries`                     | Gauge   | `ferron.cache.zone`                                                                                        | Current number of cached entries                                                                    |
| `ferron.cache.stores`                      | Counter | `ferron.cache.zone`, `ferron.cache.scope`, `http.response.status_code`                                     | Responses stored in the cache                                                                       |
| `ferron.cache.evictions`                   | Counter | `ferron.cache.zone`, `ferron.cache.reason` (`"expired"` or `"size"`)                                       | Entries evicted from the cache                                                                      |
| `ferron.cache.purges`                      | Counter | `ferron.cache.zone`, `ferron.cache.scope`                                                                  | Entries purged through LSCache-compatible controls                                                  |
| `ferron.cache.coalesced_requests`          | Counter | None                                                                                                       | Requests intercepted by the singleflight deduplication layer                                        |
| `ferron.cache.singleflight_active_locks`   | Gauge   | None                                                                                                       | Active in-flight upstream fetches coordinated by singleflight                                       |
| `ferron.cache.persistence_errors`          | Counter | `ferron.cache.zone`                                                                                        | Cache persistence failures for the zone: a journal flush failure, or a snapshot compaction failure. |
| `ferron.cache.persistence_dropped_records` | Counter | `ferron.cache.zone`                                                                                        | Journal records dropped because the zone's write queue exceeded capacity under backpressure.        |
| `ferron.cache.persistence_active`          | Gauge   | `ferron.cache.zone`                                                                                        | `0` after a journal flush failure, not emitted while the zone is healthy.                           |

The `ferron.cache.zone` attribute identifies which cache zone the request belongs to. Ferron sets it to `"global"` for the shared global zone. For named zones, it uses the zone name. For per-host zones, it uses the hostname.

Persistence runs on a background task, so its metrics above are emitted on the writer's own schedule rather than per-request. Its health is also visible through the structured log events in the table below. The `persist_interval` directive controls how often the journal is written. It sets the trade-off between write amplification and data loss on a crash.

### Logs

- `DEBUG`: Logged when Ferron skips cache storage because `X-LiteSpeed-Vary: value=...` is not supported yet.
- `DEBUG`: Logged when Ferron skips cache storage because the response body exceeds `cache.max_response_size`.
- `DEBUG`: Logged when Ferron purges through `X-LiteSpeed-Purge`.
- `DEBUG`: Logged when Ferron purges through `PURGE` HTTP method.
- `DEBUG`: Logged when Ferron receives an LSCache `stale` purge marker and falls back to a hard purge.
- `WARN`: Logged when outbound purge propagation to the control-plane fails.

### Structured logs

| Description (summary)                                              | Level | Attributes                                                                           |
| ------------------------------------------------------------------ | ----- | ------------------------------------------------------------------------------------ |
| Skipping cache store because response body exceeded maximum size   | DEBUG | -                                                                                    |
| Skipping cache store because X-LiteSpeed-Vary is not supported yet | DEBUG | -                                                                                    |
| Cache purged via LSCache controls                                  | DEBUG | `cache.purged.count` (purged cache entries)                                          |
| Cache purged via PURGE method                                      | DEBUG | `cache.purged.count` (purged cache entries)                                          |
| LSCache stale purge marker ignored                                 | DEBUG | -                                                                                    |
| Cache entries evicted                                              | DEBUG | `eviction.reason` (string), `eviction.count` (integer), `ferron.cache.zone` (string) |
| Cache entries restored from disk at startup                        | DEBUG | `ferron.cache.zone` (string)                                                         |
| Truncated tail in the persistence files, treated as a clean stop   | DEBUG | `ferron.cache.zone` (string)                                                         |
| Snapshot compaction completed                                      | DEBUG | `ferron.cache.zone` (string)                                                         |
| Could not read the persistence files on disk                       | WARN  | `ferron.cache.zone` (string)                                                         |
| Corrupted record in the persistence files; replay stopped          | WARN  | `ferron.cache.zone` (string)                                                         |
| Cache persistence journal flush failed                             | WARN  | `ferron.cache.zone` (string), `error` (string)                                       |
| Snapshot compaction failed                                         | WARN  | `ferron.cache.zone` (string)                                                         |
| Journal records dropped because the write queue exceeded capacity  | WARN  | `ferron.cache.zone` (string), `cache.dropped.count` (integer)                        |

### Access log fields

The cache module contributes the following fields to the HTTP access log line:

| Field                                    | Type   | Description                                                                                                                                                                                                                                                                   |
| ---------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ferron.cache.result`                    | string | Cache lookup outcome: `hit`, `miss`, `bypass`, `stale`, `revalidate`, `purge`, or `purge_rejected`.                                                                                                                                                                           |
| `ferron.cache.zone`                      | string | The cache zone serving the request.                                                                                                                                                                                                                                           |
| `ferron.cache.key_fingerprint`           | string | Truncated cache key (up to 48 characters) with the query string removed, useful for diagnosing why a specific request missed. Optionally with a short non-reversible tag (`q=<16 hex chars>`), and if the base itself had to be truncated, a second tag (`h=<16 hex chars>`). |
| `ferron.cache.detail`                    | string | Why the response was not served from, or not stored in, the cache, when applicable. Present on `miss` and `bypass` outcomes.                                                                                                                                                  |
| `ferron.cache.bypass_reason`             | string | The request-side reason lookup was bypassed entirely (for example `request-only-if-cached`), when applicable.                                                                                                                                                                 |
| `ferron.cache.coalesced`                 | bool   | Whether this request was a follower held by an active singleflight lock while another request revalidated the same key.                                                                                                                                                       |
| `ferron.cache.coalesce_wait_duration_ms` | float  | Time in milliseconds the follower waited for the leader upstream response. `0` for leaders and non-coalesced requests.                                                                                                                                                        |

### Trace spans

The cache stage sets the following attributes on its `ferron.stage.cache` span:

| Attribute                            | Type   | Description                                                                                                    |
| ------------------------------------ | ------ | -------------------------------------------------------------------------------------------------------------- |
| `ferron.cache.result`                | string | Cache lookup result: `hit`, `miss`, `bypass`, `revalidate`, `stale`, or `purge`.                               |
| `ferron.cache.zone`                  | string | The cache zone serving the request.                                                                            |
| `ferron.cache.scope`                 | string | Cache scope (`public` or `private`), when available.                                                           |
| `ferron.cache.detail`                | string | Additional detail about the cache decision (bypass reason or skip reason), when applicable.                    |
| `ferron.cache.key.uri`               | string | The request URI path and query, useful for debugging hit-rate degradation caused by high-cardinality metadata. |
| `ferron.cache.key.method`            | string | The HTTP method of the request.                                                                                |
| `ferron.cache.key.evaluated_cookies` | string | Semicolon-separated list of cookie names used in the cache vary rule, when available.                          |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- **`litespeed_override_cache_control`**: This makes LiteSpeed cache headers override standard HTTP cache policy. Enable only for applications that require LiteSpeed-compatible semantics.
- **`ignore_request_cache_control`**: When enabled, Ferron ignores request-based cache control (for example, `Cache-Control`) in favor of the configured cache policy.
- **`purge_method` without access control**: Cache purging enabled without `purge_allowed_ips` or `basic_auth` in the same scope allows unauthenticated cache invalidation.
- **`purge_allowed_ips` with wildcard**: Allowing every source address for cache purging risks abuse. Restrict this to trusted operators or internal networks.
- **`control_plane_url` without `shared_secret`**: Purge propagation configured without a shared secret allows any source to trigger cache purges across all edge instances.
- **`control_plane_url` using HTTP**: Purge webhooks sent over unencrypted HTTP expose the shared secret and purge payloads. Use HTTPS in production environments.
- **`persist_private`**: Persisting private cache entries writes personal data to disk. Enable it only when the persistence directory is protected and your privacy policy requires it.
