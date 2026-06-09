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

- **`http-abuseban`** - a new module for lightweight, native Fail2ban-like IP banning with temporary lockouts based on rate limit breaches and brute-force failures.
- **`http-traceid`** - a new module that injects the current request's trace ID into HTTP response headers (configurable header name, optional on-demand reflection via `X-Ferron-Trace-Reflect`).
- **`tls-http`** - support for obtaining TLS certificates from a remote HTTP endpoint, featuring automatic refresh cycles and dedicated observability metrics.
- **OS metrics** - added Windows support for native process metrics in the `metrics-process` module.

#### Reverse proxy & load balancing

- **`circuit_breaker`** - new directive with rolling failure windows, temporary backend ejection, and half-open recovery states.
- **`p2c_ewma`** - a new adaptive, latency-aware load balancing algorithm combining Power of Two Choices with Exponentially Weighted Moving Average (EWMA) latency scoring.
- **Session affinity (sticky sessions)** - native support for `cookie`, `header`, `ip`, and `hash` types utilizing a Ketama-style hash ring for deterministic backend routing.
- **String interpolation** - upstream URLs and Unix socket paths now support interpolated strings.
- **SRV routing** - added active health check support for SRV upstream URLs.
- **Outbound mTLS** - reverse proxy upstreams now support presenting client certificates (`cert` and `key` subdirectives) for mutual TLS authentication with HTTPS backends, including per-upstream credential scoping and SRV upstream support.

#### DNS & ACME

- **58 new DNS providers** - native ACME DNS-01 challenge support expanded to include Alibaba Cloud DNS, Azure DNS, ClouDNS, Hetzner DNS, Oracle Cloud DNS, Vercel, Vultr, Yandex Cloud DNS, and 50+ others.
- **CLI arguments in post-obtain command** - added support for passing CLI arguments to the post-obtain command in automatic TLS configurations.

#### HTTP server core

- **`basic_auth_concurrency`** - global directive to limit concurrent, resource-heavy password verification tasks across all `basic_auth` blocks.
- **Cache purging** - native `PURGE` HTTP method support for targeted cache invalidation via the `purge_method` and `purge_allowed_ips` subdirectives.
- **`force_trace`** - global directive to force trace context creation for every request even when tracing is not explicitly enabled by a module, useful for debugging or log correlation.

#### Observability & metrics

