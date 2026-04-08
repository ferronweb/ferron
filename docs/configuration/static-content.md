---
title: "Configuration: static file serving"
description: "Static file serving, directory listings, compression, caching headers, MIME types, and error pages."
---

This page documents directives that configure static file serving, directory listings, compression, caching behavior, and custom error pages for requests resolved to the filesystem (via `root`).

## Directives

### Index and directory listings

- `index <filename: string>...`
  - This directive specifies one or more filenames to try when a request path resolves to a directory. Files are tried in order; the first existing file replaces the directory path in the file context. Only applies when the resolved path is a directory and no `path_info` is present. Default: `index index.html index.htm index.xhtml`
- `directory_listing [bool: boolean]` (`ferron-http-static`)
  - This directive specifies whether auto-generated HTML directory listings are enabled when a request path resolves to a directory and no index file is found. When omitted, defaults to `true`. Default: `directory_listing false`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    index index.html index.htm
    directory_listing
}
```

Notes:

- Only generates a listing if no `index` file was found for the directory.
- Dotfiles (names starting with `.`) are excluded from the listing, except `.maindesc` which is read as a description.
- A `.maindesc` file in the directory, if present, is displayed as a `<pre>` block below the file table.

### Compression

- `compressed [bool: boolean]` (`ferron-http-static`)
  - This directive specifies whether on-the-fly response body compression is enabled based on the `Accept-Encoding` request header. Supported algorithms: `gzip`, `brotli`, `deflate`, `zstd`. When omitted, defaults to `true`. Default: `compressed true`
- `precompressed [bool: boolean]` (`ferron-http-static`)
  - This directive specifies whether serving pre-compressed sidecar files (e.g. `style.css.gz`, `app.js.br`) instead of compressing on the fly is enabled. When omitted, defaults to `true`. Default: `precompressed false`
- `dynamic_compressed [bool: boolean]` (`ferron-http-static`)
  - This directive specifies whether on-the-fly compression is enabled for dynamic (non-static) response bodies, such as responses from reverse proxies or application handlers. Supported algorithms: `gzip`, `brotli`, `deflate`, `zstd`. Default: `dynamic_compressed false`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    compressed
    precompressed
}
```

Notes:

- `compressed`: applied to files larger than 256 bytes with compressible extensions. A `Vary: Accept-Encoding` header is added when compression is possible.
- `precompressed`: when enabled, the server checks for a pre-compressed file alongside the original based on the client's `Accept-Encoding` preference.
- `dynamic_compressed`: compression is only applied to responses with compressible MIME types. A suffix is appended to the ETag (e.g. `W/"abc123-dynamic-br"`) to distinguish compressed variants.

### Caching headers

- `etag [bool: boolean]` (`ferron-http-static`)
  - This directive specifies whether ETag generation for static file responses is enabled. ETags are weak ETags (`W/"..."`) generated from an xxHash3 hash of the file path, size, and modification time. When omitted, defaults to `true`. Default: `etag true`
- `file_cache_control <value: string>` (`ferron-http-static`)
  - This directive specifies the `Cache-Control` response header for all static file responses. The value is passed through as-is. Default: not set

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    etag
    file_cache_control "public, max-age=3600"
}
```

Notes:

- When compression is used, a suffix is appended to the ETag (e.g. `W/"abc123-br"` for Brotli).
- `If-None-Match` requests that match the current ETag return `304 Not Modified`.
- Pre-compressed sidecar files receive their own ETag based on the sidecar file's own metadata.

### MIME types

- `mime_type <extension: string> <mime-type: string>` (`ferron-http-static`)
  - This directive maps a file extension (with or without leading dot) to a MIME type. Custom MIME type mappings override the built-in database for matching extensions. Multiple `mime_type` directives can be used to map different extensions. Default: built-in MIME database

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    mime_type .wasm application/wasm
    mime_type .webmanifest application/manifest+json
}
```

Notes:

- If the extension is not found in custom mappings, the built-in database is used as a fallback.
- If neither custom nor built-in mappings match, the response is sent with no `Content-Type` header.

### Error pages

- `error_page <status-code: integer>... <file-path: string>`
  - This directive specifies one or more HTTP status codes followed by a file path to serve as the error response body. The last argument is always the file path; all preceding arguments are status codes. Default: built-in error pages

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    error_page 404 /custom/404.html
    error_page 500 502 503 504 /custom/50x.html
}
```

Notes:

- Only applies when an error response is being generated and no custom response has already been set.
- The file path is absolute or relative to the current working directory.
- If the specified error page file does not exist, the directive is skipped and the built-in error page is used.
- Multiple status codes can be mapped to the same error page in a single directive.

## Observability

### Metrics

#### Static file serving

- `ferron.static.files_served` (Counter) — number of static files served.
  - Attributes: `ferron.compression` (`"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`), `ferron.cache_hit` (`"true"` or `"false"`)
- `ferron.static.bytes_sent` (Histogram) — bytes sent for static file responses. Buckets: 1KB, 10KB, 100KB, 1MB, 10MB, 100MB.
  - Attributes: same as above

### Logs

- **`WARN`**: logged when an `error_page` file cannot be opened. The directive is skipped and the built-in error page is used instead.

## Notes and troubleshooting

- The `root` directive is defined in [Routing and URL processing](/docs/v3/configuration/routing-url-processing).
- For `trailing_slash_redirect`, see [Routing and URL processing](/docs/v3/configuration/routing-url-processing#url-sanitation-and-redirects).
- For response control (`status`, `abort`), see [HTTP response control](/docs/v3/configuration/http-response).
- For URL rewriting, see [URL rewriting](/docs/v3/configuration/http-rewrite).
