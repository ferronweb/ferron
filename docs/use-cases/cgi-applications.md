---
title: CGI applications
description: "Host legacy CGI applications on Ferron, including extension mapping, interpreters, and environment variables."
---

Ferron supports classic CGI applications through the `cgi` module. This is mainly useful for legacy stacks and script-based workflows.

For new deployments, prefer HTTP reverse proxying or FastCGI when possible, because it avoids starting a new process per request and usually performs better.

## Basic CGI setup

To run CGI programs, enable `cgi`:

```kdl
// Example configuration with CGI enabled. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace with your site directory
    cgi
}
```

In this setup, scripts inside `cgi-bin` can be executed as CGI programs.

## Executing scripts by extension

You can also execute CGI scripts outside `cgi-bin` using `cgi_extension`:

```kdl
// Example configuration with CGI extensions. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace with your site directory
    cgi
    cgi_extension ".cgi" ".pl" ".py"
}
```

## Custom CGI interpreters and environment variables

If needed, define explicit interpreters and pass environment variables to CGI applications:

```kdl
// Example configuration with CGI interpreters and environment variables.
example.com {
    root "/var/www/html" // Replace with your site directory
    cgi
    cgi_extension ".cgi" ".pl" ".py"
    cgi_interpreter ".py" "/usr/bin/python3"
    cgi_interpreter ".pl" "/usr/bin/perl"
    cgi_environment "PATH" "/usr/bin:/bin"
}
```

## Notes and troubleshooting

- If a script is downloaded instead of executed, verify `cgi` is enabled and the script extension is listed in `cgi_extension` (when outside `cgi-bin`).
- If CGI scripts fail with `500 Internal Server Error`, confirm executable permissions and interpreter paths.
- If using PHP via CGI, you may need `cgi.force_redirect = 0` in the CGI `php.ini`, depending on your PHP-CGI build.
- Keep upload/download directories outside `cgi-bin` to reduce the risk of accidental execution of uploaded files.
- For PHP-specific guidance, see [PHP hosting](/docs/use-cases/php).
- For directive reference (`cgi`, `cgi_extension`, `cgi_interpreter`, `cgi_environment`), see [Configuration: application backends](/docs/configuration/application-backends).
