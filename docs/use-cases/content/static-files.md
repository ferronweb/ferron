---
title: Static file serving
description: "Serve static sites with Ferron using root, compression, directory listings, SPA rewrites, caching, and precompressed assets."
---

Configure Ferron as a static file server with the `root` directive. The directive sets the directory that contains your static files. Use this configuration:

```ferron
example.com {
    root /var/www/html
}
```

## HTTP compression for static files

Ferron enables HTTP compression for static files by default. To disable it, use this configuration:

```ferron
example.com {
    root /var/www/html
    compressed false
}
```

## Directory listings

Ferron disables directory listings by default. To enable them, use this configuration:

```ferron
example.com {
    root /var/www/html
    directory_listing
}
```

> [!tip]
> If you get `404 Not Found` for files that should exist, verify the `root` path is correct and readable by the user running Ferron.

## Single-page applications

Ferron also supports single-page applications (SPAs). Add a URL rewrite rule to the static file serving configuration. Use this configuration:

```ferron
example.com {
    root /var/www/html
    rewrite "^/.*" "/" {
        last
        directory false
        file false
    }
}
```

> [!tip]
> If SPA routes (for example `/dashboard/settings`) return `404 Not Found`, add the rewrite rule from the SPA section so unknown paths fall back to `/`.

## Static file serving with caching headers

Ferron supports setting `Cache-Control` headers for static files. To enable caching headers for static files, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    etag
    file_cache_control "public, max-age=3600"
}
```

> [!tip]
> If responses look stale while using `file_cache_control`, reduce cache lifetime or temporarily disable caching while debugging.

## Serving precompressed static files

Ferron supports serving precompressed static files (sidecar files like `app.js.gz`, `app.js.br`). To enable this feature, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    precompressed
}
```

In this configuration, Ferron serves precompressed versions of static files if they exist. The precompressed files use the `.gz` extension for gzip, `.br` for Brotli, `.deflate` for Deflate, and `.zst` for Zstandard.

> [!tip]
> If Ferron does not serve precompressed assets, check that the matching files exist (for example `app.js.br` or `app.js.gz`). Regenerate the files after you change the source assets.
