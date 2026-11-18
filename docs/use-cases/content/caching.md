---
title: HTTP caching
description: "Cache GET responses in memory, vary entries by request headers, and control stale content during backend failures."
---

Ferron stores complete `GET` response representations in memory and serves them directly to clients. `HEAD` requests reuse the cached `GET` metadata. This reduces backend load for HTML pages, API responses, and static assets.

## Enable caching

Set a `cache` block at the host level:

```ferron
example.com {
    cache {
        max_response_size 1048576
    }
}
```

This caches responses up to 1 MB. The default `max_response_size` is 2 MB. The global default `max_entries` is 1024.

The cache has these limits:

- It stores only `GET` and `HEAD` requests.
- It never stores responses with `Vary: *`.
- It does not store public responses that contain `Set-Cookie`.
- It clears on server restart.

> [!important]
> The in-memory cache does not survive a restart. For persistent caching, see [Disk persistence](/docs/configuration/content/cache#disk-persistence).

## Vary by request headers

Use `vary` to keep separate entries for request headers such as `Accept-Encoding` and `Accept-Language`:

```ferron
example.com {
    cache {
        vary Accept-Encoding Accept-Language
    }
}
```

Without `vary`, Ferron can store a response for one header combination and serve it to a client that sent different headers.

> [!tip]
> If you see unexpected cache misses, check that the `vary` headers match the responses. If the cache grows without limit, lower `max_response_size` or inspect large responses.

## Remove headers from cached responses

Use `ignore` to remove a header from cached responses while keeping it in live responses:

```ferron
example.com {
    cache {
        ignore Set-Cookie
    }
}
```

This is useful when the backend sends `Set-Cookie` on content that should still be cacheable.

## Disable caching for a path

Override the host cache in a `location` block:

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

## LiteSpeed Cache applications

If the upstream sends `X-LiteSpeed-Cache-Control`, enable LSCache override mode:

```ferron
example.com {
    cache {
        litespeed_override_cache_control
        emit_litespeed_headers
    }
}
```

Ferron then gives `X-LiteSpeed-Cache-Control` priority over `Cache-Control` and `Expires` when it decides whether to store a response and which TTL to use. `emit_litespeed_headers` adds `X-LiteSpeed-Cache` to the response.

## Authenticated responses

Ferron partitions private responses by client context. The partition uses the client IP, the authenticated username, and detected private cookies. Authenticated users receive their own cached responses.

```ferron
example.com {
    cache {
        max_response_size 1048576
    }

    location /dashboard {
        basic_auth
    }
}
```

## Cache proxied responses

Put `cache` in the same scope as `proxy`:

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

This stores backend responses and reduces load during traffic spikes.

## Stale-while-revalidate

A cached response can be served after `max-age` expires, while one request revalidates it. The backend sets the window with `Cache-Control`:

```http
Cache-Control: public, max-age=10, stale-while-revalidate=300
```

With this header:

1. Ferron stores the response for 10 seconds.
2. After 10 seconds, the first request revalidates with the backend.
3. Concurrent requests receive the stale response until revalidation finishes.

To disable the behavior:

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
> Ferron 3 revalidates synchronously. One request acts as the leader and contacts the upstream. Other concurrent requests receive the stale response. Ferron 3 has no background revalidation.

## Stale-if-error

Stale-if-error serves a stale cached response when revalidation returns a 5xx error. The backend sets the window:

```http
Cache-Control: public, max-age=60, stale-if-error=3600
```

Ferron then serves the stale response instead of the error. To disable the behavior:

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

A cache hit does not reach the rate limiter or the backend. Put both directives in the same scope:

```ferron
example.com {
    location /api {
        rate_limit {
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

## Purge propagation across instances

When several Ferron instances run behind a load balancer, a local purge does not invalidate the other instances. `purge_propagation` sends purge events to an external control-plane service. That service broadcasts the purge to the other edge instances.

```ferron
example.com {
    proxy http://backend:3000
    cache {
        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url http://control-plane:9090/cache/purge
            shared_secret edge-to-plane-secret
            node_id edge-1
        }
    }
}
```

The flow is:

1. A `PURGE` request or an `X-LiteSpeed-Purge` header purges the local cache on `edge-1`.
2. `edge-1` sends a `POST` to the control plane with the purged path and `origin: "edge-1"`.
3. The control plane sends `PURGE` requests to `edge-2` and `edge-3`, but not to `edge-1`.
4. Each edge purges its local cache and returns `200 OK`.

Loop prevention:

- An edge that receives `X-Purge-Source: propagation` purges locally and does not propagate again.
- The control plane excludes the origin node from its broadcast list.

The control-plane webhook looks like this:

```http
POST /cache/purge HTTP/1.1
Host: control-plane:9090
Content-Type: application/json
X-Purge-Secret: edge-to-plane-secret

{
  "path": "/blog/post-123",
  "origin": "edge-1"
}
```

The control plane must authenticate the request with `X-Purge-Secret` and send `PURGE` requests to all registered edges except the origin. Ferron does not include a control plane. You can implement one with any HTTP framework.

## See also

- [Configuration: cache](/docs/configuration/content/cache)
- [Edge caching](/docs/use-cases/content/edge-caching)
- [PHP edge caching](/docs/use-cases/content/php-edge-cache)
