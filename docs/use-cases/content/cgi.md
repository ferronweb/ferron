---
title: "CGI applications"
description: "Host legacy CGI applications on Ferron, including extension mapping, interpreters, environment variables, and security considerations."
---

Ferron supports classic CGI (Common Gateway Interface) applications through the `http-cgi` module. This is mainly useful for legacy stacks and script-based workflows.

For new deployments, prefer HTTP reverse proxying or FastCGI when possible, because it avoids starting a new process per request and usually performs better.

## Basic CGI setup

To run CGI programs, enable `cgi` at the HTTP host scope:

```ferron
example.com {
    root /var/www/html
    cgi
}
```

In this setup, scripts inside a `cgi-bin` directory are automatically treated as CGI programs. The directory must be named exactly `cgi-bin` (case-insensitive) and must be directly under the document root.

Example directory structure:

```text
/var/www/html/
├── index.html
└── cgi-bin/
    ├── handler.php
    └── script.py
```

> [!note]
>
> - The `Proxy` header is always removed to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
> - If a CGI script exits with a non-zero status, Ferron logs a `WARN` message and returns a `500 Internal Server Error` response — stderr output from the script is logged as a warning (trimmed before logging).
> - The working directory is set to the directory containing the script file.

## Executing scripts by extension

You can also execute CGI scripts outside `cgi-bin` by registering additional file extensions:

```ferron
example.com {
    root /var/www/html
    cgi {
        extension ".php"
        extension ".py"
        extension ".rb"
    }
}
```

With this configuration:

- `/var/www/html/scripts/process.php` is treated as a CGI script
- `/var/www/html/scripts/convert.py` is treated as a CGI script
- `/var/www/html/static/style.css` is served as a static file

> [!note]
>
> - Extensions are matched case-insensitively (`.PHP` matches `.php`).
> - Files with registered extensions are executed as CGI scripts regardless of their location.
> - This is complementary to `cgi-bin` directory matching — a file can be CGI either by being in `cgi-bin` or by having a registered extension.

## Custom CGI interpreters

Define explicit interpreters for specific file extensions:

```ferron
example.com {
    root /var/www/html
    cgi {
        interpreter ".php" php-cgi -c /etc/php/8.2/cgi/php.ini
        interpreter ".pl" perl
        interpreter ".py" python3
    }
}
```

The file path is automatically appended as the final argument to the interpreter command. For example, a request to `/cgi-bin/handler.php` with the above configuration runs:

```bash
php-cgi -c /etc/php/8.2/cgi/php.ini /var/www/html/cgi-bin/handler.php
```

### Disabling default interpreters

Pass `false` as the second argument to `interpreter` to disable the default interpreter for that extension:

```ferron
example.com {
    root /var/www/html
    cgi {
        interpreter ".php" false
    }
}
```

This allows PHP files to be handled via shebang lines (`#!/usr/bin/env php`) or direct execution instead of the default `php-cgi` interpreter.

### Built-in default interpreters

When no custom `interpreter` directive matches, Ferron uses these built-in defaults:

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

On Unix systems, scripts with a shebang line (e.g., `#!/usr/bin/env python3`) are parsed and the interpreter is derived from the shebang. On Windows, `.exe` files are executed directly.

## Environment variables

Set CGI environment variables that are passed to the interpreter process:

```ferron
example.com {
    root /var/www/html
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
> - Values support interpolation (e.g., `{{env.VAR}}` for environment variable substitution).
> - Ferron always sets the following CGI environment variables automatically:

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
| `AUTH_TYPE` | Authentication type from the `Authorization` header. |
| `REMOTE_USER` | Authenticated username, if available. |
| `SERVER_ADMIN` | Server administrator email (from `admin_email` configuration). |
| `HTTPS` | Set to `on` when the connection is encrypted. |

## Security considerations

### File upload safety

Keep upload and download directories **outside** `cgi-bin` and outside any extension-registered directories. Otherwise, a user could upload a malicious script and execute it as CGI.

Example safe configuration:

```ferron
example.com {
    root /var/www/html

    # CGI is only enabled for cgi-bin and .php scripts
    cgi {
        extension ".php"
    }

    # Upload directory is safe (no CGI execution)
    location /uploads {
        root /var/www/html/uploads
        cgi false
    }
}
```

### Interpreter permissions

On Unix systems, scripts without a matching `interpreter` directive must have the executable permission bit set (`chmod +x`). On Windows, `.exe` files are executed directly, and scripts with shebangs are parsed similarly to Unix.

### Distributed tracing

When tracing is enabled in Ferron, CGI scripts automatically receive W3C Trace Context headers (`traceparent`, `tracestate`, `baggage`) as CGI environment variables (`HTTP_TRACEPARENT`, `HTTP_TRACESTATE`, `HTTP_BAGGAGE`). This enables end-to-end distributed tracing without any CGI-side configuration.

> [!info]
> See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details on enabling trace generation and sampling.

## Default index files

When CGI is enabled and no explicit `index` directive is configured, Ferron automatically injects default index file names. By default, the following files are checked in order: `index.html`, `index.htm`, `index.xhtml`.

If you register additional extensions via the `extension` directive, Ferron also prepends corresponding index files to the front of the list:

| Registered extension | Prepend to index list |
| --- | --- |
| `.cgi` | `index.cgi` |
| `.php` | `index.php` |

For example, with `extension ".php"` configured, the injection order becomes: `index.php`, `index.html`, `index.htm`, `index.xhtml`.

This injection only applies when no explicit `index` directive is set. If you configure your own `index` directive, Ferron will use that instead.

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
