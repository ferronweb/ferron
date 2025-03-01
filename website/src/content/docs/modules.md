---
title: Server modules
---

You can extend Project Karpacz with modules written in Rust.

The following modules are built into Project Karpacz:

- _cache_ (from Project Karpacz 0.4.0) - this module enables server response caching.
- _cgi_ (from Project Karpacz 0.5.0) - this modules enables the execution of CGI programs.
- _fcgi_ (from Project Karpacz 0.6.0) - this modules enables the support for connecting to FastCGI servers.
- _fproxy_ (from Project Karpacz 0.3.0) - this module enables forward proxy functionality.
- _rproxy_ (from Project Karpacz 0.2.0) - this module enables reverse proxy functionality, if either _proxyTo_ or _secureProxyTo_ configuration property is specified.
- _scgi_ (from Project Karpacz 0.6.0) - this modules enables the support for connecting to SCGI servers.

The following modules are dynamically linked (not supported on musl libc builds), and available in both source and binary form:

- _example_ - this module responds with "Hello World!" for "/hello" request paths.

## Module notes

### _cache_ module

The _cache_ module is a simple in-memory cache module for Project Karpacz that works with "Cache-Control" and "Vary" headers. The cache is shared across all threads.

If you use this module with Project Karpacz's static file serving functionality, set the caching headers for the cache to work, and add "ETag" and "Accept-Encoding" to either a list of headers in the Vary header or in the _cacheVaryHeaders_ property in the Project Karpacz configuration.

### _cgi_ module

To run PHP scripts with this module, you may need to adjust the PHP configuration file, typically located at `/etc/php/<php version>/cgi/php.ini`, by setting the `cgi.force_redirect` property to 0. If you don't make this change, PHP-CGI will show a warning indicating that the PHP-CGI binary was compiled with `force-cgi-redirect` enabled. It is advisable to use directories outside of _cgi-bin_ for user uploads and downloads to prevent the _cgi_ module from mistakenly treating uploaded scripts with shebangs and ELF binary files as CGI applications, which could lead to issues such as malware infections, remote code execution vulnerabilities, or 500 Internal Server Errors.

### _fcgi_ module

PHP-FPM may run on different user than Project Karpacz, so you might need to set permissions for the PHP-FPM user.

If you are using PHP-FPM only for Project Karpacz, you can set the `listen.owner` and `listen.group` properties to the Project Karpacz user in the PHP-FPM pool configuration file (e.g. `/etc/php/8.2/fpm/pool.d/www.conf`).

### _fproxy_ module

If you are using the _fproxy_ module, then hosts on the local network and local host are also accessible from the proxy. You may block these using a firewall, if you don’t want these hosts to be accessible from the proxy.
