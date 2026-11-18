---
title: "Configuration: syntax and file structure"
description: "Ferron configuration file format, blocks, value types, includes, and the configuration resolution model."
---

Ferron configuration files use the `.conf` or `.ferron` extension. The `config-ferronconf` adapter parses them. A file contains top-level statements that define global blocks, host blocks, matchers, and snippets.

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

A configuration file can contain these statements at the top level:

- Global blocks: `{ ... }` for server-wide settings.
- Host blocks: `<host-pattern> { ... }` for virtual host configuration.
- Match blocks: `match <name> { ... }` for reusable conditional matchers.
- Snippet blocks: `snippet <name> { ... }` for reusable directive groups.
- Includes: `include "path.conf"` loads another configuration file.

## Value types

Ferron configuration supports these value types:

- Strings: bare (`example.com`) or quoted (`"example.com"`).
- Integers: `80`, `443`, `1000`.
- Floats: `3.14`.
- Booleans: `true`, `false`.
- Interpolated strings: `{{env.TLS_CERT}}` reads an environment variable.
- Duration strings: `30m`, `1h`, `90s`, `1d`.

### Boolean flags

A boolean directive with no argument means `true`:

```ferron
example.com {
    directory_listing
}
```

To disable a boolean directive, pass `false`:

```ferron
example.com {
    directory_listing false
}
```

### Duration strings

Duration values accept these suffixes:

| Suffix     | Unit              | Example      | Result     |
| ---------- | ----------------- | ------------ | ---------- |
| `h` or `H` | Hours             | `12h`, `1H`  | 12 hours   |
| `m` or `M` | Minutes           | `30m`, `30M` | 30 minutes |
| `s` or `S` | Seconds           | `90s`, `90S` | 90 seconds |
| `d` or `D` | Days              | `1d`, `1D`   | 1 day      |
| (none)     | Seconds (default) | `12`         | 12 seconds |

A number without a suffix counts as seconds.

### Raw strings

A raw string (`r"..."`) does not process escape sequences. Use one for regex patterns and other values that contain backslashes:

```ferron
match api_request {
    request.uri.path ~ r"^/files/.+\.(?:png|jpg)$"
}
```

In a normal string, each backslash must be doubled:

```ferron
match api_request {
    request.uri.path ~ "^/files/.+\\.(?:png|jpg)$"
}
```

> [!warning]
> An invalid escape sequence in a string, for example `\z` or `\$`, causes a parse error. Use a raw string when the value contains literal backslashes.

> [!note]
> A raw string does not support interpolation (`{{...}}`). Use a normal string when you need variable substitution.

## Line continuations

End a line with `\` to continue a directive on the next line. Indent the continuation:

```ferron
example.com {
    index index.html \
      index.htm
}
```

A continuation line can carry a trailing comment:

```ferron
example.com {
    index index.html \ # first choice
      index.htm
}
```

## Comments

Comments start with `#` and run to the end of the line. Multiline comments are not supported, so start every line with `#`.

## Host blocks

Host blocks appear only at the top level. These selectors are supported:

- `example.org`: hostname
- `*.example.org`: wildcard hostname
- `127.0.0.1`: IP-based host
- `[2001:db8::1]`: IPv6 address
- `http example.org`: explicit protocol
- `http example.org:8080`: explicit protocol and port
- `tcp *:5432`: TCP listener
- `*:8080`: explicit port for the entire server
- `*`: wildcard for the entire server at the default ports

The protocol defaults to `http`. An HTTP host block without a port uses port `80`.

A hostname without an explicit port starts two listeners. One listens on port `80` for HTTP. The other listens on port `443` for HTTPS with automatic ACME TLS.

## Includes and snippets

`include "path.conf"` at the top level loads another configuration file. The path is relative to the file that contains the directive.

`snippet <name> { ... }` defines a reusable group of directives. `use <snippet-name>` expands the snippet in place.

> [!note]
> Includes and snippet expansion work differently.
>
> A parse error rejects include cycles and snippet cycles.
>
> A snippet can apply to several hosts.
>
> Ferron merges host blocks for the same host, including blocks from different included files. To reject a host that appears in more than one file, enable [`strict_hosts`](/docs/configuration/server/core-directives#host-definitions).

## Configuration model

Ferron resolves configuration in layers, once per request, before the pipeline stages run:

1. Global configuration from `{ ... }` provides startup and runtime settings.
2. Ferron selects matching host blocks by local IP and hostname.
3. Ferron merges matching `location` blocks. Matching is prefix only, and the longest match wins.
4. Ferron merges matching `if` and `if_not` blocks.

`location` uses prefix matching, so `/api` matches both `/api` and `/api/users`. All expressions in a `match` block use AND semantics.

A rewrite in a later pipeline stage does not trigger a new round of `location` matching. See [Request pipeline order](/docs/configuration/fundamentals/request-pipeline).

## Inheritance

Directives inherit from outer blocks to inner blocks. A `location` block starts with the host block values. An `if` block starts with the enclosing host or `location` values.

- A directive in both a parent and a child block takes the child value in that block.
- A directive only in the parent block is inherited by the child block.
- Some directives accumulate across layers instead of overriding, for example `rewrite` rules and `map` entries. Their reference pages state the behavior.
- Handler and backend selectors are host-isolated. Directives such as `proxy`, `fcgi`, `cgi`, `scgi`, `forward_proxy`, `root`, `index`, `basic_auth`, and the static file tunables do not inherit from the wildcard `*` host into a named host. They still inherit from global defaults, and a `location` block still inherits them from its host. Each directive page states its exact behavior.

> [!note]
> When validation and runtime behavior differ, the directive page explains the difference.

## Hot reload

Ferron detects a change to a `.conf` file and reloads the configuration gracefully:

```bash
ferron run --config-params 'watch=1;file=ferron.conf' --config-adapter ferronconf
```

## Configuration drift hints

With hot reload off (the default), Ferron still detects a changed configuration file that has not been loaded. This state is configuration drift: the running configuration may not match the file on disk.

Ferron compares file modification times on a timer and does not re-parse the file. It emits a WARN log and sets the `ferron.admin.config_drift` gauge to `1`. When a reload resolves the drift, Ferron emits an INFO log and resets the gauge to `0`.

Drift hints are on by default. To disable them:

```bash
ferron run --config-params 'drift_hints=false;file=ferron.conf' --config-adapter ferronconf
```

The `GET /status` endpoint of the admin API reports the state in the `config_drift` and `config_drift_hints_enabled` fields.

## See also

- [Conditionals and variables](/docs/configuration/fundamentals/conditionals)
- [Formatting a configuration](/docs/configuration/fundamentals/formatting): `ferron-fmt` for formatting
- [Routing and URL processing](/docs/configuration/routing/url-processing) (`location`, `if`, `if_not`)
- [Directives](/docs/configuration/server/core-directives)
