---
title: Reverse proxying
description: "Configure Ferron as a reverse proxy with WebSocket support, optional static/SPA hosting, multiple locations, load balancing, and header manipulation."
---

Configuring Ferron as a reverse proxy is straightforward — you just need to specify the backend server URL using the `proxy` directive. To configure Ferron as a reverse proxy, you can use the configuration below:

```ferron
example.com {
    proxy http://localhost:3000
}
```

The WebSocket protocol is supported out of the box in this configuration — no additional configuration is required.

## Reverse proxy with static file serving support

Ferron supports serving static files and reverse proxying at once. You can use separate `location` blocks for this:

```ferron
example.com {
    # The "/api" location is used for reverse proxying
    # For example, "/api/login" is proxied to "http://localhost:3000/api/login"
    location /api {
        proxy http://localhost:3000/api
    }

    # The "/" location is used for serving static files
    location / {
        root /var/www/html
    }
}
```

## Reverse proxy with a single-page application

Ferron supports serving a single-page application and reverse proxying at once. You can use this configuration:

```ferron
example.com {
    # The "/api" location is used for reverse proxying
    location /api {
        proxy http://localhost:3000/api
    }

    # The "/" location is used for serving static files with SPA fallback
    location / {
        root /var/www/html
        rewrite "^/.*" "/" {
            last true
            directory false
            file false
        }
    }
}
```

## Load balancing

Ferron supports load balancing by specifying multiple upstream backends inside a `proxy` block. To configure Ferron as a load balancer, you can use the configuration below:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        lb_algorithm two_random
    }
}
```

### Load balancing algorithms

| Algorithm | Description |
| --- | --- |
| `random` | Selects a backend randomly for each request. |
| `round_robin` | Cycles through backends in order. |
| `least_conn` | Selects the backend with the fewest active tracked connections. |
| `two_random` | Picks two random backends and selects the less loaded one. |

## Health checks

Ferron supports passive health checks. To enable passive health checking:

```ferron
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        lb_health_check
        lb_health_check_max_fails 3
        lb_health_check_window 5s
    }
}
```

## Reverse proxy to backends listening on Unix sockets

Ferron supports reverse proxying to backends listening on Unix sockets:

```ferron
example.com {
    proxy http://backend {
        upstream http://backend {
            unix /run/backend/web.sock
        }
    }
}
```

## Reverse proxy to gRPC backends

Ferron supports reverse proxying to gRPC backends that accept HTTP/2 requests:

```ferron
grpc.example.com {
    proxy http://localhost:3000 {
        http2_only true
    }
}
```

## Reverse proxy to dynamic backends (via SRV records)

Ferron supports reverse proxying to dynamic backends via DNS SRV records (requires `srv-lookup` feature):

```ferron
example.com {
    proxy {
        srv _backend._tcp.example.com
    }
}
```

## Example: Ferron multiplexing to several backend servers

In this example, the `example.com` and `bar.example.com` domains point to a server running Ferron.

Below are assumptions made for this example:

- `https://example.com` is "main site", while `https://example.com/agenda` is hosting a calendar service.
- `https://foo.example.com` is passed to `https://saas.foo.net`
- `https://bar.example.com` is the front for an internal backend.

You can configure Ferron like this:

```ferron
* {
    tls {
        provider "manual"
        cert "/path/to/certificate.crt"
        key "/path/to/private.key"
    }
}

example.com {
    location /agenda {
        # It proxies /agenda/example to http://calender.example.net:5000/agenda/example
        proxy http://calender.example.net:5000
    }

    location / {
        # Catch-all path
        proxy http://localhost:3000
    }
}

foo.example.com {
    location / {
        proxy https://saas.foo.net
    }
}

bar.example.com {
    location / {
        proxy http://backend.example.net:4000
    }
}
```

For `http://calender.example.net:5000/agenda/example`, you will probably have to either configure the calendar service to strip `agenda/` or configure URL rewriting in Ferron.

## Notes and troubleshooting

- If you get `502 Bad Gateway` or `504 Gateway Timeout`, verify the `upstream` URL is reachable and check `lb_health_check_max_fails` settings.
- If only some paths fail, review `location` matching order — more specific locations win over less specific ones.
- For mixed static + API setups, keep API routes in a dedicated prefix like `/api` and use a catch-all `/` location for static files or SPA fallback.
- For gRPC upstreams, enable `http2_only`; without HTTP/2-only proxying, many gRPC backends will fail.
- If Ferron is behind an HTTPS-terminating proxy and you also use automatic TLS, use HTTP-01 challenge instead of TLS-ALPN-01. See [Automatic TLS](/docs/v3/use-cases/automatic-tls#note-about-cloudflare-proxies-and-other-https-proxies).
- For upstream header forwarding details, see [Reverse proxying configuration reference](/docs/v3/configuration/reverse-proxying).
