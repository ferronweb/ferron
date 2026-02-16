---
title: "Configuration: static & content handling"
description: "Static file serving, cache directives, and response content processing in KDL configuration."
---

This page describes KDL directives for serving static assets, tuning response caching, and controlling content processing.

## Global-only directives

### Caching

- `cache_max_entries <cache_max_entries: integer|null>` (_cache_ module)
  - This directive specifies the maximum number of entries that can be stored in the HTTP cache. If set as `cache_max_entries #null`, the cache can theoretically store an unlimited number of entries. The cache keys for entries depend on the request method, the rewritten request URL, the "Host" header value, and varying request headers. Default: `cache_max_entries 1024`

## Configuration example

```kdl
* {
    cache_max_entries 2048
}
```

## Directives

### Static file serving

- `root <webroot: string|null>`
  - This directive specifies the webroot from which static files are served. If set as `root #null`, the static file serving functionality is disabled. Default: none
- `etag [enable_etag: bool]` (_static_ module)
  - This directive specifies whether the ETag header is enabled. Default: `etag #true`
- `compressed [enable_compression: bool]` (_static_ module)
  - This directive specifies whether the HTTP compression for static files is enabled. Default: `compressed #true`
- `directory_listing [enable_directory_listing: bool]` (_static_ module)
  - This directive specifies whether the directory listings are enabled. Default: `directory_listing #false`
- `precompressed [enable_precompression: bool]` (_static_ module)
  - This directive specifies whether serving the precompressed static files is enabled. The precompressed static files would additionally have `.gz` extension for gzip, `.deflate` for Deflate, `.br` for Brotli, or `.zst` for Zstandard. Default: `precompressed #false`
- `mime_type <file_extension: string> <mime_type: string>` (_static_ module; Ferron 2.1.0 or newer)
  - This directive specifies an additional MIME type corresponding to a file extension (like `.html`) for static files. Default: none
- `index <index_file: string> [<another_index_file: string> ...]` (_static_ module; Ferron 2.1.0 or newer)
  - This directive specifies the index files to be used when a directory is requested. Default: `index "index.html" "index.htm" "index.html"` (static file serving), `index "index.php" "index.cgi" "index.html" "index.htm" "index.html"` (CGI, FastCGI)
- `dynamic_compressed [enable_dynamic_content_compression: bool]` (_dcompress_ module; Ferron 2.1.0 or newer)
  - This directive specifies whether the HTTP compression for dynamic content is enabled. Default: `dynamic_compressed #false`

## Configuration example

```kdl
example.com {
    root "/var/www/example.com"
    etag
    compressed
    directory_listing #false

    // Set "Cache-Control" header for static files
    file_cache_control "public, max-age=3600"
}
```

### Caching

- `cache [enable_cache: bool]` (_cache_ module)
  - This directive specifies whether the HTTP cache is enabled. Default: `cache #false`
- `cache_max_response_size <cache_max_response_size: integer|null>` (_cache_ module)
  - This directive specifies the maximum size of the response (in bytes) that can be stored in the HTTP cache. If set as `cache_max_response_size #null`, the cache can theoretically store responses of any size. Default: `cache_max_response_size 2097152`
- `cache_vary <varying_request_header: string> [<varying_request_header: string> ...]` (_cache_ module)
  - This directive specifies the request headers that are used to vary the cache entries. This directive can be specified multiple times. Default: none
- `cache_ignore <ignored_response_header: string> [<ignored_response_header: string> ...]` (_cache_ module)
  - This directive specifies the response headers that are ignored when caching the response. This directive can be specified multiple times. Default: none
- `file_cache_control <cache_control: string|null>` (_static_ module)
  - This directive specifies the Cache-Control header value for static files. If set as `file_cache_control #null`, the Cache-Control header is not set. Default: `file_cache_control #null`

## Configuration example

```kdl
example.com {
    cache
    cache_max_response_size 2097152
    cache_vary "Accept-Encoding" "Accept-Language"
    cache_ignore "Set-Cookie" "Cache-Control"
}
```

### Content processing

Disabling HTTP compression is required for string replacement.

- `replace <searched_string: string> <replaced_string: string> [once=<replace_once: bool>]` (_replace_ module)
  - This directive specifies the string to be replaced in a response body, and a replacement string. The `once` prop specifies whether the string will be replaced once, by default this prop is set to `#true`. This directive can be specified multiple times. Default: none
- `replace_last_modified [preserve_last_modified: bool]` (_replace_ module)
  - This directive specifies whether to preserve the "Last-Modified" header in the response. Default: `replace_last_modified #false`
- `replace_filter_types <filter_type: string> [<filter_type: string> ...]` (_replace_ module)
  - This directive specifies the response MIME type filters. The filter can be either a specific MIME type (like `text/html`) or a wildcard (`*`) specifying that responses with all MIME types are processed for replacement. This directive can be specified multiple times. Default: `replace_filter_types "text/html"`

## Configuration example

```kdl
example.com {
    // Disabling HTTP compression is required for string replacement
    compressed #false

    // String replacement in response bodies (works with HTTP compression disabled)
    replace "old-company-name" "new-company-name" once=#false
    replace "http://old-domain.com" "https://new-domain.com" once=#true

    replace_last_modified
    replace_filter_types "text/html" "text/css" "application/javascript"
}
```
