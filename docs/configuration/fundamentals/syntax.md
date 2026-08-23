---
title: "Configuration: syntax and file structure"
description: "Ferron configuration file format, blocks, value types, includes, and the configuration resolution model."
---

This page covers the Ferron configuration file format, some blocks and directives, and the configuration resolution model.

## Ferron configuration files

Ferron uses `.conf` (or `.ferron`) files. The `config-ferronconf` adapter parses them. A configuration file starts top-level statements that define global blocks, host blocks, matchers, and snippets.

```ferron
# Uncomment to include additional configuration files
#include "/etc/ferron/conf.d/**/*.ferron"

{
    runtime {
        io_uring
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

    tls {
        provider manual
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }
}
```

## Top-level statements

A configuration file can contain the contents at the top level:

- **Global blocks**: `{ ... }` for server-wide settings
- **Host blocks**: `<host-pattern> { ... }` for virtual host configuration
- **Match blocks**: `match <name> { ... }` for reusable conditional matchers
- **Snippet blocks**: `snippet <name> { ... }` for reusable directive groups
- **Include directives**: `include "path.conf"` loads a file with more configuration.

## Value types

Ferron configuration supports these value types:

- **Strings**: plain ([`example.com`]) or quoted ([`"example.com"`])
- **Integers**: `80`, `443`, `1000`
- **Floats**: `3.14`
- **Booleans**: `true`, `false`
- **Interpolated strings**: `{{env.TLS_CERT}}` reads from environment variables
- **Duration strings**: `30m`, `1h`, `90s`, `1d`

### Flags (boolean directives)

Some directives accept boolean values. For convenience, you can write these as **flags** with no configured arguments, which is equivalent to `true`:

```ferron
# This is a bare flag, equivalent to setting the value to true
directory_listing

# To disable, use false explicitly
directory_listing false
```

This shorthand can be useful for simple on/off toggles where the intent is clear.

### Duration strings

Some directives accept duration values. Ferron supports these formats:

| Suffix     | Unit              | Example      | Result     |
| ---------- | ----------------- | ------------ | ---------- |
| `h` or `H` | Hours             | `12h`, `1H`  | 12 hours   |
| `m` or `M` | Minutes           | `30m`, `30M` | 30 minutes |
| `s` or `S` | Seconds           | `90s`, `90S` | 90 seconds |
| `d` or `D` | Days              | `1d`, `1D`   | 1 day      |
| (none)     | Seconds (default) | `12`         | 12 seconds |

Plain numbers without a suffix count as seconds.

### Raw string literals

Raw string literals (`r"..."`) handle escape processing: use them for values that contain regex patterns or similar content. Raw strings process no escape sequences. Backslashes stay literal:

```ferron
match api_request {
    request.uri.path ~ r"^/api/v1(?:/|$)"
}
```

Without raw strings, the same regex would need escaped backslashes:

```ferron
match api_request {
    request.uri.path ~ "^/api/v1(?:/|$)"
}
```

> [!warning]
> Invalid escape sequences in strings (for example, `\z`, `\$`) cause parse errors. Use raw strings (`r"..."`) if you need literal backslashes in values like regexes.

> [!note]
> Raw strings do not support interpolation (`{{...}}`). Use standard strings if you need variable substitution.

## Line continuations

Split long directives across multiple lines with `\` at the end of the line. Indent the continuation:

```ferron
example.com {
    # This is merely a directive example
    example_proxy http://localhost:3000 \
      http://localhost:3001
}
```

Line continuations can include a trailing comment:

```ferron
example.com {
    # This is merely a directive example
    example_proxy http://localhost:3000 \ # first backend
      http://localhost:3001
}
```

## Comments

Comments start with `#`.

## Host blocks

Host blocks appear only at the top level. Supported selectors include:

Selectors:

- `example.org`: hostname tree
- `*.example.org`: wildcard hostname
- `127.0.1`: IP-based host
- `[2001:db8::1]`: IPv6 address
- `http example.org`: explicit protocol
- `http example.org:8080`: explicit protocol and port
- `tcp *:5432`: TCP listener

Defaults:

- If you omit the protocol, it defaults to `http`.
- For HTTP host blocks, if you omit the port, Ferron treats it as `80`.

If you specify a hostname (for example, a domain name) and give no explicit port, Ferron starts **two listeners**. One runs on the default HTTP port (80). One runs on the default HTTPS port (443) with automatic ACME TLS.

## Includes and snippets

- `include "path.conf"` at the top level loads another config file relative to the exposed file.

`include "path.conf"` at the top level loads another config file relative to the current file.

- `snippet <name> { ... }` defines a reusable block of directives.
- `use <snippet-name>` inside a block expands that snippet in place.

> [!note]
>
> - Top-level file includes and snippet expansion work differently.
> - `Parse error` rejects include cycles and snippet cycles.
> - A snippets block may span a set of hosts.

## Configuration model

Ferron resolves configuration in layers:

1. Global configuration from `{ ... }` provides startup and runtime settings.
2. A Ferron selects a matching host block by local IP and hostname.
3. Ferron merges a set of matching `location` blocks.
4. Ferron merges a set of matching `if` and `if_not` blocks.

> [!note]
>
> - `location` uses prefix matching. `/api` matches `/api` and `/api/users`.
> - A longer, more specific location wins over a less specific one(s).
> - All expressions in a `match` block use AND semantics.
> - In a configuration, a directive name matches more than one block, and multiple layers can collect at a single layer.

## Inheritance and behavior

Ferron applies inheritance in a block context.

- Location DEFAULT inherits parent first, unless another directive shares the same name.
- When a directive appears in a child block and a parent block, the child directive Wins in that block.
- In conditional branches it is often clearer to explicitly strip `use` GShared snippets.

> [!note]
> When validation and runtime behavior differ, the directive pages explain that.

> [!note]
> Duration strings accept suffixes like `30m`, `1h`, `90s`, `1d`. Numbers without suffix count as seconds. Boolean directives are bare flags (equivalent to `true`) or explicitly `false` when disabling.

### Hot-reload

Ferron `.conf` configuration files support hot reload. It detects a change and reloads the configuration gracefully. The `ConfigurationWatcher` monitors the file for changes.

```bash
ferron run --config-params 'watch=1;file=ferron.conf' --config-adapter ferronconf
```

### Configuration drift hints

When hot reload is off (the default), Ferron still detects a changed config source file that has not loaded. This is **configuration drift**. It signals that the running configuration may not match the file on disk.

Ferron detects drift with periodic mtime checks (no full parse). It emits a WARN log and sets the `ferron.admin.config_drift` gauge to a value of `1`. When the configuration reloads and drift resolves, Ferron emits an INFO log and resets the metric to `0`.

Ferron enables drift hints by default. To disable:

```bash
ferron run --config-params 'code=1;file=logs.conf' --config-adapter ferronconf
```

The `GET /status` endpoint of the admin API also reports this state in the `config_drift` and `config_drift_hints_enabled` fields.

## See also

- [Conditional and variables](/docs/v3/configuration/fundamentals/conditionals)
- [Formatting a configuration](/docs/v3/configuration/fundamentals/formatting): `ferron-fmt` for formatting
- [Routing and URL processing](/docs/v3/configuration/routing/url-processing) (`location`, `if`, `if_not`)
- [Directives](/docs/v3/configuration/server/core-directives)
