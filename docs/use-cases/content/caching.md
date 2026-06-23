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

> [!important]
>
> - Only `GET` and `HEAD` requests are cached. `HEAD` requests reuse cached `GET` representations.
> - Responses with `Vary: *` are never stored.
> - Public responses containing `Set-Cookie` are not stored.
> - The cache is in-memory and will be cleared on server restart — for persistent caching, consider using an external cache like Redis.

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

> [!tip]
> If you see unexpected cache misses, check that `vary` headers are configured correctly for your use case. If cache size is growing unbounded, check for frequently accessed large responses and consider reducing `max_response_size`.

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
        litespeed_override_cache_control

        # Also, emit X-LiteSpeed-Cache response header
        emit_litespeed_headers
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

    location /dashboard {
        basic_auth
        cache {
            max_response_size 1048576
        }
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

## Stale-while-revalidate

Stale-while-revalidate allows a cached response to be served after its `max-age` has expired, as long as it falls within the `stale-while-revalidate` window set by the origin. This avoids latency spikes when the cache entry expires and concurrent requests arrive:

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
        cache {
            max_response_size 524288
        }
    }
}
```

The backend controls the stale window with `Cache-Control`:

```http
Cache-Control: public, max-age=10, stale-while-revalidate=300
```

With this configuration:

- Responses are cached for 10 seconds.
- After 10 seconds, the first request revalidates with the backend and gets fresh content.
- Concurrent requests during revalidation receive the stale response immediately.

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
        cache {
            enable_stale_while_revalidate false
        }
    }
}
```

> [!note]
> Ferron 3 does not support background revalidation — stale-while-revalidate always involves a synchronous upstream request for one request (the leader). Other concurrent requests see the stale content. This is a known limitation stemming from the absence of internal route invocation in Ferron 3.

## Stale-if-error

Stale-if-error provides resilience against transient backend failures by falling back to stale cached content when revalidation encounters a 5xx error:

```http
Cache-Control: public, max-age=60, stale-if-error=3600
```

If the backend returns a 5xx error during revalidation, Ferron serves the stale cached response instead of forwarding the error to the client. This keeps your application running during brief backend outages.

```ferron
example.com {
    location /api {
        proxy http://localhost:3000
        cache {
            enable_stale_if_error false
        }
    }
}
```

## Caching with rate limiting

Use caching alongside rate limiting to protect backend services:

```ferron
example.com {
    location /api {
        ratelimit {
            rate 100
            burst 50
        }
        proxy http://localhost:3000
        cache {
            max_response_size 524288
        }
    }
}
```

Cached responses bypass the rate limiter and backend entirely, providing maximum protection.
