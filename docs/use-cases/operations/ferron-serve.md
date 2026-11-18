---
title: "'ferron-serve' command"
description: "Using the `ferron-serve` command to serve files on the local filesystem without needing to write a configuration file."
---

`ferron-serve` serves files from the local filesystem without a configuration file. It works like the [serve NPM package](https://www.npmjs.com/package/serve). Run `ferron-serve --help` for all options.

By default, it listens on `127.0.0.1:3000` and serves the current directory (`.`).

> [!tip]
> For production deployments, use a Ferron configuration file. A file gives you repeatable settings and full control. See [Configuration syntax](/docs/configuration/fundamentals/syntax).

## Quick start

Serve the current directory on the default address:

```bash
ferron-serve
```

Serve a specific directory:

```bash
ferron-serve --root /var/www/html
```

Bind to all interfaces on a custom port:

```bash
ferron-serve --listen-ip 0.0.0.0 --port 8080
```

## Common options

- `-l, --listen-ip <LISTEN_IP>`: listening IP address (default: `127.0.0.1`).
- `-p, --port <PORT>`: listening port (default: `3000`).
- `-r, --root <ROOT>`: filesystem directory to serve (default: `.`).
- `--disable-console-log`: disable console logs. Console logs are on by default.

## Basic authentication

Protect the served files with HTTP Basic Authentication. Pass one or more `-c, --credential` options in `username:hashed_password` format:

```text
username:hashed_password
```

Generate the hash with `ferron-passwd` or another tool that produces `password-auth` compatible hashes.

Example with two users:

```bash
ferron-serve \
  --root /srv/private-files \
  --credential "alice:$ALICE_HASH" \
  --credential "bob:$BOB_HASH"
```

If needed, disable brute-force protection with `--disable-brute-protection`.

## Forward proxy mode

`ferron-serve` can also run as a forward proxy:

```bash
ferron-serve --forward-proxy --listen-ip 127.0.0.1 --port 3128
```

When you use `--forward-proxy`, treat it as a network proxy setup, not static file hosting.

## Additional options

### Directory listings

Enable directory listings for directories without an index file:

```bash
ferron-serve --directory-listing
```

### Index files

Customize the index filenames to try when a request path resolves to a directory:

```bash
ferron-serve --index index.html index.htm
```

### Compression

Enable or disable on-the-fly response body compression:

```bash
ferron-serve --compress
```

Compression is on by default.

## How it works

`ferron-serve` does the following:

1. Builds a temporary Ferron configuration from the command-line options.
2. Writes the configuration to a temporary file.
3. Starts Ferron with that configuration.
4. Exits when the Ferron process exits.
