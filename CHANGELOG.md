# Ferron 3 change log

## Ferron UNRELEASED

**Not yet released**

### Added

#### Observability & tracing

- **Process identity in OTel resources** — the OTLP resource now automatically includes `process.pid` and `process.start_time` attributes, allowing observability backends to distinguish between concurrent and sequential process lifetimes. This prevents cumulative counters from adjacent process lifetimes from being visually interleaved in dashboards.
- **Status-code labels on cache store counters** — the `ferron.cache.stores` metric now includes an `http.response.status_code` attribute, allowing operators to directly query whether error responses are entering the cache layer without correlating timing across separate request and error series.

## Ferron 3.0.0-beta.5

**Released in July 5, 2026**

### Added

#### Security

- **Configurable symlink traversal protection** — the new `disable_symlinks` directive allows configuration of symlink handling during static file serving (`false`, `true`, or `if_not_owner` on Unix). When enabled, symlinks are detected without following them during path traversal, preventing symlink-based escape attacks from outside the configured webroot. Defaults to `false` for backward compatibility.
- **Error rate threshold** — the `abuse_protection` directive now supports an `error_rate_threshold` sub-directive that monitors HTTP response status codes. When a client triggers an abnormal number of configured error responses (e.g., 404, 403) within a time window, their IP is temporarily banned. This detects hostile scanning behavior such as probing for old vulnerabilities or non-existent plugin paths.

#### Observability & tracing

- **Per-host TLS handshake metrics** — new per-host metrics for TLS handshake duration (`ferron.tls.handshake.duration` histogram), handshake count (`ferron.tls.handshake.total` counter), and active connections (`ferron.tls.connections.active` up-down counter). Each metric carries `ferron.host`, `tls.protocol.version`, and `tls.cipher_suite` attributes, enabling per-host visibility into TLS performance and client compatibility.
- **TLS attributes on request traces** — the `ferron.request` root span now includes `tls.protocol.version` and `tls.cipher_suite` attributes for HTTPS connections, correlating TLS parameters with individual requests without requiring a separate TLS span.
- **Per-host OCSP metrics** — OCSP fetching metrics (`ferron.ocsp.fetches_total`, `ferron.ocsp.fetch_duration_seconds`) now include a `ferron.host` dimension, and a new `ferron.ocsp.stapling.hit_total` counter tracks per-request OCSP stapling outcomes per host.
- **Client and server IPs in error logs** — structured error logs for bad requests (400), request timeouts (408), TLS handshake failures, and TCP connection errors now include `client.address` and `server.address` attributes, enabling IP-based correlation and troubleshooting.
- **Load-balancer selection score metric** — new `ferron.proxy.lb.score` Gauge metric exposes the combined selection score used by the `two_random` and `p2c_ewma` load-balancing algorithms. For P2C-based algorithms, the two candidate scores compared during selection are captured and the winner's score is emitted per request. Lower scores indicate more preferred backends, enabling direct visibility into traffic distribution without manual TTFB correlation.
- **Upstream response truncation detection** — new `ferron.proxy.upstream.response_truncated` Counter metric and structured warning log detect when an upstream backend closes the response stream before the declared `Content-Length` bytes have been forwarded. The proxy tracks bytes received during body streaming and flags truncation when the stream ends prematurely. A structured warning log includes the backend URL, bytes received, and expected content length, enabling immediate visibility into backend issues that return partial responses with valid HTTP status codes. This does not capture response bodies; only the byte count and content length are tracked.
- **Latency-aware circuit breaker** — the `circuit_breaker` directive now supports an optional `latency_threshold` sub-directive. When configured, upstream responses exceeding this duration count as failures toward tripping the circuit, alongside transport failures and (optionally) `5xx` responses. This allows circuit breakers to eject pathologically slow backends that return successful status codes.
- **Circuit breaker flapping detection** — the `circuit_breaker` directive now supports `flapping_transitions` and `flapping_window` sub-directives for detecting rapidly oscillating upstream backends. When an upstream transitions between circuit breaker states more than `flapping_transitions` times (default: 3) within `flapping_window` (default: `10s`), a single warning log is emitted and the `ferron.proxy.circuit.flapping` Gauge metric is set to 1. Individual transition logs are suppressed while the upstream is flapping. When transitions stabilize, a recovery log is emitted and the metric resets to 0.
- **Circuit breaker slow-start** — the `circuit_breaker` directive now supports a `slow_start` duration sub-directive. When a backend transitions from half-open to closed (recovers), the load balancer applies a temporary virtual connection penalty that linearly decays over the configured duration (e.g., `slow_start 10s`), preventing thundering herd by preventing the recovered backend from being immediately overwhelmed with new connections.
- **Configurable metric cardinality control** — the `metrics_resolved_ip` directive (default: `false`) controls whether `ferron.proxy.backend_resolved_ip` is included in proxy metrics attributes. When disabled, metrics identify backends only by their configured URL, keeping cardinality low for environments with ephemeral IP addresses (e.g., Kubernetes). When enabled, each resolved IP becomes a distinct metric label value. This replaces the previous behavior of always emitting resolved IPs, which could cause cardinality explosion.

