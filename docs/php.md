---
title: PHP support
---

Ferron supports running PHP scripts either with a _cgi_ module (using PHP-CGI) or with a _fcgi_ module (using either PHP-CGI configured as a FastCGI server or PHP-FPM). This allows you to host websites built with PHP-based CMSes (like WordPress or Joomla) with Ferron.

To configure PHP through CGI with Ferron, you can use this configuration:

```kdl
// Example global configuration with PHP through CGI
* {
    root "/var/www/html"
    cgi
    cgi_extension ".php"
}
```

To configure PHP through FastCGI with Ferron, you can use this configuration:

```kdl
// Example global configuration with PHP through FastCGI
* {
    root "/var/www/html"
    fcgi_php "unix:///run/php/php8.2-fpm.sock" // Replace with the Unix socket URL with actual path to the PHP FastCGI daemon socket.
}
```

To ensure optimal web server performance and efficiency, it is recommended to use FastCGI instead of CGI, as FastCGI keeps PHP processes running persistently, reducing the overhead of starting a new process for each request, therefore improving response times and resource utilization.
