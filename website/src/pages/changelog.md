---
layout: "../layouts/MarkdownPage.astro"
title: Ferron change log
description: Stay updated on Ferron web server improvements with a change log, featuring bug fixes, new features, and enhancements for each release.
---

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

## Ferron 1.3.7

**Released in December 23, 2025**

- Added support for accepting CIDR ranges for IP blocklists (backported from Ferron 2).
- Fixed a panic when the global web server configuration is not present in the configuration file.

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

## Ferron 1.3.6

**Released in November 1, 2025**

- Added support for disabling X-Forwarded-\* headers for the reverse proxy

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

## Ferron 1.3.5

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

## Ferron 1.3.4

**Released in July 22, 2025**

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

## Ferron 1.3.3

**Released in June 22, 2025**

- Fixed 502 errors caused by canceled operations when reverse proxying with Docker

## Ferron 1.3.2

**Released in June 21, 2025**

- Fixed Rust panics when trying to use reverse proxying with HTTP/3
- The server now wrap ETags in quotes for partial content requests

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

## Ferron 1.3.1

**Released in May 26, 2025**

- Fixed "http.request" ASGI event with the incorrect assigned "lifespan.shutdown" type
- Fixed incorrect configuration validation of error and location configurations

## Ferron 1.3.0

**Released in May 6, 2025**

- Added support for configurable error handling that uses a regular request handler
- Added support for intercepting error responses from backend servers

## Ferron 1.2.0

**Released in May 3, 2025**

- Added support for environment variable overrides
- Fixed the "http2Settings" configuration property logged an unused by Ferron when it's configured
- The server now adds a date to HTTP/3 responses
- The server now sends the original request headers to the WebSocket backend server, when Ferron is configured as a reverse proxy
- The server now sends the original request headers to the ASGI application, when the server is connected via WebSocket protocol

## Ferron 1.1.2

**Released in April 30, 2025**

- Fixed a bug with server indicating alternative HTTP/3 service in a "Alt-Svc" header even if HTTP/3 is disabled

## Ferron 1.1.1

**Released in April 29, 2025**

- Fixed an infinite loop when fetching the request body from the HTTP/3 client
- Fixed duplicate alternative services in "Alt-Svc" header when using the "cache" module

## Ferron 1.1.0

**Released in April 29, 2025**

- Added experimental support for HTTP/3
- Added support for HTTP-01 ACME challenge for automatic TLS
- Added support for WSGI and ASGI (not enabled by default, you must compile Ferron yourself to use these features)

## Ferron 1.0.0

**Released in April 12, 2025**

- First stable release

## Ferron 1.0.0-beta11

**Released in April 5, 2025**

- ETags now are wrapped in double quotes and vary based on the used compression algorithm
- Fixed bug with handling the "s-maxage" directive in "Cache-Control" header value
- The server now adds "Vary" header to the static content responses
- The server now doesn't add "Status" CGI/SCGI/FastCGI header as a HTTP response header

## Ferron 1.0.0-beta10

**Released in March 30, 2025**

- Fixed bug with "userList" and "users" property validation for non-standard codes
- The server now enables OCSP stapling by default

## Ferron 1.0.0-beta9

**Released in March 29, 2025**

- The server now uses the directory containing the executed CGI program as a working directory for the CGI program (this fixed YaBB setup not starting at all)

## Ferron 1.0.0-beta8

**Released in March 28, 2025**

- Added support for `{path}` placeholders for custom header values
- The server now uses the request URL before rewriting in CGI, SCGI, and FastCGI "REQUEST_URI" environment variables (this fixed the redirect loop when URL rewriting is used with Joomla)
- The server now uses the request URL before rewriting in directory listings

## Ferron 1.0.0-beta7

**Released in March 27, 2025**

- Dropped support for dynamically-loaded server modules (Ferron now only supports compiled-in optional modules that can be disabled via Cargo features)
- HTTP/2 is now enabled by default for encrypted connections
- Refactored HTTP connection acception logic

## Ferron 1.0.0-beta6

**Released in March 23, 2025**

- Added option for limiting the cache size by a specific number of entries
- Limited the Zstandard window size to 128KB for better HTTP client support
- Optimized Brotli compression for static files

## Ferron 1.0.0-beta5

