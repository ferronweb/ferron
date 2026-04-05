# Conditionals And Variables

Named matchers are declared with `match <name> { ... }` and referenced by `if <name> { ... }` or `if_not <name> { ... }`.

Example:

```ferron
match curl_client {
    request.header.user_agent ~ "curl"
}

example.com {
    if curl_client {
    }
}
```

Language matching example:

```ferron
match english_language {
    "en" in request.header.accept_language
}

example.com {
    if english_language {
        root "/var/www/english"
    }
}
```

## Operators

Current matcher operators:

| Operator | Meaning in current code |
| --- | --- |
| `==` | String equality |
| `!=` | String inequality |
| `~` | Substring match |
| `!~` | Negated substring match |
| `in` | Left value must equal one of the comma-separated items in the right value, or match a language in an `Accept-Language` header |

Notes:

- `~` and `!~` are not regular expressions yet. The resolver currently uses substring matching.
- `in` splits the right-hand string on commas and trims each item.
- When the right value looks like an `Accept-Language` header (contains quality values or multiple language ranges), `in` performs language matching with support for base language codes (e.g., `en` matches `en-US`).
- All expressions inside a single `match` block must pass.

## Built-In Matcher Variables

The current HTTP resolver exposes these names:

| Variable | Value |
| --- | --- |
| `request.method` | HTTP method |
| `request.uri.path` | Request path |
| `request.uri.query` | Query string, or empty string |
| `request.uri` | Full request URI |
| `request.version` | HTTP version string such as `HTTP/1.1` |
| `request.header.<name>` | Request header value |
| `request.host` | Resolved request hostname |
| `request.scheme` | `http` or `https` |
| `server.ip` | Local listener IP address |

Header names are normalized by lowercasing them and converting `_` to `-`. For example, `request.header.x_forwarded_for` reads the `x-forwarded-for` header.

## Interpolated Strings

Interpolated strings use `{{name}}`.

Current behavior:

- `{{env.NAME}}` reads the `NAME` environment variable.
- Other interpolation variables depend on the consumer of that directive.
- If a variable cannot be resolved, the placeholder is kept as `{{name}}`.

For startup-only TLS settings such as `cert` and `key`, the bundled manual TLS provider effectively relies on plain strings or `env.*` interpolation.

## Related Directives

- [`if`](./http-control.md#if)
- [`if_not`](./http-control.md#if_not)
