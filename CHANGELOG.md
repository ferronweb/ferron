# Ferron 3 change log

## Ferron UNRELEASED

**Not yet released**

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
