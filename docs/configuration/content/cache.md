---
title: "Configuration: HTTP cache"
description: "In-memory HTTP response caching with RFC 9111 behavior, an optional LSCache override mode, and cache observability."
---

This page documents the `cache` directive for configuring Ferron's in-memory HTTP response cache. The cache stores complete `GET` response representations, serves `HEAD` from cached `GET` metadata, follows standard HTTP caching semantics by default, and understands a subset of LiteSpeed Cache response headers for LSCache-aware applications.

The cache applies to final HTTP responses produced by static file serving, reverse proxying, and other response stages.

> [!info]
>
> - For static file cache headers such as `file_cache_control` and `etag`, see [Static file serving](/docs/v3/configuration/content/static-files.md).
> - For response headers and reverse proxy configuration, see [HTTP headers and CORS](/docs/v3/configuration/content/headers.md) and [Reverse proxying](/docs/v3/configuration/proxy/reverse-proxy.md).

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
        ignore Set-Cookie
    }

    location /admin {
        cache false
    }
}
```

At HTTP host scope, `cache` can be written either as a block or as a boolean flag. Block form enables caching for that scope and configures nested directives. Boolean form is useful when you want to enable or disable inherited caching without changing any nested settings.

### Global `cache` block

Use the global `cache { ... }` block to configure shared cache capacity.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_entries` | `<int>` | This directive specifies the maximum number of response entries stored in the shared in-memory HTTP cache. Setting this directive to `0` keeps the module loaded but prevents new entries from being stored. | `1024` |

**Configuration example:**

```ferron
{
    cache {
        max_entries 4096
    }
}
```

> [!tip]
> Global `cache { ... }` blocks are only for shared cache sizing — they do not enable caching for HTTP hosts by themselves.

### HTTP host `cache` block

