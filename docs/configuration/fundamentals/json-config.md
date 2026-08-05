---
title: "JSON configuration and adapt command"
description: "Use ferron adapt to output configuration as JSON. Work with JSON-formatted configurations."
---

This page covers the `ferron adapt` command, the JSON configuration format, and how to work with JSON-based configurations. The `config-ferronconf` module parses `.conf` files. The `config-json` module parses `.json` files.

> [!info]
> For configuration format details, see [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax), [Conditionals and variables](/docs/v3/configuration/fundamentals/conditionals), and [Core directives](/docs/v3/configuration/server/core-directives).

## The adapt command

The `adapt` command converts `.conf` configuration files to JSON. This is useful for debugging, programmatic configuration generation, or understanding how Ferron parses your configuration.

```bash
ferron adapt -c ferron.conf
```

This reads your `.conf` file and outputs the complete parsed configuration as JSON to standard output.

**Configuration example:**

Given a `ferron.conf` file:

```ferron
{
    runtime {
        io_uring
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

> [!note]
> The `ferron adapt` command produces JSON that faithfully represents the parsed configuration. You can represent Boolean directives as `"args": []` (flag-style, treated as `true`) or with explicit booleans. When spans are `null`, the system likely constructed the configuration programmatically.

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
| `span` | `ServerConfigurationSpan \| null` | Where this block is |

Blocks appear at multiple levels:

- **Global configuration**: server-wide settings
- **Port/host configuration**: protocol and host-specific settings
- **Nested directives**: child blocks within directive entries (for example, `runtime { io_uring }`)

### Directive entries

Each directive entry represents one directive:

| Field | Type | Description |
|-------|------|-------------|
| `args` | `Vec<ServerConfigurationValue>` | Arguments provided to this directive |
| `children` | `ServerConfigurationBlock \| null` | Optional nested configuration block |
| `span` | `ServerConfigurationSpan \| null` | Where this directive is |

Multiple entries with the same name can exist in a single block, allowing for repeated directives.

### Configuration values

`ServerConfigurationValue` uses a tagged union to represent different value types:

| Variant | JSON structure | Example |
|---------|----------------|---------|
| `String` | `["String", [value, span]]` | `["String", ["/var/www/html", {"line": 8, "column": 8, "file": "ferron.conf"}]]` |
| `Number` | `["Number", [value, span]]` | `["Number", [8080, null]]` |
| `Float` | `["Float", [value, span]]` | `["Float", [3.14, null]]` |
| `Boolean` | `["Boolean", [value, span]]` | `["Boolean", [true, {"line": 3, "column": 14, "file": "ferron.conf"}]]` |
| `InterpolatedString` | `["InterpolatedString", [parts, span]]` | See interpolated strings section below |

Span information can be null.

### Interpolated strings

Interpolated strings use `{{name}}` syntax. The system represents them as an array of parts:

| Part type | JSON structure | Description |
|-----------|----------------|-------------|
| `String` | `["String", literal_text]` | Literal text content |
| `Variable` | `["Variable", var_name]` | Variable reference that the system resolves |

Variables resolve at runtime:

- `env.NAME`: resolved from environment variables
- `NAME`: resolved from the consumer's variable map

If the system cannot resolve a variable, the placeholder stays as `{{NAME}}` in the output.

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
| `port` | `u16 \| null` | Port number (the protocol may provide a default) |
| `hosts` | `Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>` | Host configurations with filters |

### Host filters

`ServerConfigurationHostFilters` controls which host/IP a port configuration applies to:

| Field | Type | Description |
|-------|------|-------------|
| `ip` | `IpAddr \| null` | IP address to match (for multi-homed servers) |
| `host` | `String \| null` | Host/domain name to match (for SNI) |

When both are `null`, the configuration applies to all hosts on that port (for example, `*:8080`).

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

The system preserves span information in JSON output to provide accurate error messages during validation and runtime.

## Configuration adapters

Ferron uses a pluggable adapter system for loading configuration from different sources.

### Built-in adapters

| Adapter | File extensions | Description |
|---------|-----------------|-------------|
| `config-ferronconf` | `.conf` | Parses Ferron's custom configuration syntax |
| `config-json` | `.json` | Loads JSON configuration directly |

### Selecting an adapter

Ferron detects the adapter based on file extension. You can override it explicitly:

```bash
ferron run -c config.json --config-adapter json
ferron validate -c myconfig.conf --config-adapter ferronconf
```

### Adapter interface

Adapters implement the `ConfigurationAdapter` trait, which defines:

- `adapt(params)`: loads and parses configuration, returning a `ServerConfiguration` and a `ConfigurationWatcher`
- `file_extension()`: lists file extensions this adapter handles

The `ConfigurationWatcher` monitors the configuration source for changes and triggers hot-reload when the source changes.

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

- **Programmatic generation**: tools and scripts can generate configuration without parsing Ferron's custom syntax
- **API integration**: external systems can push configuration as JSON
- **Debugging**: inspect how Ferron parsed your `.conf` file
- **Testing**: precise control over configuration structure in automated tests

### Loading JSON configurations

You can load JSON configurations directly:

```bash
ferron run -c config.json
ferron validate -c config.json
```

Ferron detects the adapter from the `.json` extension.

### Hot-reload support

JSON configuration files support hot-reload. When the file changes, Ferron detects the update and reloads the configuration gracefully. The `ConfigurationWatcher` monitors the file for modifications.

To enable hot reloading, specify a `watch` configuration adapter parameter:

```bash
ferron run --config-params 'watch=1;file=ferron.json' --config-adapter json
```

### Configuration drift hints

By default, hot-reload is off. Ferron can still detect when the JSON configuration file changes on disk but you have not reloaded it. Drift hints are on by default. Disable with `drift_hints=false`:

```bash
ferron run --config-params 'drift_hints=false;file=ferron.json' --config-adapter json
```

See [Configuration drift hints](/docs/v3/configuration/fundamentals/syntax#configuration-drift-hints) for details.

## See also

- [Configuration validation](/docs/v3/configuration/fundamentals/validation)
- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
- [Configuration formatting](/docs/v3/configuration/fundamentals/formatting): `ferron-fmt` for formatting `.conf` files
- [Core directives](/docs/v3/configuration/server/core-directives)
