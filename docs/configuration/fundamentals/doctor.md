---
title: "Configuration doctor"
description: "Using ferron doctor to detect best-practice violations and security misconfigurations."
---

`ferron doctor` extends configuration validation with best-practice checks for security, reliability, and operational hygiene.

## The doctor command

The `doctor` command runs the same structural validation as `ferron validate`. It also reports configuration patterns that are valid but differ from recommended practice.

```bash
ferron doctor -c ferron.conf
```

Exit codes:

- `0`: no findings, or only best-practice violations.
- `1`: structural errors.

> [!note]
> Best-practice violations are advisory and do not block startup. Treat the findings as guidance, not as absolute rules.
>
> `ferron validate` suppresses doctor diagnostics. Use `ferron doctor` to see them.

> [!tip]
> Some checks are contextual and fire only on specific directive combinations. The doctor cannot catch every security-relevant pattern at configuration time. Runtime monitoring and network controls still matter. For the full list of checks, see the pages in the "Configuration" category.

### Log output

By default, Ferron prints diagnostics as log messages:

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
> The JSON output format is stable and safe for tools and CI pipelines to parse.

## How it differs from validate

| Feature                  | `ferron validate` | `ferron doctor`     |
| ------------------------ | ----------------- | ------------------- |
| Unknown directives       | Reported          | Reported            |
| Invalid configuration    | Reported (errors) | Reported (errors)   |
| Best practice violations | Suppressed        | Reported (advisory) |

The `validate` command strips `BestPracticeViolation` diagnostics from its output. The `doctor` command retains them. All other behavior is identical. The same validators run in the same order.

## Diagnostic kind

Best-practice violations use the `"Best practice violation"` diagnostic kind. They are advisory: the server can start with these patterns, but they may indicate security risks or operational issues.

## See also

- [Configuration validation](/docs/configuration/fundamentals/validation)
- [Configuration formatting](/docs/configuration/fundamentals/formatting): `ferron-fmt` for formatting `.conf` files
- [Syntax and file structure](/docs/configuration/fundamentals/syntax)
- [Security and TLS](/docs/configuration/security/tls)
