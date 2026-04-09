---
title: "JSON configuration and adapt command"
description: "Using ferron adapt to output configuration as JSON, and working with JSON-formatted configurations."
---

This page covers the `ferron adapt` command, the JSON configuration format, and how to work with JSON-based configurations.

## The adapt command

The `adapt` command converts `.conf` configuration files into their JSON representation. This is useful for debugging, programmatic configuration generation, or understanding how Ferron parses your configuration.

```bash
ferron adapt -c ferron.conf
```

This reads your `.conf` file and outputs the complete parsed configuration as JSON to standard output.

**Configuration example:**

Given a `ferron.conf` file:

```ferron
{
  runtime {
    io_uring true
  }
}

*:8080 {
  root /var/www/html
}
```

Running `ferron adapt -c ferron.conf` outputs:

```json
{
  "global_config": {
    "directives": {
      "runtime": [
        {
          "args": [],
          "children": {
            "directives": {
              "io_uring": [
                {
                  "args": [
                    {
                      "Boolean": [
                        true,
                        {
                          "line": 3,
                          "column": 14,
                          "file": "/path/to/ferron.conf"
                        }
                      ]
                    }
                  ],
                  "children": null,
                  "span": {
                    "line": 3,
                    "column": 5,
                    "file": "/path/to/ferron.conf"
                  }
                }
              ]
            },
            "matchers": {},
            "span": {
              "line": 2,
              "column": 11,
              "file": "/path/to/ferron.conf"
            }
          },
          "span": {
            "line": 2,
            "column": 3,
            "file": "/path/to/ferron.conf"
          }
        }
      ]
    },
    "matchers": {},
    "span": {
      "line": 1,
      "column": 1,
      "file": "/path/to/ferron.conf"
    }
  },
  "ports": {
    "http": [
      {
        "port": 8080,
        "hosts": [
          [
            {
              "ip": null,
              "host": null
            },
            {
              "directives": {
                "root": [
                  {
                    "args": [
                      {
                        "String": [
                          "/var/www/html",
                          {
                            "line": 8,
                            "column": 8,
                            "file": "/path/to/ferron.conf"
                          }
                        ]
                      }
                    ],
                    "children": null,
                    "span": {
                      "line": 8,
                      "column": 3,
                      "file": "/path/to/ferron.conf"
                    }
                  }
                ]
              },
              "matchers": {},
              "span": {
                "line": 7,
                "column": 8,
                "file": "/path/to/ferron.conf"
              }
            }
          ]
        ]
      }
    ]
  }
}
```

## JSON configuration structure

The JSON configuration follows a hierarchical structure that mirrors Ferron's internal configuration model.

### Root object

The top-level configuration contains two main sections:

| Field | Type | Description |
|-------|------|-------------|
| `global_config` | `ServerConfigurationBlock` | Global configuration applying to all protocols |
| `ports` | `BTreeMap<String, Vec<ServerConfigurationPort>>` | Per-protocol port configurations, keyed by protocol name (e.g., "http", "https", "tcp") |

### Configuration blocks

A `ServerConfigurationBlock` represents a scope of directives:

| Field | Type | Description |
|-------|------|-------------|
| `directives` | `HashMap<String, Vec<ServerConfigurationDirectiveEntry>>` | All directives in this block, indexed by name |
| `matchers` | `HashMap<String, ServerConfigurationMatcher>` | Named matcher expressions for conditional directives |
| `span` | `ServerConfigurationSpan \| null` | Source location of this block |

Blocks appear at multiple levels:

- **Global configuration** — server-wide settings
- **Port/host configuration** — protocol and host-specific settings
- **Nested directives** — child blocks within directive entries (e.g., `runtime { io_uring true }`)

### Directive entries

Each directive entry represents one occurrence of a directive:

| Field | Type | Description |
|-------|------|-------------|
| `args` | `Vec<ServerConfigurationValue>` | Arguments provided to this directive |
| `children` | `ServerConfigurationBlock \| null` | Optional nested configuration block |
| `span` | `ServerConfigurationSpan \| null` | Source location of this directive |

Multiple entries with the same name can exist in a single block, allowing for repeated directives.

### Configuration values

`ServerConfigurationValue` is a tagged union representing different value types:

| Variant | JSON structure | Example |
|---------|----------------|---------|
| `String` | `["String", [value, span]]` | `["String", ["/var/www/html", {"line": 8, "column": 8, "file": "ferron.conf"}]]` |
| `Number` | `["Number", [value, span]]` | `["Number", [8080, null]]` |
| `Float` | `["Float", [value, span]]` | `["Float", [3.14, null]]` |
| `Boolean` | `["Boolean", [value, span]]` | `["Boolean", [true, {"line": 3, "column": 14, "file": "ferron.conf"}]]` |
| `InterpolatedString` | `["InterpolatedString", [parts, span]]` | See interpolated strings section below |

Span information is optional and can be `null`.

### Interpolated strings

Interpolated strings use `{{name}}` syntax and are represented as an array of parts:

| Part type | JSON structure | Description |
|-----------|----------------|-------------|
| `String` | `["String", literal_text]` | Literal text content |
| `Variable` | `["Variable", var_name]` | Variable reference to be resolved |

Variables are resolved at runtime:

- `env.NAME` — resolved from environment variables
- `NAME` — resolved from the consumer's variable map

If a variable cannot be resolved, the placeholder is kept as `{{NAME}}` in the output.

**Configuration example:**

```json
{
  "InterpolatedString": [
    [
      ["String", "/certs/"],
      ["Variable", "env.DOMAIN"],
      ["String", ".crt"]
    ],
    {
      "line": 15,
      "column": 10,
      "file": "ferron.conf"
    }
  ]
}
```

### Port configurations