- **Edge-case visibility** - granular HTTP observability metrics for pre-handler failures, server redirects, client-IP rewrites, CORS preflights, connection lifecycle failures, forward-proxy outcomes, reverse-proxy failures, and static-file response outcomes.
- **Admin sinks** - added a dropped-events admin metric for non-blocking observability sinks.
- **Unix file descriptor metrics** - added `process.unix.file_descriptor.count` (UpDownCounter) to track the change in number of unix file descriptors since the last measurement. (Linux only)
- **Backend exclusion tracking** - new `ferron.proxy.backend.excluded` counter that records each exclusion event with `backend_url` and `reason` attributes, covering passive-check failures, open circuit breakers, retry exclusion, and overloaded backends.
- **Retry observability** - new `ferron.proxy.retry.count` counter and `ferron.proxy.retry.final` gauge to track retry attempts and whether the final attempt succeeded.
- **Connection pool metrics** - new `ferron.proxy.pool.hit` and `ferron.proxy.pool.miss` counters, plus `ferron.proxy.pool.idle` and `ferron.proxy.pool.outstanding` worker-scoped gauges for per-upstream connection pool depth monitoring.
- **Connect latency & TTFB** - new `ferron.proxy.connect.latency` and `ferron.proxy.ttfb` histograms measuring TCP connection establishment time and time to first response byte.
- **Health check instrumentation** - new `ferron.proxy.health.success`, `ferron.proxy.health.failure`, and `ferron.proxy.health.duration` metrics for active health probe outcomes.
- **Circuit breaker metrics** - new `ferron.proxy.circuit.state` gauge (0/1/2 for Closed/HalfOpen/Open) and `ferron.proxy.circuit.open_total` counter tracking circuit breaker state transitions.
- **Unified certificate expiration gauge** - a single `ferron.tls.certificate_not_after` gauge is emitted by every TLS provider (`manual`, `acme`, `http`, `local`) whenever a certificate is mounted into the in-memory rustls context. The value is the certificate `notAfter` field as Unix epoch seconds; attributes are `ferron.host` (SNI hostname or IP), `ferron.tls.provider` (provider name), and `crypto.certificate.serial_number` (lowercase hex). Replaces the previous provider-specific expiration gauges.
- **Baggage promotion** - new `baggage` sub-directive in observability backend blocks (`otlp`, `prometheus`) to promote specific W3C Baggage keys into telemetry attributes for logs, metrics, and traces. Supports per-signal filtering and a `max_distinct` cap to prevent high-cardinality metric label explosion.
- **Baggage propagation** - Baggage is now propagated from incoming requests to outgoing ones, allowing distributed tracing context to be preserved across service boundaries.
- **`log_style` directive** - new `log_style legacy|modern` directive in the OTLP observability backend block to opt into OTEL-style structured logs. In `modern` mode, each log record publishes a short `summary` body plus typed per-event attributes (string, bool, integer, float) instead of the human-readable message. Access logs are remapped to OTEL semantic conventions (`url.path`, `http.request.method`, `http.response.status_code`, `client.address`, `http.server.request.duration`, etc.). The default `legacy` mode preserves existing behavior.
- **Logs for OTLP errors** - Errors with the logs, metrics and traces providers are now logged.

#### Admin API

- Added `GET /reload` and `GET /runtime` endpoints to the admin listener.

#### Configuration syntax

- **Globs in `include` directives** - `include "path.conf"` now supports glob patterns, e.g. `include "/etc/ferron/conf.d/**/*.conf"`.

#### Configuration validation

- **Best-practice enforcement** - `ferron doctor` subcommand for validating best practices across the configuration.
- **Print "all good" message** - a message is now printed when the configuration is valid with no diagnostics.
- JSON configuration validation results are now printed to stdout when `--json` is specified, improving observability for automated tools and CI/CD pipelines.
- Unused subdirectives are now reported as well as directives.

#### Docker

- **Entrypoint configuration** - Docker images now include a dedicated entrypoint config with JSON console logging, trace ID forcing, and an `include /etc/ferron/conf.d/**/*.conf` directive for user-provided split configurations.

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
- **Structured log events** - every log emission site now carries a short OTEL-friendly `summary` plus typed `attributes`, enabling downstream OTLP consumers to receive structured events without changing existing console or file log output.
- **Trace ID in console and file logs** - console and file loggers now prefix log messages with `[trace=<span_id>]` when a trace context is available, enabling grep-based filtering by trace ID.

#### Core runtime

- **Unified durations** - improved configuration-wide consistency for duration formatting values.
- **Graceful shutdown** - the server process now handles standard Unix `SIGTERM` signals for seamless graceful shutdowns.
- **Frictionless local TLS** - the server now issues a clean warning if local automatic TLS is configured but the cache directory isn't writable, instead of refusing to boot.

#### Docker

- **Split configuration layout** - Docker images now use a two-file layout: `/etc/ferron/ferron.conf` as the entrypoint config and `/etc/ferron/conf.d/` for user configuration files. Docker Compose bind mounts now target the `conf.d/` directory.

#### Package configuration

- **Log rotation and structured logging** - Default package configurations now include log rotation settings and commented-out structured logging and trace ID format options.

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
- **Several HTTP/1.x protocol handling bugs fixed** - fixed chunked-length encoding DoS vulnerability and other protocol-related issues (see [`vibeio-http` changelog](https://github.com/ferronweb/vibeio-http/blob/main/CHANGELOG.md#vibeio-http-032)).
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
