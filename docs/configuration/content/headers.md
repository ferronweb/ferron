---
title: "Configuration: HTTP headers and CORS"
description: "Response header manipulation and Cross-Origin Resource Sharing (CORS) directives."
---

This page documents the `header` and `cors` directives for configuring response header manipulation and Cross-Origin Resource Sharing (CORS) handling.

## Directives

### `header`

The `header` directive manipulates response headers before sending to the client. The directive supports three forms:

| Syntax                 | Effect                                                |
| ---------------------- | ----------------------------------------------------- |
| `header +Name "value"` | **Add** header (appends, allows duplicates)           |
| `header -Name`         | **Remove** all instances of the header                |
| `header Name "value"`  | **Replace** header (removes existing, sets new value) |

Header values support interpolation with `{{...}}` syntax.

**Configuration example:**

```ferron
example.com {
    header +X-Client-IP "{{remote.ip}}"
    header X-Powered-By "Ferron"
    header -Server
}
```

#### Interpolation variables

| Variable             | Description                       |
| -------------------- | --------------------------------- |
| `{{remote.ip}}`      | IP address of the client           |
| `{{remote.port}}`    | Port of the client                 |
| `{{server.ip}}`      | Listening IP address of the server |
| `{{server.port}}`    | Listening port of the server       |
| `{{request.host}}`   | The matched hostname              |
| `{{request.scheme}}` | `http` or `https`                 |
| `{{env.NAME}}`       | Environment variable `NAME`       |

> [!note]
> For header interpolation, `remote.ip` and `server.ip` automatically canonicalize IPv4-mapped IPv6 addresses to IPv4. See [Conditionals and variables](../fundamentals/conditionals.md#ip-canonicalization) and [HTTP host directives](/docs/v3/configuration/server/host) for details.

> [!info]
> For the complete variable reference, see [Conditionals and variables](../fundamentals/conditionals.md#built-in-variables).

Ferron leaves unresolved variables as `{{name}}` in the output.

### `cors`

The `cors` directive configures Cross-Origin Resource Sharing behavior.

```ferron
example.com {
    cors {
        origins "https://example.com" "https://app.example.com"
        methods GET POST PUT DELETE
        headers "Content-Type" "Authorization"
        credentials
        max_age 86400
        expose_headers "X-Custom-Header"
    }
}
```

| Nested directive | Arguments     | Description                                                                                                                                        | Default              |
| ---------------- | ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------- |
| `origins`        | `<string>...` | Allowed origins. Use `"*"` to allow all. Accepts variable interpolations (for example `{{request.header.origin}}` for `Origin` header reflection). | none (CORS disabled) |
| `methods`        | `<string>...` | Allowed HTTP methods for preflight.                                                                                                                | none                 |
| `headers`        | `<string>...` | Allowed request headers for preflight.                                                                                                             | none                 |
| `credentials`    | `<bool>`      | Allow credentials (cookies, auth headers).                                                                                                         | `false`              |
| `max_age`        | `<number>`    | Preflight cache duration in seconds.                                                                                                               | none                 |
| `expose_headers` | `<string>...` | Headers exposed to the browser in responses.                                                                                                       | none                 |

#### Behavior

1. **Preflight handling**: When an `OPTIONS` request includes `Origin` and `Access-Control-Request-Method` headers, the module returns `204 No Content` with the appropriate CORS response headers.

2. **Response headers**: When enabled, the module adds CORS headers to all responses (including error responses). The headers include `Access-Control-Allow-Origin`, `Access-Control-Allow-Credentials`, `Access-Control-Allow-Methods`, `Access-Control-Allow-Headers`, `Access-Control-Max-Age`, `Access-Control-Expose-Headers`, and `Vary: Origin`.

#### Origin matching

- If `origins` contains `"*"`, the module allows any origin and sets `Access-Control-Allow-Origin` to `*`.
- Otherwise, the module compares the incoming `Origin` header against the list. If it matches, the module echoes the header back. If it does not match, the module adds no CORS headers.

**Configuration example: allow all origins**

```ferron
api.example.com {
    cors {
        origins "*"
        methods GET POST
        headers "Content-Type" "Authorization"
        credentials false
        max_age 3600
    }
}
```

**Configuration example: specific origins with credentials**

```ferron
api.example.com {
    cors {
        origins "https://app.example.com" "https://admin.example.com"
        methods GET POST PUT DELETE OPTIONS
        headers "Content-Type" "Authorization" "X-Request-ID"
        credentials
        max_age 86400
        expose_headers "X-Total-Count" "X-Page"
    }
}
```

> [!note]
> If CORS headers do not appear in responses, verify that you set `origins`. Ferron disables CORS by default if `origins` is empty.

## Best practices

`ferron doctor` reports the following best-practice check for directives on this page.

- **`cors { credentials true }` with `origins "*"`**. Allowing credentials with wildcard origins defeats browser same-origin protection. Use explicit trusted origins when you allow credentials.

## Observability

### Trace spans

The headers stage sets the following attributes on its `ferron.stage.headers` span:

| Attribute              | Type | Description                         |
| ---------------------- | ---- | ----------------------------------- |
| `ferron.headers.set`   | int  | Number of response headers set.     |
| `ferron.headers.unset` | int  | Number of response headers removed. |
