---
title: "Configuration: Variable setting"
description: "The `set_var` and `log_field` directives for setting variables from request conditions and writing custom access log fields."
---

`set_var` sets interpolation variables from request conditions. `log_field` writes variables and interpolated values to custom access log fields.

## `set_var`

`set_var <source: string> <regex: string> <variable: string>` sets a variable when the source value matches the regular expression. On a match, the value defaults to `"1"`. Default: none.

> [!note]
> `set_var` is similar to Apache `SetEnvIf`. It evaluates a regex against a resolved variable and conditionally sets a new variable. Multiple `set_var` directives can target the same variable. Ferron evaluates them in declaration order, and the last match wins.

| Sub-directive      | Arguments  | Description                                                             | Default |
| ------------------ | ---------- | ----------------------------------------------------------------------- | ------- |
| `value`            | `<string>` | Value to assign when the pattern matches.                               | `"1"`   |
| `case_insensitive` | `<bool>`   | Match the regex case-insensitively.                                     | `false` |
| `negate`           | `<bool>`   | Set the variable when the pattern does not match.                       | `false` |

```ferron
example.com {
    set_var request.uri.path r"\.pdf$" is_pdf
    set_var remote.ip r"^192\.168\." network_type {
        value private
    }
}
```

## `log_field`

`log_field <field: string> <source: string>` maps a variable or interpolated value to a custom access log field. Ferron evaluates the field after it generates the response, so response-time variables are available. Default: none.

> [!note]
> The source can be a plain variable name such as `network_type`, or an interpolated string such as `"{{request.header.x_custom_header}}"`. Ferron resolves plain names through the `Variables` trait at runtime. It resolves interpolated strings at log time with the full variable resolution system, including request headers, URI components, and custom variables.

```ferron
http * {
    set_var request.uri.path r"\.pdf$" is_pdf
    set_var remote.ip r"^192\.168\." network_type {
        value private
    }

    log_field file_type is_pdf
    log_field network network_type
}
```

After Ferron generates the response, the access log includes `file_type` and `network` with values from the variables set earlier.

## Examples

### Match request properties

```ferron
http * {
    set_var request.uri.path r"\.pdf$" is_pdf
    set_var request.uri.path r"\.(jpg|png|gif)$" is_image
    set_var request.method r"^POST$" is_post
}
```

Requests to `/document.pdf` set `is_pdf` to `"1"`, requests to `/photo.jpg` set `is_image` to `"1"`, and POST requests set `is_post` to `"1"`. Downstream directives can interpolate these variables.

### Set custom values

```ferron
http * {
    set_var request.uri.path r"\.pdf$" file_type {
        value pdf
    }
    set_var request.uri.path r"\.txt$" file_type {
        value text
    }
}
```

### Match case-insensitively

```ferron
http * {
    set_var request.header.user_agent r"mobile" is_mobile {
        case_insensitive
    }
}
```

This matches user agents that contain "mobile" in any capitalization, for example "Mobile", "MOBILE", or "MoBiLe".

### Negate a match

```ferron
http * {
    set_var request.header.x_forwarded_for r"." has_xff {
        negate
    }
}
```

Ferron sets `has_xff` to `"1"` when the `X-Forwarded-For` header is absent or empty. Use this to tell direct connections from proxied requests.

### Use a variable in another directive

```ferron
http * {
    set_var remote.ip r"^10\." is_internal {
        value "true"
    }

    location /admin {
        proxy http://admin-backend {
            request_header X-Internal "{{is_internal}}"
        }
    }
}
```

## Pipeline position

`set_var` runs after client IP resolution and before URL rewriting and the `map` directive. Variables set by `set_var` are available for `map` evaluation, `rewrite` patterns, and all downstream pipeline stages.

`log_field` runs during the inverse (post-response) phase, after the content-generating stages such as reverse proxy, static file, and CGI have produced a response.

> [!info]
> For variable mapping based on complex patterns, see [HTTP map](./map.md). For URL rewriting, see [URL rewriting](./rewrite.md).

## Observability

### Trace spans

The variables stage sets this attribute on its `ferron.stage.variables` span:

| Attribute              | Type | Description                                |
| ---------------------- | ---- | ------------------------------------------ |
| `ferron.variables.set` | int  | Number of variables set during this stage. |
