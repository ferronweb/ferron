# HTTP Headers & CORS Directives

The `header` and `cors` directives configure response header manipulation and Cross-Origin Resource Sharing (CORS) handling.

## Categories

- Main directives: `header`, `cors`
- Header actions: add (`+`), replace, remove (`-`)
- CORS: `origins`, `methods`, `headers`, `credentials`, `max_age`, `expose_headers`

## `header`

Manipulates response headers before sending to the client. Three forms are supported:

| Syntax | Effect |
| --- | --- |
| `header +Name "value"` | **Add** header (appends, allows duplicates) |
| `header -Name` | **Remove** all instances of the header |
| `header Name "value"` | **Replace** header (removes existing, sets new value) |

Header values support interpolation with `{{...}}` syntax.

### Examples

```ferron
example.com {
    # Add a custom header with interpolated client IP
    header +X-Client-IP "{{remote_address}}"

    # Replace the Server header
    header X-Powered-By "Titanium"

    # Remove the Server header entirely
    header -Server
}
```

### Interpolation Variables

| Variable | Description |
| --- | --- |
| `{{remote_address}}` | The client's IP address |
| `{{local_address}}` | The server's listening address |
| `{{hostname}}` | The matched hostname |
| `{{env.NAME}}` | Environment variable `NAME` |
| Any custom variable set via the `variables` map | User-defined per-request variables |

Unresolved variables are left as `{{name}}` in the output.

## `cors`

Syntax — block form:

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

### Behavior

1. **Preflight handling**: When an `OPTIONS` request includes `Origin` and `Access-Control-Request-Method` headers, the module returns `204 No Content` with the appropriate CORS response headers.

2. **Response headers**: For all responses (including error responses), the following headers are added when CORS is enabled:
   - `Access-Control-Allow-Origin`: The requesting origin (if allowed) or `*`
   - `Access-Control-Allow-Credentials`: `true` when credentials are enabled and origin is not `*`
   - `Access-Control-Allow-Methods`: The configured methods (on preflight)
   - `Access-Control-Allow-Headers`: The configured allowed headers (on preflight)
   - `Access-Control-Max-Age`: The configured max age (on preflight)
   - `Access-Control-Expose-Headers`: The configured exposed headers
   - `Vary: Origin`: When origins are not `*`

### Origin Matching

- If `origins` contains `"*"`, any origin is allowed and `Access-Control-Allow-Origin` is set to `*`.
- Otherwise, the incoming `Origin` header is compared against the list. If it matches, the header is echoed back. If it doesn't match, no CORS headers are added.

### Example: Allow All Origins

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

### Example: Specific Origins with Credentials

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
