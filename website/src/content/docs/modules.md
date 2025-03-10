---
title: Server modules
---

You can extend Ferron with modules written in Rust.

The following modules are built into Ferron:

- _cache_ (from Project Karpacz 0.4.0) - this module enables server response caching.
- _cgi_ (from Project Karpacz 0.5.0) - this module enables the execution of CGI programs.
- _fauth_ (from Ferron 1.0.0-beta2) - this module enables authentication forwarded to the authentication server.
- _fcgi_ (from Project Karpacz 0.6.0) - this module enables the support for connecting to FastCGI servers.
- _fproxy_ (from Project Karpacz 0.3.0) - this module enables forward proxy functionality.
- _rproxy_ (from Project Karpacz 0.2.0) - this module enables reverse proxy functionality.
- _scgi_ (from Project Karpacz 0.6.0) - this module enables the support for connecting to SCGI servers.

The following modules are dynamically linked (not supported on musl libc builds), and available in both source and binary form:

- _example_ - this module responds with "Hello World!" for "/hello" request paths.

## Module notes

### _cache_ module

The _cache_ module is a simple in-memory cache module for Ferron that works with "Cache-Control" and "Vary" headers. The cache is shared across all threads.

If you use this module with Ferron's static file serving functionality, set the caching headers for the cache to work, and add "ETag" and "Accept-Encoding" to either a list of headers in the Vary header or in the _cacheVaryHeaders_ property in the Ferron configuration.

### _cgi_ module

To run PHP scripts with this module, you may need to adjust the PHP configuration file, typically located at `/etc/php/<php version>/cgi/php.ini`, by setting the `cgi.force_redirect` property to 0. If you don't make this change, PHP-CGI will show a warning indicating that the PHP-CGI binary was compiled with `force-cgi-redirect` enabled. It is advisable to use directories outside of _cgi-bin_ for user uploads and downloads to prevent the _cgi_ module from mistakenly treating uploaded scripts with shebangs and ELF binary files as CGI applications, which could lead to issues such as malware infections, remote code execution vulnerabilities, or 500 Internal Server Errors.

### _fauth_ module

This module is inspired by [Traefik's ForwardAuth middleware](https://doc.traefik.io/traefik/middlewares/http/forwardauth/). If the authentication server replies with a 2xx status code, access is allowed, and the initial request is executed. If not, the response from the authentication server is sent back.

The following request headers are provided to the authentication server:
- **X-Forwarded-Method** - the HTTP method used by the original request
- **X-Forwarded-Proto** - if the original request is encrypted, it's `"https"`, otherwise it's `"http"`.
- **X-Forwarded-Host** - the value of the _Host_ header from the original request
- **X-Forwarded-Uri** - the original request URI
- **X-Forwarded-For** - the client's IP address

### _fcgi_ module

PHP-FPM may run on different user than Ferron, so you might need to set permissions for the PHP-FPM user.

If you are using PHP-FPM only for Ferron, you can set the `listen.owner` and `listen.group` properties to the Ferron user in the PHP-FPM pool configuration file (e.g. `/etc/php/8.2/fpm/pool.d/www.conf`).

### _fproxy_ module

If you are using the _fproxy_ module, then hosts on the local network and local host are also accessible from the proxy. You may block these using a firewall, if you don’t want these hosts to be accessible from the proxy.

### _rproxy_ module

The reverse proxy functionality is enabled when _proxyTo_ or _secureProxyTo_ configuration property is specified.

The following request headers are provided to the backend server:
- **X-Forwarded-Proto** (Ferron 1.0.0-beta2 and newer) - if the original request is encrypted, it's `"https"`, otherwise it's `"http"`.
- **X-Forwarded-Host** (Ferron 1.0.0-beta2 and newer) - the value of the _Host_ header from the original request
- **X-Forwarded-For** - the client's IP address