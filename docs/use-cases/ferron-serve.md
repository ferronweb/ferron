---
title: "'ferron serve' subcommand"
description: "Using the `ferron serve` subcommand to serve files on the local filesystem without needing to write a configuration file."
---

If you simply need to serve files on the local filesystem, you can
use the `ferron serve` subcommand to do so. If you are familiar with
the [serve NPM package](https://www.npmjs.com/package/serve), the
`ferron serve` command will offer similar functionality. See also
the help message from running `ferron serve --help` for supported
options.

By default, `ferron serve` listens on `127.0.0.1:3000` and serves files
from the current directory (`.`).

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
- `--log <LOG>` - access log output (`stdout`, `stderr`, `off`; default: `stdout`).
- `--error-log <ERROR_LOG>` - error log output (`stdout`, `stderr`, `off`; default: `stderr`).

## Basic authentication

You can protect the served files with HTTP Basic Authentication by
passing one or more credentials with `-c, --credential`.

Each credential must be formatted as:

```text
username:hashed_password
```

The password hash should be generated with `ferron-passwd` or another
tool that supports `password-auth` compatible hashes.

Example with two users:

```bash
ferron serve \
  --root /srv/private-files \
  --credential "alice:$ALICE_HASH" \
  --credential "bob:$BOB_HASH"
```

If needed, brute-force protection can be disabled with
`--disable-brute-protection`.

## Forward proxy mode

`ferron serve` can also run as a forward proxy:

```bash
ferron serve --forward-proxy --listen-ip 127.0.0.1 --port 3128
```

When `--forward-proxy` is enabled, treat it as a network proxy setup
rather than static file hosting.
