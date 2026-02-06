<p align="center">
  <a href="https://ferron.sh" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="wwwroot/assets/logo-dark.png">
      <img alt="Ferron logo" src="wwwroot/assets/logo.png" width="256">
    </picture>
  </a>
</p>
<p align="center">
  <b>Ferron</b> - a fast, modern, and easily configurable web server with automatic TLS
</p>

* * *

<p align="center">
  <a href="https://ferron.sh/docs" target="_blank"><img alt="Static Badge" src="https://img.shields.io/badge/Documentation-orange?style=for-the-badge"></a>
  <a href="https://ferron.sh" target="_blank"><img alt="Website" src="https://img.shields.io/website?url=https%3A%2F%2Fferron.sh&style=for-the-badge"></a>
  <a href="https://matrix.to/#/#ferronweb:matrix.org" target="_blank"><img alt="Chat" src="https://img.shields.io/matrix/ferronweb%3Amatrix.org?style=for-the-badge"></a>
  <a href="https://x.com/ferron_web" target="_blank"><img alt="X (formerly Twitter) Follow" src="https://img.shields.io/twitter/follow/ferron_web?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/ferronserver/ferron" target="_blank"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/ferronserver/ferron?style=for-the-badge"></a>
  <a href="https://github.com/ferronweb/ferron" target="_blank"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/ferronweb/ferron?style=for-the-badge"></a>
</p>

## Why Ferron?

- **High performance** - thoroughly optimized for speed with support for high concurrency.
- **Memory-safe** - built with [Rust](https://rust-lang.org/), which is a programming language that can offer strong memory safety guarantees.
- **Automatic TLS** - automatic SSL/TLS certificate acquisition and renewal with Let's Encrypt integration.
- **Easy configuration** - simple, intuitive configuration with sensible, secure defaults and [comprehensive documentation](https://ferron.sh/docs).
- **Extensibility** - modular architecture for easy customization.
- **Powerful reverse proxy** - advanced reverse proxy capabilities with support for load balancing and health checks.

## Components

Ferron consists of multiple components:

- **`ferron`** - the main web server.
- **`ferron-passwd`** - a tool for generating hashed passwords, which can be copied into the web server's configuration file.
- **`ferron-precompress`** - a tool for precompressing static files for Ferron.
- **`ferron-yaml2kdl`** - a tool for attempting to convert the Ferron 1.x YAML configuration to Ferron 2.x KDL configuration.

Ferron also consists of:

- **`build-prepare`** - internal tool for preparation when building Ferron with modules.
- **`ferron-common`** - code common for Ferron and its modules.
- **`ferron-dns-builtin`** - built-in Ferron DNS providers.
- **`ferron-load-modules`** - functions for loading Ferron modules.
- **`ferron-modules-builtin`** - built-in Ferron modules.
- **`ferron-observability-builtin`** - built-in Ferron observability backend support.
- **`ferron-yaml2kdl-core`** - the core library behind the `ferron-yaml2kdl` tool.

## Installing Ferron from pre-built binaries

The easiest way to install Ferron is installing it from pre-built binaries.

Below are the different ways to install Ferron:

- [Installer (GNU/Linux)
](https://ferron.sh/docs/installation/installer-linux)
- [Installer (Windows Server)
](https://ferron.sh/docs/installation/installer-windows)
- [Package managers (Debian/Ubuntu)](https://ferron.sh/docs/installation/debian)
- [Package managers (RHEL/Fedora)](https://ferron.sh/docs/installation/rpm)
- [Docker](https://ferron.sh/docs/installation/docker)
- [Package managers (community)](https://ferron.sh/docs/installation/package-managers)
- [Manual installation](https://ferron.sh/docs/installation/manual)

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
cargo update # If you experience crate conflicts
cargo build -r --target-dir ../target
cd ..
cp ferron-test.kdl ferron.kdl
target/release/ferron
```

You can also, for convenience, use `make`:

```sh
make build # Build the web server
make build-dev # Build the web server, for development and debugging
make run # Run the web server
make run-dev # Run the web server, for development and debugging
make smoketest # Perform a smoke test
make smoketest-dev # Perform a smoke test, for development and debugging
make package # Package the web server to a ZIP archive (run it after building it)
make package-deb # Package the web server to a Debian package (run it after building it)
make package-rpm # Package the web server to an RPM package (run it after building it)
make installer # Build installers for Ferron 2
```

Or a `build.ps1` build script, if you're on Windows:
```batch
REM Build the web server
powershell -ExecutionPolicy Bypass .\build.ps1 Build

REM Build the web server, for development and debugging
powershell -ExecutionPolicy Bypass .\build.ps1 BuildDev

REM Run the web server
powershell -ExecutionPolicy Bypass .\build.ps1 Run

REM Run the web server, for development and debugging
powershell -ExecutionPolicy Bypass .\build.ps1 RunDev

REM Perform a smoke test
powershell -ExecutionPolicy Bypass .\build.ps1 Smoketest

REM Perform a smoke test, for development and debugging
powershell -ExecutionPolicy Bypass .\build.ps1 SmoketestDev

REM Package the web server to a ZIP archive (run it after building it)
powershell -ExecutionPolicy Bypass .\build.ps1 Package

REM Build installers for Ferron 2
powershell -ExecutionPolicy Bypass .\build.ps1 Installer
```

You can also create a ZIP archive that can be used by the Ferron installer:

```sh
make build-with-package
```

Or if you're on Windows:

```batch
powershell -ExecutionPolicy Bypass .\build.ps1 BuildWithPackage
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
./target/release/ferron
```

For compilation notes, see the [compilation notes page](./COMPILATION.md).

## Modules

If you would like to develop Ferron modules, you can find the [Ferron module development notes](./MODULES.md).

## Server configuration

You can check the [Ferron documentation](https://ferron.sh/docs/configuration-kdl) to see configuration properties used by Ferron.

## Contributing

See [Ferron contribution page](https://ferron.sh/contribute) for details.

Below is a list of contributors to Ferron. **Thank you to all of them!**

[![Contributor list](./CONTRIBUTORS.svg)](https://github.com/ferronweb/ferron/graphs/contributors)

## License

Ferron is licensed under the MIT License. See `LICENSE` for details.
