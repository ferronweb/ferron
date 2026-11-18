---
title: Static file serving
description: "Serve a static site with root, compression, directory listings, SPA fallback, cache headers, and precompressed assets."
---

Set `root` to the directory that contains the site:

```ferron
example.com {
    root /var/www/html
}
```

Ferron serves files from that directory. A path that does not match a file returns `404 Not Found`.

> [!tip]
> If a file exists but Ferron returns `404 Not Found`, check the `root` path and the read permissions of the user that runs Ferron.

## Compression

Ferron compresses static files on the fly by default. It compresses files larger than 256 bytes with a compressible extension. To disable compression:

```ferron
example.com {
    root /var/www/html
    compressed false
}
```

## Directory listings

Directory listings are off by default. To enable them:

```ferron
example.com {
    root /var/www/html
    directory_listing
}
```

## Single-page applications

A single-page application needs a fallback rewrite. Without it, client-side routes such as `/dashboard/settings` return `404 Not Found`.

```ferron
example.com {
    root /var/www/html
    rewrite r"^/.*" "/" {
        last
        directory false
        file false
    }
}
```

The `file false` and `directory false` guards keep real files and directories on the normal file path. Only unmatched paths fall back to `/`.

## Cache headers

`etag` adds an ETag validator to file responses. `file_cache_control` sets the `Cache-Control` header:

```ferron
example.com {
    root /var/www/html
    etag
    file_cache_control "public, max-age=3600"
}
```

> [!tip]
> If clients keep stale content after you deploy, lower the `max-age` value or add a content hash to the file names.

## Precompressed files

Precompressed serving uses sidecar files next to the original file. For example, `app.js.br` serves the Brotli variant of `app.js`.

```ferron
example.com {
    root /var/www/html
    precompressed
}
```

Ferron checks for these sidecar extensions:

| Encoding  | Extension |
| --------- | --------- |
| gzip      | `.gz`     |
| Brotli    | `.br`     |
| Deflate   | `.deflate` |
| Zstandard | `.zst`    |

> [!tip]
> If Ferron does not serve a precompressed variant, check that the sidecar file exists next to the source file. Regenerate the sidecars after you change the source assets.

> [!info]
> For all static file options, see [Configuration: static files](/docs/configuration/content/static-files). For rewrite syntax, see [URL rewriting](/docs/use-cases/traffic/url-rewriting).