Use the HTTP host `cache { ... }` block to enable caching and tune how responses are stored for that host or matching `location`.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_response_size` | `<int>` | The maximum response body size, in bytes, that can be buffered and stored in the cache. Responses larger than this limit are still served, but they are not stored. | `2097152` |
| `litespeed_override_cache_control` | `[<bool>]` | Whether `X-LiteSpeed-Cache-Control` overrides standard response caching headers such as `Cache-Control` and `Expires` when Ferron decides whether to store a response and what TTL to use. This mode is intentionally non-standard and is intended only for applications that expect LiteSpeed-style cache semantics. | `false` |
| `emit_litespeed_headers` | `[<bool>]` | Whether the `X-LiteSpeed-Cache-Control` response header should be emitted when serving a cached response. | `false` |
| `purge_method` | `[<bool>]` | Whether the `PURGE` HTTP method is accepted for cache invalidation. When enabled, requests with method `PURGE` to a given URL will remove all cached entries matching that URL. This directive requires either HTTP basic authentication or the `purge_allowed_ips` directive; unauthenticated requests from non-allowed IPs are rejected with a 403 Forbidden response. | `false` |
| `purge_allowed_ips` | `<string> [<string> ...]` | One or more IP addresses or CIDR ranges that are allowed to send `PURGE` requests. When non-empty, only requests from these IPs are allowed (unless the request is already authenticated via HTTP basic authentication). This directive can be specified multiple times. | none |
| `vary` | `<string> [<string> ...]` | Additional request headers that are added to the cache key, alongside any standard `Vary` response headers returned by the origin. This directive can be specified multiple times. | none |
| `ignore` | `<string> [<string> ...]` | Response headers that are removed from the stored cache representation while leaving the live response unchanged. This directive can be specified multiple times. | none |
| `ignore_request_cache_control` | `[<bool>]` | When enabled, request-based cache control (e.g., `Cache-Control`) is ignored in favor of the configured cache policy. | `false` |
| `enable_stale_while_revalidate` | `[<bool>]` | When enabled, cached responses with a `stale-while-revalidate` directive are revalidated synchronously after their `max-age` expires instead of being returned immediately as a cache hit. See [Stale-while-revalidate](#stale-while-revalidate) below. | `true` |
| `enable_stale_if_error` | `[<bool>]` | When enabled, cached responses with a `stale-if-error` directive are served from cache when the upstream returns a 5xx error during revalidation. See [Stale-if-error](#stale-if-error) below. | `true` |

**Configuration example:**

```ferron
example.com {
    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
        vary Accept-Encoding Accept-Language
        ignore Set-Cookie
    }
}
```

> [!important]
> `litespeed_override_cache_control` makes Ferron treat `X-LiteSpeed-Cache-Control` as overriding standard HTTP caching rules. It is intentionally non-compliant with RFC 9111 — enable it only when the upstream is written for LiteSpeed-style cache semantics. Request-side directives such as `Cache-Control: no-cache` and `Pragma: no-cache` still affect cache lookup behavior normally (unless overridden by `ignore_request_cache_control`).

### Boolean `cache` form

| Form | Description | Default |
| --- | --- | --- |
| `cache` | Enables caching for the current HTTP host or `location` scope. | `false` |
| `cache true` | Explicitly enables caching for the current scope. | `false` |
| `cache false` | Disables caching for the current scope, which is useful for overriding an inherited `cache { ... }` block. | `false` |

## Behavior

### Cache eligibility

- Only `GET` and `HEAD` requests perform cache lookups.
- `HEAD` requests reuse cached `GET` representations and return only headers.
- Non-`GET` responses are not stored, but they may still trigger LSCache-compatible purge headers.
- Responses with `Vary: *` are never stored.
- Built-in error responses generated after the main HTTP pipeline are not currently stored.

### PURGE method cache invalidation

When the `purge_method` subdirective is enabled, Ferron accepts the `PURGE` HTTP method for cache invalidation. A `PURGE` request to a specific URL removes all cached entries (both public and private) matching that URL, causing subsequent requests to fetch fresh content.

**Security:**

PURGE requests must be either:

- Authenticated via HTTP basic authentication (the `basic_auth` directive), or
- Originating from an IP address matching the `purge_allowed_ips` list.

If neither condition is met, Ferron returns a **403 Forbidden** response. This ensures that cache purging is never accidentally left unsecured.

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

- Public responses containing `Set-Cookie` are not stored.
- Private responses are partitioned by client context. Ferron currently uses the client IP address, the authenticated username when available, and detected private cookies.
- If Ferron cannot determine a narrower private cookie set, it falls back to all request cookies for the private cache key.

### Stale-while-revalidate

When an upstream response includes the `stale-while-revalidate` directive in its `Cache-Control` header, Ferron extends the usable lifetime of the cached entry beyond its `max-age`. The behavior differs depending on concurrent request patterns:

- **Leader request** — the first request to encounter the expired entry becomes the leader and revalidates synchronously with the upstream. It receives a fresh response that replaces the cache entry.
- **Follower requests** — concurrent requests that arrive while the leader is revalidating are served the stale cached response immediately.

This ensures that one request still contacts the upstream for fresh content — no background tasks are involved — while other concurrent requests avoid waiting for revalidation.

> [!note]
> Ferron 3 does not have an internal route invocation mechanism, so background revalidation is not supported. `stale-while-revalidate` always involves a synchronous upstream request for the leader, and followers receive the stale response.

The `stale-while-revalidate` duration is taken from the origin's `Cache-Control` header. For example:

```http
Cache-Control: public, max-age=60, stale-while-revalidate=3600
```

This caches the response for 60 seconds, then allows stale serving for up to 3600 seconds after expiry.

When Ferron serves a stale response via this mechanism, the `Cache-Status` response header includes `detail=stale-while-revalidate`:

```http
Cache-Status: FerronCache; hit; detail=stale-while-revalidate,public; age=120
```

#### Interaction with `must-revalidate` and `proxy-revalidate`

Per RFC 9111, responses with `must-revalidate` or `proxy-revalidate` directives (or `s-maxage`, which implies `proxy-revalidate`) are never served stale, even within a `stale-while-revalidate` window. When either directive is present, Ferron treats the entry as strictly fresh-or-miss — it will either revalidate or return a miss rather than serving stale content.

### Stale-if-error

When an upstream response includes the `stale-if-error` directive in its `Cache-Control` header, Ferron can serve the stale cached response when revalidation fails with a 5xx server error:

```http
Cache-Control: public, max-age=300, stale-if-error=3600
```

This caches the response for 300 seconds, then allows stale serving on upstream errors for up to 3600 seconds after expiry.

How it works:

1. A request triggers revalidation (e.g., the cached entry has expired, or the client sent `Cache-Control: max-age=0`).
2. Ferron contacts the upstream, which returns a 5xx status code.
3. If a valid stale entry with `stale-if-error` exists, Ferron serves the stale response with a `Cache-Status` header containing `detail=stale-while-revalidate`.
4. If no stale entry exists or the `stale-if-error` window has elapsed, Ferron returns the 5xx error to the client.

This provides resilience against transient backend failures by falling back to previously cached content.

### LSCache-compatible response headers

When the cache module is enabled, Ferron understands the following response headers from upstream applications and origin handlers:

| Header | Description | Notes |
| --- | --- | --- |
| `X-LiteSpeed-Cache-Control` | Controls cache scope and TTL using LSCache-style directives such as `public`, `private`, `max-age`, `s-maxage`, `no-cache`, and `no-store`. | By default, standard HTTP caching rules still take precedence. Enable `litespeed_override_cache_control` to prefer this header instead. |
| `X-LiteSpeed-Vary` | Adds LSCache-style vary dimensions. | `cookie=<name>` is supported. `value=<name>` is not supported yet and causes Ferron to skip cache storage for that response. |
| `X-LiteSpeed-Tag` | Assigns tags to cached responses so they can be purged later. | On private responses, `public:` prefixes remain public tags. |
| `X-LiteSpeed-Purge` | Purges cached responses by tag, URL, or wildcard. | The `stale` marker currently falls back to an immediate hard purge. |
| `LSC-Cookie` | Adds cache-safe cookie replay metadata. | Ferron converts this header to `Set-Cookie` before sending the response. |
| `X-LiteSpeed-Cache` | Exposes cache hit, miss, or bypass status on outgoing responses. | Ferron sets this header itself (if enabled). Origin-provided values are ignored. |

> [!note]
> `X-LiteSpeed-Vary: value=...` is not supported yet because Ferron does not currently have a request-time equivalent of LiteSpeed's rewrite-rule vary environment values. The `ignore` directive affects only the stored representation — the live response sent to the client still includes those headers unless another module removes them.

## Observability

### Metrics

The cache module emits the following metrics:

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.cache.requests` | Counter | `ferron.cache.result`, `ferron.cache.scope` | Cache hits, misses, and bypasses |
| `ferron.cache.entries` | Gauge | — | Current number of cached entries |
| `ferron.cache.stores` | Counter | `ferron.cache.scope` | Responses stored in the cache |
| `ferron.cache.evictions` | Counter | `ferron.cache.reason` (`"expired"` or `"size"`) | Entries evicted from the cache |
| `ferron.cache.purges` | Counter | `ferron.cache.scope` | Entries purged through LSCache-compatible controls |

