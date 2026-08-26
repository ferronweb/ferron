---
title: "Configuration: HTTP map"
description: "The `map` directive for creating variables whose values depend on values of other variables."
---

This page documents the `map` directive. It creates variables whose values come from matching a source variable against a set of patterns. Mapped variables are available via `{{variable}}` interpolation in other directives.

## Directives

### `map`

- `map <source: string> <destination: string>`
  - This directive specifies a source variable to match and a destination variable name to create. The nested block defines the mapping rules. Default: none

> [!note]
> The destination variable name can be any identifier. Ferron stores it in the request variable map. You access it via `{{name}}` interpolation.

#### Block sub-directives

| Sub-directive | Arguments                            | Description                                                                                       | Default      |
| ------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------- | ------------ |
| `default`     | `<value: string>`                    | The fallback value when no entry matches the source.                                              | Empty string |
| `exact`       | `<pattern: string> <result: string>` | Exact string match, or wildcard match if the pattern contains `*`.                                | None         |
| `regex`       | `<pattern: string> <result: string>` | Regular expression match. You can reference capture groups in the result as `$1`, `$2`, and so on | None         |

> [!note]
> If Ferron cannot resolve the source variable, it treats the source value as an empty string and uses the `default` value. Ferron compiles regex patterns at parse time. It rejects invalid patterns during validation. Ferron converts wildcard patterns (`*`) to regex internally.

> [!tip]
> Values can also contain variable interpolations (`{{name}}`) that Ferron resolves at runtime.

#### Block options (inside `regex { ... }`)

| Option             | Arguments | Description                                                                    | Default |
| ------------------ | --------- | ------------------------------------------------------------------------------ | ------- |
| `case_insensitive` | `<bool>`  | When `true`, Ferron matches the regular expression pattern case-insensitively. | `false` |

**Configuration example:**

```ferron
http * {
    map request.uri.path category {
        default uncategorized
        exact /api/* api
        exact /blog/* blog
    }
}
```

### Matching priority

When Ferron evaluates a `map` block, it checks the entries in this order:

1. **Exact match**: the source value equals the pattern string exactly.
2. **Wildcard match**: the pattern contains `*` which matches any characters (equivalent to `.*` in regex). The longest-matching wildcard wins.
3. **Regex match**: the first regular expression in declaration order that matches the source value.
4. **Default**: the `default` value, or an empty string if not specified.

### Simple variable mapping

```ferron
http * {
    map request.uri.path category {
        default uncategorized
        exact /api/* api
        exact /blog/* blog
        exact /docs docs
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

Requests to `/api/users` set `category` to `api`, requests to `/blog/post` set it to `blog`, and `/docs` sets it to `docs`. Everything else falls back to `uncategorized`. Ferron then passes the mapped variable to the backend as a header.

### Regex with capture groups

```ferron
http * {
    map request.uri.path user_id {
        default ""
        regex "^/users/([0-9]+)" "$1"
    }
}
```

A request to `/users/42` sets `user_id` to `42`. Capture groups from the regex are available as `$1`, `$2`, and so on in the result string. If the pattern has no capture groups or the group does not exist, Ferron keeps the reference literally (for example `$1`).

### Case-insensitive matching

```ferron
http * {
    map request.header.user_agent is_mobile {
        default "0"
        regex "mobile" "1" {
            case_insensitive
        }
        regex "android" "1" {
            case_insensitive
        }
    }
}
```

The `case_insensitive` option applies to individual `regex` entries. Alternatively, you can use the inline `(?i)` flag in the pattern itself: `regex "(?i)mobile" "1"`.

### Map at host and location level

You can define `map` blocks inside host blocks and `location` blocks. They inherit from parent scopes using standard Ferron inheritance:

```ferron
http * {
    map request.uri.path site_section {
        default default
        exact /public/* public
    }
}

example.com {
    # Overrides the global map for this host
    map request.uri.path site_section {
        default example-default
        exact /special/* special
    }

    location /admin {
        # Overrides at location level
        map request.uri.path site_section {
            default admin
        }
    }
}
```

When you define a `map` with the same destination variable at multiple levels, the innermost scope takes precedence. Ferron evaluates all maps with different destination variables.

## Pipeline position

Map evaluation runs after client IP resolution and before URL rewriting. This means mapped variables are available for use in `rewrite` patterns, proxy configuration, and other downstream directives.

> [!info]
> For `map` interaction with rewriting, see [URL rewriting](./rewrite.md).

## Observability

### Trace spans

The map stage sets the following attributes on its `ferron.stage.map` span:

| Attribute             | Type   | Description                                     |
| --------------------- | ------ | ----------------------------------------------- |
| `ferron.map.variable` | string | The variable name that Ferron maps.             |
| `ferron.map.edited`   | bool   | Whether the mapping changed the variable value. |
