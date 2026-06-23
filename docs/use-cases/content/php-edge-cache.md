---
title: PHP edge caching (LSCache)
description: "Use Ferron as an edge caching proxy in front of Apache for PHP hosting, with LSCache plugin compatibility for WordPress and other PHP applications."
---

Serving PHP through Apache is a proven pattern — `.htaccess` files, `mod_rewrite`, `mod_php`, and decades of hosting infrastructure all rely on it. Ferron can sit in front of this setup as an edge caching proxy, adding high-performance HTTP caching, TLS termination, rate limiting, and DDoS protection without changing your Apache configuration.

```text
Client ──► Ferron (edge cache) ──► Apache (PHP origin) ──► PHP-FPM / mod_php
```

When paired with an LSCache-compatible PHP plugin (such as LiteSpeed Cache for WordPress, Joomla, or OpenCart), Ferron respects `X-LiteSpeed-Cache-Control` headers from the PHP application, giving you cache control directly from your app — no server-level tuning required.

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
- Enables LSCache semantics — `X-LiteSpeed-Cache-Control` from the PHP app controls cache TTL and scope.
- Emits `X-LiteSpeed-Cache` response headers (hit/miss/bypass) for debugging and plugin visibility.
- Caches responses separately by `Accept-Encoding` so compressed and uncompressed variants are stored independently.

> [!tip]
> Before adding caching, verify that the proxy path works: `ferron validate -c ferron.conf` and check Apache is reachable. The caching layer can be added once the reverse proxy is confirmed working.

## WordPress with the LSCache plugin

WordPress sites using the [LiteSpeed Cache plugin](https://wordpress.org/plugins/litespeed-cache/) are the most common deployment of this pattern. The plugin emits `X-LiteSpeed-Cache-Control` headers on pages it considers cacheable. With `litespeed_override_cache_control` enabled, Ferron respects those headers and serves cached copies to subsequent visitors, dramatically reducing Apache and PHP-FPM load.

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

The `ignore Set-Cookie` directive strips `Set-Cookie` headers from the cached representation while keeping them in the live response — essential for maintaining cacheability alongside session cookies.

## Cache exclusion for admin paths

Admin areas, login pages, and checkout flows must never be cached. Use conditional blocks to disable caching for specific paths:

```ferron
example.com {
    proxy http://127.0.0.1:8080

    cache {
        max_response_size 2097152
        litespeed_override_cache_control
        emit_litespeed_headers
    }

    match EXCLUDE_WORDPRESS_CACHE {
        request.uri.path !~ "^/(?:wp-(?:admin|login\.php)|wc-api)\b"
    }

    if EXCLUDE_WORDPRESS_CACHE {
        cache false
    }
}
```

> [!note]
> The LSCache WordPress plugin already sends `no-cache` directives on admin and login pages by default. The explicit `location` blocks are a safety net in case the plugin's cache-control headers are not present for any reason.

## Cache purging from PHP

When content changes — a new blog post, updated product, or comment — the cache must be invalidated. LSCache plugins emit an `X-LiteSpeed-Purge` response header to invalidate tagged or URL-specific cache entries. Ferron processes these headers automatically when caching is enabled — no additional configuration needed.

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
> Without `mod_remoteip` (or equivalent), Apache logs and PHP applications will show Ferron's IP address for every request. WordPress plugins that depend on visitor IP (geolocation, security, analytics) will not work correctly.

## Additional edge features

Because Ferron terminates client connections before they reach Apache, you can add edge-level features that apply to all traffic — including cached responses that never touch the backend:

- **TLS termination** — Offload HTTPS at Ferron with automatic (ACME) or manual certificates. See [Automatic TLS](/docs/v3/use-cases/security/automatic-tls) and [Manual TLS](/docs/v3/use-cases/security/manual-tls).
- **Rate limiting** — Protect Apache and PHP from traffic spikes and brute-force attacks. See [Rate limiting](/docs/v3/use-cases/security/rate-limiting).
- **Abuse protection** — Drop malicious requests before they reach the backend. See [Abuse protection](/docs/v3/use-cases/security/abuse-protection).
- **Security headers** — Add headers like `Strict-Transport-Security`, `Content-Security-Policy`, and `X-Frame-Options` at the edge. See [Security headers](/docs/v3/use-cases/security/security-headers).
- **Observability** — Log requests, monitor cache hits/misses, and track performance metrics. See [Logging & observability](/docs/v3/use-cases/operations/logging-observability).

> [!note]
> Cached responses are served directly from Ferron's in-memory cache — they never reach Apache. This means edge-level features like rate limiting and security headers still apply, but backend-side logic (`.htaccess` rewrites, Apache access controls) does not execute on cache hits.

## See also

- [HTTP caching](/docs/v3/use-cases/content/caching) — general caching patterns and LSCache overview
- [PHP hosting](/docs/v3/use-cases/content/php) — running PHP directly on Ferron without Apache
- [Reverse proxying](/docs/v3/use-cases/traffic/reverse-proxy) — proxy configuration reference
- [Configuration: HTTP cache](/docs/v3/configuration/content/cache) — full cache directive reference