### Logs

- `DEBUG` — logged when Ferron skips cache storage because `X-LiteSpeed-Vary: value=...` is not supported yet.
- `DEBUG` — logged when Ferron skips cache storage because the response body exceeds `cache.max_response_size`.
- `DEBUG` — logged when Ferron performs a purge through `X-LiteSpeed-Purge`.
- `DEBUG` — logged when Ferron performs a purge through `PURGE` HTTP method.
- `DEBUG` — logged when Ferron receives an LSCache `stale` purge marker and falls back to a hard purge.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Skipping cache store because response body exceeded maximum size | DEBUG | - |
| Skipping cache store because X-LiteSpeed-Vary is not supported yet | DEBUG | - |
| Cache purged via LSCache controls | DEBUG | `cache.purged.count` (purged cache entries) |
| Cache purged via PURGE method | DEBUG | `cache.purged.count` (purged cache entries) |
| LSCache stale purge marker ignored | DEBUG | - |

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

- **`litespeed_override_cache_control`** — This makes LiteSpeed cache headers override standard HTTP cache policy. Enable only for applications that require LiteSpeed-compatible semantics.
- **`ignore_request_cache_control`** — When enabled, request-based cache control (e.g., `Cache-Control`) is ignored in favor of the configured cache policy.
- **`purge_method` without access control** — Cache purging enabled without `purge_allowed_ips` or `basic_auth` in the same scope allows unauthenticated cache invalidation.
- **`purge_allowed_ips` with wildcard** — Allowing every source address for cache purging should be restricted to trusted operators or internal networks.
