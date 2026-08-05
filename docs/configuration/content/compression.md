---
title: "Configuration: HTTP compression"
description: On-the-fly and pre-compressed HTTP response body compression, algorithm preference, and configuration.
---

This page documents the Ferron HTTP compression system. It covers the algorithm preference order, supported algorithms, and configuration options.

> [!info]
> The `http-compression` module handles compression. For static file compression specifically, see [Static file serving](/docs/v3/configuration/content/static-files#compression).

## Algorithm preference

When a client sends an `Accept-Encoding` header, Ferron selects the best compression algorithm based on a **preference order**:

1. **Zstandard**: the preferred algorithm. Offers the best compression ratio for text content and fast decoding.
2. **Brotli**: excellent compression ratio, widely supported.
3. **gzip**: the most universally supported compression algorithm.
4. **Deflate**: similar to gzip but without the CRC checksum overhead. Less common in practice.

The server iterates through the `Accept-Encoding` header values that the client sends. It selects the **first** algorithm that matches the preference order. For example, if a client sends:

```text
Accept-Encoding: gzip, br, zstd
```

The server selects **Zstandard** because it appears first in the preference order, even though the client listed gzip first.

If the client does not send an `Accept-Encoding` header, the server serves the response without compression (`identity`). The same applies when none of the supported algorithms appear in the header.

## Configuration

### On-the-fly compression

The `compressed` directive (`http-static`) enables on-the-fly compression for static file responses. The `dynamic_compressed` directive (`http-compression`) enables on-the-fly compression for dynamic response bodies such as reverse proxy responses. The server compresses files larger than 256 bytes with compressible extensions. Default: `compressed true`, `dynamic_compressed false`

### Pre-compressed sidecar files

- `precompressed [bool: boolean]` (`http-static`)
  - Enables serving pre-compressed sidecar files (for example, `style.css.zst`, `app.js.br`) instead of compressing on the fly. The server checks for a pre-compressed file alongside the original based on which algorithms the client lists in `Accept-Encoding`. Default: `precompressed false`

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

| Algorithm | `Content-Encoding` value | Pre-compressed file extension | ETag suffix                                                       | Encoding parameters                            |
| --------- | ------------------------ | ----------------------------- | ----------------------------------------------------------------- | ---------------------------------------------- |
| Zstandard | `zstd`                   | `.zst`                        | `-zstd` (static files), `-dynamic-zstd` (dynamic responses)       | Quality level 4, window log 17, hash log 10    |
| Brotli    | `br`                     | `.br`                         | `-br` (static files), `-dynamic-br` (dynamic responses)           | Quality level 4, window size 17, block size 18 |
| gzip      | `gzip`                   | `.gz`                         | `-gzip` (static files), `-dynamic-gzip` (dynamic responses)       | Quality level 4                                |
| Deflate   | `deflate`                | `.deflate`                    | `-deflate` (static files), `-dynamic-deflate` (dynamic responses) | Quality level 4                                |

## Browser compatibility

Ferron detects and handles browsers with known compression bugs:

- **Netscape 4.x** (non-IE): Ferron disables compression for text/html content.
- **w3m/0.5.x**: Ferron disables HTML compression.
- **IE masquerading as Netscape 4.x**: Ferron allows compression (`MSIE` in the user agent indicates it is safe).

## ETag handling

When Ferron applies compression, it modifies the ETag to distinguish compressed variants:

- **Static files**: Ferron appends a suffix to the ETag (for example, `W/"abc123-zstd"` for zstd-compressed files). Pre-compressed sidecar files receive their own ETag. That ETag derives from the metadata of the sidecar file.
- **Dynamic responses**: Ferron appends a `-dynamic-` prefixed suffix (for example, `W/"abc123-dynamic-zstd"`).

When the `If-None-Match` header is present, the server checks both the base ETag and the compressed variant. This determines whether to return `304 Not Modified`.

## Vary header

When compression is possible (based on file size and extension), the server adds `Accept-Encoding` to the `Vary` header. This makes sure caches serve the correct compressed variant to each client.

## Observability

### Access log fields

The compression module contributes the following field to the HTTP access log line:

| Field                          | Type   | Description                                                            |
| ------------------------------ | ------ | ---------------------------------------------------------------------- |
| `ferron.compression.algorithm` | string | Compression algorithm: `gzip`, `br`, `deflate`, `zstd`, or `identity`. |

### Trace spans

The dynamic compression stage sets the following attributes on its `ferron.stage.dynamic_compression` span:

| Attribute                          | Type   | Description                                                                 |
| ---------------------------------- | ------ | --------------------------------------------------------------------------- |
| `ferron.compression.algorithm`     | string | Compression algorithm used: `gzip`, `br`, `deflate`, `zstd`, or `identity`. |
| `ferron.compression.precompressed` | bool   | Whether the server served a precompressed file variant.                     |
