# Configuration Reference

This reference describes the current Titanium configuration surface implemented in this workspace.
It is split by topic and by scope so the directive reference is easier to scan.

## Reference Map

- [Syntax And File Structure](/home/dorian/Projects/titanium/docs/configuration/syntax.md)
- [Conditionals And Variables](/home/dorian/Projects/titanium/docs/configuration/conditionals.md)
- [Global Directives](/home/dorian/Projects/titanium/docs/configuration/global.md)
- [HTTP Host Directives](/home/dorian/Projects/titanium/docs/configuration/http-host.md)
- [HTTP Control Directives](/home/dorian/Projects/titanium/docs/configuration/http-control.md)

## Scopes

Titanium currently has two main directive scopes in this codebase:

- Global scope: directives inside top-level `{ ... }` blocks
- HTTP host scope: directives inside HTTP host blocks such as `example.com { ... }`

The HTTP host scope also has control directives that affect request matching and layering:

- `location`
- `if`
- `if_not`
- `handle_error`

## Important Notes

- Where validation and runtime behavior differ, the directive pages call that out explicitly.
- `handle_error` is parsed and prepared, but it is not currently applied by the HTTP request handler.
