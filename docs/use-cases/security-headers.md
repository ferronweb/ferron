---
title: Security headers
description: "Harden Ferron responses with common security headers, CORS configuration, and header cleanup."
---

Ferron can add, remove, and replace response headers. This is useful for baseline browser hardening, CORS handling, and hiding framework/server fingerprints.

## Baseline hardening

```ferron
example.com {
    root /var/www/html

    header "X-Content-Type-Options" "nosniff"
    header "X-Frame-Options" "DENY"
    header "Referrer-Policy" "strict-origin-when-cross-origin"
    header "Permissions-Policy" "geolocation=(), microphone=(), camera=()"
    header "Content-Security-Policy" "default-src 'self'; object-src 'none'; frame-ancestors 'none'"
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"
}
```

## Remove or replace unwanted headers

```ferron
app.example.com {
    proxy http://127.0.0.1:3000

    # Remove or normalize headers from upstream responses.
    header -X-Powered-By
    header Server "Ferron"
}
```

The `header` directive supports three forms:
- `header +Name "value"` — **add** header (appends, allows duplicates)
- `header -Name` — **remove** all instances of the header
- `header Name "value"` — **replace** header (removes existing, sets new value)

## Per-path policies

```ferron
example.com {
    root /var/www/html

    location /admin {
        header "Cache-Control" "no-store"
        header "X-Frame-Options" "DENY"
    }
}
```

## CORS configuration

For APIs that need cross-origin access, use the `cors` directive:

```ferron
api.example.com {
    proxy http://localhost:3000

    cors {
        origins "https://app.example.com" "https://admin.example.com"
        methods GET POST PUT DELETE OPTIONS
        headers "Content-Type" "Authorization" "X-Request-ID"
        credentials true
        max_age 86400
        expose_headers "X-Total-Count" "X-Page"
    }
}
```

To allow all origins (use with caution for public APIs):

```ferron
api.example.com {
    proxy http://localhost:3000

    cors {
        origins "*"
        methods GET POST
        headers "Content-Type" "Authorization"
        max_age 3600
    }
}
```

## Header interpolation

Header values support interpolation with `{{...}}` syntax:

```ferron
example.com {
    root /var/www/html

    header +X-Client-IP "{{remote_address}}"
    header X-Powered-By "Ferron"
}
```

Available variables include `{{remote_address}}`, `{{local_address}}`, `{{hostname}}`, and `{{env.NAME}}` for environment variables.

## Notes and troubleshooting

- Keep `Strict-Transport-Security` only on HTTPS hosts you intend to keep on HTTPS permanently.
- Treat `Content-Security-Policy` as application-specific; start simple, then tighten based on real asset/script needs.
- If a header appears multiple times, use `header -Name` to remove all instances, then `header Name "value"` to set a single value.
- If CORS headers are not appearing, verify that `origins` is configured — CORS is disabled by default if `origins` is empty.
- For directive details, see [Configuration: HTTP headers and CORS](/docs/v3/configuration/http-headers).
