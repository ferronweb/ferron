---
title: "Configuration: static file serving"
description: "Static file serving, directory listings, compression, caching headers, MIME types, and error pages."
---

This page documents directives that configure static file serving, directory listings, compression, caching behavior, and custom error pages. These directives apply to requests that resolve to the filesystem (via `root`).

> [!info]
> The `http-static` module handles static file serving.
> For related features, see [Routing and URL processing](/docs/configuration/routing/url-processing),
> [HTTP cache](/docs/configuration/content/cache), [HTTP response control](/docs/configuration/routing/response),
> [URL rewriting](/docs/configuration/routing/rewrite), [HTTP compression](/docs/configuration/content/compression),
> and [Canary deployments](/docs/configuration/routing/canary).

## Directives

### Web root

- `root <path: string>`
  - This directive specifies the webroot that the HTTP file-handler pipeline uses after regular HTTP stages leave the request without a response. Ferron canonicalizes the resolved path before file stages run. Ferron rejects requests that try to escape the webroot. Default: not configured

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
}
```

> [!note]
> If a request continues below a matched file path, Ferron carries the unmatched suffix into the file-stage context as `path_info` (for PHP, CGI and FastCGI).

### Index and directory listings

- `index <filename: string>...`
  - This directive specifies one or more filenames to try when a request path resolves to a directory. Ferron tries them in order. The first existing file replaces the directory path in the file context. This applies only when the resolved path is a directory and no `path_info` is present. Default: `index index.html index.htm index.xhtml`
- `directory_listing [bool: boolean]` (`http-static`)
  - This directive controls whether Ferron auto-generates an HTML listing when a request path resolves to a directory. Ferron generates a listing only when no index file exists. Default: `directory_listing false`

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
> - Ferron generates a listing only if it finds no `index` file in the directory.
> - The listing excludes dotfiles (names that start with `.`). Ferron reads `.maindesc` as a description.
> - If a `.maindesc` file exists in the directory, Ferron shows it as a `<pre>` block below the file table.

### Caching headers

- `etag [bool: boolean]` (`http-static`)
  - This directive controls whether Ferron generates ETags for static file responses. Ferron uses weak ETags (`W/"..."`) and derives them from an xxHash3 hash of the file path, size, and modification time. Default: `etag true`
- `file_cache_control <value: string>` (`http-static`)
  - This directive specifies the `Cache-Control` response header for all static file responses. Ferron passes the value through as-is. Default: not set

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
> - When compression is active, Ferron appends a suffix to the ETag (for example, `W/"abc123-br"` for Brotli).
> - `If-None-Match` requests that match the current ETag return `304 Not Modified`.
> - Pre-compressed sidecar files receive their own ETag based on their own metadata.

### MIME types

- `mime_type <extension: string> <mime-type: string>` (`http-static`)
  - This directive maps a file extension (with or without leading dot) to a MIME type. Custom MIME type mappings override the built-in database for matching extensions. You can use multiple `mime_type` directives to map different extensions. Default: built-in MIME database

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
> - If custom mappings do not contain the extension, Ferron uses the built-in database as a fallback.
> - If neither mapping matches, Ferron sends the response with no `Content-Type` header.

### Error pages

- `error_page <status-code: integer>... <file-path: string>`
  - This directive maps one or more HTTP status codes to a file path. Ferron serves that file as the error response body. The last argument is always the file path. All preceding arguments are status codes. Default: built-in error pages
- `error_page_placeholders [bool: boolean]`
  - When enabled, Ferron replaces `{{trace.id}}` and `{{trace.spanid}}` in the error page file. It uses the trace ID and span ID of the request. Default: `false`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    error_page 404 /custom/404.html
    error_page 500 502 503 504 /custom/50x.html
    error_page_placeholders true
}
```

> [!note]
>
> - Ferron applies this only when it generates an error response and no custom response exists.
> - The file path is absolute or relative to the current working directory.
> - If the specified error page file does not exist, Ferron skips the directive and uses the built-in error page.
> - You can map multiple status codes to the same error page in a single directive.
> - Placeholder substitution reads the file into memory and replaces the placeholders with the trace context of the request.
> - The zerocopy/sendfile optimization does not run while substitution is active.

### Symlink handling

- `disable_symlinks [bool: boolean | string: "if_not_owner"]`
  - This directive controls whether Ferron allows symbolic links during file path resolution. When the resolver encounters a symlink while traversing the request path, the behavior depends on this setting:
    - `false`: Allow all symlinks without restriction.
    - `true` (default): Reject all symbolic links with a `403 Forbidden` response. The resolver detects symlinks during path traversal without following them, mitigating symlink-based escape attacks.
    - `"if_not_owner"`: Allow symlinks when the same user owns the link and the target file. On non-Unix systems, Ferron treats this value as `true`.
  - Default: `disable_symlinks true`

