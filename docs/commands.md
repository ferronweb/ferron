---
title: Commands
---

Ferron comes with several additional command-line tools.

## Usage of commands

### `ferron`

```
A fast, memory-safe web server written in Rust

Usage: ferron [OPTIONS]

Options:
  -c, --config <config>
          The path to the server configuration file [default: ./ferron.kdl]
      --config-adapter <config-adapter>
          The configuration adapter to use [possible values: kdl, yaml-legacy]
      --module-config
          Prints the used compile-time module configuration (`ferron-build.yaml` or `ferron-build-override.yaml` in the Ferron source) and exits
  -V, --version
          Print version and build information
  -h, --help
          Print help
```

### `ferron-passwd`

```
A password tool for Ferron

Usage: ferron-passwd

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `ferron-precompress`

```
A utility that precompresses static files for Ferron

Usage: ferron-precompress <assets>...

Arguments:
  <assets>...  The path to the static assets (it can be a directory or a file)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `ferron-yaml2kdl`

```
A utility that attempts to convert Ferron 1.x YAML configuration to Ferron 2.x KDL configuration

Usage: ferron-yaml2kdl <input> <output>

Arguments:
  <input>   The name of an input file, containing Ferron 1.x YAML configuration
  <output>  The name of an output file, containing Ferron 2.x KDL configuration

Options:
  -h, --help     Print help
  -V, --version  Print version
```
