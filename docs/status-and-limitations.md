---
title: Status and limitations
description: "What currently works in Ferron 3 alpha, known limitations, and the planned roadmap."
---

> **Status: Alpha** — This is an early development release. Not production-ready. APIs and configuration may change.

## What works today

The following features are implemented and functional in Ferron 3:

### HTTP serving

| Feature | Module | Notes |
| --- | --- | --- |
| Static file serving | `http-static` | `root`, compression, ETags, directory listings, MIME types, precompressed sidecar files |
| Reverse proxy | `http-proxy` | `proxy` with load balancing, health checks, connection pooling, keepalive reuse, header manipulation |
| Forward proxy | `http-fproxy` | CONNECT method support with optional authentication |
| Compression | `http-static` | On-the-fly gzip, brotli, deflate, zstd based on `Accept-Encoding` |
| Rate limiting | `http-ratelimit` | Token bucket algorithm keyed on IP, URI, or request header |
| Headers and CORS | `http-headers` | Add, remove, replace headers; full CORS preflight handling |
| URL rewriting | `http-rewrite` | Regex-based rewrite with `last`, `file`, `directory` options |
| Basic authentication | `http-basicauth` | Argon2, PBKDF2, scrypt password hashes with brute-force protection |
| Response control | `http-response` | Custom status codes, connection abort, IP block/allow, 103 Early Hints |

### TLS

| Feature | Module | Notes |
| --- | --- | --- |
| Manual TLS | `tls-manual` | Certificate/key paths with environment variable interpolation |
| ACME automatic TLS | `tls-acme` | HTTP-01, TLS-ALPN-01, and DNS-01 challenges with caching and auto-renewal |
| OCSP stapling | `ocsp-stapler` | Automatic OCSP response fetching and stapling |
| mTLS | `tls-manual` | Client certificate authentication with configurable trust store |
| Custom crypto | `tls-manual` | Cipher suite selection, ECDH curves, TLS version restrictions |

### Observability

| Feature | Module | Notes |
| --- | --- | --- |
| Console logging | `observability-consolelog` | Structured events to Ferron's log output |
| File logging | `observability-logfile` | Access and error logs with log rotation support |
| JSON formatting | `observability-format-json` | JSON-serialized access log entries |
| Text formatting | `observability-format-text` | Combined Log Format or custom text patterns |
| OTLP export | `observability-otlp` | Logs, metrics, and traces to OpenTelemetry collectors via gRPC or HTTP |
| Process metrics | `observability-process-metrics` | CPU and memory metrics from `/proc/self/stat` (Linux only) |

### Admin and runtime

| Feature | Module | Notes |
| --- | --- | --- |
| Health endpoint | `admin-api` | `GET /health` — `200 OK` or `503` during shutdown |
| Status endpoint | `admin-api` | `GET /status` — uptime, active connections, request count, reload count |
| Config dump | `admin-api` | `GET /config` — sanitized effective configuration (sensitive fields redacted) |
| Hot reload | `admin-api` | `POST /reload` or SIGHUP — graceful configuration reload |
| io_uring | runtime | Linux `io_uring` support with epoll fallback |
| PROXY protocol | runtime | PROXY protocol v1/v2 parsing from HAProxy and similar load balancers |

## Known limitations

- **Alpha quality** — not battle-tested; expect bugs and configuration changes between releases.
- **Admin API has no authentication** — bind it to localhost (`127.0.0.1:8081`) or restrict access via firewall rules.
- **All modules are compiled into the binary** — no runtime plugin loading yet. Every module ships with the default build.
- **Primary testing target is Linux** — Windows and macOS receive less coverage and may have edge-case issues.
- **HTTP/3 is not yet supported** — only HTTP/1.1 and HTTP/2 are available.
- **No DNS provider modules** — the DNS-01 ACME challenge type is defined, but no DNS provider backends (Cloudflare, Route 53, etc.) are implemented yet. If you need wildcard certificates, obtain them externally and use [Manual TLS](/docs/v3/use-cases/manual-tls).
- **Rate limiting is per-server-instance** — buckets are stored in memory and not shared across multiple Ferron instances. For distributed rate limiting, use an external service (e.g. Redis).
- **HTTP response caching** — a full HTTP cache module is planned but not yet implemented. The `file_cache_control` directive only sets `Cache-Control` headers; it does not perform server-side caching of proxied responses.
- **`client_auth` is per-host** — mTLS is configured inside individual `tls` blocks. There is no global mTLS toggle that applies to all hosts.

## Roadmap

Planned direction for future releases:

- Dynamically loadable modules (WebAssembly?)
- HTTP/3 and QUIC support
- Additional observability backends (Prometheus, Jaeger/Zipkin?)
- More authentication methods (JWT?, OAuth2?)
- Full HTTP response caching module

## Upgrading from Ferron 2

Ferron 3 is a complete rewrite. It shares the vision but not the code:

| Aspect | Ferron 2 | Ferron 3 |
| --- | --- | --- |
| Architecture | Monolithic | Module-driven, pluggable |
| Observability | Basic logging | Structured events with multiple backends |
| Configuration | KDL-based | Custom `.conf`, layered scopes, snippets |
| Extensibility | Compile-time modules | Runtime-registered stages and providers |
| Request Processing | Linear pipeline | DAG-ordered stages with inverse cleanup |

Configuration files from Ferron 2 are **not yet compatible** with Ferron 3. See the [configuration syntax](/docs/v3/configuration/syntax) page for the new format.

## Contributing

Feedback, bug reports, and testing are welcome. When reporting issues, include your configuration file, `--verbose` output, and steps to reproduce.