**Released in March 16, 2025**

- Fixed a bug related to HTTP cookies and HTTP/2

## Ferron 1.0.0-beta4

**Released in March 16, 2025**

- Added an option to disable backend server certificate verification for the reverse proxy
- Added support for CGI/SCGI/FastCGI "HTTPS" environment variable
- Added support for configuration reloading without entirely restarting the server via a "SIGHUP" signal
- Fixed virtual host resolution not working for HTTP/2 connections

## Ferron 1.0.0-beta3

**Released in March 14, 2025**

- Added support for configuration file includes
- Added support for passive health checks for load balancer
- Added support for request processing timeouts to prevent slow HTTP attacks
- Added support for WebSocket request handlers
- Added support for WebSocket reverse proxying

## Ferron 1.0.0-beta2

**Released in March 8, 2025**

- Added a forwarded authentication module (_fauth_)
- Added support for per-location configuration
- Added support for X-Forwarded-Proto and X-Forwarded-Host headers for _rproxy_ module
- Fixed bug with FastCGI connections not being closed when only partial request body is sent
- Improved server performance when no CGI program is executed

## Ferron 1.0.0-beta1

**Released in March 2, 2025**

- Fixed directory listings and some server error pages displaying HTML as plain text
- Fixed handling of per-host URL rewriting and non-standard code configuration
- Fixed `wwwroot` configuration property resulting in a redirect loop
- Rebranded the web server from "Project Karpacz" to "Ferron"
- The directory listings no longer show a return link for the website root directory
- The entries in the directory listings are now sorted alphabetically

## Project Karpacz 0.7.0

**Released in February 26, 2025**

- Added automatic TLS through TLS-ALPN-01 ACME challenge
- Changed the cryptography provider for Rustls from AWS-LC to _ring_
- Fixed HTTPS server using address-port combinations intended for non-encrypted HTTP server
- Fixed Unix socket URL parsing failures for _scgi_ and _fcgi_ modules

## Project Karpacz 0.6.0

**Released in February 24, 2025**

- Added a FastCGI module (_fcgi_)
- Added a SCGI module (_scgi_)
- Added support for `Must-Staple` marked TLS certificates
- The CGI handler now trims CGI error messages
- The CGI handler now sanitizes double slashes for checking if the request path is in the "cgi-bin" directory

## Project Karpacz 0.5.0

**Released in February 22, 2025**

- Added a CGI module (_cgi_)
- Decreased the cache TTL for static file serving and trailing slash redirects from 1s to 100ms
- Rewritten HTTP status code descriptions
- The request handler now uses a `Request<BoxBody<Bytes, hyper::Error>>` object instead of `Request<Incoming>` object.

## Project Karpacz 0.4.0

**Released in February 20, 2025**

- Added a caching module (_cache_)
- Added concurrency for the keep-alive connection pool in the _rproxy_ module.
- Added support for randomly-distributed load balancing in the _rproxy_ module.
- The web server no longer applies host configuration for forward proxy requests.
- The web server now adds custom headers before executing response modifying handlers.

## Project Karpacz 0.3.0

**Released in February 18, 2025**

- Added a forward proxy module (_fproxy_)
- Added CONNECT forward proxy request handler support
- Added HTTP keep-alive support for reverse proxy module
- Added support for HTTP upgrades
- Added support for optional built-in modules
- Fixed server hang-ups with reverse proxy with high concurrency
- Modified `parallel_fn` function to accept async closures without needing to use `Box::pin` in the module itself
- The error logger struct is now clonable
- The reverse proxy module (_rproxy_) is now an optional reverse proxy module

## Project Karpacz 0.2.0

**Released in February 16, 2025**

- Added a reverse proxy module (_rproxy_)
- Added `builder_without_request` method for ResponseData builder
- Added `ServerConfigurationRoot` parameter for configuration validation functions
- Fixed `BadValues` error when querying configuration by modules
- Implemented parallel function execution (by spawning a Tokio task) in ResponseData
- Improved server configuration processing performance
- The web server now uses `async-channel` crate instead of Tokio's MPSC channel
- The web server now uses `local_dynamic_tls` feature of `mimalloc` crate to fix module loading issues

## Project Karpacz 0.1.0

**Released in February 13, 2025**

- First alpha release
