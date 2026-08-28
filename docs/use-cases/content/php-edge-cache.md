---
title: PHP edge caching (LSCache)
description: "Use Ferron as an edge caching proxy for Apache PHP hosting with LSCache plugin support."
---

Serving PHP through Apache proves reliable. `.htaccess` files, `mod_rewrite`, `mod_php`, and decades of hosting infrastructure all rely on it. Ferron can sit in front of this setup as an edge caching proxy. It adds high-performance HTTP caching, TLS termination, rate limiting, and DDoS protection. You do not need to change your Apache configuration.

```text
Client -> Ferron (edge cache) -> Apache (PHP origin) -> PHP-FPM / mod_php
```

Ferron works with LSCache-compatible PHP plugins such as LiteSpeed Cache for WordPress, Joomla, or OpenCart. Ferron respects `X-LiteSpeed-Cache-Control` headers from the PHP application. This gives you cache control directly from your app. You do not need server-level tuning.

## Basic edge cache configuration

The simplest setup proxies all requests to Apache on `localhost:8080` and enables caching with LSCache override mode:

```ferron
example.com {
    proxy http://127.0.0.1:8080

    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
        vary Accept-Encoding
    }
}
```

This configuration:

- Proxies every request to Apache, which handles PHP execution via PHP-FPM or `mod_php`.
- Enables LSCache semantics (`X-LiteSpeed-Cache-Control` from the PHP app controls cache TTL and scope).
- Emits `X-LiteSpeed-Cache` response headers (hit/miss/bypass) for debugging and plugin visibility.
- Caches responses separately by `Accept-Encoding` so the server stores compressed and uncompressed variants independently.

> [!tip]
> Before adding caching, verify that the proxy path works: `ferron validate -c ferron.conf`. Then check that Apache is reachable. Add the caching layer once you confirm the reverse proxy works.

## WordPress with the LSCache plugin

