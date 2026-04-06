# URL Rewriting

The URL rewriting module provides the `rewrite` directive for transforming request URLs using regular expression patterns. Rewrites are applied early in the request pipeline, before proxying or static file serving, so the rewritten URL is used for routing.

## Overview

- Rewrite request URLs using regular expression patterns
- Capture groups in the regex can be referenced in the replacement string (`$1`, `$2`, etc.)
- Chain multiple rules — each rule's output becomes the next rule's input
- Control whether rules apply to files, directories, or both
- Mark rules as `last` to stop further rewriting
- Optional logging of all rewrite operations

## `rewrite`

Syntax:

```ferron
example.com {
    rewrite "<regex>" "<replacement>"
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<regex>` | Regular expression to match against the request URL (path + query string) | — |
| `<replacement>` | Replacement string; capture groups referenced as `$1`, `$2`, etc. | — |

### Block options

The `rewrite` directive supports an optional block for additional configuration:

| Option | Arguments | Description | Default |
| --- | --- | --- | --- |
| `last` | `<bool>` | When `true`, stop processing further rewrite rules after this one matches | `false` |
| `directory` | `<bool>` | When `true`, apply this rule when the URL corresponds to a directory | `true` |
| `file` | `<bool>` | When `true`, apply this rule when the URL corresponds to a file | `true` |
| `allow_double_slashes` | `<bool>` | When `true`, preserve double slashes (`//`) in the URL instead of collapsing them | `false` |

### Examples

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

A request to `/legacy/foo` is first rewritten to `/modern/foo`, then the second rule rewrites it to `/current/foo`. Both rules are applied in sequence.

#### File/directory-specific rules

```ferron
example.com {
    root "/var/www"

    # Only rewrite URLs that correspond to files
    rewrite "^/static/(.*)" "/assets/$1" {
        file true
        directory false
    }
}
```

#### Preserve double slashes

```ferron
example.com {
    rewrite "^/special//(.*)" "/normalized/$1" {
        allow_double_slashes true
    }
}
```

## `rewrite_log`

Syntax:

```ferron
example.com {
    rewrite_log true
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<bool>` | When `true`, log each URL rewrite operation to the error log | `false` |

When enabled, each rewrite operation is logged with the original and rewritten URLs:

```
URL rewritten from "/old-path/users" to "/new-path/users"
```

## Regex syntax

The regular expression engine used is [`fancy-regex`](https://crates.io/crates/fancy-regex), which supports most PCRE-like features including lookahead, lookbehind, and non-capturing groups. The matching is case-insensitive on Windows and case-sensitive on other platforms.

## URL sanitation interaction

When URL sanitization is enabled (the default), dangerous path sequences like `../` are normalized before rewrite rules are applied. If you need raw URL processing, you can disable URL sanitation with `url_sanitize false` (see [HTTP Control Directives](./http-control.md)).

## Pipeline position

Rewrite rules are applied after client IP resolution and before reverse proxying, static file serving, and response generation. This means rewritten URLs are used for all subsequent routing decisions.
