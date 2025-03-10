---
title: PHP support
---

Ferron supports running PHP scripts either with a _cgi_ module (using PHP-CGI) or with a _fcgi_ module (using either PHP-CGI configured as a FastCGI server or PHP-FPM). This allows you to host websites built with PHP-based CMSes (like WordPress or Joomla) with Ferron.

To configure PHP through CGI with Ferron, you can use this configuration:

```yaml
# Example global configuration with PHP through CGI
global:
  wwwroot: /var/www/html # Replace it with the path to the webroot
  loadModules:
    - cgi
  cgiScriptExtensions:
    - .php
```

To configure PHP through FastCGI with Ferron, you can use this configuration:

```yaml
# Example global configuration with PHP through FastCGI
global:
  wwwroot: /var/www/html # Replace it with the path to the webroot
  loadModules:
    - fcgi
  fcgiScriptExtensions:
    - .php
  fcgiTo: unix:///run/php/php8.2-fpm.sock # Replace with the Unix socket URL with actual path to the PHP FastCGI daemon socket.
```
