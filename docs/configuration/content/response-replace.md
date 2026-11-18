---
title: "Configuration: HTTP response body replacement"
description: "Replace strings in HTTP response bodies before the response reaches the client or the cache."
---

`replace` rewrites strings in HTTP response bodies. Ferron applies replacements after content generation, from static files or a proxy, and before caching. Clients receive the modified body, and the cache stores it.

## `replace`

`replace <search: string> <replacement: string>` replaces every occurrence of `search` with `replacement`. You can use several `replace` directives. Ferron applies them in order, and each one sees the output of the previous ones. Default: none.

```ferron
example.com {
    # Replace all occurrences
    replace "old-company-name" "new-company-name"

    # Replace only the first occurrence
    replace http://old-domain.com https://new-domain.com {
        once
    }
}
```

The `once` block option replaces only the first match. Default: `false`.

| Option | Arguments | Description                                      | Default |
| ------ | --------- | ------------------------------------------------ | ------- |
| `once` | `<bool>`  | Replace only the first occurrence of the string. | `false` |

A chained pair such as `replace "foo" "bar"` followed by `replace "bar" "baz"` turns `foo and foo` into `baz and baz`. The second rule also changes the output of the first.

## `replace_filter_types`

`replace_filter_types <mime-type: string>...` limits replacement to the listed response MIME types. A value can be a specific type such as `text/html`, or `*` to process every response. Default: `replace_filter_types "text/html"`.

```ferron
example.com {
    replace_filter_types "text/html" "text/css" "application/javascript"

    replace "old" "new"
}
```

With no `replace_filter_types`, Ferron modifies only `text/html` responses. To process every type, use `*`:

```ferron
example.com {
    replace_filter_types "*"

    replace "footer-old" "footer-new"
}
```

If replacements do not appear, check that compression is off for those responses. For JSON, add `application/json` to `replace_filter_types`.

## `replace_last_modified`

`replace_last_modified <preserve: bool>` controls the `Last-Modified` header on responses that Ferron modifies. When `false`, Ferron removes the header from modified responses. Default: `replace_last_modified false`.

```ferron
example.com {
    replace_last_modified

    replace "old" "new"
}
```

A bare `replace_last_modified` sets it to `true` and keeps the header.

## HTTP compression interaction

Replacement needs uncompressed data. When a response has a `Content-Encoding` header, the body is already compressed. Ferron skips the replacement to avoid corrupting the data.

To replace strings in responses that would be compressed, turn compression off:

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

If compression remains on and compresses a response, Ferron skips the replacement and increments `ferron.replace.skipped_compressed`.

## Pipeline position

The replace stage runs after dynamic compression, so the body is not compressed, and before the HTTP cache, so the cache stores the modified body. Replacement therefore operates on raw response bodies, and clients and the cache see the same modified content.

## Observability

### Metrics

| Metric                                | Type    | Attributes | Description                                                          |
| ------------------------------------- | ------- | ---------- | -------------------------------------------------------------------- |
| `ferron.replace.replacements_applied` | Counter | none       | Responses modified by replacement rules.                            |
| `ferron.replace.skipped_compressed`   | Counter | none       | Responses skipped because of a `Content-Encoding` header.           |
| `ferron.replace.skipped_mime`         | Counter | none       | Responses skipped because the MIME type did not match.              |

### Trace spans

The replacement stage sets these attributes on its `ferron.stage.http_replace` span.

| Attribute                    | Type   | Description                                                                                                  |
| ---------------------------- | ------ | ------------------------------------------------------------------------------------------------------------ |
| `ferron.replace.applied`     | bool   | Whether Ferron applied the replacement.                                                                      |
| `ferron.replace.skip_reason` | string | Why Ferron skipped the replacement, such as `compressed_body` or `unsupported_mime`.                         |
