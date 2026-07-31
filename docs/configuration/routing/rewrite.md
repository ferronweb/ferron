---
title: "Configuration: URL rewriting"
description: "The `rewrite` directive for transforming request URLs using regular expression patterns."
---

This page documents the `rewrite` directive for transforming request URLs using regular expression patterns. Ferron applies rewrites early in the request pipeline, before proxying or static file serving, so routing uses the rewritten URL.

> [!info]
> For `url_sanitize` interaction, see [Routing and URL processing](/docs/v3/configuration/routing/url-processing#url-sanitation-and-redirects). For static file serving, see [Static file serving](/docs/v3/configuration/content/static-files).

## Directives

### `rewrite`

- `rewrite <regex: string> <replacement: string>`
  - This directive specifies a regular expression pattern and replacement string for URL rewriting. You can reference regex capture groups in the replacement string (`$1`, `$2`, etc.). Default: none

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

Ferron internally rewrites all `/old-path/anything` requests to `/new-path/anything`. The client sees no redirect — the rewrite is transparent.

> [!tip]
> If you get unexpected routing behavior, verify that Ferron applies rewrite rules in the order you expect — rules with `last` stop further processing.

#### Stop processing with `last`

```ferron
example.com {
    rewrite "^/api/v1/(.*)" "/api/v2/$1" {
        last
    }
    rewrite "^/api/v2/(.*)" "/api/v3/$1"
}
```

Ferron rewrites `/api/v1/users` requests to `/api/v2/users` and then stops. The second rule never sees the `/api/v2/` prefix.

#### Chained rules without `last`

```ferron
example.com {
    rewrite "^/legacy/(.*)" "/modern/$1"
    rewrite "^/modern/(.*)" "/current/$1"
}
```

Ferron first rewrites a `/legacy/foo` request to `/modern/foo`. Then the second rule rewrites it to `/current/foo`.

#### File/directory-specific rules

```ferron
example.com {
    root /var/www

    rewrite "^/static/(.*)" "/assets/$1" {
        file
        directory false
    }
}
```

### `rewrite_log`

- `rewrite_log <bool>`
  - This directive specifies whether Ferron logs each URL rewrite operation to the error log. Default: `rewrite_log false`

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

When URL sanitization is enabled (the default), Ferron normalizes dangerous path sequences like `/../` before applying rewrite rules. If you need raw URL processing, you can disable URL sanitation with `url_sanitize false` (see [Routing and URL processing](./url-processing.md)).

## Pipeline position

Ferron applies rewrite rules after client IP resolution and before reverse proxying, static file serving, and response generation. This means all later routing decisions use rewritten URLs.

## Observability

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.rewrite.rewrites_applied` | Counter | — | URLs successfully rewritten |
| `ferron.rewrite.invalid` | Counter | — | Rewrite rules that produced an invalid path (resulting in a 400 response) |

### Logs

When `rewrite_log` is enabled, Ferron logs each rewrite operation to the error log at `INFO` level.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| URL rewritten         | INFO  | `ferron.rewrite.from` (string) — original request path + query string, `ferron.rewrite.to` (string) — rewritten path + query string |

### Access log fields

The rewrite module contributes the following field to the HTTP access log line:

| Field | Type | Description |
| --- | --- | --- |
| `ferron.rewrite.applied` | bool | Whether a URL rewrite was applied. |

### Trace spans

The rewrite stage sets the following attributes on its `ferron.stage.rewrite` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `ferron.rewrite.applied` | bool | Whether a rewrite rule was applied to the request. |
| `ferron.rewrite.pattern_count` | int | Number of rewrite rules that matched. |
