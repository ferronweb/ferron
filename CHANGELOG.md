# Ferron 3 change log

## Ferron UNRELEASED

**Not released yet**

### Breaking changes

If you are upgrading to this beta version, you must update your configuration files to accommodate the following syntax refactors:

- **Rate limit windows** - syntax updated to enforce standard duration strings (e.g., `10s`, `5m`, `1h`).
- **OTLP verification** - `no_verify` has been renamed to `no_verification` and now operates strictly as a configuration flag.
- **Proxy configuration** - syntax for passive/active health checks, load balancing algorithms, and connection retries has been unified into a cleaner, more consistent format.

### Added

#### Modules

- **`abuse_protection`** - a new module for lightweight, native Fail2ban-like IP banning with temporary lockouts based on rate limit breaches and brute-force failures.
- **`tls-http`** - support for obtaining TLS certificates from a remote HTTP endpoint, featuring automatic refresh cycles and dedicated observability metrics.
- **OS metrics** - added Windows support for native process metrics in the `metrics-process` module.

#### Reverse proxy & load balancing

- **`circuit_breaker`** - new directive with rolling failure windows, temporary backend ejection, and half-open recovery states.
- **`p2c_ewma`** - a new adaptive, latency-aware load balancing algorithm combining Power of Two Choices with Exponentially Weighted Moving Average (EWMA) latency scoring.
- **Session affinity (sticky sessions)** - native support for `cookie`, `header`, `ip`, and `hash` types utilizing a Ketama-style hash ring for deterministic backend routing.
- **String interpolation** - upstream URLs and Unix socket paths now support interpolated strings.
- **SRV routing** - added active health check support for SRV upstream URLs.

#### DNS & ACME

- **58 new DNS providers** - native ACME DNS-01 challenge support expanded to include Alibaba Cloud DNS, Azure DNS, ClouDNS, Hetzner DNS, Oracle Cloud DNS, Vercel, Vultr, Yandex Cloud DNS, and 50+ others.
- **CLI arguments in post-obtain command** - added support for passing CLI arguments to the post-obtain command in automatic TLS configurations.

#### HTTP server core

- **`basic_auth_concurrency`** - global directive to limit concurrent, resource-heavy password verification tasks across all `basic_auth` blocks.
- **Cache purging** - native `PURGE` HTTP method support for targeted cache invalidation via the `purge_method` and `purge_allowed_ips` subdirectives.

#### Observability & metrics

- **Edge-case visibility** - granular HTTP observability metrics for pre-handler failures, server redirects, client-IP rewrites, CORS preflights, connection lifecycle failures, forward-proxy outcomes, reverse-proxy failures, and static-file response outcomes.
- **Admin sinks** - added a dropped-events admin metric for non-blocking observability sinks.

### Changed

#### Reverse proxy

- **Weighted load balancing** - both `least_conn` and `round_robin` algorithms now support per-upstream `weight` directives for proportional traffic distribution.
- **Metric attributes** - multiple HTTP reverse proxy metrics now embed the specific backend URL or Unix socket path as a context attribute.

#### DNS & TLS

- **Early OCSP verification** - OCSP responses are now strictly verified before being cached and stapled.
- **Verbose errors** - significantly improved error reporting layouts for local automatic TLS and specific TLS handshake failures.

#### HTTP server core

- **Safe configuration reloads** - configuration failures during a live reload no longer crash or stop the server; errors are logged, and execution safely continues using the previous valid configuration.
- **Smarter brute-force locking** - protection now locks by **IP address** instead of username, preventing malicious actors from intentionally locking out legitimate users.
- **Accurate status codes** - refactored error handling to return context-aware status codes over generic `500` or `404` errors:
  - File pipeline returns `408 Request Timeout` if a request takes too long (instead of `404`).
  - Basic Auth returns `429 Too Many Requests` when max failed attempts are reached.
  - File serving errors return `403 Forbidden` (permissions) or `400 Bad Request` (invalid filenames) instead of `500`.
- **String interpolation** - forwarded authentication now supports interpolated string values for backend URLs.
- **Security tightening** - URL canonicalization now strictly rejects paths containing null bytes (`\0` or `%00`).
- **Cache cleanliness** - `X-LiteSpeed-Cache` headers are no longer emitted by default; can be re-enabled via the `emit_litespeed_headers` subdirective.

#### Observability & tracing

- **Unified request tracing** - HTTP tracing now rolls up into a single `ferron.request` root span with nested child spans for pipelines, stages, file-serving, and error pipelines.
- **Log correlation** - OTLP request and access logs now include the active request span context out of the box.
- **Backend exporting** - admin API metrics are now also pushed directly to configured observability backends, not just the local admin endpoint.
- **Cardinality control** - Prometheus label values are now sanitized to heavily reduce high-cardinality label inflation.

#### Core runtime

- **Unified durations** - improved configuration-wide consistency for duration formatting values.
- **Graceful shutdown** - the server process now handles standard Unix `SIGTERM` signals for seamless graceful shutdowns.
- **Frictionless local TLS** - the server now issues a clean warning if local automatic TLS is configured but the cache directory isn't writable, instead of refusing to boot.

### Fixed

#### Reverse proxy

- Fixed a bug where pool limits weren't being respected when pulling connections from the pool, leading to possible handle exhaustion.
- Fixed a bug where reverse proxy boolean subdirectives with empty values (implying `true`) were being ignored.
- Fixed an issue where the proxy failed to strip headers specified by the `Connection` header per RFC 7230.

