---
title: "Configuration: HTTP map"
description: "The `map` directive for creating variables whose values depend on values of other variables."
---

`map` creates variables by matching a source value against patterns. Other directives read the result with `{{variable}}` interpolation.

> [!important]
> `map` belongs in HTTP scope: an `http *` block, a host block, or a `location` block. It has no effect at the bare top level, outside any block. When you migrate from NGINX, `http *` is the counterpart of the NGINX `http` context.

## `map`

`map <source: string> <destination: string>` matches a source variable (a plain string) or an interpolated string, and creates the destination variable. The nested block defines the mapping rules. Default: none.

The destination name can be any identifier. Ferron stores it in the request variable map, and you read it with `{{name}}`.

| Sub-directive | Arguments                            | Description                                                                                        | Default      |
| ------------- | ------------------------------------ | -------------------------------------------------------------------------------------------------- | ------------ |
| `default`     | `<value: string>`                    | Fallback value when no entry matches the source.                                                   | Empty string |
| `exact`       | `<pattern: string> <result: string>` | Exact string match, or wildcard match when the pattern contains `*`.                               | None         |
| `regex`       | `<pattern: string> <result: string>` | Regular expression match. Capture groups are available in the result as `$1`, `$2`, and so on.    | None         |

If Ferron cannot resolve the source variable, it uses an empty string as the source value and applies `default`. Ferron compiles regex patterns at parse time and rejects invalid patterns during validation. It converts wildcard patterns (`*`) to regex internally.

Values can also contain variable interpolations (`{{name}}`), which Ferron resolves at runtime.

### `regex` options

| Option             | Arguments | Description                                            | Default |
| ------------------ | --------- | ------------------------------------------------------ | ------- |
| `case_insensitive` | `<bool>`  | Match the regular expression case-insensitively.       | `false` |

```ferron
http * {
    map request.uri.path category {
        default uncategorized
        exact /api/* api
        exact /blog/* blog
    }
}
```

## Matching priority

Ferron checks entries in this order:

1. Exact match: the source value equals the pattern string.
2. Wildcard match: the pattern contains `*`, which matches any characters (equivalent to `.*` in regex). The longest matching wildcard wins.
3. Regex match: the first regular expression in declaration order that matches the source value.
4. Default: the `default` value, or an empty string if it is not set.

## Examples

### Route requests by path

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

Requests to `/api/users` set `category` to `api`, requests to `/blog/post` set it to `blog`, and `/docs` sets it to `docs`. Everything else falls back to `uncategorized`. Ferron passes the mapped value to the backend as a header.

### Capture a regex group

```ferron
http * {
    map request.uri.path user_id {
        default ""
        regex r"^/users/([0-9]+)" "$1"
    }
}
```

A request to `/users/42` sets `user_id` to `42`. If the pattern has no capture groups, or the referenced group does not exist, Ferron keeps the reference literally, for example `$1`.

### Match case-insensitively

```ferron
http * {
    map request.header.user_agent is_mobile {
        default "0"
        regex r"mobile" "1" {
            case_insensitive
        }
        regex r"android" "1" {
            case_insensitive
        }
    }
}
```

The `case_insensitive` option applies to one `regex` entry. You can also use the inline `(?i)` flag in the pattern: `regex "(?i)mobile" "1"`.

## Scope and inheritance

You can define `map` blocks in `http *` blocks, host blocks, and `location` blocks. They inherit from parent scopes through standard Ferron inheritance. Do not place `map` in the bare global `{ ... }` block.

When maps at several levels use the same destination variable, the innermost scope wins. Ferron evaluates all maps that use different destination variables.

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

### Match a multi-variable source

```ferron
http * {
    map "{{request.method}} {{request.uri.path}}" type {
        default uncategorized
        exact "GET /*" homepage
        exact "GET /docs/*" docs
    }
}

example.com {
    location / {
        proxy http://backend {
            request_header X-Request-Type "{{type}}"
        }
    }
}
```

## Pipeline position

Map evaluation runs after client IP resolution and before URL rewriting. Mapped variables are available in `rewrite` patterns, proxy configuration, and other downstream directives.

> [!info]
> For `map` interaction with rewriting, see [URL rewriting](./rewrite.md).

## Observability

### Trace spans

The map stage sets these attributes on its `ferron.stage.map` span:

| Attribute             | Type   | Description                                     |
| --------------------- | ------ | ----------------------------------------------- |
| `ferron.map.variable` | string | The variable name that Ferron maps.             |
| `ferron.map.edited`   | bool   | Whether the mapping changed the variable value. |
