<p align="center">
  <a href="https://ferron.sh" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="wwwroot/assets/logo-dark.png">
      <img alt="Ferron logo" src="wwwroot/assets/logo.png" width="256">
    </picture>
  </a>
</p>
<p align="center">
  <b>Ferron</b> — a fast, modern web server built for production debugging.
</p>

* * *

<p align="center">
  <a href="https://ferron.sh/docs/v3" target="_blank"><img alt="Static Badge" src="https://img.shields.io/badge/Documentation-orange?style=for-the-badge"></a>
  <a href="https://ferron.sh" target="_blank"><img alt="Website" src="https://img.shields.io/website?url=https%3A%2F%2Fferron.sh&style=for-the-badge"></a>
  <a href="https://matrix.to/#/#ferronweb:matrix.org" target="_blank"><img alt="Chat" src="https://img.shields.io/matrix/ferronweb%3Amatrix.org?style=for-the-badge"></a>
  <a href="https://x.com/ferron_web" target="_blank"><img alt="X (formerly Twitter) Follow" src="https://img.shields.io/twitter/follow/ferron_web?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/ferronserver/ferron" target="_blank"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/ferronserver/ferron?style=for-the-badge"></a>
  <a href="https://github.com/ferronweb/ferron" target="_blank"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/ferronweb/ferron?style=for-the-badge"></a>
</p>

> [!note]
> **Status: Beta** — This release can be considered feature-complete, but is not yet recommended for production deployments. If you experience any issues when testing it, [opening an issue](https://github.com/ferronweb/ferron/issues/new/choose) is welcome.

## Why Ferron 3?

Built to set up quickly, behave predictably, and hold up reliably in production.

- **Readable configuration** — set up websites and reverse proxies with a clear, compact config: no sprawl, no hidden surprises.
- **Automatic TLS** — certificates are issued and renewed automatically. You get clear signals when it works (or doesn't).
- **First-class observability** — see exactly what happened with any request. Traces cover every layer and link directly to the relevant logs.
- **Predictable performance** — fast and consistent under load, right out of the box. No runtime tuning required.
- **Memory-safe** — entire categories of memory-related security holes simply don't exist in Ferron (it's built with [Rust](https://rust-lang.org/)).
- **Reliable in production** — handles messy real-world traffic, upstream failures, and protocol edge cases — predictably.

> [!tip]
> Ferron 3 is designed around two core principles: **ease of setup** (get a working config in minutes) and **ease of debugging** (when something goes wrong, find the root cause fast).

## Configuration examples

### Static file serving

```ferron
example.com {
    root "/var/www/html"

    # If uncommented, directory listing is enabled.
    #directory_listing
}
```

### Reverse proxy

```ferron
api.example.com {
    proxy http://localhost:8080
}
```

More examples are available in the [configuration documentation](https://ferron.sh/docs/v3/configuration/fundamentals/syntax).

## Installing Ferron 3 (pre-built)

The most convenient way to get started with Ferron 3 is to use the installer script for Linux:

```sh
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

See the full instructions in the [Linux installation documentation](https://ferron.sh/docs/v3/installation/linux/installer).

## Building from source

```sh
git clone https://github.com/ferronweb/ferron -b develop-3.x
cd ferron
cargo build --workspace
```

Run the server:

```sh
cargo run -p ferron -- run -c ferron.conf
cargo run -p ferron -- run -c ferron.conf --verbose  # with debug logging
```

Other CLI commands:

```sh
cargo run -p ferron -- validate -c ferron.conf   # validate without starting
cargo run -p ferron -- adapt -c ferron.conf      # output config as JSON
cargo run -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid  # Unix daemon
```

Run tests and checks:

```sh
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Package Ferron for distribution (requires `just`):

```sh
just package # Archive (.zip for Windows, .tar.gz for Unix)
just package-deb # Debian package
just package-rpm # RPM package
just package-windows # Windows installer
just installer # Linux installer
```

## Configuration

The full directive reference is in [docs/configuration/](https://ferron.sh/docs/v3/configuration/fundamentals/syntax).

## Contributing

Feedback, bug reports, and testing are welcome. When reporting issues, include your configuration file, `--verbose` output, and steps to reproduce.

## License

MIT. See `LICENSE` for details.
