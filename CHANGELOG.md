# Ferron 2 LTS change log

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
