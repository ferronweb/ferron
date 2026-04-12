---
title: "Configuration: HTTP response body replacement"
description: "The `replace` directive for string replacement in HTTP response bodies."
---

This page documents the `replace`, `replace_last_modified`, and `replace_filter_types` directives for modifying HTTP response bodies on the fly. String replacement is applied after all content generation (static files, proxy responses, etc.) and before caching, so the modified content is what clients receive and what gets cached.

## Directives

### String replacement

- `replace <search: string> <replacement: string>`
  - This directive specifies a string to search for in the response body and its replacement. Multiple `replace` directives can be defined and are applied in order. Default: none

#### Block options

| Option | Arguments | Description | Default |
| --- | --- | --- | --- |
| `once` | `<bool>` | When `true`, only the first occurrence of the searched string is replaced. | `false` |

**Configuration example:**

```ferron
example.com {
    # Replace all occurrences
    replace "old-company-name" "new-company-name"
    
    # Replace only the first occurrence
    replace "http://old-domain.com" "https://new-domain.com" {
        once true
    }
}
```

#### Simple replacement

```ferron
example.com {
    replace "foo" "bar"
}
```

All occurrences of `foo` in the response body are replaced with `bar`.

#### Replace only first occurrence

```ferron
example.com {
    replace "old" "new" {
        once true
    }
}
```

Only the first occurrence of `old` in the response body is replaced. Subsequent occurrences remain unchanged.

#### Chained replacements

```ferron
example.com {
    replace "foo" "bar"
    replace "bar" "baz"
}
```

The replacements are applied in order. A response body containing `foo and foo` becomes `bar and bar` after the first replacement, then `baz and baz` after the second. Note that the second replacement also affects the output of the first.

### MIME type filtering

- `replace_filter_types <mime-type: string>...`
  - This directive specifies which response MIME types should be processed for string replacement. The filter can be a specific MIME type (like `text/html`) or a wildcard (`*`) to process all responses. Default: `replace_filter_types "text/html"`

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

When `replace_filter_types` is not configured, only `text/html` responses are processed:

```ferron
example.com {
    # Only text/html responses are modified
    replace "old" "new"
}
```

### Last-Modified header handling

- `replace_last_modified <preserve: bool>`
  - This directive specifies whether the `Last-Modified` response header is preserved when the body is modified. When `false`, the `Last-Modified` header is removed from responses that undergo replacement. Default: `replace_last_modified false`

**Configuration example:**

```ferron
example.com {
    replace_last_modified true
    
    replace "old" "new"
}
```

## Scoping

The `replace`, `replace_last_modified`, and `replace_filter_types` directives can be placed at different configuration levels:

- **Host level** — applies to all requests for that host
- **`location` block** — applies only to requests matching that path prefix
- **`if` / `if_not` blocks** — applies conditionally based on a matcher

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

String replacement **requires HTTP compression to be disabled** for the affected responses. When a response has a `Content-Encoding` header (indicating it is compressed with gzip, brotli, etc.), the replacement is skipped to avoid corrupting the compressed data.

If you need to replace strings in responses that would otherwise be compressed, you must disable compression:

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

> **Note:** If compression is enabled and a response is compressed, the replacement is silently skipped and a `ferron.replace.skipped_compressed` metric is emitted.

## Pipeline position

The replace stage runs:

- **After** the dynamic compression stage (to ensure uncompressed data)
- **Before** the HTTP cache stage (so cached content is already replaced)

This ordering ensures that string replacement operates on raw, uncompressed response bodies and that the modified content is what gets stored in the cache.

## Observability

### Metrics

- `ferron.replace.replacements_applied` (Counter) — responses successfully modified by replacement rules.
- `ferron.replace.skipped_compressed` (Counter) — responses skipped due to `Content-Encoding` header (compressed data).
- `ferron.replace.skipped_mime` (Counter) — responses skipped due to MIME type mismatch.

## Notes and troubleshooting

- If replacements are not being applied, verify that HTTP compression is disabled for the affected responses. Compressed responses are always skipped.
- If you need to replace strings in JSON responses, add `application/json` to `replace_filter_types`.
- Multiple `replace` directives are applied in order — later replacements operate on the output of earlier ones.
- The `once` option defaults to `false` (replace all occurrences). Use `once true` to replace only the first occurrence.
- For compression configuration, see [Static content](/docs/v3/configuration/static-content).
- For caching interaction, see [HTTP cache](/docs/v3/configuration/http-cache).
