---
title: "Configuration: CGI support"
description: "Server-side CGI script execution with per-extension interpreters, environment variables, and shebang-line detection."
---

This page documents the `cgi` directive for configuring CGI (Common Gateway Interface) support in Ferron. CGI enables dynamic content by spawning external interpreters for scripts matched by file extension or placed under a `cgi-bin` directory.

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

You can write the `cgi` block as a boolean flag to enable CGI with all defaults. You can also write it as a block with nested directives to customize behavior.

| Form | Description |
| --- | --- |
| `cgi` | Enables CGI with all defaults. |
| `cgi true` | Explicitly enables CGI. |
| `cgi false` | Disables CGI for the current scope. |
| `cgi { ... }` | Enables CGI and configures nested directives. |

### `extension`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `extension` | `<string>` | This directive registers an additional file extension that Ferron treats as a CGI script. Unlike `cgi-bin` directory matching, the file does not need to be executable. You can specify this directive multiple times. | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        extension ".php"
        extension ".rb"
    }
}
```

> [!note]
>
> - Ferron matches extensions case-insensitively.
> - Ferron treats files with these extensions as CGI scripts wherever they appear in the file tree.
> - This complements `cgi-bin` directory matching. Ferron always treats files inside `cgi-bin` as CGI scripts.

### `interpreter`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `interpreter` | `<extension: string> <arg: string>...` | This directive maps a file extension to a custom interpreter command. The first argument is the extension (with a leading dot, for example `.php`). Later arguments form the interpreter command line. Pass `false` as the second argument to disable the interpreter for that extension. You can specify this directive multiple times. | built-in defaults |

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

> [!note]
>
> - The file path is automatically appended as the final argument.
> - When you pass `false` as the second argument, Ferron clears the interpreter list. The file must then run directly, for example via a shebang line or native executable.
> - For Unix systems, files without a matching interpreter must have the executable permission bit set.
> - On Unix systems, Ferron parses scripts with a shebang line (for example, `#!/usr/bin/env python3`). It derives the interpreter from the shebang.
> - On Windows, Ferron executes `.exe` files directly. It uses `cmd /c` for `.bat` and `.cmd` files. Ferron parses shebang scripts in the same way as on Unix.

### `environment`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `environment` | `<name: string> <value: string>` | This directive sets a CGI environment variable passed to the interpreter process. Ferron resolves values with the same interpolation syntax as other directives. You can specify this directive multiple times. | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        environment "APP_ENV" "production"
        environment "APP_SECRET" "{{env.APP_SECRET}}"
        environment "RUBY_VERSION" "3.3"
    }
}
```

> [!note]
>
> - Environment variables take precedence over any existing variables with the same name.
> - Ferron automatically removes the `Proxy` header from the request to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
> - Ferron always sets `SERVER_SOFTWARE`, `SERVER_NAME`, and `SERVER_PORT` automatically.
> - Ferron also sets `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN`.

> [!note]
>
> - Ferron always removes the `Proxy` header to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
> - If a CGI script exits with a non-zero status, Ferron returns a `500 Internal Server Error` response. Ferron trims the stderr output and logs a `WARN` message.
> - Ferron sets the working directory to the directory that contains the script file.

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

## Default index files

When you enable CGI and do not set an explicit `index` directive, Ferron adds default index file names. By default, Ferron checks `index.html`, `index.htm`, and `index.xhtml` in order.

If you register additional extensions via the `extension` directive, Ferron also prepends corresponding index files to the front of the list:

| Registered extension | Prepend to index list |
| --- | --- |
| `.cgi` | `index.cgi` |
| `.php` | `index.php` |

For example, with `extension ".php"` configured, the injection order becomes: `index.php`, `index.html`, `index.htm`, `index.xhtml`.

This injection applies only when you do not set an explicit `index` directive. With your own `index` directive, Ferron uses it instead.

> [!important]
> CGI scripts must be inside a `cgi-bin` directory or have an extension registered via the `extension` directive. On Unix, scripts without a matching `interpreter` directive must have the executable permission bit set (`chmod +x`). On Windows, shebang lines work for `.bat`, `.cmd`, and other script files. Ferron executes native `.exe` files directly.

## CGI script locations

Ferron treats a request as a CGI script when:

1. The resolved path is inside a `cgi-bin` directory (case-insensitive match on the first path component after the document root), **or**
2. The file extension matches one registered via the `extension` directive.

When Ferron finds a matching file, it looks for an interpreter in this priority:

1. Custom `interpreter` directive matching the file extension.
2. Built-in default interpreter for the extension.
3. If the file is directly executable (executable bit on Unix, or native `.exe` on Windows), Ferron runs it directly.
4. If the file has a shebang line, Ferron parses the interpreter from the shebang.

> [!tip]
> CGI scripts receive `REMOTE_USER` and `AUTH_TYPE` only when used alongside `http-basicauth`. For related configuration, see [Static file serving](/docs/v3/configuration/content/static-files), [URL rewriting](/docs/v3/configuration/routing/rewrite), and [HTTP headers and CORS](/docs/v3/configuration/content/headers).

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
| `AUTH_TYPE` | Authentication type from the `Authorization` header (for example, `Basic`, `Bearer`). |
| `REMOTE_USER` | Authenticated username, if available. |
| `SERVER_ADMIN` | Server administrator email (from `admin_email` configuration). |
| `HTTPS` | `on` when the server encrypts the connection. |

Additional variables set by `environment` directives override any automatically set variables with the same name.

## Trace context injection

When the request has a trace context, Ferron injects W3C Trace Context headers into the CGI request. These headers (`traceparent`, `tracestate`, and `baggage`) map to standard CGI environment variables:

| Header | CGI environment variable |
| --- | --- |
| `traceparent` | `HTTP_TRACEPARENT` |
| `tracestate` | `HTTP_TRACESTATE` |
| `baggage` | `HTTP_BAGGAGE` |

This enables end-to-end distributed tracing with CGI scripts. For example, a PHP script running with the official OpenTelemetry SDK for PHP can read these headers. It then creates child spans automatically.

> [!info]
> You need no per-module configuration. Trace context injection depends globally on whether a trace context exists. See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details on enabling trace generation and sampling.

## Observability

### Logs

- **`WARN`**: logged when a CGI script exits with a non-zero status and produces output on stderr. The message includes the trimmed stderr content.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| CGI errors on stderr  | WARN  | `error.message` (string): trimmed stderr output from the CGI process |

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.cgi.requests` | Counter | — | Number of CGI requests processed |
| `ferron.cgi.failures` | Counter | `error.type` (`"non_zero_exit_code"`), `ferron.cgi.exit_code` | Number of CGI requests that failed with a non-zero exit code |
| `ferron.cgi.process.duration` | Histogram | — | How long a CGI process runs |
| `ferron.cgi.stderr_errors` | Counter | — | Number of CGI requests that produced non-empty stderr output |

### Access log fields

The CGI module contributes the following fields to the HTTP access log line:

| Field | Type | Description |
| --- | --- | --- |
| `ferron.cgi.script_path` | string | Path to CGI script executed. |
| `ferron.cgi.exit_code` | int | CGI process exit code. |

### Trace spans

The CGI stage sets the following attributes on its `ferron.stage.cgi` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `http.response.status_code` | int | HTTP status code returned by the CGI script. |
| `ferron.cgi.script_path` | string | Path to the CGI script. |
| `ferron.cgi.exit_code` | int | Exit code of the CGI process. |
| `error.type` | string | Error type on failure, enabling trace UI highlighting. |

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

This allows Ferron to handle PHP files via shebang lines or direct execution instead.

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
