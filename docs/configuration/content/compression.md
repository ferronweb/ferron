---
title: "Configuration: HTTP compression"
description: "On-the-fly and pre-compressed HTTP response body compression, algorithm preference, and configuration."
---

This page documents Ferron's HTTP compression system, including the algorithm preference order, supported algorithms, and configuration options.

> [!info]
> Compression is handled by the `http-compression` module. For static file compression specifically, see [Static file serving](/docs/v3/configuration/content/static-files#compression).

## Algorithm preference

When a client sends an `Accept-Encoding` header, Ferron selects the best compression algorithm based on a **preference order**:

1. **Zstandard** — the preferred algorithm. Offers the best compression ratio for text content and fast decoding.
2. **Brotli** — excellent compression ratio, widely supported.
3. **gzip** — the most universally supported compression algorithm.
4. **Deflate** — similar to gzip but without the CRC checksum overhead; less common in practice.

The server iterates through the client's `Accept-Encoding` header values and selects the **first** algorithm that matches the preference order. For example, if a client sends:

```text
Accept-Encoding: gzip, br, zstd
```

The server will select **Zstandard** because it appears first in the preference order, even though the client listed gzip first.

If the client does not send an `Accept-Encoding` header, or none of the supported algorithms are listed, the response is served without compression (`identity`).

## Configuration

### On-the-fly compression

- `compressed [bool: boolean]` (`http-static`)
  - Enables on-the-fly compression for static file responses. Files larger than 256 bytes with compressible extensions are compressed dynamically. Default: `compressed true`
- `dynamic_compressed [bool: boolean]` (`http-compression`)
  - Enables on-the-fly compression for dynamic response bodies (e.g., responses from reverse proxies or application handlers). Default: `dynamic_compressed false`

### Pre-compressed sidecar files

- `precompressed [bool: boolean]` (`http-static`)
  - Enables serving pre-compressed sidecar files (e.g. `style.css.zst`, `app.js.br`) instead of compressing on the fly. The server checks for a pre-compressed file alongside the original based on the client's `Accept-Encoding` preference. Default: `precompressed false`

**Configuration example:**

```ferron
example.com {
    root /srv/www/example
    compressed
    dynamic_compressed
    precompressed
}
```

## Algorithm details

| Algorithm | `Content-Encoding` value | Pre-compressed file extension | ETag suffix | Encoding parameters |
|---|---|---|---|---|
| Zstandard | `zstd` | `.zst` | `-zstd` (static files), `-dynamic-zstd` (dynamic responses) | Quality level 4, window log 17, hash log 10 |
| Brotli | `br` | `.br` | `-br` (static files), `-dynamic-br` (dynamic responses) | Quality level 4, window size 17, block size 18 |
| gzip | `gzip` | `.gz` | `-gzip` (static files), `-dynamic-gzip` (dynamic responses) | Quality level 4 |
| Deflate | `deflate` | `.deflate` | `-deflate` (static files), `-dynamic-deflate` (dynamic responses) | Quality level 4 |

## Browser compatibility

Ferron detects and handles browsers with known compression bugs:

- **Netscape 4.x** (non-IE): compression is disabled for text/html content.
- **w3m/0.5.x**: HTML compression is disabled.
- **IE masquerading as Netscape 4.x**: compression is allowed (the presence of `MSIE` in the user agent indicates it is safe).

## ETag handling

When compression is applied, the ETag is modified to distinguish compressed variants:

- **Static files**: a suffix is appended to the ETag (e.g. `W/"abc123-zstd"` for zstd-compressed files). Pre-compressed sidecar files receive their own ETag based on the sidecar file's metadata.
- **Dynamic responses**: a `-dynamic-` prefixed suffix is appended (e.g. `W/"abc123-dynamic-zstd"`).

When the `If-None-Match` header is present, the server checks both the base ETag and the compressed variant to determine whether to return `304 Not Modified`.

## Vary header

When compression is possible (based on file size and extension), the server adds `Accept-Encoding` to the `Vary` header to ensure caches serve the correct compressed variant to each client.
