# Ferron 2 change log

[View Ferron 1.x changelog](https://ferron.sh/changelog/v1)

## Ferron 2.5.3

**Released in February 11, 2026**

- Fixed process-related metrics not being sent at all.

## Ferron 2.5.2

**Released in February 11, 2026**

- Improved memory usage during configuration reloads.

## Ferron 2.5.1

**Released in February 10, 2026**

- Fixed graceful shutdowns when reloading the server configuration.

## Ferron 2.5.0

**Released in February 10, 2026**

- Added support for logging into standard I/O.
- Added support for saving TLS certificates and private keys (when using automatic TLS functionality) into disk and executing commands afterwards.
- Added support for sending `Forwarded` HTTP header to backend servers as a reverse proxy.
- Added support for specifying configuration in command-line arguments.
- Added the `ferron serve` subcommand.
- Fixed TLS certificate not resolved for "localhost" SNI hostname.
- Optimized the server configuration lookup performance.
- Optimized the SNI certificate resolution performance.
- Refreshed the default placeholder page design.
- The server now differentiates between file types in directory listings.
- The server now reuses threads when reloading the configuration, if possible.
- The server now shuts down multiple threads at once when reloading the configuration.

## Ferron 2.4.1

**Released in January 17, 2026**

- Fixed a rarely occurring crash when upgrading backend server's HTTP connection as a reverse proxy.

## Ferron 2.4.0

**Released in January 17, 2026**

- Added bunny.net, DigitalOcean and OVH DNS providers for DNS-01 ACME challenge.
- Added support for HTTP Basic authentication for forward proxying.
- Fixed ACME cache file handling during certificate renewals. Cache files are now correctly truncated when rewritten, preventing stale data from causing parse failures.
- Fixed brute-force protection not being able to be disabled due to wrong configuration validation check.
- Fixed `Connection` header setting for reverse proxying being set to `keep-alive, keep-alive`.
- Fixed graceful shutdown (during configuration reloading) for the HTTP/3 server.
- Fixed precompressed files not being picked up when the original filename doesn't have a file extension.
- Fixed the original request URL not preserved when the server is configured to rewrite URLs using `rewrite` directive.
- Fixed trailing slash redirects leading to an URL without base when `remove_base` prop of a location block is set to `#true`.
- Fixed URL rewrites not applied when `remove_base` prop of a location block is set to `#true`.
- Improved compliance of static file serving functionality with RFC 7232 (conditional requests) and RFC 7233 (range requests).
- The forwarded authentication module now uses an unlimited idle kept-alive connection pool, just like the reverse proxy module.
- The server now falls back with `io_uring` disabled when `io_uring` couldn't be initialized and `io_uring` is implicitly enabled.
- The server now logs a warning if `status 200` directive is used without specifying a response body.
- The server now performs cleanup of TLS-ALPN-01 and HTTP-01 challenges after obtaining the TLS certificates.
- The server now reuses connections that aren't ready after waiting for readiness when the concurrent limit is reached, instead of establishing a new connection.

## Ferron 2.3.2

**Released in January 6, 2026**

- The server now gracefully handles canceled I/O operations that could previously cause 502 Bad Gateway errors (when io_uring is disabled).
- The server now gracefully handles canceled I/O operations that could previously cause a crash under rare conditions (when io_uring is enabled).

## Ferron 2.3.1

**Released in January 6, 2026**

- The server now gracefully handles canceled I/O operations that could previously cause a crash under rare conditions (when io_uring is disabled).

## Ferron 2.3.0

**Released in January 6, 2026**

- Added a metric for reverse proxy connections (grouped by whether the connection is reused)
- Added option to disable the URL sanitizer (to allow passing request path as-is to proxy backend servers without the sanitizer rewriting the URL).
- Added support for canonicalized IP address placeholders.
- Added support for global and local reverse proxy TCP connection concurrency limits.
- Added support for timeouts for idle kept-alive connections in a reverse proxy.
- Fixed a CGI, SCGI and FastCGI interoperability issue caused by the wrong value of the "HTTPS" variable.
- Fixed an XSS bug through server administrator's email address specified in the server configuration.
- Fixed errors when using URL-safe Base64-encoded ACME EAB key HMACs with "=" at the end.
- Fixed explicit TLS version configuration being incorrectly applied.
- Improved error reporting for invalid URLs for SCGI and FastCGI.
- Optimized the performance of overall network I/O.
- Optimized the QUIC and HTTP/3 performance.
- Removed a configuration directive for specifying maximum idle kept-alive connection pool in a reverse proxy.
- Replaced mimalloc v2 with mimalloc v3 (and also dropped support for very early 64-bit x86 CPUs).
- Slightly optimized ETag generation for static file serving.
- The H3_NO_ERROR errors are no longer logged into the error log.
- The reverse proxy now no longer waits for non-ready connections to be ready (it now just pulls another connection from the pool).
- The reverse proxy now uses an unlimited idle kept-alive connection pool.
- The server is now accessible via IPv4 by default on Windows (IPv6 is enabled by default).
- The server now no longer fails automatic TLS certificate management tasks, when the ACME cache is inaccessible or corrupted.
- The server now removes some response headers that are invalid in HTTP/3, if the client is connected to the server via HTTP/3
- The server now uses a faster asynchronous Rust runtime (Monoio) on Windows (like it is on other platforms) instead of Tokio only.

## Ferron 2.2.1

**Released in December 5, 2025**

- Fixed a bug causing a deadlock when the server is gracefully reloading its configuration and OTLP observability backend was enabled before.
- The server now no longer overrides `X-Forwarded-Host` and `X-Forwarded-Proto` request headers before sending them to backend servers, when they exist, and the `X-Forwarded-For` header is trusted.

## Ferron 2.2.0

**Released in December 3, 2025**

- Added support for observability (via logs, metrics and traces) via OpenTelemetry Protocol (OTLP).
- Fixed a bug causing requests to not be logged at all to host-specific access logs, if the global access log file wasn't specified.
- Fixed a bug causing the default cache item count limit to be not enforced.

## Ferron 2.1.0

**Released in November 26, 2025**

- Added a language matching subcondition (based on the `Accept-Language` header).
- Added support for custom MIME types for static file serving.
- Added support for dynamic content compression.
- Added support for HTTP/2-only (and gRPC over plain text) backend servers.
- Added support for sending PROXY protocol headers to backend servers when acting as a reverse proxy.
- Added support for setting constants inside conditions.
- Added support for specifying custom directory index files.
- Added support for using snippets inside conditions.
- Configuration validation and module loading error messages now also report in what block did the error occur.
- Corrected the configuration validation for `cgi_interpreter` directive.
- Fixed access logs wrongly written to global log files instead of host-specific ones.
- Fixed bug preventing some configuration properties in `error_config` blocks from being applied.
- The `block` and `allow` directives (used for access control) are no longer global-only.
- The server now disables HTTP/2 for backend servers when `proxy_http2` directive is used, and the request contains `Upgrade` header.
- The server now removes `Forwarded` header before sending requests to backend servers as a reverse proxy.

## Ferron 2.0.1

**Released in November 4, 2025**

- Fixed bugs related to wrongly applying configurations from configuration blocks.

## Ferron 2.0.0

**Released in November 4, 2025**

- First stable release of Ferron 2

## Ferron 2.0.0-rc.2

**Released in October 25, 2025**

- Fixed bugs related to ACME EAB (External Account Binding).
- Fixed bugs related to FastCGI, when using Ferron with fcgiwrap.

## Ferron 2.0.0-rc.1

**Released in October 20, 2025**

- Added an utility to precompress static files ("ferron-precompress")
- Added support for Rego-based subconditions
- Added support for specifying maximum idle connections to backend servers to keep alive
- Added various load balancing algorithms (round-robin, power of two random choices, least connection)
- Changed the default load balancing algorithm to power of two random choices
- Improved subcondition error handling by logging specific subcondition errors
- Optimized the keep-alive behavior in reverse proxying and load balancing for better performance
- The "lb_health_check_window" directive is no longer global-only
- The web server now removes ACME accounts from the ACME cache, if they don't exist in an ACME server

## Ferron 2.0.0-beta.20

**Released in October 11, 2025**

- The server now logs the client IP address instead of the server IP address in the access log

## Ferron 2.0.0-beta.19

**Released in October 11, 2025**

- Added support for `{path_and_query}` placeholder
- Added support for ACME Renewal Information (ARI)
- Added support for connecting to backend servers via Unix sockets as a reverse proxy
- Added support for custom access log formats

## Ferron 2.0.0-beta.18

**Released in October 4, 2025**

- Added default ACME cache directory path to the Docker image
- Added support for serving precompressed static files
- Fixed flipped access and error log filenames in the default server configuration for Docker images
- The server now uses an embedded certificate store as a fallback when native TLS is not available

## Ferron 2.0.0-beta.17

**Released in September 8, 2025**

- Fixed the server crashing when resolving a TLS certificate when a client connects to the server via HTTPS (versions compiled with Tokio only were affected)

## Ferron 2.0.0-beta.16

**Released in July 31, 2025**

- Adjusted the Brotli and Zstandard compression parameters for lower memory usage

## Ferron 2.0.0-beta.15

**Released in July 29, 2025**

- Added support for ACME EAB (External Account Binding)
- Added support for CIDR ranges for `block` and `allow` directives
- Added support for conditional configurations
- Added support for external Ferron modules and DNS providers
- Added support for load balancer connection retries to other backend servers in case of TCP connection or TLS handshake errors
- Added support for reusable snippets in KDL configuration
- Added support for `status` directives without `url` nor `regex` props
- Changed the styling of default error pages and directory listings
- Fixed graceful shutdowns with ASGI enabled
- Fixed several erroneous HTTP to HTTPS redirects
- Improved overall server performance, including static file serving
- The server now determines the host configuration order automatically based on the hostname specificity
- The server now determines the location order automatically based on the location and conditionals’ depth
- The server now disables automatic TLS by default for "localhost" and other loopback addresses
- The YAML to KDL configuration translator now inherits header values from higher configuration levels in YAML configuration to the KDL configuration

## Ferron 2.0.0-beta.14

**Released in July 22, 2025**

- Added support for buffering request and response bodies
- The server now determines the server configuration again (with changed location) after replacing the URL with a sanitized one.

## Ferron 2.0.0-beta.13

**Released in July 20, 2025**

- Added support for Amazon Route 53 DNS provider for DNS-01 ACME challenge
- Added support for automatic TLS on demand
- Added support for connecting to backend servers via HTTP/2 as a reverse proxy
- Added support for global configurations that don't imply a host
- Added support for host blocks that specify multiple hostnames
- Fixed SNI hostname handling for non-default HTTPS ports
- Improved graceful connection shutdowns while gracefully restarting the server
- The server now can use multiple "Vary" response headers for caching

## Ferron 2.0.0-beta.12

**Released in July 13, 2025**

- Fixed "address in use" errors when listening to a TCP port after stopping and shortly after starting the web server

## Ferron 2.0.0-beta.11

**Released in July 13, 2025**

- Fixed unexpected connection closure errors in `w3m` and `lynx` (the fix in Ferron 2.0.0-beta.10 might be a partial fix)

## Ferron 2.0.0-beta.10

**Released in July 13, 2025**

- Added support for accepting connections that use PROXY protocol
- Fixed unexpected connection closure errors in `w3m` and `lynx`

## Ferron 2.0.0-beta.9

**Released in July 11, 2025**

- Added support for ACME profiles
- Added support for DNS-01 ACME challenge
- Added support for header replacement
- Added support for IP allowlists
- Added support for more header value placeholders
- Added support for setting "Cache-Control"
- The server now obtains TLS certificates from ACME server sequentially

## Ferron 2.0.0-beta.8

**Released in July 4, 2025**

- Fixed TCP connection closure when the server request the closure (the fix in Ferron 2.0.0-beta.6 might not have worked)

## Ferron 2.0.0-beta.7

**Released in July 4, 2025**

- Fixed an ACME error related to contact addresses, even if the contact address is valid

## Ferron 2.0.0-beta.6

**Released in July 4, 2025**

- Fixed TCP connection closure when the server request the closure
- Switched ACME implementation to prepare for DNS-01 ACME challenge support

## Ferron 2.0.0-beta.5

**Released in June 29, 2025**

- Added a configuration directive for removing response headers
- Added support for disabling HTTP keep-alive for reverse proxy
- Added support for setting headers for HTTP requests sent by the reverse proxy
- Added support for specifying custom response bodies in the `status` directive
- Fixed explicitly specified HTTP-only ports erroneously marked as HTTPS ports
- The HTTP cache size is now limited by default

## Ferron 2.0.0-beta.4

**Released in June 22, 2025**

- Fixed host configurations not being used.

## Ferron 2.0.0-beta.3

**Released in June 22, 2025**

- Added several automatic TLS-related configuration directives
- Added support for per-host logging
- Fixed 502 errors caused by canceled operations when reverse proxying with Docker
- Fixed a bug where location configurations were checked in incorrect order
- Fixed Rust panics when trying to use reverse proxying with HTTP/3
- Fixed the translation of "maximumCacheEntries" Ferron 1.x YAML configuration property
- The server now uses common ACME account cache directory for automatic TLS

## Ferron 2.0.0-beta.2

**Released in June 17, 2025**

- Added a configuration adapter to automatically determine the configuration path when running the web server in a Docker image
- Added a module to replace substrings in response bodies
- Added a rate limiting module
- Added support for key exchanges with post-quantum cryptography
- Fixed infinite recursion of error handler execution
- Fixed translation of "errorPages" YAML configuration property
- Fixed translation of "users" YAML configuration property
- The KDL parsing errors are now formatted

## Ferron 2.0.0-beta.1

**Released in June 4, 2025**

- First beta release of Ferron 2.x
