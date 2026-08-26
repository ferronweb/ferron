---
title: "Configuration: Variable setting"
description: "The `set_var` and `log_field` directives for setting variables based on request conditions and custom access log fields."
---

This page documents the `set_var` and `log_field` directives. They set interpolation variables based on request conditions and map them to custom access log fields.

## Directives

### `set_var`

- `set_var <source: string> <regex: string> <variable: string>` (`http-variables`)
  - Sets a variable when the source value matches the regular expression. By default, Ferron sets the variable to `"1"` on match. Default: none

> [!note]
> The `set_var` directive is similar to the `SetEnvIf` directive in Apache. It evaluates a regex against a resolved variable and conditionally sets a new variable. Multiple `set_var` directives can target the same variable. Ferron evaluates them in declaration order with last-match-wins semantics.

#### Block sub-directives

| Sub-directive      | Arguments  | Description                                                                | Default |
| ------------------ | ---------- | -------------------------------------------------------------------------- | ------- |
| `value`            | `<string>` | The value to assign when the pattern matches.                              | `"1"`   |
| `case_insensitive` | `<bool>`   | When `true`, Ferron matches the regex case-insensitively.                  | `false` |
| `negate`           | `<bool>`   | When `true`, Ferron sets the variable when the pattern does not match. | `false` |

**Configuration example:**

```ferron
example.com {
    set_var request.uri.path r"\.pdf$" is_pdf
    set_var remote.ip r"^192\.168\." network_type {
        value private
    }
}
```

### `log_field`

- `log_field <field: string> <source: string>` (`http-variables`)
  - Maps a variable or interpolated value to a custom access log field. Ferron evaluates the field after it generates the response, so response-time variables are available. Default: none

> [!note]
> The source can be a plain variable name (for example `network_type`) or an interpolated string (for example `"{{request.header.x_custom_header}}"`). Ferron resolves plain variable names via the `Variables` trait at runtime.

**Configuration example:**

```ferron
example.com {
    log_field user_network network_type
    log_field is_pdf_request is_pdf
    log_field custom_tag "{{request.header.x_custom_header}}"
}
```

### Setting variables with `set_var`

**Basic matching:**

```ferron
http * {
    set_var request.uri.path r"\.pdf$" is_pdf
    set_var request.uri.path r"\.(jpg|png|gif)$" is_image
    set_var request.method "^POST$" is_post
}
```

Requests to `/document.pdf` set `is_pdf` to `"1"`, requests to `/photo.jpg` set `is_image` to `"1"`, and POST requests set `is_post` to `"1"`. These variables are then available for interpolation in downstream directives.

**Custom values:**

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

**Case-insensitive matching:**

```ferron
http * {
    set_var request.header.user_agent "mobile" is_mobile {
        case_insensitive
    }
}
```

This matches user agents containing "mobile" regardless of capitalization (for example "Mobile", "MOBILE", "MoBiLe").

**Negated matching:**

```ferron
http * {
    set_var request.header.x_forwarded_for "." has_xff {
        negate
    }
}
```

Ferron sets the variable `has_xff` to `"1"` when the `X-Forwarded-For` header is not present or is empty. This is useful for identifying direct connections versus proxied requests.

### Custom access log fields with `log_field`

**Mapping variables to log fields:**

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

After Ferron generates the response, the access log includes `file_type` and `network` fields with the values resolved from the variables set earlier.

**Interpolated values:**

```ferron
http * {
    log_field custom_header "{{request.header.x_custom}}"
    log_field request_path "{{request.uri.path}}"
}
```

Ferron resolves the interpolated string at log time. It uses the full variable resolution system, including request headers, URI components, and custom variables.

### Using `set_var` with other directives

You can use variables set by `set_var` in any directive that supports interpolation:

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

The `set_var` directive runs after client IP resolution and before URL rewriting and the `map` directive. This means variables set by `set_var` are available for `map` evaluation, `rewrite` patterns, and all downstream pipeline stages.

The `log_field` directive runs during the inverse (post-response) phase. It runs after the content-generating stages (reverse proxy, static file, CGI, and so on) have produced a response.

> [!info]
> For variable mapping based on complex patterns, see [HTTP map](./map.md). For URL rewriting, see [URL rewriting](./rewrite.md).

## Observability

### Trace spans

The variables stage sets the following attributes on its `ferron.stage.variables` span:

| Attribute              | Type | Description                                |
| ---------------------- | ---- | ------------------------------------------ |
| `ferron.variables.set` | int  | Number of variables set during this stage. |
