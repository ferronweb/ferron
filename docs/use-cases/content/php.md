---
title: PHP hosting
description: "Host PHP sites on Ferron using CGI or FastCGI (PHP-FPM or PHP-CGI), with Ferron configuration examples and troubleshooting notes."
---

Ferron can run PHP applications through CGI or FastCGI. For most deployments, FastCGI is the recommended approach because PHP worker processes stay alive between requests, which reduces process startup overhead and improves throughput.

## PHP through FastCGI (recommended)

To run PHP with FastCGI (commonly PHP-FPM), use `fcgi_php`:

```ferron
# Example configuration with PHP through FastCGI. Replace "example.com" with your domain name.
example.com {
    root /var/www/html # Replace "/var/www/html" with your PHP app directory
    fcgi_php "unix:///run/php/php8.4-fpm.sock" # Replace with your PHP FastCGI socket or TCP URL

    # If using PHP-FPM over a Unix socket, ensure the socket is accessible by Ferron.
    # For example, in your PHP-FPM pool configuration:
    #   listen.owner = ferron
    #   listen.group = ferron
}
```

You can also point `fcgi_php` to TCP listeners (for example `tcp://127.0.0.1:9000/`) when your PHP FastCGI server is not exposed through a Unix socket.

## PHP through CGI

If you specifically want classic CGI execution, enable `cgi` and map the `.php` extension:

```ferron
# Example configuration with PHP through CGI. Replace "example.com" with your domain name.
example.com {
    root /var/www/html # Replace "/var/www/html" with your PHP app directory
    cgi {
        extension ".php"
    }
}
```

CGI is functional but usually slower than FastCGI for production workloads because a PHP process is started per request. For more control, see [Configuration: FastCGI support](/docs/v3/configuration/content/fastcgi).

> [!tip]
>
> - If using PHP-CGI with the CGI module, you may need `cgi.force_redirect = 0` in your CGI `php.ini`; otherwise requests can fail with a force-cgi-redirect warning.
> - If PHP files download instead of executing, verify you enabled either `fcgi_php` or `cgi` + `extension ".php"` in the correct domain/location block.

## Distributed tracing with PHP

PHP applications served through CGI or FastCGI automatically receive W3C Trace Context headers (`traceparent`, `tracestate`, and `baggage`) when tracing is enabled in Ferron. These headers are available as CGI environment variables (`HTTP_TRACEPARENT`, `HTTP_TRACESTATE`, `HTTP_BAGGAGE`).

With the official [OpenTelemetry SDK for PHP](https://opentelemetry.io/docs/languages/php/), these headers enable distributed tracing out of the box — the SDK automatically reads the incoming `traceparent` header and creates child spans, connecting your PHP backend traces to the rest of your infrastructure.

> [!info]
> No additional PHP-side configuration is needed beyond installing and configuring the OpenTelemetry SDK. See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details on enabling trace generation and sampling in Ferron.

> [!important]
> Keep upload/download directories outside of `cgi-bin` when using CGI to avoid accidental CGI execution of uploaded files.

## See also

- [PHP edge caching (LSCache)](/docs/v3/use-cases/content/php-edge-cache) — use Ferron as an edge caching proxy in front of Apache for PHP hosting with LSCache plugin compatibility
