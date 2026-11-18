---
title: PHP hosting
description: "Run PHP with FastCGI or CGI, disable PHP for selected paths, and pass trace context to the application."
---

Ferron runs PHP through FastCGI or CGI. Use FastCGI for production. PHP worker processes stay alive between requests, so throughput is higher and startup cost is lower.

## FastCGI with PHP-FPM

Point `fcgi_php` to the PHP-FPM socket or TCP address:

```ferron
example.com {
    root /var/www/html
    fcgi_php unix:///run/php/php8.4-fpm.sock
}
```

The socket must be readable and writable by the user that runs Ferron. In the PHP-FPM pool configuration:

```ini
listen.owner = ferron
listen.group = ferron
```

You can also use a TCP listener, for example `fcgi_php tcp://127.0.0.1:9000/`.

## Disable PHP for a path

A host with `fcgi_php` sends every `.php` file to PHP. Disable that for static paths or proxied API paths with `fcgi_php false`:

```ferron
example.com {
    root /var/www/html
    fcgi_php unix:///run/php/php8.4-fpm.sock

    location /static {
        fcgi_php false
        root /var/www/static
    }

    location /api {
        fcgi_php false
        proxy http://localhost:3000
    }
}
```

Without `fcgi_php false`, Ferron sends `.php` files in those locations to the PHP backend. A mixed deployment can then run the wrong handler.

## CGI

For classic CGI, set the `cgi` block and map the `.php` extension:

```ferron
example.com {
    root /var/www/html
    cgi {
        extension ".php"
    }
}
```

CGI starts a PHP process for each request. It is slower than FastCGI under load. For interpreter options, see [Configuration: CGI](/docs/configuration/content/cgi).

> [!tip]
> If PHP files download instead of execute, check that the host or location has `fcgi_php` or `cgi` and `extension ".php"`.
>
> If PHP-CGI reports a force-cgi-redirect warning, set `cgi.force_redirect = 0` in the CGI `php.ini`.

## Distributed tracing

When tracing is enabled, Ferron sends W3C Trace Context headers to CGI and FastCGI applications:

- `HTTP_TRACEPARENT`
- `HTTP_TRACESTATE`
- `HTTP_BAGGAGE`

The [OpenTelemetry SDK for PHP](https://opentelemetry.io/docs/languages/php/) reads `traceparent` and creates child spans. No other PHP-side configuration is required.

> [!important]
> Keep upload and download directories outside `cgi-bin`. CGI must not execute files that users can upload.
>
> See [Tracing configuration](/docs/configuration/observability/tracing) to enable trace generation and sampling in Ferron.

## See also

- [PHP edge caching (LSCache)](/docs/use-cases/content/php-edge-cache): use Ferron as an edge cache in front of Apache with the LSCache plugin.
- [Configuration: FastCGI](/docs/configuration/content/fastcgi)
- [Configuration: CGI](/docs/configuration/content/cgi)
