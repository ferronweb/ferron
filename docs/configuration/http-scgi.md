---
title: "Configuration: SCGI support"
description: "Server-side CGI protocol support for backend application servers using the SCGI protocol."
---

This page documents the `scgi` directive for configuring Ferron's SCGI (Simple Common Gateway Interface) support. SCGI is a protocol for interfacing external application servers with web servers, similar to CGI but with binary framing for better performance.

## `scgi`

```ferron
example.com {
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
            environment "APP_ENV" "production"
        }
    }
}
```

The `scgi` directive enables SCGI protocol support within a `cgi` block. When specified, Ferron will forward requests to the configured SCGI backend using the SCGI protocol instead of spawning local processes.

| Form | Description |
| --- | --- |
| `scgi { ... }` | Enables SCGI and configures nested directives. |

### `backend`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `backend` | `<url: string>` | This directive specifies the SCGI backend server URL. Supports TCP URLs (`tcp://host:port`) and Unix socket URLs (`unix:///path/to/socket`). | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
        }
    }
}
```

**Configuration example with Unix socket:**

```ferron
example.com {
    cgi {
        scgi {
            backend "unix:///var/run/app.sock"
        }
    }
}
```

**Notes:**

- TCP URLs must include both host and port (e.g., `tcp://127.0.0.1:4000`).
- Unix socket paths must be absolute paths.
- When a connection failure occurs (connection refused, host unreachable, etc.), Ferron logs an error and returns a `503 Service Unavailable` response.

### `environment`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `environment` | `<name: string> <value: string>` | This directive sets an SCGI environment variable passed to the backend server. Values are resolved with the same interpolation syntax as other directives. This directive can be specified multiple times. | — |

**Configuration example:**

```ferron
example.com {
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
            environment "APP_ENV" "production"
            environment "APP_SECRET" "{env:APP_SECRET}"
            environment "RUBY_VERSION" "3.3"
        }
    }
}
```

**Notes:**

- Environment variables take precedence over any existing variables with the same name.
- The `Proxy` header is automatically removed from the request to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Ferron always sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_ADDR`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically.

## Environment variables

Ferron automatically sets the following SCGI environment variables:

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

## Authentication

When used alongside an authentication module (e.g., `http-basicauth`), Ferron automatically populates the `AUTH_TYPE` and `REMOTE_USER` environment variables in the SCGI request. The authentication type is extracted from the `Authorization` header (e.g., `Basic` or `Bearer`).

## Observability

### Logs

- **`ERROR`**: logged when a connection to the SCGI backend fails. The message includes the connection error details.

## Examples

### Basic SCGI backend

```ferron
example.com {
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
        }
    }
}
```

### SCGI with Unix socket

```ferron
example.com {
    cgi {
        scgi {
            backend "unix:///var/run/app.sock"
        }
    }
}
```

### SCGI with environment variables

```ferron
example.com {
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
            environment "APP_ENV" "production"
            environment "APP_SECRET" "{env:APP_SECRET}"
            environment "RUBY_VERSION" "3.3"
        }
    }
}
```

### SCGI with authentication

```ferron
example.com {
    root /var/www/html
    
    basicauth {
        user "admin" "password"
    }
    
    cgi {
        scgi {
            backend "tcp://127.0.0.1:4000"
        }
    }
}
```

## Notes and troubleshooting

- The `scgi` directive must be nested inside a `cgi` block. It cannot be used at the global scope or directly under an HTTP host block.
- SCGI backends must implement the SCGI protocol correctly. Ferron uses the `cega-scgi` crate for protocol compliance.
- When a connection to the SCGI backend fails, Ferron returns a `503 Service Unavailable` response and logs an error message.
- For TCP backends, ensure the host and port are specified in the URL (e.g., `tcp://127.0.0.1:4000`).
- For Unix socket backends, the path must be absolute (e.g., `unix:///var/run/app.sock`).
- The `Proxy` header is always removed to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Ferron sets `SERVER_SOFTWARE` to `Ferron` automatically.
- For CGI stderr output, Ferron logs warnings when the script produces output on stderr. The output is trimmed before logging.
- For authentication integration, SCGI scripts receive `REMOTE_USER` and `AUTH_TYPE` only when used alongside a module like `http-basicauth` that sets `ctx.auth_user`.
- For static file serving alongside SCGI, see [Static file serving](/docs/v3/configuration/static-content).
- For URL rewriting, see [URL rewriting](/docs/v3/configuration/http-rewrite).
- For response headers and CORS, see [HTTP headers and CORS](/docs/v3/configuration/http-headers).