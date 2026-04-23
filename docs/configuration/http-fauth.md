---
title: "Configuration: forwarded authentication"
description: "External authentication backend integration with connection pooling, header copying, and configurable backends."
---

This page documents the `auth_to` directive for configuring forwarded authentication. Forwarded authentication sends every incoming request to an external backend server for verification before the request is processed. If the backend returns a success status (2xx), the request continues through the pipeline. If it returns a failure status (4xx/5xx), the backend's response is returned directly to the client.

This pattern is commonly used with authentication proxies like [Authelia](https://www.authelia.com/), [Keycloak](https://www.keycloak.org/), or custom authentication services.

## Directives

### `auth_to`

```ferron
example.com {
    auth_to http://localhost:9091 {
        limit 50
        idle_timeout "30s"
        no_verification false

        copy X-Auth-User X-Auth-Roles
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `url` | `<string>` | Backend server URL (http:// or https://). Required if not provided as an argument. | — |
| `unix` | `<path>` | Connect to the backend via Unix domain socket instead of TCP. | TCP |
| `limit` | `<number>` | Maximum concurrent connections to this backend. | `1` (per upstream) |
| `idle_timeout` | `<duration>` | Keep-alive idle timeout for connections. Connections idle longer than this are evicted. | `60s` |
| `no_verification` | `<bool>` | Skip TLS certificate verification for HTTPS backends. | `false` |
| `copy` | `<string>...` | Headers to copy from the auth response back to the original request. Supports multiple headers. | none |

#### Backend URL

The `auth_to` directive requires a backend URL, specified either as a direct argument or via a nested `url` directive:

```ferron
example.com {
    # Direct argument form
    auth_to http://auth.example.com/auth

    # Nested form
    auth_to {
        url http://auth.example.com/auth
    }
}
```

#### Unix socket connections

To connect to a backend via Unix domain socket, use the `unix` nested directive:

```ferron
example.com {
    auth_to http://localhost/auth {
        unix /var/run/authelia/authelia.sock
    }
}
```

When `unix` is specified, the URL host is ignored for the actual connection but must still be present for the HTTP scheme.

#### Connection limits

Each backend can have its own connection limit via the `limit` directive:

```ferron
example.com {
    auth_to http://auth1.example.com {
        limit 100
        idle_timeout "60s"
    }
}

second.example.com {
    auth_to http://auth2.example.com {
        limit 50
    }
}
```

Multiple `auth_to` blocks can be defined for different backends. Ferron uses the first matching configuration.

#### Header copying

When authentication succeeds, headers from the backend response can be copied to the original request. This is useful for passing user identity, roles, or other metadata downstream:

```ferron
example.com {
    auth_to http://auth.example.com/auth {
        copy X-Auth-User X-Auth-Roles X-Auth-Email
    }

    # The copied headers are now available in the request
    proxy http://backend:8080
}
```

Headers are copied by name — if the auth response contains the specified header, it is added to the original request. Multiple values are preserved.

### Global connection limit

The global `auth_to_concurrent_conns` directive controls the maximum number of concurrent connections across all forwarded authentication backends:

```ferron
{
    auth_to_concurrent_conns 16384
}

example.com {
    auth_to http://auth.example.com
}
```

| Argument | Description |
| --- | --- |
| `<number>` | Maximum concurrent connections (positive integer). |
| `false` | Disable the limit (unbounded). |

Default: `auth_to_concurrent_conns 16384`

## Authentication flow

1. The stage receives the incoming request and parses the `auth_to` configuration.
2. A new HTTP request is constructed using the original request's method, path, query string, and headers.
3. Standard forwarding headers (`X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Uri`, `X-Forwarded-Method`, `Forwarded`) are added.
4. The request is sent to the authentication backend via the connection pool.
5. **On success (2xx)**: Configured headers are copied from the response to the original request. The pipeline continues.
6. **On failure (4xx/5xx)**: The backend's response is returned directly to the client. The pipeline stops.

## Stage ordering

The `forwarded_auth` stage runs in the following position in the pipeline:

- **After** `cache` (caching occurs before authentication)
- **After** `basicauth` (basic auth is checked before forwarded auth)
- **Before** `reverse_proxy` (authentication before proxying)
- **Before** `forward_proxy` (authentication before forwarding)

## Configuration examples

### Basic forwarded authentication

```ferron
example.com {
    auth_to http://auth.example.com/auth

    proxy http://backend:8080
}
```

### Authentication with user headers

```ferron
api.example.com {
    auth_to http://auth.example.com/validate {
        copy X-Auth-User X-Auth-Roles X-Auth-Email
    }

    proxy http://backend:8080 {
        request_header +X-User "{{request.header.x_auth_user}}"
        request_header +X-Roles "{{request.header.x_auth_roles}}"
    }
}
```

### Unix socket backend

```ferron
secure.example.com {
    auth_to http://localhost/auth {
        unix /var/run/authelia/authelia.sock
        limit 100
        idle_timeout "120s"
    }

    proxy http://backend:8080
}
```

### Self-signed certificate backend

```ferron
internal.example.com {
    auth_to https://auth.internal:8443/auth {
        no_verification true
    }

    proxy https://backend:8443 {
        no_verification true
    }
}
```

### Disabling the global connection limit

```ferron
{
    auth_to_concurrent_conns false
}

example.com {
    auth_to http://auth.example.com
}
```

## Notes and troubleshooting

- The forwarded auth request uses the **same path and query string** as the original request. The backend URL provides the base address.
- If the backend is unreachable or returns a non-2xx status, the request is **blocked** and the backend's response is returned to the client.
- Connection pooling is used for authenticated backends to reduce latency. Connections are reused across requests.
- When `client_ip_from_header` is enabled, `X-Forwarded-For` is **appended** to the existing chain rather than replaced. The `Forwarded` header (RFC 7239) is also managed accordingly.
- Upgrade and Connection headers are removed from auth requests since HTTP upgrades are not supported for authentication.
- For authentication backends behind TLS, ensure the backend's certificate is valid or use `no_verification true` for development/testing.
- For Basic Auth configuration, see [HTTP basic authentication](/docs/v3/configuration/http-basicauth).
- For reverse proxy configuration, see [Reverse proxy](/docs/v3/configuration/reverse-proxying).
