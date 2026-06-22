---
title: "Configuration formatting"
description: "Using ferron-fmt to format, check, and style Ferron configuration files."
---

This page documents the `ferron-fmt` tool, a formatter for Ferron configuration files. It formats `.conf` files with configurable indentation, quote style, and other styling options.

## What it does

`ferron-fmt` parses a Ferron configuration file and rewrites it with consistent formatting. It handles:

- **Indentation** — configurable width and style (spaces or tabs)
- **Quote style** — auto (bare when possible, quoted when necessary), always double-quoted, or always bare
- **Blank lines** — normalization with a configurable maximum consecutive blank lines
- **Trailing newline** — optional trailing newline
- **Directive sorting** — optional alphabetical sorting of directives within blocks
- **Comment preservation** — inline and trailing comments are preserved
- **Quoting normalization** — bare strings when possible, double-quoted when necessary (e.g., for values that would be ambiguous)

This tool can be used to ensure consistent formatting across multiple Ferron configuration files.

> [!tip]
> The formatter is idempotent — running it multiple times on the same file produces identical output.

## Installation

`ferron-fmt` is included in all Ferron distributions alongside the main server binary. See [Installation](/docs/v3/installation) for details.

## Usage

### Format a file to stdout

```bash
ferron-fmt ferron.conf
```

### Format a file in place

```bash
ferron-fmt -i ferron.conf
```

### Write to a specific file

```bash
ferron-fmt -o formatted.conf ferron.conf
```

### Read from stdin

```bash
cat ferron.conf | ferron-fmt
```

### Check if a file is already formatted

```bash
ferron-fmt --check ferron.conf
```

Exits with code 0 if the file is already formatted, 1 if not. Useful for CI:

```bash
ferron-fmt --check ferron.conf && echo "Formatted" || echo "Needs formatting"
```

## Options

| Option | Default | Description |
|--------|---------|-------------|
| `--indent-width <n>` | `4` | Number of spaces per indentation level |
| `--indent-style <style>` | `spaces` | Indentation style: `spaces` or `tabs` |
| `--quote-style <style>` | `auto` | Quote style: `auto`, `always-double`, or `always-bare` |
| `--no-normalize-quotes` | — | Preserve original quoting style instead of normalizing |
| `--max-blank-lines <n>` | `2` | Maximum number of consecutive blank lines to preserve |
| `--no-trailing-newline` | — | Don't add a trailing newline at the end of the file |
| `--sort-directives` | — | Sort directives alphabetically within blocks |
| `--check` | — | Check if input is already formatted (exit 1 if not) |
| `-i, --in-place` | — | Edit file in place |
| `-o, --output <file>` | — | Write output to file instead of stdout |

## Indentation

### Spaces (default)

```bash
ferron-fmt --indent-style spaces --indent-width 4 ferron.conf
```

```ferron
example.com {
    root /var/www/html
    tls {
        provider manual
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
    }
}
```

### Tabs

```bash
ferron-fmt --indent-style tabs ferron.conf
```

```ferron
example.com {
	root /var/www/html
	tls {
		provider manual
		cert /etc/ssl/cert.pem
		key /etc/ssl/key.pem
	}
}
```

## Quote style

### Auto (default)

Bare strings when possible, double-quoted when necessary:

```bash
ferron-fmt --quote-style auto ferron.conf
```

```ferron
example.com {
    root /var/www/html
    tls {
        provider manual
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
    }
}
```

Values that require quoting (would be ambiguous):

```ferron
example.com {
    root "/var/www/html"
    tls {
        provider "manual"
        cert "/etc/ssl/cert.pem"
    }
}
```

### Always double-quoted

```bash
ferron-fmt --quote-style always-double ferron.conf
```

```ferron
example.com {
    root "/var/www/html"
    tls {
        provider "manual"
        cert "/etc/ssl/cert.pem"
        key "/etc/ssl/key.pem"
    }
}
```

### Always bare

```bash
ferron-fmt --quote-style always-bare ferron.conf
```

```ferron
example.com {
    root /var/www/html
    tls {
        provider manual
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
    }
}
```

> [!note]
> `always-bare` will produce a parse error if any value cannot be represented as a bare string (e.g., values containing spaces or special characters).

## Preserving original quoting

By default, `ferron-fmt` normalizes quoting according to the configured quote style. Use `--no-normalize-quotes` to preserve the original quoting:

```bash
ferron-fmt --no-normalize-quotes ferron.conf
```

This is useful when you want consistent indentation but don't want to change the quoting style.

## Trailing newline

By default, `ferron-fmt` adds a trailing newline at the end of the file:

```bash
ferron-fmt --no-trailing-newline ferron.conf
```

## Sorting directives

By default, directives are output in the order they appear in the source. Use `--sort-directives` to sort them alphabetically:

```bash
ferron-fmt --sort-directives ferron.conf
```

```ferron
example.com {
    root /var/www/html

    tls {
        provider manual
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
    }

    # Directives are sorted alphabetically:
    # compress > directory_listing > root > tls
}
```

## Blank lines

`ferron-fmt` preserves blank lines between directives up to the configured maximum (default: 2):

```bash
ferron-fmt --max-blank-lines 1 ferron.conf
```

This collapses consecutive blank lines beyond the limit.

## See also

- [Configuration validation](/docs/v3/configuration/fundamentals/validation)
- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
- [Configuration doctor](/docs/v3/configuration/fundamentals/doctor)
