---
title: "Configuration validation"
description: "Using ferron validate to check your configuration, understanding diagnostics, and working with JSON validation output."
---

This page covers the `ferron validate` command, the types of diagnostics it produces, and how to interpret validation output.

## The validate command

The `validate` command checks your configuration file for errors and unknown directives without starting the server.

```bash
ferron validate -c ferron.conf
```

If the configuration is valid, the command exits with code 0. If there are errors, it exits with code 1.

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

The configuration above is still structurally valid — `tlss` is an unrecognized directive, but it does not prevent the server from starting. Validation exits with code 0 in this case.

### JSON output

Use the `--json` (or `-j`) flag to get machine-readable output:

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

| Field | Type | Description |
|-------|------|-------------|
| `valid` | `bool` | Whether the configuration is valid. `false` only when invalid directives are found. |
| `diagnostics` | `Vec<Diagnostic>` | List of diagnostic messages, warnings, and errors. |

### Diagnostic fields

Each diagnostic entry contains:

| Field | Type | Description |
|-------|------|-------------|
| `kind` | `string` | The diagnostic category — `"Unknown directive"`, `"Invalid configuration"`, or `"Best practice violation"`. |
| `message` | `string` | A human-readable description of the issue. |
| `span` | `Span \| null` | Source location (line, column, file) where the issue occurred. |
| `scope` | `string \| null` | The configuration block scope (e.g., `"http port 443"`, `"global"`). |

## Diagnostic kinds

### Unknown directive

A directive in the configuration is not recognized by any loaded module. This is reported as a **warning** — the server can still start.

```ferron
example.com {
  non_existent_directive true
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

A recognized directive has invalid arguments or a misconfigured value. This is an **error** — validation fails and the server cannot start with this configuration.

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

A configuration pattern is technically valid but deviates from recommended practices. This is an **advisory** diagnostic — `ferron validate` suppresses it, and `ferron doctor` reports it. The server can start with these patterns.

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

The `span` field pinpoints the exact location of the diagnostic in the configuration file:

| Field | Type | Description |
|-------|------|-------------|
| `line` | `usize` | Line number, 1-indexed. |
| `column` | `usize` | Column number, 1-indexed. |
| `file` | `string \| null` | Absolute path to the configuration file, or `null` if constructed programmatically. |

## Validation scope

Validation runs against two levels:

- **Global configuration** — directives inside the top-level `{ ... }` block
- **Per-protocol blocks** — host blocks such as `example.com`, `*:443`, `http *:8080`

Each protocol registers its own validators via the module system. If a module is not loaded (e.g., a custom build with a reduced feature set), its directives will not be recognized and will show as unknown.

## Configuration adapters and validation

The `validate` command works with any configuration adapter. The adapter is auto-detected from the file extension:

```bash
ferron validate -c ferron.conf    # uses config-ferronconf adapter
ferron validate -c config.json    # uses config-json adapter
```

You can also specify an adapter explicitly:

```bash
ferron validate -c config.json --config-adapter json
```

Validation runs after the adapter has loaded and parsed the configuration. If the configuration file cannot be parsed, validation reports the parse error directly.

## Related commands

### ferron doctor

The `doctor` command extends validation with best-practice checks for security, reliability, and operational hygiene. It reports advisory diagnostics for patterns that are technically valid but deviate from recommended practices:

```bash
ferron doctor -c ferron.conf
```

See [Configuration doctor](/docs/v3/configuration/fundamentals/doctor) for the full list of checks.

### ferron adapt

The `adapt` command outputs the parsed configuration as JSON without running validators. This is useful for inspecting how Ferron interprets your configuration:

```bash
ferron adapt -c ferron.conf
```

See [JSON configuration and adapt command](/docs/v3/configuration/fundamentals/json-config) for details.

### ferron run

The `run` command performs the same validation during startup. If validation fails, the server exits with an error.

## Notes and troubleshooting

- **Unknown directives produce warnings, not errors.** The server can start, but the directive is silently ignored.
- **Invalid configurations produce errors.** The server will not start.
- **Validation is module-aware.** A directive recognized by a loaded module is valid; an unrecognized one is flagged as unknown.
- **Validation does not guarantee runtime correctness.** Some issues (e.g., unresolvable upstream hosts, permission errors) can only be detected at runtime.
- The JSON output format is stable and suitable for programmatic consumption by tools and CI pipelines.

### See also

- [JSON configuration and adapt command](/docs/v3/configuration/fundamentals/json-config)
- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
- [Core directives](/docs/v3/configuration/server/core-directives)
