---
title: "Configuration: CGI support"
description: "Run CGI scripts with per-extension interpreters, environment variables, and shebang detection."
---

`cgi` runs external interpreter processes for scripts. Ferron matches scripts by a `cgi-bin` directory, by registered file extension, or by an interpreter mapping.

A named host does not inherit `cgi` from the wildcard `*` host. It still inherits from global defaults, and a `location` block inherits `cgi` from its host.

```ferron
example.com {
    cgi {
        extension ".php"
        interpreter ".php" php-cgi -c /etc/php/cgi.ini
        environment APP_ENV "production"
    }
}
```

| Form          | Effect                                        |
| ------------- | --------------------------------------------- |
| `cgi`         | Enables CGI with all defaults.                |
| `cgi true`    | Explicitly enables CGI.                       |
| `cgi false`   | Disables CGI for the current scope.           |
| `cgi { ... }` | Enables CGI and configures nested directives. |

### `extension`

| Nested directive | Arguments  | Description                                                        | Default |
| ---------------- | ---------- | ------------------------------------------------------------------ | ------- |
| `extension`      | `<string>` | Treat this file extension as a CGI script. Repeatable.             | none    |

```ferron
example.com {
    cgi {
        extension ".php"
        extension ".rb"
    }
}
```

Files with these extensions are CGI scripts anywhere in the file tree. Unlike `cgi-bin` matching, the file does not need to be executable. Extension matching is case-insensitive. Files inside `cgi-bin` are always CGI scripts, with or without an `extension` directive.

### `interpreter`

| Nested directive | Arguments                              | Description                                                    | Default           |
| ---------------- | -------------------------------------- | -------------------------------------------------------------- | ----------------- |
| `interpreter`    | `<extension: string> <arg: string>...` | Map an extension to an interpreter command line. Repeatable.   | built-in defaults |

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

The first argument is the extension, with a leading dot. The other arguments form the command line. Ferron appends the file path as the final argument.

Pass `false` as the second argument to clear the interpreter list for that extension. The file must then run as a shebang script or as a native executable directly.

On Unix, a file without a matching interpreter must have the executable permission bit set. Ferron also reads a shebang line such as `#!/usr/bin/env python3` and derives the interpreter from it.

On Windows, Ferron runs `.exe` files directly and uses `cmd /c` for `.bat` and `.cmd` files. Shebang parsing works the same as on Unix.

### `environment`

| Nested directive | Arguments                        | Description                                              | Default |
| ---------------- | -------------------------------- | -------------------------------------------------------- | ------- |
| `environment`    | `<name: string> <value: string>` | Set an environment variable for the interpreter. Repeatable. | none |

```ferron
example.com {
    cgi {
        environment APP_ENV "production"
        environment APP_SECRET "{{env.APP_SECRET}}"
        environment RUBY_VERSION "3.3"
    }
}
```

Values use the same interpolation syntax as other directives. Variables set here override existing variables with the same name.

