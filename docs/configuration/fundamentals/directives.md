---
title: "Configuration directives"
description: "Listing available directives with the ferron directives command."
---

The `ferron directives` command prints every configuration directive registered by the loaded modules as a JSON document. It does not validate a configuration file — it only reflects the directive schema known to the running binary.

```bash
ferron directives
```

> [!tip]
> Pipe the output to a JSON processor (for example, `jq`) for filtering, or save it to a file for reference.

## Output structure

The JSON output is an object whose keys are **directive sections** — logical groupings of related directives. Each section maps to a list of directive definitions:

```json
{
  "section_name": [
    {
      "name": "directive_name",
      "usage": "directive_name <arg>",
      "description": "What the directive does.",
      "applicable_protocols": ["http"],
      "global_only": false,
      "subblock_link": null
    }
  ]
}
```

## Directive fields

| Field                  | Type               | Description                                                                                                                                                          |
| ---------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                 | `string`           | The directive name as it appears in the configuration file.                                                                                                          |
| `usage`                | `string`           | A usage hint showing the expected argument shape. `<arg>` indicates a required value, `[bool]` an optional boolean flag, and `{ ... }` a block with sub-directives.  |
| `description`          | `string`           | A short human-readable description of the directive's purpose.                                                                                                       |
| `applicable_protocols` | `string[] \| null` | The protocols this directive can appear in (e.g. `["http"]`). `null` means the directive is valid globally or in all protocol contexts.                              |
| `global_only`          | `bool`             | If `true`, the directive can only appear at the top level of the configuration file (outside any host block).                                                        |
| `subblock_link`        | `string \| null`   | When non-null, the directive has child directives registered under this subblock name. The child directives are grouped under a separate section with the same name. |

## Sections

Sections group directives that belong to the same logical area. For example, `http_proxy` contains reverse-proxy directives, while `http_proxy_upstream` contains per-upstream-server directives. Section names are prefixed with `custom_` for module-contributed directives, or are `default` for core directives.

## Example

```bash
ferron directives | jq '.default'
```

```json
[
  {
    "name": "runtime",
    "usage": "runtime { ... }",
    "description": "This directive specifies global runtime settings.",
    "applicable_protocols": null,
    "global_only": true,
    "subblock_link": "custom_global_runtime"
  },
  ...
]
```

> [!note]
> The directive list depends on which modules are compiled into the binary. A minimal custom build may expose fewer directives than the default binary.

## See also

- [Configuration validation](/docs/v3/configuration/fundamentals/validation) — `ferron validate` for checking a configuration file
- [Configuration doctor](/docs/v3/configuration/fundamentals/doctor) — `ferron doctor` for best-practice checks
- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
