---
title: Compatibility with web applications
---

Ferron has been tested for compatibility with various web applications, ensuring seamless integration with popular content management systems (CMS). Below, you'll find specific configurations and recommendations for using Ferron with different platforms. If you encounter any issues, consider checking for additional plugins or configurations that enhance compatibility.

## WordPress

WordPress is a very popular, open-source content management system (CMS) that allows website owners to manage web content, primarily in form of websites and blogs.

We tested Ferron 1.0.0-beta8 with WordPress, and it works without any problems, although it doesn't support URL rewriting in Ferron. For WordPress to support URL rewriting in Ferron, you can install and activate the [Ferron URL rewriting support plugin](https://github.com/ferronweb/ferron-rewrite-support).

You can use the configuration below for websites built on WordPress:

```yaml
global:
  loadModules:
   - fcgi
  logFilePath: /var/log/ferron/access.log # Replace with the path to the access log
  errorLogFilePath: /var/log/ferron/error.log # Replace with the path to the error log

hosts:
  - domain: "*"
    wwwroot: /var/www/ferron # Replace with the path to the webroot
    fcgiScriptExtensions:
      - .php
    fcgiTo: unix:///run/php/php8.4-fpm.sock # Replace with the path to the PHP-FPM socket
    nonStandardCodes:
      - scode: 403
        regex: "/\\."
      - scode: 403
        regex: "^/(?:uploads|files)/.*\\.php(?:$|[#?])"
    rewriteMap:
      - regex: "^/(.*)"
        replacement: "/index.php/$1"
        isNotFile: true
        isNotDirectory: true
        last: true
```

## Joomla

Joomla is an open-source content management system (CMS) written in PHP that allows website owners to easily manage web content. Joomla is known for its extensibility and flexibility, making it sutable for a wide range of websites, from simple blogs to complex e-commerce platforms and corporate sites.

We tested Ferron 1.0.0-beta8 with Joomla, and it works without any problems.

You can use the configuration below (without caching) for websites built on Joomla:

```yaml
global:
  loadModules:
    - fcgi
  logFilePath: /var/log/ferron/access.log # Replace with the path to the access log
  errorLogFilePath: /var/log/ferron/error.log # Replace with the path to the error log

hosts:
  - domain: "*"
    wwwroot: /var/www/ferron # Replace with the path to the webroot
    fcgiScriptExtensions:
      - .php
    fcgiTo: unix:///run/php/php8.4-fpm.sock # Replace with the path to the PHP-FPM socket
    nonStandardCodes:
      - scode: 403
        regex: "^/(?:images|cache|media|logs|tmp)/.*\\.(?:php|pl|py|jsp|asp|sh|cgi)(?:$|[#?])"
    rewriteMap:
      - regex: "^/api(?:/(.*))?"
        replacement: "/api/index.php/$1"
        isNotFile: true
        isNotDirectory: true
        last: true
      - regex: "^/(.*)"
        replacement: "/index.php/$1"
        isNotFile: true
        isNotDirectory: true
        last: true
```

If you enable and configure a `cache` module in Ferron, you can install the [Server Cache for Joomla](https://www.web-expert.gr/en/joomla-extensions/item/127-nginx-server-cache-joomla) extension. This extension sets `Cache-Control` header, which will be then used by the `cache` module.