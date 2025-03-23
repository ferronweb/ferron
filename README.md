<p align="center">
  <a href="https://www.ferronweb.org" target="_blank">
    <img src="logo.png" width="256">
  </a>
</p>
<p align="center">
  <b>Ferron</b> - a fast, memory-safe web server written in Rust
</p>
<p align="center">
  <a href="https://www.ferronweb.org/docs" target="_blank"><img alt="Static Badge" src="https://img.shields.io/badge/Documentation-orange"></a>
  <a href="https://www.ferronweb.org" target="_blank"><img alt="Website" src="https://img.shields.io/website?url=https%3A%2F%2Fwww.ferronweb.org"></a>
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

- **`ferron`**: The main web server.
- **`ferron-passwd`**: A tool for generating user entries with hashed passwords, which can be copied into the web server's configuration file.

## Installation

Since Ferron is still a work in progress, installation instructions will be provided once an initial release is available. Stay tuned!

## Building Ferron

You can clone the repository and explore the existing code:

```sh
git clone https://github.com/ferronweb/ferron.git
cd ferron
```

You can then build and run the web server using Cargo:

```sh
cargo build -r
cargo run -r --bin ferron
```

Optional modules can be chosen to be compiled in via Cargo "features".

## Server configuration

You can check the [Ferron documentation](https://www.ferronweb.org/docs/configuration) to see configuration properties used by Ferron.

## Contributing

See [Ferron contribution page](https://www.ferronweb.org/contribute) for details.

## License

Ferron is licensed under the MIT License. See `LICENSE` for details.