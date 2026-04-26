---
title: "Configuration: URL rewriting"
description: "The `rewrite` directive for transforming request URLs using regular expression patterns."
---

This page documents the `rewrite` directive for transforming request URLs using regular expression patterns. Rewrites are applied early in the request pipeline, before proxying or static file serving, so the rewritten URL is used for routing.

## Directives

### `rewrite`

- `rewrite <regex: string> <replacement: string>`
  - This directive specifies a regular expression pattern and replacement string for URL rewriting. Capture groups in the regex can be referenced in the replacement string (`$1`, `$2`, etc.). Default: none

#### Block options

| Option | Arguments | Description | Default |
| --- | --- | --- | --- |
| `last` | `<bool>` | When `true`, stop processing further rewrite rules after this one matches. | `false` |
| `directory` | `<bool>` | When `true`, apply this rule when the URL corresponds to a directory. | `true` |
| `file` | `<bool>` | When `true`, apply this rule when the URL corresponds to a file. | `true` |
| `allow_double_slashes` | `<bool>` | When `true`, preserve double slashes (`//`) in the URL instead of collapsing them. | `false` |

**Configuration example:**

```ferron
example.com {
    rewrite "^/old-path/(.*)" "/new-path/$1"
}
```

#### Simple rewrite

```ferron
example.com {
    rewrite "^/old-path/(.*)" "/new-path/$1"
}
```

All requests to `/old-path/anything` are internally rewritten to `/new-path/anything`. The client sees no redirect — the rewrite is transparent.

#### Stop processing with `last`

```ferron
example.com {
    rewrite "^/api/v1/(.*)" "/api/v2/$1" {
        last true
    }
    rewrite "^/api/v2/(.*)" "/api/v3/$1"
}
```

Requests to `/api/v1/users` are rewritten to `/api/v2/users` and then stop — the second rule never sees the `/api/v2/` prefix.

#### Chained rules without `last`

```ferron
example.com {
    rewrite "^/legacy/(.*)" "/modern/$1"
    rewrite "^/modern/(.*)" "/current/$1"
}
```

A request to `/legacy/foo` is first rewritten to `/modern/foo`, then the second rule rewrites it to `/current/foo`.

#### File/directory-specific rules

```ferron
example.com {
    root /var/www

    rewrite "^/static/(.*)" "/assets/$1" {
        file true
        directory false
    }
}
```

### `rewrite_log`

- `rewrite_log <bool>`
  - This directive specifies whether each URL rewrite operation is logged to the error log. Default: `rewrite_log false`

**Configuration example:**

```ferron
example.com {
    rewrite_log true
}
```

Example log output:

```text
URL rewritten from "/old-path/users" to "/new-path/users"
```

## Regex syntax

The regular expression engine used is [`fancy-regex`](https://crates.io/crates/fancy-regex), which supports most PCRE-like features including lookahead, lookbehind, and non-capturing groups. The matching is case-insensitive on Windows and case-sensitive on other platforms.

## URL sanitation interaction

When URL sanitization is enabled (the default), dangerous path sequences like `./docs/v3/configuration/` are normalized before rewrite rules are applied. If you need raw URL processing, you can disable URL sanitation with `url_sanitize false` (see [Routing and URL processing](./routing-url-processing)).

## Pipeline position

Rewrite rules are applied after client IP resolution and before reverse proxying, static file serving, and response generation. This means rewritten URLs are used for all subsequent routing decisions.

## Observability

### Metrics

- `ferron.rewrite.rewrites_applied` (Counter) — URLs successfully rewritten.
- `ferron.rewrite.invalid` (Counter) — rewrite rules that produced an invalid path (resulting in a 400 response).

### Logs

When `rewrite_log` is enabled, each rewrite operation is logged to the error log at `INFO` level.

## Notes and troubleshooting

- If you get unexpected routing behavior, verify that rewrite rules are applied in the order you expect. Rules with `last true` stop further processing.
- For `url_sanitize` interaction, see [Routing and URL processing](/docs/v3/configuration/routing-url-processing#url-sanitation-and-redirects).
- For static file serving, see [Static file serving](/docs/v3/configuration/static-content).
