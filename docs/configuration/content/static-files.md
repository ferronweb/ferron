---
title: "Configuration: static file serving"
description: "Static file serving, directory listings, compression, caching headers, MIME types, and error pages."
---

This page documents directives that configure static file serving, directory listings, compression, caching behavior, and custom error pages for requests resolved to the filesystem (via `root`).

> [!info]
> Static file serving is handled by the `http-static` module. For related features, see [Routing and URL processing](/docs/v3/configuration/routing/url-processing), [HTTP cache](/docs/v3/configuration/content/cache), [HTTP response control](/docs/v3/configuration/routing/response), [URL rewriting](/docs/v3/configuration/routing/rewrite), and [HTTP compression](/docs/v3/configuration/content/compression).

## Directives

### Symlink handling

- `disable_symlinks [bool: boolean | string: "if_not_owner"]`
  - This directive controls whether symbolic links are allowed during file path resolution. When a symlink is encountered while traversing the request path, the behavior depends on this setting:
    - `false` (default): Allow all symlinks without restriction.
    - `true`: Reject all symbolic links with a `403 Forbidden` response. Symlinks are detected during path traversal without following them, mitigating symlink-based escape attacks.
    - `if_not_owner`: Allow symlinks only if owned by the same user as the target file (Unix only; treated as `on` on non-Unix systems).
  - Default: `disable_symlinks false`

> [!warning]
> Symlink-based attacks can bypass directory boundaries. If your `root` directory contains untrusted symlinks or is in a shared hosting environment, enable `disable_symlinks on` to protect against escape attacks.

> [!note]
>
> - Symlink detection uses `symlink_metadata()`, which does not follow the symlink, so no file I/O is performed on the symlink target.
> - When enabled, symlinks are detected at each path component level during traversal, not just at the final target.
> - `if_not_owner` mode is Unix-specific and requires the symlink and target to have the same owner UID.

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    disable_symlinks
}

# Allow symlinks only in a specific virtual host
uploads.example.com {
    root /srv/uploads
    disable_symlinks if_not_owner
}

# Allow symlinks (default, backward compatible)
legacy.example.com {
    root /srv/www/legacy
    disable_symlinks false
}
```

### Index and directory listings

- `index <filename: string>...`
  - This directive specifies one or more filenames to try when a request path resolves to a directory. Files are tried in order; the first existing file replaces the directory path in the file context. Only applies when the resolved path is a directory and no `path_info` is present. Default: `index index.html index.htm index.xhtml`
- `directory_listing [bool: boolean]` (`http-static`)
  - This directive specifies whether auto-generated HTML directory listings are enabled when a request path resolves to a directory and no index file is found. Default: `directory_listing false`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    index index.html index.htm
    directory_listing
}
```

> [!note]
>
> - Only generates a listing if no `index` file was found for the directory.
> - Dotfiles (names starting with `.`) are excluded from the listing, except `.maindesc` which is read as a description.
> - A `.maindesc` file in the directory, if present, is displayed as a `<pre>` block below the file table.

### Caching headers

- `etag [bool: boolean]` (`http-static`)
  - This directive specifies whether ETag generation for static file responses is enabled. ETags are weak ETags (`W/"..."`) generated from an xxHash3 hash of the file path, size, and modification time. Default: `etag true`
- `file_cache_control <value: string>` (`http-static`)
  - This directive specifies the `Cache-Control` response header for all static file responses. The value is passed through as-is. Default: not set

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    etag
    file_cache_control "public, max-age=3600"
}
```

> [!note]
>
> - When compression is used, a suffix is appended to the ETag (e.g. `W/"abc123-br"` for Brotli).
> - `If-None-Match` requests that match the current ETag return `304 Not Modified`.
> - Pre-compressed sidecar files receive their own ETag based on the sidecar file's own metadata.

### MIME types

- `mime_type <extension: string> <mime-type: string>` (`http-static`)
  - This directive maps a file extension (with or without leading dot) to a MIME type. Custom MIME type mappings override the built-in database for matching extensions. Multiple `mime_type` directives can be used to map different extensions. Default: built-in MIME database

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    mime_type ".wasm" "application/wasm"
    mime_type ".webmanifest" "application/manifest+json"
}
```

> [!note]
>
> - If the extension is not found in custom mappings, the built-in database is used as a fallback.
> - If neither custom nor built-in mappings match, the response is sent with no `Content-Type` header.

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

> [!note]
>
> - Only applies when an error response is being generated and no custom response has already been set.
> - The file path is absolute or relative to the current working directory.
> - If the specified error page file does not exist, the directive is skipped and the built-in error page is used.
> - Multiple status codes can be mapped to the same error page in a single directive.

## File metadata and handle reuse

Static file metadata is obtained directly from the validated file handle via `file.metadata().await`, which uses `statx` with `AT_EMPTY_PATH` on Linux. This closes the TOCTOU window between path validation and metadata read — the metadata always reflects the file that was opened, not a concurrent rename target.

A per-thread file descriptor reuse pool is available for future handle reuse optimization. The pool uses a 3-tier eviction strategy:

1. **Preemptive** — bulk removal of all expired handles (TTL-based)
2. **Critical** — single expired handle removal when over capacity
3. **LRU** — oldest handle by insertion time when no expired handles remain

This pool is infrastructure for future use and is not yet exposed as a user-facing configuration.

## Observability

### Metrics

#### Static file serving

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.static.files_served` | Counter | `ferron.compression` (`"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`), `ferron.cache_hit` (`"true"` or `"false"`) | Number of static files served |
| `ferron.static.bytes_sent` | Histogram | `ferron.compression` (`"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`), `ferron.cache_hit` (`"true"` or `"false"`) | Bytes sent for static file responses. Buckets: 1KB, 10KB, 100KB, 1MB, 10MB, 100MB |
| `ferron.static.responses` | Counter | `http.response.status.code` (HTTP response status code), `ferron.static.outcome` (static file serving outcome) | Static-file responses across normal, conditional, range, and error paths |

### Logs

- **`WARN`**: logged when an `error_page` file cannot be opened. The directive is skipped and the built-in error page is used instead.

### Trace spans

The static file stage sets the following attributes on its `ferron.stage.static_file` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `http.response.status_code` | int | HTTP status code of the file response. |
| `ferron.static.file_path` | string | The file path relative to the document root. |
| `ferron.static.precompressed` | bool | Whether a precompressed variant of the file was served. |

## Best practices

The following best-practice check is reported by `ferron doctor` for directives on this page.

- **`directory_listing` enabled** — Auto-generated directory indexes expose file structure. Enable only for intentionally public file listings.