#### Reverse proxy

- **Weighted load balancing for all algorithms** — `random`, `two_random`, and `p2c_ewma` now support per-upstream `weight` directives for proportional traffic distribution, joining `round_robin` and `least_conn`. Higher weight values receive proportionally more requests across all five load balancing algorithms and session affinity.
- **Priority-based failover** — backends can now be assigned numeric priority values to implement tiered failover. Lower values indicate higher priority. When all backends in a tier are unavailable, the next tier is used as a fallback. SRV upstreams support priority via an additive offset applied to DNS SRV priorities.
- **SRV native weight support** — DNS SRV record weights are now used for weighted load balancing within priority tiers. Each backend's effective weight is `dns_weight × config_weight`. The config `weight` subdirective acts as a multiplier on top of DNS weights.
- **Strict DNS (A/AAAA) resolution** — static upstreams with hostnames now resolve A/AAAA records via Hickory DNS by default, creating a separate backend per resolved IP for per-IP load balancing, circuit breaking, and health checking. IP literals, `localhost`, Unix sockets, and `logical_dns true` upstreams are unaffected. Use `dns_servers` to specify custom resolver IPs.
- **DNS result caching** — resolved DNS results for both strict DNS (A/AAAA) and SRV upstreams are cached in memory with TTL-based expiry derived from the DNS response TTL. This avoids redundant DNS resolution and lock contention for high-traffic hostnames. Cache entries expire based on the minimum TTL from the DNS response records, with a 30-second fallback when no TTL is available.
- **HTTP upstream connection timeout** — the `connection_timeout` subdirective is now available for all upstreams, allowing configuration of the maximum time to establish a TCP connection to a backend. This prevents long hangs when backends are unreachable or slow to accept connections.
- **Health check logging** — health check initialization logs now include the HTTP method and URI in the log output, making it easier to debug and understand which health checks are being performed.
- **Upstream circuit transition logging** — circuit breaker state transitions (open/half-open/closed) now log the upstream address and duration of the open circuit, making it easier to diagnose and troubleshoot circuit breaker behavior.

#### Observability & tracing