> [!warning]
> Symlink-based attacks can bypass directory boundaries. If the `root` directory contains untrusted symlinks, enable `disable_symlinks true`. If you run a shared hosting environment, enable it there as well.

> [!important]
> `disable_symlinks true` is the default. When a request path crosses a symlink, Ferron returns `403 Forbidden`. The error page does not name the symlink. If static files return 403 after you add symlinks, check for links in every path component with `ls -la`, then set `disable_symlinks false` for trusted content or `disable_symlinks if_not_owner` when link and target share an owner.

> [!note]
>
> - Symlink detection uses `symlink_metadata()`, which does not follow the symlink. It does no file I/O on the symlink target.
> - When enabled, the resolver detects symlinks at each path component level during traversal, not just at the final target.
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

# Allow symlinks (default)
legacy.example.com {
    root /srv/www/legacy
    disable_symlinks false
}
```

## File handle reuse

Ferron reuses file handles (and I/O errors) for static file responses to reduce file I/O overhead. The reuse lasts at most 200 milliseconds after the first request.

## Observability

### Metrics

#### Static file serving

| Metric                       | Type      | Attributes                                                                                                               | Description                                                                       |
| ---------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| `ferron.static.files_served` | Counter   | `ferron.compression` (`"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`), `ferron.cache_hit` (`"true"` or `"false"`) | Number of static files served                                                     |
| `ferron.static.bytes_sent`   | Histogram | `ferron.compression` (`"identity"`, `"gzip"`, `"br"`, `"deflate"`, `"zstd"`), `ferron.cache_hit` (`"true"` or `"false"`) | Bytes sent for static file responses. Buckets: 1KB, 10KB, 100KB, 1MB, 10MB, 100MB |
| `ferron.static.responses`    | Counter   | `http.response.status.code` (HTTP response status code), `ferron.static.outcome` (static file serving outcome)           | Static-file responses across normal, conditional, range, and error paths          |

### Access log fields

The static file serving module and the file resolution stage contribute the following fields to the HTTP access log line:

| Field                                     | Type   | Description                                                                    |
| ----------------------------------------- | ------ | ------------------------------------------------------------------------------ |
| `ferron.static.file_path`                 | string | Absolute file path served.                                                     |
| `ferron.static.file_path_precompressed`   | string | The precompressed file path (if applicable).                                   |
| `ferron.static.dir_path`                  | string | Directory path when Ferron serves a listing.                                   |
| `ferron.file_resolve.request_path`        | string | Decoded request path that Ferron resolves (error paths only).                  |
| `ferron.file_resolve.root_path`           | string | Configured document root (error paths only).                                   |
| `ferron.file_resolve.outcome`             | string | Resolution outcome: `forbidden`, `bad_request`, or `error` (error paths only). |
| `ferron.file_resolve.last_candidate_path` | string | Last filesystem path attempted before failure (error paths only).              |

### Trace spans

The file resolution span (`ferron.pipeline.file_resolve`) captures the resolution process before any file-serving stage runs:

| Attribute                                 | Type   | Description                                                      |
| ----------------------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.file_resolve.request_path`        | string | The decoded request URI path.                                    |
| `ferron.file_resolve.root_path`           | string | The configured document root.                                    |
| `ferron.file_resolve.outcome`             | string | `resolved`, `not_found`, `forbidden`, `bad_request`, or `error`. |
| `ferron.file_resolve.resolved_path`       | string | The resolved filesystem path (success only).                     |
| `ferron.file_resolve.last_candidate_path` | string | The last path attempted before failure (error only).             |

The static file stage sets the following attributes on its `ferron.stage.static_file` span:

| Attribute                               | Type   | Description                                                |
| --------------------------------------- | ------ | ---------------------------------------------------------- |
| `http.response.status_code`             | int    | HTTP status code of the file response.                     |
| `ferron.static.file_path`               | string | The file path relative to the document root.               |
| `ferron.static.file_path_precompressed` | string | The precompressed file path (if applicable).               |
| `ferron.static.precompressed`           | bool   | Whether Ferron served a precompressed variant of the file. |

The directory listing stage sets the following attributes on its `ferron.stage.directory_listing` span:

| Attribute                   | Type   | Description                           |
| --------------------------- | ------ | ------------------------------------- |
| `http.response.status_code` | int    | HTTP status code of the response.     |
| `ferron.static.dir_path`    | string | The directory path that Ferron lists. |

## Best practices

`ferron doctor` reports the following best-practice check for the directives on this page.

- **`directory_listing` enabled**: Auto-generated directory indexes expose file structure. Enable only for intentionally public file listings.
