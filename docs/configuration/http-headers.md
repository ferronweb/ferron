---
title: "Configuration: HTTP headers and CORS"
description: "Response header manipulation and Cross-Origin Resource Sharing (CORS) directives."
---

This page documents the `header` and `cors` directives for configuring response header manipulation and Cross-Origin Resource Sharing (CORS) handling.

## Directives

### `header`

The `header` directive manipulates response headers before sending to the client. Three forms are supported:

| Syntax | Effect |
| --- | --- |
| `header +Name "value"` | **Add** header (appends, allows duplicates) |
| `header -Name` | **Remove** all instances of the header |
| `header Name "value"` | **Replace** header (removes existing, sets new value) |

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

| Variable | Description |
| --- | --- |
| `{{remote.ip}}` | The client's IP address |
| `{{remote.port}}` | The client's port |
| `{{server.ip}}` | The server's listening IP address |
| `{{server.port}}` | The server's listening port |
| `{{request.host}}` | The matched hostname |
| `{{request.scheme}}` | `http` or `https` |
| `{{env.NAME}}` | Environment variable `NAME` |

For the complete variable reference, see [Conditionals and variables](./conditionals.md#built-in-variables).

Unresolved variables are left as `{{name}}` in the output.

### `cors`

The `cors` directive configures Cross-Origin Resource Sharing behavior.

```ferron
example.com {
    cors {
        origins "https://example.com" "https://app.example.com"
        methods GET POST PUT DELETE
        headers "Content-Type" "Authorization"
        credentials true
        max_age 86400
        expose_headers "X-Custom-Header"
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `origins` | `<string>...` | Allowed origins. Use `"*"` to allow all. | none (CORS disabled) |
| `methods` | `<string>...` | Allowed HTTP methods for preflight. | none |
| `headers` | `<string>...` | Allowed request headers for preflight. | none |
| `credentials` | `<bool>` | Allow credentials (cookies, auth headers). | `false` |
| `max_age` | `<number>` | Preflight cache duration in seconds. | none |
| `expose_headers` | `<string>...` | Headers exposed to the browser in responses. | none |

#### Behavior

1. **Preflight handling**: When an `OPTIONS` request includes `Origin` and `Access-Control-Request-Method` headers, the module returns `204 No Content` with the appropriate CORS response headers.

2. **Response headers**: For all responses (including error responses), CORS headers are added when enabled, including `Access-Control-Allow-Origin`, `Access-Control-Allow-Credentials`, `Access-Control-Allow-Methods`, `Access-Control-Allow-Headers`, `Access-Control-Max-Age`, `Access-Control-Expose-Headers`, and `Vary: Origin`.

#### Origin matching

- If `origins` contains `"*"`, any origin is allowed and `Access-Control-Allow-Origin` is set to `*`.
- Otherwise, the incoming `Origin` header is compared against the list. If it matches, the header is echoed back. If it doesn't match, no CORS headers are added.

**Configuration example — allow all origins:**

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

**Configuration example — specific origins with credentials:**

```ferron
api.example.com {
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

## Notes and troubleshooting

- If CORS headers are not appearing in responses, verify that `origins` is configured (CORS is disabled by default if `origins` is empty).
- For header interpolation, `remote.ip` and `server.ip` automatically canonicalize IPv4-mapped IPv6 addresses to IPv4. See [Conditionals and variables](./conditionals.md#ip-canonicalization) for details.
- For HTTP host directives, see [HTTP host directives](/docs/v3/configuration/http-host).
