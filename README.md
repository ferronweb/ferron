<p align="center">
  <a href="https://v2.ferronweb.org" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="wwwroot/img/logo-dark.png">
      <img alt="Ferron logo" src="wwwroot/img/logo.png" width="256">
    </picture>
  </a>
</p>
<p align="center">
  <b>Ferron</b> - a fast, memory-safe web server written in Rust
</p>
<p align="center">
  <a href="https://v2.ferronweb.org/docs" target="_blank"><img alt="Static Badge" src="https://img.shields.io/badge/Documentation-orange"></a>
  <a href="https://v2.ferronweb.org" target="_blank"><img alt="Website" src="https://img.shields.io/website?url=https%3A%2F%2Fv2.ferronweb.org"></a>
  <a href="https://matrix.to/#/#ferronweb:matrix.org" target="_blank"><img alt="Chat" src="https://img.shields.io/matrix/ferronweb%3Amatrix.org"></a>
  <a href="https://x.com/ferron_web" target="_blank"><img alt="X (formerly Twitter) Follow" src="https://img.shields.io/twitter/follow/ferron_web"></a>
  <a href="https://hub.docker.com/r/ferronserver/ferron" target="_blank"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/ferronserver/ferron"></a>
  <a href="https://github.com/ferronweb/ferron" target="_blank"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/ferronweb/ferron"></a>
</p>

* * *

## Features

- **High performance** - built with Rust’s async capabilities for optimal speed.
- **Memory-safe** - built with Rust, which is a programming language offering memory safety.
- **Extensibility** - modular architecture for easy customization.
- **Secure** - focus on robust security practices and safe concurrency.

## Components

Ferron consists of multiple components:

- **`ferron`** - the main web server.
- **`ferron-passwd`** - a tool for generating hashed passwords, which can be copied into the web server's configuration file.
- **`ferron-yaml2kdl`** - a tool for attempting to convert the Ferron 1.x YAML configuration to Ferron 2.x KDL configuration.

Ferron also consists of:

- **`build-prepare`** - internal tool for preparation when building Ferron with modules.
- **`ferron-common`** - code common for Ferron and its modules.
- **`ferron-load-modules`** - functions for loading Ferron modules.
- **`ferron-modules-builtin`** - built-in Ferron modules.
- **`ferron-yaml2kdl-core`** - the core library behind the `ferron-yaml2kdl` tool.

## Building Ferron from source

You can clone the repository and explore the existing code:

```sh
git clone https://github.com/ferronweb/ferron.git
cd ferron
```

You can then build and run the web server using Cargo:

```sh
cargo run --manifest-path build-prepare/Cargo.toml
cd build-workspace
cargo build -r --target-dir ../target
cargo run -r --bin ferron
```

You can also, for convenience, use `make`:

```sh
make build # Build the web server
make build-dev # Build the web server, for development and debugging
make run # Run the web server
make run-dev # Run the web server, for development and debugging
make package # Package the web server to a ZIP archive (run it after building it)
```

You can also create a ZIP archive that can be used by the Ferron installer:

```sh
make build-with-package
```

The ZIP archive will be located in the `dist` directory.

You can also cross-compile the web server for a different target:

```sh
# Replace "i686-unknown-linux-gnu" with the target (as defined by the Rust target triple) you want to build for
make build TARGET="i686-unknown-linux-gnu" CARGO_FINAL="cross"
```

It's also possible to use only Cargo to build the web server, although you wouldn't be able to use external modules:
```sh
cargo build -r
cargo run -r --bin ferron
```

For compilation notes, see the [compilation notes page](./COMPILATION.md).

~~You can also use [Ferron Forge](https://github.com/ferronweb/ferron-forge) to build the web server. Ferron Forge outputs a ZIP archive that can be used by the Ferron installer.~~

## Modules

If you would like to develop Ferron modules, you can find the [Ferron module development notes](./MODULES.md).

## Server configuration

You can check the [Ferron documentation](https://v2.ferronweb.org/docs/configuration-kdl) to see configuration properties used by Ferron.

## Contributing

See [Ferron contribution page](https://v2.ferronweb.org/contribute) for details.

## License

Ferron is licensed under the MIT License. See `LICENSE` for details.