#### DNS & ACME

- Fixed an "invalid socket address" error that broke RFC 2136 dynamic DNS updates for the ACME DNS-01 challenge.
- Resolved an edge case where OCSP stapling failed to immediately fetch the response after new certificates were registered.

#### HTTP server core

- **Auth bypass closed** - fixed a critical flaw where a misconfigured forwarded authentication block could result in bypassing authentication entirely.
- **Cache thundering herd fixed** - implemented request coalescing for cache misses to prevent thundering herd scenarios when multiple requests hit the same uncached resource simultaneously.
- Fixed a `500 Internal Server Error` when using the `auth_to { ... }` syntax inside forwarded authentication blocks.
- Fixed a bug where case-insensitive HTTP cache control directives were not recognized correctly.
- Fixed a bug where CONNECT requests with authority-form URIs were erroneously blocked by the URL canonicalizer.
- Fixed a bug where HTTP timeout durations were not being respected correctly.
- Fixed a bug where TLS certificate resolver from domain name level higher (non-wildcard) was incorrectly used for TLS handshakes if the one from the domain name level matching the requested SNI isn't present.
- Fixed HTTP-to-HTTPS redirects to correctly target the original requested URL rather than internal rewritten URLs.

#### Observability

- **Log injection fixed** - implemented strict sanitization of log fields to prevent potential log injection attacks for plain-text logs via malicious header values or other user input.
- Fixed a data blind spot where malformed and timed-out requests rejected before normal handler completion went uncounted by the observability pipeline.
- Corrected inaccurate memory metrics that were calculating values relative to initial memory usage instead of absolute usage.

#### Security

- **DNS rebinding fixed** - closed a vulnerability where forward-proxy DNS validation could be bypassed via a race condition combined with a DNS rebinding attack.
- Fixed an issue where forward-proxy allowed ports and denied IP addresses were being treated as additive rather than strictly respecting user configuration overrides.

#### Runtime operations

- Fixed an Admin API-initiated reload loop that caused infinite configuration reload loops.
- Fixed admin API over-redacting configuration directives in `/config` endpoint responses, which could lead to confusion when verifying configuration values.
- Eliminated a rate limiting race condition when initializing a brand new key bucket, which previously allowed traffic to briefly exceed configured capacities.
- Fixed manual TLS session ticket key rotation to properly read from configured key files instead of silently falling back to in-memory generation.
- Fixed a bug on Linux where `io_uring` could not be explicitly disabled through the server configuration file.
- Fixed an edge case where cached responses replaced by non-cached default error pages could be returned stale.

## Ferron 3.0.0-beta.1

## Released in May 5, 2026

### Added

- CLI utility for hashing passwords.
- CLI utility for pre-compressing static files.
- CLI utility for translating Ferron 2 configurations into Ferron 3 ones.
- CLI utility for zero-configuration serving.

### Changed

- Non-existent webroots now lead to 404 Not Found errors instead of 500 Internal Server Error errors.

### Fixed

- Partial hostname resolution match in HTTP server could lead to incorrect routing.
- Redirects configured with `status` directive didn't have some placeholder locations (such as `$1`) replaced when using a regex match.
- Redirects configured with `status` directive didn't lead to any destination.
- Reverse proxy was sometimes routed to wrong backend server.
- Some default cache paths were unwritable in some cases.
- Unknown directives in global blocks for `status` directive (even though they're known in host blocks) caused the web server to fail to start.
- When using OTLP, access logs were emitted with "access_log" body, not actual access logs.

## Ferron 3.0.0-alpha.3

**Released in April 23, 2026**

### Added

- Multiple DNS providers for DNS-01 ACME challenge.
- Support for CGI (Common Gateway Interface).
- Support for FastCGI (including PHP-FPM).
- Support for forwarded authentication.
- Support for SCGI (Simple Common Gateway Interface).

### Fixed

- Connections to HTTP/2 backends in reverse proxy were aborted.
- Error interception configuration blocks weren't applied properly.
- HTTP to HTTPS redirect wasn't enabled by default.
- HTTP upgrades weren't performed properly.
- HTTP-01 ACME challenge failed due to challenge not being served for implicit automatic TLS.
- On-demand automatic TLS wasn't functioning correctly.
- Some ACME events were logged only to the console, not observability backends.
- TLS certificates with local CA weren't checked if they're expired.

## Ferron 3.0.0-alpha.2

**Released in April 17, 2026**

### Added

- Active health checking in reverse proxy support.
- Automatic TLS with local certificate authority (CA).
- Experimental HTTP/3 support.
- `map` directive for mapping variables.
- Prometheus metrics export support.
- Response body string replacement support.
- Support for body interpolation in `status` directives.
- Support for interpolated strings in header values.
- W3C Trace Context (traceparent and tracestate) propagation and generation.

### Changed

- Improved the request URL normalization.
- Requests with multiple Host headers are now rejected.

### Fixed

- PROXY protocol setting, connection retry setting and error interception weren't working for reverse proxy.
- Zerocopy static file serving wasn't working properly on Linux, because it wasn't enabled.

## Ferron 3.0.0-alpha.1

**Released in April 10, 2026**

### Changed

- First alpha release of Ferron 3.
