# Ferron 3 change log

## Ferron UNRELEASED

**Not released yet**

### Added

- Windows support for process metrics in the `metrics-process` module.
- A dropped-events admin metric for non-blocking observability sinks.
- `abuse_protection` module for lightweight Fail2ban-like IP banning with temporary lockouts based on rate limit breaches and brute-force failures.
- `basic_auth_concurrency` global directive to limit concurrent password verification tasks across all `basic_auth` blocks.
- `PURGE` HTTP method support for cache invalidation via `purge_method` and `purge_allowed_ips` subdirectives in the `cache` block.
- HTTP observability metrics for pre-handler request failures, server redirects, client-IP rewrites, CORS preflights, connection lifecycle failures, forward-proxy outcomes, reverse-proxy failures, and static-file response outcomes.
- Session affinity (sticky sessions) support for reverse proxy with `cookie`, `header`, `ip`, and `hash` affinity types.
- `consistent_hash` load balancing algorithm using a Ketama-style hash ring for deterministic backend selection.
- Support for interpolated strings in reverse proxy upstream URLs and Unix socket paths.
- `circuit_breaker` reverse proxy directive with rolling failure windows, temporary backend ejection, and half-open recovery.
- `weighted_round_robin` load balancing algorithm with per-upstream `weight` directive for proportional traffic distribution.
- `tls-http` module for obtaining TLS certificates from a remote HTTP endpoint, with automatic refresh and observability metrics.
- 58 newly-supported DNS providers for the ACME DNS-01 challenge: Alibaba Cloud DNS, ArvanCloud, AutoDNS, Azure DNS, Baidu Cloud DNS, BlueCat Address Manager v2, ClouDNS, Constellix, cPanel, DDNSS.de, DNS Made Easy, Domeneshop, DreamHost, DuckDNS, Dynu, EasyDNS, Akamai Edge DNS, Exoscale, FreeMyIP, Gandi v5, Gcore, GleSYS, GoDaddy, Hetzner DNS, hosting.de, Hostinger, Huawei Cloud DNS, Hurricane Electric, IBM Cloud, Infoblox NIOS, Infomaniak, INWX, IONOS, IPv64, Joker, AWS Lightsail, Linode, LuaDNS, Mythic Beasts, Namecheap, Name.com, NameSilo, netcup, Netlify, Nifcloud, NS1, Oracle Cloud DNS, Plesk, ANS SafeDNS, Scaleway, Tencent Cloud DNSPod, TransIP, UltraDNS, Vercel, Vultr, Websupport, Volcano Engine, and Yandex Cloud DNS.

### Changed

- Admin API metrics are now also emitted to observability backends, not just the admin status endpoint.
- Brute-force protection now uses IP-based locking instead of username-based locking, preventing locking out users.
- Configuration failures when reloading the server no longer cause the server to stop; instead, they are logged and the server continues to run.
- File pipeline now returns a 408 Request Timeout status code when the request takes too long, instead of a 404 Not Found status code.
- Forwarded authentication now supports interpolated string values for the backend URL.
- HTTP Basic Auth now return a 429 Too Many Requests status code when the user has exceeded the maximum number of failed attempts.
- HTTP tracing now uses a single `ferron.request` root span with nested pipeline, stage, file-serving, and error-pipeline spans.
- Improved consistency for duration values across the configuration.
- Improved error reporting for some TLS handshake failures.
- Improved error reporting for local automatic TLS failures.
- OCSP responses are now verified before being cached and stapled.
- OTLP request logs and access logs now include the active request span context for correlation with exported traces.
- Prometheus label values are now sanitized to reduce high-cardinality labels.
- Some file serving errors are now handled more gracefully, returning a 403 Forbidden (for permission denied) or 400 Bad Request (for invalid filename or too long one) status code instead of a generic 500 Internal Server Error.
- Syntax for rate limit window has been updated to use duration strings (**potentially breaking**).
- Syntax for passive and active health checks, load balancing algorithm, and connection retries has been updated to use a more consistent and readable format (**potentially breaking**).
- The web server now warns when local automatic TLS is configured but the cache directory isn't writable, instead of straight-up failing to start.
- The web server process now performs graceful shutdown when SIGTERM is sent to the process on Unix.
- URL canonicalization now rejects paths containing null bytes (`\0` or `%00`).
- `X-LiteSpeed-Cache` headers aren't emitted by default anymore; this can be still enabled using `emit_litespeed_headers` subdirective in `cache` directive.

### Fixed

- Admin API-initiated reload would trigger configuration reload loops.
- Cached responses which are replaced by non-cached default error pages might have been returned as stale.
- CONNECT requests with authority-form URIs were rejected by the URL canonicalizer.
- Forward-proxy allowed ports were additive (meaning that ports 80 and 443 were always included).
- Forward-proxy denied IP addresses were additive (meaning that RFC 1918 private IP ranges were always included).
- Forward-proxy DNS validation could be bypassed by performing a DNS rebinding attack (along with exploiting a race condition) against the configured allowed hostnames.
- Forwarded authentication would fail with 500 Internal Server Error when using `auth_to { ... }` syntax.
- HTTP-to-HTTPS redirects used rewritten URLs instead of the original URL.
- Manual TLS session ticket key rotation didn't use the session ticket key files, instead using in-memory key generation.
- Malformed and timed-out requests rejected before normal handler completion are now counted by Ferron's observability pipeline.
- `io_uring` (on Linux) couldn't be disabled via the web server configuration.
- Memory usage metrics were inaccurate (relative to the initial memory usage instead of absolute one).
- Misconfigured forwarded authentication could lead to completely bypassing the authentication.
- OCSP stapling might not have fetched the OCSP response immediately after new certificates were added.
- Rate limiting had a race condition when first creating a new bucket for a key, which could lead to allowing more requests than the configured capacity.
- Reverse proxy boolean subdirectives with empty values (implying `true`) weren't effective.
- Reverse proxy didn't remove headers as indicated by the "Connection" header, per RFC 7230.
- RFC 2136 dynamic DNS updates for ACME DNS-01 challenge didn't work due to "invalid socket address" errors even when configured correctly.

## Ferron 3.0.0-beta.1

**Released in May 5, 2026**

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
