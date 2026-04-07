---
title: "Configuration: syntax and file structure"
description: "Ferron configuration file format, blocks, value types, includes, and the configuration resolution model."
---

This page covers the Ferron configuration file format, how blocks and directives are structured, and how configuration is resolved at runtime.

## Ferron configuration files

Ferron uses `.conf` files parsed by the `ferronconf` adapter. A configuration file is made of top-level statements that define global blocks, host blocks, matchers, and snippets.

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

## Top-level statements

A configuration file can contain the following at the top level:

- **Global blocks** — `{ ... }` for server-wide settings
- **Host blocks** — `<host-pattern> { ... }` for virtual host configuration
- **Match blocks** — `match <name> { ... }` for reusable conditional matchers
- **Snippet blocks** — `snippet <name> { ... }` for reusable directive groups
- **Include directives** — `include "path.conf"` to load additional configuration files

## Value types

Ferron configuration supports these value types:

- **Strings** — bare (`example.com`) or quoted (`"example.com"`)
- **Integers** — `80`, `443`, `1000`
- **Floats** — `3.14`
- **Booleans** — `true`, `false`
- **Interpolated strings** — `{{env.TLS_CERT}}` reads from environment variables
- **Duration strings** — `30m`, `1h`, `90s`, `1d`

### Flags (boolean directives)

Several directives accept boolean values. For convenience, these can be written as **flags** with no arguments, which is equivalent to `true`:

```ferron
# These are equivalent:
directory_listing
directory_listing true

# To explicitly disable, use false:
directory_listing false
```

This shorthand is useful for simple on/off toggles where the intent is clear. The following directives support flag syntax: `abort`, `compressed`, `precompressed`, `etag`, `directory_listing`, `trailing_slash_redirect`, `url_sanitize`, `keepalive`, `http2`, `http2_only`, `intercept_errors`, `no_verification`, `lb_health_check`, `lb_retry_connection`, `on_demand`, `client_auth`, and others.

### Duration strings

Several directives accept duration values. The following formats are supported:

| Suffix | Unit | Example | Result |
|--------|------|---------|--------|
| `h` or `H` | Hours | `12h`, `1H` | 12 hours |
| `m` or `M` | Minutes | `30m`, `30M` | 30 minutes |
| `s` or `S` | Seconds | `90s`, `90S` | 90 seconds |
| `d` or `D` | Days | `1d`, `1D` | 1 day |
| (none) | Hours (default) | `12` | 12 hours |

Plain numbers without a suffix are treated as hours for backward compatibility.

Comments start with `#`.

## Host block syntax

Host blocks are top-level only. Supported selectors include:

- `example.com` — hostname-based virtual host
- `*.example.com` — wildcard hostname
- `127.0.0.1` — IP-based virtual host
- `[2001:db8::1]` — IPv6 address
- `http example.com` — explicit protocol
- `http example.com:8080` — explicit protocol and port
- `tcp *:5432` — TCP listener

Current defaults:

- If the protocol is omitted, it defaults to `http`.
- For HTTP host blocks, if the port is omitted, Ferron treats it as port `80`.

When a hostname is specified (e.g. `example.com`) and no explicit port is given, Ferron starts **two listeners** — one on the default HTTP port (80) and one on the default HTTPS port (443) with automatic ACME TLS. See [ACME automatic TLS](/docs/v3/tls-acme) for details.

## Includes and snippets

- `include "path.conf"` at the top level loads another config file relative to the current file.
- `snippet <name> { ... }` defines a reusable block of directives.
- `use <snippet-name>` inside a block expands that snippet in place.

Notes:

- Top-level file includes and snippet expansion are different features.
- Include cycles and snippet cycles are rejected.
- Snippets can be reused across multiple host blocks.

## Resolution model

Configuration is resolved in layers:

1. Global configuration from `{ ... }` is used for startup and runtime settings.
2. An HTTP host block is selected by local IP and hostname.
3. Matching `location` blocks are layered in.
4. Matching `if` and `if_not` blocks are layered in.

Important behavior:

- `location` is prefix-based. `/api` matches `/api` and `/api/users`.
- More specific locations win over less specific ones.
- All expressions inside a `match` block are combined with AND semantics.
- Duplicate `location`, `if`, `if_not`, and `handle_error` blocks with the same selector are merged during preparation.

## Inheritance and override behavior

Ferron applies inheritance by block context:

- Location blocks inherit parent directives unless the child block defines directives with the same name.
- When a child block defines a directive with the same name as one in the parent, the child's directives take precedence in that block.
- For conditional branches, it is often clearer to explicitly `use` shared snippets inside each `if`/`if_not` branch.

## See also

- [Conditionals and variables](/docs/v3/conditionals)
- [Routing and URL processing](/docs/v3/routing-url-processing) (`location`, `if`, `if_not`)
- [Core directives](/docs/v3/core-directives)

## Notes and troubleshooting

- Hostnames that start with a number must be quoted (e.g. `"1.example.com"`).
- Multiple host identifiers in one block are comma-separated without spaces (e.g. `example.com,example.org`).
- In complex configurations, explicitly reusing shared snippets inside `if`/`if_not` branches avoids surprises and keeps behavior clear.
- Duration strings: plain numbers without a suffix are treated as hours for backward compatibility.
