---
title: "Configuration: HTTP cache"
description: "In-memory HTTP response caching with RFC 9111 behavior, an optional LSCache override mode, and cache observability."
---

This page documents the `cache` directive for configuring Ferron's in-memory HTTP response cache. The cache stores complete `GET` response representations, serves `HEAD` from cached `GET` metadata, follows standard HTTP caching semantics by default, and understands a subset of LiteSpeed Cache response headers for LSCache-aware applications.

The cache applies to final HTTP responses produced by static file serving, reverse proxying, and other response stages.

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

### HTTP host `cache` block

Use the HTTP host `cache { ... }` block to enable caching and tune how responses are stored for that host or matching `location`.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `max_response_size` | `<int>` | This directive specifies the maximum response body size, in bytes, that can be buffered and stored in the cache. Responses larger than this limit are still served, but they are not stored. | `2097152` |
| `litespeed_override_cache_control` | `[<bool>]` | This directive specifies whether `X-LiteSpeed-Cache-Control` overrides standard response caching headers such as `Cache-Control` and `Expires` when Ferron decides whether to store a response and what TTL to use. This mode is intentionally non-standard and is intended only for applications that expect LiteSpeed-style cache semantics. | `false` |
| `vary` | `<string> [<string> ...]` | This directive specifies additional request headers that are added to the cache key, alongside any standard `Vary` response headers returned by the origin. This directive can be specified multiple times. | none |
| `ignore` | `<string> [<string> ...]` | This directive specifies response headers that are removed from the stored cache representation while leaving the live response unchanged. This directive can be specified multiple times. | none |

**Configuration example:**

```ferron
example.com {
    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        vary Accept-Encoding Accept-Language
        ignore Set-Cookie
    }
}
```

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

### Public and private cache behavior

- Public responses containing `Set-Cookie` are not stored.
- Private responses are partitioned by client context. Ferron currently uses the client IP address, the authenticated username when available, and detected private cookies.
- If Ferron cannot determine a narrower private cookie set, it falls back to all request cookies for the private cache key.

### LSCache-compatible response headers

When the cache module is enabled, Ferron understands the following response headers from upstream applications and origin handlers:

| Header | Description | Notes |
| --- | --- | --- |
| `X-LiteSpeed-Cache-Control` | Controls cache scope and TTL using LSCache-style directives such as `public`, `private`, `max-age`, `s-maxage`, `no-cache`, and `no-store`. | By default, standard HTTP caching rules still take precedence. Enable `litespeed_override_cache_control` to prefer this header instead. |
| `X-LiteSpeed-Vary` | Adds LSCache-style vary dimensions. | `cookie=<name>` is supported. `value=<name>` is not supported yet and causes Ferron to skip cache storage for that response. |
| `X-LiteSpeed-Tag` | Assigns tags to cached responses so they can be purged later. | On private responses, `public:` prefixes remain public tags. |
| `X-LiteSpeed-Purge` | Purges cached responses by tag, URL, or wildcard. | The `stale` marker currently falls back to an immediate hard purge. |
| `LSC-Cookie` | Adds cache-safe cookie replay metadata. | Ferron converts this header to `Set-Cookie` before sending the response. |
| `X-LiteSpeed-Cache` | Exposes cache hit, miss, or bypass status on outgoing responses. | Ferron sets this header itself. Origin-provided values are ignored. |

## Observability

### Metrics

The cache module emits the following metrics:

- `ferron.cache.requests` (Counter) — cache hits, misses, and bypasses.
  - Attributes: `ferron.cache.result`, `ferron.cache.scope`
- `ferron.cache.entries` (Gauge) — current number of cached entries.
- `ferron.cache.stores` (Counter) — responses stored in the cache.
  - Attributes: `ferron.cache.scope`
- `ferron.cache.evictions` (Counter) — entries evicted from the cache.
  - Attributes: `ferron.cache.reason` (`"expired"` or `"size"`)
- `ferron.cache.purges` (Counter) — entries purged through LSCache-compatible controls.
  - Attributes: `ferron.cache.scope`

### Logs

- `DEBUG` — logged when Ferron skips cache storage because `X-LiteSpeed-Vary: value=...` is not supported yet.
- `DEBUG` — logged when Ferron skips cache storage because the response body exceeds `cache.max_response_size`.
- `DEBUG` — logged when Ferron performs a purge through `X-LiteSpeed-Purge`.
- `DEBUG` — logged when Ferron receives an LSCache `stale` purge marker and falls back to a hard purge.

## Notes and troubleshooting

- By default, Ferron treats LSCache response headers as additional cache controls, not as a way to weaken standard HTTP caching rules. `Cache-Control`, `Authorization`, `Vary`, and `Set-Cookie` constraints still apply.
- `litespeed_override_cache_control` changes only response-side store policy and TTL selection. Request-side directives such as `Cache-Control: no-cache` and `Pragma: no-cache` still affect cache lookup behavior normally.
- `litespeed_override_cache_control` is intentionally non-compliant with RFC 9111 behavior when `X-LiteSpeed-Cache-Control` is present. Enable it only when the upstream application is written for LiteSpeed-style server cache semantics.
- `X-LiteSpeed-Vary: value=...` is not supported yet because Ferron does not currently have a request-time equivalent of LiteSpeed's rewrite-rule vary environment values.
- `ignore` affects only the stored representation. The live response sent to the client still includes those headers unless another module removes them.
- Global `cache { ... }` blocks are only for shared cache sizing. They do not enable caching for HTTP hosts by themselves.
- For static file cache headers such as `file_cache_control` and `etag`, see [Static file serving](/docs/v3/configuration/static-content.md).
- For response header mutation and CORS handling, see [HTTP headers and CORS](/docs/v3/configuration/http-headers.md).
- For reverse proxy configuration, see [Reverse proxying](/docs/v3/configuration/reverse-proxying.md).
