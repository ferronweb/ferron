---
title: "Configuration: routing and URL processing"
description: "Request matching, conditional configuration, error handling, and URL redirects."
---

These directives control host, `location`, and conditional matching inside host blocks. The `http-server` module's radix tree resolver processes them.

> [!info]
> For the full request order, see [Request pipeline order](/docs/configuration/fundamentals/request-pipeline). Ferron resolves host, `location`, and `if` blocks once per request, before any pipeline stage runs.

## Path matching

`location <path: string>` matches a path prefix. `/api` matches `/api` and `/api/...`. Longer matches are more specific. When the block matches, Ferron automatically rewrites the URL to remove the base URL, so `/api/users` becomes `/users` for a `/api` location. Default: not configured.

```ferron
example.com {
    location /api {
        # Configuration for /api paths
    }
}
```

- Matching is prefix-based and the most specific location wins.
- Matching a location also strips its prefix from the URL.
- Ferron supports prefixes only. There is no regex form such as `location ~`. Use a `match` block with `if` or `if_not` for pattern routing. See [Conditionals and variables](/docs/configuration/fundamentals/conditionals).
- Ferron selects the `location` block once, on the original URL, before pipeline stages run. A later `rewrite` does not move the request to a different `location` block. See [Request pipeline order](/docs/configuration/fundamentals/request-pipeline).

## Inheritance

A `location` block inherits directives from the enclosing host block and from global defaults. When the same directive appears at both levels, the `location` value wins for requests in that block. The same rule applies to `if` and `if_not` blocks nested in a host or `location` block.

These handler and backend selectors are host-isolated: `proxy`, `fcgi`, `cgi`, `scgi`, `forward_proxy`, `root`, `index`, `basic_auth`, and related static-file tunables. They inherit from global defaults and from the enclosing named host into its `location` blocks. A named host does not inherit them from the wildcard `*` host.

## Conditional matching

### `if`

`if <matcher-name: string>` applies the nested block when the named matcher evaluates to true. Default: not configured.

### `if_not`

`if_not <matcher-name: string>` applies the nested block when the named matcher evaluates to false. Default: not configured.

```ferron
example.com {
    match api_request {
        request.uri.path ~ r"^/api(?:$|/)"
    }

    if api_request {
        # Applied when api_request matcher passes
    }

    if_not api_request {
        # Applied when api_request matcher fails
    }
}
```

> [!info]
> For named matcher syntax and available variables, see [Conditionals and variables](/docs/configuration/fundamentals/conditionals).

## Error handling

`handle_error [status: integer]` associates a nested block with an error code. With no code, it associates the block with the default error case. Default: not configured.

```ferron
example.com {
    handle_error 404 {
        # Custom handling for 404 errors
    }
}
```

## URL redirects

`trailing_slash_redirect [bool: boolean]` enables automatic 301 redirects from a directory path without a trailing slash to the same path with one. When omitted, the value is `true`.

> [!note]
> Host isolation: a named host does not inherit `trailing_slash_redirect` from the wildcard `*` host. It inherits from global defaults. A `location` block inherits it from its own host.

```ferron
example.com {
    root /srv/www/example
    trailing_slash_redirect
}
```

- The redirect applies only when the resolved request path maps to a directory on the filesystem.
- Ferron preserves query strings. For example, `/blog?foo=bar` becomes `/blog/?foo=bar`.
- The redirect keeps relative links in directory-served pages correct and helps SEO consistency.
