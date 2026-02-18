---
title: CORS headers
description: "Configure CORS in Ferron with response headers, path scoping, and optional preflight handling."
---

Ferron does not use dedicated CORS directives. Instead, you configure CORS with standard response header directives such as `header` and `header_replace`.

## Public API (allow all origins)

Use this for public, unauthenticated APIs:

```kdl
// Replace "api.example.com" with your domain name.
api.example.com {
    location "/api" {
        proxy "http://127.0.0.1:3000/"

        header "Access-Control-Allow-Origin" "*"
        header "Access-Control-Allow-Methods" "GET, POST, PUT, PATCH, DELETE, OPTIONS"
        header "Access-Control-Allow-Headers" "Authorization, Content-Type"
        header "Access-Control-Max-Age" "86400"
    }
}
```

## Restricted origins (single frontend)

If your API is used only by one frontend origin, set that exact origin:

```kdl
// Replace hostnames and backend URL with your values.
api.example.com {
    location "/api" {
        proxy "http://127.0.0.1:3000/"

        header "Access-Control-Allow-Origin" "https://app.example.com"
        header "Access-Control-Allow-Methods" "GET, POST, PUT, PATCH, DELETE, OPTIONS"
        header "Access-Control-Allow-Headers" "Authorization, Content-Type"
        header "Access-Control-Allow-Credentials" "true"
        header "Vary" "Origin"
    }
}
```

When `Access-Control-Allow-Credentials` is `true`, do not use `*` for `Access-Control-Allow-Origin`.

## Preflight (`OPTIONS`) responses in Ferron

If your backend already handles `OPTIONS`, just keep the CORS headers and proxy as normal.

If you want Ferron to handle preflight requests directly:

```kdl
api.example.com {
    condition "IS_OPTIONS" {
        is_equal "{method}" "OPTIONS"
    }

    location "/api" {
        header "Access-Control-Allow-Origin" "https://app.example.com"
        header "Access-Control-Allow-Methods" "GET, POST, PUT, PATCH, DELETE, OPTIONS"
        header "Access-Control-Allow-Headers" "Authorization, Content-Type"
        header "Access-Control-Max-Age" "86400"
        header "Vary" "Origin"

        if "IS_OPTIONS" {
            status 204 body=""
        }

        if_not "IS_OPTIONS" {
            proxy "http://127.0.0.1:3000/"
        }
    }
}
```

## Reverse proxy + upstream CORS headers

If your upstream already sets CORS headers and you want Ferron to be the single source of truth, replace or remove upstream values:

```kdl
api.example.com {
    proxy "http://127.0.0.1:3000/"

    header_replace "Access-Control-Allow-Origin" "https://app.example.com"
    header_replace "Access-Control-Allow-Methods" "GET, POST, PUT, PATCH, DELETE, OPTIONS"
    header_replace "Access-Control-Allow-Headers" "Authorization, Content-Type"
    header "Vary" "Origin"
    
    // Or delete headers
    //header_remove "Access-Control-Allow-Origin"
    //header_remove "Access-Control-Allow-Methods"
    //header_remove "Access-Control-Allow-Headers"
}
```

## Notes and troubleshooting

- Apply CORS headers only where needed (for example under `/api`) to avoid exposing unnecessary cross-origin access on unrelated routes.
- If browser preflight fails, verify `OPTIONS` is handled either by Ferron (`status 204`) or by your backend.
- If you use custom request headers from frontend code, include them in `Access-Control-Allow-Headers`.
- For directive details, see [Configuration: routing & URL processing](/docs/configuration/routing-url-processing), [Configuration: conditionals](/docs/configuration/conditionals), and [Configuration: security & TLS](/docs/configuration/security-tls).
