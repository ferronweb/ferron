---
title: Web applications
---

Ferron is compatible with various web applications, like WordPress, Joomla, and more.

## WordPress

WordPress is a very popular, open-source content management system (CMS) that allows website owners to manage web content, primarily in form of websites and blogs.

For WordPress to support URL rewriting in Ferron, you can install and activate the [Ferron URL rewriting support plugin](https://github.com/ferronweb/ferron-rewrite-support).

You can use the configuration below for websites built on WordPress:

```kdl
// Example configuration with WordPress. Replace "example.com" with your domain name.
example.com {
    root "/var/www/wordpress" // Replace with the path to the directory, where WordPress is installed

    // Deny access to some files and directories
    status 403 regex="/\\."
    status 403 regex="^/(?:uploads|files)/.*\\.php(?:$|[#?])"

    // Pretty URLs
    rewrite "^/(.*)" "/index.php/$1" file=#false directory=#false last=#true

    fcgi_php "unix:///run/php/php8.4-fpm.sock" // Replace with the path to the PHP-FPM socket
    // Also, if using Unix socket with PHP-FPM,
    // set the listener owner and group in the pool configuration to the web server user (`ferron`, if you used installer for GNU/Linux)
    // For example:
    //   listen.owner = ferron
    //   listen.group = ferron
}
```

## Joomla

Joomla is an open-source content management system (CMS) known for its extensibility and flexibility, making it sutable for a wide range of websites, from simple blogs to complex e-commerce platforms and corporate sites.

You can use the configuration below (without caching) for websites built on Joomla:

```kdl
// Example configuration with Joomla. Replace "example.com" with your domain name.
example.com {
    root "/var/www/joomla" // Replace with the path to the directory, where Joomla is installed

    // Deny access to some directories and files
    status 403 regex="^/(?:images|cache|media|logs|tmp)/.*\\.(?:php|pl|py|jsp|asp|sh|cgi)(?:$|[#?])"

    // Pretty URLs
    rewrite "^/api(?:/(.*))?" "/api/index.php/$1" file=#false directory=#false last=#true
    rewrite "^/(.*)" "/index.php/$1" file=#false directory=#false last=#true

    fcgi_php "unix:///run/php/php8.4-fpm.sock" // Replace with the path to the PHP-FPM socket
    // Also, if using Unix socket with PHP-FPM,
    // set the listener owner and group in the pool configuration to the web server user (`ferron`, if you used installer for GNU/Linux)
    // For example:
    //   listen.owner = ferron
    //   listen.group = ferron
}
```

If you enable HTTP caching in Ferron (using the `cache` directive), you can install the [Server Cache for Joomla](https://www.web-expert.gr/en/joomla-extensions/item/127-nginx-server-cache-joomla) extension. This extension sets `Cache-Control` header, which can be then used by the `cache` module.

## Laravel

Laravel is a free, open-source PHP web framework, which is used to develop web applications.

You can use the configuration below for websites built with Laravel:

```kdl
// Example configuration with Laravel. Replace "example.com" with your domain name.
example.com {
    root "/var/www/laravel/public" // Replace with the path to the "public" directory of the Laravel application

    // Pretty URLs
    rewrite "^/(.*)" "/index.php/$1" file=#false directory=#false last=#true

    fcgi_php "unix:///run/php/php8.4-fpm.sock" // Replace with the Unix socket URL with actual path to the PHP FastCGI daemon socket.
    // Also, if using Unix socket with PHP-FPM,
    // set the listener owner and group in the pool configuration to the web server user (`ferron`, if you used installer for GNU/Linux)
    // For example:
    //   listen.owner = ferron
    //   listen.group = ferron
}
```

## YaBB

YaBB is a free Internet forum package written in Perl.

You can use the configuration below for forums powered by YaBB:

```kdl
// Example configuration with YaBB. Replace "example.com" with your domain name.
example.com {
    root "/var/www/yabb" // Replace with the path to the directory, where Joomla is installed (including the "cgi-bin" directory)
    cgi

    // Redirect the index to YaBB
    status 301 regex="^/(?:$|[?#])" location="/cgi-bin/yabb2/YaBB.pl"

    // Forbid access to some directories and files
    status 403 regex="^/cgi-bin/yabb2/(?:Convert|Backups|Templates|Members|Sources|Admin|Messages|Languages|Variables|Boards|Help|Modules)(?:$|[/?#])"
}
```
