---
title: "Configuration formatting"
description: "Using ferron-fmt to format, check, and style Ferron configuration files."
---

`ferron-fmt` formats Ferron configuration files. Indentation, quote style, blank lines, and directive order are configurable.

> [!tip]
> The idiomatic Ferron 3 style uses bare strings without quotes when possible, 4-space indentation, and bare boolean flags. `ferron-fmt` defaults to this style and also preserves raw string literals for regex patterns.

## What it does

`ferron-fmt` parses a configuration file and rewrites it with consistent formatting. It handles:

- Indentation: configurable width and style (spaces or tabs).
- Quote style: bare when possible, always double-quoted, or always bare.
- Blank lines: normalization with a configurable maximum.
- Trailing newline: optional.
- Directive sorting: optional alphabetical order within blocks.
- Comments: inline and trailing comments are preserved.
- Quoting: bare strings when possible, double-quoted when necessary.
- Raw strings: `r"..."` syntax is preserved for strings that were originally raw.
- Line continuations: `\` at end of line stays at the same position.

Use the formatter to keep style consistent across configuration files.

> [!tip]
> The formatter is idempotent. Running it again on its own output produces identical output.

## Installation

All Ferron distributions include `ferron-fmt` alongside the main server binary. See [Getting started](/docs/getting-started) for installation options.

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

Exits with code 0 if the file is already formatted, and 1 if not. This suits CI:

```bash
ferron-fmt --check ferron.conf && echo "Formatted" || echo "Needs formatting"
```

## Options

| Option                   | Default  | Description                                            |
| ------------------------ | -------- | ------------------------------------------------------ |
| `--indent-width <n>`     | `4`      | Number of spaces per indentation level                 |
| `--indent-style <style>` | `spaces` | Indentation style: `spaces` or `tabs`                  |
| `--quote-style <style>`  | `auto`   | Quote style: `auto`, `always-double`, or `always-bare` |
| `--no-normalize-quotes`  | none     | Preserve original quoting style instead of normalizing |
| `--max-blank-lines <n>`  | `2`      | Maximum number of consecutive blank lines to preserve  |
| `--no-trailing-newline`  | none     | Do not add a trailing newline at the end of the file   |
| `--sort-directives`      | none     | Sort directives alphabetically within blocks           |
| `--check`                | none     | Check if input is already formatted (exit 1 if not)    |
| `-i, --in-place`         | none     | Edit file in place                                     |
| `-o, --output <file>`    | none     | Write output to file instead of stdout                 |

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

The output indents each level with one tab character instead of spaces.

## Quote style

### Auto (default)

Bare strings where possible, double-quoted where necessary. This produces idiomatic Ferron 3 style:

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

`auto` keeps double quotes when a value cannot be a bare string. A value with spaces is one example:

```ferron
example.com {
    header X-Example "value with spaces"
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
> `always-bare` fails when a value cannot be written as a bare string, for example a value with spaces or special characters.

## Preserving original quoting

By default, `ferron-fmt` normalizes quoting to the configured quote style. `--no-normalize-quotes` preserves the original quoting:

```bash
ferron-fmt --no-normalize-quotes ferron.conf
```

Use it when you want consistent indentation but no change to quoting.

## Trailing newline

`ferron-fmt` adds a trailing newline by default. To omit it:

```bash
ferron-fmt --no-trailing-newline ferron.conf
```

## Sorting directives

By default, directives keep their order from the source. `--sort-directives` sorts them alphabetically:

```bash
ferron-fmt --sort-directives ferron.conf
```

```ferron
example.com {
    compressed false
    directory_listing false
    root /var/www/html

    tls {
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
        provider manual
    }
}
```

## Blank lines

`ferron-fmt` keeps blank lines between directives up to the configured maximum (default: 2):

```bash
ferron-fmt --max-blank-lines 1 ferron.conf
```

Runs of blank lines longer than the limit collapse to the limit.

## Raw strings

The formatter keeps raw string syntax (`r"..."`) for strings that were raw in the input. A raw string is written without escape processing, which suits regex patterns:

```ferron
match api_request {
    request.uri.path ~ r"^/api/v1(?:/|$)"
}
```

The formatter leaves the `r"..."` syntax intact. It does not convert quoted strings (`"..."`) to raw strings.

> [!note]
> Raw strings do not support interpolation. The formatter does not convert interpolated strings to raw strings.

## Line continuations

The formatter keeps the line continuations (`\` at end of line) that were in the input. This helps with long directives:

```ferron
example.com {
    index index.html \
      index.htm
}
```

The continuation stays at the same position. A continuation with a trailing comment is preserved too:

```ferron
example.com {
    index index.html \ # first choice
      index.htm
}
```

> [!note]
> Line continuations are only preserved when the formatter reads from a file or stdin. When formatting a pre-parsed AST directly (programmatic API), continuations are not available and the output uses single-line values.

## See also

- [Configuration validation](/docs/configuration/fundamentals/validation)
- [Syntax and file structure](/docs/configuration/fundamentals/syntax)
- [Configuration doctor](/docs/configuration/fundamentals/doctor)
