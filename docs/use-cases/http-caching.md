---
title: HTTP caching
description: "Use Ferron's HTTP response cache to improve performance for frequently accessed content with minimal backend load."
---

Ferron's HTTP response cache stores complete `GET` response representations in memory and serves them directly to clients, reducing backend load and improving response times. This is especially useful for frequently accessed content like HTML pages, API responses, and static assets.

## Basic HTTP caching

To enable caching for an entire host, use the `cache` directive at the HTTP host level:

```ferron
example.com {
    cache {
        max_response_size 1048576
    }
}
```

This configuration caches responses up to 1MB in size. The default `max_response_size` is 2MB, and the global default `max_entries` is 1024.

## Caching with Vary headers

The `vary` directive ensures responses are cached separately based on request headers. This is crucial for content that varies by `Accept-Encoding`, `Accept-Language`, or other headers:

```ferron
example.com {
    cache {
        vary Accept-Encoding Accept-Language
    }
}
```

Without `vary`, responses with different headers would be incorrectly cached together, potentially serving the wrong content to clients.

## Excluding sensitive responses from cache

Use the `ignore` directive to remove headers from cached responses while keeping them in live responses. This is useful for removing `Set-Cookie` from cached content:

```ferron
example.com {
    cache {
        ignore Set-Cookie
    }
}
```

## Disabling cache for specific paths

Override inherited caching settings for specific paths using `location` blocks:

```ferron
example.com {
    cache {
        max_response_size 1048576
    }

    location /admin {
        cache false
    }

    location /api/private {
        cache false
    }
}
```

This disables caching for `/admin` and `/api/private` paths while keeping caching enabled for the rest of the host.

## LSCache-compatible applications

If your upstream application uses LiteSpeed Cache-style headers, enable override mode:

```ferron
example.com {
    cache {
        max_response_size 1048576
        litespeed_override_cache_control true
    }
}
```

This tells Ferron to prioritize `X-LiteSpeed-Cache-Control` headers over standard `Cache-Control` and `Expires` headers when deciding whether to store responses and what TTL to use.

## Caching with authentication

Private responses are partitioned by client context using the client IP, authenticated username, and detected private cookies. This means authenticated users get personalized cached responses:

```ferron
example.com {
    cache {
        max_response_size 1048576
    }
}

location /dashboard {
    basic_auth
    cache {
        max_response_size 1048576
    }
}
```

Each authenticated user will have their own cached dashboard pages based on their credentials.

## Caching with reverse proxying

Combine reverse proxying with caching to cache backend responses:

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
        cache {
            max_response_size 524288
            vary Accept-Encoding
        }
    }
}
```

This caches API responses from the backend, reducing load during traffic spikes.

## Caching with rate limiting

Use caching alongside rate limiting to protect backend services:

```ferron
example.com {
    location /api {
        ratelimit 100 "60s"
        proxy http://localhost:3000
        cache {
            max_response_size 524288
        }
    }
}
```

Cached responses bypass the rate limiter and backend entirely, providing maximum protection.

## Observability

Monitor your cache performance with these metrics:

- `ferron.cache.requests` — cache hits, misses, and bypasses
- `ferron.cache.entries` — current number of cached entries
- `ferron.cache.stores` — responses stored in the cache
- `ferron.cache.evictions` — entries evicted from the cache
- `ferron.cache.purges` — entries purged through LSCache-compatible controls

Enable verbose logging to see detailed cache operations:

```ferron
{
    console_log
}
```

## Notes and troubleshooting

- Only `GET` and `HEAD` requests are cached. `HEAD` requests reuse cached `GET` representations.
- Responses with `Vary: *` are never stored in the cache.
- Public responses containing `Set-Cookie` are not stored.
- The cache is in-memory and will be cleared on server restart. For persistent caching, consider using an external cache like Redis.
- If you see unexpected cache misses, check that `vary` headers are configured correctly for your use case.
- For static file cache headers like `file_cache_control` and `etag`, see [Static file serving](/docs/v3/use-cases/static-file-serving).
- For the full HTTP response cache module configuration, see [HTTP cache](/docs/v3/configuration/http-cache).
- If cache size is growing unbounded, check for frequently accessed large responses and consider reducing `max_response_size`.
- For private responses, the cache is partitioned by client context. If you want truly shared private caching, use public `Cache-Control: max-age` headers instead.
