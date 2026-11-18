---
title: "JSON configuration and adapt command"
description: "Use ferron adapt to output configuration as JSON. Work with JSON-formatted configurations."
---

`ferron adapt` writes a configuration as JSON. The `config-ferronconf` module parses `.conf` files and the `config-json` module parses `.json` files.

> [!info]
> For the configuration format, see [Syntax and file structure](/docs/configuration/fundamentals/syntax), [Conditionals and variables](/docs/configuration/fundamentals/conditionals), and [Core directives](/docs/configuration/server/core-directives).

## The adapt command

The `adapt` command converts a `.conf` file to JSON. Use it to debug a configuration, generate configuration from a tool, or see how Ferron parses a file.

```bash
ferron adapt -c ferron.conf
```

The command reads the `.conf` file and writes the parsed configuration as JSON to standard output.

For this `ferron.conf`:

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
> The `ferron adapt` output represents the parsed configuration exactly. A Boolean directive is either a flag with `"args": []` (treated as `true`) or an explicit boolean value. A `null` span means the configuration was built programmatically.

## JSON configuration structure

The JSON configuration mirrors the internal configuration model.

### Root object

The top level has two sections:

| Field           | Type                                             | Description                                                                                    |
| --------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `global_config` | `ServerConfigurationBlock`                       | Global configuration applying to all protocols                                                 |
| `ports`         | `BTreeMap<String, Vec<ServerConfigurationPort>>` | Per-protocol port configurations, keyed by protocol name (for example, "http", "https", "tcp") |

### Configuration blocks

A `ServerConfigurationBlock` represents a scope of directives:

| Field        | Type                                                      | Description                                          |
| ------------ | --------------------------------------------------------- | ---------------------------------------------------- |
| `directives` | `HashMap<String, Vec<ServerConfigurationDirectiveEntry>>` | All directives in this block, indexed by name        |
| `matchers`   | `HashMap<String, ServerConfigurationMatcher>`             | Named matcher expressions for conditional directives |
| `span`       | `ServerConfigurationSpan \| null`                         | Where this block is                                  |

Blocks appear at several levels:

- Global configuration: server-wide settings.
- Port and host configuration: protocol and host-specific settings.
- Nested directives: child blocks inside a directive entry, for example `runtime { io_uring }`.

### Directive entries

Each directive entry represents one directive:

| Field      | Type                               | Description                          |
| ---------- | ---------------------------------- | ------------------------------------ |
| `args`     | `Vec<ServerConfigurationValue>`    | Arguments provided to this directive |
| `children` | `ServerConfigurationBlock \| null` | Optional nested configuration block  |
| `span`     | `ServerConfigurationSpan \| null`  | Where this directive is              |

Multiple entries with the same name can exist in one block, which allows repeated directives.

### Configuration values

`ServerConfigurationValue` uses a tagged union to represent different value types:

| Variant              | JSON structure                          | Example                                                                          |
| -------------------- | --------------------------------------- | -------------------------------------------------------------------------------- |
| `String`             | `["String", [value, span]]`             | `["String", ["/var/www/html", {"line": 8, "column": 8, "file": "ferron.conf"}]]` |
| `Number`             | `["Number", [value, span]]`             | `["Number", [8080, null]]`                                                       |
| `Float`              | `["Float", [value, span]]`              | `["Float", [3.14, null]]`                                                        |
| `Boolean`            | `["Boolean", [value, span]]`            | `["Boolean", [true, {"line": 3, "column": 14, "file": "ferron.conf"}]]`          |
| `InterpolatedString` | `["InterpolatedString", [parts, span]]` | See interpolated strings section below                                           |

Span information can be null.

### Interpolated strings

Interpolated strings use the `{{name}}` syntax. JSON represents them as an array of parts:

| Part type  | JSON structure             | Description                                 |
| ---------- | -------------------------- | ------------------------------------------- |
| `String`   | `["String", literal_text]` | Literal text content                        |
| `Variable` | `["Variable", var_name]`   | Variable reference that Ferron resolves     |

Variables resolve at runtime:

- `env.NAME`: from environment variables.
- `NAME`: from the variable map of the directive that consumes it.

If Ferron cannot resolve a variable, the placeholder stays as `{{NAME}}` in the output.

For example:

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

| Field   | Type                                                              | Description                                      |
| ------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| `port`  | `u16 \| null`                                                     | Port number (the protocol may provide a default) |
| `hosts` | `Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>` | Host configurations with filters                 |

### Host filters

`ServerConfigurationHostFilters` controls which host/IP a port configuration applies to:

