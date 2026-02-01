---
title: Commands
---

Ferron comes with several additional command-line tools.

## Usage of commands

### `ferron`

```plain
A fast, memory-safe web server written in Rust

Usage: ferron [OPTIONS] [COMMAND]

Commands:
  serve  Utility command to start up a basic HTTP server
  help   Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>
          The path to the server configuration file [default: ./ferron.kdl]
      --config-string <CONFIG_STRING>
          The path to the server configuration file
      --config-adapter <CONFIG_ADAPTER>
          The configuration adapter to use [possible values: kdl, yaml-legacy]
      --module-config
          Prints the used compile-time module configuration (`ferron-build.yaml` or `ferron-build-override.yaml` in the Ferron source) and exits
  -V, --version
          Print version and build information
  -h, --help
          Print help
```

### `ferron serve`

```plain
Utility command to start up a basic HTTP server

Usage: ferron serve [OPTIONS]

Options:
  -l, --listen-ip <LISTEN_IP>  The listening IP to use [default: 127.0.0.1]
  -p, --port <PORT>            The port to use [default: 3000]
  -r, --root <ROOT>            The root directory to serve [default: .]
      --log <LOG>              Where to output logs [default: stdout] [possible values: stdout, stderr, off]
      --error-log <ERROR_LOG>  Where to output error logs [default: stderr] [possible values: stdout, stderr, off]
  -h, --help                   Print help
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

Usage: ferron-precompress [OPTIONS] <assets>...

Arguments:
  <assets>...  The path to the static assets (it can be a directory or a file)

Options:
  -t, --threads <threads>  The number of threads to use for compression [default: 64]
  -h, --help               Print help
  -V, --version            Print version
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
