---
title: "Configuration: static file serving"
description: "Serve files from disk with directory listings, cache headers, MIME types, error pages, and symlink rules."
---

Static file serving answers requests that resolve to a file under `root`. The `http-static` module provides these directives.

> [!info]
> For related features, see [Routing and URL processing](/docs/configuration/routing/url-processing), [HTTP cache](/docs/configuration/content/cache), [HTTP response control](/docs/configuration/routing/response), [URL rewriting](/docs/configuration/routing/rewrite), [HTTP compression](/docs/configuration/content/compression), and [Canary deployments](/docs/configuration/routing/canary).

The `root`, `index`, `directory_listing`, `etag`, `file_cache_control`, `mime_type`, and `disable_symlinks` directives do not inherit from the wildcard `*` host. They inherit from global defaults, and a `location` block inherits them from its host.

## Web root

`root <path: string>` sets the directory that the file handler uses after the HTTP stages leave a request without a response. Ferron canonicalizes the resolved path before the file stages run and rejects requests that try to escape the webroot. Default: not configured.

```ferron
example.com {
    root /srv/www/example
}
```

If a request continues below a matched file path, Ferron carries the unmatched suffix into the file-stage context as `path_info` for PHP, CGI, and FastCGI.

## Index files and directory listings

| Directive           | Arguments                | Description                                                     | Default                             |
| ------------------- | ------------------------ | --------------------------------------------------------------- | ----------------------------------- |
| `index`             | `<filename: string>...`  | Filenames to try when a request path is a directory.            | `index.html index.htm index.xhtml`  |
| `directory_listing` | `[bool]`                 | Generate an HTML listing when a directory has no index file.    | `false`                             |

Ferron tries `index` names in order. The first existing file replaces the directory path in the file context. This applies only when the resolved path is a directory and there is no `path_info`.

```ferron
example.com {
    root /srv/www/example
    index index.html index.htm
    directory_listing
}
```

A listing never includes dotfiles. Ferron reads `.maindesc` in the directory and shows it as a `<pre>` block below the file table.

## Cache headers

| Directive            | Arguments       | Description                                            | Default |
| -------------------- | --------------- | ------------------------------------------------------ | ------- |
| `etag`               | `[bool]`        | Generate ETags for static responses.                   | `true`  |
| `file_cache_control` | `<value: string>` | `Cache-Control` value for every static response.     | not set |

```ferron
example.com {
    root /srv/www/example
    etag
    file_cache_control "public, max-age=3600"
}
```

ETags are weak (`W/"..."`) and derive from an xxHash3 hash of the file path, size, and modification time. A request with `If-None-Match` that matches the current ETag gets `304 Not Modified`.

With compression active, Ferron appends a suffix such as `-br` for Brotli. Pre-compressed sidecar files get their own ETag from the sidecar metadata.

`file_cache_control` passes the value through unchanged. A value that cannot be sent as a header is a configuration error, and `ferron validate` reports it.

## MIME types

`mime_type <extension: string> <mime-type: string>` maps a file extension to a MIME type. The extension may have a leading dot. Custom mappings override the built-in database, and you can define several. Default: built-in MIME database.

```ferron
example.com {
    root /srv/www/example
    mime_type ".wasm" "application/wasm"
    mime_type ".webmanifest" "application/manifest+json"
}
```

An extension missing from the custom mappings falls back to the built-in database. If neither matches, Ferron sends the response without a `Content-Type` header.

The value is sent as `Content-Type` and written into `multipart/byteranges` part headers for multi-range requests. A value that cannot be sent as a header is a configuration error, and `ferron validate` reports it.

## Error pages

| Directive                 | Arguments                          | Description                                        | Default           |
| ------------------------- | ---------------------------------- | -------------------------------------------------- | ----------------- |
| `error_page`              | `<status-code: integer>... <file-path: string>` | Map status codes to an error page file. | built-in pages    |
| `error_page_placeholders` | `[bool]`                           | Replace trace placeholders in the error page file. | `false`           |

```ferron
example.com {
    root /srv/www/example
    error_page 404 /srv/www/custom/404.html
    error_page 500 502 503 504 /srv/www/custom/50x.html
    error_page_placeholders
}
```

The last `error_page` argument is always the file path. All arguments before it are status codes, so one directive can map several codes to one page.

The path is absolute, or relative to the current working directory. It is not relative to the webroot. If the file does not exist, Ferron skips the directive and uses the built-in page.

Ferron uses `error_page` only when it generates an error response and no custom response exists.

With `error_page_placeholders`, Ferron replaces `{{trace.id}}` and `{{trace.spanid}}` with the trace ID and span ID of the request. Substitution reads the file into memory and disables the zerocopy/sendfile optimization while active.

