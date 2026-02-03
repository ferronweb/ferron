---
title: Static file serving
description: "Serve static sites with Ferron using root, compression, directory listings, SPA rewrites, caching, and precompressed assets."
---

Configuring Ferron as a static file server is straightforward - you just need to specify the directory containing your static files in the `root` directive. To configure Ferron as a static file server, you can use the configuration below:

```kdl
// Example configuration with static file serving. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
}
```

## HTTP compression for static files

HTTP compression for static files is enabled by default. To disable it, you can use this configuration:

```kdl
// Example configuration with static file serving and HTTP compression disabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    compressed #false
}
```

## Directory listings

Directory listings are disabled by default. To enable them, you can use this configuration:

```kdl
// Example configuration with static file serving and directory listings enabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    directory_listing
}
```

## Single-page applications

Single-page applications (SPAs) are also supported by Ferron by adding an URL rewrite rule (if using static file serving only) in addition to the static file serving configuration. You can use this configuration:

```kdl
// Example configuration with static file serving and URL rewrite rule for SPAs. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    rewrite "^/.*" "/" directory=#false file=#false last=#true
}
```

## Static file serving with in-memory caching

Ferron supports in-memory caching for speeding up websites. To enable in-memory caching for static files, you can use this configuration:

```kdl
// Example configuration with static file serving and in-memory caching enabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    cache
    file_cache_control "max-age=3600"
}
```

## Serving precompressed static files

Ferron supports serving precompressed static files. To enable this feature, you can use this configuration:

```kdl
// Example configuration with static file serving and precompressed files enabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    precompressed
}
```

In this configuration, Ferron will serve precompressed versions of static files if they exist. The precompressed static files would additionally have `.gz` extension for gzip, `.deflate` for Deflate, `.br` for Brotli, or `.zst` for Zstandard.

To create precompressed static files, you can use the `ferron-precompress` tool that comes with Ferron:

```bash
# Replace "/var/www/html" with the directory containing your static files
ferron-precompress /var/www/html
```
