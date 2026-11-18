---
title: "Configuration: HTTP headers and CORS"
description: "Add, replace, or remove response headers, and configure Cross-Origin Resource Sharing."
---

`header` changes response headers before Ferron sends them. `cors` adds Cross-Origin Resource Sharing headers and answers preflight requests.

## `header`

| Syntax                 | Effect                                        |
| ---------------------- | --------------------------------------------- |
| `header +Name "value"` | Add a header. Existing headers stay.          |
| `header -Name`         | Remove every instance of the header.          |
| `header Name "value"`  | Replace the header with a single value.       |

Header values support `{{...}}` interpolation.

```ferron
example.com {
    header +X-Client-IP "{{remote.ip}}"
    header X-Powered-By "Ferron"
    header -Server
}
```

### Interpolation variables

| Variable             | Value                              |
| -------------------- | ---------------------------------- |
| `{{remote.ip}}`      | Client IP address.                 |
| `{{remote.port}}`    | Client port.                       |
| `{{server.ip}}`      | Listening IP address.              |
| `{{server.port}}`    | Listening port.                    |
| `{{request.host}}`   | Matched hostname.                  |
| `{{request.scheme}}` | `http` or `https`.                 |
| `{{env.NAME}}`       | Environment variable `NAME`.       |

For header interpolation, `remote.ip` and `server.ip` convert IPv4-mapped IPv6 addresses to IPv4. See [Conditionals and variables](../fundamentals/conditionals.md#ip-canonicalization) and [HTTP host directives](/docs/configuration/server/host) for details. For the full variable reference, see [Conditionals and variables](../fundamentals/conditionals.md#built-in-variables).

Ferron leaves an unresolved variable as `{{name}}` in the output.

## `cors`

```ferron
example.com {
    cors {
        origins https://example.com https://app.example.com
        methods GET POST PUT DELETE
        headers Content-Type Authorization
        credentials
        max_age 86400
        expose_headers X-Custom-Header
    }
}
```

| Nested directive | Arguments     | Description                                     | Default              |
| ---------------- | ------------- | ----------------------------------------------- | -------------------- |
| `origins`        | `<string>...` | Allowed origins. Use `"*"` for all.             | none (CORS disabled) |
| `methods`        | `<string>...` | Allowed methods for preflight.                  | none                 |
| `headers`        | `<string>...` | Allowed request headers for preflight.          | none                 |
| `credentials`    | `<bool>`      | Allow credentials such as cookies and auth headers. | `false`          |
| `max_age`        | `<number>`    | Preflight cache duration in seconds.            | none                 |
| `expose_headers` | `<string>...` | Response headers exposed to the browser.        | none                 |

`origins` accepts interpolation, for example `{{request.header.origin}}` to reflect the `Origin` header. With no `origins` value, CORS stays disabled.

### Preflight and response headers

Ferron answers a preflight request when an `OPTIONS` request has both `Origin` and `Access-Control-Request-Method`. The response is `204 No Content` with the CORS headers.

With CORS enabled, Ferron adds CORS headers to all responses, including error responses: `Access-Control-Allow-Origin`, `Access-Control-Allow-Credentials`, `Access-Control-Allow-Methods`, `Access-Control-Allow-Headers`, `Access-Control-Max-Age`, `Access-Control-Expose-Headers`, and `Vary: Origin`.

### Origin matching

- With `"*"` in `origins`, Ferron allows any origin and sets `Access-Control-Allow-Origin` to `*`.
- Otherwise, Ferron compares the request `Origin` with the list. On a match it echoes the origin back. On a mismatch it adds no CORS headers.

Allow all origins:

```ferron
api.example.com {
    cors {
        origins "*"
        methods GET POST
        headers Content-Type Authorization
        credentials false
        max_age 3600
    }
}
```

Allow specific origins with credentials:

```ferron
api.example.com {
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

If CORS headers do not appear, check that you set `origins`.

## Best practices

`ferron doctor` reports this check for directives on this page.

- `cors { credentials true }` with `origins "*"`: credentials plus wildcard origins defeat browser same-origin protection. Use explicit trusted origins when you allow credentials.

## Observability

### Trace spans

The headers stage sets these attributes on its `ferron.stage.headers` span.

| Attribute              | Type | Description                         |
| ---------------------- | ---- | ----------------------------------- |
| `ferron.headers.set`   | int  | Number of response headers set.     |
| `ferron.headers.unset` | int  | Number of response headers removed. |
