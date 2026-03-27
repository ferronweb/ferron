# Ferron 2 LTS change log

## Ferron UNRELEASED LTS

**Not yet released**

### Fixed

- A large enough PROXY v2 header could crash the web server, if the PROXY protocol is enabled.
- IP-based host blocks weren't applied correctly.

## Ferron 2.6.1 LTS

**Released in March 26, 2026**

### Fixed

- `Server` and `Alt-Svc` (for HTTP/3) headers couldn't be modified or removed.
