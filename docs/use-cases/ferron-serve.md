---
title: "'ferron serve' subcommand"
description: "Using the `ferron serve` subcommand to serve files on the local filesystem without needing to write a configuration file."
---

If you simply need to serve files on the local filesystem, you can use the `ferron serve` subcommand to do so. If you are familiar with the [serve NPM package](https://www.npmjs.com/package/serve), the `ferron serve` command will offer similar functionality. See also the help message from running `ferron serve --help` for supported options.

By default, `ferron serve` listens on `127.0.0.1:3000` and serves files from the current directory (`.`).

## Quick start

Serve the current directory on the default address:

```bash
ferron serve
```

Serve a specific directory:

```bash
ferron serve --root /var/www/html
```

Bind to all interfaces on a custom port:

```bash
ferron serve --listen-ip 0.0.0.0 --port 8080
```

## Common options

- `-l, --listen-ip <LISTEN_IP>` - listening IP address (default: `127.0.0.1`).
- `-p, --port <PORT>` - listening port (default: `3000`).
- `-r, --root <ROOT>` - filesystem directory to serve (default: `.`).
- `--disable-console-log` - whether to disable console logs (default: console logs enabled).

## Basic authentication

You can protect the served files with HTTP Basic Authentication by passing one or more credentials with `-c, --credential`.

Each credential must be formatted as:

```text
username:hashed_password
```

The password hash should be generated with `ferron-passwd` or another tool that supports `password-auth` compatible hashes.

Example with two users:

```bash
ferron serve \
  --root /srv/private-files \
  --credential "alice:$ALICE_HASH" \
  --credential "bob:$BOB_HASH"
```

If needed, brute-force protection can be disabled with `--disable-brute-protection`.

## Forward proxy mode

`ferron serve` can also run as a forward proxy:

```bash
ferron serve --forward-proxy --listen-ip 127.0.0.1 --port 3128
```

When `--forward-proxy` is enabled, treat it as a network proxy setup rather than static file hosting.

## Additional options

### Directory listings

Enable directory listings for directories without an index file:

```bash
ferron serve --directory-listing
```

### Index files

Customize the index filenames to try when a request path resolves to a directory:

```bash
ferron serve --index index.html index.htm
```

### Compression

Enable or disable on-the-fly response body compression:

```bash
ferron serve --compress
```

Compression is enabled by default.

## How it works

When you run `ferron serve`, the utility:

1. Generates a temporary Ferron configuration file based on your command-line options.
2. Writes the configuration to a temporary file.
3. Starts Ferron with the generated configuration.
4. Exits when the Ferron process terminates.

This means the `ferron serve` command is essentially a convenient wrapper around Ferron's configuration system, allowing you to quickly serve files without writing a configuration file.

## Notes and troubleshooting

- If you need more control over your server configuration, consider writing a custom Ferron configuration file instead. See [Syntax and file structure](/docs/v3/configuration/syntax) for details.
- If you get permission errors when binding to a port below 1024, you may need to run with elevated privileges or use a different port.
- If the server doesn't start, check that the port is available and not blocked by a firewall.
- The `ferron serve` command generates temporary configuration files in your system's temporary directory.
- For production deployments, use a proper configuration file for better control and reproducibility.
- If you need persistent basic authentication, consider using [HTTP basic authentication](/docs/v3/configuration/http-basicauth) in a configuration file.
- For more on reverse proxying, see [Reverse proxying](/docs/v3/use-cases/reverse-proxy).