| Field  | Type             | Description                                   |
| ------ | ---------------- | --------------------------------------------- |
| `ip`   | `IpAddr \| null` | IP address to match (for multi-homed servers) |
| `host` | `String \| null` | Host/domain name to match (for SNI)           |

When both are `null`, the configuration applies to every host on that port, as in `*:8080`.

### Match expressions

Named matchers contain expressions for conditional configuration:

| Field   | Type                                  | Description                     |
| ------- | ------------------------------------- | ------------------------------- |
| `exprs` | `Vec<ServerConfigurationMatcherExpr>` | List of expressions to evaluate |
| `span`  | `ServerConfigurationSpan \| null`     | Source location                 |

Each expression has three components:

| Field   | Type                                 | Description         |
| ------- | ------------------------------------ | ------------------- |
| `left`  | `ServerConfigurationMatcherOperand`  | Left operand        |
| `right` | `ServerConfigurationMatcherOperand`  | Right operand       |
| `op`    | `ServerConfigurationMatcherOperator` | Comparison operator |

Operands can be:

| Variant      | JSON structure         | Example                            |
| ------------ | ---------------------- | ---------------------------------- |
| `Identifier` | `["Identifier", name]` | `["Identifier", "request.method"]` |
| `String`     | `["String", value]`    | `["String", "GET"]`                |
| `Integer`    | `["Integer", value]`   | `["Integer", 8080]`                |
| `Float`      | `["Float", value]`     | `["Float", 3.14]`                  |

Supported operators:

| Operator | JSON value     | Meaning           |
| -------- | -------------- | ----------------- |
| `==`     | `["Eq"]`       | String equality   |
| `!=`     | `["NotEq"]`    | String inequality |
| `~`      | `["Regex"]`    | Regex match       |
| `!~`     | `["NotRegex"]` | Regex non-match   |
| `in`     | `["In"]`       | Membership check  |

### Span metadata

`ServerConfigurationSpan` tracks source locations for error reporting:

| Field    | Type             | Description               |
| -------- | ---------------- | ------------------------- |
| `line`   | `usize`          | Line number (1-indexed)   |
| `column` | `usize`          | Column number (1-indexed) |
| `file`   | `String \| null` | Source file path          |

Ferron keeps spans in JSON output so validation and runtime errors point to the source location.

## Configuration adapters

Ferron loads configuration through an adapter. An adapter handles one or more file extensions and returns a parsed configuration.

### Built-in adapters

| Adapter             | File extensions | Description                                 |
| ------------------- | --------------- | ------------------------------------------- |
| `config-ferronconf` | `.conf`         | Parses Ferron's custom configuration syntax |
| `config-json`       | `.json`         | Loads JSON configuration directly           |

### Selecting an adapter

Ferron detects the adapter from the file extension. To override the detection:

```bash
ferron run -c config.json --config-adapter json
ferron validate -c myconfig.conf --config-adapter ferronconf
```

### Adapter interface

Adapters implement the `ConfigurationAdapter` trait, which defines:

- `adapt(params)`: loads and parses configuration, and returns a `ServerConfiguration` with a `ConfigurationWatcher`.
- `file_extension()`: lists the file extensions this adapter handles.

The `ConfigurationWatcher` watches the configuration source and triggers hot reload when it changes.

A JSON configuration with no span information looks like this:

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

Tools generate JSON configuration in these situations:

- Programmatic generation: a tool writes configuration without parsing the `.conf` syntax.
- API integration: an external system pushes configuration as JSON.
- Debugging: you inspect how Ferron parsed a `.conf` file.
- Testing: a test controls the configuration structure exactly.

### Loading JSON configurations

```bash
ferron run -c config.json
ferron validate -c config.json
```

Ferron detects the adapter from the `.json` extension.

### Hot reload

JSON configuration files support hot reload. When the file changes, `ConfigurationWatcher` detects the change and Ferron reloads the configuration gracefully.

To enable hot reload, pass the `watch` adapter parameter:

```bash
ferron run --config-params 'watch=1;file=ferron.json' --config-adapter json
```

### Configuration drift hints

Hot reload is off by default. Ferron can still detect a JSON file that changed on disk without a reload. Drift hints are on by default. To disable them:

```bash
ferron run --config-params 'drift_hints=false;file=ferron.json' --config-adapter json
```

See [Configuration drift hints](/docs/configuration/fundamentals/syntax#configuration-drift-hints) for details.

## See also

- [Configuration validation](/docs/configuration/fundamentals/validation)
- [Syntax and file structure](/docs/configuration/fundamentals/syntax)
- [Configuration formatting](/docs/configuration/fundamentals/formatting): `ferron-fmt` for formatting `.conf` files
- [Core directives](/docs/configuration/server/core-directives)
