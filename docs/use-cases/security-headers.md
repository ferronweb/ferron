---
title: Security headers
description: "Harden Ferron responses with common security headers and clean up/replace undesired response headers."
---

Ferron can add, remove, and replace response headers. This is useful for baseline browser hardening and for hiding framework/server fingerprints.

## Baseline hardening

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    header "X-Content-Type-Options" "nosniff"
    header "X-Frame-Options" "DENY"
    header "Referrer-Policy" "strict-origin-when-cross-origin"
    header "Permissions-Policy" "geolocation=(), microphone=(), camera=()"
    header "Content-Security-Policy" "default-src 'self'; object-src 'none'; frame-ancestors 'none'"
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"
}
```

## Remove or replace unwanted headers

```kdl
// Replace "app.example.com" with your domain name.
app.example.com {
    proxy "http://127.0.0.1:3000/"

    // Remove or normalize headers from upstream responses.
    header_remove "X-Powered-By"
    header_replace "Server" "Ferron"
}
```

## Per-path policies

```kdl
example.com {
    root "/var/www/html"

    location "/admin" {
        header "Cache-Control" "no-store"
        header "X-Frame-Options" "DENY"
    }
}
```

## Notes and troubleshooting

- Keep `Strict-Transport-Security` only on HTTPS hosts you intend to keep on HTTPS permanently.
- Treat `Content-Security-Policy` as application-specific; start simple, then tighten based on real asset/script needs.
- If a header appears multiple times, use `header_replace` when you need one canonical value.
- For directive details, see [Configuration: routing & URL processing](/docs/configuration/routing-url-processing).
