---
title: PHP support
---

Ferron supports running PHP scripts either with a _cgi_ module (using PHP-CGI) or with a _fcgi_ module (using either PHP-CGI configured as a FastCGI server or PHP-FPM).

To configure PHP through CGI with Ferron, you can use this configuration (for global configuration):

```yaml
global:
  cgiScriptExtensions:
    - .php
```

To configure PHP through FastCGI with Ferron, you can use this configuration (for global configuration):

```yaml
global:
  fcgiScriptExtensions:
    - .php
  fcgiTo: unix:///run/php/php8.2-fpm.sock # Replace with the Unix socket URL with actual path to the PHP FastCGI daemon socket.
```
