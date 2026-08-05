---
title: Configuration: HTTP response body replacement
description: The `replace` directive for string replacement in HTTP response bodies.
---

This page documents the `replace`, `replace_last_modified`, and `replace_filter_types` directives for modifying HTTP response bodies on the fly. Ferron applies string replacement after all content generation (static files, proxy responses, etc.) and before caching. Clients receive the modified content, and the cache stores it.

## Directives

### String replacement

- `replace <search: string> <replacement: string>`
  - This directive specifies a string to search for in the response body and its replacement. You can define multiple `replace` directives. Ferron applies them in order. Default: none

#### Block options

| Option | Arguments | Description                                                                   | Default |
| ------ | --------- | ----------------------------------------------------------------------------- | ------- |
| `once` | `<bool>`  | When `true`, Ferron replaces only the first time the searched string appears. | `false` |

**Configuration example:**

```ferron
example.com {
    # Replace all occurrences
    replace "old-company-name" "new-company-name"

    # Replace only the first occurrence
    replace "http://old-domain.com" "https://new-domain.com" {
        once
    }
}
```

> [!note]
>
> - Ferron applies multiple `replace` directives in order. Later replacements operate on the output of earlier ones.
> - The `once` option defaults to `false` (replace all occurrences). Use `once true` to replace only the first.

#### Simple replacement

```ferron
example.com {
    replace "foo" "bar"
}
```

Ferron replaces every time `foo` appears in the response body with `bar`.

> [!tip]
> If Ferron does not apply replacements, verify that you disabled HTTP compression for the affected responses. For JSON replacement, add `application/json` to `replace_filter_types`.

#### Replace only first occurrence

```ferron
example.com {
    replace "old" "new" {
        once
    }
}
```

Ferron replaces only the first time `old` appears in the response body. Later appearances remain unchanged.

#### Chained replacements

```ferron
example.com {
    replace "foo" "bar"
    replace "bar" "baz"
}
```

Ferron applies the replacements in order. A response body containing `foo and foo` becomes `bar and bar` after the first replacement. It becomes `baz and baz` after the second. Note that the second replacement also affects the output of the first.

### MIME type filtering

- `replace_filter_types <mime-type: string>...`
  - This directive specifies which response MIME types Ferron processes for string replacement. The filter can be a specific MIME type (like `text/html`) or a wildcard (`*`) to process all responses. Default: `replace_filter_types "text/html"`

**Configuration example:**

```ferron
example.com {
    replace_filter_types "text/html" "text/css" "application/javascript"

    replace "old" "new"
}
```

#### Wildcard filter

```ferron
example.com {
    # Process all response types
    replace_filter_types "*"

    replace "footer-old" "footer-new"
}
```

#### Default behavior

When `replace_filter_types` is not configured, Ferron processes only `text/html` responses:

```ferron
example.com {
    # Only text/html responses are modified
    replace "old" "new"
}
```

### Last-Modified header handling

- `replace_last_modified <preserve: bool>`
  - This directive specifies whether Ferron preserves the `Last-Modified` response header when it modifies the body. When `false`, Ferron removes the `Last-Modified` header from responses that undergo replacement. Default: `replace_last_modified false`

**Configuration example:**

```ferron
example.com {
    replace_last_modified

    replace "old" "new"
}
```

## Scoping

You can place the `replace`, `replace_last_modified`, and `replace_filter_types` directives at different configuration levels:

- **Host level**: applies to all requests for that host
- **`location` block**: applies only to requests matching that path prefix
- **`if` / `if_not` blocks**: applies conditionally based on a matcher

```ferron
example.com {
    # Global replacements for all requests
    replace "old-brand" "new-brand"

    location /api {
        # API-specific replacements
        replace_filter_types "application/json"
        replace "v1" "v2"
    }

    location /legacy {
        replace "deprecated" "archived"
        replace_last_modified false
    }
}
```

## HTTP compression interaction

String replacement **requires you to disable HTTP compression** for the affected responses. When a response has a `Content-Encoding` header, the data is already compressed with gzip, brotli, or another algorithm. Ferron skips the replacement to avoid corrupting the compressed data.

If you need to replace strings in responses that gzip, brotli, or another algorithm would compress, you must disable compression:

```ferron
example.com {
    # Disable static file compression
    compressed false

    # Disable dynamic content compression
    dynamic_compressed false

    # Now replacement can work safely
    replace "old" "new"
}
```

> [!note]
> If you enable compression and the algorithm compresses a response, Ferron silently skips the replacement and emits a `ferron.replace.skipped_compressed` metric.

## Pipeline position

The replace stage runs:

- **After** the dynamic compression stage (to make sure the data is not compressed)
- **Before** the HTTP cache stage (so cached content is already replaced)

This ordering makes sure that string replacement operates on raw, uncompressed response bodies. It also makes sure that the modified content is what gets stored in the cache.

## Observability

### Metrics

| Metric                                | Type    | Attributes | Description                                                          |
| ------------------------------------- | ------- | ---------- | -------------------------------------------------------------------- |
| `ferron.replace.replacements_applied` | Counter | None       | Responses successfully modified by replacement rules                 |
| `ferron.replace.skipped_compressed`   | Counter | None       | Responses skipped due to `Content-Encoding` header (compressed data) |
| `ferron.replace.skipped_mime`         | Counter | None       | Responses skipped due to MIME type mismatch                          |

### Trace spans

The response replacement stage sets the following attributes on its `ferron.stage.http_replace` span:

| Attribute                    | Type   | Description                                                                                                  |
| ---------------------------- | ------ | ------------------------------------------------------------------------------------------------------------ |
| `ferron.replace.applied`     | bool   | Whether Ferron applied the replacement.                                                                      |
| `ferron.replace.skip_reason` | string | Reason Ferron skipped the replacement, when applicable (for example, `compressed_body`, `unsupported_mime`). |
