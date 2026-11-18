---
title: Security headers
description: "Add, remove, and replace response headers for browser hardening, CORS, and fingerprint cleanup."
---

Ferron can add, remove, and replace response headers. Use it for baseline browser hardening, CORS, and hiding framework or server details.

## Baseline hardening

```ferron
example.com {
    root /var/www/html

    header X-Content-Type-Options "nosniff"
    header X-Frame-Options "DENY"
    header Referrer-Policy "strict-origin-when-cross-origin"
    header Permissions-Policy "geolocation=(), microphone=(), camera=()"
    header Content-Security-Policy "default-src 'self'; object-src 'none'; frame-ancestors 'none'"
    header Strict-Transport-Security "max-age=31536000; includeSubDomains"
}
```

> [!warning]
> Add `Strict-Transport-Security` only to HTTPS hosts that will stay on HTTPS.

> [!tip]
> A Content Security Policy is specific to the application. Start with a small policy and add sources as the application needs them.

## Remove or replace headers

```ferron
app.example.com {
    proxy http://127.0.0.1:3000

    header -X-Powered-By
    header Server "Ferron"
}
```

The `header` directive has three forms:

- `header +Name "value"` adds a header. Duplicates are allowed.
- `header -Name` removes all instances of the header.
- `header Name "value"` replaces the header value.

## Per-path policies

```ferron
example.com {
    root /var/www/html

    location /admin {
        header Cache-Control "no-store"
        header X-Frame-Options "DENY"
    }
}
```

## CORS

For an API with known clients, list the origins:

```ferron
api.example.com {
    proxy http://localhost:3000

    cors {
        origins https://app.example.com https://admin.example.com
        methods GET POST PUT DELETE OPTIONS
        headers Content-Type Authorization X-Request-ID
        credentials
        max_age 86400
        expose_headers X-Total-Count X-Page
    }
}
```

To allow all origins:

```ferron
api.example.com {
    proxy http://localhost:3000

    cors {
        origins "*"
        methods GET POST
        headers Content-Type Authorization
        max_age 3600
    }
}
```

> [!warning]
> Allowing all origins lets any site call the API. Restrict `origins` to trusted domains when possible.

## Interpolate values

Header values support `{{...}}` interpolation:

```ferron
example.com {
    root /var/www/html

    header +X-Client-IP "{{remote.ip}}"
    header X-Powered-By "Ferron"
}
```

Available variables include `{{remote.ip}}`, `{{server.ip}}`, `{{request.host}}`, and `{{env.NAME}}`. For the complete reference, see [Conditionals and variables](/docs/configuration/fundamentals/conditionals).
