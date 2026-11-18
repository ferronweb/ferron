---
title: "CGI applications"
description: "Host legacy CGI applications on Ferron with extension mapping, interpreters, environment variables, and safe upload paths."
---

Ferron runs classic CGI (Common Gateway Interface) applications through the `http-cgi` module. Use it for legacy stacks and script-based workflows. For new deployments, prefer reverse proxying or FastCGI. Both avoid a new process for each request.

## Basic setup

Enable CGI at the host scope:

```ferron
example.com {
    root /var/www/html
    cgi
}
```

Ferron treats scripts inside a `cgi-bin` directory as CGI programs. The directory name must be exactly `cgi-bin` (case-insensitive). It must sit directly under the document root.

```text
/var/www/html/
├── index.html
└── cgi-bin/
    ├── handler.php
    └── script.py
```

Ferron does the following for CGI requests:

- Removes the `Proxy` header to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Sets the working directory to the directory that contains the script.
- Returns `500 Internal Server Error` and logs a `WARN` message when a script exits with a non-zero status.
- Logs stderr output from the script as a warning, trimmed before logging.

## Execute by extension

To run CGI scripts outside `cgi-bin`, register their extensions:

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

Ferron then treats `/var/www/html/scripts/process.php` and `/var/www/html/scripts/convert.py` as CGI scripts. It still serves `/var/www/html/static/style.css` as a static file.

Extension matching is case-insensitive, so `.PHP` matches `.php`. Extension matching is complementary to `cgi-bin` matching. A file can be CGI because it sits in `cgi-bin`, because it has a registered extension, or both.

## Custom interpreters

Set an interpreter for a specific extension:

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

Ferron appends the file path as the last argument. A request to `/cgi-bin/handler.php` runs:

```bash
php-cgi -c /etc/php/8.2/cgi/php.ini /var/www/html/cgi-bin/handler.php
```

To disable the default interpreter for an extension, pass `false`:

```ferron
example.com {
    root /var/www/html
    cgi {
        interpreter ".php" false
    }
}
```

Ferron then handles PHP files through a shebang line, such as `#!/usr/bin/env php`, or executes them directly.

### Default interpreters

When no custom `interpreter` matches, Ferron uses these defaults:

| Extension        | Default interpreter |
| ---------------- | ------------------- |
| `.pl`            | `perl`              |
| `.py`            | `python`            |
| `.sh`            | `bash`              |
| `.ksh`           | `ksh`               |
| `.csh`           | `csh`               |
| `.rb`            | `ruby`              |
| `.php`           | `php-cgi`           |
| `.exe` (Windows) | direct execution    |
| `.bat` (Windows) | `cmd /c`            |
| `.vbs` (Windows) | `cscript`           |

On Unix, Ferron reads a shebang line such as `#!/usr/bin/env python3` and uses that interpreter. On Windows, Ferron executes `.exe` files directly.

## Environment variables

Set environment variables for the interpreter process:

```ferron
example.com {
    root /var/www/html
    cgi {
        environment APP_ENV "production"
        environment APP_SECRET "{{env.APP_SECRET}}"
        environment RUBY_VERSION "3.3"
    }
}
```

Environment variables take precedence over existing variables with the same name. Values support interpolation, for example `{{env.VAR}}`.

Ferron also sets these CGI environment variables:

| Variable          | Description                                                    |
| ----------------- | -------------------------------------------------------------- |
| `SERVER_SOFTWARE` | Always `Ferron`.                                               |
| `SERVER_NAME`     | Server hostname.                                               |
| `SERVER_ADDR`     | Local server address.                                          |
| `SERVER_PORT`     | Server port.                                                   |
| `REQUEST_METHOD`  | HTTP method.                                                   |
| `REQUEST_URI`     | Original request URI.                                          |
| `QUERY_STRING`    | Query string. Empty when the request has none.                 |
| `PATH_INFO`       | Path info extracted from the request.                          |
| `SCRIPT_NAME`     | Script path relative to the document root.                     |
| `AUTH_TYPE`       | Authentication type from the `Authorization` header.           |
| `REMOTE_USER`     | Authenticated username, when available.                        |
| `SERVER_ADMIN`    | Server administrator email from the `admin_email` setting.     |
| `HTTPS`           | `on` when the connection uses encryption.                      |

## Security

### Upload paths

Keep upload and download directories outside `cgi-bin` and outside extension-registered paths. Otherwise, a user can upload a script and execute it as CGI.

```ferron
example.com {
    root /var/www/html

    cgi {
        extension ".php"
    }

    location /uploads {
        root /var/www/html/uploads
        cgi false
    }
}
```

### Interpreter permissions

On Unix, a script without a matching `interpreter` directive must have the executable bit set (`chmod +x`). On Windows, Ferron executes `.exe` files directly and reads shebang lines like Unix.

### Trace context

When tracing is enabled, CGI scripts receive W3C Trace Context headers as environment variables:

- `HTTP_TRACEPARENT`
- `HTTP_TRACESTATE`
- `HTTP_BAGGAGE`

> [!info]
> See [Tracing configuration](/docs/configuration/observability/tracing) to enable trace generation and sampling.

## Default index files

When CGI is enabled and no `index` directive is set, Ferron injects these index names in order: `index.html`, `index.htm`, `index.xhtml`.

For each registered extension, Ferron adds a matching index file to the front of the list:

| Registered extension | Prepended index |
| -------------------- | --------------- |
| `.cgi`               | `index.cgi`     |
| `.php`               | `index.php`     |

With `extension ".php"`, the order becomes `index.php`, `index.html`, `index.htm`, `index.xhtml`.

An explicit `index` directive replaces the injected list.

## See also

- [PHP hosting](/docs/use-cases/content/php)
- [Configuration: CGI](/docs/configuration/content/cgi)
- [Configuration: FastCGI](/docs/configuration/content/fastcgi)