Ferron removes the `Proxy` request header to prevent the [httpoxy](https://httpoxy.org/) vulnerability. Ferron sets `SERVER_SOFTWARE`, `SERVER_NAME`, and `SERVER_PORT` automatically, and also sets `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN`.

Ferron sets the working directory to the directory that contains the script file. If a script exits with a non-zero status, Ferron returns `500 Internal Server Error`. It trims the stderr output and logs a `WARN` message.

## Default interpreters

Ferron uses these interpreters when no `interpreter` directive matches.

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

## Default index files

With CGI enabled and no explicit `index` directive, Ferron checks `index.html`, `index.htm`, and `index.xhtml`, in that order.

Registering an extension also prepends an index file:

| Registered extension | Prepended to the index list |
| -------------------- | --------------------------- |
| `.cgi`               | `index.cgi`                 |
| `.php`               | `index.php`                 |

For example, with `extension ".php"`, the order becomes `index.php`, `index.html`, `index.htm`, `index.xhtml`. The injection applies only when you do not set an explicit `index` directive. With your own `index`, Ferron uses your list.

> [!important]
> A CGI script must sit inside a `cgi-bin` directory or have an extension registered with `extension`. On Unix, a script with no matching `interpreter` must have the executable permission bit set (`chmod +x`). On Windows, shebang lines work for `.bat`, `.cmd`, and other script files. Ferron runs native `.exe` files directly.

## Script matching

Ferron treats a request as a CGI script when either condition holds:

1. The resolved path is inside a `cgi-bin` directory. Ferron matches the first path component after the document root, case-insensitively.
2. The file extension is registered with `extension`.

When Ferron finds a matching file, it looks for an interpreter in this order:

1. A custom `interpreter` directive for the extension.
2. The built-in default interpreter for the extension.
3. Direct execution, when the file is executable on Unix or is a native `.exe` on Windows.
4. The shebang line, when the file has one.

Scripts receive `REMOTE_USER` and `AUTH_TYPE` only when used with `http-basicauth`. For related configuration, see [Static file serving](/docs/configuration/content/static-files), [URL rewriting](/docs/configuration/routing/rewrite), and [HTTP headers and CORS](/docs/configuration/content/headers).

## Environment variables

Ferron sets these CGI environment variables automatically.

| Variable          | Description                                                        |
| ----------------- | ------------------------------------------------------------------ |
| `SERVER_SOFTWARE` | Always `Ferron`.                                                   |
| `SERVER_NAME`     | Server hostname.                                                   |
| `SERVER_ADDR`     | Local server address.                                              |
| `SERVER_PORT`     | Server port.                                                       |
| `REQUEST_METHOD`  | HTTP method.                                                       |
| `REQUEST_URI`     | Original request URI.                                              |
| `QUERY_STRING`    | Query string, empty when there is none.                            |
| `PATH_INFO`       | Path info extracted from the request.                              |
| `SCRIPT_NAME`     | Script path relative to the document root.                         |
| `AUTH_TYPE`       | Authentication type from the `Authorization` header, such as `Basic` or `Bearer`. |
| `REMOTE_USER`     | Authenticated username, when available.                            |
| `SERVER_ADMIN`    | Administrator email from `admin_email`.                            |
| `HTTPS`           | `on` when the connection is encrypted.                             |

An `environment` directive overrides an automatically set variable with the same name.

## Trace context injection

When the request has a trace context, Ferron injects W3C Trace Context headers into the CGI request and maps them to CGI variables.

| Header        | CGI environment variable |
| ------------- | ------------------------ |
| `traceparent` | `HTTP_TRACEPARENT`       |
| `tracestate`  | `HTTP_TRACESTATE`        |
| `baggage`     | `HTTP_BAGGAGE`           |

This gives end-to-end distributed tracing for CGI scripts. For example, a PHP script with the official OpenTelemetry SDK for PHP reads these headers and creates child spans automatically.

No per-module configuration is required. Trace context injection depends only on whether a trace context exists. See [Tracing configuration](/docs/configuration/observability/tracing) for trace generation and sampling.

## Observability

### Logs

Ferron logs a `WARN` message when a CGI script exits with a non-zero status and writes output to stderr. The message includes the trimmed stderr content.

| Event                | Level | Attributes                                                         |
| -------------------- | ----- | ------------------------------------------------------------------ |
| CGI errors on stderr | WARN  | `error.message` (string): trimmed stderr output from the CGI process. |

### Metrics

| Metric                        | Type      | Attributes                                                    | Description                                             |
| ----------------------------- | --------- | ------------------------------------------------------------- | ------------------------------------------------------- |
| `ferron.cgi.requests`         | Counter   | none                                                          | CGI requests processed.                                 |
| `ferron.cgi.failures`         | Counter   | `error.type` (`non_zero_exit_code`), `ferron.cgi.exit_code`   | CGI requests that failed with a non-zero exit code.     |
| `ferron.cgi.process.duration` | Histogram | none                                                          | CGI process run time.                                   |
| `ferron.cgi.stderr_errors`    | Counter   | none                                                          | CGI requests that wrote to stderr.                      |

### Access log fields

| Field                    | Type   | Description                  |
| ------------------------ | ------ | ---------------------------- |
| `ferron.cgi.script_path` | string | Path to the CGI script run.  |
| `ferron.cgi.exit_code`   | int    | CGI process exit code.       |

### Trace spans

The CGI stage sets these attributes on its `ferron.stage.cgi` span.

| Attribute                   | Type   | Description                                            |
| --------------------------- | ------ | ------------------------------------------------------ |
| `http.response.status_code` | int    | Status code returned by the CGI script.                |
| `ferron.cgi.script_path`    | string | Path to the CGI script.                                |
| `ferron.cgi.exit_code`      | int    | Exit code of the CGI process.                          |
| `error.type`                | string | Error type on failure. Highlights the span in the trace UI. |
