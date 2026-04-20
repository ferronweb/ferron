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
| Compression | `http-compression` | On-the-fly gzip, brotli, deflate, zstd based on `Accept-Encoding`; precompressed sidecar files |
| Rate limiting | `http-ratelimit` | Token bucket algorithm keyed on IP, URI, or request header |
| Headers and CORS | `http-headers` | Add, remove, replace headers; full CORS preflight handling |
| URL rewriting | `http-rewrite` | Regex-based rewrite with `last`, `file`, `directory` options |
| Basic authentication | `http-basicauth` | Argon2, PBKDF2, scrypt password hashes with brute-force protection |
| Response control | `http-response` | Custom status codes, connection abort, IP block/allow, 103 Early Hints |
| Response body replacement | `http-replace` | String replacement in response bodies with MIME type filtering, `once` mode, `Last-Modified` preservation |
| Variable mapping | `http-map` | Create variables from patterns (exact, wildcard, regex with captures) matched against source variables |
| HTTP buffering | `http-buffer` | Request and response body buffering with configurable byte limits |
| HTTP caching | `http-cache` | In-memory response cache with RFC 9111 semantics, LSCache override, vary headers, private/public cache partitioning |
| CGI support | `http-cgi` | Spawn external interpreters for scripts by extension or `cgi-bin` directory |
| SCGI support | `http-scgi` | Binary protocol for application servers with TCP or Unix socket backends |
| Forwarded authentication | `http-fauth` | Forward authentication requests to external identity providers |

### TLS

| Feature | Module | Notes |
| --- | --- | --- |
| Manual TLS | `tls-manual` | Certificate/key paths with environment variable interpolation |
| ACME automatic TLS | `tls-acme` | HTTP-01, TLS-ALPN-01, and DNS-01 challenges with caching and auto-renewal |
| DNS Providers | `dns-stalwart` | DNS-01 challenge support for Bunny, Cloudflare, deSEC, DigitalOcean, DNSimple, Google Cloud, OVH, Porkbun, RFC2136, Route 53, and Spaceship |
| OCSP stapling | `ocsp-stapler` | Automatic OCSP response fetching and stapling |
| mTLS | `tls-manual` | Client certificate authentication with configurable trust store |
| Custom crypto | `tls-manual` | Cipher suite selection, ECDH curves, TLS version restrictions |
| Session tickets | `tls-manual` | Stateless TLS session resumption with automatic key rotation and file-backed persistence |

### Observability

| Feature | Module | Notes |
| --- | --- | --- |
| Console logging | `observability-consolelog` | Structured events to Ferron's log output |
| File logging | `observability-logfile` | Access and error logs with log rotation support |
| JSON formatting | `observability-format-json` | JSON-serialized access log entries |
| Text formatting | `observability-format-text` | Combined Log Format or custom text patterns |
| OTLP export | `observability-otlp` | Logs, metrics, and traces to OpenTelemetry collectors via gRPC or HTTP. See [OTLP observability](/docs/v3/configuration/observability-otlp). |
| Process metrics | `observability-process-metrics` | CPU and memory metrics from `/proc/self/stat` (Linux only) |
| Prometheus metrics | `observability-prometheus` | Exports metrics in Prometheus format via HTTP endpoint. See [Prometheus metrics](/docs/v3/configuration/observability-prometheus). |

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
- **All modules are compiled into the binary** — no runtime plugin loading yet. Every module ships with the default build.
- **Primary testing target is Linux** — Windows and macOS receive less coverage and may have edge-case issues.

### Experimental features

- **HTTP/3 (QUIC) support** — HTTP/3 is available via the `protocols h3` directive but is **experimental**. When enabled, Ferron binds an additional UDP listener on the same port. This feature may change or be removed in future releases. See [HTTP host directives](/docs/v3/configuration/http-host) for configuration details.

## Roadmap

Planned direction for future releases:

- Dynamically loadable modules (WebAssembly?)
- Additional observability backends (Jaeger/Zipkin?)
- More authentication methods (JWT?, OAuth2?)

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
