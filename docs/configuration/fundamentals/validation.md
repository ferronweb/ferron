---
title: "Configuration validation"
description: "Using ferron validate to check your configuration, understanding diagnostics, and working with JSON validation output."
---

`ferron validate` checks a configuration without starting the server. It reports the diagnostic kinds below and can write JSON output.

## The validate command

```bash
ferron validate -c ferron.conf
```

The command exits with code `0` for a valid configuration and `1` for errors.

> [!note]
> Validation is module-aware. A directive from a loaded module is valid, and a directive from no loaded module is unknown. Validation cannot prove runtime correctness, because some problems surface only at runtime.

### Log output

By default, validation prints diagnostics as log messages:

```ferron
*:443 {
    tlss
}
```

```text
$ ferron validate -c ferron.conf
[2026-05-30 07:18:34.372 WARN] Unknown directive (block 'http port 443' in file '/home/.../ferron.conf' at line 1, column 7): `tlss` is unused in the block
```

The configuration above is still structurally valid. `tlss` is unknown, so it does not prevent startup, and validation exits with code `0`.

### JSON output

Use the `--json` (or `-j`) flag for machine-readable output:

```bash
ferron validate -c ferron.conf --json
```

```json
{
  "valid": true,
  "diagnostics": [
    {
      "kind": "Unknown directive",
      "message": "`tlss` is unused in the block",
      "span": {
        "line": 1,
        "column": 7,
        "file": "/home/.../ferron.conf"
      },
      "scope": "http port 443"
    }
  ]
}
```

## Response structure

The JSON response contains two top-level fields:

| Field         | Type              | Description                                                                                |
| ------------- | ----------------- | ------------------------------------------------------------------------------------------ |
| `valid`       | `bool`            | Whether the configuration is valid. `false` only when the server finds invalid directives. |
| `diagnostics` | `Vec<Diagnostic>` | List of diagnostic messages, warnings, and errors.                                         |

### Diagnostic fields

Each diagnostic entry contains:

| Field     | Type             | Description                                                                                                 |
| --------- | ---------------- | ----------------------------------------------------------------------------------------------------------- |
| `kind`    | `string`         | The diagnostic category (`"Unknown directive"`, `"Invalid configuration"`, or `"Best practice violation"`). |
| `message` | `string`         | A human-readable description of the issue.                                                                  |
| `span`    | `Span \| null`   | Source location (line, column, file) where the issue occurred.                                              |
| `scope`   | `string \| null` | The configuration block scope (for example, `"http port 443"`, `"global"`).                                 |

## Diagnostic kinds

### Unknown directive

A directive in the configuration belongs to no loaded module. Ferron reports this as a warning, and the server can still start.

```ferron
example.com {
    non_existent_directive
}
```

```json
{
  "valid": true,
  "diagnostics": [
    {
      "kind": "Unknown directive",
      "message": "`non_existent_directive` is unused in the block",
      "span": { "line": 2, "column": 3, "file": "ferron.conf" },
      "scope": "http example.com"
    }
  ]
}
```

### Invalid configuration

A recognized directive has invalid arguments or a misconfigured value. Ferron reports this as an error. Validation fails and the server does not start.

```ferron
# INVALID: bogus TLS provider
example.com {
    tls {
        provider bogus
    }
}
```

```json
{
  "valid": false,
  "diagnostics": [
    {
      "kind": "Invalid configuration",
      "message": "unknown tls provider: `bogus`",
      "span": { "line": 3, "column": 5, "file": "ferron.conf" },
      "scope": "http example.com"
    }
  ]
}
```

When `valid` is `false`, the command exits with code 1.

### Best practice violation

A configuration pattern is valid but differs from recommended practice. The diagnostic is advisory. `ferron validate` suppresses it and `ferron doctor` reports it. The server can start with these patterns.

```ferron
example.com {
    directory_listing
}
```

```json
{
  "valid": true,
  "diagnostics": [
    {
      "kind": "Best practice violation",
      "message": "`directory_listing` exposes generated indexes for directories without index files; enable it only for intentionally public file listings",
      "span": { "line": 2, "column": 5, "file": "ferron.conf" },
      "scope": "http example.com"
    }
  ]
}
```

## Span metadata

The `span` field gives the exact location of the diagnostic in the configuration file:

| Field    | Type             | Description                                                                         |
| -------- | ---------------- | ----------------------------------------------------------------------------------- |
| `line`   | `usize`          | Line number, 1-indexed.                                                             |
| `column` | `usize`          | Column number, 1-indexed.                                                           |
| `file`   | `string \| null` | Absolute path to the configuration file, or `null` if constructed programmatically. |

## Validation scope

Validation runs at two levels:

- Global configuration: directives inside the top-level `{ ... }` block.
- Per-protocol blocks: host blocks such as `example.com`, `*:443`, and `http *:8080`.

Each protocol registers its own validators through the module system. Without a module, its directives appear as unknown. A custom build with fewer features can trigger this.

## Configuration adapters and validation

The `validate` command works with any configuration adapter. Ferron detects the adapter from the file extension:

```bash
ferron validate -c ferron.conf    # uses config-ferronconf adapter
ferron validate -c config.json    # uses config-json adapter
```

You can also name the adapter:

```bash
ferron validate -c config.json --config-adapter json
```

Validation runs after the adapter loads and parses the configuration. A parse error is reported directly.

## Related commands

### ferron doctor

The `doctor` command adds best-practice checks for security, reliability, and operational hygiene. It reports advisory diagnostics for patterns that are valid but differ from recommended practice:

```bash
ferron doctor -c ferron.conf
```

> [!info]
> See [Configuration doctor](/docs/configuration/fundamentals/doctor) for the full list of checks.

### ferron adapt

The `adapt` command writes the parsed configuration as JSON and runs no validators. Use it to inspect how Ferron reads a configuration:

```bash
ferron adapt -c ferron.conf
```

> [!tip]
> The JSON output format is stable and safe for tools and CI pipelines to parse.

> [!info]
> See [JSON configuration and adapt command](/docs/configuration/fundamentals/json-config) for details.

### ferron run

The `run` command runs the same validation during startup. If validation fails, the server exits with an error.

> [!note]
> An unknown directive produces a warning. The server starts and ignores the directive. An invalid configuration produces an error and the server does not start.

## See also

- [JSON configuration and adapt command](/docs/configuration/fundamentals/json-config)
- [Configuration formatting](/docs/configuration/fundamentals/formatting): `ferron-fmt` for formatting `.conf` files
- [Syntax and file structure](/docs/configuration/fundamentals/syntax)
- [Core directives](/docs/configuration/server/core-directives)
