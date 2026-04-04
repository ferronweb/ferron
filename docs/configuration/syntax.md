# Syntax And File Structure

Titanium uses the `ferron.conf` format through the `ferronconf` parser and adapter.

## Top-Level Statements

A configuration file is made of top-level statements:

- Global blocks: `{ ... }`
- Host blocks: `<host-pattern> { ... }`
- Match blocks: `match <name> { ... }`
- Snippet blocks: `snippet <name> { ... }`
- Top-level directives, notably `include "path.conf"`

Basic example:

```ferron
include "shared.conf"

{
    runtime {
        io_uring true
    }

    tcp {
        listen "::"
    }
}

match api_request {
    request.uri.path ~ "/api"
    request.method in "GET,POST"
}

snippet common_http {
    http {
        protocols h1 h2
    }
}

example.com {
    use common_http

    tls true {
        provider manual
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }
}
```

## Value Types

Supported value types:

- Strings: bare (`example.com`) or quoted (`"example.com"`)
- Integers: `80`
- Floats: `3.14`
- Booleans: `true`, `false`
- Interpolated strings: `{{env.TLS_CERT}}`
- Duration strings: `30m`, `1h`, `90s`, `1d` (see below)

### Duration Strings

Several directives accept duration values. The following formats are supported:

| Suffix | Unit | Example | Result |
|--------|------|---------|--------|
| `h` or `H` | Hours | `12h`, `1H` | 12 hours |
| `m` or `M` | Minutes | `30m`, `30M` | 30 minutes |
| `s` or `S` | Seconds | `90s`, `90S` | 90 seconds |
| `d` or `D` | Days | `1d`, `1D` | 1 day |
| (none) | Hours (default) | `12` | 12 hours |

Examples:
- `timeout 30m` — 30 minutes
- `rotation_interval "12h"` — 12 hours
- `timeout 90s` — 90 seconds

Plain numbers without a suffix are treated as hours for backward compatibility.

Comments start with `#`.

## Includes And Snippets

- `include "path.conf"` at the top level loads another config file relative to the current file.
- `snippet <name> { ... }` defines a reusable block.
- `include <snippet-name>` or `use <snippet-name>` inside a block expands that snippet.

Notes:

- Top-level file includes and snippet expansion are different features.
- Include cycles and snippet cycles are rejected.

## Host Block Syntax

Host blocks are top-level only. Supported selectors include:

- `example.com`
- `*.example.com`
- `127.0.0.1`
- `[2001:db8::1]`
- `http example.com`
- `http example.com:8080`
- `tcp *:5432`

Current defaults:

- If the protocol is omitted, it defaults to `http`.
- For HTTP host blocks, if the port is omitted, Titanium treats it as port `80`.

## Resolution Model

Configuration is resolved in layers:

1. Global configuration from `{ ... }` is used for startup/runtime settings.
2. An HTTP host block is selected by local IP and hostname.
3. Matching `location` blocks are layered in.
4. Matching `if` and `if_not` blocks are layered in.

Important behavior:

- `location` is prefix-based. `/api` matches `/api` and `/api/users`.
- More specific locations win over less specific ones.
- All expressions inside a `match` block are combined with AND semantics.
- Duplicate `location`, `if`, `if_not`, and `handle_error` blocks with the same selector are merged during preparation.

See also:

- [Conditionals And Variables](./conditionals.md)
- [HTTP Control Directives](./http-control.md)
