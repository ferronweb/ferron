---
title: PHP hosting
description: "Host PHP sites on Ferron using CGI or FastCGI/PHP-FPM, with example KDL configs and performance guidance."
---

Ferron supports running PHP scripts either using PHP-CGI, PHP-CGI configured as a FastCGI server or PHP-FPM. This allows you to host websites built with PHP-based CMSes (like WordPress or Joomla) with Ferron.

To configure PHP through CGI with Ferron, you can use this configuration:

```kdl
// Example configuration with PHP through CGI. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"
    cgi
    cgi_extension ".php"
}
```

To configure PHP through FastCGI with Ferron, you can use this configuration:

```kdl
// Example configuration with PHP through FastCGI. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"
    fcgi_php "unix:///run/php/php8.4-fpm.sock" // Replace with the Unix socket URL with actual path to the PHP FastCGI daemon socket.
    // Also, if using Unix socket with PHP-FPM,
    // set the listener owner and group in the PHP pool configuration to the web server user (`ferron`, if you used installer for GNU/Linux)
    // For example:
    //   listen.owner = ferron
    //   listen.group = ferron
}
```

To ensure optimal web server performance and efficiency, it is recommended to use FastCGI instead of CGI, as FastCGI keeps PHP processes running persistently, reducing the overhead of starting a new process for each request, therefore improving response times and resource utilization.
