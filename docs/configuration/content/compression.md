---
title: "Configuration: HTTP compression"
description: "On-the-fly and pre-compressed response compression, with algorithm preference and ETag handling."
---

Ferron compresses response bodies when the client accepts a supported encoding. It can also serve pre-compressed sidecar files from disk.

Dynamic compression comes from the `http-compression` module. Static file compression comes from the static file module. See [Static file serving](/docs/configuration/content/static-files#compression) for the static side.

## Algorithm selection

Ferron uses a fixed preference order, not the order in the client header:

1. Zstandard. Best ratio for text, fast decoding.
2. Brotli. Strong ratio, widely supported.
3. gzip. The most widely supported algorithm.
4. Deflate. Similar to gzip without the CRC checksum overhead. Less common.

```text
Accept-Encoding: gzip, br, zstd
```

For this header Ferron selects Zstandard, even though the client listed gzip first. If the header is missing or lists no supported algorithm, Ferron sends the response uncompressed (`identity`).

## Configuration

Static files and dynamic responses use separate directives. Do not mix them up.

### Static file compression

`compressed [bool]` (`http-static`) enables on-the-fly compression for files served from `root`. Ferron compresses files larger than 256 bytes that have a compressible extension. Default: `compressed true`.

A named host does not inherit `compressed` from the wildcard `*` host. It still inherits from global defaults, and a `location` block inherits from its host.

### Dynamic response compression

`dynamic_compressed [bool]` (`http-compression`) enables on-the-fly compression for dynamic bodies, such as reverse proxy, FastCGI, and CGI responses. Use it when a backend returns compressible text without compressing it itself. Default: `dynamic_compressed false`.

Ferron never compresses `101 Switching Protocols` or `304 Not Modified` responses. Protocol upgrades such as WebSockets pass through unchanged.

### Pre-compressed sidecar files

`precompressed [bool]` (`http-static`) serves sidecar files such as `style.css.zst` or `app.js.br` instead of compressing on the fly. Ferron checks for a sidecar next to the original file, based on the algorithms in `Accept-Encoding`. Default: `precompressed false`.

A named host does not inherit `precompressed` from the wildcard `*` host. It still inherits from global defaults, and a `location` block inherits from its host.

```ferron
example.com {
    root /srv/www/example
    compressed
    dynamic_compressed
    precompressed
}
```

## Algorithm details

| Algorithm | `Content-Encoding` | Sidecar extension | ETag suffix                                       | Encoder settings                               |
| --------- | ------------------ | ----------------- | ------------------------------------------------- | ---------------------------------------------- |
| Zstandard | `zstd`             | `.zst`            | `-zstd`, `-dynamic-zstd` for dynamic responses     | Quality 4, window log 17, hash log 10          |
| Brotli    | `br`               | `.br`             | `-br`, `-dynamic-br` for dynamic responses         | Quality 4, window 17, block 18                 |
| gzip      | `gzip`             | `.gz`             | `-gzip`, `-dynamic-gzip` for dynamic responses     | Quality 4                                      |
| Deflate   | `deflate`          | `.deflate`        | `-deflate`, `-dynamic-deflate` for dynamic responses | Quality 4                                    |

## ETag handling

Ferron changes the ETag so each encoding has a distinct validator.

- Static files get a suffix, for example `W/"abc123-zstd"`.
- Pre-compressed sidecar files get their own ETag, derived from the sidecar metadata.
- Dynamic responses get a `-dynamic-` suffix, for example `W/"abc123-dynamic-zstd"`.

With `If-None-Match`, Ferron checks both the base ETag and the compressed variant before it returns `304 Not Modified`.

## Vary header

When compression is possible, based on file size and extension, Ferron adds `Accept-Encoding` to `Vary`. Caches then keep separate variants for each client encoding.

## Browser compatibility

Ferron handles browsers with known compression bugs:

- Netscape 4.x, other than IE: compression is disabled for `text/html`.
- w3m/0.5.x: HTML compression is disabled.
- IE posing as Netscape 4.x: compression stays enabled. An `MSIE` user agent marks this case as safe.

## Observability

### Access log fields

| Field                          | Type   | Description                                                            |
| ------------------------------ | ------ | ---------------------------------------------------------------------- |
| `ferron.compression.algorithm` | string | `gzip`, `br`, `deflate`, `zstd`, or `identity`.                         |

### Trace spans

The dynamic compression stage sets these attributes on its `ferron.stage.dynamic_compression` span.

| Attribute                          | Type   | Description                                                 |
| ---------------------------------- | ------ | ----------------------------------------------------------- |
| `ferron.compression.algorithm`     | string | `gzip`, `br`, `deflate`, `zstd`, or `identity`.              |
| `ferron.compression.precompressed` | bool   | Whether Ferron served a precompressed file variant.          |
