# Static File Serving Directives

These directives configure static file serving, directory listings, compression, and caching behavior for requests resolved to the filesystem (via `root`). They are consumed by the HTTP static file serving pipeline stages that operate on `HttpFileContext`.

## Categories

- Index resolution: `index`
- Directory listings: `directory_listing`
- Compression: `compressed`, `precompressed`
- Caching headers: `etag`, `file_cache_control`
- MIME types: `mime_type`

See also:

- [HTTP Control Directives](./http-control.md) (where `root` is defined)

## `index`

Syntax:

```ferron
example.com {
    root /srv/www/example
    index index.html index.htm
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>...` | One or more filenames to try when a request path resolves to a directory. Files are tried in order; the first existing file replaces the directory path in the file context. | `index.html index.htm index.xhtml` |

Notes:

- Only applies when the resolved path is a directory and no `path_info` is present.
- If none of the listed files exist, the request falls through to the next stage (which may generate a directory listing or return 403).
- The resolved index file is canonicalized and checked for path traversal before use.

## `directory_listing`

Syntax:

```ferron
example.com {
    root /srv/www/example
    directory_listing on
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<boolean>` | Enables or disables auto-generated HTML directory listings when a request path resolves to a directory and no index file is found. | `off` |

Notes:

- Only generates a listing if no `index` file was found for the directory.
- The listing is generated as an HTML page with a table showing filenames, sizes, and modification dates.
- Dotfiles (names starting with `.`) are excluded from the listing, except `.maindesc` which is read as a description.
- A `.maindesc` file in the directory, if present, is displayed as a `<pre>` block below the file table.
- The generated page uses the built-in CSS stylesheets (`common.css` and `directory.css`).

## `compressed`

Syntax:

```ferron
example.com {
    root /srv/www/example
    compressed on
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<boolean>` | Enables or disables on-the-fly response body compression based on the `Accept-Encoding` request header. | `on` |

Notes:

- Supported algorithms: `gzip`, `brotli`, `deflate`, `zstd`.
- Compression is only applied to files larger than 256 bytes and with compressible extensions (a built-in deny list of already-compressed formats like `.zip`, `.jpg`, `.mp4`, etc.).
- A `Vary: Accept-Encoding` header is added when compression is possible.
- Known broken clients (Netscape 4.x, w3m) are detected via `User-Agent` and compression is skipped for them.

## `precompressed`

Syntax:

```ferron
example.com {
    root /srv/www/example
    precompressed on
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<boolean>` | Enables serving pre-compressed sidecar files (e.g., `style.css.gz`, `app.js.br`) instead of compressing on the fly. | `off` |

Notes:

- When enabled, the server checks for a pre-compressed file alongside the original (e.g., `index.html.gz` for `index.html`) based on the client's `Accept-Encoding` preference.
- If a matching sidecar file exists, it is served directly with the appropriate `Content-Encoding` header.
- This avoids CPU overhead from on-the-fly compression for static assets.
- If no pre-compressed variant is found, the original file is served uncompressed (or on-the-fly compressed if `compressed` is also enabled).

## `etag`

Syntax:

```ferron
example.com {
    root /srv/www/example
    etag on
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<boolean>` | Enables or disables ETag generation for static file responses. | `on` |

Notes:

- ETags are weak ETags (`W/"..."`) generated from an xxHash3 hash of the file path, size, and modification time.
- When compression is used, a suffix is appended to the ETag (e.g., `W/"abc123-br"` for Brotli, `W/"abc123-gzip"` for Gzip).
- `If-None-Match` requests that match the current ETag return `304 Not Modified`.
- `If-Match` is acknowledged but Ferron only produces weak ETags, so strong validator comparisons are not possible.
- Pre-compressed sidecar files receive their own ETag (based on the sidecar file's own metadata).

## `file_cache_control`

Syntax:

```ferron
example.com {
    root /srv/www/example
    file_cache_control "public, max-age=3600"
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` | Sets the `Cache-Control` response header for all static file responses. | not set |

Notes:

- The value is passed through as-is to the `Cache-Control` header.
- Also applied to `304 Not Modified` responses when ETags match.
- Useful for setting browser caching policies for static assets.

## `mime_type`

Syntax:

```ferron
example.com {
    root /srv/www/example
    mime_type .wasm application/wasm
    mime_type .webmanifest application/manifest+json
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` `<string>` | Maps a file extension (with or without leading dot) to a MIME type. The first argument is the extension, the second is the MIME type string. | built-in MIME database |

Notes:

- Custom MIME type mappings override the built-in `new_mime_guess` database for matching extensions.
- Multiple `mime_type` directives can be used to map different extensions.
- If the extension is not found in custom mappings, the built-in database is used as a fallback.
- If neither custom nor built-in mappings match, the response is sent with no `Content-Type` header.