Each entry in the `ports` map represents a protocol:

| Field | Type | Description |
|-------|------|-------------|
| `port` | `u16 \| null` | Port number (may be inherited from protocol defaults) |
| `hosts` | `Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>` | Host configurations with filters |

### Host filters

`ServerConfigurationHostFilters` controls which host/IP a port configuration applies to:

| Field | Type | Description |
|-------|------|-------------|
| `ip` | `IpAddr \| null` | IP address to match (for multi-homed servers) |
| `host` | `String \| null` | Host/domain name to match (for SNI) |

When both are `null`, the configuration applies to all hosts on that port (e.g., `*:8080`).

### Match expressions

Named matchers contain expressions for conditional configuration:

| Field | Type | Description |
|-------|------|-------------|
| `exprs` | `Vec<ServerConfigurationMatcherExpr>` | List of expressions to evaluate |
| `span` | `ServerConfigurationSpan \| null` | Source location |

Each expression has three components:

| Field | Type | Description |
|-------|------|-------------|
| `left` | `ServerConfigurationMatcherOperand` | Left operand |
| `right` | `ServerConfigurationMatcherOperand` | Right operand |
| `op` | `ServerConfigurationMatcherOperator` | Comparison operator |

Operands can be:

| Variant | JSON structure | Example |
|---------|----------------|---------|
| `Identifier` | `["Identifier", name]` | `["Identifier", "request.method"]` |
| `String` | `["String", value]` | `["String", "GET"]` |
| `Integer` | `["Integer", value]` | `["Integer", 8080]` |
| `Float` | `["Float", value]` | `["Float", 3.14]` |

Supported operators:

| Operator | JSON value | Meaning |
|----------|------------|---------|
| `==` | `["Eq"]` | String equality |
| `!=` | `["NotEq"]` | String inequality |
| `~` | `["Regex"]` | Regex match |
| `!~` | `["NotRegex"]` | Regex non-match |
| `in` | `["In"]` | Membership check |

### Span metadata

`ServerConfigurationSpan` tracks source locations for error reporting:

| Field | Type | Description |
|-------|------|-------------|
| `line` | `usize` | Line number (1-indexed) |
| `column` | `usize` | Column number (1-indexed) |
| `file` | `String \| null` | Source file path |

Span information is preserved in JSON output to provide accurate error messages during validation and runtime.

## Configuration adapters

Ferron uses a pluggable adapter system for loading configuration from different sources.

### Built-in adapters

| Adapter | File extensions | Description |
|---------|-----------------|-------------|
| `config-ferronconf` | `.conf` | Parses Ferron's custom configuration syntax |
| `config-json` | `.json` | Loads JSON configuration directly |

### Selecting an adapter

The adapter is auto-detected based on file extension. You can override it explicitly:

```bash
ferron run -c config.json --config-adapter json
ferron validate -c myconfig.conf --config-adapter ferronconf
```

### Adapter interface

Adapters implement the `ConfigurationAdapter` trait, which defines:

- `adapt(params)` — load and parse configuration, returning a `ServerConfiguration` and a `ConfigurationWatcher`
- `file_extension()` — list of file extensions this adapter handles

The `ConfigurationWatcher` monitors the configuration source for changes and triggers hot-reload when the source is updated.

**Configuration example:**

```json
{
  "global_config": {
    "directives": {
      "runtime": [
        {
          "args": [],
          "children": {
            "directives": {
              "io_uring": [
                {
                  "args": [
                    {
                      "Boolean": [true, null]
                    }
                  ],
                  "children": null,
                  "span": null
                }
              ]
            },
            "matchers": {},
            "span": null
          },
          "span": null
        }
      ]
    },
    "matchers": {},
    "span": null
  },
  "ports": {
    "http": [
      {
        "port": 8080,
        "hosts": [
          [
            {
              "ip": null,
              "host": null
            },
            {
              "directives": {
                "root": [
                  {
                    "args": [
                      {
                        "String": ["/var/www/html", null]
                      }
                    ],
                    "children": null,
                    "span": null
                  }
                ]
              },
              "matchers": {},
              "span": null
            }
          ]
        ]
      }
    ]
  }
}
```

## Working with JSON configurations

JSON configurations are typically used in the following scenarios:

- **Programmatic generation** — tools and scripts can generate configuration without parsing Ferron's custom syntax
- **API integration** — external systems can push configuration as JSON
- **Debugging** — inspect how Ferron parsed your `.conf` file
- **Testing** — precise control over configuration structure in automated tests

### Loading JSON configurations

You can load JSON configurations directly:

```bash
ferron run -c config.json
ferron validate -c config.json
```

The adapter is auto-detected from the `.json` extension.

### Hot-reload support

JSON configuration files support hot-reload. When the file changes, Ferron detects the update and reloads the configuration gracefully. The `ConfigurationWatcher` monitors the file for modifications.

## Notes and troubleshooting

- The JSON output from `ferron adapt` is a faithful representation of the parsed configuration, including all span metadata for error reporting.
- Boolean directives can be represented as `"args": []` (flag-style, treated as `true`) or `"args": [{"Boolean": [true, null]}]` (explicit boolean).
- When spans are `null`, the configuration was likely constructed programmatically rather than parsed from a file.
- The `ports` map is organized by protocol name (e.g., "http", "https", "tcp"). Each protocol can have multiple ports.
- Host configurations are stored as tuples of `(filters, block)`. The filters control which hosts the configuration applies to.
- For the `.conf` file format and syntax details, see [Syntax and file structure](/docs/v3/configuration/syntax).
- For conditional matchers and variables, see [Conditionals and variables](/docs/v3/configuration/conditionals).
- For how configuration is processed at runtime, see [Core directives](/docs/v3/configuration/core-directives).