## Symlink handling

`disable_symlinks [bool | "if_not_owner"]` controls symbolic links during path resolution.

| Value          | Behavior                                                                                     |
| -------------- | -------------------------------------------------------------------------------------------- |
| `false`        | Allow all symlinks.                                                                          |
| `true`         | Reject symlinks with `403 Forbidden`. This is the default.                                   |
| `if_not_owner` | Allow a symlink when the same user owns the link and the target. Non-Unix treats it as `true`. |

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

Ferron detects symlinks at each path component during traversal, not only at the final target. Detection uses `symlink_metadata()`, which does not follow the link and does no I/O on the target. `if_not_owner` is Unix-specific and requires the symlink and target to have the same owner UID.

> [!warning]
> Symlink-based attacks can bypass directory boundaries. Keep `disable_symlinks true` when `root` contains untrusted symlinks, and in shared hosting environments.

> [!important]
> `disable_symlinks true` is the default. A request path that crosses a symlink gets `403 Forbidden`, and the error page does not name the link. If static files return 403 after you add symlinks, check every path component with `ls -la`. Then set `disable_symlinks false` for trusted content, or `disable_symlinks if_not_owner` when the link and target share an owner.

## Compression

Static file compression uses `compressed`, which is enabled by default for files larger than 256 bytes with a compressible extension. `precompressed` serves `.zst`, `.br`, `.gz`, and `.deflate` sidecar files. See [HTTP compression](/docs/configuration/content/compression) for the algorithm order, ETag suffixes, and `Vary` behavior.

## File handle reuse

Ferron reuses file handles and I/O errors for static responses for at most 200 milliseconds after the first request. This reduces file I/O overhead.

## Observability

### Metrics

| Metric                   | Type    | Attributes                                                                                                | Description                                                                   |
| ------------------------ | ------- | --------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `ferron.static.responses` | Counter | `http.response.status.code`, `ferron.static.outcome`                                                      | Static responses across normal, conditional, range, and error paths.          |

### Access log fields

| Field                                     | Type   | Description                                                                    |
| ----------------------------------------- | ------ | ------------------------------------------------------------------------------ |
| `ferron.static.file_path`                 | string | Absolute file path served.                                                     |
| `ferron.static.file_path_precompressed`   | string | Precompressed file path, when applicable.                                      |
| `ferron.static.dir_path`                  | string | Directory path when Ferron serves a listing.                                   |
| `ferron.file_resolve.request_path`        | string | Decoded request path. Error paths only.                                        |
| `ferron.file_resolve.root_path`           | string | Configured document root. Error paths only.                                    |
| `ferron.file_resolve.outcome`             | string | Resolution outcome: `forbidden`, `bad_request`, or `error`. Error paths only.  |
| `ferron.file_resolve.last_candidate_path` | string | Last filesystem path attempted before failure. Error paths only.               |

### Trace spans

The file resolution span, `ferron.pipeline.file_resolve`, records resolution before any file-serving stage runs.

| Attribute                                 | Type   | Description                                                      |
| ----------------------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.file_resolve.request_path`        | string | Decoded request URI path.                                        |
| `ferron.file_resolve.root_path`           | string | Configured document root.                                        |
| `ferron.file_resolve.outcome`             | string | `resolved`, `not_found`, `forbidden`, `bad_request`, or `error`.  |
| `ferron.file_resolve.resolved_path`       | string | Resolved filesystem path. Success only.                          |
| `ferron.file_resolve.last_candidate_path` | string | Last path attempted before failure. Error only.                  |

The static file stage sets these attributes on its `ferron.stage.static_file` span.

| Attribute                               | Type   | Description                                                |
| --------------------------------------- | ------ | ---------------------------------------------------------- |
| `http.response.status_code`             | int    | Status code of the file response.                          |
| `ferron.static.file_path`               | string | Absolute static file path.                                 |
| `ferron.static.file_path_precompressed` | string | Precompressed file path, when applicable.                  |
| `ferron.static.precompressed`           | bool   | Whether Ferron served a precompressed variant.             |

The directory listing stage sets these attributes on its `ferron.stage.directory_listing` span.

| Attribute                   | Type   | Description                           |
| --------------------------- | ------ | ------------------------------------- |
| `http.response.status_code` | int    | Status code of the response.          |
| `ferron.static.dir_path`    | string | Directory path that Ferron lists.     |

## Best practices

`ferron doctor` reports this check for directives on this page.

- `directory_listing` enabled: auto-generated directory indexes expose the file structure. Enable listings only for intentionally public directories.
