---
title: "Configuration: HTTP map"
description: "The `map` directive for creating variables whose values depend on values of other variables."
---

This page documents the `map` directive, which creates variables whose values are determined by matching a source variable against a set of patterns. Mapped variables are available via `{{variable}}` interpolation in other directives.

## Directives

### `map`

- `map <source: string> <destination: string>`
  - This directive specifies a source variable to match and a destination variable name to create. The nested block defines the mapping rules. Default: none

#### Block sub-directives

| Sub-directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `default` | `<value: string>` | The fallback value when no entry matches the source. | Empty string |
| `exact` | `<pattern: string> <result: string>` | Exact string match, or wildcard match if the pattern contains `*`. | None |
| `regex` | `<pattern: string> <result: string>` | Regular expression match. Capture groups can be referenced in the result as `$1`, `$2`, etc. | None |

#### Block options (inside `regex { ... }`)

| Option | Arguments | Description | Default |
| --- | --- | --- | --- |
| `case_insensitive` | `<bool>` | When `true`, the regular expression pattern is matched case-insensitively. | `false` |

**Configuration example:**

```ferron
http * {
    map request.uri.path category {
        default "uncategorized"
        exact "/api/*" "api"
        exact "/blog/*" "blog"
    }
}
```

### Matching priority

When evaluating a `map` block, entries are checked in this order:

1. **Exact match** — the source value equals the pattern string exactly.
2. **Wildcard match** — the pattern contains `*` which matches any characters (equivalent to `.*` in regex). The longest-matching wildcard wins.
3. **Regex match** — the first regular expression in declaration order that matches the source value.
4. **Default** — the `default` value, or an empty string if not specified.

### Simple variable mapping

```ferron
http * {
    map request.uri.path category {
        default "uncategorized"
        exact "/api/*" "api"
        exact "/blog/*" "blog"
        exact "/docs" "docs"
    }
}

example.com {
    location / {
        proxy http://backend {
            request_header X-Category "{{category}}"
        }
    }
}
```

Requests to `/api/users` set `category` to `api`, requests to `/blog/post` set it to `blog`, and `/docs` sets it to `docs`. Everything else falls back to `uncategorized`. The mapped variable is then passed as a header to the backend.

### Regex with capture groups

```ferron
http * {
    map request.uri.path user_id {
        default ""
        regex "^/users/([0-9]+)" "$1"
    }
}
```

A request to `/users/42` sets `user_id` to `42`. Capture groups from the regex are available as `$1`, `$2`, etc. in the result string. If the pattern has no capture groups or the group doesn't exist, the reference is kept literally (e.g. `$1`).

### Case-insensitive matching

```ferron
http * {
    map request.header.user_agent is_mobile {
        default "0"
        regex "mobile" "1" { case_insensitive true }
        regex "android" "1" { case_insensitive true }
    }
}
```

The `case_insensitive` option applies to individual `regex` entries. Alternatively, you can use the inline `(?i)` flag in the pattern itself: `regex "(?i)mobile" "1"`.

### Map at host and location level

`map` blocks can be defined inside host blocks and `location` blocks. They inherit from parent scopes using standard Ferron inheritance:

```ferron
http * {
    map request.uri.path site_section {
        default "default"
        exact "/public/*" "public"
    }
}

example.com {
    # Overrides the global map for this host
    map request.uri.path site_section {
        default "example-default"
        exact "/special/*" "special"
    }

    location /admin {
        # Overrides at location level
        map request.uri.path site_section {
            default "admin"
        }
    }
}
```

When a `map` with the same destination variable is defined at multiple levels, the innermost scope takes precedence. Maps with different destination variables are all evaluated.

## Pipeline position

Map evaluation runs after client IP resolution and before URL rewriting. This means mapped variables are available for use in `rewrite` patterns, proxy configuration, and other downstream directives.

## Notes and troubleshooting

- If the source variable cannot be resolved, the source value is treated as an empty string, and the `default` value (or empty string) is used.
- Regex patterns are compiled at configuration parse time, not at request time. Invalid patterns are rejected during validation.
- Wildcard patterns (`*`) are converted to regex under the hood and are slightly more expensive than exact matches, but cheaper than general regex patterns.
- For `map` interaction with rewriting, see [URL rewriting](./http-rewrite). Rewrites receive the mapped variables since `map` runs first in the pipeline.
- The destination variable name can be any identifier — it is stored in the request's variable map and accessed via `{{name}}` interpolation.
