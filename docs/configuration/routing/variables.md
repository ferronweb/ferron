---
title: "Configuration: Variable setting"
description: "The `set_var` and `log_field` directives for setting variables based on request conditions and custom access log fields."
---

This page documents the `set_var` and `log_field` directives, which set interpolation variables based on request conditions and map variables to custom access log fields.

## Directives

### `set_var`

- `set_var <source: string> <regex: string> <variable: string>` (`http-variables`)
  - Sets a variable when the source value matches the regular expression. By default, the variable is set to `"1"` on match. Default: none

> [!note]
> The `set_var` directive is similar to Apache's `SetEnvIf` — it evaluates a regex against a resolved variable and conditionally sets a new variable. Multiple `set_var` directives can target the same variable; they are evaluated in declaration order with last-match-wins semantics.

#### Block sub-directives

| Sub-directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `value` | `<string>` | The value to assign when the pattern matches. | `"1"` |
| `case_insensitive` | `<bool>` | When `true`, the regex match is performed case-insensitively. | `false` |
| `negate` | `<bool>` | When `true`, the variable is set when the pattern does **not** match. | `false` |

**Configuration example:**

```ferron
example.com {
    set_var request.uri.path "\.pdf$" is_pdf
    set_var remote.ip "^192\.168\." network_type {
        value "private"
    }
}
```

### `log_field`

- `log_field <field: string> <source: string>` (`http-variables`)
  - Maps a variable or interpolated value to a custom access log field. The field is evaluated after the response is generated, so response-time variables are available. Default: none

> [!note]
> The source can be a plain variable name (e.g., `network_type`) or an interpolated string (e.g., `"{{request.header.x_custom_header}}"`). Plain variable names are resolved via the `Variables` trait at runtime.

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
    set_var request.uri.path "\.pdf$" is_pdf
    set_var request.uri.path "\.(jpg|png|gif)$" is_image
    set_var request.method "^POST$" is_post
}
```

Requests to `/document.pdf` set `is_pdf` to `"1"`, requests to `/photo.jpg` set `is_image` to `"1"`, and POST requests set `is_post` to `"1"`. These variables are then available for interpolation in downstream directives.

**Custom values:**

```ferron
http * {
    set_var request.uri.path "\.pdf$" file_type {
        value "pdf"
    }
    set_var request.uri.path "\.txt$" file_type {
        value "text"
    }
}
```

**Case-insensitive matching:**

```ferron
http * {
    set_var request.header.user_agent "mobile" is_mobile {
        case_insensitive true
    }
}
```

This matches user agents containing "mobile" regardless of capitalization (e.g., "Mobile", "MOBILE", "MoBiLe").

**Negated matching:**

```ferron
http * {
    set_var request.header.x_forwarded_for "." has_xff {
        negate true
    }
}
```

The variable `has_xff` is set to `"1"` when the `X-Forwarded-For` header is **not** present or is empty. This is useful for identifying direct connections versus proxied requests.

### Custom access log fields with `log_field`

**Mapping variables to log fields:**

```ferron
http * {
    set_var request.uri.path "\.pdf$" is_pdf
    set_var remote.ip "^192\.168\." network_type {
        value "private"
    }

    log_field file_type is_pdf
    log_field network network_type
}
```

After the response is generated, the access log will include `file_type` and `network` fields with the values resolved from the variables set earlier.

**Interpolated values:**

```ferron
http * {
    log_field custom_header "{{request.header.x_custom}}"
    log_field request_path "{{request.uri.path}}"
}
```

The interpolated string is resolved at log time using the full variable resolution system, including request headers, URI components, and custom variables.

### Using `set_var` with other directives

Variables set by `set_var` can be used in any directive that supports interpolation:

```ferron
http * {
    set_var remote.ip "^10\." is_internal {
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

The `log_field` directive runs during the inverse (post-response) phase, after the content-generating stages (reverse proxy, static file, CGI, etc.) have produced a response.

> [!info]
> For variable mapping based on complex patterns, see [HTTP map](./map.md). For URL rewriting, see [URL rewriting](./rewrite.md).

## Observability

### Trace spans

The variables stage sets the following attributes on its `ferron.stage.variables` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `ferron.variables.set` | int | Number of variables set during this stage. |
