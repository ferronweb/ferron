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
    root "/var/www/html" # Replace "/var/www/html" with your PHP app directory
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
    root "/var/www/html" # Replace "/var/www/html" with your PHP app directory
    cgi {
        extension ".php"
    }
}
```

CGI is functional but usually slower than FastCGI for production workloads because a PHP process is started per request. For more control, see [Configuration: FastCGI support](/docs/v3/configuration/http-fcgi).

## Notes and troubleshooting

- If PHP files download instead of executing, verify you enabled either `fcgi_php` or `cgi` + `extension ".php"` in the correct domain/location block.
- If using PHP-CGI with the CGI module, you may need `cgi.force_redirect = 0` in your CGI `php.ini`; otherwise requests can fail with a force-cgi-redirect warning.
- If you get `500 Internal Server Error` with `fcgi_php`, verify the socket or TCP endpoint exists and your PHP FastCGI daemon (PHP-FPM or PHP-CGI in FastCGI mode) is running.
- If using Unix sockets, ensure Ferron can access the socket file (owner/group/mode in PHP-FPM pool config).
- Keep upload/download directories outside of `cgi-bin` when using CGI to avoid accidental CGI execution of uploaded files.
- For "pretty URLs" and front controller patterns (common in PHP CMS/framework stacks), see [URL rewriting](/docs/v3/use-cases/url-rewriting).
- For directive details (`cgi`, `extension`, `fcgi_php`), see [Configuration: CGI support](/docs/v3/configuration/http-cgi) and [Configuration: FastCGI support](/docs/v3/configuration/http-fcgi).
