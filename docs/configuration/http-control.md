# HTTP Control Directives

These directives affect HTTP request matching and configuration layering inside host blocks.

## Categories

- Path matching: `location`
- Conditional matching: `if`, `if_not`
- Error layering: `handle_error`
- Web root: `root`
- URL sanitization: `url_sanitize`

## `location`

Syntax:

```ferron
example.com {
    location /api {
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` | Matches a request path by exact match or prefix. `/api` matches `/api` and `/api/...`. | not configured |

Notes:

- Matching is path-prefix based.
- Longer matches are more specific.
- If this block matches, the URL is automatically rewritten to remove the base URL

## `if`

Syntax:

```ferron
example.com {
    if api_request {
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<matcher-name>` | Applies the nested block when the named matcher evaluates to true. | not configured |

See also:

- [Conditionals And Variables](./conditionals.md)

## `if_not`

Syntax:

```ferron
example.com {
    if_not api_request {
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<matcher-name>` | Applies the nested block when the named matcher evaluates to false. | not configured |

See also:

- [Conditionals And Variables](./conditionals.md)

## `handle_error`

Syntax:

```ferron
example.com {
    handle_error 404 {
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<number>` or no argument | Associates a nested block with a specific error code, or with a default error case when no code is given. | not configured |

Current status:

- `handle_error` is prepared and stored by the resolver.
- It is not currently applied by the HTTP request handler.
- Treat it as reserved for future error-layer handling.

## `root`

Syntax:

```ferron
example.com {
    root /srv/www/example
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` | Sets the webroot used by the HTTP file-handler pipeline after regular HTTP stages leave the request without a response. | not configured |

Notes:

- The resolved path is canonicalized before file stages run.
- Requests that try to escape the webroot are rejected.
- If a request continues below a matched file path, the unmatched suffix is carried into the file-stage context as `path_info`.

## `url_sanitize`

Syntax:

```ferron
example.com {
    http {
        url_sanitize false
    }
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<boolean>` | Enables or disables URL path sanitization. When enabled (the default), dangerous sequences such as path traversal attempts (`../`, `..\\`), null bytes, and invalid percent-encodings are removed or normalized. | `true` (enabled) |

Notes:

- URL sanitization is applied early in request processing, before configuration resolution.
- This directive is only read from the **global** configuration block. Per-host settings are not currently supported.
- Disabling URL sanitization may improve RFC 3986 compliance for URLs that use valid but unusual encodings.
- **Warning**: When disabled, Ferron will not protect backend services from path traversal attacks if reverse proxying is implemented. Use with caution.
- Even when disabled, the file resolution stage still canonicalizes paths and rejects requests that escape the configured webroot.
