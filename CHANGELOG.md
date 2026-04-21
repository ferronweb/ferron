# Ferron 3 change log

## Ferron UNRELEASED

**Not released yet**

### Added

- Multiple DNS providers for DNS-01 ACME challenge.
- Support for CGI (Common Gateway Interface).
- Support for FastCGI (including PHP-FPM).
- Support for forwarded authentication.
- Support for SCGI (Simple Common Gateway Interface).

### Fixed

- HTTP to HTTPS redirect wasn't enabled by default.
- HTTP-01 ACME challenge failed due to challenge not being served for implicit automatic TLS.
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
