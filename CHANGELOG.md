# Ferron 2 LTS change log

## Ferron UNRELEASED

**Not yet released**

### Fixed

- Fixed a routing bug for host blocks with configurations in and outside `location "/"` blocks ([GitHub issue](https://github.com/ferronweb/ferron/issues/783))

## Ferron 2.6.3 LTS

**Released in June 12, 2026**

### Changed

- CONNECT requests with pathname URIs are now rejected.
- Improved RFC 7230 compliance for reverse proxy (by stripping hop-by-hop headers).
- OCSP responses are now verified when stapling is enabled.

### Fixed

- 403 Forbidden responses were returned when URL sanitizer was disabled, even when it should have returned 404 Not Found.
- File paths in directory listings weren't properly escaped.
- HTTP Basic Authentication was vulnerable to time-based user enumeration.
- `location` blocks matched path segments anywhere in the URL, not just at the start ([bug report](https://github.com/ferronweb/ferron/issues/639)).
- PROXY v2 headers with lengths greater than 512 bytes were allowed, possibly leading to memory DoS.
- So You Start endpoint names for OVH DNS provider were swapped.

## Ferron 2.6.2 LTS

**Released in March 27, 2026**

### Fixed

- A large enough PROXY v2 header could crash the web server, if the PROXY protocol is enabled.
- IP-based host blocks weren't applied correctly.
- Path traversal might have been possible if URL sanitizer is disabled and the path canonicalization failed.
- The `Proxy` header was passed when using CGI, FastCGI or SCGI (see https://httpoxy.org/).

## Ferron 2.6.1 LTS

**Released in March 26, 2026**

### Fixed

- `Server` and `Alt-Svc` (for HTTP/3) headers couldn't be modified or removed.
