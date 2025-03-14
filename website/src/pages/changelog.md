---
layout: "../layouts/MarkdownPage.astro"
title: Ferron change log
description: Stay updated on Ferron web server improvements with a change log, featuring bug fixes, new features, and enhancements for each release.
---

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
