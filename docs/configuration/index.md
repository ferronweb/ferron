---
title: "Configuration: reference map"
description: "Index of all configuration directives organized by topic and scope."
---

This reference describes the configuration surface for Ferron 3. Directives are organized by topic and scope so you can quickly find what you need.

## Reference map

- [Syntax and file structure](/docs/v3/syntax)
- [Conditionals and variables](/docs/v3/conditionals)
- [Core directives](/docs/v3/core-directives)
- [Admin API](/docs/v3/core-directives#admin-api)
- [HTTP host directives](/docs/v3/http-host)
- [Routing and URL processing](/docs/v3/routing-url-processing)
- [HTTP response control](/docs/v3/http-response)
- [URL rewriting](/docs/v3/http-rewrite)
- [Reverse proxy](/docs/v3/reverse-proxying)
- [Forward proxy](/docs/v3/http-fproxy)
- [HTTP basic authentication](/docs/v3/http-basicauth)
- [Static file serving](/docs/v3/static-content)
- [HTTP headers and CORS](/docs/v3/http-headers)
- [Rate limiting](/docs/v3/http-ratelimit)
- [Security and TLS](/docs/v3/security-tls)
- [ACME automatic TLS](/docs/v3/tls-acme)
- [TLS session ticket keys](/docs/v3/tls-session-tickets)
- [OCSP stapling](/docs/v3/ocsp-stapling)
- [Observability and logging](/docs/v3/observability-logging)

## Scopes

Ferron has three main directive scopes:

- **Global scope** — directives inside top-level `{ ... }` blocks. These affect startup, listeners, and server-wide behavior.
- **Admin API scope** — directives inside the `admin { ... }` global block. These control the built-in administration endpoints.
- **HTTP host scope** — directives inside HTTP host blocks such as `example.com { ... }`. These control per-host behavior including TLS, routing, and content serving.

HTTP host blocks also support control directives that affect request matching and configuration layering:

- `location` — path-based matching and nesting
- `if` / `if_not` — conditional matching based on named matchers
- `handle_error` — error-specific handling for particular status codes

For details on conditionals and matchers, see [Conditionals and variables](/docs/v3/conditionals).

## Important notes

- Where validation and runtime behavior differ, the directive pages call that out explicitly.
- Duration strings accept suffixes like `30m`, `1h`, `90s`, `1d`. Plain numbers without a suffix are treated as hours.
- Boolean directives can be written as bare flags (equivalent to `true`), or explicitly as `true` or `false`.
