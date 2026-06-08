---
title: "Configuration doctor"
description: "Using ferron doctor to detect best-practice violations and security misconfigurations."
---

This page documents the `ferron doctor` command, which extends configuration validation with best-practice checks for security, reliability, and operational hygiene.

## The doctor command

The `doctor` command runs the same structural validation as `ferron validate`, then additionally checks for configuration patterns that are technically valid but deviate from recommended practices.

```bash
ferron doctor -c ferron.conf
```

If the configuration is valid and contains no best-practice violations, the command exits with code 0. If violations are found, it still exits with code 0 (they are advisory, not errors). If structural errors are found, it exits with code 1.

> [!note]
>
> - Best-practice violations are advisory — they do not prevent the server from starting. Treat the findings as opinionated guidance, not absolute truth.
> - The `ferron validate` command suppresses doctor diagnostics — use `ferron doctor` to see them.

> [!tip]
> Some checks are contextual and only fire when specific directive combinations are detected. Not all security-relevant patterns can be detected at configuration time — runtime monitoring and network controls remain important. For the full list of detected best-practice violations, see the respective documentation pages in the "Configuration" category.

### Log output

By default, diagnostics are printed as log messages:

```text
$ ferron doctor -c ferron.conf
[2026-05-30 07:18:34.372 INFO] Best practice violation (block 'http example.com' in file 'ferron.conf' at line 5, column 5): `directory_listing` exposes generated indexes for directories without index files; enable it only for intentionally public file listings
```

### JSON output

Use the `--json` (or `-j`) flag for machine-readable output:

```bash
ferron doctor -c ferron.conf --json
```

```json
{
  "valid": true,
  "diagnostics": [
    {
      "kind": "Best practice violation",
      "message": "`directory_listing` exposes generated indexes for directories without index files; enable it only for intentionally public file listings",
      "span": { "line": 5, "column": 5, "file": "ferron.conf" },
      "scope": "http example.com"
    }
  ]
}
```

> [!note]
> The JSON output format is stable and suitable for programmatic consumption by tools and CI pipelines.

## How it differs from validate

| Feature | `ferron validate` | `ferron doctor` |
|---------|-------------------|-----------------|
| Unknown directives | Reported | Reported |
| Invalid configuration | Reported (errors) | Reported (errors) |
| Best practice violations | Suppressed | Reported (advisory) |

The `validate` command strips `BestPracticeViolation` diagnostics from its output. The `doctor` command retains them. All other behavior is identical — the same validators run in the same order.

## Diagnostic kind

Best-practice violations use the `"Best practice violation"` diagnostic kind. They are advisory: the server can start with these patterns, but they may indicate security risks or operational issues.

## See also

- [Configuration validation](/docs/v3/configuration/fundamentals/validation)
- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
- [Security and TLS](/docs/v3/configuration/security/tls)
