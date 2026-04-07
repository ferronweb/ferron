---
title: "Configuration: routing and URL processing"
description: "Request matching, conditional configuration, error handling, web root, and URL sanitation."
---

This page documents directives that affect HTTP request matching and configuration layering inside host blocks.

## Directives

### Path matching

- `location <path: string>`
  - This directive specifies a path prefix for request matching. `/api` matches `/api` and `/api/...`. Longer matches are more specific. If this block matches, the URL is automatically rewritten to remove the base URL. Default: not configured

**Configuration example:**

```ferron
example.com {
    location /api {
        # Configuration for /api paths
    }
}
```

Notes:

- Matching is path-prefix based.
- If this block matches, the URL is automatically rewritten to remove the base URL.

### Conditional matching

- `if <matcher-name: string>`
  - This directive specifies a named matcher to evaluate. When the named matcher evaluates to true, the nested block's directives are applied. Default: not configured
- `if_not <matcher-name: string>`
  - This directive specifies a named matcher to evaluate. When the named matcher evaluates to false, the nested block's directives are applied. Default: not configured

**Configuration example:**

```ferron
example.com {
    if api_request {
        # Applied when api_request matcher passes
    }

    if_not api_request {
        # Applied when api_request matcher fails
    }
}
```

For named matcher syntax and available variables, see [Conditionals and variables](/docs/v3/configuration/conditionals).

### Error handling

- `handle_error [status: integer]`
  - This directive associates a nested block with a specific error code, or with a default error case when no code is given. Default: not configured

**Configuration example:**

```ferron
example.com {
    handle_error 404 {
        # Custom handling for 404 errors
    }
}
```

### Web root

- `root <path: string>`
  - This directive specifies the webroot used by the HTTP file-handler pipeline after regular HTTP stages leave the request without a response. The resolved path is canonicalized before file stages run. Requests that try to escape the webroot are rejected. Default: not configured

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
}
```

Notes:

- If a request continues below a matched file path, the unmatched suffix is carried into the file-stage context as `path_info`.
- Additional static file behavior (index resolution, compression, ETags, directory listings, MIME types) is controlled by separate directives. See [Static file serving](/docs/v3/configuration/static-content).

### URL sanitation and redirects

- `url_sanitize [bool: boolean]`
  - This directive specifies whether URL path sanitization is enabled. When enabled (the default), dangerous sequences such as path traversal attempts (`../`, `..\\`), null bytes, and invalid percent-encodings are removed or normalized. When omitted, defaults to `true`. Default: `url_sanitize true`
- `trailing_slash_redirect [bool: boolean]`
  - This directive specifies whether automatic 301 redirects from directory paths without a trailing slash to the same path with a trailing slash are enabled. When omitted, defaults to `true`. Default: `trailing_slash_redirect true`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    trailing_slash_redirect
}
```

Notes for `url_sanitize`:

- URL sanitization is applied early in request processing, before configuration resolution.
- This directive is only read from the **global** configuration block. Per-host settings are not currently supported.
- Disabling URL sanitization may improve RFC 3986 compliance for URLs that use valid but unusual encodings.
- **Warning:** When disabled, Ferron will not protect backend services from path traversal attacks if reverse proxying is implemented. Use with caution.
- Even when disabled, the file resolution stage still canonicalizes paths and rejects requests that escape the configured webroot.

Notes for `trailing_slash_redirect`:

- Only applies when the resolved request path maps to a directory on the filesystem.
- Query strings are preserved in the redirect (e.g. `/blog?foo=bar` → `/blog/?foo=bar`).
- This is useful for SEO consistency and ensuring relative links within directory-served pages resolve correctly.

## Notes and troubleshooting

- `location` is prefix-based. `/api` matches `/api` and `/api/users`.
- More specific locations win over less specific ones.
- For [conditionals and variables](/docs/v3/configuration/conditionals), see the dedicated page.
- For static file serving, see [Static file serving](/docs/v3/configuration/static-content).
- For URL rewriting, see [URL rewriting](/docs/v3/configuration/http-rewrite).
