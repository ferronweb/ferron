---
title: Static file serving
description: "Serve static sites with Ferron using root, compression, directory listings, SPA rewrites, caching, and precompressed assets."
---

Configuring Ferron as a static file server is straightforward — you just need to specify the directory containing your static files using the `root` directive inside a `location` block. To configure Ferron as a static file server, you can use the configuration below:

```ferron
example.com {
    root /var/www/html
}
```

## HTTP compression for static files

HTTP compression for static files is enabled by default. To disable it, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    compressed false
}
```

## Directory listings

Directory listings are disabled by default. To enable them, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    directory_listing
}
```

## Single-page applications

Single-page applications (SPAs) are also supported by Ferron by adding a URL rewrite rule in addition to the static file serving configuration. You can use this configuration:

```ferron
example.com {
    root /var/www/html
    rewrite "^/.*" "/" {
        last true
        directory false
        file false
    }
}
```

## Static file serving with caching headers

Ferron supports setting `Cache-Control` headers for static files. To enable caching headers for static files, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    etag
    file_cache_control "public, max-age=3600"
}
```

## Serving precompressed static files

Ferron supports serving precompressed static files (sidecar files like `app.js.gz`, `app.js.br`). To enable this feature, you can use this configuration:

```ferron
example.com {
    root /var/www/html
    precompressed
}
```

In this configuration, Ferron will serve precompressed versions of static files if they exist. The precompressed static files would additionally have `.gz` extension for gzip, `.br` for Brotli, `.deflate` for Deflate, or `.zst` for Zstandard.

## Notes and troubleshooting

- If you get `404 Not Found` for files that should exist, verify the `root` path is correct and readable by the user running Ferron.
- If SPA routes (for example `/dashboard/settings`) return `404 Not Found`, add the rewrite rule from the SPA section so unknown paths fall back to `/`.
- If precompressed assets are not served, check that matching files exist (for example `app.js.br` or `app.js.gz`) and regenerate them after changing source assets.
- If responses look stale while using `file_cache_control`, reduce cache lifetime or temporarily disable caching while debugging.
- If your site serves both static files and API traffic, split routing with `location` blocks (for example `/api` for proxying, `/` for static files). See [Reverse proxying](/docs/v3/use-cases/reverse-proxy).
- If you enable automatic TLS for static hosting behind an HTTPS-terminating proxy (for example Cloudflare), use HTTP-01 ACME challenge. See [Automatic TLS](/docs/v3/use-cases/automatic-tls#note-about-cloudflare-proxies-and-other-https-proxies).
- The `root` directive is documented in [Routing and URL processing](/docs/v3/configuration/routing-url-processing).
- For the full HTTP response cache module, see [HTTP cache](/docs/v3/configuration/http-cache).
