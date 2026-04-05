# Configuration Reference

This reference describes the current Titanium configuration surface implemented in this workspace.
It is split by topic and by scope so the directive reference is easier to scan.

## Reference Map

- [Syntax And File Structure](./syntax.md)
- [Conditionals And Variables](./conditionals.md)
- [Global Directives](./global.md)
- [Admin API](./global.md#admin)
- [HTTP Host Directives](./http-host.md)
- [HTTP Control Directives](./http-control.md)
- [TLS Crypto Settings and mTLS](./tls-crypto.md)
- [TLS Session Ticket Keys](./tls-session-tickets.md)
- [OCSP Stapling](./ocsp-stapling.md)
- [Observability And Logging](./observability.md)

## Scopes

Titanium currently has three main directive scopes in this codebase:

- Global scope: directives inside top-level `{ ... }` blocks
- Admin API scope: directives inside the `admin { ... }` global block
- HTTP host scope: directives inside HTTP host blocks such as `example.com { ... }`

The HTTP host scope also has control directives that affect request matching and layering:

- `location`
- `if`
- `if_not`
- `handle_error`

## Important Notes

- Where validation and runtime behavior differ, the directive pages call that out explicitly.
- `handle_error` is parsed and prepared, but it is not currently applied by the HTTP request handler.