WordPress sites using the [LiteSpeed Cache plugin](https://wordpress.org/plugins/litespeed-cache/) are the most common use of this pattern. The plugin emits `X-LiteSpeed-Cache-Control` headers on pages it considers cacheable. With `litespeed_override_cache_control` enabled, Ferron respects those headers and serves cached copies to later visitors. This reduces Apache and PHP-FPM load.

```ferron
example.com {
    proxy http://127.0.0.1:8080

    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
        vary Accept-Encoding
        ignore Set-Cookie
    }
}
```

The `ignore Set-Cookie` directive strips `Set-Cookie` headers from the cached representation. It keeps them in the live response. This is essential for maintaining cacheability alongside session cookies.

## Cache exclusion for admin paths

You must never cache admin areas, login pages, and checkout flows. Use conditional blocks to disable caching for specific paths:

```ferron
example.com {
    proxy http://127.0.0.1:8080

    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
    }

    match EXCLUDE_WORDPRESS_CACHE {
        request.uri.path !~ r"^/(?:wp-(?:admin|login\.php)|wc-api)\b"
    }

    if EXCLUDE_WORDPRESS_CACHE {
        cache false
    }
}
```

> [!note]
> The LSCache WordPress plugin already sends `no-cache` directives on admin and login pages by default. The explicit `location` blocks serve as a safety net in case the plugin does not send cache-control headers.

## Caching static files from Apache

Apache still serves static assets (theme CSS/JS, uploaded images, fonts) through the same `mod_php`/`.htaccess` stack as your PHP pages, even though nothing dynamic happens for those requests. Every hit still pays for a full Apache round trip. You do not need a `location` block or a separate Ferron directive to fix this: the `cache` block from [Basic edge cache configuration](#basic-edge-cache-configuration) already caches these responses. Ferron's cache policy reads the standard `Cache-Control` header on any proxied response. `litespeed_override_cache_control` only takes effect when the response also carries `X-LiteSpeed-Cache-Control`, and plain static files served directly by Apache (as opposed to pages rendered by the LSCache plugin) never send that header. So standard `Cache-Control` governs static assets, and LSCache semantics keep governing PHP pages, from the same host block.

The only thing missing is telling Apache to send `Cache-Control` on static files. Without it, Ferron still caches a bare `200 OK` for a short time under its default heuristic (5 minutes), but you get no control over the TTL and no `immutable` hint. Add explicit headers with `mod_headers` and `mod_expires`:

```apache
<IfModule mod_headers.c>
    <FilesMatch "\.(?:css|js|mjs|png|jpe?g|gif|svg|webp|ico|woff2?|ttf|eot)$">
        Header set Cache-Control "public, max-age=2592000, immutable"
    </FilesMatch>
</IfModule>
```

Use `immutable` only for assets with a hashed or versioned filename (for example `app.a1b2c3.js`), since it tells clients and Ferron never to revalidate the entry for the lifetime of `max-age`. For unversioned assets that change in place, drop `immutable` and pick a shorter `max-age`, or plan to `PURGE` the entry after a deploy (see [PURGE method cache invalidation](/docs/v3/configuration/content/cache#purge-method-cache-invalidation)).

With that header in place, the flow looks like this:

1. The first request for `/theme/style.css` reaches Ferron, which has no cached entry and proxies to Apache.
2. Apache serves the file with `Cache-Control: public, max-age=2592000, immutable`.
3. Ferron stores the response body and headers, keyed by the request path (and by `Accept-Encoding`, thanks to `vary Accept-Encoding` in the `cache` block).
4. Every later request for that asset, from any client, is served directly from Ferron's in-memory cache. Apache never sees it again until the entry expires or is purged.

> [!tip]
> Static assets are usually far larger than typical PHP HTML responses. If images or bundled JS exceed the default `max_response_size` (2 MB), Ferron proxies them correctly but does not cache them. Raise `max_response_size` in the `cache` block, or give static assets their own [named zone](/docs/v3/configuration/content/cache#cache-zones) so a few large files do not crowd out cached HTML pages.

> [!note]
> This pattern only removes the Apache round trip for GET/HEAD requests that qualify for caching. Requests with `Authorization` headers, or responses that carry `Set-Cookie`, still bypass the cache unless the response also authorizes shared caching (`public` or `s-maxage`). See [Public and private cache behavior](/docs/v3/configuration/content/cache#public-and-private-cache-behavior).

## Cache purging from PHP

When content changes (a new blog post, updated product, or comment), you must invalidate the cache. LSCache plugins emit an `X-LiteSpeed-Purge` response header to invalidate tagged or URL-specific cache entries. Ferron processes these headers automatically when you enable caching. You do not need additional configuration.

## Forwarding client IP to Apache

When Ferron terminates client connections, Apache sees all traffic as coming from Ferron's IP address. Ferron forwards the original client IP automatically so PHP applications and Apache logs see real visitor addresses.

On the Apache side, enable `mod_remoteip` to trust Ferron's IP and use `X-Forwarded-For` for logging and access control:

```apache
# Enable in Apache config or an included snippet
RemoteIPHeader X-Forwarded-For
RemoteIPTrustedProxy 127.0.0.1
```

Replace `127.0.0.1` with Ferron's actual IP if running on a different host.

> [!tip]
> Without `mod_remoteip` (or equivalent), Apache logs and PHP applications show Ferron's IP address for every request. WordPress plugins that depend on visitor IP (geolocation, security, analytics) will not work correctly.

## Additional edge features

Because Ferron terminates client connections before they reach Apache, you can add edge-level features that apply to all traffic. These features apply to cached responses that never touch the backend:

- **TLS termination.** Offload HTTPS at Ferron with automatic (ACME) or manual certificates. See [Automatic TLS](/docs/v3/use-cases/security/automatic-tls) and [Manual TLS](/docs/v3/use-cases/security/manual-tls).
- **Rate limiting.** Protect Apache and PHP from traffic spikes and brute-force attacks. See [Rate limiting](/docs/v3/use-cases/security/rate-limiting).
- **Abuse protection.** Drop malicious requests before they reach the backend. See [Abuse protection](/docs/v3/use-cases/security/abuse-protection).
- **Security headers.** Add headers like `Strict-Transport-Security`, `Content-Security-Policy`, and `X-Frame-Options` at the edge. See [Security headers](/docs/v3/use-cases/security/security-headers).
- **Observability.** Log requests, monitor cache hits/misses, and track performance metrics. See [Logging & observability](/docs/v3/use-cases/operations/logging-observability).

> [!note]
> Ferron serves cached responses directly from its in-memory cache. Cached responses never reach Apache. This means edge-level features like rate limiting and security headers still apply. Backend-side logic (`.htaccess` rewrites, Apache access controls) does not run on cache hits.

## See also

- [HTTP caching](/docs/v3/use-cases/content/caching): general caching patterns and LSCache overview
- [PHP hosting](/docs/v3/use-cases/content/php): running PHP directly on Ferron without Apache
- [Reverse proxying](/docs/v3/use-cases/traffic/reverse-proxy): proxy configuration reference
- [Configuration: HTTP cache](/docs/v3/configuration/content/cache): full cache directive reference
