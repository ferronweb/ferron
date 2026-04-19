---
title: "Configuration: CGI support"
description: "Server-side CGI script execution with per-extension interpreters, environment variables, and shebang-line detection."
---

This page documents the `cgi` directive for configuring Ferron's CGI (Common Gateway Interface) support. CGI enables dynamic content by spawning external interpreters for scripts matched by file extension or placed under a `cgi-bin` directory.

## `cgi`

```ferron
example.com {
    cgi {
        extension ".php"
        interpreter ".php" php-cgi -c /etc/php/cgi.ini
        environment "APP_ENV" "production"
    }
}
```

The `cgi` block can be written as a boolean flag to enable CGI with all defaults, or as a block with nested directives to customize behavior.

| Form | Description |
| --- | --- |
| `cgi` | Enables CGI with all defaults. |
| `cgi true` | Explicitly enables CGI. |
| `cgi false` | Disables CGI for the current scope. |
| `cgi { ... }` | Enables CGI and configures nested directives. |

### `extension`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `extension` | `<string>` | This directive registers an additional file extension that should be treated as a CGI script. Unlike `cgi-bin` directory matching, the file does not need to be executable. This directive can be specified multiple times. | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        extension ".php"
        extension ".rb"
    }
}
```

**Notes:**

- Extensions are matched case-insensitively.
- Files with these extensions are treated as CGI scripts regardless of their location in the file tree.
- This is complementary to `cgi-bin` directory matching — files inside `cgi-bin` are always treated as CGI scripts.

### `interpreter`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `interpreter` | `<extension: string> <arg: string>...` | This directive maps a file extension to a custom interpreter command. The first argument is the extension (with a leading dot, e.g. `.php`). Subsequent arguments form the interpreter command line. Pass `false` as the second argument to disable the interpreter for that extension. This directive can be specified multiple times. | built-in defaults |

**Configuration example:**

```ferron
example.com {
    cgi {
        interpreter ".php" php-cgi -c /etc/php/cgi.ini
        interpreter ".pl" perl
        interpreter ".py" python3
        interpreter ".php" false
    }
}
```

**Notes:**

- The file path is automatically appended as the final argument.
- When `false` is used as the second argument, the interpreter list is cleared, meaning the file must be directly executable (e.g., via a shebang line or native executable).
- For Unix systems, files without a matching interpreter must have the executable permission bit set.
- On Unix systems, scripts with a shebang line (e.g., `#!/usr/bin/env python3`) are parsed and the interpreter is derived from the shebang.
- On Windows, `.exe` files are executed directly, `.bat`/`.cmd` files use `cmd /c`, and scripts with shebangs are parsed similarly to Unix.

### `environment`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `environment` | `<name: string> <value: string>` | This directive sets a CGI environment variable passed to the interpreter process. Values are resolved with the same interpolation syntax as other directives. This directive can be specified multiple times. | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        environment "APP_ENV" "production"
        environment "APP_SECRET" "{env:APP_SECRET}"
        environment "RUBY_VERSION" "3.3"
    }
}
```

**Notes:**

- Environment variables take precedence over any existing variables with the same name.
- The `Proxy` header is automatically removed from the request to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Ferron always sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically.

## Default interpreters

The following built-in interpreters are available when no custom `interpreter` directive matches:

| Extension | Default interpreter |
| --- | --- |
| `.pl` | `perl` |
| `.py` | `python` |
| `.sh` | `bash` |
| `.ksh` | `ksh` |
| `.csh` | `csh` |
| `.rb` | `ruby` |
| `.php` | `php-cgi` |
| `.exe` (Windows) | *(direct execution)* |
| `.bat` (Windows) | `cmd /c` |
| `.vbs` (Windows) | `cscript` |

## CGI script locations

A request is handled as a CGI script when:

1. The resolved path is inside a `cgi-bin` directory (case-insensitive match on the first path component after the document root), **or**
2. The file extension matches one registered via the `extension` directive.

When a matching file is found, Ferron checks for an interpreter using the following priority:

1. Custom `interpreter` directive matching the file extension.
2. Built-in default interpreter for the extension.
3. If the file is directly executable (has the executable bit on Unix, or is a native `.exe` on Windows), it is run directly.
4. If the file has a shebang line, the interpreter is parsed from the shebang.

## Environment variables

Ferron automatically sets the following CGI environment variables:

| Variable | Description |
| --- | --- |
| `SERVER_SOFTWARE` | Always `Ferron`. |
| `SERVER_NAME` | Server hostname. |
| `SERVER_ADDR` | Local server address. |
| `SERVER_PORT` | Server port. |
| `REQUEST_METHOD` | HTTP method. |
| `REQUEST_URI` | Original request URI. |
| `QUERY_STRING` | Query string (empty string if none). |
| `PATH_INFO` | Path info extracted from the request. |
| `SCRIPT_NAME` | The script path relative to the document root. |
| `AUTH_TYPE` | Authentication type from the `Authorization` header (e.g., `Basic`, `Bearer`). |
| `REMOTE_USER` | Authenticated username, if available. |
| `SERVER_ADMIN` | Server administrator email (from `admin_email` configuration). |
| `HTTPS` | Set to `on` when the connection is encrypted. |

Additional variables set by `environment` directives override any automatically set variables with the same name.

## Observability

### Logs

- **`WARN`**: logged when a CGI script exits with a non-zero status and produces output on stderr. The message includes the trimmed stderr content.

## Examples

### PHP with a custom PHP-CGI binary

```ferron
example.com {
    root /srv/www/example
    cgi {
        extension ".php"
        interpreter ".php" php-cgi -c /etc/php/8.2/cgi/php.ini
    }
}
```

### Multiple interpreters with environment variables

```ferron
example.com {
    root /srv/www/app
    cgi {
        extension ".rb"
        interpreter ".rb" ruby
        interpreter ".py" python3
        environment "RUBY_VERSION" "3.3"
        environment "PYTHONUNBUFFERED" "1"
    }
}
```

### Disabling the default PHP interpreter

```ferron
example.com {
    root /srv/www/example
    cgi {
        interpreter ".php" false
    }
}
```

This allows PHP files to be handled via shebang lines or direct execution instead.

### Using `cgi-bin` with additional extensions

```ferron
example.com {
    root /srv/www/example

    cgi {
        extension ".php"
        environment "APP_ENV" "production"
    }

    # /srv/www/example/cgi-bin/handler.py is treated as CGI
    # /srv/www/example/scripts/script.php is also treated as CGI
    # (because of the ".php" extension directive)
}
```

## Notes and troubleshooting

- CGI scripts must be inside a `cgi-bin` directory or have an extension registered via the `extension` directive.
- On Unix, scripts without a matching `interpreter` directive must have the executable permission bit set (`chmod +x`).
- On Windows, shebang lines are supported for `.bat`, `.cmd`, and other script files. Native `.exe` files are executed directly.
- The `Proxy` header is always removed to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- If a CGI script exits with a non-zero status, Ferron logs a `WARN` message and returns a `500 Internal Server Error` response.
- For CGI stderr output, Ferron logs warnings when the script produces output on stderr. The output is trimmed before logging.
- The working directory for a CGI script is set to the directory containing the script file.
- For authentication integration, CGI scripts receive `REMOTE_USER` and `AUTH_TYPE` only when used alongside a module like `http-basicauth` that sets `ctx.auth_user`.
- For static file serving alongside CGI, see [Static file serving](/docs/v3/configuration/static-content).
- For URL rewriting, see [URL rewriting](/docs/v3/configuration/http-rewrite).
- For response headers and CORS, see [HTTP headers and CORS](/docs/v3/configuration/http-headers).
