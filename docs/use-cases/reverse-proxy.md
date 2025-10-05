---
title: Reverse proxying
---

Configuring Ferron as a reverse proxy is straightforward - you just need to specify the backend server URL in `proxy` directive. To configure Ferron as a reverse proxy, you can use the configuration below:

```kdl
// Example configuration with reverse proxy. Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000/" // Replace "http://localhost:3000" with the backend server URL
}
```

## Reverse proxy with static file serving support

Ferron supports serving static files and reverse proxying at once. You can use this configuration for this use case:

```kdl
// Example configuration with reverse proxy and static file serving. Replace "example.com" with your domain name.
example.com {
    // The "/api" location is used for reverse proxying
    // For example, the "/api/login" endpoint is proxied to "http://localhost:3000/api/login"
    location "/api" {
        proxy "http://localhost:3000" // Replace "http://localhost:3000" with the backend server URL
    }

    // The "/" location is used for serving static files
    location "/" {
        root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
    }
}
```

## Reverse proxy with a single-page application

Ferron supports serving a single-page application and reverse proxying at once. You can use this configuration for this use case:

```kdl
// Example configuration with reverse proxy and static file serving. Replace "example.com" with your domain name.
example.com {
    // The "/api" location is used for reverse proxying
    // For example, the "/api/login" endpoint is proxied to "http://localhost:3000/api/login"
    location "/api" {
        proxy "http://localhost:3000" // Replace "http://localhost:3000" with the backend server URL
    }

    // The "/" location is used for serving static files
    location "/" {
        root "/var/www/html" // Replace "/var/www/html" with the directory containing your static files
        rewrite "^/.*" "/" directory=#false file=#false last=#true
    }
}
```

## Load balancing

Ferron supports load balancing by specifying multiple backend servers in the `proxy` directive. To configure Ferron as a load balancer, you can use the configuration below:

```kdl
// Example configuration with load balancing. Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000/" // Replace "http://localhost:3000" with the backend server URL
    proxy "http://localhost:3001/" // Replace "http://localhost:3001" with the second backend server URL
}
```

## Health checks

Ferron supports passive health checks; you can enable it using `lb_health_check` directive. To configure Ferron as a load balancer with passive health checking, you can use the configuration below:

```kdl
// Example configuration with load balancing and passive health checking. Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000/" // Replace "http://localhost:3000" with the backend server URL
    proxy "http://localhost:3001/" // Replace "http://localhost:3001" with the second backend server URL
    lb_health_check
}
```

## Caching reverse proxy

Ferron supports in-memory caching for speeding up websites. To enable in-memory caching for the reverse proxy, you can use this configuration:

```kdl
// Example configuration with caching reverse proxy. Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000/" // Replace "http://localhost:3000" with the backend server URL
    cache
    // Optional: set Cache-Control header if you want to also cache responses from backend servers without the Cache-Control header
    header "Cache-Control" "max-age=3600"
}
```

## Example: Ferron multiplexing to several backend servers

In this example, the `example.com` and `bar.example.com` domains point to a server running Ferron.

Below are assumptions made for this example:

- `https://example.com` is "main site", while `https://example.com/agenda` is hosting a calendar service.
- `https://foo.example.com` is passed to `https://saas.foo.net`
- `https://bar.example.com` is the front for an internal backend.

You can configure Ferron like this:

```kdl
* {
    tls "/path/to/certificate.crt" "/path/to/private.key"
}

example.com {
    location "/agenda" {
        // It proxies /agenda/example to http://calender.example.net:5000/agenda/example
        proxy "http://calender.example.net:5000"
    }

    location "/" {
        // Catch-all path
        proxy "http://localhost:3000/"
    }
}

foo.example.com {
    proxy "https://saas.foo.net"
}

bar.example.com {
    proxy "http://backend.example.net:4000"
}
```

For `http://calender.example.net:5000/agenda/example`, you will probably have to either configure the calendar service to strip 'agenda/' or configure URL rewriting in Ferron.