- **Per-stage span attributes** — all HTTP pipeline stages now emit stage-specific span attributes on their `ferron.stage.<name>` spans, enabling detailed flame graph analysis without requiring metric export. Each module's per-stage span carries module-specific context such as upstream backend URLs, cache results, rate limit decisions, authentication outcomes, and response status codes. See each module's documentation for the full list of attributes and the [tracing reference](/docs/v3/configuration/observability/tracing#per-stage-spans).
- **Upstream state in client traces** — the `ferron.stage.reverse_proxy` span now includes upstream runtime state attributes (`ferron.proxy.upstream.circuit_state`, `ferron.proxy.upstream.is_flapping`, `ferron.proxy.upstream.health_status`, `ferron.proxy.upstream.consecutive_failures`, `ferron.proxy.upstream.active_connections`) correlating the selected backend's health, circuit breaker, and connection state with individual client requests for OTLP trace debugging.
- **Resolved IP address attributes** — reverse proxy metrics now include resolved IP addresses when strict DNS (A/AAAA) resolution is enabled. The `connect_to` field of `UpstreamInner` is exposed as `ferron.proxy.backend_resolved_ip` metric attribute for all backend-related metrics, allowing observability of the actual IP addresses backends are connected to. This complements existing `backend_url` and `backend_unix_path` attributes.
- **Module-injected access log fields** — all HTTP pipeline modules now emit debuggability attributes directly in access log lines, not just as trace span attributes. Proxy requests include `ferron.proxy.backend_url`, `ferron.proxy.connection_reused`, and `ferron.proxy.retry_count`. Cache results include `ferron.cache.result` and `ferron.cache.zone`. Rate limit decisions include `ferron.ratelimit.result` and `ferron.ratelimit.zone`. Auth outcomes include `ferron.basicauth.result` and `ferron.fauth.result`. Abuse protection includes `ferron.abuseban.action`. Static files include `ferron.static.file_path`. CGI/FastCGI/SCGI backends include their respective `backend_url` and `script_path` fields. See the [logging reference](/docs/v3/configuration/observability/logging#module-contributed-fields) for the full list.
- **Circuit breaker state in access logs** — access log lines for proxied requests now include `ferron.proxy.circuit_breaker_state` (`closed`, `open`, or `half_open`), and the `ferron.proxy.circuit.state` gauge is now emitted on every proxied request instead of only on state transitions. This ensures circuit breaker visibility in both log queries and metric dashboards.

### Changed

#### Static file serving

- **FD-based metadata and file handle reuse** — static file metadata is now obtained via `fstat`-like system calls instead of a separate `stat`-like calls, closing the TOCTOU window between path validation and metadata read. A per-thread file descriptor reuse pool with preemptive bulk expired removal eviction is implemented for handle reuse, improving performance. `HttpFileContext` now carries `file: ReusedFile` instead of `metadata: Metadata`, so all consumers derive metadata from the validated file handle.
- **Path resolution without canonicalization** — static file path resolution now uses component-level validation with simple PathBuf checks instead of costly filesystem canonicalization. This improves performance, eliminates path canonicalization overhead, and reduces the TOCTOU window by validating paths before symlink traversal checks. Combined with `disable_symlinks`, this implements nginx-style defense-in-depth protection.

#### Reverse proxy

- **Circuit breakers enabled by default** — the `circuit_breaker` directive is now enabled by default for all `proxy` configurations. Circuit breakers track transport failures (TCP connect errors, TLS errors) and optionally upstream `5xx` responses (when `record_5xx true`), then temporarily eject unstable backends from the load balancer. This change improves resilience by protecting against cascading failures without requiring explicit configuration. To disable circuit breakers, set `circuit_breaker false`.
- **`ferron.proxy.requests` metric with backend URL** — the `ferron.proxy.requests` counter metric now includes a backend URL (and related) attribute for all upstream requests, allowing observability of which backend URL handled each request.

#### Observability

- **Canonical IP addresses in logs and traces** - `client_ip_canonical` and `server_ip_canonical` are now available as OpenTelemetry-style log attributes and trace span attributes, replacing the legacy `client_ip`/`server_ip`.
- **Custom log fields** - custom log fields that have `.` in their names are used as-is in OTLP logs instead of being converted to `_` in the log attribute name. This allows for more flexible log field naming and avoids conflicts with existing attribute names.

### Fixed

#### Observability

- **Consistent log targets** - `tls-acme` and `tls-http` now use `ferron-tls-acme` and `ferron-tls-http` log targets respectively instead of `ferron_tls_acme` and `ferron_tls_http`.
- **Circuit breaker gauge value fix** - the `ferron.proxy.circuit.state` gauge now correctly uses `1` for `open` and `2` for `half_open`, matching the constant definitions. Previously these values were swapped.

## Ferron 3.0.0-beta.4

**Released in June 28, 2026**

### Added

#### HTTP caching

- **Cache zones** — the `cache` directive now supports a `zone` subdirective at host scope, allowing multiple hostnames to share a single in-memory cache store. Named zones can be pre-configured at global scope with custom `max_entries` capacity. When a global `cache { max_entries }` block exists without explicit `zone` blocks, all hosts share a global zone by default. Hosts can opt out with an explicit `zone` directive.
- **Generation-aware cache capacity** — `max_entries` is now only evaluated on configuration reload instead of on every request, preventing premature LRU eviction when different host blocks specify different capacities for the same zone.
- **Zone attribute in cache metrics** — all cache metrics (`ferron.cache.requests`, `ferron.cache.entries`, `ferron.cache.stores`, `ferron.cache.evictions`, `ferron.cache.purges`) now include a `ferron.cache.zone` attribute identifying the zone the request belongs to.

#### Rate limiting

- **Rate limit zones** — the `rate_limit` directive now supports a `zone` subdirective at host scope, allowing multiple hostnames to share the same rate limit token bucket registries. Named zones can be defined at global scope. When a global `rate_limit` block exists without explicit `zone` blocks, all hosts share a global zone by default. Hosts can opt out with their own `rate_limit` block.
- **Key extractor in fingerprint** — the rate limit fingerprint now includes the key extractor type (`ip`, `uri`, `header`), so rules with different key types no longer share the same registry.
- **Zone attribute in rate limit metrics** — rate limit metrics (`ferron.ratelimit.rejected`, `ferron.ratelimit.allowed`) and structured log events now include a `ferron.ratelimit.zone` attribute identifying the zone the request belongs to.

#### Static file serving

- **`If-Range` support** — The `If-Range` header is now supported for conditional range requests (RFC 7233 §3.2), allowing clients to receive a partial response when the entity tag or modification date matches, or a full response when the representation has changed.

### Fixed

#### Reverse proxy

- **SRV priority/weight swap fix** — SRV upstream resolution was reading the priority and weight fields in the wrong order, causing traffic to be routed to the wrong priority group. Priority and weight are now correctly interpreted per RFC 2782.
- **Upgrade connection pool leak fix** — HTTP 101 upgrade connections (WebSocket, etc.) were not decrementing the pool outstanding counter, permanently reducing available pool capacity by one per upgrade. The pool item now drops correctly, releasing the slot.
- **Hop-by-hop header stripping on requests** — the outgoing proxy request now strips hop-by-hop headers (`Connection`, `Keep-Alive`, `Transfer-Encoding`, `TE`, `Trailer`, `Proxy-Authorization`, `Proxy-Authenticate`) per RFC 7230 §6.1, preventing clients from injecting these headers to influence backend behavior.

#### Static file serving

- **HTTP range requests fix** — HTTP range requests now correctly handle out-of-bounds ranges, returning `206 Partial Content` with the available range instead of `416 Range Not Satisfiable` (per RFC 7233 §2.1).
- **On-the-fly compression allowed while using precompressed files** — When serving precompressed files, the content is now compressed on-the-fly if precompressed files are not available, improving performance and reducing disk usage.
- **Multipart byterange fix** — Multipart range responses now include the required `bytes ` prefix in `Content-Range` part headers (per RFC 7233 §4.1) and correctly serve one additional byte per range to match inclusive-to-exclusive bounds (`is_end_stream` no longer signals end-of-stream while the last range's data is still being streamed).
- **`If-None-Match` POST fix** — POST requests with a matching `If-None-Match` header now correctly return `412 Precondition Failed` instead of `200 OK` (per RFC 7232 §4.2).
- **`If-Match: *` with non-GET/HEAD fix** — `If-Match: *` now correctly passes for POST and other non-GET/HEAD requests when a representation exists, per RFC 7232 §3.1.
- **Invalid Range header handling** — Syntactically invalid `Range` headers are now treated as absent (returning `200 OK`) instead of `416 Range Not Satisfiable`, per RFC 7233 §3.1.
- **`file_cache_control` panic fix** — Malformed `file_cache_control` config values can no longer panic the request handler; a validation warning now catches invalid characters.
- **Missing `Content-Length` for compressed responses** — Non-identity (compressed and precompressed) file responses now include the correct `Content-Length` header.
- **`bytes_sent` metric fix** — The `bytes_sent` metric now reports the actual compressed file size for precompressed responses instead of the original file size.

#### HTTP caching

- **Cache revalidation fix** - when a cached response is revalidated, the HTTP status code is now preserved from the cached response instead of always using `200 OK`.
- **Abuse protection fix** - 403 Forbidden responses generated by abuse protection module are no longer cached (as they are served before the cache).
- **Cache scope fix** - in the previous version (Ferron 3.0.0-beta.3), caches were per-host instead of global, even when maximum cache items subdirective was global-only. This has been fixed to restore the behavior before 3.0.0-beta.3.
- **Stale-while-revalidate inflight request fix** - when a stale response is revalidated, the inflight request handling no longer causes possible hangs.
- **`Expires` header in past edge case fix** - a response with a past `Expires` header and no `Cache-Control` directives was incorrectly cached for 5 minutes. This has been fixed to cache the response for 0 seconds (expires immediately) instead.
- **Stale-if-error correctness fix** - a stale response is no longer served when `must-revalidate` is set, per RFC 9111 §4.3.4.
- **Cache key fix** - an attacker could craft a cookie value containing `&cookie:` to collide with another user's cache key. This severe cache collision was fixed by using `\0` as the separator instead of `&` in the cache key.
- **304 revalidation TTL fix** - when a cached response is revalidated via a `304 Not Modified` response, the stored response's TTL, `stale-while-revalidate`, `stale-if-error`, and `must-revalidate` directives are now updated from the 304 response's `Cache-Control` headers per RFC 9111 §4.3.4. Previously only `ETag` and `Last-Modified` validators were updated, causing the original TTL to persist even when the upstream sent a different `max-age`.
- **Cache variant metadata leak fix** - the `variants_by_base` map used for cache lookups previously grew without bound, retaining variant records for all URLs ever cached even after entries were purged. This map is now cleaned up when all entries for a base key are removed via purge.

## Ferron 3.0.0-beta.3

**Released in June 26, 2026**

### Breaking changes

If you are upgrading to this beta version, you must update your configuration files to accommodate the following syntax refactors:

- **Removed `force_trace` directive** - the `force_trace` directive has been removed as trace context creation is now always enabled for every request. Configurations using `force_trace true` should remove the directive; configurations using `force_trace false` should also remove it since the behavior now matches the default.
- **Default text log format changed** - the default text access log format has been changed from Combined Log Format to Enhanced Combined Log Format, which adds `Host` header and trace ID fields. If you rely on the default text log format and want to keep the previous behavior, explicitly set the pattern to the Combined Log Format using `access_pattern "%client_ip - %auth_user [%t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\""` in your `log` block.
- **Circuit breaker `5xx` responses no longer counted by default** - upstream `5xx` responses no longer trip the circuit breaker unless `record_5xx true` is explicitly set. This makes the circuit breaker purely transport-failure-based by default, giving operators explicit control over Layer 7 error sensitivity.
- **`passive_check` directive removed** - the `passive_check` directive has been replaced with a more generic `circuit_breaker` directive that supports passive health checking and circuit breaking for both HTTP and gRPC services.

### Added

#### Modules

- **`http-variables`** - a new module for setting interpolation variables based on request conditions and mapping variables to custom access log fields.

#### Security

- **Custom abuse event registration** - it's now possible to register custom abuse events, for example, for honeypots.

#### Observability & metrics

- **Reload metrics and logs** - the new `metrics-reload` module emits application log events and a `ferron.reloads` counter metric around configuration reloads, with a `ferron.reload.successful` attribute indicating success or failure and an `error.message` attribute on failures.
- **Abuse ban logging** - the abuse ban logger now logs at the WARN level when an IP is triggered for banning, including the banned IP address and reason.

#### HTTP server core

- **`set_var` directive** - new directive for setting interpolation variables based on regex matching against request attributes, with optional `value`, `case_insensitive`, and `negate` subdirectives. (`http-variables`)
- **`log_field` directive** - new directive for mapping any resolvable variable (including interpolated strings) to custom access log field names, evaluated after the response is generated. (`http-variables`)
- **Support for date-based caching** - the `If-Modified-Since` header is now supported for static file responses, allowing clients to cache responses and avoid unnecessary re-downloads when the file has not been modified.
- **Auth user variable support** - the `auth.user` variable is now available in variable interpolations for the HTTP server.
- **Trace ID and span ID variable support** - the `trace.id` and `trace.spanid` variables are now available in variable interpolations for the HTTP server.

#### HTTP caching

- **RFC 9111 conditional request revalidation** - when a client sends `Cache-Control: max-age=0` or `no-cache`, the HTTP cache now performs conditional revalidation using stored `ETag` and `Last-Modified` validators instead of bypassing the cache entirely. If the upstream returns `304 Not Modified`, the cached response body is served with fresh headers. If the upstream returns `200 OK` (content changed), the cached entry is replaced. (`http-cache`)
- **`stale-while-revalidate` support** - when a cached response's `max-age` has expired but a `stale-while-revalidate` directive is present, the stale response is served immediately to the client while the cache entry remains available within the stale window. This avoids latency spikes for expired cache entries. (`http-cache`)
- **`stale-if-error` support** - when an upstream revalidation request returns a `5xx` error and a `stale-if-error` directive is present on the cached response, the stale cached response is served instead of the error. This provides resilience against backend failures. (`http-cache`)
- **`must-revalidate` and `proxy-revalidate` enforcement** - entries with `must-revalidate` or `proxy-revalidate` directives (or `s-maxage`, which implies `proxy-revalidate`) are never served stale, even within `stale-while-revalidate` or `stale-if-error` windows. (`http-cache`)
- **Ignore request cache control** - when `ignore_request_cache_control` in `cache` is enabled, request-based cache control (e.g., `Cache-Control: no-cache`) is ignored in favor of the configured cache policy.
- **Cache purge propagation** - when a cache purge occurs (via `PURGE` method or `X-LiteSpeed-Purge` header), the purge can be propagated to other instances via an external control-plane service. Edge instances send webhook POST requests to a configured control-plane URL, which broadcasts `PURGE` requests to all other registered edges. Loop prevention is handled via the `X-Purge-Source: propagation` header and origin exclusion. (`http-cache`)

#### Automatic TLS

- **HTTP auth for on-demand ask endpoint** - when `on_demand_ask_auth` is configured, HTTP Basic Auth credentials are used for the on-demand ask endpoint, allowing for secure access to the endpoint.
- **Fallback ACME providers** — the `fallback` directive allows configuring multiple ACME providers with sequential failover. When the primary provider fails (e.g., CA outage), Ferron automatically tries the next configured fallback. Each provider maintains its own cached credentials. (`tls-acme`)

#### HTTP TLS provider

- **On-demand certificate fetching** — the `tls-http` module now supports on-demand (lazy) certificate retrieval. When `on_demand true` is set in a wildcard host block (e.g., `*:443`), certificates are fetched on the first TLS handshake for each SNI hostname, rather than polling a single endpoint at startup.
- **On-demand approval endpoint** — the new `on_demand_ask`, `on_demand_ask_auth`, and `on_demand_ask_no_verification` directives control an authorization endpoint that is consulted before fetching a certificate for a hostname. The hostname is sent as a `?domain=<encoded>` query parameter.
- **Per-SNI certificate refresh** — on-demand fetched certificates are individually refreshed at the configured `refresh_interval`, ensuring certificate rotation is applied per hostname without affecting other SNIs.

#### Utilities

- **Formatter utility** - a new `ferron-fmt` command-line tool is available for formatting and validating Ferron configuration files.

### Changed

#### Admin API

- **Reload failure status** - the `/reload` endpoint now returns an error message in the response body when reload fails. Previously, it only indicated that reloading was initiated without any details about why it failed.

#### Observability & logging

- **Improved error reporting in structured logs** - reverse proxy, HTTP, TCP and QUIC error logs now include an `error.message` attribute when available. Also, some of the reverse proxy error log summaries being overly long have been fixed.
- **OCSP response caching log improvements** - OCSP response caching logs are now emitted at info level by default. The `ferron.ocsp.cert.primary_san` attribute is also included in the log message when available.
- **Trace context always enabled** - the `force_trace` directive has been removed, as trace context creation is now always enabled for every request. Additionally, when no tracing export is configured, Ferron will not generate a new span ID for incoming requests, avoiding unnecessary overhead.

#### TLS & ACME

- **ACME provisioning cycle log improvements** - ACME provisioning logs are now no longer emitted at debug level when a certificate is still valid or loaded from cache, as these messages were redundant with the `ferron.acme.domains` attribute already included in the main log message.

#### Proxying

- **Forward proxying CONNECT cycle log improvements** - CONNECT connection closure logs are now no longer emitted at debug level when a tunnel is closed to reduce noise in the application logs.
- **Circuit breaker `record_5xx` toggle** - the `circuit_breaker` directive now supports a `record_5xx` subdirective (default: `false`) to control whether upstream `5xx` responses count toward tripping the circuit.

### Fixed

#### Observability & logging

- **HTTP/2 and /3 host header log visbility fix** - HTTP/2 and HTTP/3 requests now log the `host` header correctly, ensuring visibility into the host name of the server that handled the request.

#### TLS

- **OCSP stapling ECDSA issuer fix** - when using ECDSA certificates for OCSP stapling, the issuer OID was not being correctly extracted from the certificate, leading to incorrect validation. This has been fixed to ensure the issuer OID is extracted correctly and used for OCSP validation.
- **TLS certificate provider inheritance fix** - When using TLS certificates provided by a parent configuration, the certificate provider was incorrectly inherited, leading to errors when attempting to use these certificates in child configurations. This has been fixed to ensure that the certificate provider is correctly inherited and available for use in all child configurations.

#### HTTP server core

- **Error configuration routing fix** - error configuration routing were resolved with a wrong configuration, when using a wildcard hostname in the configuration, even when no error configuration is set. This has been fixed to ensure wildcard hostnames are correctly handled.

#### Server startup

- **Critical Unix daemon initialization failure fix** - when initializing as a Unix daemon, the forked process would exit due to logging re-initialization issues. This has been fixed to ensure that the parent process continues running even in such cases. ([GitHub issue](https://github.com/ferronweb/ferron/issues/774))

## Ferron 3.0.0-beta.2

**Released in June 14, 2026**

### Breaking changes

If you are upgrading to this beta version, you must update your configuration files to accommodate the following syntax refactors:

- **Rate limit windows** - syntax updated to enforce standard duration strings (e.g., `10s`, `5m`, `1h`).
- **OTLP verification** - `no_verify` has been renamed to `no_verification` and now operates strictly as a configuration flag.
- **OTLP defaults** - OTLP sink now uses `log_style modern` default, which may break existing configurations with custom OTLP log formats.
- **Proxy configuration** - syntax for passive/active health checks, load balancing algorithms, and connection retries has been unified into a cleaner, more consistent format.
- **Incoming trace context discarded by default** - Ferron no longer trusts incoming `traceparent`, `tracestate`, or `baggage` headers unless `trust_request true` is explicitly set in the `trace` block. Previously, incoming trace context was always parsed and used as the parent span. To restore the old behavior, add `trust_request true` to your `http { trace { ... } }` block.

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
- **`trust_request`** - new directive in the `trace` block to accept incoming `traceparent`, `tracestate`, and `baggage` headers as the parent trace context. Default: `false`. When disabled (default), incoming trace headers are discarded and Ferron generates a fresh trace ID for each request.
- **Optimal server-preferred compression algorithm** - now supports automatic detection of the optimal compression algorithm based on client preferences ([GitHub issue](https://github.com/ferronweb/ferron/issues/709)).

#### Authentication

- **Forwarded authentication backend chaining** - the forwarded authentication module now supports chaining multiple backends together, allowing for flexible and secure multi-backend authentication scenarios.

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
- **CGI/FastCGI/SCGI observability metrics** - added request count, failure count, upstream duration, and stderr error metrics for the `http-fcgi`, `http-cgi`, and `http-scgi` modules. FastCGI also exposes connection pool wait and upstream duration metrics; CGI exposes process duration and exit code metrics; SCGI exposes upstream duration metrics.
- **`log_style` directive** - new `log_style legacy|modern` directive in the OTLP observability backend block to opt into OTEL-style structured logs. In `modern` mode, each log record publishes a short `summary` body plus typed per-event attributes (string, bool, integer, float) instead of the human-readable message. Access logs are remapped to OTEL semantic conventions (`url.path`, `http.request.method`, `http.response.status_code`, `client.address`, `http.server.request.duration`, etc.). The default `legacy` mode preserves existing behavior.
- **Application log formatters via `error_format`** - the `error_log` directive and `provider file` now support an `error_format` subdirective to choose how application (error) log messages are formatted. The `text` formatter (default) produces human-readable lines with timestamp, level, optional trace ID, and message. The `json` formatter produces structured JSON records with `summary`, `level`, `target`, `attributes`, and `trace_context` fields. Formatter implementations are provided by the `logformat-text` and `logformat-json` modules respectively.
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
- **`/var/cache/ferron-acme` prioritized over user's cache dir** - for Docker images with Ferron running as `root`, the ACME cache directory is now `/var/cache/ferron-acme`, which takes precedence over user's ACME cache directory.

#### HTTP server core

- **Safe configuration reloads** - configuration failures during a live reload no longer crash or stop the server; errors are logged, and execution safely continues using the previous valid configuration.
- **Smarter brute-force locking** - protection now locks by **IP address** instead of username, preventing malicious actors from intentionally locking out legitimate users.
- **Accurate status codes** - refactored error handling to return context-aware status codes over generic `500` or `404` errors:
  - File pipeline returns `408 Request Timeout` if a request takes too long (instead of `404`).
  - Basic Auth returns `429 Too Many Requests` when max failed attempts are reached.
  - File serving errors return `403 Forbidden` (permissions) or `400 Bad Request` (invalid filenames) instead of `500`.
- **Support for multi-range partial static file serving** - now supports serving multiple ranges from a single request, improving efficiency and reducing latency.
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
- **Trace context injection for CGI/FastCGI/PHP-FPM/SCGI** - trace context headers are now automatically propagated to the backend environment via CGI/FastCGI/PHP-FPM/SCGI.
- **Incoming trace context discarded by default** - Ferron now generates a fresh trace ID for every request by default, discarding incoming `traceparent`/`tracestate`/`baggage` headers unless `trust_request true` is configured. Existing deployments relying on incoming trace context must add `trust_request true` to the `trace` block.
- **Sanitized trace header injection** - `inject_trace_headers` now removes any existing `traceparent`, `tracestate`, and `baggage` headers before injecting new values, preventing header duplication or conflicts.

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
- Fixed a bug where HTTP connections couldn't be accepted on 32-bit Windows (see [`vibeio` changelog](https://github.com/ferronweb/vibeio/blob/main/CHANGELOG.md#vibeio-0213); [GitHub issue](https://github.com/ferronweb/ferron/issues/662)).
- Fixed a bug where HTTP connections were accepted by only one OS thread on Windows.
- Fixed a bug where HTTP timeout durations were not being respected correctly.
- Fixed a bug where TLS certificate resolver from domain name level higher (non-wildcard) was incorrectly used for TLS handshakes if the one from the domain name level matching the requested SNI isn't present.
- Fixed HTTP-to-HTTPS redirects to correctly target the original requested URL rather than internal rewritten URLs.

#### Observability

- **Log injection fixed** - implemented strict sanitization of log fields to prevent potential log injection attacks for plain-text logs via malicious header values or other user input.
- Fixed a data blind spot where malformed and timed-out requests rejected before normal handler completion went uncounted by the observability pipeline.
- The server now removes sensitive HTTP headers from access logs (such as `cookie`, `authorization`), to prevent sensitive data exposure in log output.
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
