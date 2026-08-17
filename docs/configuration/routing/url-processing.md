---
title: "Configuration: routing and URL processing"
description: "Request matching, conditional configuration, error handling, web root, and URL sanitation."
---

This page documents directives that affect HTTP request matching and configuration layering inside host blocks. The `http-server` module's radix tree resolver processes these directives.

## Directives

### Path matching

- `location <path: string>`
  - This directive specifies a path prefix for request matching. `/api` matches `/api` and `/api/...`. Longer matches are more specific. If this block matches, Ferron automatically rewrites the URL to remove the base URL. Default: not configured

**Configuration example:**

```ferron
example.com {
    location /api {
        # Configuration for /api paths
    }
}
```

> [!note]
>
> - Matching is prefix-based (`/api` matches `/api` and `/api/users`). More specific locations win over less specific ones.
> - If this block matches, Ferron automatically rewrites the URL to remove the base URL.

### Conditional matching

- `if <matcher-name: string>`
  - This directive specifies a named matcher to evaluate. When the named matcher evaluates to true, Ferron applies the nested block's directives. Default: not configured
- `if_not <matcher-name: string>`
  - This directive specifies a named matcher to evaluate. When the named matcher evaluates to false, Ferron applies the nested block's directives. Default: not configured

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

> [!info]
> For named matcher syntax and available variables, see [Conditionals and variables](/docs/v3/configuration/fundamentals/conditionals).

### Error handling

- `handle_error [status: integer]`
  - This directive associates a nested block with a specific error code, or with a default error case when you give no code. Default: not configured

**Configuration example:**

```ferron
example.com {
    handle_error 404 {
        # Custom handling for 404 errors
    }
}
```

### URL redirects

- `trailing_slash_redirect [bool: boolean]`
  - This directive specifies whether automatic 301 redirects from directory paths without a trailing slash to the same path with a trailing slash are enabled. When omitted, defaults to `true`. Default: `trailing_slash_redirect true`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    trailing_slash_redirect
}
```

> [!note]
> Notes for `trailing_slash_redirect`:
>
> - Only applies when the resolved request path maps to a directory on the filesystem.
> - Ferron preserves query strings in the redirect (for example `/blog?foo=bar` → `/blog/?foo=bar`).
> - This is useful for SEO consistency and for making sure relative links within directory-served pages resolve correctly.
