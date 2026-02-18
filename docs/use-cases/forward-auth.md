---
title: Forward auth
description: "Protect Ferron routes with external authentication/SSO services using forwarded authentication."
---

Forwarded authentication lets Ferron ask an external auth service whether a request is allowed before proxying to your app.

This is useful for SSO gateways, central auth policies across many services, and protecting internal tools without adding auth logic to each backend.

## Protect an app behind an auth service

```kdl
// Replace hostnames and backend URLs with your values.
app.example.com {
    // Send auth checks to your auth service.
    auth_to "https://auth.example.com/validate"

    // Forward selected original request headers to auth service.
    auth_to_copy "Authorization" "Cookie" "X-Forwarded-For"

    // Main application backend.
    proxy "http://127.0.0.1:3000/"
}
```

The `fauth` module is modeled after ForwardAuth middleware behavior: if auth returns 2xx, the request proceeds; otherwise, the auth response is returned.

## Protect only selected paths

Apply forward auth only where needed:

```kdl
app.example.com {
    // Public content (no forward auth).
    location "/public" {
        root "/var/www/public"
    }

    // Protected app area.
    location "/app" {
        auth_to "https://auth.example.com/validate"
        auth_to_copy "Authorization" "Cookie"
        proxy "http://127.0.0.1:3000/"
    }
}
```

## Connection tuning for auth backends

```kdl
globals {
    // Limit concurrent outbound connections to auth services.
    auth_to_concurrent_conns 4096
}
```

## Notes and troubleshooting

- Keep `auth_to_no_verification #false` unless you are in a controlled test environment.
- If you see redirect loops, verify your auth service's allowlist/bypass rules for login or callback paths.
- Start by forwarding only required headers via `auth_to_copy` (commonly `Authorization` and `Cookie`).
- For directive details, see [Configuration: reverse proxying](/docs/configuration/reverse-proxying) and `fauth` behavior notes in [Server modules](/docs/reference/modules).
